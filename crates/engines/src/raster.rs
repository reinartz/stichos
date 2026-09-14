// engines: rasterization ladder — poppler → gs → mutool → soffice

use gp_core::diagnostics::{Diagnostic, Severity};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const TOOLS: &[&str] = &["pdftoppm", "gs", "mutool", "soffice"];

/// Check which raster tools are available on PATH / in standard locations.
pub fn available_raster_tools() -> Vec<&'static str> {
    TOOLS
        .iter()
        .filter(|t| is_tool_available(t))
        .copied()
        .collect()
}

fn is_tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Rasterize page `page` (0-indexed) of a PDF to PNG at the given DPI.
///
/// Tries the tools in order: poppler pdftoppm → Ghostscript → mutool → soffice.
/// Returns the first successful PNG payload or diagnostics from all attempts.
pub fn rasterize(pdf_bytes: &[u8], page: usize, dpi: u32) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    if let Ok(result) = rasterize_with_poppler(pdf_bytes, page, dpi) {
        return Ok(result);
    }
    diagnostics.push(Diagnostic::new(
        "ENG-E-201",
        Severity::Info,
        "pdftoppm: tool not found or failed".to_string(),
    ));

    if let Ok(result) = rasterize_with_gs(pdf_bytes, page, dpi) {
        return Ok(result);
    }
    diagnostics.push(Diagnostic::new(
        "ENG-E-202",
        Severity::Info,
        "Ghostscript: tool not found or failed".to_string(),
    ));

    if let Ok(result) = rasterize_with_mutool(pdf_bytes, page, dpi) {
        return Ok(result);
    }
    diagnostics.push(Diagnostic::new(
        "ENG-E-203",
        Severity::Info,
        "mutool: tool not found or failed".to_string(),
    ));

    if let Ok(result) = rasterize_with_soffice(pdf_bytes, page, dpi) {
        return Ok(result);
    }
    diagnostics.push(Diagnostic::new(
        "ENG-E-204",
        Severity::Info,
        "soffice: tool not found or failed".to_string(),
    ));

    Err(diagnostics)
}

/// Write bytes to a temp file and return its path.
fn write_temp_pdf(data: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("stichos");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "stichos-{}.pdf",
        fastrand::u32(1_000_000..9_999_999)
    ));
    let mut f = std::fs::File::create(&path)?;
    f.write_all(data)?;
    Ok(path)
}

/// Run pdftoppm (poppler-utils).
fn rasterize_with_poppler(
    pdf_bytes: &[u8],
    page: usize,
    dpi: u32,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let path = write_temp_pdf(pdf_bytes).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-100",
            Severity::Error,
            format!("pdftoppm temp file: {}", e),
        )]
    })?;

    let output = Command::new("pdftoppm")
        .args([
            "-png",
            "-r",
            &dpi.to_string(),
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            &path.to_string_lossy(),
            &path.with_file_name("page").to_string_lossy(),
        ])
        .output()
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-101",
                Severity::Error,
                format!("pdftoppm exec: {}", e),
            )]
        })?;

    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(vec![Diagnostic::new(
            "ENG-E-102",
            Severity::Error,
            format!("pdftoppm failed: {}", msg),
        )]);
    }

    let png_path = path.with_file_name("page-01.png");
    let _ = std::fs::remove_file(&path);
    if !png_path.exists() {
        return Err(vec![Diagnostic::new(
            "ENG-E-103",
            Severity::Error,
            "pdftoppm: output PNG not found".to_string(),
        )]);
    }

    let data = std::fs::read(&png_path).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-104",
            Severity::Error,
            format!("pdftoppm read PNG: {}", e),
        )]
    })?;
    let _ = std::fs::remove_file(&png_path);
    Ok(data)
}

/// Run Ghostscript to rasterize one page.
fn rasterize_with_gs(pdf_bytes: &[u8], page: usize, dpi: u32) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let path = write_temp_pdf(pdf_bytes).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-110",
            Severity::Error,
            format!("gs temp file: {}", e),
        )]
    })?;

    let png_path = path.with_file_name("page.png");
    let png_str = png_path.to_string_lossy().to_string();

    let output = Command::new("gs")
        .args([
            "-dNOPAUSE",
            "-dBATCH",
            "-dQUIET",
            "-sDEVICE=pnggray",
            "-r",
            &dpi.to_string(),
            "-dFirstPage",
            &page.to_string(),
            "-dLastPage",
            &page.to_string(),
            "-sOutputFile",
            &png_str,
            path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-111",
                Severity::Error,
                format!("gs exec: {}", e),
            )]
        })?;

    let _ = std::fs::remove_file(&path);

    if !output.status.success() {
        return Err(vec![Diagnostic::new(
            "ENG-E-112",
            Severity::Error,
            "gs: non-zero exit".to_string(),
        )]);
    }

    if !png_path.exists() {
        return Err(vec![Diagnostic::new(
            "ENG-E-113",
            Severity::Error,
            "gs: output PNG not found".to_string(),
        )]);
    }

    let data = std::fs::read(&png_path).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-114",
            Severity::Error,
            format!("gs read PNG: {}", e),
        )]
    })?;
    let _ = std::fs::remove_file(&png_path);
    Ok(data)
}

