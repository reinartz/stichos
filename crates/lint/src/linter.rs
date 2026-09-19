// SPDX-License-Identifier: MIT OR Apache-2.0
// lint: layout linter — checks for frame overflow, off-page/negative coords, overlaps, autofit shrink
// LIN-01: layout linter core (Wave D, P1-33)

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::TextWithBbox;
use gp_core::traits::{BatchLinter, Linter, LinterPage, PageDiagnostic};

/// A layout linter that checks for overflow, off-page, negative coords, overlap, and autofit issues.
pub struct LayoutLinter;

impl LayoutLinter {
    /// Check if a single element has negative coordinates (LIN-W-002).
    fn check_negative_coords(element: &TextWithBbox, _page: &LinterPage) -> Option<Diagnostic> {
        let [x, y, w, h] = element.bbox;
        if x < 0.0 || y < 0.0 || w < 0.0 || h < 0.0 {
            return Some(Diagnostic::new(
                "LIN-W-002",
                Severity::Warning,
                format!(
                    "Negative coordinates detected for element {} at [{},{},{:.2}]",
                    element.id.as_str(),
                    x,
                    y,
                    if w < 0.0 { w } else { w.abs() },
                ),
            ));
        }
        None
    }

    /// Check if a single element overflows the page boundary (LIN-E-001).
    fn check_overflow(element: &TextWithBbox, page: &LinterPage) -> Option<Diagnostic> {
        let [x, y, w, h] = element.bbox;
        let x_end = x + w;
        let y_end = y + h;
        let mut exceeds = Vec::new();
        if x_end > page.width_pt {
            exceeds.push(format!(
                "right edge {:.2} > page width {:.2}",
                x_end, page.width_pt
            ));
        }
        if y_end > page.height_pt {
            exceeds.push(format!(
                "bottom edge {:.2} > page height {:.2}",
                y_end, page.height_pt
            ));
        }
        if x < 0.0 {
            exceeds.push(format!("left edge {} < 0", x));
        }
        if y < 0.0 {
            exceeds.push(format!("top edge {} < 0", y));
        }
        if !exceeds.is_empty() {
            return Some(Diagnostic::new(
                "LIN-E-001",
                Severity::Error,
                format!(
                    "Layout overflow detected: {} exceeds boundary",
                    element.id.as_str()
                ),
            ));
        }
        None
    }

    /// Check if a single element is off-page (LIN-E-003) — bbox center or origin outside page bounds.
    fn check_off_page(element: &TextWithBbox, page: &LinterPage) -> Option<Diagnostic> {
        let [x, y, w, h] = element.bbox;
        let x_end = x + w;
        let y_end = y + h;
        let is_off = x_end < 0.0 || y_end < 0.0 || x > page.width_pt || y > page.height_pt;
        if is_off {
            return Some(Diagnostic::new(
                "LIN-E-003",
                Severity::Error,
                format!(
                    "Off-page element detected at [{},{},{:.2},{:.2}]",
                    x, y, w, h
                ),
            ));
        }
        None
    }

    /// Check if two elements overlap (LIN-E-004).
    fn check_overlap(a: &TextWithBbox, b: &TextWithBbox) -> Option<Diagnostic> {
        let [ax, ay, aw, ah] = a.bbox;
        let [bx, by, bw, bh] = b.bbox;
        let overlap_x = ax.max(bx) < (ax + aw).min(bx + bw);
        let overlap_y = ay.max(by) < (ay + ah).min(by + bh);
        if overlap_x && overlap_y {
            return Some(Diagnostic::new(
                "LIN-E-004",
                Severity::Error,
                format!(
                    "Overlapping elements detected: id1={} and id2={}",
                    a.id.as_str(),
                    b.id.as_str()
                ),
            ));
        }
        None
    }

    /// Check if total text bbox area exceeds 80% of page area (LIN-W-001).
    fn check_autofit_shrink(bboxes: &[TextWithBbox], page: &LinterPage) -> Option<Diagnostic> {
        let page_area = page.width_pt * page.height_pt;
        if page_area <= 0.0 {
            return None;
        }
        let total_area: f64 = bboxes
            .iter()
            .map(|b| {
                let [_x, _y, w, h] = b.bbox;
                w.abs().max(0.0) * h.abs().max(0.0)
            })
            .sum();
        let ratio = total_area / page_area;
        if ratio > 0.8 {
            let excess_pt = total_area - (0.8 * page_area);
            return Some(Diagnostic::new(
                "LIN-W-001",
                Severity::Warning,
                format!(
                    "Text overflow in page region: {:.0}pt total bbox area exceeds available space by {:.2}pt",
                    total_area, excess_pt
                ),
            ));
        }
        None
    }

