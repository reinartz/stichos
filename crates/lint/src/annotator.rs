//! Annotator: draws numbered lint boxes on rendered pages.
//!
//! Per REN-06: every lint diagnostic on a changed page gets a numbered box.
//! Produces marked + clean image pairs and a JSON legend manifest in the payload.

use crate::budget::compute_visual_tokens;
use anyhow::{Context, Result};
use gp_core::diagnostics::Diagnostic;
use gp_core::executor::RenderPage;
use image::{ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A lint annotation: a numbered box on a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Visual offset applied to the label position for collision avoidance (dx, dy) in pixels.
    pub label_offset: [f64; 2],
}

/// A marked image pair: clean original + annotated version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkedImagePair {
    /// The clean (unannotated) page image as base64.
    pub clean: ImageData,
    /// The marked (annotated) page image as base64.
    pub marked: ImageData,
    /// List of annotations drawn on this page.
    pub annotations: Vec<LintAnnotation>,
}

/// Image data as base64-encoded PNG.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Result of annotating a set of pages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatorResult {
    /// Marked + clean image pairs per page.
    pub pairs: Vec<MarkedImagePair>,
    /// JSON-serializable legend summarizing all annotations.
    pub legend: AnnotatorLegend,
}

/// A JSON-serializable legend alongside marked+clean pairs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatorLegend {
    /// Version of the legend format.
    pub version: String,
    /// List of unique codes seen in the annotations.
    pub codes: Vec<LegendCode>,
    /// Total annotation count.
    pub total_annotations: usize,
    /// Count by severity.
    pub severity_counts: HashMap<String, usize>,
}

/// Summary of a single diagnostic code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegendCode {
    /// The diagnostic code (e.g. "LIN-E-001").
    pub code: String,
    /// Severity string (e.g. "error", "warning", "info").
    pub severity: String,
    /// How many times this code appears.
    pub count: usize,
    /// Hex color used for this severity (e.g. "#E2483A").
    pub color: String,
    /// One sample message from annotations with this code.
    pub sample_message: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const LABEL_OFFSET_STEP: f64 = 15.0;
const LEGEND_VERSION: &str = "1.0.0";

// Severity colours (R, G, B)
const COLOR_ERROR: [u8; 3] = [226, 72, 58]; // vermilion #E2483A
const COLOR_WARNING: [u8; 3] = [244, 190, 50]; // amber #F4BE32
const COLOR_INFO: [u8; 3] = [147, 160, 184]; // slate #93A0B8

/// Line thickness by severity (in pixels).
fn severity_line_width(severity: &str) -> u32 {
    match severity {
        "error" => 2,
        "warning" => 1,
        _ => 1,
    }
}

/// Whether the box should use a dashed outline.
fn severity_dashed(severity: &str) -> bool {
    matches!(severity, "warning")
}

/// Severity color mapping — returns [R, G, B].
fn severity_color(severity: &str) -> [u8; 3] {
    match severity {
        "error" => COLOR_ERROR,
        "warning" => COLOR_WARNING,
        "info" | "hint" => COLOR_INFO,
        _ => COLOR_INFO,
    }
}

/// Convert severity to hex colour string.
pub fn severity_color_hex(severity: &str) -> String {
    let [r, g, b] = severity_color(severity);
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

/// Convert page bbox (points) to pixel coordinates at the given DPI.
pub fn bbox_to_pixels(bbox_pt: [f64; 4], dpi: f64) -> [f64; 4] {
    let pt_to_px = dpi / 72.0; // 72 points per inch
    let [x, y, w, h] = bbox_pt;
    [x * pt_to_px, y * pt_to_px, w * pt_to_px, h * pt_to_px]
}

/// Check whether two pixel-coordinate bounding boxes overlap.
fn boxes_overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    let [ax, ay, aw, ah] = a;
    let [bx, by, _bw, _bh] = b;
    ax < bx + _bw && ax + aw > bx && ay < by + ah && ay + ah > by
}

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

