// SPDX-License-Identifier: MIT OR Apache-2.0
use gp_core::diagnostics::ReadError;

/// Probe for veraPDF availability on the host system.
///
/// Returns `Ok(VerapdfInfo)` when both `java` and `verapdf` (or `verapdf.jar`)
/// are found on PATH.  Returns `Err(ReadError)` with an info-level diagnostic
/// when either tool is absent — callers should treat this as "skipped" rather
/// than a failure.
///
/// Diagnostic codes:
/// - `LIN-I-003` — PDF/A validation skipped (tool unavailable)
///
/// ## Host facts
/// `verapdf` is absent on the CI host by design; CI installs it when needed.
pub fn probe_verapdf() -> Result<VerapdfInfo, ReadError> {
    // Check for java
    let java_status = std::process::Command::new("java")
        .args(["-version"])
        .output();

    if java_status.is_err() {
        return Err(ReadError::LinterError {
            code: "LIN-I-003",
            detail: "java not available on PATH".to_string(),
        });
    }

    let java_stderr = String::from_utf8_lossy(&java_status.as_ref().unwrap().stderr);
    let java_version = extract_java_version(&java_stderr);

    // Check for verapdf binary or verapdf.jar
    let verapdf_available = std::process::Command::new("verapdf")
        .args(["-h"])
        .output()
        .is_ok()
        || find_verapdf_jar().is_some();

    if !verapdf_available {
        return Err(ReadError::LinterError {
            code: "LIN-I-003",
            detail: format!(
                "verapdf not available on PATH (java {} found)",
                java_version
            ),
        });
    }

    // Try to get veraPDF version
    let version = std::process::Command::new("verapdf")
        .args(["--version"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().to_string().into())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(VerapdfInfo {
        version,
        java_version,
    })
}

/// Check if a PDF has tagged structure.
///
/// Uses poppler's `pdftotext -struct` or pdfinfo as fallback to detect
/// whether the PDF contains a tagged structure tree.  Returns
/// `TaggedResult` with the findings.
///
/// If no tools are available, returns an info diagnostic (LIN-I-003).
pub fn check_tagged(pdf_path: &std::path::Path) -> Result<TaggedResult, ReadError> {
    // Try pdftotext -struct to extract structure tags
    if let Ok(output) = std::process::Command::new("pdftotext")
        .args(["-struct", "-", "-"])
        .current_dir(pdf_path.parent().unwrap_or(std::path::Path::new(".")))
        .output()
    {
        if output.status.success() {
            let struct_content = String::from_utf8_lossy(&output.stdout);
            return Ok(count_tags_in_struct(&struct_content));
        }
    }

    // Fallback: try pdfinfo for tagged indicator
    if let Ok(output) = std::process::Command::new("pdfinfo")
        .args([pdf_path.to_str().unwrap()])
        .output()
    {
        if output.status.success() {
            let info = String::from_utf8_lossy(&output.stdout);
            if info.contains("Tagged:") {
                let is_tagged = info
                    .lines()
                    .find(|l| l.contains("Tagged:"))
                    .map(|l| l.contains("Yes"))
                    .unwrap_or(false);
                return Ok(TaggedResult {
                    is_tagged,
                    role_count: 0,
                    lang_attribute_count: 0,
                });
            }
        }
    }

    // If veraPDF is available, defer to it
    match probe_verapdf() {
        Ok(info) => {
            // veraPDF is available but we already ran check_tagged via external tools
            // Return default result — the veraPDF runner handles full a11y checks
            Err(ReadError::LinterError {
                code: "LIN-I-003",
                detail: format!(
                    "Tagged structure check inconclusive; veraPDF {} available for full a11y validation",
                    info.version
                ),
            })
        }
        Err(_) => Err(ReadError::LinterError {
            code: "LIN-I-003",
            detail: "No tools available for tagged structure detection (pdftotext/pdfinfo)"
                .to_string(),
        }),
    }
}

/// Run veraPDF on a PDF file and return accessibility checks.
///
/// If veraPDF is unavailable, returns empty results with info diagnostic.
/// Otherwise spawns: `java -jar verapdf.pdf --profile ...` and parses the output.
pub fn check_pdfa(pdf_path: &std::path::Path) -> Result<Vec<A11yCheck>, ReadError> {
    // Probe first — if veraPDF is absent, return info diagnostic
    let verapdf_info = probe_verapdf()?;

    if !pdf_path.exists() {
        return Err(ReadError::LinterError {
            code: "PDF-E-001",
            detail: format!("PDF file not found: {}", pdf_path.display()),
        });
    }

    // Try running veraPDF via the `verapdf` binary first
    let output = std::process::Command::new("verapdf")
        .args([
            "--profile",
            "verapdf/pdfa",
            "--format",
            "minimal",
            pdf_path.to_str().unwrap(),
        ])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            Ok(parse_verapdf_output(&stdout, &stderr, &verapdf_info))
        }
        Err(_) => {
            // Try via `java -jar verapdf.jar`
            if let Some(jar_path) = find_verapdf_jar() {
                match std::process::Command::new("java")
                    .args([
                        "-jar",
                        &jar_path,
                        "--profile",
                        "verapdf/pdfa",
                        pdf_path.to_str().unwrap(),
                    ])
                    .output()
                {
                    Ok(o) => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        Ok(parse_verapdf_output(&stdout, &stderr, &verapdf_info))
                    }
                    Err(e) => Err(ReadError::LinterError {
                        code: "LIN-I-003",
                        detail: format!("verapdf.jar execution failed: {}", e),
                    }),
                }
            } else {
                Err(ReadError::LinterError {
                    code: "LIN-I-003",
                    detail: "verapdf binary and verapdf.jar both unavailable".to_string(),
                })
            }
        }
    }
}

