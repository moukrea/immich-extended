use std::path::Path;
use std::sync::OnceLock;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

use crate::error::YoloError;

static SESSION: OnceLock<Session> = OnceLock::new();

/// Loads `model_path` into a global `ort::Session` on first call; subsequent calls return
/// the cached reference.
///
/// `intra_threads=1` because YOLOv11n inference is small enough that single-threaded execution
/// is fine on consumer CPUs and we run rule cycles sequentially.
pub fn session(model_path: &Path) -> Result<&'static Session, YoloError> {
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
    // `set` is only Err if SESSION was already initialised — race with a parallel call. In that
    // case the existing one wins and we discard our build.
    let _ = SESSION.set(built);
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
