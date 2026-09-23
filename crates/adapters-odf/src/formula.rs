// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula evaluation for ODS: re-exports from gp-core.
//!
//! This module provides the shared formula evaluation engine for ODS spreadsheet
//! parsing. It re-exports all types and functions from `gp-core::formula` to
//! maintain API compatibility.

pub use gp_core::formula::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_refusal_simple() {
        assert_eq!(check_refusal("1+2"), None);
        assert_eq!(check_refusal("SUM(A1:A5)"), None);
        assert_eq!(check_refusal("IF(A1>0,1,0)"), None);
    }

    #[test]
    fn test_check_refusal_external_workbook() {
        assert_eq!(
            check_refusal("SUM([Book2.xlsx]Sheet1!A1)"),
            Some(GpFCode::ExternalWorkbook)
        );
    }

    #[test]
    fn test_check_refusal_array_function() {
        assert_eq!(
            check_refusal("MMULT(A1:B2,C1:D2)"),
            Some(GpFCode::ArrayMatrixFunction)
        );
    }

    #[test]
    fn test_check_refusal_macro() {
        assert_eq!(
            check_refusal("SUBPROCESS!RUN"),
            Some(GpFCode::MacroReference)
        );
    }

    #[test]
    fn test_check_refusal_dde() {
        assert_eq!(
            check_refusal("DDE|server|topic"),
            Some(GpFCode::DdeOleReference)
        );
        assert_eq!(
            check_refusal("OLE|object|data"),
            Some(GpFCode::DdeOleReference)
        );
    }

    #[test]
    fn test_check_refusal_deep_nesting() {
        let deep = format!("{}1{}", "(".repeat(33), ")".repeat(33));
        assert_eq!(check_refusal(&deep), Some(GpFCode::DeepNesting));
    }

    #[test]
    fn test_parse_and_evaluate_sum() {
        let node = parse_formula("SUM(A1:A5)").unwrap();
        let ctx = TestCtx::default();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(0.0)));
    }

    #[test]
    fn test_parse_and_evaluate_arithmetic() {
        let wrap = |n: FormulaNode| FormulaNodeWrapper(n);
        assert_eq!(
            eval_with(&wrap(parse_formula("1+2").unwrap())),
            Some(EvalResult::Number(3.0))
        );
        assert_eq!(
            eval_with(&wrap(parse_formula("10-3").unwrap())),
            Some(EvalResult::Number(7.0))
        );
        assert_eq!(
            eval_with(&wrap(parse_formula("4*3").unwrap())),
            Some(EvalResult::Number(12.0))
        );
        assert_eq!(
            eval_with(&wrap(parse_formula("15/3").unwrap())),
            Some(EvalResult::Number(5.0))
        );
    }

    #[test]
    fn test_parse_and_evaluate_if() {
        let node = parse_formula("IF(TRUE,10,20)").unwrap();
        let ctx = TestCtx::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(10.0)));

        let node = parse_formula("IF(FALSE,10,20)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(20.0)));
    }

    #[test]
    fn test_parse_and_evaluate_and_or() {
        let node = parse_formula("AND(TRUE,TRUE)").unwrap();
        assert_eq!(
            evaluate(&node, &TestCtx::default()),
            Some(EvalResult::Bool(true))
        );

        let node = parse_formula("OR(FALSE,FALSE)").unwrap();
        assert_eq!(
            evaluate(&node, &TestCtx::default()),
            Some(EvalResult::Bool(false))
        );
    }

    #[test]
    fn test_parse_and_evaluate_cell_ref() {
        let ctx = TestCtx::default().with_value("A1", 42.0);
        let node = parse_formula("A1").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_parity_match() {
        let result = EvalResult::Number(42.0);
        assert_eq!(check_parity(Some("42"), &result), ParityResult::Match);
    }

    #[test]
    fn test_parity_mismatch() {
        let result = EvalResult::Number(42.0);
        assert!(matches!(
            check_parity(Some("100"), &result),
            ParityResult::Mismatch { .. }
        ));
    }

    #[test]
    fn test_parity_no_cache() {
        let result = EvalResult::Number(42.0);
        assert_eq!(check_parity(None, &result), ParityResult::NoCache);
    }

    #[test]
    fn test_parity_refused() {
        assert!(matches!(
            parity_refused(GpFCode::ExternalWorkbook),
            ParityResult::Refused(_)
        ));
    }

    /// Minimal test context for formula evaluation.
    #[derive(Default)]
    struct TestCtx {
        cells: std::collections::HashMap<String, CachedCellValue>,
    }

    impl TestCtx {
        fn with_value(mut self, ref_name: &str, value: f64) -> Self {
            self.cells.insert(
                ref_name.to_string(),
                CachedCellValue {
                    numeric: Some(value),
                    text: None,
                    bool: None,
                },
            );
            self
        }
    }

    impl FormulaContext for TestCtx {
        fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
            self.cells.get(ref_name).cloned()
        }
        fn get_range(&self, _range: &str) -> Option<Vec<CachedCellValue>> {
            None
        }
    }

    fn eval_with(node: &FormulaNodeWrapper) -> Option<EvalResult> {
        evaluate(&node.0, &TestCtx::default())
    }

    struct FormulaNodeWrapper(pub FormulaNode);
}