/// Check if a PDF has tagged structure using veraPDF if available.
pub fn check_tagged_with_verapdf(pdf_path: &std::path::Path) -> Result<TaggedResult, ReadError> {
    // First try lightweight tools
    if let Ok(result) = check_tagged(pdf_path) {
        if result.role_count > 0 || result.is_tagged {
            return Ok(result);
        }
    } // Fall through to veraPDF

    // veraPDF is available for deeper check
    let verapdf_info = probe_verapdf()?;

    // Run a quick tagged-structure check via veraPDF
    let output = std::process::Command::new("verapdf")
        .args([
            "--profile",
            "verapdf/pdfa",
            "--format",
            "minimal",
            pdf_path.to_str().unwrap(),
        ])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let checks = parse_verapdf_output(&stdout, &stderr, &verapdf_info);
            let is_tagged = !checks.iter().any(|c| c.code == "LIN-W-008");
            Ok(TaggedResult {
                is_tagged,
                role_count: checks.iter().filter(|c| c.code == "LIN-W-008").count(),
                lang_attribute_count: 0,
            })
        }
        Err(e) => Err(ReadError::LinterError {
            code: "LIN-I-003",
            detail: format!("verapdf execution failed: {}", e),
        }),
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Information about the veraPDF installation found on the host.
#[derive(Debug, Clone)]
pub struct VerapdfInfo {
    /// veraPDF version string.
    pub version: String,
    /// Java version string detected.
    pub java_version: String,
}

/// A single accessibility check result from veraPDF or structure analysis.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct A11yCheck {
    /// Diagnostic code (e.g. "LIN-E-007", "LIN-W-008").
    pub code: String,
    /// Severity of this check result.
    pub severity: String,
    /// Human-readable message.
    pub message: String,
    /// Page number this check applies to, if known.
    pub page: Option<usize>,
}

/// Result of a tagged-structure check on a PDF.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaggedResult {
    /// Whether the PDF has a tagged structure tree.
    pub is_tagged: bool,
    /// Count of structural roles found in the tree.
    pub role_count: usize,
    /// Count of language attribute occurrences.
    pub lang_attribute_count: usize,
}

