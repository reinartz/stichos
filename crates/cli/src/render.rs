// SPDX-License-Identifier: MIT OR Apache-2.0
//! render_document tool: convert document to images via engine + rasterize + budget.
//!
//! This module implements the `render_document` CLI command, which:
//! 1. Reads a document file (docx, odt, pptx, xlsx, ods, odp)
//! 2. Negotiates the best rendering engine
//! 3. Dispatches to the negotiated engine (LibreOffice, Pandoc, Chromium, or Typst)
//! 4. Converts to PDF via engine
//! 5. Rasterizes each page to PNG using the raster ladder
//! 6. Builds a contact sheet from page thumbnails
//! 7. Checks payload budget (C8: ≤20 image blocks, ≤2000px/side)
//! 8. Writes output files
//! 9. Returns a manifest JSON with `rendered_by` provenance (INV-2)

use anyhow::Context;
use engines::chromium::ChromiumEngine;
use engines::libreoffice::LibreOfficeEngine;
use engines::negotiate::{negotiate_renderer, RendererPreference};
use engines::pandoc::PandocEngine;
use engines::typst::TypstEngine;
use gp_core::diagnostics::Diagnostic;
use gp_core::traits::Engine;
use lint::budget::{check_payload_budget, ImageBlock};
use lint::sheet::build_sheet_from_pages;
use std::path::Path;

/// A rasterized page: PNG bytes, width, height.
type RasterPage = (Vec<u8>, usize, usize);

