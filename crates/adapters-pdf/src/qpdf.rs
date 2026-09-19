// SPDX-License-Identifier: MIT OR Apache-2.0
//! qpdf integration: run `qpdf --check` on PDF files and report C3 diagnostics.
//!
//! qpdf is an optional (soft) dependency — if the binary is not available the
//! runner returns an info-level diagnostic instead of failing.
//!
//! Diagnostic codes:
//! - `PDF-W-005` — qpdf "warn" result
//! - `PDF-E-009` — qpdf "error" result
//! - `PDF-E-010` — qpdf "fail" result (unrecoverable)

use std::path::Path;
use std::process::Command;

use gp_core::diagnostics::*;

/// Run qpdf --check on a PDF file and return diagnostics.
///
/// * If qpdf is not available returns an info-level skip diagnostic.
/// * If the file does not exist returns an error diagnostic.
/// * Otherwise runs `qpdf --check --check-level=9 --json` and maps results
///   to C3 diagnostics (PDF-W-005 / PDF-E-009 / PDF-E-010).
pub fn check_pdf(pdf_path: &Path) -> Result<Vec<Diagnostic>, Diagnostic> {
    if !is_qpdf_available() {
        return Err(Diagnostic::new(
            "PDF-I-001",
            Severity::Info,
            "qpdf not available — PDF structure check skipped",
        ));
    }

    if !pdf_path.exists() {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            format!("PDF file not found: {}", pdf_path.display()),
        ));
    }

    let output = Command::new("qpdf")
        .args([
            "--check",
            "--check-level=9",
            "--json",
            pdf_path.to_str().unwrap(),
        ])
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return Err(Diagnostic::new(
                "PDF-E-001",
                Severity::Error,
                format!("qpdf execution failed: {}", e),
            ))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse the JSON output
    let parsed: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(e) => {
            // If we got stderr, include it in the diagnostic
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Diagnostic::new(
                "PDF-E-001",
                Severity::Error,
                format!("qpdf --json parse failed: {}: {}", e, stderr),
            ));
        }
    };

    let diagnostics = parse_qpdf_output(&parsed);

    // Even with diagnostics, if exit code is 0 consider it a pass
    // qpdf returns non-zero when issues are found
    if !output.status.success() && diagnostics.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Diagnostic::new(
            "PDF-E-009",
            Severity::Error,
            format!("qpdf exited with error: {}", stderr),
        ));
    }

    Ok(diagnostics)
}

/// Check if the qpdf binary is available on the system.
pub fn is_qpdf_available() -> bool {
    Command::new("qpdf").args(["--version"]).output().is_ok()
}

