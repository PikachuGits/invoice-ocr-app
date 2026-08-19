use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::html_parser::{parse_structured_tables, Cell};

// ============================================================
// Standard output fields
// ============================================================

pub const STANDARD_FIELDS: &[&str] = &[
    "InvoiceNumDigit", "ServiceType", "InvoiceNum", "InvoiceNumConfirm",
    "SellerName", "CommodityTaxRate", "SellerBank", "Checker",
    "TotalAmount", "CommodityAmount", "InvoiceDate", "CommodityTax",
    "PurchaserName", "CommodityNum", "Province", "City", "SheetNum",
    "Agent", "PurchaserBank", "Remarks", "Password", "SellerAddress",
    "PurchaserAddress", "InvoiceCode", "InvoiceCodeConfirm",
    "CommodityUnit", "Payee", "PurchaserRegisterNum", "CommodityPrice",
    "NoteDrawer", "AmountInWords", "AmountInFigures", "TotalTax",
    "InvoiceType", "SellerRegisterNum", "CommodityName", "CommodityType",
    "CommodityPlateNum", "CommodityVehicleType", "CommodityStartDate",
    "CommodityEndDate", "OnlinePay",
];

pub const LIST_FIELDS: &[&str] = &[
    "CommodityTaxRate", "CommodityAmount", "CommodityTax", "CommodityNum",
    "CommodityUnit", "CommodityPrice", "CommodityName", "CommodityType",
    "CommodityPlateNum", "CommodityVehicleType", "CommodityStartDate",
    "CommodityEndDate",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowItem {
    pub word: String,
    pub row: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardResult {
    pub log_id: String,
    pub words_result_num: u64,
    pub words_result: HashMap<String, serde_json::Value>,
}

// ============================================================
// Utility functions
// ============================================================

/// Decode one HTML entity body (without `&` and `;`). Numeric references and
/// the entities that actually appear in invoice OCR output are covered.
fn decode_entity(entity: &str) -> Option<String> {
    if let Some(num) = entity.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            num.parse::<u32>().ok()
        };
        return code.and_then(char::from_u32).map(|c| c.to_string());
    }
    let decoded = match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "yen" => "¥",
        "times" => "×",
        "middot" => "·",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "copy" => "©",
        _ => return None,
    };
    Some(decoded.to_string())
}

