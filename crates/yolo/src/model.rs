use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::error::YoloError;

pub const MODEL_VERSION: &str = "yolo11n-v1";
pub const MODEL_INPUT_SIZE: u32 = 640;
pub const PERSON_CLASS_ID: usize = 0;
pub const CONF_THRESHOLD: f32 = 0.5;
pub const NMS_IOU_THRESHOLD: f32 = 0.5;

/// Default model download URL — a pinned GitHub release asset on this repo.
/// Operators may override with the `YOLO_MODEL_URL` env var.
pub const DEFAULT_MODEL_URL: &str =
    "https://github.com/moukrea/immich-extended/releases/download/models-yolo11n-v1/yolo11n.onnx";

const MODEL_REL_PATH: &str = "models/yolo.onnx";

pub fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MODEL_REL_PATH)
}

pub fn model_url() -> String {
    std::env::var("YOLO_MODEL_URL").unwrap_or_else(|_| DEFAULT_MODEL_URL.to_string())
}

/// Ensures `data_dir/models/yolo.onnx` exists.
///
/// Cached path is reused if present; otherwise downloads from `url`, streams to a `.tmp`
/// file, and atomically renames into place.
pub async fn ensure_model_with(data_dir: &Path, url: &str) -> Result<PathBuf, YoloError> {
    let path = model_path(data_dir);

    if path.exists() {
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("onnx.tmp");

    tracing::info!("yolo: downloading model from {} to {}", url, tmp.display());
    let resp = reqwest::get(url).await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&tmp).await?;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    drop(file);

    tokio::fs::rename(&tmp, &path).await?;
    Ok(path)
}

/// Convenience wrapper using the baked-in [`DEFAULT_MODEL_URL`] constant. Operators may
/// override the URL with the `YOLO_MODEL_URL` env var.
pub async fn ensure_model(data_dir: &Path) -> Result<PathBuf, YoloError> {
    let url = model_url();
    ensure_model_with(data_dir, &url).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn unique_tempdir(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "yolo-model-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn ensure_model_with_returns_cached_path_when_file_exists() {
        let tmp = unique_tempdir("cached-ok");
        let path = model_path(&tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"hello yolo cache")
            .unwrap();

        let got = ensure_model_with(&tmp, "http://unused.invalid/x")
            .await
            .unwrap();
        assert_eq!(got, path);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn ensure_model_with_errors_when_url_unreachable_and_cache_missing() {
        let tmp = unique_tempdir("download-unreachable");
        let result = ensure_model_with(&tmp, "http://127.0.0.1:1/missing").await;
        assert!(matches!(result, Err(YoloError::Download(_))));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn model_path_appends_models_yolo_onnx() {
        let dp = std::path::Path::new("/data");
        assert_eq!(
            model_path(dp),
            std::path::PathBuf::from("/data/models/yolo.onnx")
        );
    }

    #[test]
    fn default_model_url_is_https_onnx() {
        assert!(DEFAULT_MODEL_URL.starts_with("https://"));
        assert!(DEFAULT_MODEL_URL.ends_with(".onnx"));
    }

    /// Belt-and-suspenders: the public URL resolver returns SOME non-empty value
    /// without panicking, whether or not the env var is set in this test process.
    #[test]
    fn url_resolver_returns_non_empty_string() {
        let url = model_url();
        assert!(url.starts_with("http"), "model_url: {url}");
    }
}
