//! End-to-end YOLO inference tests. Requires:
//!   - libonnxruntime in scope (set `ORT_DYLIB_PATH` or load via system linker).
//!   - The model fixture at `tests/fixtures/yolo11n.onnx`. If absent, the test is
//!     skipped with an `eprintln!` — there is no network fallback in CI to avoid
//!     flake from upstream rate-limits.
//!
//! Run with `cargo test -p immich-extended-yolo --test inference`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use yolo::count_people_in_image;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Returns a freshly-created tempdir holding `models/yolo.onnx` symlinked to the bundled
/// fixture. `count_people_in_image` reuses it without hitting the network.
/// Returns `None` if the fixture is not present — the caller skips in that case.
fn stage_model_tempdir() -> Option<(tempfile::TempDir, PathBuf)> {
    let fixture = fixtures_dir().join("yolo11n.onnx");
    if !fixture.exists() {
        eprintln!(
            "yolo: skipping (fixture {} not present; ship it via `yolo export ...`)",
            fixture.display()
        );
        return None;
    }
    let dir = tempfile::TempDir::new().expect("create tempdir");
    let models = dir.path().join("models");
    std::fs::create_dir_all(&models).expect("mkdir models");
    let dst = models.join("yolo.onnx");
    // Hard-link if possible (same fs); fall back to copy.
    if std::fs::hard_link(&fixture, &dst).is_err() {
        std::fs::copy(&fixture, &dst).expect("copy fixture");
    }
    let data_dir = dir.path().to_path_buf();
    Some((dir, data_dir))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_one_person_in_portrait() {
    let Some((_keep, data_dir)) = stage_model_tempdir() else {
        return;
    };
    let img: &Path = &fixtures_dir().join("one_person.jpg");
    let count = count_people_in_image(&data_dir, img)
        .await
        .expect("inference");
    assert!(
        count >= 1,
        "expected >= 1 person in Obama portrait, got {count}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn returns_zero_for_landscape() {
    let Some((_keep, data_dir)) = stage_model_tempdir() else {
        return;
    };
    let img: &Path = &fixtures_dir().join("empty_landscape.jpg");
    let count = count_people_in_image(&data_dir, img)
        .await
        .expect("inference");
    assert_eq!(count, 0, "expected 0 persons in Grand Canyon, got {count}");
}
