use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use crate::error::AppError;
use crate::invoice_extractor::RowItem;

const MODEL: &str = "PaddleOCR-VL-1.6";
const REQUEST_TIMEOUT: u64 = 60;
const POLL_INTERVAL: u64 = 5;
const MAX_POLL_COUNT: u64 = 720;

const DEFAULT_PADDLEOCR_URL: &str = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs";
const DEFAULT_SCNET_URL: &str = "https://api.scnet.cn/api/llm/v1/ocr/recognize";
const DEFAULT_SCNET_OCR_TYPE: &str = "GENERAL";

// ============================================================
// 数据结构
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrProvider {
    Paddleocr,
    Scnet,
}

impl Default for OcrProvider {
    fn default() -> Self {
        Self::Paddleocr
    }
}

impl std::fmt::Display for OcrProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paddleocr => write!(f, "PaddleOCR VL"),
            Self::Scnet => write!(f, "SCNet OCR"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageData {
    pub markdown_text: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub block_label: String,
    pub block_content: String,
}

/// 设置页保存的全部 OCR 配置（DB config 表）。
#[derive(Debug, Clone, Default)]
pub struct OcrSettings {
    pub provider: OcrProvider,
    pub paddleocr_url: String,
    pub paddleocr_token: String,
    pub scnet_url: String,
    pub scnet_token: String,
    pub scnet_ocr_type: String,
}

impl OcrSettings {
    fn from_config_json() -> serde_json::Map<String, serde_json::Value> {
        let _ = dotenvy::dotenv();
        let config_paths = [
            "config.json".to_string(),
            {
                let exe_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_default();
                exe_dir.join("config.json").to_string_lossy().to_string()
            },
        ];
        for path in &config_paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(obj) = v.as_object() {
                        log::info!("Loaded config from {}", path);
                        return obj.clone();
                    }
                }
            }
        }
        serde_json::Map::new()
    }

    /// 优先级：DB 配置 > 环境变量 > config.json > 内置默认值。
    pub fn from_db(get: impl Fn(&str) -> Option<String>) -> Self {
        let file_cfg = Self::from_config_json();
        let file_get = |key: &str| -> Option<String> {
            file_cfg.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
        };

        let provider_str = get("ocr_provider")
            .or_else(|| std::env::var("OCR_PROVIDER").ok().filter(|s| !s.is_empty()))
            .or_else(|| file_get("ocr_provider"))
            .unwrap_or_default();
        let provider = match provider_str.as_str() {
            "scnet" => OcrProvider::Scnet,
            _ => OcrProvider::Paddleocr,
        };

        let resolve = |db_key: &str, env_key: &str, file_key: &str, default: &str| -> String {
            get(db_key)
                .filter(|s| !s.is_empty())
                .or_else(|| std::env::var(env_key).ok().filter(|s| !s.is_empty()))
                .or_else(|| file_get(file_key).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| default.to_string())
        };

        Self {
            provider,
            paddleocr_url: resolve("paddleocr_url", "PADDLEOCR_API_URL", "paddleocr_url", DEFAULT_PADDLEOCR_URL),
            paddleocr_token: resolve("paddleocr_token", "PADDLEOCR_TOKEN", "paddleocr_token", ""),
            scnet_url: resolve("scnet_url", "SCNET_OCR_URL", "scnet_url", DEFAULT_SCNET_URL),
            scnet_token: resolve("scnet_token", "SCNET_OCR_TOKEN", "scnet_token", ""),
            scnet_ocr_type: resolve("scnet_ocr_type", "SCNET_OCR_TYPE", "scnet_ocr_type", DEFAULT_SCNET_OCR_TYPE),
        }
    }
}

// ============================================================
// OcrClient
// ============================================================

pub struct OcrClient {
    client: Client,
    settings: OcrSettings,
}

impl OcrClient {
    pub fn new(settings: OcrSettings) -> Self {
        log::info!(
            "OCR provider: {}, paddleocr_url: {}, scnet_url: {}",
            settings.provider, settings.paddleocr_url, settings.scnet_url
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, settings }
    }

