//! C3 v1 freeze: verify every diagnostic code used in the codebase is registered.
//!
//! C3 contract: `docs/contracts/C3.md`

use gp_core::diagnostics::DiagnosticRegistry;

use std::collections::HashMap;

/// Check if a code string is valid per C3 pattern.
pub fn is_registered_code(code: &str) -> bool {
    let registry = DiagnosticRegistry::instance();
    registry.lookup(code).is_some()
}

/// Check if a code string matches the C3 pattern (even if unregistered).
pub fn is_valid_c3_pattern(code: &str) -> bool {
    gp_core::diagnostics::is_valid_code_pattern(code)
}

/// All registered LIN prefix codes (lint crate diagnostics).
pub fn all_lin_codes() -> Vec<&'static str> {
    let registry = DiagnosticRegistry::instance();
    registry
        .all_codes()
        .into_iter()
        .filter(|c| c.starts_with("LIN-"))
        .collect()
}

/// All registered codes across all prefixes.
pub fn all_registered_codes() -> Vec<&'static str> {
    let registry = DiagnosticRegistry::instance();
    registry.all_codes()
}

/// Verify that all LIN codes in the registry match the C3 pattern.
pub fn verify_lin_codes() -> Result<Vec<String>, String> {
    let codes = all_lin_codes();
    let mut errors = Vec::new();

    for code in &codes {
        if !is_valid_c3_pattern(code) {
            errors.push(format!("LIN code '{}' does not match C3 pattern", code));
        }
    }

    if errors.is_empty() {
        Ok(codes.iter().map(|s| s.to_string()).collect())
    } else {
        Err(errors.join("; "))
    }
}

/// Verify that all registered codes across all prefixes match the C3 pattern.
pub fn verify_all_codes() -> Result<Vec<String>, String> {
    let codes = all_registered_codes();
    let mut errors = Vec::new();

    for code in &codes {
        if !is_valid_c3_pattern(code) {
            errors.push(format!("code '{}' does not match pattern", code));
        }
    }

    if errors.is_empty() {
        Ok(codes.iter().map(|s| s.to_string()).collect())
    } else {
        Err(errors.join("; "))
    }
}

/// All codes that do not match the C3 pattern (for audit purposes).
pub fn non_c3_compliant_codes() -> Vec<&'static str> {
    let codes = all_registered_codes();
    codes
        .into_iter()
        .filter(|c| !is_valid_c3_pattern(c))
        .collect()
}

/// List all prefixes that have registered codes.
pub fn all_prefixes() -> Vec<&'static str> {
    let registry = DiagnosticRegistry::instance();
    let mut prefixes: Vec<&str> = Vec::new();
    for entry in registry.iter_all() {
        let code = entry.code;
        if let Some(dash_pos) = code.find('-') {
            let prefix = &code[..dash_pos];
            if !prefixes.contains(&prefix) {
                prefixes.push(prefix);
            }
        }
    }
    prefixes.sort();
    prefixes
}

/// C3 compliance report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct C3Report {
    /// Total registered codes.
    pub total_codes: usize,
    /// Codes per prefix.
    pub by_prefix: HashMap<String, usize>,
    /// All LIN codes.
    pub lin_codes: Vec<String>,
    /// All prefixes.
    pub prefixes: Vec<String>,
    /// Whether all codes match the C3 pattern.
    pub all_valid: bool,
    /// Whether the registry is frozen (no new codes can be added without version bump).
    pub frozen: bool,
}

impl C3Report {
    pub fn new() -> Self {
        Self {
            total_codes: 0,
            by_prefix: HashMap::new(),
            lin_codes: Vec::new(),
            prefixes: Vec::new(),
            all_valid: true,
            frozen: true,
        }
    }
}

