// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula parity lint harness — inventory and classify formulas across corpus fixtures.
//!
//! Accepts pre-parsed `Spreadsheet` IR (produced by `adapters_ooxml::xlsx::read_xlsx`) and
//! classifies each formula as `Supported`, `Refused`, or `ParseError`.
//!
//! The caller is responsible for OPC parsing (via `adapters_ooxml`) and directory walking.
//! This keeps `lint` free of a circular dependency on `adapters_ooxml`.
//!
//! Example usage:
//! ```ignore
//! use lint::formula_parity::inventory_from_spreadsheet;
//!
//! // After parsing an xlsx with adapters_ooxml:
//! let pkg = adapters_ooxml::opc::parse_opc(&data).unwrap();
//! let read_result = adapters_ooxml::xlsx::read_xlsx(&pkg).unwrap();
//! let spreadsheet: Spreadsheet = serde_json::from_slice(&read_result.ir).unwrap();
//!
//! let results = inventory_from_spreadsheet("my_fixture", &spreadsheet);
//! for f in &results {
//!     println!("{:?}: {}", f.category, f.formula);
//! }
//! ```

use gp_core::formula::check_refusal;
use gp_core::formula::is_supported_function;
use gp_core::formula::GpFCode;
use gp_core::ir::Spreadsheet;

/// A formula found in a corpus fixture.
#[derive(Debug, Clone)]
pub struct CorpusFormula {
    pub fixture_name: String,
    pub formula: String,
    pub category: FormulaCategory,
}

/// Classification of a formula string against the supported function set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaCategory {
    /// The formula's primary function is in the supported set.
    Supported,
    /// The formula references a known unsupported/refused function.
    Refused(GpFCode),
    /// The formula string does not parse as a valid formula expression.
    ParseError,
}

/// Inventory all formulas from a single pre-parsed spreadsheet.
///
/// The caller must have already parsed the OPC package and deserialized
/// the IR (typically via `adapters_ooxml::xlsx::read_xlsx` + `serde_json`).
pub fn inventory_from_spreadsheet(
    fixture_name: &str,
    spreadsheet: &Spreadsheet,
) -> Vec<CorpusFormula> {
    let mut formulas = Vec::new();
    for sheet in &spreadsheet.sheets {
        for row in &sheet.rows {
            for cell in &row.cells {
                if let Some(ref formula) = cell.formula {
                    let category = classify_formula(formula);
                    formulas.push(CorpusFormula {
                        fixture_name: fixture_name.to_string(),
                        formula: formula.clone(),
                        category,
                    });
                }
            }
        }
    }
    formulas
}

