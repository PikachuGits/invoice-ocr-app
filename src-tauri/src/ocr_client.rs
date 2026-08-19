use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::AppError;

const MODEL: &str = "PaddleOCR-VL-1.6";
const REQUEST_TIMEOUT: u64 = 60;
const POLL_INTERVAL: u64 = 5;
const MAX_POLL_COUNT: u64 = 720;

const DEFAULT_API_URL: &str = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    token: String,
    #[serde(default)]
    api_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            token: String::new(),
            api_url: String::new(),
        }
    }
}

impl AppConfig {
    fn load() -> Self {
        // 尝试从工作目录加载 .env（PADDLEOCR_TOKEN / PADDLEOCR_API_URL），不存在则忽略
        let _ = dotenvy::dotenv();

        // Try to read config.json from the same directory as the executable,
        // or from the current working directory
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
                if let Ok(config) = serde_json::from_str::<AppConfig>(&content) {
                    log::info!("Loaded config from {}", path);
                    return config;
                }
            }
        }
        AppConfig::default()
    }

    /// 优先级：环境变量 > config.json。未配置时返回 None（不再内置默认 token）。
    fn get_token(&self) -> Option<String> {
        std::env::var("PADDLEOCR_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if self.token.is_empty() {
                    None
                } else {
                    Some(self.token.clone())
                }
            })
    }

    fn get_api_url(&self) -> String {
        std::env::var("PADDLEOCR_API_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if self.api_url.is_empty() {
                    None
                } else {
                    Some(self.api_url.clone())
                }
            })
            .unwrap_or_else(|| DEFAULT_API_URL.to_string())
    }
}

pub struct OcrClient {
    client: Client,
    token: String,
    api_url: String,
}

impl OcrClient {
    /// 创建客户端。`db_token` / `db_api_url` 为数据库配置表（设置页）中的值，
    /// 优先级：DB 配置 > 环境变量 > config.json。
    pub fn new(db_token: Option<String>, db_api_url: Option<String>) -> Self {
        let config = AppConfig::load();
        let token = db_token
            .filter(|s| !s.is_empty())
            .or_else(|| config.get_token());
        let api_url = db_api_url
            .filter(|s| !s.is_empty())
            .or_else(|| Some(config.get_api_url()))
            .unwrap_or_default();
        log::info!("OCR API: {}", api_url);
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            token: token.unwrap_or_default(),
            api_url,
        }
    }

    /// 提交前校验：Token 未配置时给出明确提示。
    fn ensure_token(&self) -> Result<(), AppError> {
        if self.token.is_empty() {
            return Err(AppError::Config(
                "未配置 API Token，请到「设置 → API 配置」填写，或在 .env 中设置 PADDLEOCR_TOKEN"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Submit an image file for OCR, return the job ID.
    pub async fn submit(&self, file_path: &str) -> Result<String, AppError> {
        self.ensure_token()?;
        log::info!("[OCR] 处理文件: {}", file_path);
        let headers = self.auth_headers();

        let optional_payload = serde_json::json!({
            "useDocOrientationClassify": false,
            "useDocUnwarping": false,
            "useChartRecognition": false,
            "useSealRecognition": true
        });

        let resp = if file_path.starts_with("http://") || file_path.starts_with("https://") {
            let mut h = headers;
            h.insert("Content-Type", "application/json".parse().unwrap());
            let payload = serde_json::json!({
                "fileUrl": file_path,
                "model": MODEL,
                "optionalPayload": optional_payload.to_string(),
            });
            self.client.post(&self.api_url).headers(h).json(&payload).send().await?
        } else {
            let file_bytes = tokio::fs::read(file_path).await?;
            let file_name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image.jpg");
            let mime = if file_name.ends_with(".png") {
                "image/png"
            } else {
                "image/jpeg"
            };
            let part = reqwest::multipart::Part::bytes(file_bytes)
                .file_name(file_name.to_string())
                .mime_str(mime)?;
            let form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("model", MODEL.to_string())
                .text("optionalPayload", optional_payload.to_string());

            self.client
                .post(&self.api_url)
                .headers(headers)
                .multipart(form)
                .send()
                .await?
        };

        log::info!("[OCR] 响应状态: {}", resp.status());
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
        log::info!("[OCR] 任务已提交, jobId: {}", job_id);
        Ok(job_id)
    }

    /// Poll until the job is done, calling on_progress with status updates.
    pub async fn poll<F>(
        &self,
        job_id: &str,
        on_progress: F,
    ) -> Result<(Vec<PageData>, String), AppError>
    where
        F: Fn(String) + Send + Sync,
    {
        let headers = self.auth_headers();
        let mut jsonl_url = String::new();

        for _ in 0..MAX_POLL_COUNT {
            let resp = self
                .client
                .get(format!("{}/{}", self.api_url, job_id))
                .headers(headers.clone())
                .send()
                .await?;
            let payload: serde_json::Value = resp.json().await?;
            let data = &payload["data"];
            let state = data["state"].as_str().unwrap_or("");

            match state {
                "pending" => {
                    on_progress("任务排队中...".to_string());
                }
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

        // Download the JSONL result
        let json_resp = self.client.get(&jsonl_url).send().await?;
        let raw_text = json_resp.text().await?;
        let pages = self.parse_jsonl_response(&raw_text)?;
        Ok((pages, raw_text))
    }

    /// Parse the API JSON/JSONL response into per-page data. `pub(crate)`
    /// so the extractor regression tests exercise the real pipeline.
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

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("bearer {}", self.token).parse().unwrap(),
        );
        headers
    }
}