    pub fn settings_provider(&self) -> OcrProvider {
        self.settings.provider
    }

    // ----------------------------------------------------------
    // 统一入口
    // ----------------------------------------------------------

    /// PaddleOCR：识别一张图片，返回 (pages, raw_json_text)。
    pub async fn recognize<F>(
        &self,
        file_path: &str,
        on_progress: F,
    ) -> Result<(Vec<PageData>, String), AppError>
    where
        F: Fn(String) + Send + Sync,
    {
        let job_id = self.paddleocr_submit(file_path).await?;
        self.paddleocr_poll(&job_id, on_progress).await
    }

    /// SCNet：识别一张图片，直接返回 sparse 结果列表 + raw_json_text。
    pub async fn recognize_sparse(
        &self,
        file_path: &str,
        on_progress: impl Fn(String) + Send + Sync,
    ) -> Result<(Vec<HashMap<String, String>>, String), AppError> {
        self.scnet_recognize_to_sparse(file_path, on_progress).await
    }

    // ==========================================================
    // PaddleOCR VL（异步任务模式）
    // ==========================================================

    fn paddleocr_ensure_token(&self) -> Result<(), AppError> {
        if self.settings.paddleocr_token.is_empty() {
            return Err(AppError::Config(
                "未配置 PaddleOCR Token，请到「设置 → API 配置」填写".to_string(),
            ));
        }
        Ok(())
    }

