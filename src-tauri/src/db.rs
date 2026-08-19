use rusqlite::{params, params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

/// 发票主数据：按 (invoice_code, invoice_num) 合并，一张发票一条记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceRecord {
    pub id: i64,
    pub invoice_code: String,
    pub invoice_num: String,
    pub file_name: String, // 主文件名（首个附件）
    pub status: String,    // "success" | "failed"
    pub retry_count: i64,
    pub ocr_count: i64,
    pub parsed_result: String,
    pub created_at: String,
    pub updated_at: String,
    pub attachment_count: i64,
}

/// 发票附件（文件级记录，sha256 唯一去重）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceFile {
    pub id: i64,
    pub invoice_id: i64,
    pub sha256: String,
    pub md5: String,
    pub file_name: String,
    pub file_path: String,
    pub ocr_raw_json: String,
    pub page_count: i64,
    pub created_at: String,
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("SELECT {} FROM {} LIMIT 0", column, table))
        .is_ok()
}

fn extract_invoice_keys(parsed: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(parsed).unwrap_or_default();
    let wr = &v["words_result"];
    let num = wr["InvoiceNum"].as_str().unwrap_or("").trim().to_string();
    let code = wr["InvoiceCode"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    (code, num)
}

fn create_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS invoices (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            invoice_code    TEXT NOT NULL DEFAULT '',
            invoice_num     TEXT NOT NULL DEFAULT '',
            file_name       TEXT NOT NULL DEFAULT '',
            status          TEXT NOT NULL DEFAULT 'success',
            retry_count     INTEGER NOT NULL DEFAULT 0,
            ocr_count       INTEGER NOT NULL DEFAULT 1,
            parsed_result   TEXT NOT NULL DEFAULT '',
            created_at      TEXT NOT NULL DEFAULT (datetime('now','localtime')),
            updated_at      TEXT NOT NULL DEFAULT (datetime('now','localtime'))
        );
        CREATE INDEX IF NOT EXISTS idx_invoices_num ON invoices(invoice_code, invoice_num);

        CREATE TABLE IF NOT EXISTS invoice_files (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            invoice_id      INTEGER NOT NULL,
            sha256          TEXT NOT NULL UNIQUE,
            md5             TEXT NOT NULL DEFAULT '',
            file_name       TEXT NOT NULL DEFAULT '',
            file_path       TEXT NOT NULL DEFAULT '',
            ocr_raw_json    TEXT NOT NULL DEFAULT '',
            page_count      INTEGER NOT NULL DEFAULT 1,
            created_at      TEXT NOT NULL DEFAULT (datetime('now','localtime')),
            FOREIGN KEY(invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS config (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to init DB: {}", e))
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| format!("Failed to open DB: {}", e))?;

        // 旧结构（文件级 invoices 表含 sha256/md5/file_path 等列）→ 迁移为新结构
        if table_has_column(&conn, "invoices", "sha256") {
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE invoices RENAME TO invoices_legacy;
                 COMMIT;",
            )
            .map_err(|e| format!("Failed to rename legacy table: {}", e))?;
            create_tables(&conn)?;
            migrate_legacy(&conn)?;
            conn.execute_batch(
                "BEGIN;
                 DROP TABLE invoices_legacy;
                 COMMIT;",
            )
            .map_err(|e| format!("Failed to drop legacy table: {}", e))?;
        } else {
            create_tables(&conn)?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ---------- 查询辅助 ----------

    const SELECT_WITH_COUNT: &'static str = "i.id, i.invoice_code, i.invoice_num, i.file_name, \
         i.status, i.retry_count, i.ocr_count, i.parsed_result, i.created_at, i.updated_at, \
         (SELECT COUNT(*) FROM invoice_files f WHERE f.invoice_id = i.id) AS attachment_count";

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<InvoiceRecord> {
        Ok(InvoiceRecord {
            id: row.get(0)?,
            invoice_code: row.get(1)?,
            invoice_num: row.get(2)?,
            file_name: row.get(3)?,
            status: row.get(4)?,
            retry_count: row.get(5)?,
            ocr_count: row.get(6)?,
            parsed_result: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            attachment_count: row.get::<_, i64>(10).unwrap_or(0),
        })
    }

    fn row_to_file(row: &rusqlite::Row) -> rusqlite::Result<InvoiceFile> {
        Ok(InvoiceFile {
            id: row.get(0)?,
            invoice_id: row.get(1)?,
            sha256: row.get(2)?,
            md5: row.get(3)?,
            file_name: row.get(4)?,
            file_path: row.get(5)?,
            ocr_raw_json: row.get(6)?,
            page_count: row.get(7)?,
            created_at: row.get(8)?,
        })
    }

    // ---------- 附件（文件级去重） ----------

    /// 按文件哈希查附件（识别去重：同一文件不重复处理）。
    pub fn find_file_by_sha256(&self, sha256: &str) -> Result<Option<InvoiceFile>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, invoice_id, sha256, md5, file_name, file_path, \
                 ocr_raw_json, page_count, created_at FROM invoice_files WHERE sha256 = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![sha256], Self::row_to_file)
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(file)) => Ok(Some(file)),
            _ => Ok(None),
        }
    }

    /// 插入附件；sha256 已存在则仅更新归属发票（文件换发票合并）。
    pub fn insert_file(
        &self,
        invoice_id: i64,
        sha256: &str,
        md5: &str,
        file_name: &str,
        file_path: &str,
        ocr_raw_json: &str,
        page_count: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO invoice_files \
             (invoice_id, sha256, md5, file_name, file_path, ocr_raw_json, page_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(sha256) DO UPDATE SET invoice_id = excluded.invoice_id, \
             ocr_raw_json = excluded.ocr_raw_json",
            params![invoice_id, sha256, md5, file_name, file_path, ocr_raw_json, page_count],
        )
        .map_err(|e| format!("Insert file failed: {}", e))?;
        Ok(())
    }

    /// 更新附件识别原始结果（重新识别后）。
    pub fn update_file_raw(&self, sha256: &str, ocr_raw_json: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE invoice_files SET ocr_raw_json = ?1 WHERE sha256 = ?2",
            params![ocr_raw_json, sha256],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 发票的全部附件。
    pub fn get_files_by_invoice(&self, invoice_id: i64) -> Result<Vec<InvoiceFile>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, invoice_id, sha256, md5, file_name, file_path, \
                 ocr_raw_json, page_count, created_at FROM invoice_files \
                 WHERE invoice_id = ?1 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let files = stmt
            .query_map(params![invoice_id], Self::row_to_file)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(files)
    }

    // ---------- 发票主数据 ----------

    /// 按发票号查发票（仅当发票号非空时有效）。
    pub fn find_invoice_by_num(
        &self,
        invoice_code: &str,
        invoice_num: &str,
    ) -> Result<Option<i64>, String> {
        if invoice_num.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id FROM invoices WHERE invoice_code = ?1 AND invoice_num = ?2 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![invoice_code, invoice_num], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(id)) => Ok(Some(id)),
            _ => Ok(None),
        }
    }

    /// 新建发票（无发票号时也创建，作为独立记录）。
    pub fn create_invoice(
        &self,
        invoice_code: &str,
        invoice_num: &str,
        file_name: &str,
        parsed_result: &str,
        status: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO invoices (invoice_code, invoice_num, file_name, status, parsed_result) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![invoice_code, invoice_num, file_name, status, parsed_result],
        )
        .map_err(|e| format!("Create invoice failed: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    /// 更新发票识别结果（识别次数 +1）。
    pub fn update_invoice_result(
        &self,
        id: i64,
        parsed_result: &str,
        status: &str,
        file_name: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE invoices SET parsed_result = ?1, status = ?2, \
             ocr_count = ocr_count + 1, updated_at = datetime('now','localtime'), \
             file_name = CASE WHEN file_name = '' THEN ?3 ELSE file_name END \
             WHERE id = ?4",
            params![parsed_result, status, file_name, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 标记识别失败（失败次数 +1）。
    pub fn update_invoice_failed(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE invoices SET status = 'failed', retry_count = retry_count + 1, \
             updated_at = datetime('now','localtime') WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---------- 列表 / 详情 / 导出 ----------

    pub fn list_invoices(
        &self,
        page: u64,
        page_size: u64,
        status_filter: Option<&str>,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<(Vec<InvoiceRecord>, u64), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let mut conds: Vec<String> = Vec::new();
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(s) = status_filter {
            conds.push(format!("i.status = ?{}", args.len() + 1));
            args.push(Box::new(s.to_string()));
        }
        if let Some(d) = start_date {
            if !d.is_empty() {
                conds.push(format!("date(i.created_at) >= ?{}", args.len() + 1));
                args.push(Box::new(d.to_string()));
            }
        }
        if let Some(d) = end_date {
            if !d.is_empty() {
                conds.push(format!("date(i.created_at) <= ?{}", args.len() + 1));
                args.push(Box::new(d.to_string()));
            }
        }
        let where_clause = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let n = args.len();

        let count_sql = format!("SELECT COUNT(*) FROM invoices i {}", where_clause);
        let total: u64 = if n == 0 {
            conn.query_row(&count_sql, [], |row| row.get(0))
                .map_err(|e| e.to_string())?
        } else {
            conn.query_row(
                &count_sql,
                params_from_iter(args.iter().map(|a| a.as_ref())),
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?
        };

        let offset = page.saturating_sub(1) * page_size;
        let list_sql = format!(
            "SELECT {} FROM invoices i {} ORDER BY i.id DESC LIMIT ?{} OFFSET ?{}",
            Self::SELECT_WITH_COUNT,
            where_clause,
            n + 1,
            n + 2,
        );

        let mut stmt = conn.prepare(&list_sql).map_err(|e| e.to_string())?;
        let records: Vec<InvoiceRecord> = if n == 0 {
            stmt.query_map(params![page_size, offset], Self::row_to_record)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let mut all: Vec<Box<dyn rusqlite::types::ToSql>> = args;
            all.push(Box::new(page_size));
            all.push(Box::new(offset));
            stmt.query_map(
                params_from_iter(all.iter().map(|a| a.as_ref())),
                Self::row_to_record,
            )
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
        };

        Ok((records, total))
    }

    /// 批量删除发票（级联删除附件）。
    pub fn delete_invoices(&self, ids: &[i64]) -> Result<usize, String> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
        let in_clause = placeholders.join(",");
        conn.execute(
            &format!("DELETE FROM invoice_files WHERE invoice_id IN ({})", in_clause),
            params_from_iter(ids.iter()),
        )
        .map_err(|e| e.to_string())?;
        let removed = conn
            .execute(
                &format!("DELETE FROM invoices WHERE id IN ({})", in_clause),
                params_from_iter(ids.iter()),
            )
            .map_err(|e| e.to_string())?;
        Ok(removed)
    }

    pub fn get_invoice(&self, id: i64) -> Result<Option<InvoiceRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {} FROM invoices i WHERE i.id = ?1",
            Self::SELECT_WITH_COUNT
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![id], Self::row_to_record)
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(record)) => Ok(Some(record)),
            _ => Ok(None),
        }
    }

    /// 发票 + 附件。
    pub fn get_invoice_with_files(
        &self,
        id: i64,
    ) -> Result<Option<(InvoiceRecord, Vec<InvoiceFile>)>, String> {
        let record = self.get_invoice(id)?;
        match record {
            Some(record) => {
                let files = self.get_files_by_invoice(id)?;
                Ok(Some((record, files)))
            }
            None => Ok(None),
        }
    }

    pub fn get_invoices_by_ids(&self, ids: &[i64]) -> Result<Vec<InvoiceRecord>, String> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT {} FROM invoices i WHERE i.id IN ({}) ORDER BY i.id",
            Self::SELECT_WITH_COUNT,
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        let records = stmt
            .query_map(params_refs.as_slice(), Self::row_to_record)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(records)
    }

    pub fn count_invoices(&self, status: Option<&str>) -> Result<u64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let total = match status {
            Some(s) => conn
                .query_row(
                    "SELECT COUNT(*) FROM invoices WHERE status = ?1",
                    params![s],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?,
            None => conn
                .query_row("SELECT COUNT(*) FROM invoices", [], |row| row.get(0))
                .map_err(|e| e.to_string())?,
        };
        Ok(total)
    }

    // ---------- 配置 ----------

    pub fn get_config(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM config WHERE key = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![key], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(val)) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    pub fn set_config(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// 旧结构（文件级）迁移：解析发票号并按 (代码, 号码) 合并为发票主数据，文件转为附件。
fn migrate_legacy(conn: &Connection) -> Result<(), String> {
    #[derive(Clone)]
    struct Legacy {
        sha256: String,
        md5: String,
        file_name: String,
        file_path: String,
        ocr_raw_json: String,
        parsed_result: String,
        status: String,
        retry_count: i64,
        ocr_count: i64,
        created_at: String,
    }

    let mut stmt = conn
        .prepare(
            "SELECT sha256, md5, file_name, file_path, ocr_raw_json, parsed_result, \
             status, retry_count, ocr_count, created_at FROM invoices_legacy",
        )
        .map_err(|e| format!("Read legacy table failed: {}", e))?;
    let rows: Vec<Legacy> = stmt
        .query_map([], |row| {
            Ok(Legacy {
                sha256: row.get(0)?,
                md5: row.get(1)?,
                file_name: row.get(2)?,
                file_path: row.get(3)?,
                ocr_raw_json: row.get(4)?,
                parsed_result: row.get(5)?,
                status: row.get(6)?,
                retry_count: row.get(7)?,
                ocr_count: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    // 按发票号分组（空发票号各自独立）
    let mut groups: HashMap<(String, String), Vec<Legacy>> = HashMap::new();
    let mut singles: Vec<Legacy> = Vec::new();
    for row in rows {
        let (code, num) = extract_invoice_keys(&row.parsed_result);
        if num.is_empty() {
            singles.push(row);
        } else {
            groups.entry((code, num)).or_default().push(row);
        }
    }

    conn.execute_batch("BEGIN;").map_err(|e| e.to_string())?;

    let result = (|| -> Result<(), String> {
        for ((code, num), members) in &groups {
            let first = &members[0];
            let max_count = members.iter().map(|m| m.ocr_count).max().unwrap_or(1);
            conn.execute(
                "INSERT INTO invoices (invoice_code, invoice_num, file_name, status, \
                 retry_count, ocr_count, parsed_result, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    code,
                    num,
                    first.file_name,
                    first.status,
                    first.retry_count,
                    max_count,
                    first.parsed_result,
                    first.created_at
                ],
            )
            .map_err(|e| e.to_string())?;
            let invoice_id = conn.last_insert_rowid();
            for m in members {
                conn.execute(
                    "INSERT INTO invoice_files \
                     (invoice_id, sha256, md5, file_name, file_path, ocr_raw_json, page_count) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                    params![
                        invoice_id,
                        m.sha256,
                        m.md5,
                        m.file_name,
                        m.file_path,
                        m.ocr_raw_json
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        for row in &singles {
            conn.execute(
                "INSERT INTO invoices (invoice_code, invoice_num, file_name, status, \
                 retry_count, ocr_count, parsed_result, created_at, updated_at) \
                 VALUES ('', '', ?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    row.file_name,
                    row.status,
                    row.retry_count,
                    row.ocr_count,
                    row.parsed_result,
                    row.created_at
                ],
            )
            .map_err(|e| e.to_string())?;
            let invoice_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO invoice_files \
                 (invoice_id, sha256, md5, file_name, file_path, ocr_raw_json, page_count) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                params![
                    invoice_id,
                    row.sha256,
                    row.md5,
                    row.file_name,
                    row.file_path,
                    row.ocr_raw_json
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();

    if result.is_ok() {
        conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;
    } else {
        conn.execute_batch("ROLLBACK;").map_err(|e| e.to_string())?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db_path(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "invoice-db-test-{}-{}.db",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn test_find_invoice_by_num_and_attachment() {
        let path = tmp_db_path("merge");
        let db = Database::new(path.to_str().unwrap()).unwrap();

        let id1 = db
            .create_invoice("code1", "111", "a.jpg", "", "success")
            .unwrap();
        db.insert_file(id1, "sha-a", "m", "a.jpg", "/a.jpg", "", 1)
            .unwrap();
        db.insert_file(id1, "sha-b", "m", "b.jpg", "/b.jpg", "", 2)
            .unwrap();

        // 同号查询应返回 id1
        let found = db.find_invoice_by_num("code1", "111").unwrap();
        assert_eq!(found, Some(id1));

        // 附件数
        let files = db.get_files_by_invoice(id1).unwrap();
        assert_eq!(files.len(), 2);

        let record = db.get_invoice(id1).unwrap().unwrap();
        assert_eq!(record.attachment_count, 2);

        // 无发票号文件不能合并查询
        let none = db.find_invoice_by_num("", "").unwrap();
        assert_eq!(none, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_create_invoice_param_order() {
        let path = tmp_db_path("param-order");
        let db = Database::new(path.to_str().unwrap()).unwrap();
        let json = r#"{"words_result_num":1,"words_result":{"InvoiceNum":"999"}}"#;
        let id = db
            .create_invoice("code", "num", "a.jpg", json, "success")
            .unwrap();
        let record = db.get_invoice(id).unwrap().unwrap();
        // 参数错位会 status 存 JSON、parsed_result 存 "success"
        assert_eq!(record.status, "success");
        assert_eq!(record.parsed_result, json);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_legacy_migration_merges_same_num() {
        let path = tmp_db_path("legacy");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE invoices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sha256 TEXT NOT NULL UNIQUE,
                md5 TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                ocr_raw_json TEXT NOT NULL DEFAULT '',
                parsed_result TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'success',
                retry_count INTEGER NOT NULL DEFAULT 0,
                ocr_count INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO invoices (sha256, md5, file_name, file_path, parsed_result) \
             VALUES (?1, 'm', 'a.jpg', '/a.jpg', ?2)",
            params!["sha-a", r#"{"words_result":{"InvoiceNum":"999","InvoiceCode":"c"}}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO invoices (sha256, md5, file_name, file_path, parsed_result) \
             VALUES (?1, 'm', 'b.jpg', '/b.jpg', ?2)",
            params!["sha-b", r#"{"words_result":{"InvoiceNum":"999","InvoiceCode":"c"}}"#],
        )
        .unwrap();
        // 无发票号的失败记录独立
        conn.execute(
            "INSERT INTO invoices (sha256, md5, file_name, file_path, parsed_result, status) \
             VALUES ('sha-c', 'm', 'c.jpg', '/c.jpg', '', 'failed')",
            [],
        )
        .unwrap();
        drop(conn);

        let db = Database::new(path.to_str().unwrap()).unwrap();

        // 同号两条合并为一条发票 + 两个附件
        let merged_id = db.find_invoice_by_num("c", "999").unwrap().unwrap();
        let (record, files) = db.get_invoice_with_files(merged_id).unwrap().unwrap();
        assert_eq!(record.attachment_count, 2);
        assert_eq!(files.len(), 2);

        // 失败记录独立存在（无发票号）
        let failed: Vec<InvoiceRecord> = db
            .list_invoices(1, 100, Some("failed"), None, None)
            .unwrap()
            .0;
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].file_name, "c.jpg");

        let _ = std::fs::remove_file(&path);
    }
}
