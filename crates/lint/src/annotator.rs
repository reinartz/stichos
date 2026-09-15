//! Annotator: draws numbered lint boxes on rendered pages.
//!
//! Per REN-06: every lint diagnostic on a changed page gets a numbered box.
//! Produces marked + clean image pairs in the payload.

use crate::budget::compute_visual_tokens;
use anyhow::{Context, Result};
use gp_core::diagnostics::Diagnostic;
use gp_core::executor::RenderPage;
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

/// A lint annotation: a numbered box on a page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LintAnnotation {
    /// Sequential number (1, 2, 3...) for this annotation on the page.
    pub number: usize,
    /// Bounding box in points: [x_pt, y_pt, width_pt, height_pt].
    pub bbox_pt: [f64; 4],
    /// The diagnostic code for this annotation.
    pub code: String,
    /// The diagnostic message.
    pub message: String,
    /// Severity level.
    pub severity: String,
}

/// A marked image pair: clean original + annotated version.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarkedImagePair {
    /// The clean (unannotated) page image as base64.
    pub clean: ImageData,
    /// The marked (annotated) page image as base64.
    pub marked: ImageData,
    /// List of annotations drawn on this page.
    pub annotations: Vec<LintAnnotation>,
}

/// Image data as base64-encoded PNG.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageData {
    /// Base64-encoded PNG bytes.
    pub data: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Visual tokens consumed.
    pub visual_tokens: usize,
}

/// Convert page bbox (points) to pixel coordinates at the given DPI.
pub fn bbox_to_pixels(bbox_pt: [f64; 4], dpi: f64) -> [f64; 4] {
    let pt_to_px = dpi / 72.0; // 72 points per inch
    let [x, y, w, h] = bbox_pt;
    [x * pt_to_px, y * pt_to_px, w * pt_to_px, h * pt_to_px]
}

/// Draw a numbered box on an image at the given pixel coordinates.
fn draw_numbered_box(
    img: &mut RgbaImage,
    bbox_px: [f64; 4],
    number: usize,
    severity_color: [u8; 3],
) {
    let [x, y, w, h] = bbox_px;
    let img_w = img.width() as f64;
    let img_h = img.height() as f64;

    // Clip to image bounds
    let x = x.max(0.0).min(img_w) as u32;
    let y = y.max(0.0).min(img_h) as u32;
    let w = w.max(0.0).min((img_w - x as f64).max(0.0)) as u32;
    let h = h.max(0.0).min((img_h - y as f64).max(0.0)) as u32;

    if w == 0 || h == 0 {
        return;
    }

    // Draw box outline as Rgba pixel
    let color = Rgba([severity_color[0], severity_color[1], severity_color[2], 255]);
    for px in 0..w {
        // Top edge
        if y < img.height() {
            img.put_pixel(x + px, y, color);
        }
        // Bottom edge
        if y + h > 0 && y + h - 1 < img.height() {
            img.put_pixel(x + px, y + h - 1, color);
        }
    }
    for py in 0..h {
        // Left edge
        if x < img.width() {
            img.put_pixel(x, y + py, color);
        }
        // Right edge
        if x + w > 0 && x + w - 1 < img.width() {
            img.put_pixel(x + w - 1, y + py, color);
        }
    }

    // Draw number label in top-left corner of the box
    let label = format!("{}", number);
    let label_w = label.len() as u32 * 7;
    let label_y = y.min(img.height().saturating_sub(18));
    let label_x = x.min(img.width().saturating_sub(label_w.max(15)));

    // Draw background rect for label
    let white = Rgba([255, 255, 255, 255]);
    for px in 0..label_w.max(15) {
        for py in 0..16 {
            if label_x + px < img.width() && label_y + py < img.height() {
                img.put_pixel(label_x + px, label_y + py, white);
            }
        }
    }
    // Draw label (simple pixel rendering for single chars)
    for (i, ch) in label.chars().enumerate() {
        let cx = label_x + (i as u32).saturating_mul(8).min(label.len() as u32 * 7 - 7);
        draw_char_simple(img, cx, label_y, ch, severity_color);
    }
}

