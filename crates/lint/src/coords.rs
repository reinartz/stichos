// SPDX-License-Identifier: MIT OR Apache-2.0
//! Coordinate bookkeeping: convert between points (pt) and pixels (px).
//!
//! Per concept §10-5: the model returns pixel coords in resized image space.
//! We must track scale factors and provide bidirectional conversion.

/// Points per inch (standard).
pub const PT_PER_INCH: f64 = 72.0;

/// DPI (dots per inch) for master renders.
pub const MASTER_DPI: f64 = 150.0;

/// DPI for detail crops (inspect_region).
pub const DETAIL_DPI: f64 = 300.0;

/// A coordinate conversion context that tracks scale from pt to px.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CoordContext {
    /// Source DPI (e.g., 150 for master renders).
    pub source_dpi: f64,
    /// Target DPI or resize DPI (e.g., 150 or 300 for crops).
    pub target_dpi: f64,
    /// Image width in pixels (after resize).
    pub image_width_px: usize,
    /// Image height in pixels (after resize),
    pub image_height_px: usize,
}

impl CoordContext {
    /// Create a new coordinate context for a given image size and DPI.
    pub fn new(dpi: f64, width_px: usize, height_px: usize) -> Self {
        Self {
            source_dpi: dpi,
            target_dpi: dpi,
            image_width_px: width_px,
            image_height_px: height_px,
        }
    }

    /// Create a master render context (150 DPI).
    pub fn master(width_px: usize, height_px: usize) -> Self {
        Self::new(MASTER_DPI, width_px, height_px)
    }

    /// Create a detail crop context (300 DPI).
    pub fn detail(width_px: usize, height_px: usize) -> Self {
        Self::new(DETAIL_DPI, width_px, height_px)
    }

    /// Convert from points to pixels.
    /// px = pt * dpi / 72.0
    pub fn pt_to_px(&self, pt: f64) -> f64 {
        pt * self.target_dpi / PT_PER_INCH
    }

    /// Convert from pixels to points.
    /// pt = px * 72.0 / dpi
    pub fn px_to_pt(&self, px: f64) -> f64 {
        px * PT_PER_INCH / self.target_dpi
    }

    /// Convert a bounding box from points to pixels.
    /// bbox_pt = [x_pt, y_pt, width_pt, height_pt]
    /// bbox_px = [x_px, y_px, width_px, height_px]
    pub fn bbox_pt_to_px(&self, bbox_pt: [f64; 4]) -> [f64; 4] {
        [
            self.pt_to_px(bbox_pt[0]),
            self.pt_to_px(bbox_pt[1]),
            self.pt_to_px(bbox_pt[2]),
            self.pt_to_px(bbox_pt[3]),
        ]
    }

    /// Convert a bounding box from pixels to points.
    pub fn bbox_px_to_pt(&self, bbox_px: [f64; 4]) -> [f64; 4] {
        [
            self.px_to_pt(bbox_px[0]),
            self.px_to_pt(bbox_px[1]),
            self.px_to_pt(bbox_px[2]),
            self.px_to_pt(bbox_px[3]),
        ]
    }

    /// Get the horizontal scale factor (px per pt).
    pub fn scale_x(&self) -> f64 {
        self.target_dpi / PT_PER_INCH
    }

    /// Get the vertical scale factor (px per pt).
    pub fn scale_y(&self) -> f64 {
        self.target_dpi / PT_PER_INCH
    }

    /// Get the overall scale factor (assumes square pixels).
    pub fn scale(&self) -> f64 {
        self.scale_x()
    }
}

/// A coordinate conversion record attached to a payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CoordRecord {
    /// Scale factor used (px per pt).
    pub scale: f64,
    /// Source DPI.
    pub source_dpi: f64,
    /// Target DPI.
    pub target_dpi: f64,
    /// Image dimensions in pixels.
    pub image_width_px: usize,
    pub image_height_px: usize,
    /// Page dimensions in points.
    pub page_width_pt: f64,
    pub page_height_pt: f64,
}

impl CoordRecord {
    /// Create a coordinate record from a context and page dimensions.
    pub fn from_context(ctx: &CoordContext, page_width_pt: f64, page_height_pt: f64) -> Self {
        Self {
            scale: ctx.scale(),
            source_dpi: ctx.source_dpi,
            target_dpi: ctx.target_dpi,
            image_width_px: ctx.image_width_px,
            image_height_px: ctx.image_height_px,
            page_width_pt,
            page_height_pt,
        }
    }

    /// Convert a bbox from pt to px using this record.
    pub fn bbox_pt_to_px(&self, bbox_pt: [f64; 4]) -> [f64; 4] {
        [
            bbox_pt[0] * self.scale,
            bbox_pt[1] * self.scale,
            bbox_pt[2] * self.scale,
            bbox_pt[3] * self.scale,
        ]
    }