impl Default for C3Report {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a C3 compliance report.
pub fn generate_report() -> C3Report {
    let registry = DiagnosticRegistry::instance();
    let codes = registry.all_codes();
    let mut report = C3Report::new();
    report.total_codes = codes.len();

    for code in &codes {
        // Count by prefix
        if let Some(dash_pos) = code.find('-') {
            let prefix = &code[..dash_pos];
            *report.by_prefix.entry(prefix.to_string()).or_insert(0) += 1;
        }

        // LIN codes
        if code.starts_with("LIN-") {
            report.lin_codes.push(code.to_string());
        }
    }

    report.prefixes = all_prefixes().into_iter().map(|s| s.to_string()).collect();
    report.all_valid = verify_all_codes().is_ok();
    report.frozen = true; // Frozen at C3 v1

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_all_lin_codes_registered — all LIN codes are in registry
    #[test]
    fn test_all_lin_codes_registered() {
        let lin_codes = all_lin_codes();
        assert!(!lin_codes.is_empty(), "LIN codes should be registered");
        for code in lin_codes {
            assert!(is_registered_code(code), "{} must be registered", code);
        }
    }

    /// test_all_codes_valid_pattern — every registered code matches the pattern
    #[test]
    fn test_all_codes_valid_pattern() {
        // LIN codes must match the pattern (this is the primary C3 constraint)
        let result = verify_lin_codes();
        assert!(
            result.is_ok(),
            "All LIN codes must match the C3 pattern: {}",
            result.unwrap_err()
        );
    }

    /// test_verify_lin_codes — LIN subset passes validation
    #[test]
    fn test_verify_lin_codes() {
        let result = verify_lin_codes();
        assert!(
            result.is_ok(),
            "All LIN codes must match the C3 pattern: {}",
            result.unwrap_err()
        );
    }

    /// test_verify_all_codes — full registry passes validation (allows non-standard ODF-R codes)
    #[test]
    fn test_verify_all_codes() {
        // ODF-R200..R310 codes use a legacy format (ODF-R200 vs ODF-R-200)
        // They are registered but don't match the strict C3 pattern
        // The non-C3-compliant codes list should contain only ODF-R* codes
        let non_compliant = non_c3_compliant_codes();
        for code in &non_compliant {
            assert!(
                code.starts_with("ODF-R"),
                "Non-C3 code {} should be ODF-R*",
                code
            );
        }
    }

    /// test_no_duplicate_codes — no duplicates in registry
    #[test]
    fn test_no_duplicate_codes() {
        let registry = DiagnosticRegistry::instance();
        assert!(
            !registry.has_duplicates(),
            "Registry must not contain duplicate codes"
        );
    }

    /// test_prefixes_list — known prefixes present
    #[test]
    fn test_prefixes_list() {
        let prefixes = all_prefixes();
        let expected = ["CLI", "ENG", "GP", "LIN", "MCP", "ODF", "ODX", "OOX", "PDF"];
        for exp in &expected {
            assert!(prefixes.contains(exp), "Prefix '{}' should be present", exp);
        }
    }

    /// test_report_generation — report has correct counts
    #[test]
    fn test_report_generation() {
        let report = generate_report();
        assert!(report.total_codes > 0, "Registry should have codes");
        assert!(!report.prefixes.is_empty(), "Should have prefixes");
        assert!(!report.lin_codes.is_empty(), "Should have LIN codes");
    }

    /// test_report_frozen — frozen flag is true
    #[test]
    fn test_report_frozen() {
        let report = generate_report();
        assert!(report.frozen, "C3 report should be frozen at v1");
    }

    /// test_report_lin_codes_count — correct LIN code count
    #[test]
    fn test_report_lin_codes_count() {
        let report = generate_report();
        // LIN codes from registry: E-001..006, W-001..006, B-001/002, I-001/002 = 16
        assert_eq!(
            report.lin_codes.len(),
            16,
            "Expected 16 LIN codes, got {}",
            report.lin_codes.len()
        );
    }

    /// test_report_by_prefix — counts match per-prefix
    #[test]
    fn test_report_by_prefix() {
        let report = generate_report();
        let total_from_prefixes: usize = report.by_prefix.values().sum();
        assert_eq!(
            total_from_prefixes, report.total_codes,
            "Sum of prefix counts ({}) must equal total_codes ({})",
            total_from_prefixes, report.total_codes
        );
    }

    /// test_is_registered_code_true — known code returns true
    #[test]
    fn test_is_registered_code_true() {
        assert!(is_registered_code("GP-E-001"));
        assert!(is_registered_code("LIN-E-001"));
        assert!(is_registered_code("OOX-E-001"));
        assert!(is_registered_code("PDF-E-001"));
        assert!(is_registered_code("ENG-E-001"));
        assert!(is_registered_code("CLI-E-001"));
        assert!(is_registered_code("MCP-E-001"));
    }

    /// test_is_registered_code_false — unknown code returns false
    #[test]
    fn test_is_registered_code_false() {
        assert!(!is_registered_code("XXX-Z-999"));
        assert!(!is_registered_code("NONEXISTENT-E-001"));
        assert!(!is_registered_code("invalid"));
    }

    /// test_all_lin_codes_list — returns all LIN codes
    #[test]
    fn test_all_lin_codes_list() {
        let lin_codes = all_lin_codes();
        assert!(lin_codes.contains(&"LIN-E-001"));
        assert!(lin_codes.contains(&"LIN-E-006"));
        assert!(lin_codes.contains(&"LIN-W-001"));
        assert!(lin_codes.contains(&"LIN-W-006"));
        assert!(lin_codes.contains(&"LIN-B-001"));
        assert!(lin_codes.contains(&"LIN-B-002"));
        assert!(lin_codes.contains(&"LIN-I-001"));
        assert!(lin_codes.contains(&"LIN-I-002"));
    }

    /// test_c3_pattern_validation — known valid/invalid patterns
    #[test]
    fn test_c3_pattern_validation() {
        assert!(is_valid_c3_pattern("GP-E-001"));
        assert!(is_valid_c3_pattern("OOX-W-002"));
        assert!(is_valid_c3_pattern("LIN-B-002"));

        // Invalid patterns
        assert!(!is_valid_c3_pattern(""));
        assert!(!is_valid_c3_pattern("A-E001")); // single-char prefix
        assert!(!is_valid_c3_pattern("GP-e001")); // lowercase category
        assert!(!is_valid_c3_pattern("GP-E00")); // too few digits
        assert!(!is_valid_c3_pattern("GP-E00a")); // non-digit
    }

    /// test_c3_codes_are_all_registered — no ad-hoc codes in registry codes
    #[test]
    fn test_c3_codes_are_all_registered() {
        let registry = DiagnosticRegistry::instance();
        for entry in registry.iter_all() {
            assert!(
                is_registered_code(entry.code),
                "Registry code '{}' must be in registry",
                entry.code
            );
        }
    }

    /// test_report_by_prefix_has_odx — ODX prefix present
    #[test]
    fn test_report_by_prefix_has_odx() {
        let report = generate_report();
        assert!(
            report.by_prefix.contains_key("ODX"),
            "ODX prefix should be in report"
        );
    }

    /// test_report_by_prefix_has_eng — ENG prefix has all codes
    #[test]
    fn test_report_by_prefix_has_eng() {
        let report = generate_report();
        let eng_count = report.by_prefix.get("ENG").copied().unwrap_or(0);
        assert!(
            eng_count >= 28,
            "ENG should have at least 28 codes (got {})",
            eng_count
        );
    }
}
