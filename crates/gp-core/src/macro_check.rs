// SPDX-License-Identifier: MIT OR Apache-2.0
//! Macro-format detection and policy.
//!
//! Macro-bearing formats (xlsm, docm, pptm) are detected but NEVER executed.
//! They are flagged as unsupported in the fidelity budget.
//!
//! Detection is assert-by-construction: the code path for macro execution
//! does not exist. Only detection + flagging behavior is implemented.

use crate::diagnostics::Diagnostic;

/// Macro-bearing file extensions.
///
/// These formats may contain VBA macros or other executable content.
/// The stichos renderer detects them but never executes any code.
const MACRO_EXTENSIONS: &[&str] = &[
    "xlsm", // Excel Macro-Enabled Workbook
    "docm", // Word Macro-Enabled Document
    "pptm", // PowerPoint Macro-Enabled Presentation
    "xlsb", // Excel Binary Workbook
    "docb", // Word Binary Document
    "pptb", // PowerPoint Binary Presentation
];

/// Check if a document format is macro-bearing.
///
/// Returns `true` for macro-enabled formats (xlsm, docm, pptm, xlsb, docb, pptb).
/// Comparison is case-insensitive.
pub fn is_macro_format(ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    MACRO_EXTENSIONS.iter().any(|&e| e == ext_lower)
}

/// Check if the given bytes contain a macro indicator in the OPC.
///
/// For macro-bearing formats, returns a diagnostic warning.
/// For non-macro formats, returns `None`.
///
/// The bytes parameter is accepted for future OPC-level macro detection
/// (e.g., scanning for `vbaProject` parts), but currently only the
/// extension is checked.
pub fn bytes_contain_macros(_bytes: &[u8], ext: &str) -> Option<Diagnostic> {
    if !is_macro_format(ext) {
        return None;
    }

    Some(Diagnostic::new(
        "SEC-E-001",
        crate::diagnostics::Severity::Warning,
        format!(
            "macro-bearing format detected ({}) — content analyzed but NOT executed",
            ext
        ),
    ))
}