/// Equivalent of Python's `html.unescape` for the entities we care about.
fn unescape_html_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // Only treat it as an entity if `;` appears within a short window.
        let window_end = rest
            .char_indices()
            .nth(12)
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let decoded = rest[..window_end]
            .find(';')
            .and_then(|semi| decode_entity(&rest[1..semi]).map(|s| (s, semi + 1)));
        match decoded {
            Some((s, consumed)) => {
                out.push_str(&s);
                rest = &rest[consumed..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn normalize_text(value: &str) -> String {
    // Python's `_normalize_text` unescapes HTML entities first.
    let mut s = unescape_html_entities(value);
    // Unescape common literal newline sequences emitted by the OCR API.
    s = s.replace("\\r\\n", "\n").replace("\\n", "\n");
    s = s.replace("\\r", "\n");
    // Clean OCR artifacts: PaddleOCR-VL may render invoice asterisks (*)
    // as LaTeX-like formulas, e.g. "$ ^{{*}} $", "$^{*}$", "$*$"
    // Pattern: $ followed by up to 20 non-$ chars, then $ → replace with *
    s = Regex::new(r#"\$[^$]{0,20}\$"#)
        .unwrap()
        .replace_all(&s, "*")
        .to_string();
    // Drop useless leading watermark symbols, e.g. "ⓧ 壹仟肆佰玖拾柒圆贰角伍分"
    s = s
        .trim_start_matches(['ⓧ', 'Ⓧ', '✖', '✕', '❌', '✘', 'Ⓟ'])
        .trim()
        .to_string();
    s.trim().to_string()
}

fn normalize_label(value: &str) -> String {
    let normalized = normalize_text(value);
    let re = Regex::new(r"[\s:：,，、()（）]").unwrap();
    re.replace_all(&normalized, "").to_string()
}

fn join_text(parts: &[&str]) -> String {
    let normalized: Vec<String> = parts
        .iter()
        .map(|p| normalize_text(p))
        .filter(|p| !p.is_empty())
        .collect();
    if normalized.is_empty() {
        return String::new();
    }
    let re = Regex::new(r"[ \t\f\v]+").unwrap();
    re.replace_all(&normalized.join(" "), " ").trim().to_string()
}

fn clean_number(value: &str) -> String {
    normalize_text(value)
        .replace(",", "")
        .replace("，", "")
        .replace("¥", "")
        .replace("￥", "")
        .replace("RMB", "")
        .trim()
        .to_string()
}

fn money_tokens(value: &str) -> Vec<String> {
    let re = Regex::new(r"(?:¥|￥|RMB)?\s*[+-]?(?:\d{1,3}(?:[,，]\d{3})+|\d+)(?:\.\d+)?").unwrap();
    re.find_iter(&normalize_text(value))
        .map(|m| clean_number(m.as_str()))
        .collect()
}

fn make_row_list(values: &[String]) -> Vec<RowItem> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| RowItem {
            word: v.clone(),
            row: (i + 1).to_string(),
        })
        .collect()
}

// ============================================================
// Header mapping
// ============================================================

struct HeaderAliases;

impl HeaderAliases {
    fn matches(label: &str, field: &str) -> bool {
        match field {
            "name" => {
                matches!(
                    label,
                    "项目名称" | "货物或应税劳务服务名称" | "货物或应税劳务名称" | "货物或应税劳务、服务名称"
                )
            }
            "spec" => label == "规格型号",
            "unit" => label == "单位",
            "quantity" => label == "数量",
            "price" => label == "单价",
            "amount" => label == "金额",
            "tax_rate" => matches!(label, "税率" | "税率征收率" | "税率/征收率"),
            "tax" => label == "税额",
            _ => false,
        }
    }
}

fn header_mapping(row: &[Cell]) -> Option<HashMap<usize, String>> {
    let fields = ["name", "spec", "unit", "quantity", "price", "amount", "tax_rate", "tax"];
    let mut mapping: HashMap<usize, String> = HashMap::new();
    let mut hits: std::collections::HashSet<String> = std::collections::HashSet::new();

    for cell in row {
        let label = normalize_label(&cell.text);
        for &field in &fields {
            if HeaderAliases::matches(&label, field) {
                hits.insert(field.to_string());
                for col in cell.col_start..=cell.col_end {
                    mapping.insert(col, field.to_string());
                }
                break;
            }
        }
    }

    // Need at least name, amount, tax and 4+ fields
    if hits.contains("name") && hits.contains("amount") && hits.contains("tax") && hits.len() >= 4
    {
        Some(mapping)
    } else {
        None
    }
}

fn find_table_and_header(
    markdown_text: &str,
) -> Option<(Vec<Vec<Cell>>, usize, HashMap<usize, String>)> {
    let mut best: Option<(Vec<Vec<Cell>>, usize, HashMap<usize, String>)> = None;

    for table_rows in parse_structured_tables(markdown_text) {
        for (row_idx, row) in table_rows.iter().enumerate() {
            if let Some(mapping) = header_mapping(row) {
                let is_better = match &best {
                    Some((_, _, existing)) => mapping.len() > existing.len(),
                    None => true,
                };
                if is_better {
                    best = Some((table_rows.clone(), row_idx, mapping));
                }
            }
        }
    }

    best
}

// ============================================================
// Field extraction helpers
// ============================================================

fn row_field_text(row: &[Cell], mapping: &HashMap<usize, String>, field: &str) -> String {
    let columns: Vec<usize> = mapping
        .iter()
        .filter(|(_, v)| v.as_str() == field)
        .map(|(&k, _)| k)
        .collect();
    if columns.is_empty() {
        return String::new();
    }
    let first_col = *columns.iter().min().unwrap();
    let last_col = *columns.iter().max().unwrap();

    let values: Vec<&str> = row
        .iter()
        .filter(|c| c.col_end >= first_col && c.col_start <= last_col)
        .map(|c| c.text.as_str())
        .collect();
    join_text(&values)
}

fn split_labeled_lines(cell_text: &str) -> HashMap<String, String> {
    let normalized = normalize_text(cell_text);
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut current: Option<String> = None;

    // Same alias sets as Python's `_split_labeled_lines`. Labels are
    // normalized first, so e.g. "地 址、电 话" arrives here as "地址电话".
    let aliases_name = ["名称", "名"];
    let aliases_reg = [
        "纳税人识别号",
        "统一社会信用代码",
        "统一社会信用代码/纳税人识别号",
    ];
    let aliases_address = ["地址电话", "地址", "电话"];
    let aliases_bank = ["开户行及账号", "开户银行及账号", "开户行账号"];

    for line in &lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let colon_pos = line.find(|c: char| c == '：' || c == ':');
        let label = if let Some(pos) = colon_pos {
            normalize_label(&line[..pos])
        } else {
            normalize_label(line)
        };

        let key = if aliases_name.contains(&label.as_str()) {
            Some("name")
        } else if aliases_reg.contains(&label.as_str()) {
            Some("reg")
        } else if aliases_address.contains(&label.as_str()) {
            Some("address")
        } else if aliases_bank.contains(&label.as_str()) {
            Some("bank")
        } else {
            None
        };

        if let Some(k) = key {
            current = Some(k.to_string());
            if let Some(pos) = colon_pos {
                // Skip the colon character (may be multi-byte `：` or single-byte `:`)
                let after_colon: String = line[pos..].chars().skip(1).collect();
                fields.insert(k.to_string(), after_colon.trim().to_string());
            } else {
                fields.entry(k.to_string()).or_default();
            }
        } else if let Some(ref cur) = current {
            let existing = fields.get(cur).cloned().unwrap_or_default();
            fields.insert(cur.clone(), join_text(&[&existing, line]));
        }
    }

    fields
}

fn bank_value(value: &str) -> String {
    let compact = Regex::new(r"\s+")
        .unwrap()
        .replace_all(&normalize_text(value), "")
        .to_string();
    if let Some(m) = Regex::new(r"(\d{8,32})$")
        .unwrap()
        .find(&compact)
    {
        format!("{}{}", &compact[..m.start()], m.as_str())
    } else {
        compact
    }
}

fn party_from_cell(cell_text: &str) -> HashMap<String, String> {
    let fields = split_labeled_lines(cell_text);
    let mut result = HashMap::new();

    if let Some(name) = fields.get("name") {
        let cleaned = name
            .trim_matches(|c| c == '，' || c == ',' || c == '；' || c == ';' || c == '。' || c == '.')
            .to_string();
        result.insert("name".to_string(), cleaned);
    }
    if let Some(reg) = fields.get("reg") {
        let cleaned = Regex::new(r"\s+")
            .unwrap()
            .replace_all(reg, "")
            .to_string();
        result.insert("reg".to_string(), cleaned);
    }
    if let Some(address) = fields.get("address") {
        result.insert("address".to_string(), address.clone());
    }
    if let Some(bank) = fields.get("bank") {
        result.insert("bank".to_string(), bank_value(bank));
    }

    result
}

// ============================================================
// Party and footer extraction
// ============================================================

fn is_party_label(text: &str) -> Option<&str> {
    // Python's PARTY_LABELS keys; normalize_label strips whitespace so
    // "备 注" already normalizes to "备注".
    let label = normalize_label(text);
    match label.as_str() {
        "购买方" | "购买方信息" => Some("purchaser"),
        "销售方" | "销售方信息" => Some("seller"),
        "密码区" => Some("password"),
        "备注" => Some("remarks"),
        _ => None,
    }
}

fn extract_party_and_footer(rows: &[Vec<Cell>]) -> HashMap<String, String> {
    let mut result = HashMap::new();

    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(role) = is_party_label(&cell.text) {
                if index + 1 >= row.len() {
                    continue;
                }
                match role {
                    "purchaser" => {
                        let info = party_from_cell(&row[index + 1].text);
                        let field_map = [
                            ("name", "PurchaserName"),
                            ("reg", "PurchaserRegisterNum"),
                            ("address", "PurchaserAddress"),
                            ("bank", "PurchaserBank"),
                        ];
                        for (key, output_key) in &field_map {
                            if let Some(val) = info.get(*key) {
                                if !val.is_empty() {
                                    result.insert(output_key.to_string(), val.clone());
                                }
                            }
                        }
                    }
                    "seller" => {
                        let info = party_from_cell(&row[index + 1].text);
                        let field_map = [
                            ("name", "SellerName"),
                            ("reg", "SellerRegisterNum"),
                            ("address", "SellerAddress"),
                            ("bank", "SellerBank"),
                        ];
                        for (key, output_key) in &field_map {
                            if let Some(val) = info.get(*key) {
                                if !val.is_empty() {
                                    result.insert(output_key.to_string(), val.clone());
                                }
                            }
                        }
                    }
                    "password" => {
                        let val = normalize_text(&row[index + 1].text);
                        if !val.is_empty() {
                            result.insert("Password".to_string(), val);
                        }
                    }
                    "remarks" => {
                        let val = normalize_text(&row[index + 1].text);
                        if !val.is_empty() {
                            // Remarks are opaque - do not extract party info from them
                            result.insert("Remarks".to_string(), val);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    result
}

// ============================================================
// Commodity details extraction
// ============================================================

fn is_summary_label(text: &str) -> bool {
    // Python: `_normalize_label(value) in {"合计", "小计"}`. Whitespace is
    // already stripped by normalize_label, so "合 计" normalizes to "合计".
    let label = normalize_label(text);
    label == "合计" || label == "小计"
}

fn is_total_label(text: &str) -> bool {
    normalize_label(text).contains("价税合计")
}

fn detail_is_new(values: &HashMap<String, String>, current: &Option<HashMap<String, String>>) -> bool {
    // Mirror Python's `_detail_is_new` check order exactly.
    let has_any = values.values().any(|v| !v.is_empty());
    if !has_any {
        return false;
    }
    let Some(cur) = current else {
        return true;
    };
    let numeric_fields = ["quantity", "price", "amount", "tax_rate", "tax"];
    let name = values.get("name").cloned().unwrap_or_default();
    let has_numeric = numeric_fields.iter().any(|f| {
        values.get(*f).map(|v| !v.is_empty()).unwrap_or(false)
    });

    if name.is_empty() {
        return false;
    }
    // A wrapped description may put the remainder of the name and all
    // numeric cells on the next physical row. If the previous row has no
    // quantitative values yet, keep the row in the same detail record.
    let cur_has_numeric = numeric_fields.iter().any(|f| {
        cur.get(*f).map(|v| !v.is_empty()).unwrap_or(false)
    });
    if !cur_has_numeric {
        return false;
    }

    name.starts_with('*') || has_numeric
}

fn merge_detail_text(old: &str, new: &str) -> String {
    let old = normalize_text(old).replace('\n', " ");
    let new = normalize_text(new).replace('\n', " ");
    if old.is_empty() {
        return new;
    }
    if new.is_empty() || new == old {
        return old;
    }
    join_text(&[&old, &new])
}

fn merge_detail_record(
    old: &mut HashMap<String, String>,
    new: &HashMap<String, String>,
) {
    let text_fields = ["name", "spec", "unit"];
    for (field, value) in new {
        if value.is_empty() {
            continue;
        }
        if text_fields.contains(&field.as_str()) {
            let old_val = old.get(field).cloned().unwrap_or_default();
            old.insert(field.clone(), merge_detail_text(&old_val, value));
        } else if old.get(field).map(|v| v.is_empty()).unwrap_or(true) {
            old.insert(field.clone(), value.clone());
        }
    }
}

fn parse_details(
    rows: &[Vec<Cell>],
    header_index: usize,
    mapping: &HashMap<usize, String>,
    stop_index: Option<usize>,
) -> Vec<HashMap<String, String>> {
    let fields = ["name", "spec", "unit", "quantity", "price", "amount", "tax_rate", "tax"];
    let end = stop_index.unwrap_or(rows.len());
    let mut records: Vec<HashMap<String, String>> = Vec::new();
    let mut current: Option<HashMap<String, String>> = None;

    for row in rows.iter().take(end).skip(header_index + 1) {
        let mut values = HashMap::new();
        for &field in &fields {
            let val = row_field_text(row, mapping, field);
            let val = if field == "name" || field == "spec" || field == "unit" {
                val.replace('\n', " ").trim().to_string()
            } else {
                val
            };
            values.insert(field.to_string(), val);
        }

        let has_any = values.values().any(|v| !v.is_empty());
        if !has_any {
            continue;
        }

        if detail_is_new(&values, &current) {
            if let Some(cur) = current {
                records.push(cur);
            }
            current = Some(values);
        } else if let Some(ref mut cur) = current {
            merge_detail_record(cur, &values);
        } else {
            current = Some(values);
        }
    }

    if let Some(cur) = current {
        records.push(cur);
    }

    records
}

// ============================================================
// Summary and total extraction
// ============================================================

fn extract_summary(
    rows: &[Vec<Cell>],
    mapping: &HashMap<usize, String>,
    summary_index: Option<usize>,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let summary_idx = match summary_index {
        Some(i) => i,
        None => return result,
    };

    let mut amount_values: Vec<String> = Vec::new();
    let mut tax_values: Vec<String> = Vec::new();

    for cell in &rows[summary_idx] {
        let tokens = money_tokens(&cell.text);
        if tokens.is_empty() {
            continue;
        }

        let mut mapped_fields: std::collections::HashSet<String> = std::collections::HashSet::new();
        for col in cell.col_start..=cell.col_end {
            if let Some(f) = mapping.get(&col) {
                mapped_fields.insert(f.clone());
            }
        }

        if mapped_fields.contains("amount") && !mapped_fields.contains("tax") {
            amount_values.extend(tokens);
        } else if mapped_fields.contains("tax") && !mapped_fields.contains("amount") {
            tax_values.extend(tokens);
        } else {
            if amount_values.is_empty() && !tokens.is_empty() {
                amount_values.push(tokens[0].clone());
            }
            if tokens.len() > 1 && tax_values.is_empty() {
                tax_values.push(tokens[1].clone());
            }
        }
    }

    if let Some(val) = amount_values.first() {
        result.insert("TotalAmount".to_string(), val.clone());
    }
    if let Some(val) = tax_values.first() {
        result.insert("TotalTax".to_string(), val.clone());
    }

    result
}

fn extract_total(rows: &[Vec<Cell>], total_index: Option<usize>) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let total_idx = match total_index {
        Some(i) => i,
        None => return result,
    };

    if rows[total_idx].len() < 2 {
        return result;
    }

    let texts: Vec<&str> = rows[total_idx][1..].iter().map(|c| c.text.as_str()).collect();
    let combined = normalize_text(&texts.join(" "));

    let marker_re = Regex::new(r"[（(]\s*小写\s*[）)]").unwrap();
    let joined_fallback = if texts.len() > 1 {
        texts[1..].join(" ")
    } else {
        String::new()
    };
    let (words_part, numeric_part) = if let Some(m) = marker_re.find(&combined) {
        (&combined[..m.start()], &combined[m.end()..])
    } else {
        (
            combined.as_str(),
            if texts.len() > 1 {
                joined_fallback.as_str()
            } else {
                combined.as_str()
            },
        )
    };

    // Clean words
    let decor_re = Regex::new(r"^[⊗·•]+").unwrap();
    let words = decor_re.replace(words_part, "").trim().to_string();
    let words = Regex::new(r"价税合计[（(]大写[）)]")
        .unwrap()
        .replace(&words, "")
        .trim()
        .to_string();

    if !words.is_empty() {
        result.insert("AmountInWords".to_string(), words);
    }

    let numeric_tokens = money_tokens(numeric_part);
    if let Some(val) = numeric_tokens.last() {
        result.insert("AmountInFigures".to_string(), val.clone());
    }

    result
}

// ============================================================
// Metadata extraction
// ============================================================

fn extract_non_table_text(markdown_text: &str, blocks: &[crate::ocr_client::Block]) -> String {
    // Try blocks first
    if !blocks.is_empty() {
        let values: Vec<&str> = blocks
            .iter()
            .filter(|b| b.block_label != "table" && !b.block_content.is_empty())
            .map(|b| b.block_content.as_str())
            .collect();
        if !values.is_empty() {
            return normalize_text(&values.join("\n"));
        }
    }

    // Fall back to removing tables wholesale (content included), mirroring
    // Python's `soup.find_all("table"); table.decompose()`. Stripping only
    // the tags would leak cell text (amounts, remarks, ...) into the
    // metadata text and let the digit fallbacks pick up unit prices etc.
    let table_re = Regex::new(r"(?is)<table\b.*?</table>").unwrap();
    let without_tables = table_re.replace_all(markdown_text, " ");
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let stripped = tag_re.replace_all(&without_tables, " ");
    // HTML entities are decoded by normalize_text.
    normalize_text(&stripped)
}

fn extract_invoice_numbers(text: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();

    // InvoiceNumDigit
    if let Some(m) =
        Regex::new(r"(?:发票号码数字|数电发票号码)\s*[：:]?\s*(\d{4,32})")
            .unwrap()
            .find(text)
    {
        let caps = Regex::new(r"(?:发票号码数字|数电发票号码)\s*[：:]?\s*(\d{4,32})")
            .unwrap()
            .captures(m.as_str())
            .unwrap();
        result.insert("InvoiceNumDigit".to_string(), caps[1].to_string());
    }

    // InvoiceNum
    if let Some(caps) = Regex::new(r"发票号码\s*[：:]?\s*(\d{6,24})")
        .unwrap()
        .captures(text)
    {
        result.insert("InvoiceNum".to_string(), caps[1].to_string());
    }

    // InvoiceCode
    if let Some(caps) = Regex::new(r"发票代码\s*[：:]?\s*(\d{10,12})")
        .unwrap()
        .captures(text)
    {
        result.insert("InvoiceCode".to_string(), caps[1].to_string());
    }

    // Combined No: 18 digits
    if let Some(caps) = Regex::new(r"(?i)\bNo\s*[:：]?\s*(\d{18})\b")
        .unwrap()
        .captures(text)
    {
        let digits = &caps[1];
        result
            .entry("InvoiceNum".to_string())
            .or_insert_with(|| digits[..8].to_string());
        result
            .entry("InvoiceCode".to_string())
            .or_insert_with(|| digits[8..].to_string());
    }

    // Fallbacks use maximal digit runs. The Rust regex crate has no
    // lookarounds, so `(?<!\d)(\d{8})(?!\d)` is emulated by keeping only
    // runs whose length is exactly 8 (likewise 10-12 for InvoiceCode).
    // `\b` boundaries are NOT equivalent: they reject letter-adjacent runs
    // that Python accepts and accept `.`-adjacent decimal fragments the
    // same way, so match Python semantics exactly instead.
    let digit_runs: Vec<String> = Regex::new(r"\d+")
        .unwrap()
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect();

    if !result.contains_key("InvoiceNum") {
        let candidates: Vec<&String> = digit_runs.iter().filter(|r| r.len() == 8).collect();
        if let Some(last) = candidates.last() {
            result.insert("InvoiceNum".to_string(), (*last).clone());
        }
    }

    if !result.contains_key("InvoiceCode") {
        let candidates: Vec<&String> = digit_runs
            .iter()
            .filter(|r| (10..=12).contains(&r.len()))
            .collect();
        if let Some(first) = candidates.first() {
            result.insert("InvoiceCode".to_string(), (*first).clone());
        }
    }

    result
}

fn extract_metadata(
    markdown_text: &str,
    blocks: &[crate::ocr_client::Block],
) -> HashMap<String, String> {
    let text = extract_non_table_text(markdown_text, blocks);
    let mut result = extract_invoice_numbers(&text);

    // InvoiceDate
    if let Some(caps) =
        Regex::new(r"(\d{4})\s*[年./-]\s*(\d{1,2})\s*[月./-]\s*(\d{1,2})\s*日?")
            .unwrap()
            .captures(&text)
    {
        let date = format!(
            "{}年{:02}月{:02}日",
            &caps[1],
            caps[2].parse::<u32>().unwrap_or(0),
            caps[3].parse::<u32>().unwrap_or(0)
        );
        result.insert("InvoiceDate".to_string(), date);
    }

    // Checker, Payee, NoteDrawer
    for (label, field) in &[("复核", "Checker"), ("收款人", "Payee"), ("开票人", "NoteDrawer")] {
        let pattern = format!(r"{}\s*[：:]\s*([^\n\r]+)", label);
        if let Some(caps) = Regex::new(&pattern).unwrap().captures(&text) {
            let val = caps[1].trim().to_string();
            if !val.is_empty() {
                result.insert(field.to_string(), val);
            }
        }
    }

    // SheetNum
    if let Some(m) = Regex::new(r"第[一二三四五六七八九十百0-9]+联")
        .unwrap()
        .find(&text)
    {
        result.insert("SheetNum".to_string(), m.as_str().to_string());
    }

    // Province
    if let Some(caps) = Regex::new(r"([\u{4e00}-\u{9fa5}]{2,3})(?:省|市)税务局")
        .unwrap()
        .captures(&text)
    {
        result.insert("Province".to_string(), caps[1].to_string());
    } else if let Some(caps) =
        Regex::new(r"([\u{4e00}-\u{9fa5}]{2,3})(?:增值税|电子发票)")
            .unwrap()
            .captures(&text)
    {
        result.insert("Province".to_string(), caps[1].to_string());
    }

    // InvoiceType
    if text.contains("电子发票") && text.contains("专用") {
        result.insert("InvoiceType".to_string(), "专用发票".to_string());
    } else if text.contains("专用发票") {
        result.insert("InvoiceType".to_string(), "专用发票".to_string());
    } else if text.contains("普通发票") {
        result.insert("InvoiceType".to_string(), "普通发票".to_string());
    }

    result
}

// ============================================================
// Main entry point
// ============================================================

pub fn parse_invoice_from_markdown(
    markdown_text: &str,
    blocks: &[crate::ocr_client::Block],
) -> HashMap<String, String> {
    let mut result = extract_metadata(markdown_text, blocks);

    let table_info = find_table_and_header(markdown_text);
    if let Some((rows, header_index, mapping)) = table_info {
        // Extract party info and footer
        let party = extract_party_and_footer(&rows);
        for (k, v) in party {
            result.entry(k).or_insert(v);
        }

        // Find summary and total rows
        let mut summary_index: Option<usize> = None;
        let mut total_index: Option<usize> = None;

        for idx in (header_index + 1)..rows.len() {
            let first = rows[idx]
                .first()
                .map(|c| c.text.as_str())
                .unwrap_or("");
            if summary_index.is_none() && is_summary_label(first) {
                summary_index = Some(idx);
            }
            if is_total_label(first) {
                total_index = Some(idx);
                break;
            }
        }

        let detail_stop = summary_index.or(total_index);

        // Parse commodity details
        let details = parse_details(&rows, header_index, &mapping, detail_stop);
        if !details.is_empty() {
            let field_outputs = [
                ("amount", "CommodityAmount"),
                ("tax", "CommodityTax"),
                ("tax_rate", "CommodityTaxRate"),
                ("quantity", "CommodityNum"),
                ("price", "CommodityPrice"),
                ("unit", "CommodityUnit"),
                ("spec", "CommodityType"),
            ];
            for (field, output) in &field_outputs {
                let values: Vec<String> = details
                    .iter()
                    .map(|d| d.get(*field).cloned().unwrap_or_default())
                    .collect();
                let row_list = make_row_list(&values);
                result.insert(
                    output.to_string(),
                    serde_json::to_string(&row_list).unwrap_or_default(),
                );
            }

            let names: Vec<String> = details
                .iter()
                .map(|d| {
                    let name = d.get("name").cloned().unwrap_or_default();
                    normalize_text(&name).replace('\n', " ")
                })
                .collect();
            let name_list = make_row_list(&names);
            result.insert(
                "CommodityName".to_string(),
                serde_json::to_string(&name_list).unwrap_or_default(),
            );
        }

        // Summary
        let summary = extract_summary(&rows, &mapping, summary_index);
        for (k, v) in summary {
            result.entry(k).or_insert(v);
        }

        // Total
        let total = extract_total(&rows, total_index);
        for (k, v) in total {
            result.entry(k).or_insert(v);
        }
    }

    result
}

/// Build the standard output structure matching Python's make_standard_result.
pub fn make_standard_result(sparse: &HashMap<String, String>) -> StandardResult {
    let mut words_result: HashMap<String, serde_json::Value> = HashMap::new();

    for &field in STANDARD_FIELDS {
        if LIST_FIELDS.contains(&field) {
            // Parse list fields from JSON string
            let val = sparse.get(field).cloned().unwrap_or_default();
            if val.is_empty() {
                words_result.insert(field.to_string(), serde_json::json!([]));
            } else if let Ok(list) = serde_json::from_str::<Vec<RowItem>>(&val) {
                words_result.insert(field.to_string(), serde_json::to_value(list).unwrap());
            } else {
                words_result.insert(field.to_string(), serde_json::json!([]));
            }
        } else {
            let val = sparse.get(field).cloned().unwrap_or_default();
            words_result.insert(field.to_string(), serde_json::json!(val));
        }
    }

    // Copy confirm fields
    let invoice_num = words_result
        .get("InvoiceNum")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let invoice_code = words_result
        .get("InvoiceCode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    words_result.insert("InvoiceNumConfirm".to_string(), serde_json::json!(invoice_num));
    words_result.insert("InvoiceCodeConfirm".to_string(), serde_json::json!(invoice_code));

    // Defaults
    let service_type = words_result
        .get("ServiceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if service_type.is_empty() {
        words_result.insert("ServiceType".to_string(), serde_json::json!("其他"));
    }
    let agent = words_result
        .get("Agent")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if agent.is_empty() {
        words_result.insert("Agent".to_string(), serde_json::json!("否"));
    }

    // Count
    let mut count: u64 = 0;
    for (_, v) in &words_result {
        if let Some(arr) = v.as_array() {
            count += arr.len() as u64;
        } else if let Some(s) = v.as_str() {
            if !s.is_empty() {
                count += 1;
            }
        } else if !v.is_null() {
            count += 1;
        }
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let log_id = format!("{}", now)[..19.min(format!("{}", now).len())].to_string();

    StandardResult {
        log_id,
        words_result_num: count,
        words_result,
    }
}

/// Merge multiple sparse results (for multi-page documents / multi-file invoices).
/// 列表字段按 (row, word) 去重后合并，避免同一发票多图识别出重复明细。
pub fn merge_sparse_results(
    target: &mut HashMap<String, String>,
    source: HashMap<String, String>,
) {
    for (field, value) in source {
        if value.is_empty() {
            continue;
        }
        // Check if this is a list field (JSON array)
        if LIST_FIELDS.contains(&field.as_str()) {
            let existing = target.get(&field).cloned().unwrap_or_default();
            let mut existing_list: Vec<RowItem> = if existing.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&existing).unwrap_or_default()
            };
            let new_list: Vec<RowItem> = serde_json::from_str(&value).unwrap_or_default();
            for item in new_list {
                let dup = existing_list.iter().any(|e| {
                    e.row == item.row && e.word.trim() == item.word.trim() && !item.word.is_empty()
                });
                if !dup {
                    existing_list.push(item);
                }
            }
            target.insert(
                field,
                serde_json::to_string(&existing_list).unwrap_or_default(),
            );
        } else if !target.contains_key(&field) || target[&field].is_empty() {
            target.insert(field, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_text_cleans_ocr_formula_artifacts() {
        assert_eq!(
            normalize_text("$ ^{{*}} $金融服务 $ ^{{*}} $企业贷款"),
            "*金融服务 *企业贷款"
        );
        assert_eq!(normalize_text("$^{*}$建筑服务$^{*}$工程款"), "*建筑服务*工程款");
        assert_eq!(normalize_text("$*$货物$*$劳务"), "*货物*劳务");
        assert_eq!(normalize_text("普通文本"), "普通文本");
        assert_eq!(normalize_text("*建筑服务*工程款"), "*建筑服务*工程款");
        assert_eq!(
            normalize_text("ⓧ 壹仟肆佰玖拾柒圆贰角伍分"),
            "壹仟肆佰玖拾柒圆贰角伍分"
        );
        assert_eq!(normalize_text("✖ 壹佰元整"), "壹佰元整");
    }

    // ============================================================
    // Regression tests using the raw API responses kept in ../../output.
    // They run the full app flow: response JSON -> pages (markdown +
    // blocks) -> per-page parse -> merge -> standard result.
    // ============================================================

    fn output_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../output")
    }

    fn run_full_flow(path: &std::path::Path) -> Option<StandardResult> {
        let raw = std::fs::read_to_string(path).ok()?;
        let client = crate::ocr_client::OcrClient::new(None, None);
        let pages = client.parse_jsonl_response(&raw).ok()?;
        let mut sparse: HashMap<String, String> = HashMap::new();
        for page in &pages {
            let page_result = parse_invoice_from_markdown(&page.markdown_text, &page.blocks);
            merge_sparse_results(&mut sparse, page_result);
        }
        Some(make_standard_result(&sparse))
    }

    fn scalar<'a>(words: &'a HashMap<String, serde_json::Value>, key: &str) -> &'a str {
        words.get(key).and_then(|v| v.as_str()).unwrap_or("")
    }

    fn row_words(words: &HashMap<String, serde_json::Value>, key: &str) -> Vec<String> {
        words
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.get("word")
                            .and_then(|w| w.as_str())
                            .unwrap_or("")
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn test_full_flow_paper_invoice() {
        let path = output_root().join("ocr_api_response.json");
        let Some(result) = run_full_flow(&path) else {
            eprintln!("fixture missing, skipped: {}", path.display());
            return;
        };
        let words = &result.words_result;

        assert_eq!(scalar(words, "InvoiceNum"), "10770713");
        assert_eq!(scalar(words, "InvoiceCode"), "1200213130");
        assert_eq!(scalar(words, "InvoiceNumConfirm"), "10770713");
        assert_eq!(scalar(words, "InvoiceCodeConfirm"), "1200213130");
        assert_eq!(scalar(words, "InvoiceDate"), "2022年01月25日");
        assert_eq!(scalar(words, "InvoiceType"), "专用发票");
        assert_eq!(scalar(words, "NoteDrawer"), "胡月笙");
        assert_eq!(scalar(words, "Payee"), "胡静");
        assert_eq!(scalar(words, "Checker"), "胡锦源");
        assert_eq!(scalar(words, "Province"), "天津");
        assert_eq!(scalar(words, "SheetNum"), "第一联");

        assert_eq!(scalar(words, "PurchaserName"), "中国建筑第八工程局有限公司");
        assert_eq!(scalar(words, "PurchaserRegisterNum"), "9131000063126503X1");
        assert_eq!(
            scalar(words, "PurchaserAddress"),
            "中国（上海）自由贸易试验区世纪大道1568号27层 021-61691997"
        );
        assert_eq!(
            scalar(words, "PurchaserBank"),
            "中国建设银行股份有限公司上海六里支行31001522917055435820"
        );
        assert_eq!(scalar(words, "SellerName"), "天津相辉建筑工程有限公司");
        assert_eq!(scalar(words, "SellerRegisterNum"), "91120113MA05TWJK13");
        assert!(scalar(words, "Password").starts_with("03>57/<-5-132-"));
        assert!(scalar(words, "Remarks").contains("雄安国际酒店施工总承包"));

        // Details: one row; the *category* prefix stays in the name and
        // does not leak into CommodityType (spec column is empty).
        assert_eq!(row_words(words, "CommodityName"), vec!["*建筑服务*工程款"]);
        assert_eq!(row_words(words, "CommodityAmount"), vec!["97087.38"]);
        assert_eq!(row_words(words, "CommodityTax"), vec!["2912.62"]);
        assert_eq!(row_words(words, "CommodityTaxRate"), vec!["3%"]);
        assert_eq!(row_words(words, "CommodityNum"), vec![""]);
        assert_eq!(row_words(words, "CommodityPrice"), vec![""]);
        assert_eq!(row_words(words, "CommodityUnit"), vec![""]);
        assert_eq!(row_words(words, "CommodityType"), vec![""]);

        assert_eq!(scalar(words, "TotalAmount"), "97087.38");
        assert_eq!(scalar(words, "TotalTax"), "2912.62");
        assert_eq!(scalar(words, "AmountInWords"), "壹拾万圆整");
        assert_eq!(scalar(words, "AmountInFigures"), "100000.00");
        assert_eq!(scalar(words, "ServiceType"), "其他");
        assert_eq!(scalar(words, "Agent"), "否");
    }

    #[test]
    fn test_full_flow_electronic_15_items() {
        let path = output_root().join("ocr_api_response-1.json");
        let Some(result) = run_full_flow(&path) else {
            eprintln!("fixture missing, skipped: {}", path.display());
            return;
        };
        let words = &result.words_result;

        assert_eq!(scalar(words, "InvoiceNum"), "25122000000088522395");
        assert_eq!(scalar(words, "InvoiceNumConfirm"), "25122000000088522395");
        // Electronic invoices have no 10-12 digit code anchor; the fallback
        // must not guess one from table digits such as unit prices.
        assert_eq!(scalar(words, "InvoiceCode"), "");
        assert_eq!(scalar(words, "InvoiceDate"), "2025年11月28日");

        let names = row_words(words, "CommodityName");
        assert_eq!(names.len(), 15);
        assert_eq!(names[0], "*黑色金属冶炼压延品*友发 热镀锌钢管");
        assert_eq!(names[14], "*塑料制品*挤塑板");

        let amounts = row_words(words, "CommodityAmount");
        assert_eq!(amounts.len(), 15);
        assert_eq!(amounts[0], "11136.28");
        assert_eq!(amounts[14], "5601.47");

        let taxes = row_words(words, "CommodityTax");
        assert_eq!(taxes[0], "1447.72");
        assert_eq!(taxes[14], "728.19");

        let rates = row_words(words, "CommodityTaxRate");
        assert!(rates.iter().all(|r| r == "13%"));

        let quantities = row_words(words, "CommodityNum");
        assert_eq!(quantities[0], "110");
        assert_eq!(quantities[13], "185.76");
        assert_eq!(quantities[14], "25.5744");

        let specs = row_words(words, "CommodityType");
        assert_eq!(specs[0], "DN32");
        assert_eq!(specs[5], "DN32/J11W-16T/PH16");
        assert_eq!(specs[14], "600*1200*0.03");

        let units = row_words(words, "CommodityUnit");
        assert_eq!(units[0], "根");
        assert_eq!(units[7], "套");
        assert_eq!(units[14], "立方");

        assert_eq!(scalar(words, "TotalAmount"), "200485.32");
        assert_eq!(scalar(words, "TotalTax"), "26063.11");
        assert_eq!(scalar(words, "AmountInWords"), "贰拾贰万陆仟伍佰肆拾捌圆肆角叁分");
        assert_eq!(scalar(words, "AmountInFigures"), "226548.43");

        assert_eq!(scalar(words, "PurchaserName"), "天津杰作建筑工程有限公司");
        assert_eq!(scalar(words, "PurchaserRegisterNum"), "91120102MA05P6J80G");
        assert_eq!(scalar(words, "SellerName"), "天津德园商贸有限公司");
        assert_eq!(scalar(words, "SellerRegisterNum"), "91120102MA0695J06R");
        assert_eq!(scalar(words, "NoteDrawer"), "朱德王");
        assert!(scalar(words, "Remarks").contains("购方开户银行"));
        // Opaque remarks must not leak into party fields.
        assert_eq!(scalar(words, "PurchaserBank"), "");
        assert_eq!(scalar(words, "SellerBank"), "");
    }

    #[test]
    fn test_full_flow_electronic_wrapped_detail() {
        let path = output_root().join(
            "25122000000091445401电子专票960.5元2025-12-8_01/recognized/\
             25122000000091445401电子专票960.5元2025-12-8_01_ocr_api_response.json",
        );
        let Some(result) = run_full_flow(&path) else {
            eprintln!("fixture missing, skipped: {}", path.display());
            return;
        };
        let words = &result.words_result;

        assert_eq!(scalar(words, "InvoiceNum"), "25122000000091445401");
        // The commodity name is split across two physical rows by OCR and
        // must be merged into a single detail record.
        assert_eq!(
            row_words(words, "CommodityName"),
            vec!["*保险服务*机动车交通事故 故强制险（2008款）"]
        );
        assert_eq!(
            row_words(words, "CommodityType"),
            vec!["机动车交通事故 强制险（2008款）"]
        );
        assert_eq!(row_words(words, "CommodityNum"), vec!["1"]);
        assert_eq!(row_words(words, "CommodityPrice"), vec!["906.13"]);
        assert_eq!(row_words(words, "CommodityAmount"), vec!["906.13"]);
        assert_eq!(row_words(words, "CommodityTaxRate"), vec!["6%"]);
        assert_eq!(row_words(words, "CommodityTax"), vec!["54.37"]);
        assert_eq!(row_words(words, "CommodityUnit"), vec!["份"]);

        assert_eq!(scalar(words, "TotalAmount"), "906.13");
        assert_eq!(scalar(words, "TotalTax"), "54.37");
        assert_eq!(scalar(words, "AmountInWords"), "玖佰陆拾圆伍角整");
        assert_eq!(scalar(words, "AmountInFigures"), "960.50");
        assert!(scalar(words, "Remarks").contains("保单号"));
    }

    #[test]
    fn test_summary_label_accepts_xiaoji() {
        assert!(is_summary_label("小计"));
        assert!(is_summary_label("小 计"));
        assert!(is_summary_label("合 计"));
        assert!(!is_summary_label("价税合计（大写）"));
    }

    #[test]
    fn test_details_stop_at_xiaoji() {
        // Python stops detail parsing at the first 合计/小计 row.
        let html = r#"<table>
<tr><td>项目名称</td><td>规格型号</td><td>单位</td><td>数量</td><td>单价</td><td>金额</td><td>税率</td><td>税额</td></tr>
<tr><td>*建筑服务*工程款</td><td></td><td></td><td></td><td></td><td>100.00</td><td>3%</td><td>3.00</td></tr>
<tr><td>小计</td><td></td><td></td><td></td><td></td><td>100.00</td><td></td><td>3.00</td></tr>
<tr><td>*运输服务*运费</td><td></td><td></td><td></td><td></td><td>50.00</td><td>9%</td><td>4.50</td></tr>
</table>"#;
        let sparse = parse_invoice_from_markdown(html, &[]);
        let standard = make_standard_result(&sparse);
        let words = &standard.words_result;
        assert_eq!(row_words(words, "CommodityName"), vec!["*建筑服务*工程款"]);
        assert_eq!(scalar(words, "TotalAmount"), "100.00");
        assert_eq!(scalar(words, "TotalTax"), "3.00");
    }

    #[test]
    fn test_metadata_not_polluted_by_table_content_without_blocks() {
        // Without layout blocks the markdown fallback must remove table
        // content entirely (Python decomposes tables too); otherwise the
        // digit/role fallbacks pick up cell text.
        let md = "<table><tr><td>项目名称</td><td>规格型号</td><td>单位</td><td>数量</td><td>单价</td><td>金额</td><td>税率</td><td>税额</td></tr>\
<tr><td>复核：表内文字</td><td></td><td></td><td></td><td></td><td>88888888</td><td>3%</td><td>1.00</td></tr></table>\n\n\
开票日期：2025年03月05日\n\n复核：外部复核人";
        let sparse = parse_invoice_from_markdown(md, &[]);
        assert_eq!(sparse.get("Checker").map(String::as_str), Some("外部复核人"));
        // Table digits must not leak into the InvoiceNum fallback.
        assert!(!sparse.contains_key("InvoiceNum"));
        assert_eq!(sparse.get("InvoiceDate").map(String::as_str), Some("2025年03月05日"));
    }

    #[test]
    fn test_digit_run_fallbacks_match_python_lookarounds() {
        // Letter-adjacent 8-digit run: Python's (?<!\d)(\d{8})(?!\d)
        // matches it, `\b...\b` would not.
        let result = extract_invoice_numbers("编号A12345678B");
        assert_eq!(result.get("InvoiceNum").map(String::as_str), Some("12345678"));
        // A 13-digit run is not a 10-12 digit invoice code.
        let result = extract_invoice_numbers("x1234567890123y");
        assert!(!result.contains_key("InvoiceCode"));
        // First 10-12 digit run wins for the code, like Python findall[0].
        let result = extract_invoice_numbers("1200213130 和 499952471171");
        assert_eq!(result.get("InvoiceCode").map(String::as_str), Some("1200213130"));
    }

    #[test]
    fn test_parse_commodity_details_from_real_ocr() {
        // Actual OCR table from ocr_api_response.json (doc_0)
        let html = r#"<table border=1 style='margin: auto; word-wrap: break-word;'>
<tr><td>购买方</td><td colspan="3">名称：测试公司\n纳税人识别号：123456</td><td>密码区</td><td colspan="3">密码内容</td></tr>
<tr><td>货物或应税劳务、服务名称</td><td>规格型号</td><td>单位</td><td>数量</td><td>单价</td><td>金额</td><td>税率</td><td>税额</td></tr>
<tr><td>*建筑服务*工程款</td><td></td><td></td><td></td><td></td><td>97087.38</td><td>3%</td><td>2912.62</td></tr>
<tr><td>合 计</td><td></td><td></td><td></td><td></td><td>97087.38</td><td></td><td>2912.62</td></tr>
<tr><td>价税合计（大写）</td><td colspan="7">壹拾万圆整 (小写) ¥100000.00</td></tr>
</table>"#;

        // Verify table parsing
        let tables = crate::html_parser::parse_structured_tables(html);
        assert!(!tables.is_empty(), "Should find at least one table");
        assert_eq!(tables[0].len(), 5, "Should have 5 rows");

        // Verify header mapping
        let table_info = find_table_and_header(html);
        assert!(table_info.is_some(), "Should find table and header");
        let (rows, header_index, mapping) = table_info.as_ref().unwrap();
        assert_eq!(*header_index, 1, "Header should be row 1");
        assert!(mapping.contains_key(&0), "Mapping should have col 0 (name)");
        assert!(mapping.contains_key(&5), "Mapping should have col 5 (amount)");
        assert!(mapping.contains_key(&7), "Mapping should have col 7 (tax)");

        eprintln!("rows.len() = {}", rows.len());
        for (i, row) in rows.iter().enumerate() {
            let first_text = row.first().map(|c| c.text.as_str()).unwrap_or("");
            eprintln!("row[{}]: first={:?}, cells={}", i, first_text, row.len());
        }

        // Find summary index to use as stop_index
        let mut summary_index = None;
        for idx in (*header_index + 1)..rows.len() {
            let first = rows[idx].first().map(|c| c.text.as_str()).unwrap_or("");
            eprintln!("row[{}] first: {:?}, is_summary: {}", idx, first, is_summary_label(first));
            if is_summary_label(first) {
                summary_index = Some(idx);
                break;
            }
        }
        eprintln!("summary_index: {:?}", summary_index);

        // Verify detail parsing with correct stop_index
        let details = parse_details(rows, *header_index, mapping, summary_index);
        eprintln!("details.len() = {}", details.len());
        assert_eq!(details.len(), 1, "Should have exactly 1 commodity detail");
        assert_eq!(details[0].get("name").unwrap(), "*建筑服务*工程款");
        assert_eq!(details[0].get("amount").unwrap(), "97087.38");
        assert_eq!(details[0].get("tax").unwrap(), "2912.62");
        assert_eq!(details[0].get("tax_rate").unwrap(), "3%");
        assert_eq!(details[0].get("quantity").unwrap(), "");
        assert_eq!(details[0].get("price").unwrap(), "");
        assert_eq!(details[0].get("unit").unwrap(), "");

        // Verify full pipeline
        let sparse = parse_invoice_from_markdown(html, &[]);
        eprintln!("sparse[CommodityName] = {:?}", sparse.get("CommodityName"));
        let wr = make_standard_result(&sparse);
        let words = &wr.words_result;
        eprintln!("words[CommodityName] = {:?}", words.get("CommodityName"));

        // CommodityName is a JSON array of RowItem stored as string in sparse,
        // then parsed and stored as JSON value in words_result
        fn get_row_list(words: &HashMap<String, serde_json::Value>, key: &str) -> Vec<RowItem> {
            words.get(key)
                .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
                .unwrap_or_default()
        }

        let name_list = get_row_list(words, "CommodityName");
        assert_eq!(name_list.len(), 1, "Should have 1 commodity name");
        assert!(name_list[0].word.contains("建筑服务"), "Name: {}", name_list[0].word);

        let amount_list = get_row_list(words, "CommodityAmount");
        assert_eq!(amount_list.len(), 1);
        assert!(amount_list[0].word.contains("97087.38"), "Amount: {}", amount_list[0].word);

        let tax_list = get_row_list(words, "CommodityTax");
        assert_eq!(tax_list.len(), 1);
        assert!(tax_list[0].word.contains("2912.62"), "Tax: {}", tax_list[0].word);

        let rate_list = get_row_list(words, "CommodityTaxRate");
        assert_eq!(rate_list.len(), 1);
        assert!(rate_list[0].word.contains("3%"), "Rate: {}", rate_list[0].word);

        // Verify summary/total
        let total_amount = words.get("TotalAmount").and_then(|v| v.as_str()).unwrap_or("");
        assert!(total_amount.contains("97087.38"), "TotalAmount: {}", total_amount);
        let total_tax = words.get("TotalTax").and_then(|v| v.as_str()).unwrap_or("");
        assert!(total_tax.contains("2912.62"), "TotalTax: {}", total_tax);
    }
}