/// Accessibility section of the lint manifest.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct A11ySection {
    /// Overall PDF/A status: "passed", "failed", or "skipped".
    pub pdfa_status: String,
    /// Detected PDF/A profile (e.g. "PDF/A-2b"), if available.
    pub pdfa_profile: Option<String>,
    /// Whether the PDF has tagged structure, if determined.
    pub is_tagged: Option<bool>,
    /// All accessibility checks performed.
    pub checks: Vec<A11yCheck>,
}

impl A11ySection {
    /// Create a new a11y section with the given checks.
    pub fn new(checks: Vec<A11yCheck>) -> Self {
        let pdfa_status = if checks.is_empty() {
            "skipped".to_string()
        } else if checks.iter().any(|c| c.severity == "error") {
            "failed".to_string()
        } else if checks.iter().any(|c| c.severity == "info") {
            "skipped".to_string()
        } else {
            "passed".to_string()
        };

        let pdfa_profile = checks
            .iter()
            .find(|c| c.code == "PDF/A-1" || c.code == "PDF/A-2" || c.code == "PDF/A-3")
            .map(|c| c.code.clone());

        let has_error = checks.iter().any(|c| c.severity == "error");
        let has_info_only =
            !has_error && !checks.is_empty() && checks.iter().all(|c| c.severity == "info");
        let is_tagged = if has_info_only {
            // Only info diagnostics (e.g. skipped) — tagged is unknown
            None
        } else if checks.iter().any(|c| c.code == "LIN-W-008") {
            Some(false)
        } else if !checks.is_empty() {
            Some(true)
        } else {
            None
        };

        Self {
            pdfa_status,
            pdfa_profile,
            is_tagged,
            checks,
        }
    }

