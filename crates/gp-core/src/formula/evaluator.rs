// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula evaluator: evaluate parsed formula AST against a cell context.

use super::parser::{BinOp, FormulaNode, ParseError};

/// The result of evaluating a formula.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalResult {
    /// A numeric result.
    Number(f64),
    /// A text result.
    Text(String),
    /// A boolean result.
    Bool(bool),
    /// An error result.
    Error(String),
}

/// A cached cell value for lookup during evaluation.
#[derive(Debug, Clone)]
pub struct CachedCellValue {
    /// Numeric value, if present.
    pub numeric: Option<f64>,
    /// Text value, if present.
    pub text: Option<String>,
    /// Boolean value, if present.
    pub bool: Option<bool>,
}

/// The context trait for formula evaluation.
pub trait FormulaContext {
    /// Get the value of a cell reference like "A1".
    fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue>;
    /// Get values for a range like "A1:B5".
    fn get_range(&self, range: &str) -> Option<Vec<CachedCellValue>>;
}

/// Evaluate a parsed formula node against a cell context.
///
/// Returns None if the formula references cells not in the context
/// (the caller should use cached value instead).
pub fn evaluate(node: &FormulaNode, ctx: &dyn FormulaContext) -> Option<EvalResult> {
    eval_node(ctx, node)
}

/// Parse a formula string (without leading `=`) into an AST.
pub fn parse_formula(formula: &str) -> Result<FormulaNode, ParseError> {
    super::parser::parse_formula(formula)
}

fn eval_node(ctx: &dyn FormulaContext, node: &FormulaNode) -> Option<EvalResult> {
    match node {
        FormulaNode::Number(n) => Some(EvalResult::Number(*n)),
        FormulaNode::Text(t) => Some(EvalResult::Text(t.clone())),
        FormulaNode::Bool(b) => Some(EvalResult::Bool(*b)),
        FormulaNode::UnaryMinus(operand) => match eval_node(ctx, operand)? {
            EvalResult::Number(n) => Some(EvalResult::Number(-n)),
            _ => Some(EvalResult::Error("unary minus on non-numeric".to_string())),
        },
        FormulaNode::BinaryOp { op, left, right } => {
            let left_val = eval_node(ctx, left)?;
            let right_val = eval_node(ctx, right)?;
            eval_binop(op, &left_val, &right_val)
        }
        FormulaNode::CellRef(ref_name) => {
            let cell = ctx.get_cell(ref_name)?;
            Some(cached_to_eval(&cell))
        }
        FormulaNode::Range(range) => {
            let values = ctx.get_range(range)?;
            eval_range(&values)
        }
        FormulaNode::Call { name, args } => eval_call(ctx, name, args),
    }
}

fn eval_binop(op: &BinOp, left: &EvalResult, right: &EvalResult) -> Option<EvalResult> {
    match op {
        BinOp::Add => numeric_binop(left, right, |a, b| EvalResult::Number(a + b)),
        BinOp::Sub => numeric_binop(left, right, |a, b| EvalResult::Number(a - b)),
        BinOp::Mul => numeric_binop(left, right, |a, b| EvalResult::Number(a * b)),
        BinOp::Div => numeric_binop(left, right, |a, b| {
            if b == 0.0 {
                EvalResult::Error("divide by zero".to_string())
            } else {
                EvalResult::Number(a / b)
            }
        }),
        BinOp::Eq => eval_compare(left, right, |a, b| a == b),
        BinOp::Ne => eval_compare(left, right, |a, b| a != b),
        BinOp::Lt => eval_numeric_compare(left, right, |a, b| a < b),
        BinOp::Le => eval_numeric_compare(left, right, |a, b| a <= b),
        BinOp::Gt => eval_numeric_compare(left, right, |a, b| a > b),
        BinOp::Ge => eval_numeric_compare(left, right, |a, b| a >= b),
        BinOp::Concat => concat_values(left, right),
    }
}

fn numeric_binop(
    left: &EvalResult,
    right: &EvalResult,
    f: fn(f64, f64) -> EvalResult,
) -> Option<EvalResult> {
    let left_n = eval_result_to_numeric(left);
    let right_n = eval_result_to_numeric(right);
    Some(f(left_n, right_n))
}

