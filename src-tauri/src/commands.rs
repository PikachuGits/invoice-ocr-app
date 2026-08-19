use std::collections::HashMap;
use tauri::{AppHandle, Emitter, State};

use crate::db::{Database, InvoiceFile, InvoiceRecord};
use crate::invoice_extractor;
use crate::ocr_client::OcrClient;

#[derive(serde::Serialize, Clone)]
pub struct ProgressPayload {
    pub status: String,
    pub message: String,
}

/// 多文件排队进度：total 总文件数，done 已完成数，current 当前处理文件名。
#[derive(serde::Serialize, Clone)]
pub struct QueuePayload {
    pub total: usize,
    pub done: usize,
    pub current: String,
}

#[derive(serde::Serialize)]
pub struct InvoiceListResponse {
    pub records: Vec<InvoiceRecord>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
    pub counts: InvoiceCounts,
}

#[derive(serde::Serialize, Default)]
pub struct InvoiceCounts {
    pub all: u64,
    pub success: u64,
    pub failed: u64,
}

#[derive(serde::Serialize)]
pub struct InvoiceDetailResponse {
    pub invoice: InvoiceRecord,
    pub files: Vec<InvoiceFile>,
}

fn sha256_of_file(path: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| format!("Read file error: {}", e))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

#[allow(dead_code)]
fn md5_of_file(path: &str) -> Result<String, String> {
    use md5::{Digest, Md5};
    let bytes = std::fs::read(path).map_err(|e| format!("Read file error: {}", e))?;
    let mut hasher = Md5::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// 对一组图片依次提交识别并合并多页结果。
async fn run_ocr_pages(
    app: &AppHandle,
    paths: &[String],
) -> Result<(String, String), String> {
    let client = OcrClient::new();

    let mut all_pages: Vec<crate::ocr_client::PageData> = Vec::new();
    let mut raw_parts: Vec<String> = Vec::new();

    for (idx, path) in paths.iter().enumerate() {
        let msg = if paths.len() > 1 {
            format!("正在提交图片 {}/{}...", idx + 1, paths.len())
        } else {
            "正在提交图片...".to_string()
        };
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "progress".into(),
                message: msg,
            },
        );

        let job_id = client.submit(path).await.map_err(|e| {
            let _ = app.emit(
                "ocr-progress",
                ProgressPayload {
                    status: "error".into(),
                    message: format!("提交失败: {}", e),
                },
            );
            e.to_string()
        })?;

        let (pages, raw_json) = client
            .poll(&job_id, |status| {
                let _ = app.emit(
                    "ocr-progress",
                    ProgressPayload {
                        status: "progress".into(),
                        message: status,
                    },
                );
            })
            .await
            .map_err(|e| {
                let _ = app.emit(
                    "ocr-progress",
                    ProgressPayload {
                        status: "error".into(),
                        message: format!("识别失败: {}", e),
                    },
                );
                e.to_string()
            })?;

        all_pages.extend(pages);
        if !raw_json.trim().is_empty() {
            raw_parts.push(raw_json);
        }
    }

    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "progress".into(),
            message: "正在解析发票...".into(),
        },
    );

    let mut sparse: HashMap<String, String> = HashMap::new();
    for page in &all_pages {
        let page_result =
            invoice_extractor::parse_invoice_from_markdown(&page.markdown_text, &page.blocks);
        invoice_extractor::merge_sparse_results(&mut sparse, page_result);
    }

    let result = invoice_extractor::make_standard_result(&sparse);
    let parsed_json = serde_json::to_string(&result).map_err(|e| e.to_string())?;
    let raw_json = raw_parts.join("\n");

    Ok((raw_json, parsed_json))
}

/// 是否为 PDF 文件（按扩展名判断，不区分大小写）。
fn is_pdf(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

/// 支持的发票文件类型（图片 + PDF）。
fn is_supported_ocr_file(path: &str) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    matches!(
        ext.as_deref(),
        Some("jpg" | "jpeg" | "png" | "bmp" | "webp" | "pdf")
    )
}

/// 准备待识别文件列表：PDF 先渲染为图片，返回 (图片路径列表, 需清理的临时目录)。
fn prepare_ocr_inputs(
    file_path: &str,
) -> Result<(Vec<String>, Option<std::path::PathBuf>), String> {
    if is_pdf(file_path) {
        let (paths, dir) = crate::pdf::render_pdf_to_images(file_path)?;
        Ok((paths, Some(dir)))
    } else {
        Ok((vec![file_path.to_string()], None))
    }
}

