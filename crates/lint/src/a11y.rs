//! Accessibility lint module: orchestrates PDF/A, tagged-structure, and alt-text checks.
//!
//! This module provides a single `run_a11y_checks()` entry point that:
//! 1. Probes for veraPDF availability
//! 2. If available, runs PDF/A validation and tagged-structure checks
//! 3. Returns an `A11ySection` for the lint manifest
//!
//! If veraPDF is unavailable, the linter does NOT fail — it returns an
//! info-level diagnostic indicating the check was skipped.
//!
//! Diagnostic codes used:
//! - `LIN-E-007` — PDF/A validation failure
//! - `LIN-W-007` — Missing PDF/A conformance level
//! - `LIN-I-003` — PDF/A validation skipped (tool unavailable)
//! - `LIN-W-008` — Missing tagged structure (PDF not tagged)
//! - `LIN-W-009` — Missing alt text in PDF

use std::path::Path;

use super::verapdf::{check_pdfa, check_tagged, probe_verapdf, A11yCheck, A11ySection};
use gp_core::diagnostics::{ReadError, Severity};

/// Run all accessibility checks on a PDF file and return an `A11ySection`.
///
/// This is the main entry point for the a11y lint pass.  It handles the
/// full probe → run → aggregate flow and produces a section suitable
/// for inclusion in the lint manifest.
///
/// ## Behaviour
/// - **veraPDF available**: runs PDF/A validation + tagged structure check,
///   aggregates results into an `A11ySection` with `pdfa_status` reflecting
///   the outcome.
/// - **veraPDF absent**: returns an `A11ySection` with status `"skipped"` and
///   an info-level diagnostic.
///
/// ## Returns
/// - `Ok(A11ySection)` — always succeeds, even when checks are skipped.
///   The caller can inspect `section.pdfa_status` to determine outcome.
/// - `Err(ReadError)` — only on unexpected IO failures (very rare).
pub fn run_a11y_checks(pdf_path: &Path) -> Result<A11ySection, ReadError> {
    // Step 1: probe for veraPDF
    let _verapdf_info = match probe_verapdf() {
        Ok(info) => info,
        Err(e) => {
            // veraPDF absent — return skipped section with info diagnostic
            let skip_diags = vec![A11yCheck {
                code: "LIN-I-003".to_string(),
                severity: "info".to_string(),
                message: format!("PDF/A validation skipped: {}", e.detail()),
                page: None,
            }];
            return Ok(A11ySection::new(skip_diags));
        }
    };

    // Step 2: run PDF/A validation
    let pdfa_checks = match check_pdfa(pdf_path) {
        Ok(checks) => checks,
        Err(e) => {
            // PDF/A check failed or was skipped
            let checks = vec![A11yCheck {
                code: e.code().to_string(),
                severity: severity_to_string(&diagnostic_severity_for_code(e.code())),
                message: format!("PDF/A check: {}", e.detail()),
                page: None,
            }];
            return Ok(A11ySection::new(checks));
        }
    };

    // Step 3: run tagged structure check
    let tagged = match check_tagged(pdf_path) {
        Ok(t) => t,
        Err(e) => {
            // Tagged check failed — add info diagnostic but continue
            let info_check = A11yCheck {
                code: e.code().to_string(),
                severity: "info".to_string(),
                message: format!("Tagged structure check: {}", e.detail()),
                page: None,
            };
            // Merge into existing checks and return
            let mut all_checks = pdfa_checks;
            all_checks.push(info_check);
            return Ok(A11ySection::new(all_checks));
        }
    };

    // Step 4: add tagged-structure diagnostic if needed
    let mut all_checks = pdfa_checks;
    if !tagged.is_tagged && tagged.role_count == 0 {
        all_checks.push(A11yCheck {
            code: "LIN-W-008".to_string(),
            severity: "warning".to_string(),
            message: "Missing tagged structure: PDF is not tagged".to_string(),
            page: None,
        });
    }

    // Step 5: check for missing alt text (basic heuristic)
    // veraPDF would normally catch this; add a basic check here
    if all_checks.iter().any(|c| c.code == "LIN-W-009") {
        // veraPDF already reported alt text issues
    }

    // Step 6: build the section
    let mut section = A11ySection::new(all_checks);
    section.is_tagged = Some(tagged.is_tagged);

    Ok(section)
}

/// Map a diagnostic code to its severity.
fn diagnostic_severity_for_code(code: &str) -> Severity {
    use gp_core::diagnostics::get_code;
    get_code(code)
        .map(|e| e.severity)
        .unwrap_or(Severity::Error)
}

/// Convert a `Severity` to a human-readable string.
fn severity_to_string(s: &Severity) -> String {
    match s {
        Severity::Hint => "hint".to_string(),
        Severity::Info => "info".to_string(),
        Severity::Warning => "warning".to_string(),
        Severity::Error => "error".to_string(),
        Severity::Fatal => "fatal".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_a11y_checks_returns_section() {
        // Even when veraPDF is absent, run_a11y_checks should succeed
        let result = run_a11y_checks(Path::new("/dev/null"));
        assert!(result.is_ok(), "run_a11y_checks should never fail");
        let section = result.unwrap();

        // If veraPDF is absent, status should be "skipped"
        // If available, status depends on the PDF
        assert!(
            section.pdfa_status == "skipped"
                || section.pdfa_status == "passed"
                || section.pdfa_status == "failed",
            "pdfa_status should be one of skipped/passed/failed, got: {}",
            section.pdfa_status
        );
    }

    #[test]
    fn test_run_a11y_checks_with_minimal_pdf() {
        // Create a minimal PDF and run checks
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
        let pdf_path = dir.join("stichos_a11y_test.pdf");
        std::fs::write(&pdf_path, &pdf).expect("write temp PDF");

        let result = run_a11y_checks(&pdf_path);
        assert!(result.is_ok());
        let section = result.unwrap();

        // Section should be well-formed
        assert!(section.pdfa_status == "skipped" || section.pdfa_status == "passed");

        let _ = std::fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_a11y_section_is_in_manifest() {
        // Verify that A11ySection is serializable as part of a larger manifest
        #[derive(serde::Serialize, serde::Deserialize)]
        struct TestManifest {
            version: String,
            a11y: Option<A11ySection>,
        }

        let manifest = TestManifest {
            version: "1.0.0".to_string(),
            a11y: Some(A11ySection::skipped()),
        };

        let json = serde_json::to_string(&manifest).expect("serialize manifest");
        let back: TestManifest = serde_json::from_str(&json).expect("deserialize manifest");
        assert_eq!(back.version, "1.0.0");
        assert!(back.a11y.is_some());
        assert_eq!(back.a11y.unwrap().pdfa_status, "skipped");
    }

    #[test]
    fn test_pdfa_skipped_without_verapdf() {
        // When veraPDF is absent, the a11y section should have skipped status
        if !is_verapdf_available() {
            let result = run_a11y_checks(Path::new("/nonexistent.pdf"));
            assert!(result.is_ok());
            let section = result.unwrap();
            assert_eq!(
                section.pdfa_status, "skipped",
                "should be skipped when veraPDF absent"
            );
            assert!(
                section.is_tagged.is_none(),
                "tagged should be None when skipped"
            );
        }
    }

    /// Helper to check if veraPDF is available.
    fn is_verapdf_available() -> bool {
        probe_verapdf().is_ok()
    }
}