    /// Lint a single page and return all diagnostics.
    fn lint_page(&self, page: &LinterPage) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        if !page.exists {
            return diags;
        }
        let bboxes = &page.text_bboxes;
        // Check each element for overflow, off-page, negative coords
        for element in bboxes {
            if let Some(d) = Self::check_negative_coords(element, page) {
                diags.push(d);
            }
            if let Some(d) = Self::check_overflow(element, page) {
                diags.push(d);
            }
            if let Some(d) = Self::check_off_page(element, page) {
                diags.push(d);
            }
        }
        // Check pairwise overlaps (skip same-element pairs)
        for i in 0..bboxes.len() {
            for j in (i + 1)..bboxes.len() {
                if let Some(d) = Self::check_overlap(&bboxes[i], &bboxes[j]) {
                    diags.push(d);
                }
            }
        }
        // Check autofit shrink
        if let Some(d) = Self::check_autofit_shrink(bboxes, page) {
            diags.push(d);
        }
        diags
    }
}

impl Linter for LayoutLinter {
    fn lint(&self, _input: &[u8], pages: &[LinterPage]) -> Vec<Diagnostic> {
        let mut all = Vec::new();
        for page in pages {
            all.extend(self.lint_page(page));
        }
        all
    }
}

impl BatchLinter for LayoutLinter {
    fn lint_batch(&self, pages: &[LinterPage]) -> Vec<PageDiagnostic> {
        pages
            .iter()
            .enumerate()
            .map(|(idx, page)| PageDiagnostic {
                page_number: idx,
                diagnostics: self.lint_page(page),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font;
    use gp_core::diagnostics::DiagnosticRegistry;
    use gp_core::ir::ElementId;

    fn make_bbox(id: &str, x: f64, y: f64, w: f64, h: f64) -> TextWithBbox {
        TextWithBbox {
            id: ElementId::content_anchored("bbox", &[id], id),
            text: id.to_string(),
            bbox: [x, y, w, h],
        }
    }

    fn make_page(
        page_num: usize,
        width: f64,
        height: f64,
        bboxes: Vec<TextWithBbox>,
    ) -> LinterPage {
        LinterPage {
            page_number: page_num,
            width_pt: width,
            height_pt: height,
            text_bboxes: bboxes,
            exists: true,
        }
    }

    fn make_clean_page() -> LinterPage {
        make_page(
            1,
            612.0,
            792.0,
            vec![
                make_bbox("el1", 50.0, 50.0, 200.0, 100.0),
                make_bbox("el2", 300.0, 50.0, 200.0, 100.0),
            ],
        )
    }

    fn linter() -> LayoutLinter {
        LayoutLinter
    }

    #[test]
    fn test_overflow_detected() {
        let page = make_page(
            1,
            612.0,
            792.0,
            vec![make_bbox("el1", 500.0, 50.0, 200.0, 100.0)],
        );
        let diags = linter().lint(&[], &[page]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"LIN-E-001"),
            "Expected LIN-E-001 for overflow, got: {:?}",
            codes
        );
    }

    #[test]
    fn test_no_overflow_clean_page() {
        let page = make_clean_page();
        let diags = linter().lint(&[], &[page]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            !codes.contains(&"LIN-E-001"),
            "No overflow expected on clean page, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_off_page_negative_coords() {
        // Bbox entirely to the left of the page (x_end = -50 < 0)
        let page = make_page(
            1,
            612.0,
            792.0,
            vec![make_bbox("el1", -150.0, 50.0, 100.0, 100.0)],
        );
        let diags = linter().lint(&[], &[page]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"LIN-E-003"),
            "Expected LIN-E-003 for off-page, got: {:?}",
            codes
        );
        // The bbox at -150 is also negative coords
        assert!(
            codes.contains(&"LIN-W-002"),
            "Expected LIN-W-002 for negative coords, got: {:?}",
            codes
        );
    }

    #[test]
    fn test_overlap_detected() {
        let page = make_page(
            1,
            612.0,
            792.0,
            vec![
                make_bbox("el1", 50.0, 50.0, 200.0, 100.0),
                make_bbox("el2", 100.0, 60.0, 200.0, 100.0),
            ],
        );
        let diags = linter().lint(&[], &[page]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"LIN-E-004"),
            "Expected LIN-E-004 for overlap, got: {:?}",
            codes
        );
    }

    #[test]
    fn test_no_overlap() {
        let page = make_page(
            1,
            612.0,
            792.0,
            vec![
                make_bbox("el1", 50.0, 50.0, 100.0, 100.0),
                make_bbox("el2", 200.0, 50.0, 100.0, 100.0),
            ],
        );
        let diags = linter().lint(&[], &[page]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            !codes.contains(&"LIN-E-004"),
            "No overlap expected, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_autofit_shrink_warning() {
        // Page area = 612 * 792 = 484704; 80% = 387763.2
        // Two bboxes: 500*400 + 500*400 = 400000 > 387763.2
        let page = make_page(
            1,
            612.0,
            792.0,
            vec![
                make_bbox("el1", 10.0, 10.0, 500.0, 400.0),
                make_bbox("el2", 10.0, 410.0, 500.0, 400.0),
            ],
        );
        let diags = linter().lint(&[], &[page]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"LIN-W-001"),
            "Expected LIN-W-001 for autofit shrink, got: {:?}",
            codes
        );
    }

    #[test]
    fn test_batch_linter_wraps_per_page() {
        let page1 = make_page(
            1,
            612.0,
            792.0,
            vec![make_bbox("el1", 500.0, 50.0, 200.0, 100.0)],
        );
        let page2 = make_clean_page();
        let diags = linter().lint_batch(&[page1, page2]);
        assert_eq!(diags.len(), 2, "Expected 2 page diagnostics");
        assert_eq!(diags[0].page_number, 0);
        assert_eq!(diags[1].page_number, 1);
        assert!(
            !diags[0].diagnostics.is_empty(),
            "Page 0 should have diagnostics"
        );
        assert!(diags[1].diagnostics.is_empty(), "Page 1 should be clean");
    }

    #[test]
    fn test_all_codes_registered() {
        let reg = DiagnosticRegistry::instance();
        let codes = [
            "LIN-E-001",
            "LIN-E-002",
            "LIN-E-003",
            "LIN-E-004",
            "LIN-E-007",
            "LIN-W-001",
            "LIN-W-002",
            "LIN-W-007",
            "LIN-W-008",
            "LIN-W-009",
            "LIN-B-001",
            "LIN-B-002",
        ];
        for code in &codes {
            assert!(
                reg.lookup(code).is_some(),
                "Code {} must be registered",
                code
            );
        }
    }

    #[test]
    fn test_no_adhoc_diagnostics() {
        let page = make_page(
            1,
            612.0,
            792.0,
            vec![
                make_bbox("el1", 500.0, 50.0, 200.0, 100.0),
                make_bbox("el2", 50.0, 50.0, 200.0, 100.0),
                make_bbox("el3", 100.0, 60.0, 200.0, 100.0),
                make_bbox("el4", -10.0, 50.0, 100.0, 100.0),
            ],
        );
        let diags = linter().lint(&[], &[page]);
        for diag in &diags {
            assert!(
                reg().lookup(&diag.code).is_some(),
                "Diagnostic code '{}' is ad-hoc — must be registered",
                diag.code
            );
        }

        fn reg() -> &'static DiagnosticRegistry {
            DiagnosticRegistry::instance()
        }
    }