/// Draw a simple character using a minimal font grid.
fn draw_char_simple(img: &mut RgbaImage, x: u32, y: u32, ch: char, color: [u8; 3]) {
    let c: &[u8; 8] = match ch {
        '0' => &[
            0b011110, 0b010001, 0b010001, 0b010001, 0b010001, 0b010001, 0b011110, 0b000000,
        ],
        '1' => &[
            0b001000, 0b011000, 0b001000, 0b001000, 0b001000, 0b001000, 0b011100, 0b000000,
        ],
        '2' => &[
            0b011100, 0b010001, 0b000010, 0b000100, 0b001000, 0b010000, 0b011110, 0b000000,
        ],
        '3' => &[
            0b011100, 0b000010, 0b000100, 0b001100, 0b000010, 0b010000, 0b011100, 0b000000,
        ],
        '4' => &[
            0b000100, 0b001100, 0b001100, 0b011100, 0b010100, 0b010100, 0b011100, 0b000000,
        ],
        '5' => &[
            0b011110, 0b010000, 0b011100, 0b000010, 0b000001, 0b010000, 0b011100, 0b000000,
        ],
        '6' => &[
            0b001110, 0b010000, 0b010000, 0b011100, 0b010001, 0b010001, 0b001110, 0b000000,
        ],
        '7' => &[
            0b011110, 0b000010, 0b000100, 0b001000, 0b010000, 0b010000, 0b010000, 0b000000,
        ],
        '8' => &[
            0b001110, 0b010001, 0b010001, 0b001110, 0b010001, 0b010001, 0b001110, 0b000000,
        ],
        '9' => &[
            0b001110, 0b010001, 0b010001, 0b001111, 0b000010, 0b000100, 0b011000, 0b000000,
        ],
        _ => &[
            0b000000, 0b000000, 0b000000, 0b001000, 0b001000, 0b001000, 0b000000, 0b000000,
        ],
    };

    let color = Rgba([color[0], color[1], color[2], 255]);
    for (row, pattern) in c.iter().enumerate() {
        for col in 0..7 {
            if *pattern & (1 << (6 - col)) != 0 {
                let px = x + col;
                let py = y + row as u32;
                if px < img.width() && py < img.height() {
                    img.put_pixel(px, py, color);
                }
            }
        }
    }
}

/// Severity color mapping.
fn severity_color(severity: &str) -> [u8; 3] {
    match severity {
        "error" => [226, 72, 58], // vermilion (brand color)
        "warning" => [244, 190, 50],
        "info" => [147, 160, 184], // slate
        _ => [147, 160, 184],
    }
}

