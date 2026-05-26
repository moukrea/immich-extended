//! YOLOv11 detection postprocess: filter persons by class-score, undo the letterbox
//! transform, and apply non-maximum suppression to count distinct people.
//!
//! YOLOv11 omits the explicit objectness score that YOLOv5/8 carried; the class score
//! IS the confidence. Output shape is `[1, 84, 8400]` — for each of 8400 anchors:
//! `[cx, cy, w, h, class_0_score, class_1_score, ..., class_79_score]` in feature-axis
//! order. We only care about `class_0` (person, COCO id 0).

use ndarray::ArrayView3;

use crate::model::{CONF_THRESHOLD, NMS_IOU_THRESHOLD, PERSON_CLASS_ID};
use crate::preprocess::LetterboxMeta;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Detection {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    score: f32,
}

fn iou(a: &Detection, b: &Detection) -> f32 {
    let inter_x1 = a.x1.max(b.x1);
    let inter_y1 = a.y1.max(b.y1);
    let inter_x2 = a.x2.min(b.x2);
    let inter_y2 = a.y2.min(b.y2);
    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter = inter_w * inter_h;
    let area_a = (a.x2 - a.x1).max(0.0) * (a.y2 - a.y1).max(0.0);
    let area_b = (b.x2 - b.x1).max(0.0) * (b.y2 - b.y1).max(0.0);
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn nms(mut dets: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    dets.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Detection> = Vec::with_capacity(dets.len());
    for d in dets {
        if kept.iter().all(|k| iou(&d, k) <= iou_threshold) {
            kept.push(d);
        }
    }
    kept
}

/// Counts distinct people in a YOLOv11 raw output tensor.
///
/// `raw_output` must be `[1, 84, 8400]` (Ultralytics default export). `meta` is the
/// letterbox metadata produced by [`crate::preprocess::letterbox`]; it's used to map
/// detections back to the original image coordinate space (not strictly required for
/// counting, but keeps boxes meaningful if M6 later renders them).
pub fn count_persons(raw_output: ArrayView3<'_, f32>, meta: &LetterboxMeta) -> u32 {
    let shape = raw_output.shape();
    if shape.len() != 3 || shape[0] != 1 {
        tracing::warn!("yolo: unexpected output shape {:?}, returning 0", shape);
        return 0;
    }
    // Detect the orientation: Ultralytics default is [1, 84, 8400] (features × anchors).
    // Some older exports transpose to [1, 8400, 84]. We pick the axis equal to 84.
    let (features, anchors, transposed) = if shape[1] == 84 {
        (shape[1], shape[2], false)
    } else if shape[2] == 84 {
        (shape[2], shape[1], true)
    } else {
        tracing::warn!(
            "yolo: output shape {:?} has no 84-sized axis (expected 4 box + 80 class), returning 0",
            shape
        );
        return 0;
    };
    // 4 box coords + 80 class scores = 84 features. Sanity-check the class id fits.
    let class_feature_index = 4 + PERSON_CLASS_ID;
    if class_feature_index >= features {
        return 0;
    }

    let mut dets: Vec<Detection> = Vec::new();
    for a in 0..anchors {
        let (cx, cy, w, h, score) = if transposed {
            // [1, anchors, features]
            (
                raw_output[[0, a, 0]],
                raw_output[[0, a, 1]],
                raw_output[[0, a, 2]],
                raw_output[[0, a, 3]],
                raw_output[[0, a, class_feature_index]],
            )
        } else {
            // [1, features, anchors]
            (
                raw_output[[0, 0, a]],
                raw_output[[0, 1, a]],
                raw_output[[0, 2, a]],
                raw_output[[0, 3, a]],
                raw_output[[0, class_feature_index, a]],
            )
        };
        if score < CONF_THRESHOLD {
            continue;
        }
        // Decode (cx, cy, w, h) → (x1, y1, x2, y2) in letterboxed pixel space.
        let half_w = w * 0.5;
        let half_h = h * 0.5;
        let mut x1 = cx - half_w;
        let mut y1 = cy - half_h;
        let mut x2 = cx + half_w;
        let mut y2 = cy + half_h;
        // Undo letterbox: subtract pad, then divide by scale.
        let pad_x = meta.pad_x as f32;
        let pad_y = meta.pad_y as f32;
        x1 = (x1 - pad_x) / meta.scale;
        x2 = (x2 - pad_x) / meta.scale;
        y1 = (y1 - pad_y) / meta.scale;
        y2 = (y2 - pad_y) / meta.scale;
        // Clamp to original-image bounds.
        let orig_w = meta.original_w as f32;
        let orig_h = meta.original_h as f32;
        x1 = x1.clamp(0.0, orig_w);
        x2 = x2.clamp(0.0, orig_w);
        y1 = y1.clamp(0.0, orig_h);
        y2 = y2.clamp(0.0, orig_h);
        if x2 <= x1 || y2 <= y1 {
            continue;
        }
        dets.push(Detection {
            x1,
            y1,
            x2,
            y2,
            score,
        });
    }

    nms(dets, NMS_IOU_THRESHOLD).len() as u32
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ndarray::Array3;

    fn meta_640_no_pad() -> LetterboxMeta {
        LetterboxMeta {
            scale: 1.0,
            pad_x: 0,
            pad_y: 0,
            original_w: 640,
            original_h: 640,
            target: 640,
        }
    }

    fn empty_output() -> Array3<f32> {
        Array3::<f32>::zeros((1, 84, 8400))
    }

    fn set_anchor(arr: &mut Array3<f32>, idx: usize, cx: f32, cy: f32, w: f32, h: f32, score: f32) {
        arr[[0, 0, idx]] = cx;
        arr[[0, 1, idx]] = cy;
        arr[[0, 2, idx]] = w;
        arr[[0, 3, idx]] = h;
        arr[[0, 4 + PERSON_CLASS_ID, idx]] = score;
    }

    #[test]
    fn no_detections_when_all_scores_below_threshold() {
        let out = empty_output();
        let n = count_persons(out.view(), &meta_640_no_pad());
        assert_eq!(n, 0);
    }

    #[test]
    fn single_high_confidence_box_counts_as_one() {
        let mut out = empty_output();
        set_anchor(&mut out, 0, 320.0, 320.0, 100.0, 200.0, 0.9);
        let n = count_persons(out.view(), &meta_640_no_pad());
        assert_eq!(n, 1);
    }

    #[test]
    fn two_far_apart_boxes_survive_nms() {
        let mut out = empty_output();
        set_anchor(&mut out, 0, 100.0, 100.0, 50.0, 100.0, 0.9);
        set_anchor(&mut out, 1, 500.0, 400.0, 50.0, 100.0, 0.8);
        let n = count_persons(out.view(), &meta_640_no_pad());
        assert_eq!(n, 2);
    }

    #[test]
    fn overlapping_boxes_merge_via_nms() {
        let mut out = empty_output();
        // Two highly-overlapping boxes — IoU > 0.5 → NMS keeps the higher-score one.
        set_anchor(&mut out, 0, 320.0, 320.0, 100.0, 200.0, 0.95);
        set_anchor(&mut out, 1, 322.0, 322.0, 100.0, 200.0, 0.85);
        let n = count_persons(out.view(), &meta_640_no_pad());
        assert_eq!(n, 1);
    }

    #[test]
    fn non_person_classes_ignored() {
        let mut out = empty_output();
        // Class 1 (bicycle) at high score should not be counted.
        out[[0, 0, 0]] = 320.0;
        out[[0, 1, 0]] = 320.0;
        out[[0, 2, 0]] = 100.0;
        out[[0, 3, 0]] = 100.0;
        out[[0, 4 + 1, 0]] = 0.95; // bicycle
        let n = count_persons(out.view(), &meta_640_no_pad());
        assert_eq!(n, 0);
    }

    #[test]
    fn transposed_layout_8400_84_also_works() {
        let mut out: Array3<f32> = Array3::<f32>::zeros((1, 8400, 84));
        out[[0, 0, 0]] = 320.0;
        out[[0, 0, 1]] = 320.0;
        out[[0, 0, 2]] = 100.0;
        out[[0, 0, 3]] = 200.0;
        out[[0, 0, 4 + PERSON_CLASS_ID]] = 0.9;
        let n = count_persons(out.view(), &meta_640_no_pad());
        assert_eq!(n, 1);
    }

    #[test]
    fn box_outside_after_letterbox_undo_is_clamped_then_dropped() {
        let mut out = empty_output();
        // Box totally above the image (cy + h/2 < 0 after undoing letterbox).
        // With pad_y=200 and scale=1, cy=50 → y2 = (50+100-200)/1 = -50, gets clamped to 0
        // and y1 also < 0 → both clamp to 0 → y2 == y1 → dropped.
        let meta = LetterboxMeta {
            scale: 1.0,
            pad_x: 0,
            pad_y: 200,
            original_w: 640,
            original_h: 240,
            target: 640,
        };
        set_anchor(&mut out, 0, 320.0, 50.0, 200.0, 200.0, 0.9);
        let n = count_persons(out.view(), &meta);
        assert_eq!(n, 0);
    }

    #[test]
    fn iou_of_identical_boxes_is_one() {
        let a = Detection {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
            score: 0.0,
        };
        assert!((iou(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn iou_of_disjoint_boxes_is_zero() {
        let a = Detection {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
            score: 0.0,
        };
        let b = Detection {
            x1: 20.0,
            y1: 20.0,
            x2: 30.0,
            y2: 30.0,
            score: 0.0,
        };
        assert!(iou(&a, &b) < 1e-6);
    }
}