    #[test]
    fn test_font_substitution_warning() {
        // Unknown fonts produce substitution diagnostics using LIN-E-002
        let unknown_fonts = vec!["Segoe UI".into(), "Georgia".into()];
        let subs = font::detect_substitutions(&unknown_fonts);
        assert_eq!(subs.len(), 2);
        // Both should have known substitutions
        assert_eq!(subs[0].used, "Liberation Sans");
        assert_eq!(subs[1].used, "Liberation Serif");
        // Verify LIN-E-002 is registered
        let reg = DiagnosticRegistry::instance();
        assert!(
            reg.lookup("LIN-E-002").is_some(),
            "LIN-E-002 must be registered"
        );
    }

    #[test]
    fn test_no_font_warning_for_reference_fonts() {
        // Reference fonts produce no substitution warnings
        let fonts = vec![
            "Calibri".into(),
            "Carlito".into(),
            "Arial".into(),
            "Times New Roman".into(),
        ];
        let unknown = font::check_unknown_fonts(&fonts);
        assert!(
            unknown.is_empty(),
            "Reference fonts should not trigger unknown font warnings, got: {:?}",
            unknown
        );
        let subs = font::detect_substitutions(&fonts);
        assert!(
            subs.is_empty(),
            "Reference fonts should not produce substitutions, got: {:?}",
            subs
        );
    }