fn eval_compare<F>(left: &EvalResult, right: &EvalResult, cmp: F) -> Option<EvalResult>
where
    F: Fn(f64, f64) -> bool,
{
    let left_n = eval_result_to_numeric(left);
    let right_n = eval_result_to_numeric(right);
    Some(EvalResult::Bool(cmp(left_n, right_n)))
}

fn eval_numeric_compare<F>(left: &EvalResult, right: &EvalResult, cmp: F) -> Option<EvalResult>
where
    F: Fn(f64, f64) -> bool,
{
    let left_n = eval_result_to_numeric(left);
    let right_n = eval_result_to_numeric(right);
    Some(EvalResult::Bool(cmp(left_n, right_n)))
}

fn concat_values(left: &EvalResult, right: &EvalResult) -> Option<EvalResult> {
    let left_s = eval_result_to_string(left);
    let right_s = eval_result_to_string(right);
    Some(EvalResult::Text(format!("{}{}", left_s, right_s)))
}

fn eval_result_to_numeric(val: &EvalResult) -> f64 {
    match val {
        EvalResult::Number(n) => *n,
        EvalResult::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        EvalResult::Text(t) => t.trim().parse::<f64>().unwrap_or(0.0),
        EvalResult::Error(_) => 0.0,
    }
}

fn cached_to_eval(cell: &CachedCellValue) -> EvalResult {
    if let Some(n) = cell.numeric {
        EvalResult::Number(n)
    } else if let Some(ref t) = cell.text {
        if t.eq_ignore_ascii_case("TRUE") {
            EvalResult::Bool(true)
        } else if t.eq_ignore_ascii_case("FALSE") {
            EvalResult::Bool(false)
        } else {
            t.trim()
                .parse::<f64>()
                .map(EvalResult::Number)
                .unwrap_or(EvalResult::Text(t.clone()))
        }
    } else if let Some(b) = cell.bool {
        EvalResult::Bool(b)
    } else {
        EvalResult::Error("empty cell".to_string())
    }
}

fn eval_range(values: &[CachedCellValue]) -> Option<EvalResult> {
    if values.is_empty() {
        return Some(EvalResult::Number(0.0));
    }
    let numeric: Vec<f64> = values.iter().filter_map(|v| v.numeric).collect();
    if !numeric.is_empty() {
        let sum: f64 = numeric.iter().sum();
        Some(EvalResult::Number(sum))
    } else {
        Some(EvalResult::Number(0.0))
    }
}

fn eval_call(ctx: &dyn FormulaContext, name: &str, args: &[FormulaNode]) -> Option<EvalResult> {
    let name_upper = name.to_uppercase();
    match name_upper.as_str() {
        "SUM" => eval_sum(ctx, args),
        "AVERAGE" | "AVG" => eval_average(ctx, args),
        "COUNT" => eval_count(ctx, args),
        "MIN" => eval_min(ctx, args),
        "MAX" => eval_max(ctx, args),
        "AND" => eval_and(ctx, args),
        "OR" => eval_or(ctx, args),
        "NOT" => eval_not(ctx, args),
        "IF" => eval_if(ctx, args),
        "CONCATENATE" => eval_concatenate(ctx, args),
        "IFERROR" => eval_iferror(ctx, args),
        _ => Some(EvalResult::Error(format!("unsupported function: {}", name))),
    }
}

fn collect_range_values(
    ctx: &dyn FormulaContext,
    node: &FormulaNode,
) -> Option<Vec<CachedCellValue>> {
    match node {
        FormulaNode::Range(range) => ctx.get_range(range),
        FormulaNode::CellRef(ref_name) => ctx.get_cell(ref_name).map(|v| vec![v]),
        _ => None,
    }
}

fn eval_sum(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Number(0.0));
    }
    let mut total: f64 = 0.0;
    for arg in args {
        if let Some(values) = collect_range_values(ctx, arg) {
            for v in &values {
                if let Some(n) = v.numeric {
                    total += n;
                }
            }
        } else {
            if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
                total += n;
            }
        }
    }
    Some(EvalResult::Number(total))
}

