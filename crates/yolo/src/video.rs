//! Video person-counting via ffmpeg frame sampling.
//!
//! Extracts frames at 0.5 fps (one frame every 2 s) into a tempdir, runs
//! [`count_people_in_image`](crate::count_people_in_image) on each, and returns the
//! maximum count across frames. The PRD specifies that a video is rejected when any
//! single frame has more YOLO persons than identified faces, so reporting the max is
//! the simplest contract.

use std::path::{Path, PathBuf};

use crate::{count_people_in_image, error::YoloError};

/// Frame extraction rate used by [`count_people_in_video`]. 0.5 fps = one frame every
/// 2 s. Matches PRD §7 / §12.
pub const VIDEO_SAMPLE_FPS: f32 = 0.5;

/// Hard cap on the number of frames we inspect per video. At [`VIDEO_SAMPLE_FPS`] this
/// translates to ~120 s of video; clips longer than that have all subsequent frames
/// dropped and a `tracing::warn!` is emitted.
pub const MAX_FRAMES_PER_VIDEO: usize = 60;

/// Max byte length of the ffmpeg stderr we attach to [`YoloError::Ffmpeg`]. Full
/// stderr can be MBs of repeated codec chatter; keep it bounded so log lines stay
/// readable.
const MAX_STDERR_BYTES: usize = 4096;

/// Extracts frames from `video_path` at `fps` into `frames_dir`, returning the
/// sorted list of extracted JPEG paths (named `frame_%04d.jpg`). Returns
/// [`YoloError::Ffmpeg`] with truncated stderr on non-zero exit. Exposed for
/// integration tests; production callers should use [`count_people_in_video`].
pub async fn extract_frames(
    video_path: &Path,
    frames_dir: &Path,
    fps: f32,
) -> Result<Vec<PathBuf>, YoloError> {
    let pattern = frames_dir.join("frame_%04d.jpg");
    let output = tokio::process::Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("fps={fps}"))
        .arg("-q:v")
        .arg("5")
        .arg(&pattern)
        .output()
        .await?;

    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if stderr.len() > MAX_STDERR_BYTES {
            stderr.truncate(MAX_STDERR_BYTES);
            stderr.push_str("...[truncated]");
        }
        return Err(YoloError::Ffmpeg(stderr));
    }

    let mut entries: Vec<PathBuf> = Vec::new();
    let mut rd = tokio::fs::read_dir(frames_dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("jpg") {
            entries.push(p);
        }
    }
    entries.sort();
    Ok(entries)
}

/// Counts the maximum number of persons across sampled frames of `video_path`. Uses
/// the same global YOLO session as [`count_people_in_image`]; frames are processed
/// sequentially (the session mutex serialises inference anyway).
///
/// Frames beyond [`MAX_FRAMES_PER_VIDEO`] are dropped with a warning.
pub async fn count_people_in_video(data_dir: &Path, video_path: &Path) -> Result<u32, YoloError> {
    let tempdir = tempfile::Builder::new().prefix("yolo-frames-").tempdir()?;
    let mut frames = extract_frames(video_path, tempdir.path(), VIDEO_SAMPLE_FPS).await?;

    if frames.len() > MAX_FRAMES_PER_VIDEO {
        tracing::warn!(
            video = %video_path.display(),
            total = frames.len(),
            cap = MAX_FRAMES_PER_VIDEO,
            "yolo: video has more sampled frames than MAX_FRAMES_PER_VIDEO; truncating"
        );
        frames.truncate(MAX_FRAMES_PER_VIDEO);
    }

    let mut max_count: u32 = 0;
    for frame in &frames {
        let count = count_people_in_image(data_dir, frame).await?;
        if count > max_count {
            max_count = count;
        }
    }
    Ok(max_count)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_prd() {
        assert!((VIDEO_SAMPLE_FPS - 0.5).abs() < f32::EPSILON);
        assert_eq!(MAX_FRAMES_PER_VIDEO, 60);
    }
}