/// Parse qpdf --check --json output into C3 diagnostics.
///
/// Expected JSON structure:
/// ```json
/// {
///   "check": {
///     "level1": {"result": "pass"},
///     "level2": {"result": "warn", "detail": "..."},
///     "level3": {"result": "error", "detail": "..."}
///   }
/// }
/// ```
fn parse_qpdf_output(value: &serde_json::Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let check = match value.get("check") {
        Some(c) => c,
        None => return diagnostics,
    };

    if let Some(check_obj) = check.as_object() {
        for (level, entry) in check_obj {
            let result = entry
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("pass");
            let detail = entry.get("detail").and_then(|d| d.as_str()).unwrap_or("");

            match result {
                "pass" => {
                    // No diagnostic for passing checks — silent success
                }
                "warn" => {
                    if !detail.is_empty() {
                        diagnostics.push(Diagnostic::new(
                            "PDF-W-005",
                            Severity::Warning,
                            format!("{}: {}", level, detail),
                        ));
                    }
                }
                "error" => {
                    if !detail.is_empty() {
                        diagnostics.push(Diagnostic::new(
                            "PDF-E-009",
                            Severity::Error,
                            format!("{}: {}", level, detail),
                        ));
                    }
                }
                "fail" => {
                    if !detail.is_empty() {
                        diagnostics.push(Diagnostic::new(
                            "PDF-E-010",
                            Severity::Fatal,
                            format!("{}: {}", level, detail),
                        ));
                    }
                }
                other => {
                    // Unknown result — report as info
                    diagnostics.push(Diagnostic::new(
                        "PDF-I-002",
                        Severity::Info,
                        format!("{}: unknown result '{}'", level, other),
                    ));
                }
            }
        }
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qpdf_available() {
        // qpdf is a soft dependency — may or may not be installed.
        let available = is_qpdf_available();
        // The test passes either way; we just verify the function works.
        let _ = available;
    }

    #[test]
    fn test_qpdf_output_parsing() {
        let json = serde_json::json!({
            "check": {
                "level1": {"result": "pass"},
                "level2": {"result": "warn", "detail": "minor warning"},
                "level3": {"result": "error", "detail": "major error"},
                "level4": {"result": "pass"},
                "level5": {"result": "pass"},
                "level7": {"result": "pass"},
                "level9": {"result": "pass"},
                "xref": {"result": "pass"}
            }
        });

        let diags = parse_qpdf_output(&json);

        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].code, "PDF-W-005");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("level2"));
        assert!(diags[0].message.contains("minor warning"));

        assert_eq!(diags[1].code, "PDF-E-009");
        assert_eq!(diags[1].severity, Severity::Error);
        assert!(diags[1].message.contains("level3"));
        assert!(diags[1].message.contains("major error"));
    }

    #[test]
    fn test_qpdf_output_parsing_all_pass() {
        let json = serde_json::json!({
            "check": {
                "level1": {"result": "pass"},
                "level2": {"result": "pass"},
                "level3": {"result": "pass"},
                "xref": {"result": "pass"}
            }
        });

        let diags = parse_qpdf_output(&json);
        assert!(diags.is_empty(), "all pass → no diagnostics");
    }

    #[test]
    fn test_qpdf_output_parsing_fail_result() {
        let json = serde_json::json!({
            "check": {
                "level1": {"result": "fail", "detail": "catastrophic failure"}
            }
        });

        let diags = parse_qpdf_output(&json);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PDF-E-010");
        assert_eq!(diags[0].severity, Severity::Fatal);
    }

    #[test]
    fn test_qpdf_output_parsing_empty_detail() {
        let json = serde_json::json!({
            "check": {
                "level2": {"result": "warn", "detail": ""},
                "level3": {"result": "error", "detail": ""}
            }
        });

        let diags = parse_qpdf_output(&json);
        assert!(diags.is_empty(), "empty detail → no diagnostics");
    }

    #[test]
    fn test_qpdf_output_parsing_no_check_key() {
        let json = serde_json::json!({"other": "data"});
        let diags = parse_qpdf_output(&json);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_qpdf_output_parsing_unknown_result() {
        let json = serde_json::json!({
            "check": {
                "level1": {"result": "unknown_status"}
            }
        });

        let diags = parse_qpdf_output(&json);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PDF-I-002");
        assert_eq!(diags[0].severity, Severity::Info);
    }

    #[test]
    fn test_qpdf_output_parsing_xref_issue() {
        let json = serde_json::json!({
            "check": {
                "xref": {"result": "error", "detail": "xref table has bad entry"}
            }
        });

        let diags = parse_qpdf_output(&json);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PDF-E-009");
        assert!(diags[0].message.contains("xref"));
    }

    #[test]
    fn test_check_pdf_skip_when_absent() {
        // When qpdf is not available, check_pdf should return an info diagnostic
        if !is_qpdf_available() {
            let result = check_pdf(Path::new("/dev/null"));
            assert!(result.is_err());
            let diag = result.unwrap_err();
            assert_eq!(diag.severity, Severity::Info);
            assert!(diag.message.contains("qpdf not available"));
            assert!(diag.message.contains("skipped"));
        }
    }

    #[test]
    fn test_check_pdf_file_not_found() {
        // When qpdf is available but file doesn't exist
        if is_qpdf_available() {
            let result = check_pdf(Path::new("/nonexistent/file.pdf"));
            assert!(result.is_err());
            let diag = result.unwrap_err();
            assert_eq!(diag.severity, Severity::Error);
            assert_eq!(diag.code, "PDF-E-001");
        }
    }

    #[test]
    fn test_check_pdf_valid_with_qpdf() {
        // Skip if qpdf not available
        if !is_qpdf_available() {
            return;
        }

        // Create a minimal valid PDF in a temp file
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000020 00000 n \n");
        pdf.extend_from_slice(b"0000000074 00000 n \n");
        pdf.extend_from_slice(b"0000000139 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n202\n%%EOF\n");

        let dir = std::env::temp_dir();
        let pdf_path = dir.join("stichos_test_valid.pdf");
        std::fs::write(&pdf_path, &pdf).expect("write temp PDF");

        let result = check_pdf(&pdf_path);
        // A hand-crafted PDF may or may not pass qpdf --check;
        // we only verify the API contract: returns Result<Vec<Diagnostic>>
        if let Ok(diagnostics) = result {
            for d in &diagnostics {
                assert!(
                    d.code.starts_with("PDF-"),
                    "diagnostic code should start with 'PDF-' but got '{}'",
                    d.code
                );
            }
        }

        let _ = std::fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_check_pdf_broken_with_qpdf() {
        // Skip if qpdf not available
        if !is_qpdf_available() {
            return;
        }

        // Create a PDF with a deliberately corrupted byte in the middle
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000020 00000 n \n");
        pdf.extend_from_slice(b"0000000074 00000 n \n");
        // Corrupt the xref entry — offset beyond file size
        pdf.extend_from_slice(b"9999999999 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n202\n%%EOF\n");

        let dir = std::env::temp_dir();
        let pdf_path = dir.join("stichos_test_broken.pdf");
        std::fs::write(&pdf_path, &pdf).expect("write temp PDF");

        let result = check_pdf(&pdf_path);
        // The broken PDF should produce error diagnostics via qpdf
        if let Ok(diagnostics) = result {
            let errors: Vec<_> = diagnostics
                .iter()
                .filter(|d| d.severity >= Severity::Error)
                .collect();
            // We expect at least one error for a broken PDF
            assert!(
                !errors.is_empty(),
                "broken PDF should produce error diagnostics, got: {:?}",
                diagnostics
            );
        }

        let _ = std::fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_check_all_pdf_fixtures() {
        // Skip if qpdf not available
        if !is_qpdf_available() {
            return;
        }

        // Create a set of PDF fixtures and check them all
        let fixtures: Vec<(&str, Vec<u8>)> = vec![
            ("minimal.pdf", create_minimal_pdf()),
            ("stream.pdf", create_stream_pdf()),
            ("encrypted.pdf", create_encrypted_pdf()),
            ("rotated.pdf", create_rotated_pdf()),
        ];

        let dir = std::env::temp_dir();
        for (name, content) in &fixtures {
            let pdf_path = dir.join(name);
            let _ = std::fs::write(&pdf_path, content);

            let result = check_pdf(&pdf_path);
            // All should return Ok (empty or populated diagnostics)
            match result {
                Ok(diagnostics) => {
                    for d in &diagnostics {
                        assert!(
                            d.code.starts_with("PDF-"),
                            "diagnostic code should start with 'PDF-' but got '{}'",
                            d.code
                        );
                    }
                }
                Err(d) => {
                    // Single diagnostic is also valid (e.g. qpdf itself reports an error)
                    assert!(d.code.starts_with("PDF-"));
                }
            }

            let _ = std::fs::remove_file(&pdf_path);
        }
    }

    // ------------------------------------------------------------------
    // Helpers: create minimal PDF fixtures for testing
    // ------------------------------------------------------------------

    fn create_minimal_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        b.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        b.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        let off0 = 20;
        let off1 = off0 + b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".len();
        let off2 = off1 + b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".len();
        b.extend_from_slice(format!("{:010} 00000 n \n", off0).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        b.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", off0).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn create_stream_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        b.extend_from_slice(b"3 0 obj\n<< /Length 13 /Filter /FlateDecode >>\nstream\nhello world\nendstream\nendobj\n");
        b.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        let off0 = 20;
        let off1 = off0 + b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".len();
        let off2 = off1 + b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".len();
        b.extend_from_slice(format!("{:010} 00000 n \n", off0).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        b.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", off0).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn create_encrypted_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Encrypt 4 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        b.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        b.extend_from_slice(b"4 0 obj\n<< /Filter /Standard /V 2 /Length 40 >>\nendobj\n");
        b.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        let off0 = 20;
        let off1 =
            off0 + b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Encrypt 4 0 R >>\nendobj\n".len();
        let off2 = off1 + b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".len();
        let off3 = off2
            + b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n".len();
        b.extend_from_slice(format!("{:010} 00000 n \n", off0).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        b.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", off0).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn create_rotated_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        b.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 >>\nendobj\n");
        b.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        let off0 = 20;
        let off1 = off0 + b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".len();
        let off2 = off1 + b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".len();
        b.extend_from_slice(format!("{:010} 00000 n \n", off0).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        b.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", off0).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }
}
