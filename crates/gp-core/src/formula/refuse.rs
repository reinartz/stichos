// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula refusal detection.
//!
//! Checks formula strings for patterns that indicate unsupported or hostile formulas.
//! Returns a GP-F0xx code if the formula should be refused.

/// GP-F0xx refusal codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpFCode {
    /// GP-F-001: External workbook reference (e.g., [workbook]sheet!cell).
    ExternalWorkbook,
    /// GP-F-002: Array/matrix function.
    ArrayMatrixFunction,
    /// GP-F-003: Macro/VBA reference.
    MacroReference,
    /// GP-F-004: DDE/OLE external reference.
    DdeOleReference,
    /// GP-F-010: Deep nesting (>32 levels).
    DeepNesting,
    /// GP-F-020: Unsupported function name.
    UnsupportedFunction,
}

/// Check if a formula string matches any refusal criteria.
///
/// Returns `Some(GpFCode)` if the formula should be refused, `None` if it's potentially
/// evaluable.
pub fn check_refusal(formula: &str) -> Option<GpFCode> {
    // Check for external workbook references: [workbook], sheet!cell, DDE/OLE patterns
    if formula.contains('[') && formula.contains(']') {
        return Some(GpFCode::ExternalWorkbook);
    }

    // Check for DDE/OLE external references
    if formula.contains("DDE|") || formula.contains("OLE|") {
        return Some(GpFCode::DdeOleReference);
    }

    // Check for macro/VBA references
    let upper = formula.to_uppercase();
    if upper.contains("SUBPROCESS")
        || upper.contains("RUNMACRO")
        || upper.contains("!") && (upper.contains("VBA") || upper.contains("MACRO"))
    {
        return Some(GpFCode::MacroReference);
    }

    // Check for array/matrix functions
    let array_funcs = [
        "MMULT",
        "TRANSPOSE",
        "ARRAY",
        "MDETERM",
        "MINVERSE",
        "MMULT",
        "SERIES",
        "SUMPRODUCT",
        "SUMX2PY2",
        "SUMX2MY2",
        "SUMXMY2",
    ];
    for func in &array_funcs {
        if upper.contains(*func) {
            return Some(GpFCode::ArrayMatrixFunction);
        }
    }

    // Check for deep nesting (parentheses depth > 32)
    let mut depth = 0u32;
    for c in formula.chars() {
        if c == '(' {
            depth = depth.saturating_add(1);
        } else if c == ')' {
            depth = depth.saturating_sub(1);
        }
        if depth > 32 {
            return Some(GpFCode::DeepNesting);
        }
    }

    None
}

/// Return the list of all refusal codes — for conformance testing.
pub fn all_refuse_codes() -> &'static [GpFCode] {
    &[
        GpFCode::ExternalWorkbook,
        GpFCode::ArrayMatrixFunction,
        GpFCode::MacroReference,
        GpFCode::DdeOleReference,
        GpFCode::DeepNesting,
        GpFCode::UnsupportedFunction,
    ]
}

/// Return the GP-F0xx code string for a refusal code.
pub fn gp_f_code_str(code: GpFCode) -> &'static str {
    match code {
        GpFCode::ExternalWorkbook => "GP-F-001",
        GpFCode::ArrayMatrixFunction => "GP-F-002",
        GpFCode::MacroReference => "GP-F-003",
        GpFCode::DdeOleReference => "GP-F-004",
        GpFCode::DeepNesting => "GP-F-010",
        GpFCode::UnsupportedFunction => "GP-F-020",
    }
}

/// Return the list of supported function names (v1.0.0 + P7-3 Tranche A).
pub fn supported_functions() -> &'static [&'static str] {
    &[
        "SUM",
        "AVERAGE",
        "AVG",
        "COUNT",
        "MIN",
        "MAX",
        "AND",
        "OR",
        "NOT",
        "IF",
        "CONCATENATE",
        "IFERROR",
        "CONCAT",
        // Date/Time (A4)
        "TODAY",
        "NOW",
        "YEAR",
        "MONTH",
        "DAY",
        "DATEDIF",
        "EOMONTH",
        // Statistical (A5)
        "COUNTA",
        "COUNTBLANK",
        "STDEV",
        "STDEV_S",
        "STDEVP",
        "STDEV_P",
        "VAR",
        "VAR_S",
        "VARP",
        "VAR_P",
        "RANK",
        "RANK_EQ",
        // Error handling / type checking (A6)
        "ISERROR",
        "ISNUMBER",
        "ISTEXT",
        "IFNA",
        // Math functions (P7-3 Tranche A)
        "ABS",
        "ROUND",
        "ROUNDUP",
        "ROUNDDOWN",
        "MOD",
        "INT",
        "POWER",
        // Text functions (P7-3 Tranche A)
        "LEN",
        "TRIM",
        "UPPER",
        "LOWER",
        "REPT",
        "SUBSTITUTE",
        "LEFT",
        "RIGHT",
        "MID",
    ]
}

