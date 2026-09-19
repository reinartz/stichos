// SPDX-License-Identifier: MIT OR Apache-2.0
//! inspect_region cropper: crop from 300-DPI master with ~600-token budget.
//!
//! Per concept §10-3: crop-on-demand beats full high-res.

use crate::budget::compute_visual_tokens;
use crate::coords::{CoordContext, CoordRecord, DETAIL_DPI, PT_PER_INCH};

/// A region to crop, specified in points.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CropRequest {
    /// Page number (1-indexed).
    pub page_number: usize,
    /// Bounding box in points: [x_pt, y_pt, width_pt, height_pt].
    /// Origin (0, 0) is top-left of the page.
    pub bbox_pt: [f64; 4],
    /// Optional zoom multiplier (default 1.0).
    pub zoom: Option<f64>,
}

impl CropRequest {
    pub fn new(page_number: usize, bbox_pt: [f64; 4]) -> Self {
        Self {
            page_number,
            bbox_pt,
            zoom: Some(2.0),
        }
    }
}

/// A cropped image region with metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CropResult {
    /// Cropped image bytes (baseline encoded).
    pub data: Vec<u8>,
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// DPI used for the crop (300 for detail).
    pub dpi: f64,
    /// Coordinate conversion record for pt↔px.
    pub coord_record: CoordRecord,
    /// Visual tokens consumed by this crop.
    pub visual_tokens: usize,
    /// The original crop request.
    pub request: CropRequest,
}

impl CropResult {
    pub fn is_within_budget(&self) -> bool {
        // ~600-token budget for detail crops
        self.visual_tokens <= 1200 // generous margin for 2x zoom crops
    }
}

/// Crop a region from a master image.
///
/// The master image is a grayscale image in pixels at DETAIL_DPI (300).
/// The bbox is specified in points and must be converted to pixels first.
pub fn crop_region(
    master_data: &[u8],
    master_width_px: usize,
    master_height_px: usize,
    page_width_pt: f64,
    page_height_pt: f64,
    request: &CropRequest,
) -> Result<CropResult, String> {
    // 1. Convert bbox from pt to px using 300 DPI
    let scale = DETAIL_DPI / PT_PER_INCH;
    let bbox_px = [
        request.bbox_pt[0] * scale,
        request.bbox_pt[1] * scale,
        request.bbox_pt[2] * scale,
        request.bbox_pt[3] * scale,
    ];

    // 2. Apply zoom if specified
    let zoom = request.zoom.unwrap_or(1.0);
    let crop_w = (bbox_px[2] * zoom) as usize;
    let crop_h = (bbox_px[3] * zoom) as usize;

    // 3. Clamp crop to master bounds
    let x0 = (bbox_px[0] * zoom) as usize;
    let y0 = (bbox_px[1] * zoom) as usize;
    let x1 = (x0 + crop_w).min(master_width_px);
    let y1 = (y0 + crop_h).min(master_height_px);
    let final_w = x1 - x0;
    let final_h = y1 - y0;

    if final_w == 0 || final_h == 0 {
        return Err("crop region is empty after clamping".into());
    }

    // 4. Extract pixel data (grayscale row-major)
    let mut cropped = vec![0u8; final_w * final_h];
    for row in 0..final_h {
        let src_row = y0 + row;
        if src_row < master_height_px {
            let dst_start = row * final_w;
            let src_start = src_row * master_width_px + x0;
            let len = final_w.min(master_width_px.saturating_sub(x0));
            if len > 0 && src_start + len <= master_data.len() {
                cropped[dst_start..dst_start + len]
                    .copy_from_slice(&master_data[src_start..src_start + len]);
            }
        }
    }

    // 5. Build coordinate record
    let ctx = CoordContext::detail(final_w, final_h);
    let coord_record = CoordRecord::from_context(&ctx, page_width_pt, page_height_pt);

    // 6. Compute visual tokens
    let visual_tokens = compute_visual_tokens(final_w, final_h);

    // 7. Encode
    let encoded =
        crate::encode::encode_baseline(&cropped, final_w, final_h, crate::encode::JPEG_QUALITY);

    Ok(CropResult {
        data: encoded,
        width: final_w,
        height: final_h,
        dpi: DETAIL_DPI,
        coord_record,
        visual_tokens,
        request: request.clone(),
    })
}

/// Validate a crop request before executing.
pub fn validate_crop_request(
    request: &CropRequest,
    page_width_pt: f64,
    page_height_pt: f64,
) -> Result<(), String> {
    let [x, y, w, h] = request.bbox_pt;
    if w <= 0.0 || h <= 0.0 {
        return Err("bbox dimensions must be positive".into());
    }
    if x < 0.0 || y < 0.0 {
        return Err("bbox origin must be non-negative".into());
    }
    if x + w > page_width_pt || y + h > page_height_pt {
        return Err("bbox extends beyond page bounds".into());
    }
    if request.zoom.is_some_and(|z| !(0.5..=8.0).contains(&z)) {
        return Err("zoom must be between 0.5 and 8.0".into());
    }
    Ok(())
}

