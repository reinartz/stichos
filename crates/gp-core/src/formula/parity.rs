// SPDX-License-Identifier: MIT OR Apache-2.0
//! Cached-vs-computed parity lint for formula evaluation.
//!
//! Compares cached cell values against computed formula results to detect stale caches
//! or refused formulas.

use super::evaluator::EvalResult;
use super::refuse::GpFCode;

/// Result of a cached-vs-computed parity check.
#[derive(Debug, Clone, PartialEq)]
pub enum ParityResult {
    /// Computed value matches cached value.
    Match,
    /// Computed value differs from cached value (stale cache).
    Mismatch {
        /// The cached value string.
        cached: String,
        /// The computed value string.
        computed: String,
    },
    /// No cached value exists.
    NoCache,
    /// Formula was refused (GP-F0xx) — use cached value.
    Refused(GpFCode),
}

/// Check parity between a cached value and a computed formula result.
///
/// Returns `ParityResult` describing the relationship.
pub fn check_parity(cached_value: Option<&str>, computed: &EvalResult) -> ParityResult {
    let computed_str = eval_result_to_string(computed);

    match cached_value {
        None => ParityResult::NoCache,
        Some(cached) => {
            let cached_canonical = canonicalize_cached(cached);
            if cached_canonical == computed_str {
                ParityResult::Match
            } else {
                ParityResult::Mismatch {
                    cached: cached.to_string(),
                    computed: computed_str,
                }
            }
        }
    }
}

/// Check parity with a refused formula.
pub fn parity_refused(code: GpFCode) -> ParityResult {
    ParityResult::Refused(code)
}

/// Convert an EvalResult to a string representation for comparison with cached values.
pub fn eval_result_to_string(result: &EvalResult) -> String {
    match result {
        EvalResult::Number(n) => canonicalize_number(*n).to_string(),
        EvalResult::Text(t) => t.clone(),
        EvalResult::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        EvalResult::Error(e) => format!("#ERROR! {}", e),
    }
}

/// Canonicalize a cached value string for comparison.
fn canonicalize_cached(value: &str) -> String {
    let trimmed = value.trim();

    // Try to parse as a number
    if let Ok(n) = trimmed.parse::<f64>() {
        return canonicalize_number(n);
    }

    // Boolean check
    if trimmed.eq_ignore_ascii_case("TRUE") {
        return "TRUE".to_string();
    }
    if trimmed.eq_ignore_ascii_case("FALSE") {
        return "FALSE".to_string();
    }

    // Return as-is for text
    trimmed.to_string()
}

/// Canonicalize a number for comparison: remove trailing zeros, handle special values.
fn canonicalize_number(n: f64) -> String {
    // Handle special values
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_positive() {
            return "INF".to_string();
        }
        return "-INF".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }

    // Format and strip trailing zeros
    let formatted = format!("{}", n);

    // Handle scientific notation for very large/small numbers
    if formatted.contains('e') || formatted.contains('E') {
        return formatted;
    }

    // Strip trailing zeros after decimal point
    if formatted.contains('.') {
        let mut result = formatted;
        while result.ends_with('0') && result.contains('.') {
            result.pop();
        }
        if result.ends_with('.') {
            result.push('0');
        }
        result
    } else {
        formatted
    }
}

/// Check if two EvalResults are approximately equal (for floating-point comparison).
pub fn eval_results_approx_eq(a: &EvalResult, b: &EvalResult, tol: f64) -> bool {
    match (a, b) {
        (EvalResult::Number(x), EvalResult::Number(y)) => (x - y).abs() <= tol,
        (EvalResult::Bool(x), EvalResult::Bool(y)) => x == y,
        (EvalResult::Text(x), EvalResult::Text(y)) => x == y,
        (EvalResult::Error(x), EvalResult::Error(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_parity_match() {
        let result = EvalResult::Number(42.0);
        assert_eq!(check_parity(Some("42"), &result), ParityResult::Match);
    }

    #[test]
    fn test_parity_mismatch() {
        let result = EvalResult::Number(42.0);
        assert_eq!(
            check_parity(Some("100"), &result),
            ParityResult::Mismatch {
                cached: "100".to_string(),
                computed: "42".to_string(),
            }
        );
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

    #[test]
    fn test_eval_result_to_string_number() {
        assert_eq!(eval_result_to_string(&EvalResult::Number(42.0)), "42");
        assert_eq!(eval_result_to_string(&EvalResult::Number(3.14)), "3.14");
    }

    #[test]
    fn test_eval_result_to_string_text() {
        assert_eq!(
            eval_result_to_string(&EvalResult::Text("hello".to_string())),
            "hello"
        );
    }

    #[test]
    fn test_eval_result_to_string_bool() {
        assert_eq!(eval_result_to_string(&EvalResult::Bool(true)), "TRUE");
        assert_eq!(eval_result_to_string(&EvalResult::Bool(false)), "FALSE");
    }

    #[test]
    fn test_eval_result_to_string_error() {
        assert_eq!(
            eval_result_to_string(&EvalResult::Error("div/0".to_string())),
            "#ERROR! div/0"
        );
    }

    #[test]
    fn test_canonicalize_number() {
        assert_eq!(canonicalize_number(42.0), "42");
        assert_eq!(canonicalize_number(3.14), "3.14");
        assert_eq!(canonicalize_number(1.0), "1");
        assert_eq!(canonicalize_number(0.0), "0");
        assert_eq!(canonicalize_number(10.0), "10");
        assert_eq!(canonicalize_number(1.50), "1.5");
    }

    #[test]
    fn test_canonicalize_cached_number() {
        assert_eq!(canonicalize_cached("42"), "42");
        assert_eq!(canonicalize_cached("3.14"), "3.14");
        assert_eq!(canonicalize_cached("  10  "), "10");
    }

    #[test]
    fn test_canonicalize_cached_bool() {
        assert_eq!(canonicalize_cached("TRUE"), "TRUE");
        assert_eq!(canonicalize_cached("FALSE"), "FALSE");
        assert_eq!(canonicalize_cached("true"), "TRUE");
        assert_eq!(canonicalize_cached("false"), "FALSE");
    }

    #[test]
    fn test_canonicalize_cached_text() {
        assert_eq!(canonicalize_cached("hello world"), "hello world");
        assert_eq!(canonicalize_cached("  trimmed  "), "trimmed");
    }

    #[test]
    fn test_eval_results_approx_eq() {
        assert!(eval_results_approx_eq(
            &EvalResult::Number(1.0000000001),
            &EvalResult::Number(1.0),
            1e-9
        ));
        assert!(!eval_results_approx_eq(
            &EvalResult::Number(1.0),
            &EvalResult::Number(2.0),
            1e-9
        ));
        assert!(eval_results_approx_eq(
            &EvalResult::Bool(true),
            &EvalResult::Bool(true),
            1e-9
        ));
    }

    #[test]
    fn test_parity_match_text() {
        let result = EvalResult::Text("hello".to_string());
        assert_eq!(check_parity(Some("hello"), &result), ParityResult::Match);
    }

    #[test]
    fn test_parity_mismatch_text() {
        let result = EvalResult::Text("world".to_string());
        assert_eq!(
            check_parity(Some("hello"), &result),
            ParityResult::Mismatch {
                cached: "hello".to_string(),
                computed: "world".to_string(),
            }
        );
    }
}