/// Register the macro policy as a dropped feature in the fidelity budget.
///
/// Returns a human-readable string describing the dropped macro capability.
pub fn add_macro_dropped_feature(content: &str) -> String {
    format!(
        "macro-bearing format ({}) — features may be dropped",
        content
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_macro_format tests ---

    #[test]
    fn test_is_macro_format_xlsm() {
        assert!(is_macro_format("xlsm"));
    }

    #[test]
    fn test_is_macro_format_docm() {
        assert!(is_macro_format("docm"));
    }

    #[test]
    fn test_is_macro_format_pptm() {
        assert!(is_macro_format("pptm"));
    }

    #[test]
    fn test_is_macro_format_xlsb() {
        assert!(is_macro_format("xlsb"));
    }

    #[test]
    fn test_is_macro_format_docb() {
        assert!(is_macro_format("docb"));
    }

    #[test]
    fn test_is_macro_format_pptb() {
        assert!(is_macro_format("pptb"));
    }

    #[test]
    fn test_is_macro_format_xlsx() {
        assert!(!is_macro_format("xlsx"));
    }

    #[test]
    fn test_is_macro_format_docx() {
        assert!(!is_macro_format("docx"));
    }

    #[test]
    fn test_is_macro_format_pptx() {
        assert!(!is_macro_format("pptx"));
    }

    #[test]
    fn test_is_macro_format_pdf() {
        assert!(!is_macro_format("pdf"));
    }

    #[test]
    fn test_is_macro_format_odt() {
        assert!(!is_macro_format("odt"));
    }

    #[test]
    fn test_is_macro_format_ods() {
        assert!(!is_macro_format("ods"));
    }

    #[test]
    fn test_is_macro_format_odp() {
        assert!(!is_macro_format("odp"));
    }

    // --- Case insensitivity ---

    #[test]
    fn test_case_insensitive_uppercase() {
        assert!(is_macro_format("XLSM"));
        assert!(is_macro_format("DOCM"));
        assert!(is_macro_format("PPTM"));
    }

    #[test]
    fn test_case_insensitive_mixed() {
        assert!(is_macro_format("Xlsm"));
        assert!(is_macro_format("DoCm"));
        assert!(is_macro_format("pPtM"));
    }

    #[test]
    fn test_case_insensitive_scrambled() {
        assert!(is_macro_format("xlsM"));
        assert!(is_macro_format("Docb"));
        assert!(is_macro_format("PPTB"));
    }

    // --- bytes_contain_macros tests ---

    #[test]
    fn test_bytes_contain_macros_xlsm() {
        let diag = bytes_contain_macros(b"fake data", "xlsm");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.code, "SEC-E-001");
        assert_eq!(
            d.message,
            "macro-bearing format detected (xlsm) — content analyzed but NOT executed"
        );
    }

    #[test]
    fn test_bytes_contain_macros_docm() {
        let diag = bytes_contain_macros(b"fake data", "docm");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.code, "SEC-E-001");
    }

    #[test]
    fn test_bytes_contain_macros_xlsx() {
        let diag = bytes_contain_macros(b"fake data", "xlsx");
        assert!(diag.is_none());
    }

    #[test]
    fn test_bytes_contain_macros_docx() {
        let diag = bytes_contain_macros(b"fake data", "docx");
        assert!(diag.is_none());
    }

    #[test]
    fn test_bytes_contain_macros_pptm() {
        let diag = bytes_contain_macros(b"fake data", "pptm");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.code, "SEC-E-001");
    }

    #[test]
    fn test_bytes_contain_macros_empty_bytes() {
        let diag = bytes_contain_macros(b"", "xlsm");
        assert!(diag.is_some());
    }

    #[test]
    fn test_bytes_contain_macros_case_insensitive() {
        let diag = bytes_contain_macros(b"data", "XLSM");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.code, "SEC-E-001");
    }

    // --- add_macro_dropped_feature tests ---

    #[test]
    fn test_add_macro_dropped_feature() {
        let result = add_macro_dropped_feature("xlsm");
        assert_eq!(
            result,
            "macro-bearing format (xlsm) — features may be dropped"
        );
    }

    #[test]
    fn test_add_macro_dropped_feature_docm() {
        let result = add_macro_dropped_feature("docm");
        assert_eq!(
            result,
            "macro-bearing format (docm) — features may be dropped"
        );
    }

    #[test]
    fn test_add_macro_dropped_feature_pptm() {
        let result = add_macro_dropped_feature("pptm");
        assert_eq!(
            result,
            "macro-bearing format (pptm) — features may be dropped"
        );
    }

    // --- Integration: diagnostic code registration ---

    #[test]
    fn test_sec_e_001_registered() {
        let d = bytes_contain_macros(b"", "xlsm").unwrap();
        let entry = d.code_enum().expect("SEC-E-001 should be in registry");
        assert_eq!(entry.code, "SEC-E-001");
        assert_eq!(entry.severity, crate::diagnostics::Severity::Warning);
        assert_eq!(entry.category, "security");
    }

    // --- All macro formats produce diagnostics ---

    #[test]
    fn test_all_macro_formats_flagged() {
        for ext in &["xlsm", "docm", "pptm", "xlsb", "docb", "pptb"] {
            let diag = bytes_contain_macros(b"", ext);
            assert!(diag.is_some(), "{} should be flagged as macro-bearing", ext);
            assert_eq!(diag.unwrap().code, "SEC-E-001");
        }
    }

    // --- Non-macro formats produce no diagnostics ---

    #[test]
    fn test_non_macro_formats_no_diagnostic() {
        for ext in &[
            "xlsx", "docx", "pptx", "pdf", "odt", "ods", "odp", "txt", "csv", "html",
        ] {
            let diag = bytes_contain_macros(b"", ext);
            assert!(
                diag.is_none(),
                "{} should NOT be flagged as macro-bearing",
                ext
            );
        }
    }
}
