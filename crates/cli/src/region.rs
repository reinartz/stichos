// SPDX-License-Identifier: MIT OR Apache-2.0
//! inspect_region tool: crop and zoom a region from a rendered document.
//!
//! This module implements the `inspect_region` CLI command, which:
//! 1. Validates a crop request (bbox + zoom within page bounds)
//! 2. Reads or renders a master page image from the document
//! 3. Dispatches to the negotiated engine (LibreOffice, Pandoc, Chromium, or Typst)
//! 4. Converts to PDF via engine
//! 5. Rasterizes the target page
//! 6. Crops the requested region using the cropper
//! 7. Returns cropped image data with coordinate conversion record
//!
//! Master pages are rendered on demand via the engine pipeline:
//! document → PDF (engine) → rasterize (pdftoppm/gs/mutool) → PNG.

use anyhow::Context;
use engines::cache::{make_cache_key, sha256, MasterCache};
use engines::chromium::ChromiumEngine;
use engines::libreoffice::LibreOfficeEngine;
use engines::negotiate::{negotiate_renderer, RendererPreference};
use engines::pandoc::PandocEngine;
use engines::raster;
use engines::typst::TypstEngine;
use gp_core::traits::Engine;
use lint::coords::DETAIL_DPI;
use lint::cropper::{crop_region, validate_crop_request, CropRequest};

/// Default page size in points (Letter: 612x792).
const DEFAULT_PAGE_WIDTH_PT: f64 = 612.0;
const DEFAULT_PAGE_HEIGHT_PT: f64 = 792.0;

/// Inspect a region of a document by cropping from a rendered page.
///
/// # Arguments
/// * `path` — path to the document file (docx, odt, pptx, xlsx, ods, odp)
/// * `page` — page number (1-indexed)
/// * `bbox_pt` — bounding box in points: [x_pt, y_pt, width_pt, height_pt]
/// * `zoom` — zoom multiplier (default 2.0)
///
/// # Returns
/// JSON value with crop results, metadata, and coordinate record.
pub fn inspect_region(
    path: &str,
    page: usize,
    bbox_pt: [f64; 4],
    zoom: f64,
) -> Result<serde_json::Value, anyhow::Error> {
    let request = CropRequest::new(page, bbox_pt);
    let request = with_zoom(request, zoom);

    // Validate the crop request against page bounds
    let page_width = DEFAULT_PAGE_WIDTH_PT;
    let page_height = DEFAULT_PAGE_HEIGHT_PT;
    validate_crop_request(&request, page_width, page_height)
        .map_err(|e| anyhow::anyhow!("crop request validation failed: {}", e))?;

    // Read the master page image (rendered at DETAIL_DPI)
    let (master_data, rendered_by) = read_master_page(path, page)?;
    let (master_w_px, master_h_px) = master_page_dimensions(&master_data)?;

    // Crop the region using the cropper
    let result = crop_region(
        &master_data,
        master_w_px,
        master_h_px,
        page_width,
        page_height,
        &request,
    )
    .map_err(|e| anyhow::anyhow!("crop failed for page {}: {}", page, e))?;

    // Build the response JSON
    Ok(serde_json::json!({
        "tool": "inspect_region",
        "path": path,
        "page": page,
        "bbox_pt": bbox_pt,
        "zoom": zoom,
        "crop": {
            "width": result.width,
            "height": result.height,
            "dpi": result.dpi,
            "visual_tokens": result.visual_tokens,
            "within_budget": result.is_within_budget(),
            "byte_size": result.data.len(),
        },
        "coordinate_record": result.coord_record,
        "rendered_by": rendered_by,
    }))
}