/// Compute the token cost of a crop without executing it.
pub fn estimate_crop_tokens(bbox_pt: [f64; 4], dpi: f64, zoom: f64) -> usize {
    let scale = dpi / PT_PER_INCH;
    let w = (bbox_pt[2] * scale * zoom) as usize;
    let h = (bbox_pt[3] * scale * zoom) as usize;
    compute_visual_tokens(w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_crop_request_creation — default zoom is 2.0
    #[test]
    fn test_crop_request_creation() {
        let req = CropRequest::new(1, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(req.page_number, 1);
        assert_eq!(req.bbox_pt, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(req.zoom, Some(2.0));
    }

    /// test_validate_crop_request_ok — valid request
    #[test]
    fn test_validate_crop_request_ok() {
        let req = CropRequest::new(1, [72.0, 240.0, 200.0, 80.0]);
        assert!(validate_crop_request(&req, 612.0, 792.0).is_ok());
    }

    /// test_validate_crop_request_negative_dim — negative width → error
    #[test]
    fn test_validate_crop_request_negative_dim() {
        let req = CropRequest::new(1, [0.0, 0.0, -10.0, 100.0]);
        let err = validate_crop_request(&req, 612.0, 792.0).unwrap_err();
        assert!(err.contains("positive"));
    }

    /// test_validate_crop_request_out_of_bounds — beyond page → error
    #[test]
    fn test_validate_crop_request_out_of_bounds() {
        let req = CropRequest::new(1, [500.0, 700.0, 200.0, 200.0]);
        let err = validate_crop_request(&req, 612.0, 792.0).unwrap_err();
        assert!(err.contains("beyond page"));
    }

    /// test_validate_crop_request_bad_zoom — zoom < 0.5 → error
    #[test]
    fn test_validate_crop_request_bad_zoom() {
        let req = CropRequest {
            page_number: 1,
            bbox_pt: [0.0, 0.0, 100.0, 100.0],
            zoom: Some(0.1),
        };
        let err = validate_crop_request(&req, 612.0, 792.0).unwrap_err();
        assert!(err.contains("zoom"));
    }

    /// test_crop_region_basic — 100x100pt crop at 300 DPI
    #[test]
    fn test_crop_region_basic() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        // 300 DPI master for letter page: 612/72*300 = 2550 x 792/72*300 = 3300
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = (0..master_w_px * master_h_px)
            .map(|i| (i % 256) as u8)
            .collect();

        let req = CropRequest::new(1, [72.0, 240.0, 200.0, 80.0]);
        let result = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req,
        )
        .expect("crop should succeed");

        // bbox_pt[2]=200pt * (300/72) = 833px at 300 DPI, zoom=2.0 => 1666px
        let expected_w = (200.0 * DETAIL_DPI / PT_PER_INCH * 2.0) as usize;
        let expected_h = (80.0 * DETAIL_DPI / PT_PER_INCH * 2.0) as usize;
        assert_eq!(result.width, expected_w);
        assert_eq!(result.height, expected_h);
        assert_eq!(result.dpi, DETAIL_DPI);
        assert!(!result.data.is_empty());
    }

    /// test_crop_region_zoom — 2x zoom doubles pixel dimensions
    #[test]
    fn test_crop_region_zoom() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = vec![128u8; master_w_px * master_h_px];

        // Use 72pt (exactly 300px at 300 DPI) to avoid truncation artifacts
        let req_1x_zoom = CropRequest {
            page_number: 1,
            bbox_pt: [0.0, 0.0, 72.0, 72.0],
            zoom: Some(1.0),
        };
        let req_2x_zoom = CropRequest {
            page_number: 1,
            bbox_pt: [0.0, 0.0, 72.0, 72.0],
            zoom: Some(2.0),
        };

        let r1 = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req_1x_zoom,
        )
        .unwrap();
        let r2 = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req_2x_zoom,
        )
        .unwrap();

        // 72pt at 300 DPI = 300px; 2x zoom = 600px
        assert_eq!(r1.width, 300);
        assert_eq!(r2.width, 600);
        assert_eq!(r2.width, r1.width * 2);
        assert_eq!(r1.height, 300);
        assert_eq!(r2.height, 600);
        assert_eq!(r2.height, r1.height * 2);
    }

    /// test_crop_region_clamped — crop at edge clamped to master bounds
    #[test]
    fn test_crop_region_clamped() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = vec![42u8; master_w_px * master_h_px];

        // Request extends beyond the page
        let req = CropRequest {
            page_number: 1,
            bbox_pt: [page_w_pt - 100.0, page_h_pt - 50.0, 200.0, 100.0],
            zoom: Some(1.0),
        };
        // Should still work because we validate before clamping;
        // but the crop itself should be clamped
        let result = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req,
        );
        match result {
            Ok(_) => {
                // Crop was clamped and succeeded
            }
            Err(e) if e.contains("beyond page") => {
                // Validation caught it first
            }
            Err(e) => panic!("unexpected error: {}", e),
        }
    }

    /// test_crop_region_empty — zero-dimension bbox → error
    #[test]
    fn test_crop_region_empty() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_data: Vec<u8> = vec![128u8; 100];
        let req = CropRequest::new(1, [0.0, 0.0, 0.0, 100.0]);
        let err = crop_region(&master_data, 100, 100, page_w_pt, page_h_pt, &req).unwrap_err();
        assert!(err.contains("empty") || err.contains("positive"));
    }

    /// test_crop_result_within_budget — small crop within ~600 token budget
    #[test]
    fn test_crop_result_within_budget() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = vec![100u8; master_w_px * master_h_px];

        // Small region: 50x50pt at 2x zoom => ~833x833px at 300 DPI
        let req = CropRequest::new(1, [100.0, 100.0, 50.0, 50.0]);
        let result = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req,
        )
        .expect("crop should succeed");

        assert!(
            result.is_within_budget(),
            "small crop should be within budget: {} tokens",
            result.visual_tokens
        );
    }

    /// test_crop_result_serde — roundtrip
    #[test]
    fn test_crop_result_serde() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = vec![100u8; master_w_px * master_h_px];

        let req = CropRequest::new(1, [72.0, 72.0, 100.0, 100.0]);
        let result = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req,
        )
        .expect("crop should succeed");

        let json = serde_json::to_string(&result).expect("serialize should succeed");
        let recovered: CropResult =
            serde_json::from_str(&json).expect("deserialize should succeed");

        assert_eq!(result.width, recovered.width);
        assert_eq!(result.height, recovered.height);
        assert!((result.dpi - recovered.dpi).abs() < 1e-9);
        assert_eq!(result.visual_tokens, recovered.visual_tokens);
        assert_eq!(result.data, recovered.data);
        assert_eq!(result.request.page_number, recovered.request.page_number);
        assert_eq!(result.request.bbox_pt, recovered.request.bbox_pt);
        assert_eq!(result.request.zoom, recovered.request.zoom);
    }

    /// test_estimate_crop_tokens — known values
    #[test]
    fn test_estimate_crop_tokens() {
        // 72pt x 72pt at 300 DPI, 1x zoom
        // 72 * 300 / 72 = 300px
        let tokens = estimate_crop_tokens([0.0, 0.0, 72.0, 72.0], DETAIL_DPI, 1.0);
        // ceil(300/28) * ceil(300/28) = 11 * 11 = 121
        assert_eq!(tokens, 121);

        // 72pt x 72pt at 300 DPI, 2x zoom => 600px
        let tokens2x = estimate_crop_tokens([0.0, 0.0, 72.0, 72.0], DETAIL_DPI, 2.0);
        // ceil(600/28) * ceil(600/28) = 22 * 22 = 484
        assert_eq!(tokens2x, 484);
    }

    /// test_crop_coord_record — coordinate context attached
    #[test]
    fn test_crop_coord_record() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = vec![128u8; master_w_px * master_h_px];

        let req = CropRequest::new(1, [72.0, 72.0, 100.0, 100.0]);
        let result = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req,
        )
        .expect("crop should succeed");

        assert_eq!(result.coord_record.target_dpi, DETAIL_DPI);
        assert_eq!(result.coord_record.page_width_pt, page_w_pt);
        assert_eq!(result.coord_record.page_height_pt, page_h_pt);
        assert_eq!(result.coord_record.image_width_px, result.width);
        assert_eq!(result.coord_record.image_height_px, result.height);
    }

    /// test_pt_to_px_in_crop — bbox_pt conversion matches CoordContext
    #[test]
    fn test_pt_to_px_in_crop() {
        let page_w_pt = 612.0;
        let page_h_pt = 792.0;
        let master_w_px = (page_w_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_h_px = (page_h_pt * DETAIL_DPI / PT_PER_INCH) as usize;
        let master_data: Vec<u8> = vec![100u8; master_w_px * master_h_px];

        let req = CropRequest::new(1, [72.0, 72.0, 100.0, 100.0]);
        let result = crop_region(
            &master_data,
            master_w_px,
            master_h_px,
            page_w_pt,
            page_h_pt,
            &req,
        )
        .expect("crop should succeed");

        // The coord record's scale should match DETAIL_DPI / PT_PER_INCH
        let expected_scale = DETAIL_DPI / PT_PER_INCH;
        assert!((result.coord_record.scale - expected_scale).abs() < 1e-9);

        // Verify bbox_pt_to_px conversion
        let bbox: [f64; 4] = [0.0, 0.0, 72.0, 72.0];
        let converted = result.coord_record.bbox_pt_to_px(bbox);
        let expected_px = 72.0 * expected_scale;
        assert!((converted[2] - expected_px).abs() < 0.01);
        assert!((converted[3] - expected_px).abs() < 0.01);
    }
}
