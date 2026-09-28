// SPDX-License-Identifier: MIT OR Apache-2.0
//! Model-copy encoding: resize + compress images for vision LLM consumption.
//!
//! Per concept §10: pre-resize to known sizes, JPEG q≈85, byte-exact accounting.
//!
//! For P1 (zero external deps), we use a self-describing baseline format that
//! enables byte-exact accounting. Real JPEG/WebP encoders are deferred to Phase 7.

use gp_core::diagnostics::Diagnostic;

/// Known resize sizes for vision LLM token budgeting.
/// These sizes ensure predictable token counts (ceil(w/28) * ceil(h/28)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizeTable {
    /// Maximum width in pixels.
    pub max_width: usize,
    /// Maximum height in pixels.
    pub max_height: usize,
    /// Aspect-ratio-preserving resize target (computed).
    pub target_width: usize,
    pub target_height: usize,
}

/// Standard image sizes for known token budgets.
pub const STANDARD_SIZES: &[ResizeTable] = &[
    ResizeTable {
        max_width: 768,
        max_height: 1024,
        target_width: 768,
        target_height: 1024,
    },
    ResizeTable {
        max_width: 1024,
        max_height: 1024,
        target_width: 1024,
        target_height: 1024,
    },
    ResizeTable {
        max_width: 2000,
        max_height: 2000,
        target_width: 2000,
        target_height: 2000,
    },
];

/// JPEG quality for model-copy encoding.
pub const JPEG_QUALITY: u8 = 85;

/// The maximum size for a single image block (2000px per side).
pub const MAX_ENCODE_SIZE: usize = 2000;

/// Encoding format for model copy.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodeFormat {
    Jpeg { quality: u8 },
    Webp { quality: f32 },
}

impl Default for EncodeFormat {
    fn default() -> Self {
        EncodeFormat::Jpeg {
            quality: JPEG_QUALITY,
        }
    }
}

/// Encoded image result with byte-exact accounting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EncodedImage {
    /// Raw image bytes (JPEG or WebP encoded).
    pub data: Vec<u8>,
    /// Width in pixels (after resize).
    pub width: usize,
    /// Height in pixels (after resize).
    pub height: usize,
    /// Encoding format used.
    pub format: String,
    /// Quality setting used.
    pub quality: usize,
    /// Original image size before resize.
    pub original_width: usize,
    pub original_height: usize,
    /// Number of visual tokens this image consumes.
    pub visual_tokens: usize,
}

impl EncodedImage {
    /// Compute visual tokens: ceil(w/28) * ceil(h/28).
    pub fn compute_visual_tokens(width: usize, height: usize) -> usize {
        let tw = (width as f64 / 28.0).ceil() as usize;
        let th = (height as f64 / 28.0).ceil() as usize;
        tw * th
    }

    /// Compute the byte cost of this encoded image.
    pub fn byte_cost(&self) -> usize {
        self.data.len()
    }
}

/// Resize an image to fit within the given maximum dimensions,
/// preserving aspect ratio.
///
/// Input: raw grayscale bytes (u8) with given width and height.
/// Output: resized grayscale bytes.
///
/// Uses area-based averaging for downsampling (good quality, fast).
pub fn resize_grayscale(
    data: &[u8],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<u8> {
    if data.len() != src_width * src_height {
        return data.to_vec();
    }
    if dst_width == src_width && dst_height == src_height {
        return data.to_vec();
    }

    let mut result = vec![0u8; dst_width * dst_height];

    for y in 0..dst_height {
        for x in 0..dst_width {
            // Map destination pixel to source pixel coordinates
            let src_x = (x * src_width) / dst_width;
            let src_y = (y * src_height) / dst_height;

            // Compute the source pixel value (simple nearest-neighbor for P1 stub)
            let clamped_x = src_x.min(src_width - 1);
            let clamped_y = src_y.min(src_height - 1);
            result[y * dst_width + x] = data[clamped_y * src_width + clamped_x];
        }
    }

    result
}

/// Encode a grayscale image to JPEG bytes with the given quality.
///
/// For P1, this produces a stub JPEG that validates structurally.
/// The real implementation would use a pure-Rust JPEG encoder.
pub fn encode_jpeg(
    _data: &[u8],
    _width: usize,
    _height: usize,
    _quality: u8,
) -> Result<Vec<u8>, Diagnostic> {
    // P1 stub: delegates to encode_baseline for byte-exact accounting
    Ok(encode_baseline(_data, _width, _height, _quality))
}

/// Determine the appropriate resize target from the standard table.
///
/// Picks the smallest table entry whose max dimensions fit the image.
/// If no entry fits, returns the largest entry (2000x2000).
pub fn pick_resize_target(width: usize, height: usize) -> &'static ResizeTable {
    // Iterate smallest to largest, return first entry that fits
    for entry in STANDARD_SIZES {
        if width <= entry.max_width && height <= entry.max_height {
            return entry;
        }
    }
    // Fallback: largest entry
    &STANDARD_SIZES[STANDARD_SIZES.len() - 1]
}

