// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pdfium-based rasterization via the pdfium-render crate.
//!
//! Uses dynamic linking — `Pdfium::bind_to_system_library()` looks for
//! `libpdfium.so` in standard locations.  No static bundling.

use gp_core::diagnostics::{Diagnostic, Severity};
use pdfium_render::prelude::*;

/// Check whether the pdfium shared library can be loaded.
///
/// Returns `true` when `Pdfium::bind_to_system_library()` succeeds,
/// `false` otherwise.
pub fn pdfium_available() -> bool {
    Pdfium::bind_to_system_library().is_ok()
}

/// Rasterize page `page` (0-indexed) of a PDF to PNG at the given DPI.
///
/// If the pdfium library is unavailable, returns an error with a diagnostic
/// explaining that libpdfium was not found.
pub fn rasterize_with_pdfium(
    pdf_bytes: &[u8],
    page: usize,
    dpi: u32,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let bindings = Pdfium::bind_to_system_library().map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-140",
            Severity::Error,
            format!("pdfium: libpdfium.so not found ({e})"),
        )]
    })?;
    let pdfium = Pdfium::new(bindings);

    let pdf = pdfium
        .load_pdf_from_byte_slice(pdf_bytes, None)
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-141",
                Severity::Error,
                format!("pdfium: invalid PDF ({e})"),
            )]
        })?;

    let page_count = pdf.pages().len();
    let page_idx = page as i32;
    if page_idx >= page_count {
        return Err(vec![Diagnostic::new(
            "ENG-E-142",
            Severity::Error,
            format!(
                "pdfium: page {page} out of range (PDF has {page_count} pages)"
            ),
        )]);
    }

    let pdf_page = pdf.pages().get(page_idx).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-146",
            Severity::Error,
            format!("pdfium: could not access page ({e})"),
        )]
    })?;

    let page_width = pdf_page.width().value;
    let page_height = pdf_page.height().value;
    let target_width = (page_width * dpi as f32 / 72.0) as i32;
    let target_height = (page_height * dpi as f32 / 72.0) as i32;

    let render_config = PdfRenderConfig::new()
        .set_target_size(target_width, target_height);

    let bitmap = pdf_page.render_with_config(&render_config).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-143",
            Severity::Error,
            format!("pdfium: render failed ({e})"),
        )]
    })?;

    let mut png_bytes: Vec<u8> = Vec::new();
    bitmap
        .as_image()
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-144",
                Severity::Error,
                format!("pdfium: no image in render result ({e})"),
            )]
        })?
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-145",
                Severity::Error,
                format!("pdfium: PNG encode failed ({e})"),
            )]
        })?;

    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_pdfium_available_false: without libpdfium on the system
    /// pdfium_available() should return false.
    #[test]
    fn test_pdfium_available_false() {
        assert!(
            !pdfium_available(),
            "pdfium should not be available unless libpdfium.so is installed"
        );
    }

    /// test_rasterize_fails_without_pdfium: the function returns an error
    /// (not a panic) when the library is unavailable.
    #[test]
    fn test_rasterize_fails_without_pdfium() {
        let result = rasterize_with_pdfium(b"%PDF-1.4", 0, 150);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-140");
    }
}
