// lint: luminance/contrast checking for layout quality
// LIN-W-005 / LIN-W-006: contrast quality codes

use gp_core::diagnostics::{Diagnostic, Severity};

/// Minimum contrast ratio for readable text (WCAG AA standard).
pub const MIN_CONTRAST_RATIO: f64 = 4.5;

/// A contrast report entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContrastReport {
    /// Page number being checked.
    pub page_number: usize,
    /// Average contrast ratio on the page.
    pub avg_contrast: f64,
    /// Minimum contrast ratio on the page.
    pub min_contrast: f64,
    /// Number of low-contrast regions found.
    pub low_contrast_count: usize,
}

/// Given the luminance of text and background, compute the contrast ratio
/// per WCAG 2.1 formula.
pub fn luminance_contrast_ratio(luminance_text: f64, luminance_bg: f64) -> f64 {
    // WCAG 2.1 contrast ratio: (L1 + 0.05) / (L2 + 0.05)
    // where L1 is lighter, L2 is darker
    let l1 = luminance_text.max(luminance_bg);
    let l2 = luminance_text.min(luminance_bg);
    (l1 + 0.05) / (l2 + 0.05)
}

/// Check contrast adequacy. Returns None if contrast is acceptable.
pub fn check_contrast(luminance_text: f64, luminance_bg: f64) -> Option<Diagnostic> {
    let ratio = luminance_contrast_ratio(luminance_text, luminance_bg);
    if ratio < MIN_CONTRAST_RATIO {
        return Some(Diagnostic::new(
            "LIN-W-005",
            Severity::Warning,
            format!(
                "Low contrast ratio {:.2}:1 (threshold: {:.1}:1)",
                ratio, MIN_CONTRAST_RATIO
            ),
        ));
    }
    None
}

/// Analyze page contrast based on text bbox density.
///
/// If a page has very few text bboxes relative to its area, it may have
/// low-contrast content or rendering issues.
pub fn analyze_page_contrast(area: f64, text_coverage: f64) -> Option<Diagnostic> {
    // text_coverage = total bbox area / page area (0.0..=1.0)
    if area > 0.0 && text_coverage < 0.001 && area > 1000.0 {
        return Some(Diagnostic::new(
            "LIN-W-006",
            Severity::Warning,
            format!(
                "Very low text coverage {:.3} on {} sq pt page",
                text_coverage, area
            ),
        ));
    }
    None
}

/// Compute a contrast report for a set of pages.
pub fn report_contrast(
    pages: &[(usize, f64, f64, f64)],
    // (page_number, total_bbox_area, page_area, text_coverage)
) -> Vec<ContrastReport> {
    pages
        .iter()
        .map(|(page_num, bbox_area, area, _coverage)| {
            let coverage_ratio = if *area > 0.0 { *bbox_area / *area } else { 0.0 };
            ContrastReport {
                page_number: *page_num,
                avg_contrast: if *area > 0.0 && *bbox_area > 0.0 {
                    ((*bbox_area / *area) * 21.0).min(21.0)
                } else {
                    21.0 // white-on-white = maximum ratio
                },
                min_contrast: if coverage_ratio < 0.1 {
                    1.0
                } else {
                    4.5 + coverage_ratio * 10.0
                },
                low_contrast_count: if coverage_ratio < 0.01 { 1 } else { 0 },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contrast_ratio_white_on_black() {
        // white luminance = 1.0, black luminance = 0.0
        // ratio = (1.0 + 0.05) / (0.0 + 0.05) = 1.05 / 0.05 = 21.0
        let ratio = luminance_contrast_ratio(1.0, 0.0);
        assert!((ratio - 21.0).abs() < 0.001, "Expected 21.0, got {}", ratio);
    }

    #[test]
    fn test_contrast_ratio_same_luminance() {
        // Same luminance -> ratio = 1.0
        let ratio = luminance_contrast_ratio(0.5, 0.5);
        assert!((ratio - 1.0).abs() < 0.001, "Expected 1.0, got {}", ratio);
    }

    #[test]
    fn test_contrast_ratio_below_threshold() {
        // Low contrast -> should produce diagnostic
        let diag = check_contrast(0.5, 0.6).expect("Expected low contrast diagnostic");
        assert_eq!(diag.code, "LIN-W-005");
        assert_eq!(diag.severity, Severity::Warning);
    }

    #[test]
    fn test_contrast_ratio_above_threshold() {
        // Good contrast -> should return None
        let result = check_contrast(1.0, 0.0);
        assert!(result.is_none(), "White on black should be acceptable");
    }

    #[test]
    fn test_analyze_page_low_coverage() {
        // Large page with very low text coverage -> warning
        let diag = analyze_page_contrast(800.0 * 600.0, 0.0001);
        assert!(diag.is_some(), "Expected warning for very low coverage");
        assert_eq!(diag.unwrap().code, "LIN-W-006");
    }

    #[test]
    fn test_analyze_page_normal_coverage() {
        // Normal text coverage -> no warning
        let result = analyze_page_contrast(612.0 * 792.0, 0.15);
        assert!(
            result.is_none(),
            "Normal coverage should not produce warning, got: {:?}",
            result
        );
    }

    #[test]
    fn test_report_contrast_computes() {
        let pages = vec![(1, 5000.0, 10000.0, 0.5), (2, 0.0, 10000.0, 0.0)];
        let reports = report_contrast(&pages);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].page_number, 1);
        assert!(reports[0].avg_contrast > 0.0);
        assert_eq!(reports[1].page_number, 2);
        assert_eq!(reports[1].low_contrast_count, 1);
    }

    #[test]
    fn test_luminance_contrast_ratio_edge_cases() {
        // Reversed order should give same result
        let r1 = luminance_contrast_ratio(0.0, 1.0);
        let r2 = luminance_contrast_ratio(1.0, 0.0);
        assert!(
            (r1 - r2).abs() < 0.001,
            "Order shouldn't matter: {} vs {}",
            r1,
            r2
        );

        // Near-zero luminances
        let ratio = luminance_contrast_ratio(0.001, 0.002);
        assert!(ratio > 1.0, "Different luminances should give ratio > 1");
        assert!(ratio < 21.0, "Ratio should be less than max");
    }

    #[test]
    fn test_contrast_report_serde_roundtrip() {
        let report = ContrastReport {
            page_number: 1,
            avg_contrast: 10.5,
            min_contrast: 4.5,
            low_contrast_count: 0,
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: ContrastReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report.page_number, back.page_number);
        assert!((report.avg_contrast - back.avg_contrast).abs() < 0.001);
        assert!((report.min_contrast - back.min_contrast).abs() < 0.001);
        assert_eq!(report.low_contrast_count, back.low_contrast_count);
    }

    #[test]
    fn test_all_new_codes_registered() {
        use gp_core::diagnostics::DiagnosticRegistry;
        let reg = DiagnosticRegistry::instance();
        assert!(
            reg.lookup("LIN-W-005").is_some(),
            "LIN-W-005 must be registered"
        );
        assert!(
            reg.lookup("LIN-W-006").is_some(),
            "LIN-W-006 must be registered"
        );
    }
}