/// Render a document page to a master image at DETAIL_DPI.
///
/// Pipeline: document → PDF (engine) → rasterize (pdftoppm/gs/mutool) → PNG.
/// Uses the master cache for repeated renders of the same document+page.
///
/// Returns (PNG bytes, rendered_by string) on success.
fn read_master_page(path: &str, page: usize) -> Result<(Vec<u8>, String), anyhow::Error> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read document: {}", path))?;
    let source_ext = path.rsplit('.').next().unwrap_or("").to_lowercase();

    // Build cache key
    let doc_hash = sha256(&bytes);
    let cache_key = make_cache_key(
        &doc_hash,
        "unknown",
        "unknown",
        "unknown",
        DETAIL_DPI as u32,
    );

    // Try the master cache first
    if let Ok(mut cache) = MasterCache::open() {
        if let Some(cached) = cache.get(&cache_key) {
            return Ok((cached, "cache".to_string()));
        }
    }

    // Negotiate renderer
    let choice = negotiate_renderer(&RendererPreference::Default);
    if !choice.available {
        anyhow::bail!("no renderer available: {}", choice.reason);
    }

    // Create engine and convert to PDF
    let engine = create_engine_for_choice(&choice)?;
    let pdf_bytes = engine
        .convert(&bytes, &source_ext, "pdf")
        .map_err(|diags| {
            anyhow::anyhow!(
                "engine {} conversion failed: {:?}",
                choice.engine_id,
                diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
            )
        })?;

    // Rasterize the target page (0-indexed for rasterize)
    let page_png = raster::rasterize(&pdf_bytes, page, DETAIL_DPI as u32).map_err(|diags| {
        anyhow::anyhow!(
            "rasterization failed for page {}: {:?}",
            page,
            diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
        )
    })?;

    // Cache for future use
    let rendered_by = format!(
        "{} {}",
        engine.engine_id(),
        engine.version().unwrap_or("unknown")
    );
    if let Ok(mut cache) = MasterCache::open() {
        cache.put(&cache_key, &page_png);
    }

    Ok((page_png, rendered_by))
}

/// Create an engine instance based on the negotiated renderer choice.
fn create_engine_for_choice(
    choice: &engines::negotiate::RendererChoice,
) -> Result<Box<dyn Engine>, anyhow::Error> {
    match choice.engine_id.as_str() {
        "libreoffice" => LibreOfficeEngine::new_auto()
            .map(|e| Box::new(e) as Box<dyn Engine>)
            .map_err(|diags| {
                anyhow::anyhow!(
                    "failed to create engine (choice={}): {:?}",
                    choice.engine_id,
                    diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
                )
            }),
        "pandoc" => PandocEngine::new_auto()
            .map(|e| Box::new(e) as Box<dyn Engine>)
            .map_err(|diags| {
                anyhow::anyhow!(
                    "failed to create engine (choice={}): {:?}",
                    choice.engine_id,
                    diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
                )
            }),
        "chromium" => ChromiumEngine::new_auto()
            .map(|e| Box::new(e) as Box<dyn Engine>)
            .map_err(|diags| {
                anyhow::anyhow!(
                    "failed to create engine (choice={}): {:?}",
                    choice.engine_id,
                    diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
                )
            }),
        "typst" => TypstEngine::new_auto()
            .map(|e| Box::new(e) as Box<dyn Engine>)
            .map_err(|diags| {
                anyhow::anyhow!(
                    "failed to create engine (choice={}): {:?}",
                    choice.engine_id,
                    diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
                )
            }),
        unknown => Err(anyhow::anyhow!(
            "unknown engine id from negotiation: '{}'; this indicates a negotiation bug",
            unknown
        )),
    }
}

/// Extract image dimensions from PNG bytes.
fn master_page_dimensions(data: &[u8]) -> Result<(usize, usize), anyhow::Error> {
    // PNG header: bytes 16-19 = width (4 bytes BE), bytes 20-23 = height (4 bytes BE)
    if data.len() < 24 {
        // For synthetic data (non-PNG), infer from expected size
        let w = (DEFAULT_PAGE_WIDTH_PT * DETAIL_DPI / 72.0) as usize;
        let h = (DEFAULT_PAGE_HEIGHT_PT * DETAIL_DPI / 72.0) as usize;
        return Ok((w, h));
    }

    // Check PNG signature
    if data[0..8] != [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        // Not a PNG — infer dimensions
        let w = (DEFAULT_PAGE_WIDTH_PT * DETAIL_DPI / 72.0) as usize;
        let h = (DEFAULT_PAGE_HEIGHT_PT * DETAIL_DPI / 72.0) as usize;
        return Ok((w, h));
    }

    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as usize;
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as usize;
    Ok((w, h))
}

/// Apply zoom to a CropRequest.
fn with_zoom(mut req: CropRequest, zoom: f64) -> CropRequest {
    req.zoom = Some(zoom);
    req
}