/// Draw a numbered box on an image at the given pixel coordinates.
fn draw_numbered_box(img: &mut RgbaImage, bbox_px: [f64; 4], number: usize, severity: &str) {
    let [x, y, w, h] = bbox_px;
    let img_w = img.width() as f64;
    let img_h = img.height() as f64;
    let [r, g, b] = severity_color(severity);
    let color = Rgba([r, g, b, 255]);
    let _line = severity_line_width(severity);
    let dashed = severity_dashed(severity);

    // Clip to image bounds
    let x = x.max(0.0).min(img_w) as u32;
    let y = y.max(0.0).min(img_h) as u32;
    let w = w.max(0.0).min((img_w - x as f64).max(0.0)) as u32;
    let h = h.max(0.0).min((img_h - y as f64).max(0.0)) as u32;

    if w == 0 || h == 0 {
        return;
    }

    // Helper to draw a single horizontal line segment with optional dashes
    let draw_h_line = |img: &mut RgbaImage, start_x: u32, end_x: u32, row: u32| {
        if row >= img.height() {
            return;
        }
        if dashed {
            let dash_len = 4u32;
            let gap = 3u32;
            let mut px = start_x;
            while px < end_x {
                let seg_end = (px + dash_len).min(end_x);
                for col in px..seg_end {
                    img.put_pixel(col, row, color);
                }
                px = seg_end + gap;
            }
        } else {
            for col in start_x..end_x {
                img.put_pixel(col, row, color);
            }
        }
    };

    // Top edge
    draw_h_line(img, x, x + w, y);
    // Bottom edge
    draw_h_line(img, x, x + w, y + h - 1);
    // Left edge
    for row in y..(y + h).min(img.height()) {
        img.put_pixel(x, row, color);
    }
    // Right edge
    for row in y..(y + h).min(img.height()) {
        img.put_pixel(x + w - 1, row, color);
    }

    // Draw number label in top-left corner of the box
    draw_label(img, x, y, number, severity);
}

/// Draw a label (number) with a white background rect.
fn draw_label(img: &mut RgbaImage, x: u32, y: u32, number: usize, severity: &str) {
    let label = format!("{}", number);
    let label_w = (label.len() as u32).saturating_mul(8).max(15);
    let label_h: u32 = 16;
    let label_x = x.min(img.width().saturating_sub(label_w));
    let label_y = y.min(img.height().saturating_sub(label_h));

    // White background rect
    let white = Rgba([255, 255, 255, 255]);
    for px in 0..label_w {
        for py in 0..label_h {
            if label_x + px < img.width() && label_y + py < img.height() {
                img.put_pixel(label_x + px, label_y + py, white);
            }
        }
    }

    // Draw each character
    for (i, ch) in label.chars().enumerate() {
        let cx = label_x + (i as u32).saturating_mul(8);
        draw_char_simple(img, cx, label_y, ch, severity_color(severity));
    }
}

/// Draw a simple character using a minimal 7x8 font grid.
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
            0b011110, 0b000010, 0b000100, 0b010000, 0b010000, 0b010000, 0b010000, 0b000000,
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

// ---------------------------------------------------------------------------
// Collision avoidance
// ---------------------------------------------------------------------------

/// Resolve label collisions among annotations on a single page.
///
/// If two annotation boxes overlap, offset the later label along the axis
/// with the most available space.
pub fn resolve_label_collisions(annotations: &mut [LintAnnotation], img_w: u32, img_h: u32) {
    let count = annotations.len();
    if count <= 1 {
        return;
    }

    let mut offset = vec![[0.0, 0.0]; count];

    for i in 0..count {
        let [ix, iy, iw, ih] = pixel_bbox(&annotations[i]);
        for j in 0..i {
            let [jx, jy, jw, jh] = pixel_bbox(&annotations[j]);

            if !boxes_overlap(
                [ix, iy, iw, ih],
                [jx + offset[j][0], jy + offset[j][1], jw, jh],
            ) {
                continue;
            }

            // Overlap detected — compute offset along the axis with more space
            let h_space = img_w as f64 - iw;
            let v_space = img_h as f64 - ih;
            if h_space > v_space {
                offset[i][1] += LABEL_OFFSET_STEP;
            } else {
                offset[i][0] += LABEL_OFFSET_STEP;
            }
        }
    }

    for (ann, off) in annotations.iter_mut().zip(offset.iter()) {
        ann.label_offset = *off;
    }
}

/// Convert an annotation's point bbox to a pixel bbox using standard DPI.
fn pixel_bbox(ann: &LintAnnotation) -> [f64; 4] {
    bbox_to_pixels(ann.bbox_pt, 150.0) // 150 DPI as a reasonable default for collision checking
}