/// 单文件识别（多页 PDF 合并），返回 (raw_json, parsed_json, 页数)。
async fn recognize_one_file(
    app: &AppHandle,
    file_path: &str,
) -> Result<(String, String, i64), String> {
    let inputs_result = {
        let file_path = file_path.to_string();
        tauri::async_runtime::spawn_blocking(move || prepare_ocr_inputs(&file_path))
            .await
            .map_err(|e| format!("文件预处理线程失败: {}", e))?
    };
    let (inputs, cleanup_dir) = inputs_result?;
    let page_count = inputs.len() as i64;
    let ocr_result = run_ocr_pages(app, &inputs).await;
    if let Some(dir) = &cleanup_dir {
        let _ = std::fs::remove_dir_all(dir);
    }
    let (raw_json, parsed_json) = ocr_result?;
    Ok((raw_json, parsed_json, page_count))
}

/// StandardResult JSON → sparse 字段映射（用于跨文件汇总合并）。
fn standard_to_sparse(parsed: &str) -> HashMap<String, String> {
    let mut sparse = HashMap::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(parsed) {
        if let Some(wr) = v.get("words_result").and_then(|w| w.as_object()) {
            for (k, val) in wr {
                if let Some(s) = val.as_str() {
                    if !s.is_empty() {
                        sparse.insert(k.clone(), s.to_string());
                    }
                } else if let Some(arr) = val.as_array() {
                    if !arr.is_empty() {
                        sparse.insert(
                            k.clone(),
                            serde_json::to_string(arr).unwrap_or_default(),
                        );
                    }
                }
            }
        }
    }
    sparse
}

fn parse_standard(parsed: &str) -> invoice_extractor::StandardResult {
    serde_json::from_str(parsed).unwrap_or_else(|_| invoice_extractor::StandardResult {
        log_id: String::new(),
        words_result_num: 0,
        words_result: HashMap::new(),
    })
}

