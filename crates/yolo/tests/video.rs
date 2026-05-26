//! End-to-end video sampling + person-count tests. Mirrors `inference.rs`: the
//! `yolo11n.onnx` fixture is hard-linked into a tempdir-staged `data_dir/models/`,
//! ffmpeg is invoked on the bundled `10s_one_person.mp4` clip, and the resulting
//! frames are run through the shared session.
//!
//! Skipped (with `eprintln!`) when the ONNX fixture is absent so CI without network
//! still passes when the LFS-ish asset is excluded.
//!
//! Run with `cargo test -p immich-extended-yolo --test video`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use yolo::count_people_in_video;
use yolo::video::extract_frames;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn stage_model_tempdir() -> Option<(tempfile::TempDir, PathBuf)> {
    let fixture = fixtures_dir().join("yolo11n.onnx");
    if !fixture.exists() {
        eprintln!("yolo: skipping (fixture {} not present)", fixture.display());
        return None;
    }
    let dir = tempfile::TempDir::new().expect("create tempdir");
    let models = dir.path().join("models");
    std::fs::create_dir_all(&models).expect("mkdir models");
    let dst = models.join("yolo.onnx");
    if std::fs::hard_link(&fixture, &dst).is_err() {
        std::fs::copy(&fixture, &dst).expect("copy fixture");
    }
    let data_dir = dir.path().to_path_buf();
    Some((dir, data_dir))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extracts_one_frame_every_two_seconds_from_10s_clip() {
    let video = fixtures_dir().join("10s_one_person.mp4");
    if !video.exists() {
        eprintln!("yolo: skipping (fixture {} not present)", video.display());
        return;
    }
    let frames_dir = tempfile::TempDir::new().expect("create frames tempdir");
    let frames = extract_frames(&video, frames_dir.path(), 0.5)
        .await
        .expect("extract_frames");
    assert_eq!(
        frames.len(),
        5,
        "expected 5 frames from 10 s @ 0.5 fps, got {}",
        frames.len()
    );
    for f in &frames {
        let md = std::fs::metadata(f).expect("frame metadata");
        assert!(md.len() > 0, "frame {} is empty", f.display());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn counts_one_person_across_all_frames() {
    let Some((_keep, data_dir)) = stage_model_tempdir() else {
        return;
    };
    let video = fixtures_dir().join("10s_one_person.mp4");
    if !video.exists() {
        eprintln!("yolo: skipping (fixture {} not present)", video.display());
        return;
    }
    let count = count_people_in_video(&data_dir, &video)
        .await
        .expect("video inference");
    assert!(
        count >= 1,
        "expected >= 1 person across frames of single-person clip, got {count}"
    );
    assert!(
        count <= 2,
        "single-person clip should not exceed 2 detections per frame, got {count}"
    );
}