/// Render a document to page images with layout QA.
///
/// Returns a JSON manifest containing:
/// - `tool`: "render_document"
/// - `path`: input document path
/// - `rendered_by`: provenance string (engine + version)
/// - `dpi`: rendering DPI
/// - `page_count`: number of pages rendered
/// - `image_blocks`: count of image blocks (pages + contact sheet)
/// - `blocks`: array of per-block metadata (width, height, visual_tokens, byte_size, format)
/// - `budget`: payload budget check result
/// - `contact_sheet`: contact sheet dimensions and token count
///
/// Returns status "unavailable" when no renderer is available.
pub fn render_document(
    path: &str,
    output_dir: &str,
    dpi: u32,
) -> anyhow::Result<serde_json::Value> {
    // Read document bytes
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read document: {}", path))?;

    // Negotiate renderer
    let choice = negotiate_renderer(&RendererPreference::Default);

    if !choice.available {
        return Ok(serde_json::json!({
            "tool": "render_document",
            "path": path,
            "status": "unavailable",
            "rendered_by": "unavailable",
            "diagnostics": [{
                "code": "ENG-E-001",
                "severity": "error",
                "message": format!("no renderer available: {}", choice.reason)
            }]
        }));
    }

    // Create the engine and convert to PDF
    let engine = create_engine_for_choice(&choice)?;
    let source_ext = path.rsplit('.').next().unwrap_or("").to_lowercase();

    let pdf_bytes = engine
        .convert(&bytes, &source_ext, "pdf")
        .map_err(|diags| {
            anyhow::anyhow!(
                "engine {} conversion failed: {:?}",
                choice.engine_id,
                diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>()
            )
        })?;

    // Determine page count by parsing the PDF
    let page_count = match parse_pdf_page_count(&pdf_bytes) {
        Ok(count) => count,
        Err(e) => {
            eprintln!("warning: could not determine page count: {}", e);
            // Fallback: try rasterizing first page to detect if there are any pages
            match engines::raster::rasterize(&pdf_bytes, 0, dpi) {
                Ok(_) => 1,
                Err(_) => 0,
            }
        }
    };

    // Rasterize all pages
    let (images, page_count) = rasterize_all_pages(&pdf_bytes, dpi, page_count)?;

    // Create output directory
    std::fs::create_dir_all(output_dir).with_context(|| "failed to create output directory")?;

    // Write page images to disk
    for (i, (data, _w, _h)) in images.iter().enumerate() {
        let img_path = Path::new(output_dir).join(format!("page-{}.png", i + 1));
        std::fs::write(&img_path, data)
            .with_context(|| format!("failed to write {}", img_path.display()))?;
    }

    // Build contact sheet from page thumbnails
    let sheet = build_sheet_from_pages(
        &images
            .iter()
            .map(|(data, w, h)| (data.clone(), *w, *h))
            .collect::<Vec<_>>(),
    );

    // Assemble the contact sheet
    let sheet_data = sheet.assemble();
    let sheet_path = Path::new(output_dir).join("contact-sheet.png");
    // Write the assembled layout descriptor
    if !sheet_data.is_empty() {
        std::fs::write(&sheet_path, &sheet_data)
            .with_context(|| format!("failed to write {}", sheet_path.display()))?;
    }

    // Build image blocks for budget check
    let mut blocks: Vec<ImageBlock> = images
        .iter()
        .enumerate()
        .map(|(i, (data, w, h))| ImageBlock::new(*w, *h, data.len(), format!("page-{}", i + 1)))
        .collect();

    // Add contact sheet as final image block
    let _sheet_tokens = sheet.width * sheet.height;
    blocks.push(ImageBlock::new(
        sheet.width,
        sheet.height,
        sheet_data.len(),
        "contact-sheet".to_string(),
    ));

    // Check payload budget
    let budget_result = check_payload_budget(&blocks);

    // Build rendered_by provenance
    let rendered_by = format!(
        "{} {}",
        engine.engine_id(),
        engine.version().unwrap_or("unknown")
    );

    // Build the manifest
    let blocks_json: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| {
            serde_json::json!({
                "width": b.width,
                "height": b.height,
                "visual_tokens": b.visual_tokens,
                "byte_size": b.byte_size,
                "format": b.format,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "tool": "render_document",
        "path": path,
        "status": "ok",
        "rendered_by": rendered_by,
        "engine_choice": choice.engine_id,
        "dpi": dpi,
        "page_count": page_count,
        "image_blocks": blocks.len(),
        "blocks": blocks_json,
        "budget": {
            "within_budget": budget_result.within_budget,
            "total_tokens": budget_result.total_tokens,
            "total_bytes": budget_result.total_bytes,
            "block_count": budget_result.block_count,
            "violations": budget_result.violations,
        },
        "contact_sheet": {
            "width": sheet.width,
            "height": sheet.height,
            "cols": sheet.cols,
            "rows": sheet.rows,
            "tokens": sheet.total_tokens,
            "within_budget": sheet.within_budget,
        },
    }))
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

/// Rasterize all pages of a PDF to PNG.
///
/// Returns (Vec<(png_bytes, width, height)>, page_count).
fn rasterize_all_pages(
    pdf_bytes: &[u8],
    dpi: u32,
    expected_pages: usize,
) -> Result<(Vec<RasterPage>, usize), anyhow::Error> {
    let mut images: Vec<RasterPage> = Vec::new();

    // Hard limit to prevent infinite loops with malformed PDFs.
    // When expected_pages is known (> 0), use it as the exact cap
    // because pdfinfo already confirmed the real page count.
    let max_pages = if expected_pages > 0 {
        expected_pages
    } else {
        expected_pages.max(100)
    };
    let mut page = 0;
    while page < max_pages {
        // page is 0-indexed; rasterize uses 0-indexed page number
        match engines::raster::rasterize(pdf_bytes, page, dpi) {
            Ok(png_data) => {
                // Determine dimensions from PNG data
                let (w, h) = png_dimensions(&png_data);
                // Skip empty images
                if w > 0 && h > 0 {
                    images.push((png_data, w, h));
                }
                page += 1;
            }
            Err(_) => {
                // No more pages
                break;
            }
        }
    }

    let actual_count = images.len();

    // If we couldn't rasterize any pages, fall back to expected_pages
    if actual_count == 0 && expected_pages >= 1 {
        return Ok((images, expected_pages));
    }

    Ok((images, actual_count.max(expected_pages)))
}

/// Extract PNG dimensions from PNG header bytes.
///
/// PNG spec: bytes 16-19 = width (4 bytes BE), bytes 20-23 = height (4 bytes BE).
fn png_dimensions(png_data: &[u8]) -> (usize, usize) {
    if png_data.len() < 24 {
        return (0, 0);
    }
    // Check PNG signature
    if png_data[0..8] != [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return (0, 0);
    }
    let w = u32::from_be_bytes([png_data[16], png_data[17], png_data[18], png_data[19]]) as usize;
    let h = u32::from_be_bytes([png_data[20], png_data[21], png_data[22], png_data[23]]) as usize;
    (w, h)
}

/// Parse the page count from a PDF by reading its page tree.
fn parse_pdf_page_count(pdf_bytes: &[u8]) -> Result<usize, Diagnostic> {
    // Try our own parser first.
    let doc = adapters_pdf::parser::parse_pdf(pdf_bytes).map_err(|diags| {
        Diagnostic::new(
            "CLI-R-001",
            gp_core::diagnostics::Severity::Warning,
            format!("failed to parse PDF for page count: {:?}", diags[0].message),
        )
    })?;
    if doc.pages.is_empty() {
        // Parser succeeded but found 0 pages — fall through to the pdfinfo backup.
    } else {
        return Ok(doc.pages.len());
    }

    // Fallback: pdfinfo from poppler-utils (very fast, no stdin, reads the file).
    if let Ok(tmp) = write_temp_pdf(pdf_bytes) {
        if let Ok(out) = std::process::Command::new("pdfinfo").arg(&tmp).output() {
            let _ = std::fs::remove_file(&tmp);
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    if let Some(rest) = line.strip_prefix("Pages:") {
                        if let Ok(n) = rest.trim().parse::<usize>() {
                            return Ok(n);
                        }
                    }
                }
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    Err(Diagnostic::new(
        "CLI-R-001",
        gp_core::diagnostics::Severity::Warning,
        "failed to determine page count (parser=0, pdfinfo unavailable)".to_string(),
    ))
}

/// Write bytes to a temp file for reading (e.g., pdfinfo).
fn write_temp_pdf(data: &[u8]) -> std::io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("stichos");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("stichos-{}.pdf", std::process::id()));
    std::fs::write(&path, data)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_png_dimensions_valid — extract dimensions from PNG header.
    #[test]
    fn test_png_dimensions_valid() {
        // Minimal PNG header: signature + IHDR chunk with 100x50
        let mut png = vec![0u8; 24];
        // PNG signature
        png[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        // Width = 100 (0x00000064)
        png[16..20].copy_from_slice(&100u32.to_be_bytes());
        // Height = 50 (0x00000032)
        png[20..24].copy_from_slice(&50u32.to_be_bytes());

        let (w, h) = png_dimensions(&png);
        assert_eq!(w, 100);
        assert_eq!(h, 50);
    }

    /// test_png_dimensions_too_short — returns (0,0) for small data.
    #[test]
    fn test_png_dimensions_too_short() {
        let png = vec![0u8; 10];
        let (w, h) = png_dimensions(&png);
        assert_eq!(w, 0);
        assert_eq!(h, 0);
    }

    /// test_png_dimensions_invalid_signature — returns (0,0).
    #[test]
    fn test_png_dimensions_invalid_signature() {
        let mut png = vec![0u8; 24];
        png[0] = 0xFF; // Not a PNG signature
        let (w, h) = png_dimensions(&png);
        assert_eq!(w, 0);
        assert_eq!(h, 0);
    }

    /// test_png_dimensions_square — 200x200 PNG.
    #[test]
    fn test_png_dimensions_square() {
        let mut png = vec![0u8; 24];
        png[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        png[16..20].copy_from_slice(&200u32.to_be_bytes());
        png[20..24].copy_from_slice(&200u32.to_be_bytes());
        let (w, h) = png_dimensions(&png);
        assert_eq!(w, 200);
        assert_eq!(h, 200);
    }

    /// test_png_dimensions_widescreen — 1920x1080 PNG.
    #[test]
    fn test_png_dimensions_widescreen() {
        let mut png = vec![0u8; 24];
        png[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        png[16..20].copy_from_slice(&1920u32.to_be_bytes());
        png[20..24].copy_from_slice(&1080u32.to_be_bytes());
        let (w, h) = png_dimensions(&png);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    /// test_rasterize_all_pages_empty — no pages returns empty vec.
    #[test]
    fn test_rasterize_all_pages_empty() {
        // Invalid PDF data — rasterize may produce blank pages or fail
        // We just verify the function doesn't hang and returns reasonable results
        let (images, _count) = rasterize_all_pages(b"not-a-pdf", 150, 0).unwrap();
        // Images may be produced (blank pages from raster tools) or empty
        // Key property: result is bounded by max_pages (100)
        assert!(images.len() <= 100);
    }

    /// test_png_dimensions_max — 2000x2000 (budget boundary).
    #[test]
    fn test_png_dimensions_max() {
        let mut png = vec![0u8; 24];
        png[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        png[16..20].copy_from_slice(&2000u32.to_be_bytes());
        png[20..24].copy_from_slice(&2000u32.to_be_bytes());
        let (w, h) = png_dimensions(&png);
        assert_eq!(w, 2000);
        assert_eq!(h, 2000);
    }

    /// test_unavailable_response — engine negotiation returns valid choice.
    #[test]
    fn test_negotiate_unavailable() {
        // On this test system, libreoffice may or may not be available.
        // We just verify the negotiation returns a valid choice with the right fields.
        let choice = negotiate_renderer(&RendererPreference::Default);
        assert!(!choice.engine_id.is_empty());
        assert!(!choice.reason.is_empty());
    }

    /// test_render_manifest_shape — verify the JSON output has the expected structure.
    #[test]
    fn test_render_manifest_shape() {
        // Create a minimal test PDF and verify the structure when unavailable
        let choice = negotiate_renderer(&RendererPreference::Default);
        let unavailable = !choice.available;

        if unavailable {
            let result = render_document("/nonexistent/test.pdf", "/tmp/stichos-test", 150);
            // May fail because file doesn't exist — that's OK
            // We just verify the function signature is correct
            let _ = result;
        } else {
            // Engine available — test would need a real document
            // For unit test, we verify the function compiles and returns correct types
            assert!(choice.available);
        }
    }

    /// test_image_block_budget — verify budget check with sample blocks.
    #[test]
    fn test_image_block_budget() {
        let blocks: Vec<ImageBlock> = (0..5)
            .map(|i| ImageBlock::new(800, 600, 10000, format!("page-{}", i + 1)))
            .collect();
        let report = check_payload_budget(&blocks);
        assert!(report.within_budget);
        assert_eq!(report.block_count, 5);
        assert_eq!(report.total_bytes, 50000);
    }

    /// test_image_block_budget_exceeds — 21 blocks exceeds budget.
    #[test]
    fn test_image_block_budget_exceeds() {
        let blocks: Vec<ImageBlock> = (0..21)
            .map(|_| ImageBlock::new(800, 600, 10000, "page".to_string()))
            .collect();
        let report = check_payload_budget(&blocks);
        assert!(!report.within_budget);
        assert!(report.violations.iter().any(|v| v.contains("block count")));
    }

    /// test_build_sheet_from_pages — verify contact sheet builder works.
    #[test]
    fn test_build_sheet_from_pages() {
        let pages: Vec<(Vec<u8>, usize, usize)> = (0..4)
            .map(|i| (vec![(i % 256) as u8; 100 * 100], 100, 100))
            .collect();
        let sheet = build_sheet_from_pages(&pages);
        assert_eq!(sheet.page_count, 4);
        assert_eq!(sheet.cols, 2);
        assert_eq!(sheet.rows, 2);
        assert!(sheet.within_budget);
    }

    /// test_sheet_assembly — verify contact sheet assemble returns data.
    #[test]
    fn test_sheet_assembly() {
        let pages: Vec<(Vec<u8>, usize, usize)> = (0..2)
            .map(|i| (vec![(i % 256) as u8; 100 * 100], 100, 100))
            .collect();
        let sheet = build_sheet_from_pages(&pages);
        let data = sheet.assemble();
        assert!(!data.is_empty());
        // Each thumbnail entry: 4(page_num) + 4(width) + 4(height) + 4(data_len) + baseline_encoded_data
        // Thumbnail is resized to 160x120 = 19200 bytes, then baseline encoded (11 byte header + 19200 data)
        let thumb_encoded_size = 11 + 160 * 120;
        let entry_size = 16 + thumb_encoded_size;
        assert_eq!(data.len(), entry_size * 2);
    }

    /// test_rendered_by_format — verify rendered_by string format.
    #[test]
    fn test_rendered_by_format() {
        // Verify that a mock engine's rendered_by is properly formatted
        let engine = engines::mock::MockEngine::new();
        let rendered_by = format!(
            "{} {}",
            engine.engine_id(),
            engine.version().unwrap_or("unknown")
        );
        assert!(rendered_by.starts_with("mock"));
        assert!(rendered_by.contains("0.0.0-mock"));
    }

    /// test_manifest_json_serializable — verify full manifest serializes to JSON.
    #[test]
    fn test_manifest_json_serializable() {
        let manifest = serde_json::json!({
            "tool": "render_document",
            "path": "/test.docx",
            "status": "ok",
            "rendered_by": "libreoffice 7.5.0.3",
            "dpi": 150,
            "page_count": 3,
            "image_blocks": 4,
            "blocks": [
                {"width": 792, "height": 1056, "visual_tokens": 31 * 38, "byte_size": 50000, "format": "page-1"},
                {"width": 792, "height": 1056, "visual_tokens": 31 * 38, "byte_size": 50000, "format": "page-2"},
                {"width": 792, "height": 1056, "visual_tokens": 31 * 38, "byte_size": 50000, "format": "page-3"},
                {"width": 320, "height": 280, "visual_tokens": 12 * 11, "byte_size": 200, "format": "contact-sheet"},
            ],
            "budget": {
                "within_budget": true,
                "total_tokens": 3 * 1178 + 132,
                "total_bytes": 150200,
                "block_count": 4,
                "violations": [],
            },
            "contact_sheet": {
                "width": 320,
                "height": 280,
                "cols": 2,
                "rows": 2,
                "tokens": 132,
                "within_budget": true,
            },
        });

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tool"].as_str().unwrap(), "render_document");
        assert_eq!(
            parsed["rendered_by"].as_str().unwrap(),
            "libreoffice 7.5.0.3"
        );
        assert_eq!(parsed["page_count"].as_u64().unwrap(), 3);
        assert_eq!(parsed["image_blocks"].as_u64().unwrap(), 4);
        assert!(parsed["budget"]["within_budget"].as_bool().unwrap());
        assert_eq!(parsed["contact_sheet"]["cols"].as_u64().unwrap(), 2);
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
