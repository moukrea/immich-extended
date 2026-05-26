use image::{imageops::FilterType, DynamicImage, GenericImageView, Rgb, RgbImage};
use ndarray::Array4;

use crate::model::MODEL_INPUT_SIZE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LetterboxMeta {
    pub scale: f32,
    pub pad_x: u32,
    pub pad_y: u32,
    pub original_w: u32,
    pub original_h: u32,
    pub target: u32,
}

/// Resize `img` so the longest side fits in `size`, then pad with grey (114) to a square `size`×`size`.
///
/// Returns the padded image and metadata so postprocess can map detections back to original coords.
pub fn letterbox(img: &DynamicImage, size: u32) -> (DynamicImage, LetterboxMeta) {
    let (w, h) = img.dimensions();
    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    let new_w = ((w as f32 * scale).round() as u32).max(1).min(size);
    let new_h = ((h as f32 * scale).round() as u32).max(1).min(size);
    let resized = img
        .resize_exact(new_w, new_h, FilterType::Triangle)
        .to_rgb8();

    let mut canvas: RgbImage = RgbImage::from_pixel(size, size, Rgb([114, 114, 114]));
    let pad_x = (size - new_w) / 2;
    let pad_y = (size - new_h) / 2;
    image::imageops::replace(&mut canvas, &resized, pad_x as i64, pad_y as i64);

    let meta = LetterboxMeta {
        scale,
        pad_x,
        pad_y,
        original_w: w,
        original_h: h,
        target: size,
    };
    (DynamicImage::ImageRgb8(canvas), meta)
}

/// Convert a square RGB image to a `[1, 3, H, W]` `f32` tensor normalized to `[0, 1]`.
///
/// YOLOv8/v11 ONNX exports expect RGB in CHW order.
pub fn to_input_tensor(img: &DynamicImage) -> Array4<f32> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut arr = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        arr[[0, 0, y as usize, x as usize]] = r;
        arr[[0, 1, y as usize, x as usize]] = g;
        arr[[0, 2, y as usize, x as usize]] = b;
    }
    arr
}

/// Convenience: letterbox to the model's input size and convert to tensor in one call.
pub fn letterbox_to_tensor(img: &DynamicImage) -> (Array4<f32>, LetterboxMeta) {
    let (boxed, meta) = letterbox(img, MODEL_INPUT_SIZE);
    let tensor = to_input_tensor(&boxed);
    (tensor, meta)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb(rgb)))
    }

    #[test]
    fn letterbox_landscape_640_pads_vertically() {
        let img = solid(1280, 720, [255, 0, 0]);
        let (out, meta) = letterbox(&img, 640);
        assert_eq!(out.dimensions(), (640, 640));
        // Landscape: scale by width → 640/1280 = 0.5, height becomes 360, pad_y = 140.
        assert!((meta.scale - 0.5).abs() < 1e-5);
        assert_eq!(meta.pad_x, 0);
        assert_eq!(meta.pad_y, 140);
        assert_eq!(meta.original_w, 1280);
        assert_eq!(meta.original_h, 720);
        assert_eq!(meta.target, 640);
    }

    #[test]
    fn letterbox_portrait_pads_horizontally() {
        let img = solid(480, 640, [0, 255, 0]);
        let (out, meta) = letterbox(&img, 640);
        assert_eq!(out.dimensions(), (640, 640));
        // Portrait: scale by height → 640/640 = 1.0, width stays 480, pad_x = 80.
        assert!((meta.scale - 1.0).abs() < 1e-5);
        assert_eq!(meta.pad_x, 80);
        assert_eq!(meta.pad_y, 0);
    }

    #[test]
    fn letterbox_square_image_no_padding() {
        let img = solid(640, 640, [0, 0, 255]);
        let (out, meta) = letterbox(&img, 640);
        assert_eq!(out.dimensions(), (640, 640));
        assert_eq!(meta.pad_x, 0);
        assert_eq!(meta.pad_y, 0);
        assert!((meta.scale - 1.0).abs() < 1e-5);
    }

    #[test]
    fn to_input_tensor_shape_and_normalization() {
        let img = solid(640, 640, [255, 128, 0]);
        let arr = to_input_tensor(&img);
        assert_eq!(arr.shape(), &[1, 3, 640, 640]);
        // Channel 0 = R = 1.0, channel 1 = G ≈ 0.5, channel 2 = B = 0.0
        assert!((arr[[0, 0, 0, 0]] - 1.0).abs() < 1e-5);
        assert!((arr[[0, 1, 100, 100]] - (128.0 / 255.0)).abs() < 1e-5);
        assert!(arr[[0, 2, 200, 200]].abs() < 1e-5);
    }

    #[test]
    fn letterbox_to_tensor_returns_square_tensor() {
        let img = solid(800, 600, [10, 20, 30]);
        let (arr, meta) = letterbox_to_tensor(&img);
        assert_eq!(arr.shape(), &[1, 3, 640, 640]);
        assert_eq!(meta.target, 640);
    }
}
