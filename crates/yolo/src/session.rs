use std::path::Path;
use std::sync::{Mutex, OnceLock};

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

use crate::error::YoloError;

static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();

/// Loads `model_path` into a global, mutex-guarded `ort::Session` on first call;
/// subsequent calls return the same mutex.
///
/// `ort::Session::run` requires `&mut self`. We serialize inference behind a `Mutex` so
/// callers can share the loaded model without rebuilding it for every image. Rule cycles
/// run sequentially today (per PRD §12), so the mutex never contends in practice.
///
/// `intra_threads=1` because YOLOv11n inference is light enough that single-threaded
/// execution beats the per-call thread-pool spin-up cost on consumer CPUs.
pub fn session(model_path: &Path) -> Result<&'static Mutex<Session>, YoloError> {
    if let Some(s) = SESSION.get() {
        return Ok(s);
    }
    if !model_path.exists() {
        return Err(YoloError::ModelMissing(model_path.to_path_buf()));
    }
    let built = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(1)?
        .commit_from_file(model_path)?;
    // `set` is `Err` only if SESSION was initialised by a parallel call; in that case the
    // existing model wins and we discard our build.
    let _ = SESSION.set(Mutex::new(built));
    SESSION.get().ok_or_else(|| {
        YoloError::Ort(ort::Error::new(
            "yolo: global Session not initialized after set",
        ))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn session_returns_model_missing_when_path_absent() {
        let bogus = PathBuf::from("/nonexistent/path/yolo.onnx");
        let err = session(&bogus).unwrap_err();
        match err {
            YoloError::ModelMissing(p) => assert_eq!(p, bogus),
            other => panic!("expected ModelMissing, got {other:?}"),
        }
    }
}