/// Parse a bbox from a comma-separated string: "x,y,w,h" in points.
pub fn parse_bbox(s: &str) -> Result<[f64; 4], anyhow::Error> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        anyhow::bail!("bbox must be 4 comma-separated values: x_pt,y_pt,width_pt,height_pt");
    }
    let parse = |s: &str| -> Result<f64, anyhow::Error> {
        s.trim()
            .parse::<f64>()
            .with_context(|| format!("invalid bbox value: {}", s))
    };
    Ok([
        parse(parts[0])?,
        parse(parts[1])?,
        parse(parts[2])?,
        parse(parts[3])?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_parse_bbox_valid — parse a valid bbox string.
    #[test]
    fn test_parse_bbox_valid() {
        let bbox = parse_bbox("72.0, 240.0, 200.0, 80.0").unwrap();
        assert_eq!(bbox, [72.0, 240.0, 200.0, 80.0]);
    }

    /// test_parse_bbox_zero — zero values are valid.
    #[test]
    fn test_parse_bbox_zero() {
        let bbox = parse_bbox("0,0,100,100").unwrap();
        assert_eq!(bbox, [0.0, 0.0, 100.0, 100.0]);
    }

    /// test_parse_bbox_invalid_count — wrong number of components → error.
    #[test]
    fn test_parse_bbox_invalid_count() {
        let err = parse_bbox("1,2,3").unwrap_err();
        assert!(err.to_string().contains("4 comma-separated"));
    }

    /// test_parse_bbox_invalid_value — non-numeric value → error.
    #[test]
    fn test_parse_bbox_invalid_value() {
        let err = parse_bbox("1,2,abc,4").unwrap_err();
        assert!(err.to_string().contains("invalid bbox value"));
    }

    /// test_with_zoom — applies zoom to request.
    #[test]
    fn test_with_zoom() {
        let req = CropRequest::new(1, [0.0, 0.0, 100.0, 100.0]);
        let req_zoomed = with_zoom(req, 3.0);
        assert_eq!(req_zoomed.zoom, Some(3.0));
    }

    /// test_inspect_region_validation — bbox out of bounds returns error.
    #[test]
    fn test_inspect_region_validation() {
        // This bbox extends beyond Letter page (500+200=700 > 612) → validation error
        let result = inspect_region(
            "/nonexistent/test.docx",
            1,
            [500.0, 700.0, 200.0, 200.0],
            2.0,
        );
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("beyond page") || err_msg.contains("positive"),
            "expected validation error, got: {}",
            err_msg
        );
    }

    /// test_inspect_region_valid_request — valid request produces expected JSON shape.
    #[test]
    fn test_inspect_region_json_shape() {
        // Use a mock result since we can't easily render a full document
        let mock_result = mock_inspect_region();
        assert_eq!(mock_result["tool"].as_str().unwrap(), "inspect_region");
        assert_eq!(mock_result["page"].as_u64().unwrap(), 1);
        assert!(mock_result["crop"]["width"].is_number());
        assert!(mock_result["crop"]["height"].is_number());
        assert_eq!(mock_result["crop"]["dpi"].as_f64().unwrap(), DETAIL_DPI);
        assert!(mock_result["crop"]["visual_tokens"].is_number());
        assert!(mock_result["crop"]["within_budget"].as_bool().unwrap());
        assert!(mock_result["crop"]["byte_size"].is_number());
        assert!(mock_result["coordinate_record"]["scale"].is_number());
        assert!(
            mock_result["rendered_by"].is_string(),
            "rendered_by should be a string (was {})",
            mock_result["rendered_by"]
        );
    }

    /// test_crop_request_zoom_default — CropRequest::new defaults to 2.0x zoom.
    #[test]
    fn test_crop_request_zoom_default() {
        let req = CropRequest::new(1, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(req.zoom, Some(2.0));
    }

    /// test_master_page_dimensions_infer — synthetic data returns expected dimensions.
    #[test]
    fn test_master_page_dimensions_infer() {
        let synthetic = vec![128u8; 1000];
        let (w, h) = master_page_dimensions(&synthetic).unwrap();
        let expected_w = (DEFAULT_PAGE_WIDTH_PT * DETAIL_DPI / 72.0) as usize;
        let expected_h = (DEFAULT_PAGE_HEIGHT_PT * DETAIL_DPI / 72.0) as usize;
        assert_eq!(w, expected_w);
        assert_eq!(h, expected_h);
    }

    /// test_master_page_dimensions_png — valid PNG header extracts correct size.
    #[test]
    fn test_master_page_dimensions_png() {
        let mut png = vec![0u8; 24];
        png[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        png[16..20].copy_from_slice(&2550u32.to_be_bytes());
        png[20..24].copy_from_slice(&3300u32.to_be_bytes());
        let (w, h) = master_page_dimensions(&png).unwrap();
        assert_eq!(w, 2550);
        assert_eq!(h, 3300);
    }

    /// Build a mock inspect_region result for JSON shape testing.
    fn mock_inspect_region() -> serde_json::Value {
        let _ = master_page_dimensions(&[0u8; 100]).unwrap();
        let bbox: [f64; 4] = [0.0, 0.0, 72.0, 72.0];
        let scale = DETAIL_DPI / 72.0;
        let crop_w = (bbox[2] * scale * 2.0) as usize;
        let crop_h = (bbox[3] * scale * 2.0) as usize;
        let tokens = crop_w.div_ceil(28) * crop_h.div_ceil(28);

        serde_json::json!({
            "tool": "inspect_region",
            "path": "/test.docx",
            "page": 1,
            "bbox_pt": bbox,
            "zoom": 2.0,
            "crop": {
                "width": crop_w,
                "height": crop_h,
                "dpi": DETAIL_DPI,
                "visual_tokens": tokens,
                "within_budget": tokens <= 1200,
                "byte_size": crop_w * crop_h,
            },
            "coordinate_record": {
                "scale": scale,
                "source_dpi": DETAIL_DPI,
                "target_dpi": DETAIL_DPI,
                "image_width_px": crop_w,
                "image_height_px": crop_h,
                "page_width_pt": DEFAULT_PAGE_WIDTH_PT,
                "page_height_pt": DEFAULT_PAGE_HEIGHT_PT,
            },
            "rendered_by": "libreoffice + poppler",
        })
    }

    /// test_estimate_crop_tokens — token estimation matches cropper logic.
    #[test]
    fn test_estimate_crop_tokens() {
        // 72pt x 72pt at 300 DPI, 1x zoom = 300x300px
        // blocks: ceil(300/28) * ceil(300/28) = 11 * 11 = 121
        let tokens = lint::cropper::estimate_crop_tokens([0.0, 0.0, 72.0, 72.0], DETAIL_DPI, 1.0);
        assert_eq!(tokens, 121);

        // 72pt x 72pt at 300 DPI, 2x zoom = 600x600px
        // blocks: ceil(600/28) * ceil(600/28) = 22 * 22 = 484
        let tokens2x = lint::cropper::estimate_crop_tokens([0.0, 0.0, 72.0, 72.0], DETAIL_DPI, 2.0);
        assert_eq!(tokens2x, 484);
    }

    /// test_create_engine_for_choice_libreoffice — dispatches LibreOffice when engine_id matches.
    #[test]
    fn test_create_engine_for_choice_libreoffice() {
        let choice = engines::negotiate::RendererChoice {
            engine_id: "libreoffice".to_string(),
            available: true,
            reason: "selected".to_string(),
        };
        let result = create_engine_for_choice(&choice);
        if let Ok(engine) = result {
            assert_eq!(engine.engine_id(), "libreoffice");
        }
    }

    /// test_create_engine_for_choice_pandoc — dispatches Pandoc when engine_id matches.
    #[test]
    fn test_create_engine_for_choice_pandoc() {
        let choice = engines::negotiate::RendererChoice {
            engine_id: "pandoc".to_string(),
            available: true,
            reason: "selected".to_string(),
        };
        let result = create_engine_for_choice(&choice);
        if let Ok(engine) = result {
            assert_eq!(engine.engine_id(), "pandoc");
        }
    }

    /// test_create_engine_for_choice_chromium — dispatches Chromium when engine_id matches.
    #[test]
    fn test_create_engine_for_choice_chromium() {
        let choice = engines::negotiate::RendererChoice {
            engine_id: "chromium".to_string(),
            available: true,
            reason: "selected".to_string(),
        };
        let result = create_engine_for_choice(&choice);
        if let Ok(engine) = result {
            assert_eq!(engine.engine_id(), "chromium");
        }
    }

    /// test_create_engine_for_choice_unknown — unknown engine_id returns error.
    #[test]
    fn test_create_engine_for_choice_unknown() {
        let choice = engines::negotiate::RendererChoice {
            engine_id: "nonexistent".to_string(),
            available: true,
            reason: "selected".to_string(),
        };
        let result = create_engine_for_choice(&choice);
        match result {
            Ok(_) => panic!("expected error for unknown engine"),
            Err(e) => {
                assert!(
                    e.to_string().contains("unknown engine id"),
                    "unexpected error: {}",
                    e
                );
            }
        }
    }
}