    /// Create an empty (skipped) a11y section.
    pub fn skipped() -> Self {
        Self {
            pdfa_status: "skipped".to_string(),
            pdfa_profile: None,
            is_tagged: None,
            checks: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract the Java version from `java -version` stderr output.
fn extract_java_version(stderr: &str) -> String {
    stderr
        .lines()
        .next()
        .and_then(|l| l.find("version \"").map(|i| &l[i + 9..]))
        .and_then(|l| l.find('"').map(|j| &l[..j]))
        .unwrap_or("unknown")
        .to_string()
}

/// Find verapdf.jar on the system.
fn find_verapdf_jar() -> Option<String> {
    // Common locations
    let candidates = [
        "/usr/share/java/verapdf.jar",
        "/usr/share/verapdf/verapdf.jar",
        "/opt/verapdf/verapdf.jar",
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    // Try locating via `which` on PATH
    if let Ok(output) = std::process::Command::new("find")
        .args([
            "/usr",
            "-name",
            "verapdf.jar",
            "-type",
            "f",
            "-maxdepth",
            "4",
        ])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.ends_with(".jar") {
                return Some(line.to_string());
            }
        }
    }
    None
}

/// Count structural roles in pdftotext -struct output.
fn count_tags_in_struct(content: &str) -> TaggedResult {
    let role_count = content
        .lines()
        .filter(|l| l.trim().starts_with('<') && !l.trim().starts_with("</"))
        .count();

    let lang_count = content.lines().filter(|l| l.contains("lang=")).count();

    TaggedResult {
        is_tagged: role_count > 0,
        role_count,
        lang_attribute_count: lang_count,
    }
}

/// Parse veraPDF output (minimal JSON or XML) into A11yCheck results.
fn parse_verapdf_output(stdout: &str, stderr: &str, verapdf: &VerapdfInfo) -> Vec<A11yCheck> {
    let mut checks = Vec::new();

    // Try parsing as JSON first (minimal format)
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(stdout) {
        if let Some(fails) = json.get("fails").and_then(|f| f.as_array()) {
            for failure in fails {
                let code = failure
                    .get("ruleId")
                    .or_else(|| failure.get("rule"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let message = failure
                    .get("message")
                    .or_else(|| failure.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("validation failure")
                    .to_string();
                let page = failure
                    .get("page")
                    .and_then(|v| v.as_u64())
                    .map(|p| p as usize);

                checks.push(A11yCheck {
                    code,
                    severity: "error".to_string(),
                    message,
                    page,
                });
            }
        }

        // Check warnings
        if let Some(warnings) = json.get("warnings").and_then(|w| w.as_array()) {
            for warning in warnings {
                let code = warning
                    .get("ruleId")
                    .or_else(|| warning.get("rule"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let message = warning
                    .get("message")
                    .or_else(|| warning.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("validation warning")
                    .to_string();
                let page = warning
                    .get("page")
                    .and_then(|v| v.as_u64())
                    .map(|p| p as usize);

                checks.push(A11yCheck {
                    code,
                    severity: "warning".to_string(),
                    message,
                    page,
                });
            }
        }

        if json.get("passes").and_then(|p| p.as_bool()) == Some(true) && checks.is_empty() {
            checks.push(A11yCheck {
                code: "LIN-E-007".to_string(),
                severity: "info".to_string(),
                message: format!("PDF/A validation passed (veraPDF {})", verapdf.version),
                page: None,
            });
        }

        return checks;
    }

    // Fallback: parse line-based output
    for line in stdout.lines().chain(stderr.lines()) {
        let line_lower = line.to_lowercase();
        if line_lower.contains("fail") || line_lower.contains("error") {
            checks.push(A11yCheck {
                code: "LIN-E-007".to_string(),
                severity: "error".to_string(),
                message: line.to_string(),
                page: None,
            });
        } else if line_lower.contains("warn") {
            checks.push(A11yCheck {
                code: "LIN-W-007".to_string(),
                severity: "warning".to_string(),
                message: line.to_string(),
                page: None,
            });
        }
    }

    checks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verapdf_probe_absent_on_host() {
        // On the CI host, veraPDF should be absent.
        // This test documents the expected behaviour regardless of host config.
        let result = probe_verapdf();
        // May return Ok on hosts with veraPDF installed, or Err on hosts without.
        // Both are acceptable — we just verify the function doesn't panic.
        match result {
            Ok(info) => {
                assert!(!info.version.is_empty(), "version should be set");
            }
            Err(e) => {
                // Should be LIN-I-003 (skipped) for absent tools
                assert_eq!(e.code(), "LIN-I-003");
            }
        }
    }

    #[test]
    fn test_tagged_check_returns_result_or_skip() {
        // Create a minimal PDF for testing
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
        let pdf_path = dir.join("stichos_tagged_test.pdf");
        std::fs::write(&pdf_path, &pdf).expect("write temp PDF");

        let result = check_tagged(&pdf_path);
        // Should return Ok with TaggedResult, or Err with LIN-I-003
        match result {
            Ok(tagged) => {
                // A minimal untagged PDF should return is_tagged = false
                assert!(!tagged.is_tagged, "Minimal PDF should not be tagged");
            }
            Err(e) => {
                // Or we get a skip diagnostic
                assert_eq!(e.code(), "LIN-I-003");
            }
        }

        let _ = std::fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_a11y_section_empty_when_no_checks() {
        let section = A11ySection::new(Vec::new());
        assert_eq!(section.pdfa_status, "skipped");
        assert!(section.pdfa_profile.is_none());
        assert!(section.is_tagged.is_none());
        assert!(section.checks.is_empty());
    }

    #[test]
    fn test_a11y_section_skipped() {
        let section = A11ySection::skipped();
        assert_eq!(section.pdfa_status, "skipped");
        assert!(section.pdfa_profile.is_none());
        assert!(section.is_tagged.is_none());
    }

    #[test]
    fn test_a11y_section_with_failures() {
        let checks = vec![A11yCheck {
            code: "LIN-E-007".to_string(),
            severity: "error".to_string(),
            message: "PDF/A validation failure".to_string(),
            page: None,
        }];
        let section = A11ySection::new(checks);
        assert_eq!(section.pdfa_status, "failed");
    }

    #[test]
    fn test_a11y_section_serde_roundtrip() {
        let section = A11ySection::skipped();
        let json = serde_json::to_string(&section).expect("serialize a11y section");
        let back: A11ySection = serde_json::from_str(&json).expect("deserialize a11y section");
        assert_eq!(section.pdfa_status, back.pdfa_status);
        assert_eq!(section.pdfa_profile, back.pdfa_profile);
        assert_eq!(section.is_tagged, back.is_tagged);
    }

    #[test]
    fn test_pdfa_skipped_without_verapdf() {
        // If veraPDF is not available, check_pdfa should return Err with LIN-I-003
        // This is the expected path on the CI host
        if !is_verapdf_available() {
            let result = check_pdfa(std::path::Path::new("/dev/null"));
            assert!(
                result.is_err(),
                "check_pdfa should fail when veraPDF absent"
            );
            let err = result.unwrap_err();
            assert_eq!(
                err.code(),
                "LIN-I-003",
                "should report LIN-I-003 when absent"
            );
        }
    }

    #[test]
    fn test_parse_verapdf_output_json() {
        let verapdf = VerapdfInfo {
            version: "1.6.5".to_string(),
            java_version: "11.0.12".to_string(),
        };

        let json = serde_json::json!({
            "fails": [
                {
                    "ruleId": "PDF/A-1b",
                    "message": "Missing PDF/A conformance level",
                    "page": 1
                }
            ],
            "passes": false
        });

        let checks = parse_verapdf_output(&json.to_string(), "", &verapdf);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].code, "PDF/A-1b");
        assert_eq!(checks[0].severity, "error");
        assert_eq!(checks[0].page, Some(1));
    }

    #[test]
    fn test_parse_verapdf_output_line_based() {
        let verapdf = VerapdfInfo {
            version: "1.6.5".to_string(),
            java_version: "11.0.12".to_string(),
        };

        let output = "VERAPDF INFO: Starting validation\nVERAPDF ERROR: PDF/A-1b check failed\nVERAPDF WARN: Missing lang attribute\n";
        let checks = parse_verapdf_output(output, "", &verapdf);

        // Should have error + warning
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].severity, "error");
        assert_eq!(checks[1].severity, "warning");
    }

    #[test]
    fn test_parse_verapdf_output_all_pass() {
        let verapdf = VerapdfInfo {
            version: "1.6.5".to_string(),
            java_version: "11.0.12".to_string(),
        };

        let json = serde_json::json!({
            "passes": true,
            "fails": []
        });

        let checks = parse_verapdf_output(&json.to_string(), "", &verapdf);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].severity, "info");
    }

    #[test]
    fn test_extract_java_version() {
        let stderr = r#"java version "11.0.12" 2021-07-20
Java(TM) SE Runtime Environment 18.9 (build 11.0.12+8)
Java HotSpot(TM) 64-Bit Server VM 18.9 (build 11.0.12+8, mixed mode)"#;
        let version = extract_java_version(stderr);
        assert_eq!(version, "11.0.12");
    }

    #[test]
    fn test_extract_java_version_openjdk() {
        let stderr = r#"openjdk version "17.0.2" 2022-01-18
OpenJDK Runtime Environment (build 17.0.2+8)"#;
        let version = extract_java_version(stderr);
        assert_eq!(version, "17.0.2");
    }

    #[test]
    fn test_count_tags_in_struct() {
        let struct_content = r#"<P>Paragraph 1</P>
<L>List item</L>
<Figure>
  <Caption>Figure 1</Caption>
</Figure>
<P lang="fr">Texte en français</P>"#;
        let result = count_tags_in_struct(struct_content);
        assert!(result.is_tagged);
        assert!(result.role_count >= 3);
        assert_eq!(result.lang_attribute_count, 1);
    }

    #[test]
    fn test_empty_struct_is_untagged() {
        let result = count_tags_in_struct("");
        assert!(!result.is_tagged);
        assert_eq!(result.role_count, 0);
    }

    /// Helper to check if veraPDF is available (used by test_pdfa_skipped_without_verapdf).
    fn is_verapdf_available() -> bool {
        probe_verapdf().is_ok()
    }
}
