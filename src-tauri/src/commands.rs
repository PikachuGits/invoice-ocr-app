use std::collections::HashMap;
use tauri::{AppHandle, Emitter};

use crate::invoice_extractor;
use crate::ocr_client::OcrClient;

#[derive(serde::Serialize, Clone)]
pub struct ProgressPayload {
    pub status: String, // "progress" | "success" | "error"
    pub message: String,
}

/// Recognize an invoice image: submit to OCR, poll, parse, and return structured data.
#[tauri::command]
pub async fn recognize_invoice(
    app: AppHandle,
    image_path: String,
) -> Result<invoice_extractor::StandardResult, String> {
    let client = OcrClient::new();

    // Submit
    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "progress".into(),
            message: "正在提交图片...".into(),
        },
    );
    let job_id = client
        .submit(&image_path)
        .await
        .map_err(|e| {
            let _ = app.emit(
                "ocr-progress",
                ProgressPayload {
                    status: "error".into(),
                    message: format!("提交失败: {}", e),
                },
            );
            e.to_string()
        })?;

    // Poll
    let (pages, _raw_json) = client
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

    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "progress".into(),
            message: "正在解析发票...".into(),
        },
    );

    // Parse and extract
    let mut sparse: HashMap<String, String> = HashMap::new();
    for page in &pages {
        let page_result =
            invoice_extractor::parse_invoice_from_markdown(&page.markdown_text, &page.blocks);
        invoice_extractor::merge_sparse_results(&mut sparse, page_result);
    }

    let result = invoice_extractor::make_standard_result(&sparse);

    let _ = app.emit(
        "ocr-progress",
        ProgressPayload {
            status: "success".into(),
            message: "识别完成!".into(),
        },
    );

    Ok(result)
}