/// Classify a formula string as supported, refused, or parse error.
///
/// Extracts the function name from the formula string (e.g., `=SUM(A1:B2)` → `SUM`)
/// and checks against the supported function set and refusal criteria.
pub fn classify_formula(formula: &str) -> FormulaCategory {
    // Strip leading '=' if present (Excel formulas often start with =)
    let trimmed = formula.trim().trim_start_matches('=');
    let upper = trimmed.to_uppercase();

    // Check refusal criteria FIRST (before function name lookup)
    if let Some(code) = check_refusal(trimmed) {
        return FormulaCategory::Refused(code);
    }

    // Extract the function name from the formula (e.g., "SUM(A1:B2)" -> "SUM")
    let func_name = match upper.find('(') {
        Some(pos) => &upper[..pos],
        None => return FormulaCategory::ParseError,
    };
    let func_name = func_name.trim();

    // Check if it's a supported function
    if is_supported_function(func_name) {
        return FormulaCategory::Supported;
    }

    // Function exists but isn't in the supported set
    if !func_name.is_empty() && func_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return FormulaCategory::Refused(GpFCode::UnsupportedFunction);
    }

    FormulaCategory::ParseError
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_supported_sum() {
        assert_eq!(classify_formula("=SUM(A1:B2)"), FormulaCategory::Supported);
    }

    #[test]
    fn test_classify_supported_average() {
        assert_eq!(
            classify_formula("=AVERAGE(C1:C10)"),
            FormulaCategory::Supported
        );
    }

    #[test]
    fn test_classify_supported_lower_case() {
        assert_eq!(
            classify_formula("=round(B1, 2)"),
            FormulaCategory::Supported
        );
    }

    #[test]
    fn test_classify_refused_vlookup() {
        match classify_formula("=VLOOKUP(A1, B1:C10, 2, FALSE)") {
            FormulaCategory::Refused(code) => {
                assert_eq!(code, GpFCode::UnsupportedFunction);
            }
            other => panic!("Expected Refused, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_refused_array_function() {
        match classify_formula("=MMULT(A1:B2,C1:D2)") {
            FormulaCategory::Refused(code) => {
                assert_eq!(code, GpFCode::ArrayMatrixFunction);
            }
            other => panic!("Expected Refused(ArrayMatrixFunction), got {:?}", other),
        }
    }

    #[test]
    fn test_classify_parse_error_no_parens() {
        assert_eq!(classify_formula("=1+2"), FormulaCategory::ParseError);
    }

    #[test]
    fn test_classify_supported_text_funcs() {
        assert_eq!(classify_formula("=LEN(A1)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=TRIM(B2)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=UPPER(C3)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=LOWER(D4)"), FormulaCategory::Supported);
        assert_eq!(
            classify_formula("=SUBSTITUTE(E1,F1,G1)"),
            FormulaCategory::Supported
        );
        assert_eq!(classify_formula("=LEFT(H1,3)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=RIGHT(I1,2)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=MID(J1,1,5)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=REPT(A1,3)"), FormulaCategory::Supported);
    }

    #[test]
    fn test_classify_supported_math_funcs() {
        assert_eq!(classify_formula("=ABS(A1)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=ROUND(A1,2)"), FormulaCategory::Supported);
        assert_eq!(
            classify_formula("=ROUNDUP(A1,0)"),
            FormulaCategory::Supported
        );
        assert_eq!(
            classify_formula("=ROUNDDOWN(A1,1)"),
            FormulaCategory::Supported
        );
        assert_eq!(classify_formula("=MOD(10,3)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=INT(3.7)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=POWER(2,3)"), FormulaCategory::Supported);
    }

    #[test]
    fn test_classify_supported_date_funcs() {
        assert_eq!(classify_formula("=TODAY()"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=NOW()"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=YEAR(A1)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=MONTH(B1)"), FormulaCategory::Supported);
        assert_eq!(classify_formula("=DAY(C1)"), FormulaCategory::Supported);
        assert_eq!(
            classify_formula("=DATEDIF(D1,E1,\"d\")"),
            FormulaCategory::Supported
        );
        assert_eq!(
            classify_formula("=EOMONTH(F1,0)"),
            FormulaCategory::Supported
        );
    }

    #[test]
    fn test_classify_supported_stat_funcs() {
        assert_eq!(
            classify_formula("=COUNT(A1:A10)"),
            FormulaCategory::Supported
        );
        assert_eq!(
            classify_formula("=COUNTA(B1:B10)"),
            FormulaCategory::Supported
        );
        assert_eq!(
            classify_formula("=COUNTBLANK(C1)"),
            FormulaCategory::Supported
        );
        assert_eq!(
            classify_formula("=STDEV(D1:D10)"),
            FormulaCategory::Supported
        );
        assert_eq!(classify_formula("=VAR(E1:E10)"), FormulaCategory::Supported);
        assert_eq!(
            classify_formula("=RANK(F1,G1:G10)"),
            FormulaCategory::Supported
        );
    }

    #[test]
    fn test_classify_supported_error_funcs() {
        assert_eq!(
            classify_formula("=IFERROR(A1/0,0)"),
            FormulaCategory::Supported
        );
        assert_eq!(
            classify_formula("=ISNUMBER(B1)"),
            FormulaCategory::Supported
        );
        assert_eq!(classify_formula("=ISTEXT(C1)"), FormulaCategory::Supported);
        assert_eq!(
            classify_formula("=IFNA(D1,\"N/A\")"),
            FormulaCategory::Supported
        );
    }

    #[test]
    fn test_classify_external_workbook_refusal() {
        match classify_formula("=SUM([Book2.xlsx]Sheet1!A1)") {
            FormulaCategory::Refused(code) => {
                assert_eq!(code, GpFCode::ExternalWorkbook);
            }
            other => panic!("Expected Refused(ExternalWorkbook), got {:?}", other),
        }
    }

    #[test]
    fn test_inventory_from_spreadsheet() {
        use gp_core::ir::{ElementId, Sheet, SheetCell, SheetRow, Spreadsheet};

        let spreadsheet = Spreadsheet {
            ir_version: "1.0".to_string(),
            ir_version_migration: None,
            sheets: vec![Sheet {
                id: ElementId::content_anchored("sheet", &["sheet:0"], "1.0"),
                name: "Sheet1".to_string(),
                rows: vec![SheetRow {
                    id: ElementId::content_anchored("row", &["row:0"], "1.0"),
                    cells: vec![
                        SheetCell {
                            id: ElementId::content_anchored("cell", &["cell:A1"], "1.0"),
                            row: 0,
                            col: 0,
                            value: None,
                            formula: Some("=SUM(A1:B2)".to_string()),
                            cached_value: None,
                            style_name: None,
                            number_format: None,
                            rich: None,
                        },
                        SheetCell {
                            id: ElementId::content_anchored("cell", &["cell:B1"], "1.0"),
                            row: 0,
                            col: 1,
                            value: None,
                            formula: Some("=VLOOKUP(A1,B1:C10,2,FALSE)".to_string()),
                            cached_value: None,
                            style_name: None,
                            number_format: None,
                            rich: None,
                        },
                        SheetCell {
                            id: ElementId::content_anchored("cell", &["cell:C1"], "1.0"),
                            row: 0,
                            col: 2,
                            value: None,
                            formula: Some("=1+2".to_string()),
                            cached_value: None,
                            style_name: None,
                            number_format: None,
                            rich: None,
                        },
                    ],
                }],
                merged_ranges: vec![],
                header_row: None,
                footer_row: None,
            }],
        };

        let results = inventory_from_spreadsheet("test_fixture", &spreadsheet);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].category, FormulaCategory::Supported);
        assert_eq!(
            results[1].category,
            FormulaCategory::Refused(GpFCode::UnsupportedFunction)
        );
        assert_eq!(results[2].category, FormulaCategory::ParseError);
    }
}
