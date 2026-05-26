//! YOLO person-detection pipeline (ONNX Runtime via `ort`).

pub mod error;
pub mod model;
pub mod preprocess;
pub mod session;

pub use error::YoloError;
pub use model::{
    ensure_model, ensure_model_with, model_path, CONF_THRESHOLD, MODEL_INPUT_SIZE, MODEL_VERSION,
    NMS_IOU_THRESHOLD, PERSON_CLASS_ID,
};
pub use preprocess::{letterbox, letterbox_to_tensor, to_input_tensor, LetterboxMeta};
pub use session::session;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn model_version_is_pinned() {
        assert_eq!(MODEL_VERSION, "yolo11n-v1");
    }
}