/// Annotate a rendered page with lint diagnostics, producing a marked + clean pair.
///
/// The `dpi` parameter determines the pixel resolution for drawing boxes.
pub fn annotate_page(
    page: &RenderPage,
    diagnostics: &[Diagnostic],
    dpi: f64,
) -> Result<MarkedImagePair> {
    // Decode the source image
    let source_bytes = page
        .image_bytes
        .clone()
        .ok_or_else(|| anyhow::anyhow!("page {} has no image", page.page_number))?;
    let source = image::load_from_memory(&source_bytes)?;

    let mut marked = source.to_rgba8();
    let (width, height) = marked.dimensions();

    // Collect and number diagnostics that have bbox info
    let mut annotations: Vec<LintAnnotation> = Vec::new();

    for (number, diag) in diagnostics.iter().enumerate() {
        let sev_str = format!("{}", diag.severity).to_lowercase();
        let color = severity_color(&sev_str);

        if let Some(bbox_pt) = extract_diag_bbox(diag) {
            let bbox_px = bbox_to_pixels(bbox_pt, dpi);
            draw_numbered_box(&mut marked, bbox_px, number + 1, color);

            annotations.push(LintAnnotation {
                number: number + 1,
                bbox_pt,
                code: diag.code.clone(),
                message: diag.message.clone(),
                severity: format!("{}", diag.severity),
            });
        }
    }

    // If no bbox annotations were drawn, still create annotations with page-level boxes
    if annotations.is_empty() {
        let [w, h] = [page.width_pt, page.height_pt];
        annotations.push(LintAnnotation {
            number: 1,
            bbox_pt: [0.0, 0.0, w, h],
            code: "REN-I-001".to_string(),
            message: format!("Page has {} diagnostic(s)", diagnostics.len()),
            severity: if diagnostics.is_empty() {
                "info".to_string()
            } else {
                "warning".to_string()
            },
        });
        let mut buf = Vec::new();
        marked
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .ok();

        let visual_tokens = compute_visual_tokens(width as usize, height as usize);
        let data = base64_encode(&buf);

        return Ok(MarkedImagePair {
            clean: ImageData {
                data: data.clone(),
                width,
                height,
                visual_tokens,
            },
            marked: ImageData {
                data,
                width,
                height,
                visual_tokens,
            },
            annotations,
        });
    }

    // Encode marked image
    let mut buf = Vec::new();
    marked
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .with_context(|| "failed to encode annotated PNG")?;

    // Encode clean image
    let mut clean_buf = Vec::new();
    source
        .to_rgba8()
        .write_to(&mut Cursor::new(&mut clean_buf), ImageFormat::Png)
        .with_context(|| "failed to encode clean PNG")?;

    let visual_tokens = compute_visual_tokens(width as usize, height as usize);
    let marked_data = base64_encode(&buf);
    let clean_data = base64_encode(&clean_buf);

    Ok(MarkedImagePair {
        clean: ImageData {
            data: clean_data,
            width,
            height,
            visual_tokens,
        },
        marked: ImageData {
            data: marked_data,
            width,
            height,
            visual_tokens,
        },
        annotations,
    })
}

/// Extract bbox from a diagnostic's message or metadata.
fn extract_diag_bbox(diag: &Diagnostic) -> Option<[f64; 4]> {
    // Try to parse bbox from diagnostic message
    // Format: "overflow at [x, y, w, h]" or similar
    let msg = &diag.message;

    // Look for bbox pattern in message
    if let Some(start) = msg.find('[') {
        if let Some(end) = msg.find(']') {
            let coords_str = &msg[start + 1..end];
            let coords: Vec<f64> = coords_str
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            if coords.len() >= 4 {
                return Some([coords[0], coords[1], coords[2], coords[3]]);
            }
        }
    }
    None
}

