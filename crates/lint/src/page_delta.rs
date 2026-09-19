// SPDX-License-Identifier: MIT OR Apache-2.0
// lint: page-count delta checking between original and rendered documents
// LIN-E-005 / LIN-E-006 / LIN-W-004: page-count delta codes

use gp_core::diagnostics::{Diagnostic, Severity};

/// The maximum acceptable page-count delta between two versions.
pub const MAX_PAGE_DELTA: i64 = 2;

/// Compare the page counts of two document renderings and report deltas.
///
/// Returns diagnostics for pages that exist in one but not the other,
/// or where the page count differs beyond the threshold.
pub fn check_page_count_delta(original_pages: usize, rendered_pages: usize) -> Vec<Diagnostic> {
    let delta = (rendered_pages as i64) - (original_pages as i64);

    if delta.abs() > MAX_PAGE_DELTA {
        return vec![Diagnostic::new(
            "LIN-E-005",
            Severity::Error,
            format!(
                "Page-count delta {} ({} -> {}) exceeds threshold {}",
                delta, original_pages, rendered_pages, MAX_PAGE_DELTA
            ),
        )];
    }

    if delta != 0 {
        return vec![Diagnostic::new(
            "LIN-W-004",
            Severity::Warning,
            format!(
                "Page-count delta {} ({} -> {})",
                delta, original_pages, rendered_pages
            ),
        )];
    }

    vec![]
}

/// Check for missing pages: pages that exist in original but not rendered.
pub fn check_missing_pages(original_count: usize, rendered_count: usize) -> Vec<Diagnostic> {
    if rendered_count < original_count {
        let missing = original_count - rendered_count;
        return vec![Diagnostic::new(
            "LIN-E-006",
            Severity::Error,
            format!(
                "Missing {} page(s) in render ({} original, {} rendered)",
                missing, original_count, rendered_count
            ),
        )];
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_delta_same_count() {
        let diags = check_page_count_delta(5, 5);
        assert!(
            diags.is_empty(),
            "Same page count should produce no diagnostics, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_delta_within_threshold() {
        // delta = 1, within MAX_PAGE_DELTA (2) -> warning only
        let diags = check_page_count_delta(5, 6);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "LIN-W-004");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("Page-count delta 1"));
    }

    #[test]
    fn test_delta_exceeds_threshold() {
        // delta = 3, exceeds MAX_PAGE_DELTA (2) -> error
        let diags = check_page_count_delta(5, 8);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "LIN-E-005");
        assert_eq!(diags[0].severity, Severity::Error);
        assert!(diags[0].message.contains("exceeds threshold"));
    }

    #[test]
    fn test_missing_pages() {
        // rendered < original -> error
        let diags = check_missing_pages(10, 7);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "LIN-E-006");
        assert_eq!(diags[0].severity, Severity::Error);
        assert!(diags[0].message.contains("Missing 3 page(s)"));
    }

    #[test]
    fn test_extra_pages() {
        // rendered > original -> warning only (not missing)
        let diags = check_missing_pages(5, 8);
        assert!(
            diags.is_empty(),
            "Extra pages should not trigger missing pages error, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_delta_negative_exceeds_threshold() {
        // delta = -4, exceeds MAX_PAGE_DELTA (2) -> error
        let diags = check_page_count_delta(10, 5);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "LIN-E-005");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_delta_negative_within_threshold() {
        // delta = -1, within MAX_PAGE_DELTA (2) -> warning
        let diags = check_page_count_delta(10, 9);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "LIN-W-004");
    }

    #[test]
    fn test_zero_pages() {
        let diags = check_page_count_delta(0, 0);
        assert!(diags.is_empty(), "Zero pages should produce no diagnostics");
    }

    #[test]
    fn test_serde_roundtrip_contrast_report() {
        // ContrastReport serde roundtrip test (defined in contrast module)
        use gp_core::diagnostics::DiagnosticRegistry;
        let reg = DiagnosticRegistry::instance();
        assert!(
            reg.lookup("LIN-E-005").is_some(),
            "LIN-E-005 must be registered"
        );
        assert!(
            reg.lookup("LIN-E-006").is_some(),
            "LIN-E-006 must be registered"
        );
        assert!(
            reg.lookup("LIN-W-004").is_some(),
            "LIN-W-004 must be registered"
        );
    }
}