// ---------------------------------------------------------------------------
// Legend
// ---------------------------------------------------------------------------

/// Build a legend from page annotations.
///
/// Collects unique codes, severity counts, and per-code statistics.
pub fn build_legend(annotations: &[Vec<LintAnnotation>]) -> AnnotatorLegend {
    let mut codes_map: HashMap<String, LegendCode> = HashMap::new();
    let mut severity_counts: HashMap<String, usize> = HashMap::new();
    let mut total: usize = 0;

    for page_anns in annotations {
        for ann in page_anns {
            total += 1;

            *severity_counts.entry(ann.severity.clone()).or_default() += 1;

            let entry = codes_map.entry(ann.code.clone()).or_insert(LegendCode {
                code: ann.code.clone(),
                severity: ann.severity.clone(),
                count: 0,
                color: severity_color_hex(&ann.severity),
                sample_message: ann.message.clone(),
            });
            entry.count += 1;
        }
    }

    let codes: Vec<LegendCode> = codes_map.into_values().collect();

    AnnotatorLegend {
        version: LEGEND_VERSION.to_string(),
        codes,
        total_annotations: total,
        severity_counts,
    }
}

/// Serialize a legend to a JSON string.
pub fn legend_to_json(legend: &AnnotatorLegend) -> Result<String> {
    serde_json::to_string_pretty(legend).with_context(|| "failed to serialize legend to JSON")
}

// ---------------------------------------------------------------------------
// Page annotation
// ---------------------------------------------------------------------------

/// Annotate a rendered page with lint diagnostics, producing a marked + clean pair.
///
/// The `dpi` parameter determines the pixel resolution for drawing boxes.
pub fn annotate_page(
    page: &RenderPage,
    diagnostics: &[Diagnostic],
    dpi: f64,
) -> Result<MarkedImagePair> {
    // Decode the source image
    let source = decode_image(page)?;
    let mut marked = source.clone();
    let (width, height) = marked.dimensions();

    // Collect and number diagnostics that have bbox info
    let mut annotations: Vec<LintAnnotation> = Vec::new();

    for (number, diag) in diagnostics.iter().enumerate() {
        let sev_str = format!("{}", diag.severity);
        // Normalise Display output: "ERROR" -> "error", "WARN" -> "warning"
        let sev_key = normalise_severity(&sev_str);

        if let Some(bbox_pt) = extract_diag_bbox(diag) {
            let bbox_px = bbox_to_pixels(bbox_pt, dpi);
            draw_numbered_box(&mut marked, bbox_px, number + 1, sev_key);

            annotations.push(LintAnnotation {
                number: number + 1,
                bbox_pt,
                code: diag.code.clone(),
                message: diag.message.clone(),
                severity: sev_key.to_string(),
                label_offset: [0.0, 0.0],
            });
        }
    }

    // Resolve label collisions among annotations on this page
    resolve_label_collisions(&mut annotations, width, height);

    // If no bbox annotations were drawn, fall back to page-level annotation
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
            label_offset: [0.0, 0.0],
        });
    }

    // Encode images
    let marked_bytes = encode_png(&marked);
    let clean_bytes = encode_png(&source);

    let visual_tokens = compute_visual_tokens(width as usize, height as usize);
    let marked_data = ImageData {
        data: base64_encode(&marked_bytes),
        width,
        height,
        visual_tokens,
    };
    let clean_data = ImageData {
        data: base64_encode(&clean_bytes),
        width,
        height,
        visual_tokens,
    };

    Ok(MarkedImagePair {
        clean: clean_data,
        marked: marked_data,
        annotations,
    })
}