/// 多文件识别：文件级去重 + 按发票号合并为发票主数据。
/// 返回各发票合并后的标准结果（前端仅用于确认，列表自动刷新）。
#[tauri::command]
pub async fn recognize_invoice(
    app: AppHandle,
    db: State<'_, Database>,
    image_paths: Vec<String>,
) -> Result<Vec<invoice_extractor::StandardResult>, String> {
    struct Recognized {
        path: String,
        file_name: String,
        sha256: String,
        md5: String,
        raw: String,
        parsed: String,
        code: String,
        num: String,
        page_count: i64,
    }

    if image_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }

    // 0. 文件类型校验：只允许图片(jpg/jpeg/png/bmp/webp)和 PDF
    let mut unsupported: Vec<String> = Vec::new();
    for path in &image_paths {
        if !is_supported_ocr_file(path) {
            unsupported.push(
                std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string(),
            );
        }
    }
    if !unsupported.is_empty() {
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "error".into(),
                message: format!("不支持的文件类型，仅支持图片和 PDF: {}", unsupported.join(", ")),
            },
        );
        return Err(format!(
            "不支持的文件类型，仅支持 jpg/jpeg/png/bmp/webp/pdf:\n{}",
            unsupported.join("\n")
        ));
    }

    // 1. 文件级去重：已识别文件跳过（同文件不重复识别）
    let mut new_files: Vec<Recognized> = Vec::new();
    let mut cached_ids: Vec<i64> = Vec::new();
    for path in &image_paths {
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "progress".into(),
                message: format!("正在检查文件: {}", path),
            },
        );
        let sha256 = sha256_of_file(path)?;
        if let Some(file) = db.find_file_by_sha256(&sha256).map_err(|e| e.to_string())? {
            let cached_ok = db
                .get_invoice(file.invoice_id)
                .map_err(|e| e.to_string())?
                .map(|inv| inv.status == "success")
                .unwrap_or(false);
            if cached_ok {
                let _ = app.emit(
                    "ocr-progress",
                    ProgressPayload {
                        status: "progress".into(),
                        message: format!("检测到已识别的文件，跳过: {}", file.file_name),
                    },
                );
                if !cached_ids.contains(&file.invoice_id) {
                    cached_ids.push(file.invoice_id);
                }
                continue;
            }
            // 失败记录允许重新识别：先删除旧的失败发票
            log::warn!("缓存记录为失败状态，删除后重新识别: {}", file.file_name);
            let _ = db.delete_invoices(&[file.invoice_id]);
        }

        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let md5 = md5_of_file(path)?;
        let (raw, parsed, page_count) = recognize_one_file(&app, path).await?;
        let v: serde_json::Value = serde_json::from_str(&parsed).unwrap_or_default();
        let wr = &v["words_result"];
        let code = wr["InvoiceCode"].as_str().unwrap_or("").trim().to_string();
        let num = wr["InvoiceNum"].as_str().unwrap_or("").trim().to_string();

        new_files.push(Recognized {
            path: path.clone(),
            file_name,
            sha256,
            md5,
            raw,
            parsed,
            code,
            num,
            page_count,
        });
    }

    if new_files.is_empty() {
        let mut results = Vec::new();
        for id in cached_ids {
            if let Some(rec) = db.get_invoice(id).map_err(|e| e.to_string())? {
                if rec.status == "success" {
                    results.push(parse_standard(&rec.parsed_result));
                }
            }
        }
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "success".into(),
                message: "识别完成!（全部命中缓存）".into(),
            },
        );
        return Ok(results);
    }

    // 1.5 逐个识别新文件，发出排队进度（跳过文件计入已完成）
    let total = image_paths.len();
    let skipped = total - new_files.len();
    let mut failed_files: Vec<String> = Vec::new();
    let mut i = 0;
    while i < new_files.len() {
        let f = &mut new_files[i];
        let _ = app.emit(
            "ocr-queue",
            QueuePayload {
                total,
                done: skipped + i,
                current: f.file_name.clone(),
            },
        );
        match recognize_one_file(&app, &f.path).await {
            Ok((raw, parsed, page_count)) => {
                f.raw = raw;
                f.parsed = parsed;
                f.page_count = page_count;
                let v: serde_json::Value = serde_json::from_str(&f.parsed).unwrap_or_default();
                let wr = &v["words_result"];
                f.code = wr["InvoiceCode"].as_str().unwrap_or("").trim().to_string();
                f.num = wr["InvoiceNum"].as_str().unwrap_or("").trim().to_string();
                i += 1;
            }
            Err(e) => {
                let msg = format!("{}: {}", f.file_name, e);
                let _ = app.emit(
                    "ocr-progress",
                    ProgressPayload {
                        status: "error".into(),
                        message: format!("文件识别失败，已跳过: {}", msg),
                    },
                );
                log::warn!("批量识别文件失败 {}", msg);
                // 失败文件单独入库为失败发票，可在列表查看并重新识别
                let invoice_id = db
                    .create_invoice("", "", &f.file_name, "", "failed")
                    .map_err(|e| e.to_string())?;
                db.insert_file(
                    invoice_id,
                    &f.sha256,
                    &f.md5,
                    &f.file_name,
                    &f.path,
                    "",
                    f.page_count,
                )
                .map_err(|e| e.to_string())?;
                failed_files.push(msg);
                new_files.remove(i);
            }
        }
    }

    if new_files.is_empty() {
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "error".into(),
                message: format!("全部 {} 个文件识别失败", failed_files.len()),
            },
        );
        return Err(format!(
            "全部文件识别失败:\n{}",
            failed_files.join("\n")
        ));
    }
    let _ = app.emit(
        "ocr-queue",
        QueuePayload {
            total,
            done: total,
            current: String::new(),
        },
    );

    // 2. 按发票号分组（无发票号的文件各自独立）
    let mut groups: Vec<Vec<Recognized>> = Vec::new();
    let mut group_index: HashMap<(String, String), usize> = HashMap::new();
    for f in new_files {
        if f.num.is_empty() {
            groups.push(vec![f]);
        } else {
            let key = (f.code.clone(), f.num.clone());
            let idx = group_index.entry(key).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[*idx].push(f);
        }
    }

    // 3. 汇总合并并写入数据库（发票主数据 + 附件）
    let mut results: Vec<invoice_extractor::StandardResult> = Vec::new();
    for members in groups {
        let mut sparse: HashMap<String, String> = HashMap::new();
        for m in &members {
            invoice_extractor::merge_sparse_results(&mut sparse, standard_to_sparse(&m.parsed));
        }
        let merged = invoice_extractor::make_standard_result(&sparse);
        let merged_json = serde_json::to_string(&merged).map_err(|e| e.to_string())?;
        let first = &members[0];

        let existing_id = if first.num.is_empty() {
            None
        } else {
            db.find_invoice_by_num(&first.code, &first.num)
                .map_err(|e| e.to_string())?
        };

        let invoice_id = match existing_id {
            // 已有同号发票：新旧结果合并更新，附件追加
            Some(existing_id) => {
                let mut esparse =
                    standard_to_sparse(&db.get_invoice(existing_id).map_err(|e| e.to_string())?.map(|r| r.parsed_result).unwrap_or_default());
                invoice_extractor::merge_sparse_results(&mut esparse, sparse);
                let em = invoice_extractor::make_standard_result(&esparse);
                let ejson = serde_json::to_string(&em).map_err(|e| e.to_string())?;
                db.update_invoice_result(existing_id, &ejson, "success", &first.file_name)
                    .map_err(|e| e.to_string())?;
                results.push(em);
                existing_id
            }
            // 新发票
            None => {
                let invoice_id = db
                    .create_invoice(&first.code, &first.num, &first.file_name, &merged_json, "success")
                    .map_err(|e| e.to_string())?;
                results.push(merged);
                invoice_id
            }
        };

        for m in &members {
            db.insert_file(
                invoice_id,
                &m.sha256,
                &m.md5,
                &m.file_name,
                &m.path,
                &m.raw,
                m.page_count,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let success_msg = if failed_files.is_empty() {
        format!("识别完成! 共 {} 张发票", results.len())
    } else {
        format!(
            "识别完成! 共 {} 张发票，{} 个文件失败（已跳过）",
            results.len(),
            failed_files.len()
        )
    };
    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "success".into(),
            message: success_msg,
        },
    );

    Ok(results)
}