fn eval_average(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut sum: f64 = 0.0;
    let mut count: usize = 0;
    for arg in args {
        if let Some(values) = collect_range_values(ctx, arg) {
            for v in &values {
                if let Some(n) = v.numeric {
                    sum += n;
                    count += 1;
                }
            }
        } else {
            if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
                sum += n;
                count += 1;
            }
        }
    }
    if count == 0 {
        Some(EvalResult::Error("DIV/0".to_string()))
    } else {
        Some(EvalResult::Number(sum / count as f64))
    }
}

fn eval_count(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut count: usize = 0;
    for arg in args {
        if let Some(values) = collect_range_values(ctx, arg) {
            count += values.len();
        } else {
            eval_node(ctx, arg)?;
            count += 1;
        }
    }
    Some(EvalResult::Number(count as f64))
}

fn eval_min(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut min_val: Option<f64> = None;
    for arg in args {
        if let Some(values) = collect_range_values(ctx, arg) {
            for v in &values {
                if let Some(n) = v.numeric {
                    min_val = Some(match min_val {
                        Some(current) => current.min(n),
                        None => n,
                    });
                }
            }
        } else {
            if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
                min_val = Some(match min_val {
                    Some(current) => current.min(n),
                    None => n,
                });
            }
        }
    }
    min_val
        .map(EvalResult::Number)
        .or(Some(EvalResult::Number(f64::INFINITY)))
}

fn eval_max(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut max_val: Option<f64> = None;
    for arg in args {
        if let Some(values) = collect_range_values(ctx, arg) {
            for v in &values {
                if let Some(n) = v.numeric {
                    max_val = Some(match max_val {
                        Some(current) => current.max(n),
                        None => n,
                    });
                }
            }
        } else {
            if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
                max_val = Some(match max_val {
                    Some(current) => current.max(n),
                    None => n,
                });
            }
        }
    }
    max_val
        .map(EvalResult::Number)
        .or(Some(EvalResult::Number(f64::NEG_INFINITY)))
}

fn eval_and(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Bool(true));
    }
    let mut all_true = true;
    for arg in args {
        let result = eval_node(ctx, arg)?;
        if !is_truthy(&result) {
            all_true = false;
        }
    }
    Some(EvalResult::Bool(all_true))
}

fn eval_or(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Bool(false));
    }
    let mut any_true = false;
    for arg in args {
        let result = eval_node(ctx, arg)?;
        if is_truthy(&result) {
            any_true = true;
        }
    }
    Some(EvalResult::Bool(any_true))
}

fn eval_not(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Bool(true));
    }
    eval_node(ctx, &args[0]).map(|result| EvalResult::Bool(!is_truthy(&result)))
}

fn eval_if(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("IF requires at least 2 args".to_string()));
    }
    let condition = eval_node(ctx, &args[0])?;
    if is_truthy(&condition) {
        eval_node(ctx, &args[1])
    } else if args.len() > 2 {
        eval_node(ctx, &args[2])
    } else {
        Some(EvalResult::Bool(false))
    }
}

fn eval_concatenate(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut result = String::new();
    for arg in args {
        let eval = eval_node(ctx, arg)?;
        result.push_str(&eval_result_to_string(&eval));
    }
    Some(EvalResult::Text(result))
}

fn eval_iferror(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("IFERROR requires 2 args".to_string()));
    }
    match eval_node(ctx, &args[0]) {
        Some(EvalResult::Error(_)) => eval_node(ctx, &args[1]),
        other => other,
    }
}

fn is_truthy(val: &EvalResult) -> bool {
    match val {
        EvalResult::Number(n) => *n != 0.0,
        EvalResult::Text(t) => !t.is_empty(),
        EvalResult::Bool(b) => *b,
        EvalResult::Error(_) => false,
    }
}