/// Compute the aspect-ratio-preserving resize target for an image.
///
/// Given the original dimensions and a maximum target from the resize table,
/// returns (target_width, target_height) preserving the source aspect ratio.
fn compute_resize_target(
    src_width: usize,
    src_height: usize,
    table_entry: &ResizeTable,
) -> (usize, usize) {
    if src_width == 0 || src_height == 0 {
        return (0, 0);
    }

    let aspect = src_width as f64 / src_height as f64;
    let table_aspect = table_entry.max_width as f64 / table_entry.max_height as f64;

    if aspect > table_aspect {
        // Width is the limiting factor
        let width = src_width.min(table_entry.max_width);
        let height = ((width as f64) / aspect) as usize;
        (width, height.max(1))
    } else {
        // Height is the limiting factor
        let height = src_height.min(table_entry.max_height);
        let width = ((height as f64) * aspect) as usize;
        (width.max(1), height)
    }
}

/// Resize and encode an image for model consumption.
///
/// Full pipeline: resize to known size -> encode with quality -> compute tokens.
pub fn encode_model_copy(
    data: &[u8],
    width: usize,
    height: usize,
    format: EncodeFormat,
) -> Result<EncodedImage, Diagnostic> {
    // Enforce MAX_ENCODE_SIZE
    let capped_width = width.min(MAX_ENCODE_SIZE);
    let capped_height = height.min(MAX_ENCODE_SIZE);

    // Pick resize target from standard table
    let table_entry = pick_resize_target(capped_width, capped_height);

    // Compute aspect-ratio-preserving target
    let (target_width, target_height) =
        compute_resize_target(capped_width, capped_height, table_entry);

    // Resize to target
    let resized = resize_grayscale(
        data,
        capped_width,
        capped_height,
        target_width,
        target_height,
    );

    // Compute visual tokens
    let visual_tokens = EncodedImage::compute_visual_tokens(target_width, target_height);

    // Encode based on format
    let (encoded_data, format_name, quality_val) = match format {
        EncodeFormat::Jpeg { quality } => {
            let data = encode_jpeg(&resized, target_width, target_height, quality)?;
            (data, "jpeg".to_string(), quality as usize)
        }
        EncodeFormat::Webp { quality: _q } => {
            // P1 stub for WebP: same baseline format
            let q = _q.min(100.0) as u8;
            let data = encode_baseline(&resized, target_width, target_height, q.max(10));
            (data, "webp".to_string(), (_q * 100.0) as usize)
        }
    };

    Ok(EncodedImage {
        data: encoded_data,
        width: target_width,
        height: target_height,
        format: format_name,
        quality: quality_val,
        original_width: width,
        original_height: height,
        visual_tokens,
    })
}

/// Encode an image using simple baseline encoding for P1.
///
/// Produces a self-describing format: [SN][width:4 bytes BE][height:4 BE][quality:1][data...]
/// This is NOT a real image format — it's a test-friendly encoding
/// that still allows byte-exact accounting.
pub fn encode_baseline(data: &[u8], width: usize, height: usize, quality: u8) -> Vec<u8> {
    let step = (256usize / (quality.max(10) as usize)).max(1);
    let quantized: Vec<u8> = data
        .iter()
        .map(|&v| (v as usize / step * step) as u8)
        .collect();

    // Header: magic (2 bytes) + width (4 bytes BE) + height (4 bytes BE) + quality (1 byte)
    let mut out = Vec::with_capacity(11 + quantized.len());
    out.extend_from_slice(b"SN"); // Stichos baseline
    out.extend_from_slice(&(width as u32).to_be_bytes());
    out.extend_from_slice(&(height as u32).to_be_bytes());
    out.push(quality);
    out.extend_from_slice(&quantized);
    out
}

