use std::collections::HashMap;

use rust_xlsxwriter::{Color, Format, FormatBorder, Workbook, Worksheet, XlsxError};

use crate::db::{InvoiceFile, InvoiceRecord};

/// 发票详情字段分组 (组名, 字段列表[(key, 中文标签)])，与详情页展示一致。
const GROUP_FIELDS: &[(&str, &[(&str, &str)])] = &[
    (
        "基础信息",
        &[
            ("InvoiceNum", "发票号码"),
            ("InvoiceNumConfirm", "发票号码(确认)"),
            ("InvoiceNumDigit", "发票号码(数字)"),
            ("InvoiceCode", "发票代码"),
            ("InvoiceCodeConfirm", "发票代码(确认)"),
            ("InvoiceDate", "开票日期"),
            ("InvoiceType", "发票类型"),
            ("Province", "省份"),
            ("City", "城市"),
            ("SheetNum", "联次"),
            ("ServiceType", "服务类型"),
            ("OnlinePay", "线上支付"),
            ("Agent", "是否代理"),
        ],
    ),
    (
        "购买方",
        &[
            ("PurchaserName", "名称"),
            ("PurchaserRegisterNum", "纳税人识别号"),
            ("PurchaserAddress", "地址电话"),
            ("PurchaserBank", "开户行及账号"),
        ],
    ),
    (
        "销售方",
        &[
            ("SellerName", "名称"),
            ("SellerRegisterNum", "纳税人识别号"),
            ("SellerAddress", "地址电话"),
            ("SellerBank", "开户行及账号"),
        ],
    ),
    (
        "开票信息",
        &[
            ("NoteDrawer", "开票人"),
            ("Payee", "收款人"),
            ("Checker", "复核人"),
        ],
    ),
    (
        "合计",
        &[
            ("TotalAmount", "合计金额"),
            ("TotalTax", "合计税额"),
            ("AmountInWords", "价税合计(大写)"),
            ("AmountInFigures", "价税合计(小写)"),
        ],
    ),
    (
        "其他信息",
        &[
            ("Password", "密码区"),
            ("Remarks", "备注"),
        ],
    ),
];

/// 商品明细列表字段 (名称, 中文标签)。按 row 对齐展开。
const LIST_FIELDS: &[(&str, &str)] = &[
    ("CommodityName", "商品名称"),
    ("CommodityType", "规格型号"),
    ("CommodityUnit", "单位"),
    ("CommodityNum", "数量"),
    ("CommodityPrice", "单价"),
    ("CommodityAmount", "金额"),
    ("CommodityTaxRate", "税率"),
    ("CommodityTax", "税额"),
    ("CommodityPlateNum", "车牌号"),
    ("CommodityVehicleType", "车辆类型"),
    ("CommodityStartDate", "开始日期"),
    ("CommodityEndDate", "结束日期"),
];

fn scalar(words: &HashMap<String, serde_json::Value>, key: &str) -> String {
    words
        .get(key)
        .and_then(|v| v.as_str())
        .map(clean_scalar)
        .unwrap_or_default()
}

/// 过滤 OCR 遗留的无用水印符号前缀（老数据兜底）。
fn clean_scalar(value: &str) -> String {
    value
        .trim_start_matches(['ⓧ', 'Ⓧ', '✖', '✕', '❌', '✘', 'Ⓟ'])
        .trim()
        .to_string()
}

/// 从 parsed_result (StandardResult) 中提取 words_result 字段映射。
fn parsed_words(parsed_result: &str) -> HashMap<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(parsed_result)
        .ok()
        .and_then(|v| v.get("words_result").cloned())
        .and_then(|w| w.as_object().cloned())
        .map(|o| o.into_iter().collect())
        .unwrap_or_default()
}