    async fn paddleocr_submit(&self, file_path: &str) -> Result<String, AppError> {
        self.paddleocr_ensure_token()?;
        log::info!("[PaddleOCR] 处理文件: {}", file_path);

        let optional_payload = serde_json::json!({
            "useDocOrientationClassify": false,
            "useDocUnwarping": false,
            "useChartRecognition": false,
            "useSealRecognition": true
        });

        let auth = format!("bearer {}", self.settings.paddleocr_token);
        let resp = if file_path.starts_with("http://") || file_path.starts_with("https://") {
            let payload = serde_json::json!({
                "fileUrl": file_path,
                "model": MODEL,
                "optionalPayload": optional_payload.to_string(),
            });
            self.client
                .post(&self.settings.paddleocr_url)
                .header("Authorization", &auth)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await?
        } else {
            let file_bytes = tokio::fs::read(file_path).await?;
            let file_name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image.jpg");
            let mime = if file_name.ends_with(".png") { "image/png" } else { "image/jpeg" };
            let part = reqwest::multipart::Part::bytes(file_bytes)
                .file_name(file_name.to_string())
                .mime_str(mime)?;
            let form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("model", MODEL.to_string())
                .text("optionalPayload", optional_payload.to_string());

            self.client
                .post(&self.settings.paddleocr_url)
                .header("Authorization", &auth)
                .multipart(form)
                .send()
                .await?
        };

        log::info!("[PaddleOCR] 响应状态: {}", resp.status());
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Ocr(format!("Submit failed {}: {}", status, body)));
        }
        let payload: serde_json::Value = resp.json().await?;
        let job_id = payload["data"]["jobId"]
            .as_str()
            .ok_or_else(|| AppError::Ocr(format!("No jobId in response: {}", payload)))?
            .to_string();
        log::info!("[PaddleOCR] 任务已提交, jobId: {}", job_id);
        Ok(job_id)
    }

    async fn paddleocr_poll<F>(
        &self,
        job_id: &str,
        on_progress: F,
    ) -> Result<(Vec<PageData>, String), AppError>
    where
        F: Fn(String) + Send + Sync,
    {
        let auth = format!("bearer {}", self.settings.paddleocr_token);
        let mut jsonl_url = String::new();

        for _ in 0..MAX_POLL_COUNT {
            let resp = self
                .client
                .get(format!("{}/{}", self.settings.paddleocr_url, job_id))
                .header("Authorization", &auth)
                .send()
                .await?;
            let payload: serde_json::Value = resp.json().await?;
            let data = &payload["data"];
            let state = data["state"].as_str().unwrap_or("");

            match state {
                "pending" => on_progress("任务排队中...".to_string()),
                "running" => {
                    let progress = &data["extractProgress"];
                    let total = progress["totalPages"].as_u64().unwrap_or(0);
                    let extracted = progress["extractedPages"].as_u64().unwrap_or(0);
                    on_progress(format!("处理中, 总页数: {}, 已提取: {}", total, extracted));
                }
                "done" => {
                    let progress = &data["extractProgress"];
                    let extracted = progress["extractedPages"].as_u64().unwrap_or(0);
                    on_progress(format!("完成! 提取页数: {}", extracted));
                    jsonl_url = data["resultUrl"]["jsonUrl"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    break;
                }
                "failed" => {
                    let msg = data["errorMsg"].as_str().unwrap_or("unknown");
                    return Err(AppError::Ocr(format!("OCR failed: {}", msg)));
                }
                other => {
                    return Err(AppError::Ocr(format!("Unknown OCR state: {}", other)));
                }
            }
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL)).await;
        }

        if jsonl_url.is_empty() {
            return Err(AppError::Ocr("OCR response has no jsonUrl".to_string()));
        }

        let json_resp = self.client.get(&jsonl_url).send().await?;
        let raw_text = json_resp.text().await?;
        let pages = self.parse_jsonl_response(&raw_text)?;
        Ok((pages, raw_text))
    }

    /// Parse PaddleOCR JSON/JSONL response into per-page data.
    pub(crate) fn parse_jsonl_response(&self, raw_text: &str) -> Result<Vec<PageData>, AppError> {
        let documents: Vec<serde_json::Value> = {
            let trimmed = raw_text.trim();
            if trimmed.is_empty() {
                vec![]
            } else if let Ok(single) = serde_json::from_str::<serde_json::Value>(trimmed) {
                vec![single]
            } else {
                trimmed
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect()
            }
        };

        let mut pages = Vec::new();
        for doc in documents {
            let result = if doc.get("result").is_some() {
                &doc["result"]
            } else {
                &doc
            };
            if let Some(layout_results) = result["layoutParsingResults"].as_array() {
                for page in layout_results {
                    let markdown = &page["markdown"];
                    let markdown_text = markdown["text"].as_str().unwrap_or("").to_string();

                    let blocks = if let Some(pruned) = page.get("prunedResult") {
                        if let Some(parsing_list) = pruned["parsing_res_list"].as_array() {
                            parsing_list
                                .iter()
                                .filter_map(|b| {
                                    Some(Block {
                                        block_label: b["block_label"].as_str()?.to_string(),
                                        block_content: b["block_content"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                    })
                                })
                                .collect()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };

                    pages.push(PageData {
                        markdown_text,
                        blocks,
                    });
                }
            }
        }
        Ok(pages)
    }

    // ==========================================================
    // SCNet OCR（同步调用，直接输出 sparse）
    // 文档: https://www.scnet.cn/ac/openapi/doc/2.0/moduleapi/api/ocr.html
    // ==========================================================

    async fn scnet_recognize_to_sparse(
        &self,
        file_path: &str,
        on_progress: impl Fn(String) + Send + Sync,
    ) -> Result<(Vec<HashMap<String, String>>, String), AppError> {
        if self.settings.scnet_token.is_empty() {
            return Err(AppError::Config(
                "未配置 SCNet OCR Token，请到「设置 → API 配置」填写".to_string(),
            ));
        }
        on_progress(format!("正在调用 SCNet OCR ({})...", self.settings.scnet_ocr_type));
        log::info!("[SCNet] 处理文件: {}, ocrType: {}", file_path, self.settings.scnet_ocr_type);

        let file_bytes = if file_path.starts_with("http://") || file_path.starts_with("https://") {
            let resp = self.client.get(file_path).send().await?;
            resp.bytes().await?.to_vec()
        } else {
            tokio::fs::read(file_path).await?
        };
        let file_name = std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image.jpg");
        let mime = if file_name.ends_with(".png") { "image/png" } else { "image/jpeg" };

        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name.to_string())
            .mime_str(mime)?;
        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("ocrType", self.settings.scnet_ocr_type.clone());

        let resp = self
            .client
            .post(&self.settings.scnet_url)
            .header("Authorization", format!("Bearer {}", self.settings.scnet_token))
            .multipart(form)
            .send()
            .await?;

        log::info!("[SCNet] 响应状态: {}", resp.status());
        let status = resp.status();
        let body_text = resp.text().await?;
        if !status.is_success() {
            return Err(AppError::Ocr(format!("SCNet submit failed {}: {}", status, body_text)));
        }

        let payload: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|e| AppError::Ocr(format!("SCNet response parse error: {}", e)))?;

        if payload["code"].as_str() != Some("0") {
            let msg = payload["msg"].as_str().unwrap_or("unknown");
            return Err(AppError::Ocr(format!(
                "SCNet OCR error: code={}, msg={}",
                payload["code"], msg
            )));
        }

        let raw_text = serde_json::to_string_pretty(&payload).unwrap_or(body_text);
        let sparse_list = scnet_response_to_sparse(&payload)?;
        on_progress(format!("SCNet 识别完成，共 {} 项", sparse_list.len()));
        Ok((sparse_list, raw_text))
    }
}