/// Normalise severity Display strings to canonical lowercase keys.
fn normalise_severity(s: &str) -> &str {
    match s {
        "ERROR" => "error",
        "WARN" => "warning",
        "WARNING" => "warning",
        "INFO" => "info",
        "HINT" => "info",
        "FATAL" => "error",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Multi-page
// ---------------------------------------------------------------------------

/// Annotate all changed pages from a QA result, producing marked + clean pairs
/// and a JSON-serializable legend manifest.
pub fn annotate_pages(
    pages: &[RenderPage],
    diagnostics: &[Diagnostic],
    dpi: f64,
) -> Result<AnnotatorResult> {
    let mut pairs = Vec::new();
    for page in pages {
        if page.image_bytes.is_none() {
            continue;
        }
        let pair = annotate_page(page, diagnostics, dpi)?;
        pairs.push(pair);
    }
    let page_anns: Vec<Vec<LintAnnotation>> = pairs.iter().map(|p| p.annotations.clone()).collect();
    let legend = build_legend(&page_anns);
    Ok(AnnotatorResult { pairs, legend })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Decode the page image bytes into an RgbaImage.
fn decode_image(page: &RenderPage) -> Result<RgbaImage> {
    let source_bytes = page
        .image_bytes
        .clone()
        .ok_or_else(|| anyhow::anyhow!("page {} has no image", page.page_number))?;
    let img =
        image::load_from_memory(&source_bytes).with_context(|| "failed to decode page image")?;
    Ok(img.to_rgba8())
}

/// Encode an RgbaImage as a PNG into a Vec<u8>.
fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .ok();
    buf
}

/// Base64 encode bytes.
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::diagnostics::Severity;

    fn make_test_image(width: u32, height: u32) -> Vec<u8> {
        let img: RgbaImage = image::ImageBuffer::new(width, height);
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }

    // -----------------------------------------------------------------------
    // Severity colours
    // -----------------------------------------------------------------------

    /// test_severity_colors — verify error=vermilion, warning=amber, info=slate.
    #[test]
    fn test_severity_colors() {
        assert_eq!(severity_color("error"), COLOR_ERROR);
        assert_eq!(severity_color("warning"), COLOR_WARNING);
        assert_eq!(severity_color("info"), COLOR_INFO);
        assert_eq!(severity_color("hint"), COLOR_INFO);

        // Hex encoding
        assert_eq!(severity_color_hex("error"), "#E2483A");
        assert_eq!(severity_color_hex("warning"), "#F4BE32");
        assert_eq!(severity_color_hex("info"), "#93A0B8");
    }

    /// test_severity_color_error — error diagnostics get vermilion.
    #[test]
    fn test_severity_color_error() {
        let color = severity_color("error");
        assert_eq!(color, [226, 72, 58]);
    }

    /// test_severity_color_warning — warnings get amber.
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

    /// test_severity_line_width — thicker for errors.
    #[test]
    fn test_severity_line_width() {
        assert_eq!(severity_line_width("error"), 2);
        assert_eq!(severity_line_width("warning"), 1);
        assert_eq!(severity_line_width("info"), 1);
    }

    /// test_severity_dashed — warnings use dashed outline.
    #[test]
    fn test_severity_dashed() {
        assert!(severity_dashed("warning"));
        assert!(!severity_dashed("error"));
        assert!(!severity_dashed("info"));
    }

    /// test_normalise_severity — canonical forms.
    #[test]
    fn test_normalise_severity() {
        assert_eq!(normalise_severity("ERROR"), "error");
        assert_eq!(normalise_severity("WARN"), "warning");
        assert_eq!(normalise_severity("WARNING"), "warning");
        assert_eq!(normalise_severity("INFO"), "info");
        assert_eq!(normalise_severity("HINT"), "info");
        assert_eq!(normalise_severity("FATAL"), "error");
    }

    // -----------------------------------------------------------------------
    // Collision avoidance
    // -----------------------------------------------------------------------

    /// test_boxes_overlap — basic overlap detection.
    #[test]
    fn test_boxes_overlap_detection() {
        // Two boxes that overlap
        assert!(boxes_overlap(
            [0.0, 0.0, 50.0, 50.0],
            [30.0, 30.0, 50.0, 50.0]
        ));
        // Two boxes that touch but do not overlap (adjacent)
        assert!(!boxes_overlap(
            [0.0, 0.0, 50.0, 50.0],
            [50.0, 0.0, 50.0, 50.0]
        ));
        // Two boxes that do not overlap
        assert!(!boxes_overlap(
            [0.0, 0.0, 50.0, 50.0],
            [100.0, 100.0, 50.0, 50.0]
        ));
    }

    /// test_collision_avoidance — overlapping annotations get offset labels.
    #[test]
    fn test_collision_avoidance() {
        // Create two overlapping annotations
        let mut annotations = vec![
            LintAnnotation {
                number: 1,
                bbox_pt: [100.0, 100.0, 50.0, 50.0],
                code: "E1".to_string(),
                message: "first".to_string(),
                severity: "error".to_string(),
                label_offset: [0.0, 0.0],
            },
            LintAnnotation {
                number: 2,
                bbox_pt: [120.0, 120.0, 50.0, 50.0], // overlaps with first
                code: "E2".to_string(),
                message: "second".to_string(),
                severity: "error".to_string(),
                label_offset: [0.0, 0.0],
            },
        ];

        resolve_label_collisions(&mut annotations, 500, 500);

        // First annotation should have no offset; second should be offset
        assert_eq!(annotations[0].label_offset, [0.0, 0.0]);
        // The second annotation should have a non-zero offset due to collision
        let offset = annotations[1].label_offset;
        assert!(
            offset[0] != 0.0 || offset[1] != 0.0,
            "expected non-zero offset for collision"
        );
    }

    /// test_no_collision — non-overlapping annotations stay unoffset.
    #[test]
    fn test_no_collision_needed() {
        let mut annotations = vec![
            LintAnnotation {
                number: 1,
                bbox_pt: [10.0, 10.0, 50.0, 50.0],
                code: "E1".to_string(),
                message: "far apart".to_string(),
                severity: "error".to_string(),
                label_offset: [0.0, 0.0],
            },
            LintAnnotation {
                number: 2,
                bbox_pt: [300.0, 300.0, 50.0, 50.0],
                code: "E2".to_string(),
                message: "far apart".to_string(),
                severity: "error".to_string(),
                label_offset: [0.0, 0.0],
            },
        ];

        resolve_label_collisions(&mut annotations, 500, 500);

        assert_eq!(annotations[0].label_offset, [0.0, 0.0]);
        assert_eq!(annotations[1].label_offset, [0.0, 0.0]);
    }

    // -----------------------------------------------------------------------
    // Legend generation
    // -----------------------------------------------------------------------

    /// test_legend_generation — verify legend has correct counts.
    #[test]
    fn test_legend_generation() {
        let annotations = vec![vec![
            LintAnnotation {
                number: 1,
                bbox_pt: [0.0, 0.0, 10.0, 10.0],
                code: "LIN-E-001".to_string(),
                message: "error one".to_string(),
                severity: "error".to_string(),
                label_offset: [0.0, 0.0],
            },
            LintAnnotation {
                number: 2,
                bbox_pt: [0.0, 0.0, 10.0, 10.0],
                code: "LIN-E-001".to_string(),
                message: "error two".to_string(),
                severity: "error".to_string(),
                label_offset: [0.0, 0.0],
            },
            LintAnnotation {
                number: 3,
                bbox_pt: [0.0, 0.0, 10.0, 10.0],
                code: "LIN-W-001".to_string(),
                message: "warning one".to_string(),
                severity: "warning".to_string(),
                label_offset: [0.0, 0.0],
            },
        ]];

        let legend = build_legend(&annotations);

        assert_eq!(legend.version, LEGEND_VERSION);
        assert_eq!(legend.total_annotations, 3);
        assert_eq!(legend.codes.len(), 2);

        // Check severity counts
        assert_eq!(*legend.severity_counts.get("error").unwrap(), 2);
        assert_eq!(*legend.severity_counts.get("warning").unwrap(), 1);

        // Check per-code info
        let e_code = legend.codes.iter().find(|c| c.code == "LIN-E-001").unwrap();
        assert_eq!(e_code.count, 2);
        assert_eq!(e_code.severity, "error");
        assert_eq!(e_code.color, "#E2483A");

        let w_code = legend.codes.iter().find(|c| c.code == "LIN-W-001").unwrap();
        assert_eq!(w_code.count, 1);
        assert_eq!(w_code.severity, "warning");
        assert_eq!(w_code.color, "#F4BE32");
    }

    /// test_legend_empty — empty annotations produce empty legend.
    #[test]
    fn test_legend_empty() {
        let legend = build_legend(&[]);
        assert_eq!(legend.total_annotations, 0);
        assert!(legend.codes.is_empty());
        assert!(legend.severity_counts.is_empty());
    }

    /// test_legend_serialization — legend roundtrips through JSON.
    #[test]
    fn test_legend_serialization() {
        let annotations = vec![vec![LintAnnotation {
            number: 1,
            bbox_pt: [0.0, 0.0, 10.0, 10.0],
            code: "LIN-E-001".to_string(),
            message: "test".to_string(),
            severity: "error".to_string(),
            label_offset: [0.0, 0.0],
        }]];

        let legend = build_legend(&annotations);
        let json = serde_json::to_string(&legend).expect("serialize legend");
        let back: AnnotatorLegend = serde_json::from_str(&json).expect("deserialize legend");
        assert_eq!(back.total_annotations, 1);
        assert_eq!(back.codes.len(), 1);
    }

    /// test_legend_to_json — pretty-printed JSON.
    #[test]
    fn test_legend_to_json() {
        let annotations = vec![vec![LintAnnotation {
            number: 1,
            bbox_pt: [0.0, 0.0, 10.0, 10.0],
            code: "LIN-E-001".to_string(),
            message: "test".to_string(),
            severity: "error".to_string(),
            label_offset: [0.0, 0.0],
        }]];
        let legend = build_legend(&annotations);
        let json = legend_to_json(&legend).expect("pretty JSON");
        assert!(json.starts_with("{"));
        assert!(json.contains("\n"));
    }

    // -----------------------------------------------------------------------
    // bbox_to_pixels
    // -----------------------------------------------------------------------

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
        let expected_px = 72.0 * (300.0 / 72.0);
        assert!((x - expected_px).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // annotate_page
    // -----------------------------------------------------------------------

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
        assert_eq!(pair.annotations[0].severity, "error");
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
        assert_eq!(pair.annotations.len(), 1);
        assert_eq!(pair.annotations[0].code, "REN-I-001");
        assert_eq!(pair.annotations[0].severity, "info");
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
        assert_eq!(pair.annotations[0].severity, "error");
        assert_eq!(pair.annotations[1].severity, "warning");
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

    // -----------------------------------------------------------------------
    // annotate_pages (full flow)
    // -----------------------------------------------------------------------

    /// test_annotate_pages_returns_legend — full flow produces a legend.
    #[test]
    fn test_annotate_pages_returns_legend() {
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
        let result = annotate_pages(&pages, &diags, 150.0).unwrap();
        assert_eq!(result.pairs.len(), 2);
        assert_eq!(result.legend.version, LEGEND_VERSION);
        assert_eq!(result.legend.total_annotations, 4); // 2 pages * 2 diagnostics
        assert_eq!(*result.legend.severity_counts.get("error").unwrap(), 2);
        assert_eq!(*result.legend.severity_counts.get("warning").unwrap(), 2);
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
        let result = annotate_pages(&pages, &diags, 150.0).unwrap();
        assert_eq!(result.pairs.len(), 2);
        for pair in &result.pairs {
            assert!(!pair.clean.data.is_empty());
            assert!(!pair.marked.data.is_empty());
        }
        assert_eq!(result.legend.total_annotations, 2);
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
        let result = annotate_pages(&pages, &diags, 150.0).unwrap();
        assert_eq!(result.pairs.len(), 1);
        assert_eq!(result.pairs[0].clean.width, 100);
    }

    // -----------------------------------------------------------------------
    // extract_diag_bbox
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Serde round-trips
    // -----------------------------------------------------------------------

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
                label_offset: [0.0, 0.0],
            }],
        };
        let json = serde_json::to_string(&pair).expect("serialize");
        let back: MarkedImagePair = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.clean.width, 100);
        assert_eq!(back.marked.height, 100);
        assert_eq!(back.annotations.len(), 1);
        assert_eq!(back.annotations[0].code, "LIN-E-001");
        assert_eq!(back.annotations[0].label_offset, [0.0, 0.0]);
    }

    /// test_annotator_result_serde — full result roundtrip.
    #[test]
    fn test_annotator_result_serde() {
        let result = AnnotatorResult {
            pairs: vec![MarkedImagePair {
                clean: ImageData {
                    data: "clean".to_string(),
                    width: 100,
                    height: 100,
                    visual_tokens: 4,
                },
                marked: ImageData {
                    data: "marked".to_string(),
                    width: 100,
                    height: 100,
                    visual_tokens: 4,
                },
                annotations: vec![],
            }],
            legend: AnnotatorLegend {
                version: LEGEND_VERSION.to_string(),
                codes: vec![],
                total_annotations: 0,
                severity_counts: HashMap::new(),
            },
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let back: AnnotatorResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.pairs.len(), 1);
        assert_eq!(back.legend.version, LEGEND_VERSION);
    }
}