/// Base64 encode bytes.
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Annotate all changed pages from a QA result, producing marked + clean pairs.
pub fn annotate_pages(
    pages: &[RenderPage],
    diagnostics: &[Diagnostic],
    dpi: f64,
) -> Result<Vec<MarkedImagePair>> {
    let mut pairs = Vec::new();
    for page in pages {
        if page.image_bytes.is_none() {
            continue;
        }
        let pair = annotate_page(page, diagnostics, dpi)?;
        pairs.push(pair);
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::diagnostics::Severity;

    fn make_test_image(width: u32, height: u32) -> Vec<u8> {
        let img: image::RgbaImage = image::ImageBuffer::new(width, height);
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }

    /// test_bbox_to_pixels — converts points to pixels at given DPI.
    #[test]
    fn test_bbox_to_pixels() {
        let [x, y, w, h] = bbox_to_pixels([72.0, 72.0, 144.0, 144.0], 72.0);
        assert_eq!(x, 72.0);
        assert_eq!(y, 72.0);
        assert_eq!(w, 144.0);
        assert_eq!(h, 144.0);
    }

    /// test_bbox_to_pixels_300_dpi — higher DPI scales correctly.
    #[test]
    fn test_bbox_to_pixels_300_dpi() {
        let [x, _y, _w, _h] = bbox_to_pixels([72.0, 72.0, 72.0, 72.0], 300.0);
        // 72pt = 4.17" at 72 DPI -> 300 DPI -> 1250px
        let expected_px = 72.0 * (300.0 / 72.0);
        assert!((x - expected_px).abs() < 0.01);
    }

    /// test_severity_color_error — error diagnostics get vermilion.
    #[test]
    fn test_severity_color_error() {
        let color = severity_color("error");
        assert_eq!(color, [226, 72, 58]);
    }

    /// test_severity_color_warning — warnings get yellow.
    #[test]
    fn test_severity_color_warning() {
        let color = severity_color("warning");
        assert_eq!(color, [244, 190, 50]);
    }

    /// test_severity_color_info — info diagnostics get slate.
    #[test]
    fn test_severity_color_info() {
        let color = severity_color("info");
        assert_eq!(color, [147, 160, 184]);
    }

    /// test_annotate_page_minimal — annotates a minimal page.
    #[test]
    fn test_annotate_page_minimal() {
        let img_bytes = make_test_image(100, 100);
        let page = RenderPage {
            page_number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            image_bytes: Some(img_bytes),
            text: "test page".to_string(),
        };
        let diags = vec![Diagnostic::new(
            "LIN-E-001",
            Severity::Error,
            "overflow at [100.0, 200.0, 50.0, 30.0]",
        )];
        let pair = annotate_page(&page, &diags, 150.0).unwrap();
        assert_eq!(pair.annotations.len(), 1);
        assert_eq!(pair.annotations[0].number, 1);
        assert_eq!(pair.annotations[0].code, "LIN-E-001");
        assert!(!pair.clean.data.is_empty());
        assert!(!pair.marked.data.is_empty());
    }

    /// test_annotate_empty_diagnostics — produces pair even with no diagnostics.
    #[test]
    fn test_annotate_empty_diagnostics() {
        let img_bytes = make_test_image(100, 100);
        let page = RenderPage {
            page_number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            image_bytes: Some(img_bytes),
            text: "test page".to_string(),
        };
        let pair = annotate_page(&page, &[], 150.0).unwrap();
        assert!(!pair.clean.data.is_empty());
        assert!(!pair.marked.data.is_empty());
    }

    /// test_annotate_multiple_diagnostics — multiple diagnostics produce multiple annotations.
    #[test]
    fn test_annotate_multiple_diagnostics() {
        let img_bytes = make_test_image(100, 100);
        let page = RenderPage {
            page_number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            image_bytes: Some(img_bytes),
            text: "test page".to_string(),
        };
        let diags = vec![
            Diagnostic::new(
                "LIN-E-001",
                Severity::Error,
                "overflow at [10.0, 20.0, 5.0, 5.0]",
            ),
            Diagnostic::new(
                "LIN-W-001",
                Severity::Warning,
                "text overflow at [30.0, 40.0, 10.0, 10.0]",
            ),
        ];
        let pair = annotate_page(&page, &diags, 150.0).unwrap();
        assert_eq!(pair.annotations.len(), 2);
        assert_eq!(pair.annotations[0].number, 1);
        assert_eq!(pair.annotations[1].number, 2);
        assert_eq!(pair.annotations[0].code, "LIN-E-001");
        assert_eq!(pair.annotations[1].code, "LIN-W-001");
    }

    /// test_annotate_no_bbox — falls back to page-level annotation.
    #[test]
    fn test_annotate_no_bbox() {
        let img_bytes = make_test_image(100, 100);
        let page = RenderPage {
            page_number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            image_bytes: Some(img_bytes),
            text: "test page".to_string(),
        };
        // Diagnostic without bbox in message
        let diags = vec![Diagnostic::new(
            "LIN-E-001",
            Severity::Warning,
            "some warning without bbox",
        )];
        let pair = annotate_page(&page, &diags, 150.0).unwrap();
        assert_eq!(pair.annotations.len(), 1);
        assert_eq!(pair.annotations[0].code, "REN-I-001");
        assert!(!pair.clean.data.is_empty());
        assert!(!pair.marked.data.is_empty());
    }

    /// test_annotate_no_image — returns error when page has no image.
    #[test]
    fn test_annotate_no_image() {
        let page = RenderPage {
            page_number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            image_bytes: None,
            text: "test page".to_string(),
        };
        let diags = vec![Diagnostic::new(
            "LIN-E-001",
            Severity::Error,
            "overflow at [10.0, 20.0, 5.0, 5.0]",
        )];
        let result = annotate_page(&page, &diags, 150.0);
        assert!(result.is_err());
    }

    /// test_annotate_pages_multiple — annotates multiple pages.
    #[test]
    fn test_annotate_pages_multiple() {
        let img_bytes = make_test_image(100, 100);
        let pages = vec![
            RenderPage {
                page_number: 1,
                width_pt: 612.0,
                height_pt: 792.0,
                image_bytes: Some(img_bytes.clone()),
                text: "page 1".to_string(),
            },
            RenderPage {
                page_number: 2,
                width_pt: 612.0,
                height_pt: 792.0,
                image_bytes: Some(img_bytes),
                text: "page 2".to_string(),
            },
        ];
        let diags = vec![Diagnostic::new(
            "LIN-E-001",
            Severity::Error,
            "overflow at [10.0, 20.0, 5.0, 5.0]",
        )];
        let pairs = annotate_pages(&pages, &diags, 150.0).unwrap();
        assert_eq!(pairs.len(), 2);
        for pair in &pairs {
            assert!(!pair.clean.data.is_empty());
            assert!(!pair.marked.data.is_empty());
        }
    }

    /// test_annotate_pages_skip_none_image — skips pages without images.
    #[test]
    fn test_annotate_pages_skip_none_image() {
        let img_bytes = make_test_image(100, 100);
        let pages = vec![
            RenderPage {
                page_number: 1,
                width_pt: 612.0,
                height_pt: 792.0,
                image_bytes: Some(img_bytes.clone()),
                text: "page 1".to_string(),
            },
            RenderPage {
                page_number: 2,
                width_pt: 612.0,
                height_pt: 792.0,
                image_bytes: None,
                text: "page 2".to_string(),
            },
        ];
        let diags = vec![Diagnostic::new(
            "LIN-E-001",
            Severity::Error,
            "overflow at [10.0, 20.0, 5.0, 5.0]",
        )];
        let pairs = annotate_pages(&pages, &diags, 150.0).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].clean.width, 100);
    }

    /// test_extract_diag_bbox — parses bbox from diagnostic message.
    #[test]
    fn test_extract_diag_bbox() {
        let diag = Diagnostic::new(
            "LIN-E-001",
            Severity::Error,
            "overflow at [100.5, 200.3, 50.0, 30.0]",
        );
        let bbox = extract_diag_bbox(&diag);
        assert_eq!(bbox, Some([100.5, 200.3, 50.0, 30.0]));
    }

    /// test_extract_diag_bbox_no_coords — returns None when no bbox found.
    #[test]
    fn test_extract_diag_bbox_no_coords() {
        let diag = Diagnostic::new("LIN-E-001", Severity::Error, "no coordinates here");
        let bbox = extract_diag_bbox(&diag);
        assert_eq!(bbox, None);
    }

    /// test_marked_image_pair_serde — roundtrip serialization.
    #[test]
    fn test_marked_image_pair_serde() {
        let pair = MarkedImagePair {
            clean: ImageData {
                data: "test_data".to_string(),
                width: 100,
                height: 100,
                visual_tokens: 4,
            },
            marked: ImageData {
                data: "marked_data".to_string(),
                width: 100,
                height: 100,
                visual_tokens: 4,
            },
            annotations: vec![LintAnnotation {
                number: 1,
                bbox_pt: [0.0, 0.0, 100.0, 100.0],
                code: "LIN-E-001".to_string(),
                message: "test".to_string(),
                severity: "error".to_string(),
            }],
        };
        let json = serde_json::to_string(&pair).expect("serialize");
        let back: MarkedImagePair = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.clean.width, 100);
        assert_eq!(back.marked.height, 100);
        assert_eq!(back.annotations.len(), 1);
        assert_eq!(back.annotations[0].code, "LIN-E-001");
    }
}