// ============================================================
// SCNet 响应 → sparse HashMap（直接映射，不经过 markdown）
// ============================================================

type Sparse = HashMap<String, String>;

/// 将 SCNet OCR API 响应转换为 sparse 结果列表。
pub(crate) fn scnet_response_to_sparse(payload: &serde_json::Value) -> Result<Vec<Sparse>, AppError> {
    let mut all = Vec::new();
    let data_arr = payload["data"]
        .as_array()
        .ok_or_else(|| AppError::Ocr("SCNet response missing data array".to_string()))?;

    for doc in data_arr {
        let results = doc["result"].as_array().cloned().unwrap_or_default();
        for item in &results {
            let status = item["status"].as_i64().unwrap_or(0);
            if status != 200 {
                log::warn!("[SCNet] 单项识别失败 status={}, 跳过", status);
                continue;
            }
            let elements = &item["elements"];
            if elements.is_null() {
                continue;
            }
            let sparse = scnet_elements_to_sparse(elements);
            if !sparse.is_empty() {
                all.push(sparse);
            }
        }
    }
    Ok(all)
}

/// 读取 elements 中指定 key 的字符串值（依次尝试多个 key）。
fn ev(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(val) = obj.get(*k) {
            let s = match val {
                serde_json::Value::String(s) => s.trim().to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => continue,
            };
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

/// SCNet 日期 "20250902" → "2025年09月02日"。
fn scnet_normalize_date(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 8 {
        format!("{}年{}月{}日", &digits[..4], &digits[4..6], &digits[6..8])
    } else {
        raw.to_string()
    }
}

/// 从 title 推断发票类型。
fn scnet_invoice_type(title: &str) -> String {
    if title.contains("专用") {
        "专用发票".to_string()
    } else if title.contains("普通") {
        "普通发票".to_string()
    } else {
        title.to_string()
    }
}

/// 将 SCNet elements 转换为 sparse HashMap。
fn scnet_elements_to_sparse(elements: &serde_json::Value) -> Sparse {
    let obj = match elements.as_object() {
        Some(o) => o,
        None => return Sparse::new(),
    };

    // 检测是否为发票类型
    let is_invoice = obj.contains_key("invoiceNo")
        || obj.contains_key("goodsDetails")
        || obj.contains_key("totalAmountLower")
        || obj.contains_key("buyerName");

    if is_invoice {
        scnet_invoice_to_sparse(obj)
    } else {
        scnet_generic_to_sparse(obj)
    }
}

/// 增值税发票 → sparse。
/// SCNet 字段映射（参考文档 5.2 节 + 实际响应）：
///   invoiceNo→InvoiceNum, buyerName→PurchaserName, sellerName→SellerName,
///   goodsDetails[].goodsName→CommodityName 等。
fn scnet_invoice_to_sparse(obj: &serde_json::Map<String, serde_json::Value>) -> Sparse {
    let mut s = Sparse::new();

    let mut insert = |key: &str, val: String| {
        if !val.is_empty() {
            s.insert(key.to_string(), val);
        }
    };

    insert("InvoiceNum", ev(obj, &["invoiceNo", "printedNo"]));
    insert("InvoiceCode", ev(obj, &["invoiceCode", "printedCode"]));
    let raw_date = ev(obj, &["invoiceDate"]);
    if !raw_date.is_empty() {
        insert("InvoiceDate", scnet_normalize_date(&raw_date));
    }
    insert("PurchaserName", ev(obj, &["buyerName"]));
    insert("PurchaserRegisterNum", ev(obj, &["buyerCode"]));
    insert("PurchaserAddress", ev(obj, &["buyerAddressAndPhone"]));
    insert("PurchaserBank", ev(obj, &["buyerBankAndAccount"]));
    insert("SellerName", ev(obj, &["sellerName"]));
    insert("SellerRegisterNum", ev(obj, &["sellerCode"]));
    insert("SellerAddress", ev(obj, &["sellerAddressAndPhone"]));
    insert("SellerBank", ev(obj, &["sellerBankAndAccount"]));
    insert("TotalAmount", ev(obj, &["preTaxTotalAmount"]));
    insert("TotalTax", ev(obj, &["totalTaxAmount"]));
    insert("AmountInFigures", ev(obj, &["totalAmountLower"]));
    insert("AmountInWords", ev(obj, &["totalAmountUpper"]));
    insert("NoteDrawer", ev(obj, &["drawer"]));
    insert("Checker", ev(obj, &["checker"]));
    insert("Payee", ev(obj, &["payee"]));
    insert("Password", ev(obj, &["passwordArea"]));

    let title = ev(obj, &["title"]);
    if !title.is_empty() {
        insert("InvoiceType", scnet_invoice_type(&title));
    }

    // 商品明细
    if let Some(details) = obj.get("goodsDetails").and_then(|v| v.as_array()) {
        let mut names = Vec::new();
        let mut types = Vec::new();
        let mut units = Vec::new();
        let mut nums = Vec::new();
        let mut prices = Vec::new();
        let mut amounts = Vec::new();
        let mut rates = Vec::new();
        let mut taxes = Vec::new();

        for (i, g) in details.iter().enumerate() {
            let row = (i + 1).to_string();
            let gobj = g.as_object().cloned().unwrap_or_default();
            let gv = |keys: &[&str]| ev(&gobj, keys);

            names.push(RowItem { word: gv(&["goodsName"]), row: row.clone() });
            types.push(RowItem { word: gv(&["specification"]), row: row.clone() });
            units.push(RowItem { word: gv(&["unit"]), row: row.clone() });
            nums.push(RowItem { word: gv(&["quantity"]), row: row.clone() });
            prices.push(RowItem { word: gv(&["unitPrice"]), row: row.clone() });
            amounts.push(RowItem { word: gv(&["itemAmount"]), row: row.clone() });
            rates.push(RowItem { word: gv(&["taxRate"]), row: row.clone() });
            taxes.push(RowItem { word: gv(&["taxAmount"]), row: row.clone() });
        }

        let to_json = |list: Vec<RowItem>| serde_json::to_string(&list).unwrap_or_default();
        if !names.is_empty() {
            s.insert("CommodityName".to_string(), to_json(names));
            s.insert("CommodityType".to_string(), to_json(types));
            s.insert("CommodityUnit".to_string(), to_json(units));
            s.insert("CommodityNum".to_string(), to_json(nums));
            s.insert("CommodityPrice".to_string(), to_json(prices));
            s.insert("CommodityAmount".to_string(), to_json(amounts));
            s.insert("CommodityTaxRate".to_string(), to_json(rates));
            s.insert("CommodityTax".to_string(), to_json(taxes));
        }
    }

    s
}

/// 非发票结构化类型 → sparse（通用 KV 映射）。
fn scnet_generic_to_sparse(obj: &serde_json::Map<String, serde_json::Value>) -> Sparse {
    let mut s = Sparse::new();
    for (key, value) in obj {
        if key == "confidence" || key == "stamps" {
            continue;
        }
        let val = match value {
            serde_json::Value::String(v) => v.trim().to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => continue,
        };
        if !val.is_empty() {
            s.insert(key.clone(), val);
        }
    }
    s
}