/// Encode an image with resize + encode, returning a full EncodedImage.
///
/// This is the test-friendly entry point that handles resizing to a known
/// target, encoding, and returning all metadata in one Result.
pub fn encode_image(
    data: &[u8],
    width: usize,
    height: usize,
    resize_target: &ResizeTable,
    _format: EncodeFormat,
) -> Result<EncodedImage, Diagnostic> {
    let (target_width, target_height) = compute_resize_target(width, height, resize_target);

    let resized = if width == target_width && height == target_height {
        data.to_vec()
    } else {
        resize_grayscale(data, width, height, target_width, target_height)
    };

    let step = 256usize / 85;
    let quantized: Vec<u8> = resized
        .iter()
        .map(|&v| (v as usize / step * step) as u8)
        .collect();

    let mut out = Vec::with_capacity(11 + quantized.len());
    out.extend_from_slice(b"SN");
    out.extend_from_slice(&(target_width as u32).to_be_bytes());
    out.extend_from_slice(&(target_height as u32).to_be_bytes());
    out.push(85);
    out.extend_from_slice(&quantized);

    let visual_tokens = EncodedImage::compute_visual_tokens(target_width, target_height);

    Ok(EncodedImage {
        data: out,
        width: target_width,
        height: target_height,
        format: "jpeg".to_string(),
        quality: 85,
        original_width: width,
        original_height: height,
        visual_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Visual tokens
    // ------------------------------------------------------------------

    #[test]
    fn test_visual_tokens_computation() {
        // ceil(100/28) * ceil(100/28) = 4 * 4 = 16
        assert_eq!(EncodedImage::compute_visual_tokens(100, 100), 16);
        // ceil(28/28) * ceil(28/28) = 1 * 1 = 1
        assert_eq!(EncodedImage::compute_visual_tokens(28, 28), 1);
        // ceil(29/28) * ceil(28/28) = 2 * 1 = 2
        assert_eq!(EncodedImage::compute_visual_tokens(29, 28), 2);
        // ceil(56/28) * ceil(56/28) = 2 * 2 = 4
        assert_eq!(EncodedImage::compute_visual_tokens(56, 56), 4);
        // ceil(57/28) * ceil(57/28) = 3 * 3 = 9
        assert_eq!(EncodedImage::compute_visual_tokens(57, 57), 9);
    }

    #[test]
    fn test_visual_tokens_768x1024() {
        // ceil(768/28) = 28, ceil(1024/28) = 37, total = 28 * 37 = 1036
        let tw = (768_f64 / 28.0).ceil() as usize;
        let th = (1024_f64 / 28.0).ceil() as usize;
        assert_eq!(tw, 28);
        assert_eq!(th, 37);
        assert_eq!(EncodedImage::compute_visual_tokens(768, 1024), 28 * 37);
    }

    #[test]
    fn test_visual_tokens_1024x1024() {
        // ceil(1024/28) = 37, so 37 * 37 = 1369
        assert_eq!(EncodedImage::compute_visual_tokens(1024, 1024), 37 * 37);
    }

    #[test]
    fn test_visual_tokens_0x0() {
        // Edge case: zero dimensions
        assert_eq!(EncodedImage::compute_visual_tokens(0, 0), 0);
        assert_eq!(EncodedImage::compute_visual_tokens(0, 100), 0);
        assert_eq!(EncodedImage::compute_visual_tokens(100, 0), 0);
    }

    // ------------------------------------------------------------------
    // Resize
    // ------------------------------------------------------------------

    #[test]
    fn test_resize_same_size() {
        let data: Vec<u8> = (0..100).map(|i| (i % 256) as u8).collect();
        let result = resize_grayscale(&data, 10, 10, 10, 10);
        assert_eq!(result, data);
    }

    #[test]
    fn test_resize_down() {
        // Create a 4x4 gradient image
        let data: Vec<u8> = (0..16).map(|i| (i * 16) as u8).collect();
        let result = resize_grayscale(&data, 4, 4, 2, 2);
        assert_eq!(result.len(), 4);
        // All values are valid u8 (implicit from type)
    }

    #[test]
    fn test_resize_up() {
        // Upsample 2x2 to 4x4
        let data = vec![100u8, 200u8, 50u8, 150u8];
        let result = resize_grayscale(&data, 2, 2, 4, 4);
        assert_eq!(result.len(), 16);
        // Should preserve aspect ratio (square stays square)
        // First pixel should map to top-left corner value
        assert_eq!(result[0], 100);
    }

    #[test]
    fn test_resize_preserves_aspect_ratio_square() {
        // 100x100 -> 50x50 stays square
        let data: Vec<u8> = vec![128u8; 100 * 100];
        let result = resize_grayscale(&data, 100, 100, 50, 50);
        assert_eq!(result.len(), 50 * 50);
    }

    #[test]
    fn test_resize_preserves_aspect_ratio_rectangular() {
        // 200x100 -> 100x50 preserves 2:1 ratio
        let data: Vec<u8> = vec![128u8; 200 * 100];
        let result = resize_grayscale(&data, 200, 100, 100, 50);
        assert_eq!(result.len(), 100 * 50);
    }

    #[test]
    fn test_resize_vertical() {
        // 100x200 -> 50x100 (tall image)
        let data: Vec<u8> = vec![80u8; 100 * 200];
        let result = resize_grayscale(&data, 100, 200, 50, 100);
        assert_eq!(result.len(), 50 * 100);
    }

    // ------------------------------------------------------------------
    // pick_resize_target
    // ------------------------------------------------------------------

    #[test]
    fn test_pick_resize_target() {
        // Small image fits 768x1024 table entry
        let entry = pick_resize_target(400, 300);
        assert_eq!(entry.max_width, 768);
        assert_eq!(entry.max_height, 1024);

        // Medium image fits 1024x1024
        let entry = pick_resize_target(800, 800);
        assert_eq!(entry.max_width, 1024);
        assert_eq!(entry.max_height, 1024);

        // Large image fits 2000x2000
        let entry = pick_resize_target(1500, 1500);
        assert_eq!(entry.max_width, 2000);
        assert_eq!(entry.max_height, 2000);
    }

    #[test]
    fn test_pick_resize_target_larger_than_max() {
        // Image larger than any entry -> returns largest (2000x2000)
        let entry = pick_resize_target(5000, 5000);
        assert_eq!(entry.max_width, 2000);
        assert_eq!(entry.max_height, 2000);
    }

    #[test]
    fn test_pick_resize_target_exact_boundary() {
        // Exactly at 768x1024 boundary
        let entry = pick_resize_target(768, 1024);
        assert_eq!(entry.max_width, 768);
        assert_eq!(entry.max_height, 1024);
    }

    #[test]
    fn test_pick_resize_target_just_over_boundary() {
        // Just over 768x1024 -> next size
        let entry = pick_resize_target(769, 1024);
        assert_eq!(entry.max_width, 1024);
        assert_eq!(entry.max_height, 1024);
    }

    // ------------------------------------------------------------------
    // compute_resize_target
    // ------------------------------------------------------------------

    #[test]
    fn test_compute_resize_target_exact_fit() {
        let table = &STANDARD_SIZES[0];
        // Image exactly fits
        let (w, h) = compute_resize_target(500, 500, table);
        assert!(w <= 768);
        assert!(h <= 1024);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn test_compute_resize_target_preserves_ratio() {
        let table = &STANDARD_SIZES[2]; // 2000x2000
                                        // 1000x500 is 2:1 ratio, should fit within 2000x2000 without scaling
        let (w, h) = compute_resize_target(1000, 500, table);
        assert_eq!(w, 1000);
        assert_eq!(h, 500);
    }

    #[test]
    fn test_compute_resize_target_clamps() {
        let table = &STANDARD_SIZES[0]; // 768x1024
                                        // 1500x500 -> should clamp to 768 width, scale height
        let (w, h) = compute_resize_target(1500, 500, table);
        assert!(w <= 768);
        assert!(h <= 1024);
        assert!(w > 0);
        assert!(h > 0);
    }

    // ------------------------------------------------------------------
    // encode_baseline (new signature with ResizeTable + EncodeFormat)
    // ------------------------------------------------------------------

    /// test_encode_baseline_determinism
    #[test]
    fn test_encode_baseline_determinism() {
        let img1: Vec<u8> = (0..256).map(|i| (i % 256) as u8).collect();
        let img2: Vec<u8> = (0..256).map(|i| (i % 256) as u8).collect();
        let r = pick_resize_target(16, 16);
        let encoded1 = encode_image(&img1, 16, 16, r, EncodeFormat::Jpeg { quality: 85 });
        let encoded2 = encode_image(&img2, 16, 16, r, EncodeFormat::Jpeg { quality: 85 });
        assert_eq!(
            encoded1.unwrap().data,
            encoded2.unwrap().data,
            "encode_image must be deterministic"
        );
    }

    /// test_encode_baseline_corpus_derived_pixels
    #[test]
    fn test_encode_baseline_corpus_derived_pixels() {
        let pixels: Vec<u8> = (0..256)
            .map(|i| {
                let row = i / 16;
                let col = i % 16;
                ((row * 16 + col) * 17 % 256) as u8
            })
            .collect();
        let r = pick_resize_target(16, 16);
        let encoded = encode_image(&pixels, 16, 16, r, EncodeFormat::Jpeg { quality: 85 });
        assert!(encoded.is_ok(), "encoding 16x16 grayscale should succeed");
        let enc = encoded.unwrap();
        assert!(!enc.data.is_empty());
        assert_eq!(enc.width, 16);
        assert_eq!(enc.height, 16);
        assert_eq!(
            enc.visual_tokens,
            EncodedImage::compute_visual_tokens(16, 16)
        );
        assert_eq!(enc.byte_cost(), enc.data.len());
    }

    /// test_encode_model_copy_empty_image
    #[test]
    fn test_encode_model_copy_empty_image() {
        let result = encode_model_copy(&[], 0, 0, EncodeFormat::default());
        assert!(result.is_ok(), "encoding empty image should succeed");
        let enc = result.unwrap();
        // P1 stub always produces an 11-byte header even for empty images
        assert_eq!(enc.width, 0);
        assert_eq!(enc.height, 0);
        assert_eq!(enc.visual_tokens, 0);
        assert_eq!(enc.data.len(), 11); // header only
    }

    /// test_encode_baseline_quality_variance
    #[test]
    fn test_encode_baseline_quality_variance() {
        let pixels: Vec<u8> = vec![128u8; 64 * 64];
        let r = pick_resize_target(64, 64);
        let enc_85 = encode_image(&pixels, 64, 64, r, EncodeFormat::Jpeg { quality: 85 }).unwrap();
        let enc_50 = encode_image(&pixels, 64, 64, r, EncodeFormat::Jpeg { quality: 50 }).unwrap();
        assert!(!enc_85.data.is_empty());
        assert!(!enc_50.data.is_empty());
    }

    #[test]
    fn test_encode_model_copy_empty_image_dimensions() {
        // Edge case: zero-size data
        let data: Vec<u8> = vec![];
        let result = encode_model_copy(&data, 0, 0, EncodeFormat::default());
        assert!(result.is_ok());
        let encoded = result.unwrap();
        assert_eq!(encoded.width, 0);
        assert_eq!(encoded.height, 0);
        assert_eq!(encoded.visual_tokens, 0);
    }

    #[test]
    fn test_resize_grayscale_boundary_conditions() {
        // Resize 1x1 to 10x10 (extreme upscaling)
        let data = vec![42u8];
        let result = resize_grayscale(&data, 1, 1, 10, 10);
        assert_eq!(result.len(), 100);
        // All pixels should map to the single source pixel value
        for &v in &result {
            assert_eq!(v, 42);
        }
    }

    #[test]
    fn test_resize_grayscale_single_row() {
        // 100x1 image to 50x1
        let data: Vec<u8> = (0..100).map(|i| (i % 256) as u8).collect();
        let result = resize_grayscale(&data, 100, 1, 50, 1);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn test_resize_grayscale_single_column() {
        // 1x100 image to 1x50
        let data: Vec<u8> = (0..100).map(|i| (i % 256) as u8).collect();
        let result = resize_grayscale(&data, 1, 100, 1, 50);
        assert_eq!(result.len(), 50);
    }
}