/// 解析列表字段为 row -> value 映射 (RowItem.word / row)。
fn list_row_map(words: &HashMap<String, serde_json::Value>, key: &str) -> HashMap<u64, String> {
    let mut map = HashMap::new();
    if let Some(arr) = words.get(key).and_then(|v| v.as_array()) {
        for item in arr {
            let word = item
                .get("word")
                .and_then(|w| w.as_str())
                .unwrap_or("")
                .to_string();
            let row = item
                .get("row")
                .and_then(|r| r.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            if row > 0 {
                map.insert(row, word);
            }
        }
    }
    map
}

/// 有数据的明细字段（对应 words_result 中数组非空的字段）。
/// 普通发票不包含车牌号/车辆类型/起止日期等运输字段，动态列避免空列。
fn active_list_fields(
    words: &HashMap<String, serde_json::Value>,
) -> Vec<(&'static str, &'static str)> {
    LIST_FIELDS
        .iter()
        .copied()
        .filter(|(field, _)| {
            words
                .get(*field)
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
        })
        .collect()
}

/// 单个发票的明细行数（各列表字段的最大长度）。
fn max_detail_rows(words: &HashMap<String, serde_json::Value>) -> usize {
    LIST_FIELDS
        .iter()
        .filter_map(|(field, _)| {
            words
                .get(*field)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
        })
        .max()
        .unwrap_or(0)
}

/// 发票块背景色：很淡的浅绿色，用于区分相邻发票。
const BLOCK_BG: u32 = 0xF2F8F0;

/// 块内基础格式：浅绿背景 + 黑色文字 + 边框线。
fn block_base() -> Format {
    Format::new()
        .set_background_color(Color::RGB(BLOCK_BG))
        .set_font_color(Color::Black)
        .set_border(FormatBorder::Thin)
}

struct Styles {
    title: Format,
    group: Format,
    label: Format,
    value: Format,
    value_wrap: Format,
}

impl Styles {
    fn new() -> Self {
        Self {
            title: block_base().set_bold().set_font_size(14),
            group: block_base().set_bold().set_font_size(12),
            label: block_base().set_bold(),
            value: block_base(),
            value_wrap: block_base().set_text_wrap(),
        }
    }
}

/// 在 worksheet 中写入一张发票的完整详情块（返回块结束后下一行的行号）。
/// 布局：
///   [发票标题]                       (可选, 单sheet模式显示)
///   发票代码/号码/时间/附件
///   分组标题 + 字段-值两列
///   商品明细表（表头 + 数据行）
/// 整块统一使用浅绿背景 + 黑色文字 + 边框线。
fn write_invoice_block(
    ws: &mut Worksheet,
    mut row: u32,
    record: &InvoiceRecord,
    files: &[InvoiceFile],
    words: &HashMap<String, serde_json::Value>,
    title: Option<&str>,
    styles: &Styles,
) -> Result<u32, XlsxError> {
    let Styles {
        title: st_title,
        group: st_group,
        label: st_label,
        value: st_value,
        value_wrap: st_value_wrap,
    } = styles;

    if let Some(t) = title {
        ws.merge_range(row, 0, row, 1, t, st_title)?;
        row += 1;
    }

    // 元信息
    let mut meta: Vec<(String, String)> = vec![
        ("发票代码".to_string(), record.invoice_code.clone()),
        ("发票号码".to_string(), record.invoice_num.clone()),
        ("识别时间".to_string(), record.created_at.clone()),
    ];
    let attachment_value = files
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{}. {}", i + 1, f.file_path))
        .collect::<Vec<_>>()
        .join("\n");
    meta.push((format!("附件 ({})", files.len()), attachment_value));

    for (meta_label, meta_value) in meta {
        ws.write_string_with_format(row, 0, &meta_label, st_label)?;
        ws.write_string_with_format(row, 1, &meta_value, st_value_wrap)?;
        row += 1;
    }

    // 字段分组：购买方与销售方左右并排
    let buyer = GROUP_FIELDS
        .iter()
        .find(|(t, _)| *t == "购买方")
        .map(|(_, f)| *f)
        .unwrap();
    let seller = GROUP_FIELDS
        .iter()
        .find(|(t, _)| *t == "销售方")
        .map(|(_, f)| *f)
        .unwrap();

    // 并排标题行：购买方合并 A:B，销售方合并 C:D
    ws.merge_range(row, 0, row, 1, "购买方", st_group)?;
    ws.merge_range(row, 2, row, 3, "销售方", st_group)?;
    row += 1;

    // 左右两列同时写购买方/销售方字段，保持行对齐
    for i in 0..buyer.len().max(seller.len()) {
        if let Some((key, meta_label)) = buyer.get(i) {
            ws.write_string_with_format(row, 0, *meta_label, st_label)?;
            ws.write_string_with_format(row, 1, scalar(words, key), st_value)?;
        }
        if let Some((key, meta_label)) = seller.get(i) {
            ws.write_string_with_format(row, 2, *meta_label, st_label)?;
            ws.write_string_with_format(row, 3, scalar(words, key), st_value)?;
        }
        row += 1;
    }

    // 其余分组
    for (group_title, fields) in GROUP_FIELDS {
        if *group_title == "购买方" || *group_title == "销售方" {
            continue;
        }
        ws.write_string_with_format(row, 0, *group_title, st_group)?;
        row += 1;
        for (key, meta_label) in *fields {
            let field_value = scalar(words, key);
            if field_value.is_empty() {
                continue;
            }
            ws.write_string_with_format(row, 0, *meta_label, st_label)?;
            ws.write_string_with_format(row, 1, &field_value, st_value)?;
            row += 1;
        }
    }

    // 商品明细（动态列：仅显示有数据的字段）
    ws.write_string_with_format(row, 0, "商品明细", st_group)?;
    row += 1;
    let active_fields = active_list_fields(words);

    if active_fields.is_empty() {
        ws.write_string_with_format(row, 0, "（无商品明细）", st_value)?;
        row += 1;
    } else {
        for (col, (_, f_label)) in active_fields.iter().enumerate() {
            ws.write_string_with_format(row, col as u16, *f_label, st_group)?;
        }
        row += 1;

        let detail_rows = max_detail_rows(words);
        let list_maps: Vec<HashMap<u64, String>> = active_fields
            .iter()
            .map(|(field, _)| list_row_map(words, field))
            .collect();
        for i in 0..detail_rows {
            for (col, map) in list_maps.iter().enumerate() {
                let v = map.get(&((i + 1) as u64)).cloned().unwrap_or_default();
                ws.write_string_with_format(row, col as u16, v, st_value)?;
            }
            row += 1;
        }
    }

    Ok(row)
}

/// 单 sheet：所有发票按详情格式顺序排列，发票间空一行。
fn write_single_sheet(
    workbook: &mut Workbook,
    records: &[InvoiceRecord],
    files: &[Vec<InvoiceFile>],
) -> Result<(), XlsxError> {
    let styles = Styles::new();
    let mut ws = workbook.add_worksheet();
    ws.set_name("发票详情")?;

    let mut row: u32 = 0;
    for (i, record) in records.iter().enumerate() {
        if i > 0 {
            row += 1; // 空行分隔
        }
        let words = parsed_words(&record.parsed_result);
        let title = format!("发票 {} - {}", i + 1, record.file_name);
        let files = files.get(i).map(|f| f.as_slice()).unwrap_or(&[]);
        row = write_invoice_block(
            &mut ws,
            row,
            record,
            files,
            &words,
            Some(&title),
            &styles,
        )?;
    }

    ws.set_column_width(0, 24)?;
    ws.set_column_width(1, 56)?;
    ws.set_column_width(2, 24)?;
    ws.set_column_width(3, 56)?;
    let max_active = records
        .iter()
        .map(|r| active_list_fields(&parsed_words(&r.parsed_result)).len())
        .max()
        .unwrap_or(0);
    for col in 4..(4 + max_active as u16) {
        ws.set_column_width(col, 16).ok();
    }
    Ok(())
}

/// 多 sheet：每张发票一个 sheet，内容为完整详情。
fn write_multi_sheets(
    workbook: &mut Workbook,
    records: &[InvoiceRecord],
    files: &[Vec<InvoiceFile>],
) -> Result<(), XlsxError> {
    let styles = Styles::new();
    let mut used_names: HashMap<String, usize> = HashMap::new();

    for (i, record) in records.iter().enumerate() {
        let mut ws = workbook.add_worksheet();

        // sheet 名取发票号/文件名（Excel 限制 31 字符），重名时加序号
        let base_name = if record.invoice_num.is_empty() {
            record.file_name.clone()
        } else {
            record.invoice_num.clone()
        };
        let base: String = base_name.chars().take(28).collect();
        let count = used_names.entry(base.clone()).or_insert(0);
        *count += 1;
        let name = if *count > 1 {
            format!("{}-{}", base, count)
        } else {
            base.clone()
        };
        ws.set_name(&name)?;

        let words = parsed_words(&record.parsed_result);
        let files_i = files.get(i).map(|f| f.as_slice()).unwrap_or(&[]);
        write_invoice_block(&mut ws, 0, record, files_i, &words, None, &styles)?;

        ws.set_column_width(0, 24)?;
        ws.set_column_width(1, 56)?;
        ws.set_column_width(2, 24)?;
        ws.set_column_width(3, 56)?;
        let active_count = active_list_fields(&words).len() as u16;
        for col in 4..(4 + active_count) {
            ws.set_column_width(col, 16).ok();
        }
    }
    Ok(())
}

pub fn export_invoices_excel(
    records: &[InvoiceRecord],
    files: &[Vec<InvoiceFile>],
    export_mode: &str,
    save_path: &std::path::Path,
) -> Result<(), String> {
    let mut workbook = Workbook::new();

    let result = if export_mode == "multi_sheet" {
        write_multi_sheets(&mut workbook, records, files)
    } else {
        write_single_sheet(&mut workbook, records, files)
    };
    result.map_err(|e| format!("Write workbook failed: {}", e))?;

    workbook
        .save(save_path)
        .map_err(|e| format!("Save failed: {}", e))?;
    Ok(())
}