fn eval_result_to_string(val: &EvalResult) -> String {
    match val {
        EvalResult::Number(n) => n.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestContext {
        cells: std::collections::HashMap<String, CachedCellValue>,
    }

    impl TestContext {
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

        fn with_text(mut self, ref_name: &str, text: &str) -> Self {
            self.cells.insert(
                ref_name.to_string(),
                CachedCellValue {
                    numeric: None,
                    text: Some(text.to_string()),
                    bool: None,
                },
            );
            self
        }
    }

    impl FormulaContext for TestContext {
        fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
            self.cells.get(ref_name).cloned()
        }

        fn get_range(&self, range: &str) -> Option<Vec<CachedCellValue>> {
            if range == "A1:A5" {
                return Some(vec![
                    CachedCellValue {
                        numeric: Some(10.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(20.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(30.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(40.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(50.0),
                        text: None,
                        bool: None,
                    },
                ]);
            }
            if range == "B1:B3" {
                return Some(vec![
                    CachedCellValue {
                        numeric: Some(1.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(2.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(3.0),
                        text: None,
                        bool: None,
                    },
                ]);
            }
            if range == "A1:A3" {
                return Some(vec![
                    CachedCellValue {
                        numeric: Some(1.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(2.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(3.0),
                        text: None,
                        bool: None,
                    },
                ]);
            }
            if range == "A1:A10" {
                return Some(
                    (1..=10)
                        .map(|i| CachedCellValue {
                            numeric: Some(i as f64),
                            text: None,
                            bool: None,
                        })
                        .collect(),
                );
            }
            None
        }
    }

    fn evaluate_formula(formula: &str) -> Option<EvalResult> {
        let ctx = TestContext::default();
        let node = parse_formula(formula).ok()?;
        evaluate(&node, &ctx)
    }

    #[test]
    fn test_evaluate_number() {
        let node = parse_formula("42").unwrap();
        let ctx = TestContext::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_evaluate_text() {
        let node = parse_formula(r#""hello""#).unwrap();
        let ctx = TestContext::default();
        assert_eq!(
            evaluate(&node, &ctx),
            Some(EvalResult::Text("hello".to_string()))
        );
    }

    #[test]
    fn test_evaluate_addition() {
        assert_eq!(evaluate_formula("1+2"), Some(EvalResult::Number(3.0)));
    }

    #[test]
    fn test_evaluate_subtraction() {
        assert_eq!(evaluate_formula("5-3"), Some(EvalResult::Number(2.0)));
    }

    #[test]
    fn test_evaluate_multiplication() {
        assert_eq!(evaluate_formula("4*3"), Some(EvalResult::Number(12.0)));
    }

    #[test]
    fn test_evaluate_division() {
        assert_eq!(evaluate_formula("10/2"), Some(EvalResult::Number(5.0)));
    }

    #[test]
    fn test_evaluate_unary_minus() {
        assert_eq!(evaluate_formula("-5"), Some(EvalResult::Number(-5.0)));
    }

    #[test]
    fn test_evaluate_parentheses() {
        assert_eq!(evaluate_formula("(1+2)*3"), Some(EvalResult::Number(9.0)));
    }

    #[test]
    fn test_evaluate_sum() {
        assert_eq!(
            evaluate_formula("SUM(A1:A5)"),
            Some(EvalResult::Number(150.0))
        );
    }

    #[test]
    fn test_evaluate_average() {
        assert_eq!(
            evaluate_formula("AVERAGE(A1:A5)"),
            Some(EvalResult::Number(30.0))
        );
    }

    #[test]
    fn test_evaluate_count() {
        assert_eq!(
            evaluate_formula("COUNT(A1:A5)"),
            Some(EvalResult::Number(5.0))
        );
    }

    #[test]
    fn test_evaluate_min() {
        assert_eq!(
            evaluate_formula("MIN(A1:A5)"),
            Some(EvalResult::Number(10.0))
        );
    }

    #[test]
    fn test_evaluate_max() {
        assert_eq!(
            evaluate_formula("MAX(A1:A5)"),
            Some(EvalResult::Number(50.0))
        );
    }

    #[test]
    fn test_evaluate_and() {
        assert_eq!(
            evaluate_formula("AND(TRUE,TRUE)"),
            Some(EvalResult::Bool(true))
        );
        assert_eq!(
            evaluate_formula("AND(TRUE,FALSE)"),
            Some(EvalResult::Bool(false))
        );
    }

    #[test]
    fn test_evaluate_or() {
        assert_eq!(
            evaluate_formula("OR(TRUE,FALSE)"),
            Some(EvalResult::Bool(true))
        );
        assert_eq!(
            evaluate_formula("OR(FALSE,FALSE)"),
            Some(EvalResult::Bool(false))
        );
    }

    #[test]
    fn test_evaluate_not() {
        assert_eq!(evaluate_formula("NOT(TRUE)"), Some(EvalResult::Bool(false)));
        assert_eq!(evaluate_formula("NOT(FALSE)"), Some(EvalResult::Bool(true)));
    }

    #[test]
    fn test_evaluate_if_true() {
        assert_eq!(
            evaluate_formula("IF(TRUE,1,0)"),
            Some(EvalResult::Number(1.0))
        );
    }

    #[test]
    fn test_evaluate_if_false() {
        assert_eq!(
            evaluate_formula("IF(FALSE,1,0)"),
            Some(EvalResult::Number(0.0))
        );
    }

    #[test]
    fn test_evaluate_if_with_comparison() {
        let ctx = TestContext::default().with_value("B1", 1.0);
        let node = parse_formula("IF(B1>0,1,0)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
    }

    #[test]
    fn test_evaluate_concatenate() {
        let ctx = TestContext::default().with_text("A1", "hello");
        let node = parse_formula(r#"CONCATENATE(A1," ",B1)"#).unwrap();
        let ctx = ctx.with_text("B1", "world");
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Text("hello world".to_string())));
    }

    #[test]
    fn test_evaluate_iferror() {
        assert_eq!(
            evaluate_formula("IFERROR(1/0,0)"),
            Some(EvalResult::Number(0.0))
        );
    }

    #[test]
    fn test_evaluate_cell_ref() {
        let ctx = TestContext::default().with_value("A1", 42.0);
        let node = parse_formula("A1").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_evaluate_cell_ref_missing() {
        let ctx = TestContext::default();
        let node = parse_formula("A1").unwrap();
        assert_eq!(evaluate(&node, &ctx), None);
    }

    #[test]
    fn test_evaluate_boolean() {
        let node = parse_formula("TRUE").unwrap();
        let ctx = TestContext::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Bool(true)));
    }

    #[test]
    fn test_evaluate_comparison() {
        assert_eq!(evaluate_formula("5>3"), Some(EvalResult::Bool(true)));
        assert_eq!(evaluate_formula("5<3"), Some(EvalResult::Bool(false)));
        assert_eq!(evaluate_formula("5=5"), Some(EvalResult::Bool(true)));
        assert_eq!(evaluate_formula("5<=5"), Some(EvalResult::Bool(true)));
        assert_eq!(evaluate_formula("5>=6"), Some(EvalResult::Bool(false)));
    }

    #[test]
    fn test_evaluate_complex() {
        let node = parse_formula("SUM(A1:A3)*2+IF(B1>0,B1,0)").unwrap();
        let ctx = TestContext::default().with_value("B1", 5.0);
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(17.0)));
    }

    #[test]
    fn test_evaluate_avg_alias() {
        assert_eq!(
            evaluate_formula("AVG(A1:A5)"),
            Some(EvalResult::Number(30.0))
        );
    }

    #[test]
    fn test_evaluate_divide_by_zero() {
        assert_eq!(
            evaluate_formula("1/0"),
            Some(EvalResult::Error("divide by zero".to_string()))
        );
    }

    #[test]
    fn test_evaluate_truthey_rules() {
        let node = parse_formula("IF(0,1,0)").unwrap();
        let ctx = TestContext::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(0.0)));

        let node = parse_formula("IF(1,1,0)").unwrap();
        let ctx = TestContext::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));

        let node = parse_formula(r#"IF("",0,1)"#).unwrap();
        let ctx = TestContext::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));

        let node = parse_formula(r#"IF("hello",1,0)"#).unwrap();
        let ctx = TestContext::default();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
    }

    #[test]
    fn test_evaluate_and_with_values() {
        let ctx = TestContext::default()
            .with_value("B1", 1.0)
            .with_value("C2", 2.0);
        let node = parse_formula("AND(B1,C2)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Bool(true)));
    }

    #[test]
    fn test_evaluate_or_with_values() {
        let ctx = TestContext::default()
            .with_value("B1", 0.0)
            .with_value("C2", 2.0);
        let node = parse_formula("OR(B1,C2)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Bool(true)));
    }
}