    /// Convert a bbox from px to pt using this record.
    pub fn bbox_px_to_pt(&self, bbox_px: [f64; 4]) -> [f64; 4] {
        [
            bbox_px[0] / self.scale,
            bbox_px[1] / self.scale,
            bbox_px[2] / self.scale,
            bbox_px[3] / self.scale,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip pt→px→pt within 1px tolerance.
    #[test]
    fn test_pt_to_px_and_back() {
        let ctx = CoordContext::master(1024, 768);
        let values = [0.0, 0.5, 1.0, 72.0, 100.0, 500.0, 12.75, 612.0];
        for pt in values {
            let px = ctx.pt_to_px(pt);
            let back = ctx.px_to_pt(px);
            let diff = (back - pt).abs();
            assert!(
                diff <= 1.0,
                "pt {} → px {} → pt {} (diff {})",
                pt,
                px,
                back,
                diff
            );
        }
    }

    /// Reverse round-trip px→pt→px within 0.01pt tolerance.
    #[test]
    fn test_px_to_pt_and_back() {
        let ctx = CoordContext::master(1024, 768);
        let values: Vec<f64> = (0..=2000).step_by(7).map(|x| x as f64).collect();
        for px in values {
            let pt = ctx.px_to_pt(px);
            let back = ctx.pt_to_px(pt);
            let diff = (back - px).abs();
            assert!(
                diff <= 0.01,
                "px {} → pt {} → px {} (diff {})",
                px,
                pt,
                back,
                diff
            );
        }
    }

    /// Full bbox pt→px round-trip via CoordContext.
    #[test]
    fn test_bbox_conversion_pt_to_px() {
        let ctx = CoordContext::master(1024, 768);
        let bbox: [f64; 4] = [10.0, 20.0, 100.0, 200.0];
        let px = ctx.bbox_pt_to_px(bbox);
        let back = ctx.bbox_px_to_pt(px);
        for (original, converted) in bbox.iter().zip(back.iter()) {
            let diff = (converted - original).abs();
            assert!(
                diff <= 1.0,
                "bbox pt {} → px {} → pt {} (diff {})",
                original,
                original * ctx.scale(),
                converted,
                diff
            );
        }
    }

    /// Reverse bbox round-trip px→pt→px.
    #[test]
    fn test_bbox_conversion_px_to_pt() {
        let ctx = CoordContext::master(1024, 768);
        let bbox_px: [f64; 4] = [72.0, 144.0, 360.0, 720.0];
        let pt = ctx.bbox_px_to_pt(bbox_px);
        let back = ctx.bbox_pt_to_px(pt);
        for (original, converted) in bbox_px.iter().zip(back.iter()) {
            let diff = (converted - original).abs();
            assert!(
                diff <= 0.01,
                "bbox px {} → pt {} → px {} (diff {})",
                original,
                original / ctx.scale(),
                converted,
                diff
            );
        }
    }

    /// 150 DPI scale factor = 150/72 = 2.0833...
    #[test]
    fn test_scale_factors_150dpi() {
        let ctx = CoordContext::master(1024, 768);
        let expected = MASTER_DPI / PT_PER_INCH;
        assert!((ctx.scale() - expected).abs() < 1e-9);
        assert!((ctx.scale_x() - expected).abs() < 1e-9);
        assert!((ctx.scale_y() - expected).abs() < 1e-9);
    }

    /// 300 DPI scale factor = 300/72 = 4.1666...
    #[test]
    fn test_scale_factors_300dpi() {
        let ctx = CoordContext::detail(2048, 1536);
        let expected = DETAIL_DPI / PT_PER_INCH;
        assert!((ctx.scale() - expected).abs() < 1e-9);
        assert!((ctx.scale_x() - expected).abs() < 1e-9);
        assert!((ctx.scale_y() - expected).abs() < 1e-9);
    }

    /// CoordContext round-trips through serde.
    #[test]
    fn test_coord_context_serde() {
        let ctx = CoordContext::master(1024, 768);
        let json = serde_json::to_string(&ctx).unwrap();
        let recovered: CoordContext = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered.source_dpi, ctx.source_dpi);
        assert_eq!(recovered.target_dpi, ctx.target_dpi);
        assert_eq!(recovered.image_width_px, ctx.image_width_px);
        assert_eq!(recovered.image_height_px, ctx.image_height_px);
    }

    /// CoordRecord round-trips through serde.
    #[test]
    fn test_coord_record_serde() {
        let ctx = CoordContext::detail(2048, 1536);
        let record = CoordRecord::from_context(&ctx, 612.0, 792.0);
        let json = serde_json::to_string(&record).unwrap();
        let recovered: CoordRecord = serde_json::from_str(&json).unwrap();
        assert!((recovered.scale - record.scale).abs() < 1e-9);
        assert_eq!(recovered.source_dpi, record.source_dpi);
        assert_eq!(recovered.target_dpi, record.target_dpi);
        assert_eq!(recovered.image_width_px, record.image_width_px);
        assert_eq!(recovered.image_height_px, record.image_height_px);
        assert!((recovered.page_width_pt - record.page_width_pt).abs() < 1e-9);
        assert!((recovered.page_height_pt - record.page_height_pt).abs() < 1e-9);
    }

    /// CoordRecord bbox pt→px conversion is correct.
    #[test]
    fn test_coord_record_bbox_pt_to_px() {
        let ctx = CoordContext::master(1024, 768);
        let record = CoordRecord::from_context(&ctx, 612.0, 792.0);
        let bbox: [f64; 4] = [0.0, 0.0, 72.0, 72.0];
        let px = record.bbox_pt_to_px(bbox);
        let expected_scale = MASTER_DPI / PT_PER_INCH;
        assert!((px[0] - 0.0).abs() < 1e-9);
        assert!((px[1] - 0.0).abs() < 1e-9);
        assert!((px[2] - 72.0 * expected_scale).abs() < 1e-9);
        assert!((px[3] - 72.0 * expected_scale).abs() < 1e-9);
    }

    /// Zero points converts to zero pixels.
    #[test]
    fn test_zero_pt_converts_to_zero_px() {
        let ctx = CoordContext::master(1024, 768);
        assert!((ctx.pt_to_px(0.0)).abs() < 1e-9);
        assert!((ctx.px_to_pt(0.0)).abs() < 1e-9);
        let bbox: [f64; 4] = [0.0, 0.0, 0.0, 0.0];
        let px = ctx.bbox_pt_to_px(bbox);
        for v in px {
            assert!((v).abs() < 1e-9);
        }
    }

    /// Large coordinates (10000 pt) round-trip.
    #[test]
    fn test_large_coordinates() {
        let ctx = CoordContext::master(1024, 768);
        let pt = 10000.0;
        let px = ctx.pt_to_px(pt);
        let back = ctx.px_to_pt(px);
        let diff = (back - pt).abs();
        assert!(
            diff <= 1.0,
            "large pt {} → px {} → pt {} (diff {})",
            pt,
            px,
            back,
            diff
        );
    }

    /// Master context produces correct values.
    #[test]
    fn test_master_context() {
        let ctx = CoordContext::master(1024, 768);
        assert_eq!(ctx.source_dpi, MASTER_DPI);
        assert_eq!(ctx.target_dpi, MASTER_DPI);
        assert_eq!(ctx.image_width_px, 1024);
        assert_eq!(ctx.image_height_px, 768);
        let expected_scale = MASTER_DPI / PT_PER_INCH;
        assert!((ctx.scale() - expected_scale).abs() < 1e-9);
    }

    /// Detail context produces correct values.
    #[test]
    fn test_detail_context() {
        let ctx = CoordContext::detail(2048, 1536);
        assert_eq!(ctx.source_dpi, DETAIL_DPI);
        assert_eq!(ctx.target_dpi, DETAIL_DPI);
        assert_eq!(ctx.image_width_px, 2048);
        assert_eq!(ctx.image_height_px, 1536);
        let expected_scale = DETAIL_DPI / PT_PER_INCH;
        assert!((ctx.scale() - expected_scale).abs() < 1e-9);
    }

    /// Multiple values: pt→px→pt and px→pt→px for several test points.
    #[test]
    fn test_bbox_pt_to_px_properties() {
        let ctx = CoordContext::master(1024, 768);
        let test_cases: Vec<[f64; 4]> = vec![
            [0.0, 0.0, 100.0, 100.0],
            [72.0, 72.0, 72.0, 72.0],
            [0.0, 0.0, 612.0, 792.0],
            [1.0, 1.0, 1.0, 1.0],
            [36.5, 73.0, 18.25, 36.5],
            [500.0, 600.0, 100.0, 200.0],
        ];
        for bbox in test_cases {
            let px = ctx.bbox_pt_to_px(bbox);
            let back = ctx.bbox_px_to_pt(px);
            for (i, (original, converted)) in bbox.iter().zip(back.iter()).enumerate() {
                let diff = (converted - original).abs();
                assert!(
                    diff <= 1.0,
                    "bbox[{}] pt {} → px {} → pt {} (diff {})",
                    i,
                    original,
                    px[i],
                    converted,
                    diff
                );
            }
        }
    }
}