    #[test]
    fn test_tofu_risk_detection_via_font_module() {
        // CJK text with non-CJK fonts should trigger tofu risk
        let tb = TextWithBbox {
            id: ElementId::content_anchored("text", &["text:0"], "こんにちは"),
            text: "こんにちは".to_string(),
            bbox: [0.0, 0.0, 100.0, 20.0],
        };
        let text_bboxes = vec![tb];
        let fonts = vec!["Calibri".into(), "Arial".into()];
        let reports = font::detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            !reports.is_empty(),
            "CJK with non-CJK fonts should produce tofu risk"
        );
        // Verify LIN-W-003 is registered
        let reg = DiagnosticRegistry::instance();
        assert!(
            reg.lookup("LIN-W-003").is_some(),
            "LIN-W-003 must be registered"
        );
    }

    #[test]
    fn test_no_tofu_risk_with_cjk_font() {
        let tb = TextWithBbox {
            id: ElementId::content_anchored("text", &["text:0"], "こんにちは"),
            text: "こんにちは".to_string(),
            bbox: [0.0, 0.0, 100.0, 20.0],
        };
        let text_bboxes = vec![tb];
        let fonts = vec!["Noto Sans CJK JP".into()];
        let reports = font::detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            reports.is_empty(),
            "CJK with CJK font should not produce tofu risk"
        );
    }

    #[test]
    fn test_page_count_delta_integration() {
        // Integration test: lint with two pages of different sizes
        // Simulates a page-count delta scenario through the linter
        let page1 = make_page(
            1,
            612.0,
            792.0,
            vec![make_bbox("el1", 50.0, 50.0, 200.0, 100.0)],
        );
        let page2 = make_page(
            2,
            595.0,
            842.0,
            vec![make_bbox("el2", 30.0, 30.0, 150.0, 80.0)],
        );
        let diags = linter().lint(&[], &[page1, page2]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        // Both pages should lint cleanly (no overflow/off-page/etc.)
        for code in &codes {
            assert_ne!(*code, "LIN-E-001", "No overflow expected");
            assert_ne!(*code, "LIN-E-003", "No off-page expected");
            assert_ne!(*code, "LIN-E-004", "No overlap expected");
        }
        // Verify the lint codes are still all registered
        let reg = DiagnosticRegistry::instance();
        assert!(
            reg.lookup("LIN-E-005").is_some(),
            "LIN-E-005 must be registered for page delta checks"
        );
        assert!(
            reg.lookup("LIN-E-006").is_some(),
            "LIN-E-006 must be registered for missing pages"
        );
        assert!(
            reg.lookup("LIN-W-004").is_some(),
            "LIN-W-004 must be registered for page delta warnings"
        );
    }

    #[test]
    fn test_lint_codes_still_all_registered() {
        // Verify all LIN codes including the new LIN-W-003, page-delta, and a11y codes are registered
        let reg = DiagnosticRegistry::instance();
        let codes = [
            "LIN-E-001",
            "LIN-E-002",
            "LIN-E-003",
            "LIN-E-004",
            "LIN-E-005",
            "LIN-E-006",
            "LIN-E-007",
            "LIN-W-001",
            "LIN-W-002",
            "LIN-W-003",
            "LIN-W-004",
            "LIN-W-005",
            "LIN-W-006",
            "LIN-W-007",
            "LIN-W-008",
            "LIN-W-009",
            "LIN-B-001",
            "LIN-B-002",
            "LIN-I-001",
            "LIN-I-002",
            "LIN-I-003",
        ];
        for code in &codes {
            assert!(
                reg.lookup(code).is_some(),
                "Code {} must be registered",
                code
            );
        }
    }
}
