use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum YoloError {
    #[error("HTTP error downloading model: {0}")]
    Download(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Model SHA256 mismatch (expected {expected}, got {actual})")]
    ShaMismatch { expected: String, actual: String },
    #[error("ONNX Runtime error: {0}")]
    Ort(#[from] ort::Error),
    #[error("Image decode error: {0}")]
    Image(#[from] image::ImageError),
    #[error("Model path not found: {0}")]
    ModelMissing(PathBuf),
    #[error("Model URL/SHA256 not configured (set YOLO_MODEL_URL and YOLO_MODEL_SHA256)")]
    ModelNotConfigured,
}