/// Check if a function name is in the supported set.
pub fn is_supported_function(name: &str) -> bool {
    let upper = name.to_uppercase();
    supported_functions().contains(&upper.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_refusal_simple() {
        assert_eq!(check_refusal("1+2"), None);
        assert_eq!(check_refusal("SUM(A1:A5)"), None);
        assert_eq!(check_refusal("IF(B1>0,1,0)"), None);
    }

    #[test]
    fn test_refuse_external_workbook() {
        assert_eq!(
            check_refusal("SUM([Book2.xlsx]Sheet1!A1)"),
            Some(GpFCode::ExternalWorkbook)
        );
    }

    #[test]
    fn test_refuse_array_matrix_function() {
        assert_eq!(
            check_refusal("MMULT(A1:B2,C1:D2)"),
            Some(GpFCode::ArrayMatrixFunction)
        );
        assert_eq!(
            check_refusal("TRANSPOSE(A1:B2)"),
            Some(GpFCode::ArrayMatrixFunction)
        );
        assert_eq!(
            check_refusal("MDETERM(A1:B2)"),
            Some(GpFCode::ArrayMatrixFunction)
        );
    }

    #[test]
    fn test_refuse_macro_reference() {
        assert_eq!(
            check_refusal("SUBPROCESS!RUN"),
            Some(GpFCode::MacroReference)
        );
        assert_eq!(check_refusal("RUNMACRO"), Some(GpFCode::MacroReference));
    }

    #[test]
    fn test_refuse_dde_ole() {
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
    fn test_refuse_deep_nesting() {
        let deep = format!("{}1{}", "(".repeat(33), ")".repeat(33));
        assert_eq!(check_refusal(&deep), Some(GpFCode::DeepNesting));
    }

    #[test]
    fn test_allowed_nesting() {
        let moderate = format!("{}1{}", "(".repeat(32), ")".repeat(32));
        assert_eq!(check_refusal(&moderate), None);
    }

    #[test]
    fn test_supported_functions() {
        for func in supported_functions() {
            assert!(is_supported_function(func));
            assert!(is_supported_function(&func.to_lowercase()));
            assert!(is_supported_function(&func.to_lowercase()));
        }
    }

    #[test]
    fn test_unsupported_function() {
        assert!(!is_supported_function("VLOOKUP"));
        assert!(!is_supported_function("INDEX"));
        assert!(!is_supported_function("MATCH"));
        assert!(!is_supported_function("RANDBETWEEN"));
    }

    #[test]
    fn test_all_refuse_codes() {
        let codes = all_refuse_codes();
        assert_eq!(codes.len(), 6);
        assert!(codes.contains(&GpFCode::ExternalWorkbook));
        assert!(codes.contains(&GpFCode::ArrayMatrixFunction));
        assert!(codes.contains(&GpFCode::MacroReference));
        assert!(codes.contains(&GpFCode::DdeOleReference));
        assert!(codes.contains(&GpFCode::DeepNesting));
        assert!(codes.contains(&GpFCode::UnsupportedFunction));
    }

    #[test]
    fn test_gp_f_code_str() {
        assert_eq!(gp_f_code_str(GpFCode::ExternalWorkbook), "GP-F-001");
        assert_eq!(gp_f_code_str(GpFCode::ArrayMatrixFunction), "GP-F-002");
        assert_eq!(gp_f_code_str(GpFCode::MacroReference), "GP-F-003");
        assert_eq!(gp_f_code_str(GpFCode::DdeOleReference), "GP-F-004");
        assert_eq!(gp_f_code_str(GpFCode::DeepNesting), "GP-F-010");
        assert_eq!(gp_f_code_str(GpFCode::UnsupportedFunction), "GP-F-020");
    }
}