/// 发票级重新识别：对该发票全部附件重新识别并汇总。
#[tauri::command]
pub async fn re_recognize_invoice(
    app: AppHandle,
    db: State<'_, Database>,
    id: i64,
) -> Result<invoice_extractor::StandardResult, String> {
    let (record, files) = db
        .get_invoice_with_files(id)?
        .ok_or_else(|| format!("Invoice {} not found", id))?;

    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "progress".into(),
            message: format!("正在重新识别: {}", record.file_name),
        },
    );

    let mut sparse: HashMap<String, String> = HashMap::new();
    let mut any_ok = false;
    for file in &files {
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "progress".into(),
                message: format!("正在重新识别附件: {}", file.file_name),
            },
        );
        match recognize_one_file(&app, &file.file_path).await {
            Ok((raw, parsed, _page_count)) => {
                invoice_extractor::merge_sparse_results(&mut sparse, standard_to_sparse(&parsed));
                let _ = db.update_file_raw(&file.sha256, &raw);
                any_ok = true;
            }
            Err(e) => {
                log::warn!("附件重新识别失败 {}: {}", file.file_name, e);
            }
        }
    }

    if !any_ok {
        db.update_invoice_failed(id)?;
        let _ = app.emit(
            "ocr-progress",
            ProgressPayload {
                status: "error".into(),
                message: "所有附件识别失败".into(),
            },
        );
        return Err("所有附件识别失败".to_string());
    }

    let merged = invoice_extractor::make_standard_result(&sparse);
    let merged_json = serde_json::to_string(&merged).map_err(|e| e.to_string())?;
    db.update_invoice_result(id, &merged_json, "success", &record.file_name)
        .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "success".into(),
            message: "重新识别完成!".into(),
        },
    );

    Ok(merged)
}

#[tauri::command]
pub fn get_invoice_list(
    db: State<'_, Database>,
    page: u64,
    page_size: u64,
    status_filter: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<InvoiceListResponse, String> {
    let (records, total) =
        db.list_invoices(page, page_size, status_filter.as_deref(), start_date.as_deref(), end_date.as_deref())?;
    let counts = InvoiceCounts {
        all: db.count_invoices(None)?,
        success: db.count_invoices(Some("success"))?,
        failed: db.count_invoices(Some("failed"))?,
    };
    Ok(InvoiceListResponse {
        records,
        total,
        page,
        page_size,
        counts,
    })
}

#[tauri::command]
pub fn get_invoice_detail(
    db: State<'_, Database>,
    id: i64,
) -> Result<Option<InvoiceDetailResponse>, String> {
    let Some((invoice, files)) = db.get_invoice_with_files(id)? else {
        return Ok(None);
    };
    Ok(Some(InvoiceDetailResponse { invoice, files }))
}

/// 批量删除发票（级联删除附件）。
#[tauri::command]
pub fn delete_invoices(
    db: State<'_, Database>,
    ids: Vec<i64>,
) -> Result<usize, String> {
    db.delete_invoices(&ids)
}

#[tauri::command]
pub fn export_invoices_excel(
    db: State<'_, Database>,
    ids: Vec<i64>,
    export_mode: String,
) -> Result<String, String> {
    let records = db.get_invoices_by_ids(&ids)?;
    if records.is_empty() {
        return Err("No records to export".to_string());
    }

    let files: Vec<Vec<InvoiceFile>> = records
        .iter()
        .map(|r| db.get_files_by_invoice(r.id).unwrap_or_default())
        .collect();

    let save_path = rfd::FileDialog::new()
        .set_title("选择导出路径")
        .set_file_name("发票数据.xlsx")
        .add_filter("Excel 文件", &["xlsx"])
        .save_file();

    let save_path = save_path.ok_or("用户取消")?;

    crate::exporter::export_invoices_excel(&records, &files, &export_mode, &save_path)?;

    Ok(save_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_config_value(
    db: State<'_, Database>,
    key: String,
) -> Result<Option<String>, String> {
    db.get_config(&key)
}

#[tauri::command]
pub fn set_config_value(
    db: State<'_, Database>,
    key: String,
    value: String,
) -> Result<(), String> {
    db.set_config(&key, &value)
}
