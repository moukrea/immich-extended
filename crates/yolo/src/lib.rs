//! YOLO person-detection pipeline (ONNX Runtime via `ort`).

use std::path::{Path, PathBuf};

pub mod error;
pub mod model;
pub mod postprocess;
pub mod preprocess;
pub mod session;
pub mod video;

pub use error::YoloError;
pub use model::{
    ensure_model, ensure_model_with, model_path, CONF_THRESHOLD, MODEL_INPUT_SIZE, MODEL_VERSION,
    NMS_IOU_THRESHOLD, PERSON_CLASS_ID,
};
pub use postprocess::count_persons;
pub use preprocess::{letterbox, letterbox_to_tensor, to_input_tensor, LetterboxMeta};
pub use session::session;
pub use video::{count_people_in_video, MAX_FRAMES_PER_VIDEO, VIDEO_SAMPLE_FPS};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

/// Loads (or reuses) the global YOLO session, decodes `image_path`, runs inference, and
/// returns the number of distinct persons detected with confidence ≥ 0.5.
///
/// The model is expected at `data_dir/models/yolo.onnx`. If absent, the function falls
/// back to [`ensure_model`] which downloads from `YOLO_MODEL_URL` and verifies
/// `YOLO_MODEL_SHA256`. Inference runs on a blocking thread pool because it is CPU-bound.
pub async fn count_people_in_image(data_dir: &Path, image_path: &Path) -> Result<u32, YoloError> {
    let mp = model_path(data_dir);
    if !mp.exists() {
        ensure_model(data_dir).await?;
    }
    let image_path: PathBuf = image_path.to_path_buf();
    let mp_for_task = mp.clone();

    let result =
        tokio::task::spawn_blocking(move || run_inference_blocking(&mp_for_task, &image_path))
            .await
            .map_err(|join_err| {
                YoloError::Io(std::io::Error::other(format!(
                    "yolo inference task panicked or was cancelled: {join_err}"
                )))
            })?;
    result
}

fn run_inference_blocking(model_path: &Path, image_path: &Path) -> Result<u32, YoloError> {
    let session_mutex = session(model_path)?;
    let img = image::open(image_path)?;
    let (tensor, meta) = letterbox_to_tensor(&img);

    let mut sess = session_mutex.lock().map_err(|_| {
        YoloError::Ort(ort::Error::new(
            "yolo: session mutex poisoned by a previous panic",
        ))
    })?;
    let input_value = ort::value::TensorRef::from_array_view(&tensor)?;
    let outputs = sess.run(ort::inputs![input_value])?;
    if outputs.len() == 0 {
        return Err(YoloError::Ort(ort::Error::new(
            "yolo: session produced no outputs (model graph mismatch?)",
        )));
    }
    // Ultralytics YOLOv11 exports a single output named "output0". Fall back to the
    // first positional output if the name differs (some converters emit "outputs", others
    // use the raw node id).
    let raw = outputs.get("output0").unwrap_or(&outputs[0]);
    let dyn_view = raw.try_extract_array::<f32>()?;
    let view3 = dyn_view
        .into_dimensionality::<ndarray::Ix3>()
        .map_err(|e| {
            YoloError::Ort(ort::Error::new(format!(
                "yolo: output tensor is not 3-D (got {e}); expected [1, 84, 8400]"
            )))
        })?;
    Ok(count_persons(view3, &meta))
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