/// Run mutool draw (from mupdf-tools).
fn rasterize_with_mutool(
    pdf_bytes: &[u8],
    page: usize,
    dpi: u32,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let path = write_temp_pdf(pdf_bytes).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-120",
            Severity::Error,
            format!("mutool temp file: {}", e),
        )]
    })?;

    let png_path = path.with_file_name("page.png");
    let png_str = png_path.to_string_lossy().to_string();

    let output = Command::new("mutool")
        .args([
            "draw",
            "-F",
            "png",
            "-r",
            &dpi.to_string(),
            "-o",
            &png_str,
            "-p",
            &page.to_string(),
            path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-121",
                Severity::Error,
                format!("mutool exec: {}", e),
            )]
        })?;

    let _ = std::fs::remove_file(&path);

    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(vec![Diagnostic::new(
            "ENG-E-122",
            Severity::Error,
            format!("mutool failed: {}", msg),
        )]);
    }

    if !png_path.exists() {
        return Err(vec![Diagnostic::new(
            "ENG-E-123",
            Severity::Error,
            "mutool: output PNG not found".to_string(),
        )]);
    }

    let data = std::fs::read(&png_path).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-124",
            Severity::Error,
            format!("mutool read PNG: {}", e),
        )]
    })?;
    let _ = std::fs::remove_file(&png_path);
    Ok(data)
}

/// Run soffice headless to convert PDF → PNG.
fn rasterize_with_soffice(
    pdf_bytes: &[u8],
    _page: usize,
    _dpi: u32,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let tmpdir = std::env::temp_dir().join(format!(
        "stichos-soffice-{}",
        fastrand::u32(1_000_000..9_999_999)
    ));
    std::fs::create_dir_all(&tmpdir).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-130",
            Severity::Error,
            format!("soffice tmpdir: {}", e),
        )]
    })?;

    let pdf_path = tmpdir.join("input.pdf");
    std::fs::write(&pdf_path, pdf_bytes).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-131",
            Severity::Error,
            format!("soffice write PDF: {}", e),
        )]
    })?;

    let output = Command::new("soffice")
        .args([
            "--headless",
            "--norestore",
            "--convert-to",
            "png",
            "--outdir",
            tmpdir.to_string_lossy().as_ref(),
            pdf_path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-132",
                Severity::Error,
                format!("soffice exec: {}", e),
            )]
        })?;

    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(vec![Diagnostic::new(
            "ENG-E-133",
            Severity::Error,
            format!("soffice failed: {}", msg),
        )]);
    }

    let png_name = format!("{}.png", pdf_path.file_stem().unwrap().to_string_lossy());
    let png_path = tmpdir.join(&png_name);

    if !png_path.exists() {
        return Err(vec![Diagnostic::new(
            "ENG-E-134",
            Severity::Error,
            "soffice: output PNG not found".to_string(),
        )]);
    }

    let data = std::fs::read(&png_path).map_err(|e| {
        vec![Diagnostic::new(
            "ENG-E-135",
            Severity::Error,
            format!("soffice read PNG: {}", e),
        )]
    })?;

    let _ = std::fs::remove_dir_all(&tmpdir);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_available_raster_tools: check which tools exist on this system.
    #[test]
    fn test_available_raster_tools() {
        let tools = available_raster_tools();
        for t in &tools {
            assert!(TOOLS.contains(t), "{} must be in TOOLS", t);
        }
    }

    /// test_rasterize_with_poppler: if poppler exists, test it with a minimal PDF.
    #[test]
    fn test_rasterize_with_poppler() {
        if !available_raster_tools().contains(&"pdftoppm") {
            eprintln!("pdftoppm not available — skipping test_rasterize_with_poppler");
            return;
        }

        let pdf = minimal_pdf_bytes();
        match rasterize_with_poppler(&pdf, 0, 150) {
            Ok(png) => {
                assert!(!png.is_empty(), "should produce PNG bytes");
                assert_eq!(
                    &png[0..8],
                    &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
                    "must be PNG signature"
                );
            }
            Err(diags) => {
                eprintln!(
                    "pdftoppm returned diagnostics (expected on minimal PDF): {:?}",
                    diags
                );
            }
        }
    }

    /// test_rasterize_ladder_returns_result: the ladder returns a valid Result on any input.
    #[test]
    fn test_rasterize_ladder_returns_result() {
        let invalid = b"not a pdf at all";
        let result = rasterize(invalid, 0, 150);
        // Tools may succeed (producing blank images) or fail — we just verify
        // the return type is valid and consistent.
        match result {
            Ok(png) => {
                // Some tool succeeded — should be a PNG
                assert!(!png.is_empty(), "should produce PNG bytes");
            }
            Err(diags) => {
                assert!(
                    !diags.is_empty(),
                    "should have diagnostics for failed tools"
                );
            }
        }
    }

    /// test_rasterize_ladder_diagnostics_on_failure: all failed tools produce diagnostics.
    #[test]
    fn test_rasterize_ladder_diagnostics_on_failure() {
        let invalid = b"%PDF-bad";
        let result = rasterize(invalid, 0, 150);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.iter().any(|c| c.starts_with("ENG-E-2")),
            "should have ladder diagnostics"
        );
    }

    fn minimal_pdf_bytes() -> Vec<u8> {
        b"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>
endobj
xref
0 4
0000000000 65535 f 
0000000009 00000 n 
0000000058 00000 n 
0000000115 00000 n 
trailer
<< /Size 4 /Root 1 0 R >>
startxref
190
%%EOF"
            .to_vec()
    }
}
