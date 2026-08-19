use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)] // Timeout/Parse are part of the error surface for future use
pub enum AppError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("OCR error: {0}")]
    Ocr(String),

    #[error("OCR timeout after {0}s")]
    Timeout(u64),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("{0}")]
    Config(String),
}

// Tauri invoke error conversion
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
