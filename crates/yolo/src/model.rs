use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::YoloError;

pub const MODEL_VERSION: &str = "yolo11n-v1";
pub const MODEL_INPUT_SIZE: u32 = 640;
pub const PERSON_CLASS_ID: usize = 0;
pub const CONF_THRESHOLD: f32 = 0.5;
pub const NMS_IOU_THRESHOLD: f32 = 0.5;

/// Default model download URL — a pinned GitHub release asset on this repo.
/// Operators may override with the `YOLO_MODEL_URL` env var; the downloaded
/// bytes are always verified against the configured (default or override) SHA256.
pub const DEFAULT_MODEL_URL: &str =
    "https://github.com/moukrea/immich-extended/releases/download/models-yolo11n-v1/yolo11n.onnx";

/// Default model SHA256 — matches the bytes of the GitHub release asset above
/// and `crates/yolo/tests/fixtures/yolo11n.onnx`. Operators may override with
/// the `YOLO_MODEL_SHA256` env var; the override is the value that must match
/// the downloaded bytes.
pub const DEFAULT_MODEL_SHA256: &str =
    "aad852905370fe1b9cfb684690022013f2e0fa75ed699f472320cb38671bb04f";

const MODEL_REL_PATH: &str = "models/yolo.onnx";

pub fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MODEL_REL_PATH)
}

pub fn model_url() -> String {
    std::env::var("YOLO_MODEL_URL").unwrap_or_else(|_| DEFAULT_MODEL_URL.to_string())
}

pub fn expected_sha256() -> String {
    std::env::var("YOLO_MODEL_SHA256")
        .unwrap_or_else(|_| DEFAULT_MODEL_SHA256.to_string())
        .to_lowercase()
}

async fn sha256_file(path: &Path) -> Result<String, YoloError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = tokio::io::AsyncReadExt::read(&mut file, &mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Ensures `data_dir/models/yolo.onnx` exists with the expected SHA256.
///
/// Cached path is reused if the SHA matches `expected_sha256`; otherwise (or if missing)
/// downloads from `url`, streams to a `.tmp` file, verifies SHA, and atomically renames.
pub async fn ensure_model_with(
    data_dir: &Path,
    url: &str,
    expected_sha256: &str,
) -> Result<PathBuf, YoloError> {
    let path = model_path(data_dir);
    let expected = expected_sha256.to_lowercase();

    if path.exists() {
        let actual = sha256_file(&path).await?;
        if actual.eq_ignore_ascii_case(&expected) {
            return Ok(path);
        }
        tracing::warn!(
            "yolo: cached model at {} has SHA256 {}, expected {} — re-downloading",
            path.display(),
            actual,
            expected
        );
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("onnx.tmp");

    tracing::info!("yolo: downloading model from {} to {}", url, tmp.display());
    let resp = reqwest::get(url).await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        hasher.update(&bytes);
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&expected) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(YoloError::ShaMismatch { expected, actual });
    }

    tokio::fs::rename(&tmp, &path).await?;
    Ok(path)
}

/// Convenience wrapper using the baked-in [`DEFAULT_MODEL_URL`] +
/// [`DEFAULT_MODEL_SHA256`] constants. Operators may override either with
/// `YOLO_MODEL_URL` / `YOLO_MODEL_SHA256` env vars (the SHA256 of the
/// downloaded bytes is still verified against the configured value).
pub async fn ensure_model(data_dir: &Path) -> Result<PathBuf, YoloError> {
    let url = model_url();
    let sha = expected_sha256();
    ensure_model_with(data_dir, &url, &sha).await
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
    async fn ensure_model_with_returns_cached_path_when_sha_matches() {
        let tmp = unique_tempdir("cached-ok");
        let path = model_path(&tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload = b"hello yolo cache";
        std::fs::File::create(&path)
            .unwrap()
            .write_all(payload)
            .unwrap();

        let expected = {
            let mut h = Sha256::new();
            h.update(payload);
            hex::encode(h.finalize())
        };

        let got = ensure_model_with(&tmp, "http://unused.invalid/x", &expected)
            .await
            .unwrap();
        assert_eq!(got, path);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn ensure_model_with_errors_when_url_unreachable_and_cache_missing() {
        let tmp = unique_tempdir("download-unreachable");
        // No cached file. URL pointing at an unbound TCP port → reqwest error.
        let result = ensure_model_with(
            &tmp,
            "http://127.0.0.1:1/missing",
            "deadbeef".repeat(8).as_str(),
        )
        .await;
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
    fn default_constants_are_well_formed() {
        assert!(DEFAULT_MODEL_URL.starts_with("https://"));
        assert!(DEFAULT_MODEL_URL.ends_with(".onnx"));
        assert_eq!(DEFAULT_MODEL_SHA256.len(), 64);
        assert!(
            DEFAULT_MODEL_SHA256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "DEFAULT_MODEL_SHA256 must be lower-case hex"
        );
    }

    /// Belt-and-suspenders: the public resolver returns SOME non-empty value
    /// without panicking, whether or not the env var is set in this test
    /// process. We don't mutate env (Rust 2024 made set/remove unsafe and the
    /// crate forbids unsafe); we just exercise the call path.
    #[test]
    fn resolvers_return_non_empty_string() {
        let url = model_url();
        assert!(url.starts_with("http"), "model_url: {url}");
        let sha = expected_sha256();
        assert_eq!(sha.len(), 64);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
