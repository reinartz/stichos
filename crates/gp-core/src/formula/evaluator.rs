// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula evaluator: evaluate parsed formula AST against a cell context.

use super::parser::{BinOp, FormulaNode, ParseError};
use chrono::{Datelike, NaiveDate};

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
    /// Get the value at a specific row/col in a grid. Returns None by default
    /// so existing contexts remain compatible; implement this for 2D grid access.
    fn get_cell_at(
        &self,
        _sheet: Option<&str>,
        _row: usize,
        _col: usize,
    ) -> Option<CachedCellValue> {
        None
    }
    /// Resolve a named range (e.g., "RevenueTotal") to a cached cell value.
    /// Returns None by default so existing contexts remain compatible.
    fn get_named_range(&self, _name: &str) -> Option<CachedCellValue> {
        None
    }
    /// Resolve a 3-D reference (e.g., Sheet1:Sheet3!A1) to values across sheets.
    /// Returns None by default — implement for multi-sheet support.
    fn get_cell_at_3d(
        &self,
        _sheet_start: &str,
        _sheet_end: &str,
        _cell: &str,
    ) -> Option<Vec<CachedCellValue>> {
        None
    }
    /// Resolve a structured table reference (e.g., [SalesAmount] or [Table1[SalesAmount]]).
    /// Returns None by default — implement for table support.
    fn get_structured_ref(&self, _table: &str, _column: &str) -> Option<Vec<CachedCellValue>> {
        None
    }
    /// Returns the Excel serial date number for the "analysis date" (used by TODAY/NOW).
    fn analysis_date(&self) -> f64 {
        45672.0 // 2025-01-15 as Excel serial date
    }
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
            // Try cell ref first, then named range, then structured ref
            if let Some(cell) = ctx.get_cell(ref_name) {
                return Some(cached_to_eval(&cell));
            }
            if let Some(cell) = ctx.get_named_range(ref_name) {
                return Some(cached_to_eval(&cell));
            }
            // Try structured ref (table column reference)
            if ctx.get_structured_ref("", ref_name).is_some() {
                return ctx
                    .get_structured_ref("", ref_name)
                    .and_then(|vals| eval_range(&vals));
            }
            None
        }
        FormulaNode::Range(range) => {
            let values = ctx.get_range(range)?;
            eval_range(&values)
        }
        FormulaNode::CellRef3D {
            sheet_start,
            sheet_end,
            cell,
        } => match ctx.get_cell_at_3d(sheet_start, sheet_end, cell) {
            Some(values) => eval_range(&values),
            None => Some(EvalResult::Error(
                "GP-F-022: 3-D reference not supported".to_string(),
            )),
        },
        FormulaNode::StructuredRef { table, column } => {
            match ctx.get_structured_ref(table, column) {
                Some(values) => eval_range(&values),
                None => Some(EvalResult::Error(
                    "GP-F-023: structured reference not supported".to_string(),
                )),
            }
        }
        FormulaNode::Call { name, args } => eval_call(ctx, name, args),
        FormulaNode::Pow(left, right) => {
            let left_val = eval_node(ctx, left)?;
            let right_val = eval_node(ctx, right)?;
            eval_pow(&left_val, &right_val)
        }
        FormulaNode::Percent(inner) => {
            let val = eval_node(ctx, inner)?;
            eval_percent(&val)
        }
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

fn eval_pow(left: &EvalResult, right: &EvalResult) -> Option<EvalResult> {
    let left_n = eval_result_to_numeric(left);
    let right_n = eval_result_to_numeric(right);
    let result = left_n.powf(right_n);
    Some(EvalResult::Number(result))
}

fn eval_percent(val: &EvalResult) -> Option<EvalResult> {
    let n = eval_result_to_numeric(val);
    Some(EvalResult::Number(n * 0.01))
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
        // Math functions
        "ABS" => eval_unary_num(ctx, args, |v| v.abs()),
        "ROUND" => eval_binary_num(ctx, args, |v, d| {
            let decimals = d.round() as i32;
            let factor = 10_f64.powi(decimals);
            (v * factor).round() / factor
        }),
        "ROUNDUP" => eval_binary_num(ctx, args, |v, d| {
            let decimals = d.round() as i32;
            let factor = 10_f64.powi(decimals);
            if v > 0.0 {
                (v * factor).ceil() / factor
            } else {
                (v * factor).floor() / factor
            }
        }),
        "ROUNDDOWN" => eval_binary_num(ctx, args, |v, d| {
            let decimals = d.round() as i32;
            let factor = 10_f64.powi(decimals);
            if v > 0.0 {
                (v * factor).floor() / factor
            } else {
                (v * factor).ceil() / factor
            }
        }),
        "MOD" => eval_binary_num(ctx, args, |a, b| a - b * (a / b).floor()),
        "INT" => eval_unary_num(ctx, args, |v| v.floor()),
        "POWER" => eval_binary_num(ctx, args, |base, exp| base.powf(exp)),
        // Text functions
        "LEN" => {
            let val = match eval_node(ctx, &args[0])? {
                EvalResult::Number(n) => n.to_string(),
                EvalResult::Text(t) => t,
                _ => return Some(EvalResult::Error("non-numeric/text argument".to_string())),
            };
            Some(EvalResult::Number(val.len() as f64))
        }
        "TRIM" => eval_unary_text(ctx, args, |v| v.trim().to_string()),
        "UPPER" => eval_unary_text(ctx, args, |v| v.to_uppercase()),
        "LOWER" => eval_unary_text(ctx, args, |v| v.to_lowercase()),
        "REPT" => eval_binary_text(ctx, args, |text, count| {
            let n = count as usize;
            text.repeat(n)
        }),
        "CONCAT" => eval_concatenate(ctx, args),
        "SUBSTITUTE" => eval_substitute(ctx, args),
        "LEFT" => eval_left_fn(ctx, args),
        "RIGHT" => eval_right_fn(ctx, args),
        "MID" => eval_mid_fn(ctx, args),
        // Date/Time functions
        "TODAY" => Some(EvalResult::Number(ctx.analysis_date())),
        "NOW" => Some(EvalResult::Number(ctx.analysis_date())),
        "YEAR" => eval_year(ctx, args),
        "MONTH" => eval_month(ctx, args),
        "DAY" => eval_day_fn(ctx, args),
        "DATEDIF" => eval_datedif(ctx, args),
        "EOMONTH" => eval_eomonth(ctx, args),
        // Statistical functions
        "COUNTA" => eval_counta(ctx, args),
        "COUNTBLANK" => eval_countblank(ctx, args),
        "STDEV" | "STDEV_S" => eval_sample_stddev_fn(ctx, args),
        "STDEVP" | "STDEV_P" => eval_pop_stddev_fn(ctx, args),
        "VAR" | "VAR_S" => eval_sample_variance_fn(ctx, args),
        "VARP" | "VAR_P" => eval_pop_variance_fn(ctx, args),
        "RANK" | "RANK_EQ" => eval_rank_fn(ctx, args),
        // Error handling / type checking
        "ISERROR" => eval_iserror(ctx, args),
        "ISNUMBER" => eval_isnumber(ctx, args),
        "ISTEXT" => eval_istext(ctx, args),
        "IFNA" => eval_ifna(ctx, args),
        // Lookup functions
        "VLOOKUP" => eval_vlookup(ctx, args),
        "HLOOKUP" => eval_hlookup(ctx, args),
        "MATCH" => eval_match(ctx, args),
        "INDEX" => eval_index(ctx, args),
        // Financial functions (F1)
        "NPV" => eval_npv(ctx, args),
        "IRR" => eval_irr(ctx, args),
        "PMT" => eval_pmt(ctx, args),
        "PV" => eval_pv(ctx, args),
        "FV" => eval_fv(ctx, args),
        "RATE" => eval_rate(ctx, args),
        "NPER" => eval_nper(ctx, args),
        // Advanced date/time functions (F2)
        "WORKDAY" => eval_workday(ctx, args),
        "NETWORKDAYS" => eval_networkdays(ctx, args),
        "WEEKNUM" => eval_weeknum(ctx, args),
        "DATEVALUE" => eval_datevalue(ctx, args),
        // Advanced text functions (F3)
        "TEXT" => eval_text_format(ctx, args),
        "TEXTJOIN" => eval_textjoin(ctx, args),
        "SEARCH" => eval_search_fn(ctx, args),
        "FIND" => eval_find_fn(ctx, args),
        // Extended logical functions (F4)
        "XOR" => eval_xor(ctx, args),
        "IFS" => eval_ifs(ctx, args),
        "SWITCH" => eval_switch(ctx, args),
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

// Helper: evaluate a single argument as f64
fn eval_fn_arg(ctx: &dyn FormulaContext, node: &FormulaNode) -> Option<f64> {
    match eval_node(ctx, node)? {
        EvalResult::Number(n) => Some(n),
        EvalResult::Text(t) => t.parse::<f64>().ok(),
        _ => None,
    }
}

// Helper: evaluate a single argument as string
fn eval_fn_text(ctx: &dyn FormulaContext, node: &FormulaNode) -> Option<String> {
    Some(eval_result_to_string(&eval_node(ctx, node)?))
}

fn eval_unary_num(
    ctx: &dyn FormulaContext,
    args: &[FormulaNode],
    body: impl FnOnce(f64) -> f64,
) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("requires 1 arg".to_string()));
    }
    let val = match eval_node(ctx, &args[0])? {
        EvalResult::Number(n) => n,
        _ => return Some(EvalResult::Error("non-numeric argument".to_string())),
    };
    Some(EvalResult::Number(body(val)))
}

fn eval_binary_num(
    ctx: &dyn FormulaContext,
    args: &[FormulaNode],
    body: impl FnOnce(f64, f64) -> f64,
) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("requires 2 args".to_string()));
    }
    let a = match eval_node(ctx, &args[0])? {
        EvalResult::Number(n) => n,
        _ => return Some(EvalResult::Error("non-numeric argument".to_string())),
    };
    let b = match eval_node(ctx, &args[1])? {
        EvalResult::Number(n) => n,
        _ => return Some(EvalResult::Error("non-numeric argument".to_string())),
    };
    Some(EvalResult::Number(body(a, b)))
}

fn eval_unary_text(
    ctx: &dyn FormulaContext,
    args: &[FormulaNode],
    body: impl FnOnce(String) -> String,
) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("requires 1 arg".to_string()));
    }
    let val = eval_result_to_string(&eval_node(ctx, &args[0])?);
    Some(EvalResult::Text(body(val)))
}

fn eval_binary_text(
    ctx: &dyn FormulaContext,
    args: &[FormulaNode],
    body: impl FnOnce(String, f64) -> String,
) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("requires 2 args".to_string()));
    }
    let a = eval_result_to_string(&eval_node(ctx, &args[0])?);
    let b = match eval_node(ctx, &args[1])? {
        EvalResult::Number(n) => n,
        _ => return Some(EvalResult::Error("non-numeric argument".to_string())),
    };
    Some(EvalResult::Text(body(a, b)))
}

fn eval_substitute(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 3 {
        return Some(EvalResult::Error(
            "SUBSTITUTE requires 3 or 4 args".to_string(),
        ));
    }
    let text = eval_fn_text(ctx, &args[0])?;
    let old = eval_fn_text(ctx, &args[1])?;
    let new = eval_fn_text(ctx, &args[2])?;
    if args.len() == 3 {
        let replaced = text.replace(&old, &new);
        Some(EvalResult::Text(replaced))
    } else if args.len() == 4 {
        let n = match eval_fn_arg(ctx, &args[3]) {
            Some(n) => n as usize,
            None => return Some(EvalResult::Error("invalid occurrence number".to_string())),
        };
        let mut result = String::new();
        let mut remaining = text.as_str();
        let mut found = 0usize;
        while let Some(pos) = remaining.find(&old) {
            if found + 1 == n {
                result.push_str(&remaining[..pos]);
                result.push_str(&new);
                remaining = &remaining[pos + old.len()..];
                break;
            }
            found += 1;
            result.push_str(&remaining[..pos + old.len()]);
            remaining = &remaining[pos + old.len()..];
        }
        result.push_str(remaining);
        Some(EvalResult::Text(result))
    } else {
        Some(EvalResult::Error(
            "SUBSTITUTE requires 3 or 4 args".to_string(),
        ))
    }
}

fn eval_left_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "LEFT requires at least 1 arg".to_string(),
        ));
    }
    let text = eval_fn_text(ctx, &args[0])?;
    let chars: Vec<char> = text.chars().collect();
    let n = if args.len() > 1 {
        match eval_fn_arg(ctx, &args[1]) {
            Some(n) => n as usize,
            None => return Some(EvalResult::Error("invalid count".to_string())),
        }
    } else {
        chars.len()
    };
    let result: String = chars[..n.min(chars.len())].iter().collect();
    Some(EvalResult::Text(result))
}

fn eval_right_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "RIGHT requires at least 1 arg".to_string(),
        ));
    }
    let text = eval_fn_text(ctx, &args[0])?;
    let chars: Vec<char> = text.chars().collect();
    let n = if args.len() > 1 {
        match eval_fn_arg(ctx, &args[1]) {
            Some(n) => n as usize,
            None => return Some(EvalResult::Error("invalid count".to_string())),
        }
    } else {
        chars.len()
    };
    let start = chars.len().saturating_sub(n);
    let result: String = chars[start..].iter().collect();
    Some(EvalResult::Text(result))
}

fn eval_mid_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 3 {
        return Some(EvalResult::Error("MID requires 3 args".to_string()));
    }
    let text = eval_fn_text(ctx, &args[0])?;
    let start = match eval_fn_arg(ctx, &args[1]) {
        Some(s) => s as usize,
        None => return Some(EvalResult::Error("invalid start".to_string())),
    };
    let len = match eval_fn_arg(ctx, &args[2]) {
        Some(l) => l as usize,
        None => return Some(EvalResult::Error("invalid length".to_string())),
    };
    let chars: Vec<char> = text.chars().collect();
    let start = start.saturating_sub(1).min(chars.len());
    let end = (start + len).min(chars.len());
    let result: String = chars[start..end].iter().collect();
    Some(EvalResult::Text(result))
}

// ===========================================================================
// Date/Time helpers
// ===========================================================================

/// Days in a month (handles leap years for February).
#[allow(dead_code)]
fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Convert Excel serial date to (year, month, day).
/// Excel serial 1 = 1900-01-01.
/// Excel has a 1900 leap year bug: serial 60 = 1900-02-29 (doesn't exist).
/// For serial >= 60, the actual date is 1 day after chrono's computation.
fn serial_to_ymd(serial: f64) -> Option<(i32, i32, i32)> {
    if serial < 1.0 || serial >= 2958466.0 {
        return None;
    }
    let serial = serial as i64;
    // Excel epoch is 1899-12-31, but with the 1900 leap year bug:
    // dates >= March 1, 1900 (serial >= 60) are offset by +1
    let adjusted_serial = if serial >= 60 { serial - 1 } else { serial };
    let epoch = NaiveDate::from_ymd_opt(1899, 12, 31)?;
    let date = epoch.checked_add_signed(chrono::TimeDelta::days(adjusted_serial))?;
    Some((date.year(), date.month() as i32, date.day() as i32))
}

/// Convert (year, month, day) to Excel serial date.
/// Excel serial 1 = 1900-01-01.
/// Reverses serial_to_ymd, accounting for the 1900 leap year bug.
fn ymd_to_serial(year: i32, month: i32, day: i32) -> f64 {
    let epoch = NaiveDate::from_ymd_opt(1899, 12, 31).expect("1899-12-31 is a valid date");
    let date = NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("invalid date");
    let days = date.signed_duration_since(epoch).num_days();
    // For dates >= March 1, 1900, we added -1 in serial_to_ymd, so add +1 here
    if year > 1900 || (year == 1900 && month >= 3) {
        (days + 1) as f64
    } else {
        days as f64
    }
}

fn eval_year(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("YEAR requires 1 arg".to_string()));
    }
    let date_val = eval_node(ctx, &args[0])?;
    match date_val {
        EvalResult::Number(serial) => {
            serial_to_ymd(serial).map(|(y, _, _)| EvalResult::Number(y as f64))
        }
        _ => Some(EvalResult::Error("YEAR requires a date number".to_string())),
    }
}

fn eval_month(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("MONTH requires 1 arg".to_string()));
    }
    let date_val = eval_node(ctx, &args[0])?;
    match date_val {
        EvalResult::Number(serial) => {
            serial_to_ymd(serial).map(|(_, m, _)| EvalResult::Number(m as f64))
        }
        _ => Some(EvalResult::Error(
            "MONTH requires a date number".to_string(),
        )),
    }
}

fn eval_day_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("DAY requires 1 arg".to_string()));
    }
    let date_val = eval_node(ctx, &args[0])?;
    match date_val {
        EvalResult::Number(serial) => {
            serial_to_ymd(serial).map(|(_, _, d)| EvalResult::Number(d as f64))
        }
        _ => Some(EvalResult::Error("DAY requires a date number".to_string())),
    }
}

fn eval_datedif(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 3 {
        return Some(EvalResult::Error("DATEDIF requires 3 args".to_string()));
    }
    let d1 = eval_node(ctx, &args[0])?;
    let d2 = eval_node(ctx, &args[1])?;
    let unit = eval_node(ctx, &args[2])?;
    let start = match d1 {
        EvalResult::Number(n) => n,
        _ => return Some(EvalResult::Error("DATEDIF d1 must be numeric".to_string())),
    };
    let end = match d2 {
        EvalResult::Number(n) => n,
        _ => return Some(EvalResult::Error("DATEDIF d2 must be numeric".to_string())),
    };
    let unit_str = match &unit {
        EvalResult::Text(t) => t.as_str(),
        _ => return Some(EvalResult::Error("DATEDIF unit must be text".to_string())),
    };
    let (sy, sm, sd) = serial_to_ymd(start)?;
    let (ey, em, ed) = serial_to_ymd(end)?;
    match unit_str.to_lowercase().as_str() {
        "d" => {
            let days_start = ymd_to_serial(sy, sm, sd);
            let days_end = ymd_to_serial(ey, em, ed);
            Some(EvalResult::Number((days_end - days_start).floor()))
        }
        "m" => {
            let years_diff = ey - sy;
            let mut months = years_diff * 12 + (em - sm);
            if ed < sd {
                months -= 1;
            }
            Some(EvalResult::Number(months as f64))
        }
        "y" => {
            let mut years = ey - sy;
            if (em < sm) || (em == sm && ed < sd) {
                years -= 1;
            }
            Some(EvalResult::Number(years as f64))
        }
        _ => Some(EvalResult::Error(format!(
            "DATEDIF unknown unit: {}",
            unit_str
        ))),
    }
}

fn eval_eomonth(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("EOMONTH requires 2 args".to_string()));
    }
    let start = eval_node(ctx, &args[0])?;
    let months_arg = eval_node(ctx, &args[1])?;
    let serial = match start {
        EvalResult::Number(n) => n,
        _ => {
            return Some(EvalResult::Error(
                "EOMONTH start must be numeric".to_string(),
            ))
        }
    };
    let months = match months_arg {
        EvalResult::Number(n) => n,
        _ => {
            return Some(EvalResult::Error(
                "EOMONTH months must be numeric".to_string(),
            ))
        }
    };
    let (y, m, _) = serial_to_ymd(serial)?;
    let total_months = m + (months as i32) - 1;
    let new_year = y + total_months / 12;
    let mut new_month = total_months % 12 + 1;
    if new_month <= 0 {
        new_month += 12;
    }
    if new_month > 12 {
        new_month -= 12;
    }
    let days_in_month = month_days(new_year, new_month);
    Some(EvalResult::Number(ymd_to_serial(
        new_year,
        new_month,
        days_in_month,
    )))
}

fn month_days(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

// ===========================================================================
// Statistical functions
// ===========================================================================

/// Collect numeric values from arguments: ranges + evaluated literals.
fn collect_number_values(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<Vec<f64>> {
    let mut values: Vec<f64> = Vec::new();
    for arg in args {
        // Try range first
        if let FormulaNode::Range(range) = arg {
            if let Some(cells) = ctx.get_range(range) {
                for cell in &cells {
                    if let Some(n) = cell.numeric {
                        values.push(n);
                    }
                }
            }
        } else if let FormulaNode::CellRef(ref_name) = arg {
            if let Some(cell) = ctx.get_cell(ref_name) {
                if let Some(n) = cell.numeric {
                    values.push(n);
                }
            }
        } else if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
            values.push(n);
        }
    }
    Some(values)
}

fn eval_counta(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut count: f64 = 0.0;
    for arg in args {
        match eval_node(ctx, arg) {
            Some(EvalResult::Number(_)) => count += 1.0,
            Some(EvalResult::Text(t)) if !t.is_empty() => count += 1.0,
            Some(EvalResult::Bool(b)) if b => count += 1.0,
            _ => {}
        }
    }
    Some(EvalResult::Number(count))
}

fn eval_countblank(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut count: f64 = 0.0;
    for arg in args {
        match eval_node(ctx, arg) {
            Some(EvalResult::Text(t)) if t.is_empty() => count += 1.0,
            Some(EvalResult::Number(n)) if n.is_nan() => count += 1.0,
            Some(EvalResult::Error(_)) => count += 1.0,
            _ => {}
        }
    }
    Some(EvalResult::Number(count))
}

fn eval_sample_stddev_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let values = collect_number_values(ctx, args)?;
    if values.is_empty() {
        return Some(EvalResult::Error(
            "STDEV requires at least 1 value".to_string(),
        ));
    }
    let n = values.len() as f64;
    if n < 2.0 {
        return Some(EvalResult::Error(
            "STDEV requires at least 2 values".to_string(),
        ));
    }
    let mean: f64 = values.iter().sum::<f64>() / n;
    let sum_sq: f64 = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    Some(EvalResult::Number((sum_sq / (n - 1.0)).sqrt()))
}

fn eval_pop_stddev_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let values = collect_number_values(ctx, args)?;
    if values.is_empty() {
        return Some(EvalResult::Error(
            "STDEVP requires at least 1 value".to_string(),
        ));
    }
    let n = values.len() as f64;
    let mean: f64 = values.iter().sum::<f64>() / n;
    let sum_sq: f64 = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    Some(EvalResult::Number((sum_sq / n).sqrt()))
}

fn eval_sample_variance_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let values = collect_number_values(ctx, args)?;
    if values.is_empty() {
        return Some(EvalResult::Error(
            "VAR requires at least 1 value".to_string(),
        ));
    }
    let n = values.len() as f64;
    if n < 2.0 {
        return Some(EvalResult::Error(
            "VAR requires at least 2 values".to_string(),
        ));
    }
    let mean: f64 = values.iter().sum::<f64>() / n;
    let sum_sq: f64 = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    Some(EvalResult::Number(sum_sq / (n - 1.0)))
}

fn eval_pop_variance_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let values = collect_number_values(ctx, args)?;
    if values.is_empty() {
        return Some(EvalResult::Error(
            "VARP requires at least 1 value".to_string(),
        ));
    }
    let n = values.len() as f64;
    let mean: f64 = values.iter().sum::<f64>() / n;
    let sum_sq: f64 = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    Some(EvalResult::Number(sum_sq / n))
}

fn eval_rank_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "RANK requires at least 2 args".to_string(),
        ));
    }
    let value = eval_node(ctx, &args[0])?;
    let target = match value {
        EvalResult::Number(n) => n,
        _ => {
            return Some(EvalResult::Error(
                "RANK first arg must be numeric".to_string(),
            ))
        }
    };
    // Get range values
    let mut range_values: Vec<f64> = Vec::new();
    let range_arg = &args[1];
    if let FormulaNode::Range(range) = range_arg {
        if let Some(cells) = ctx.get_range(range) {
            for cell in &cells {
                if let Some(n) = cell.numeric {
                    range_values.push(n);
                }
            }
        }
    } else if let FormulaNode::CellRef(ref_name) = range_arg {
        if let Some(cell) = ctx.get_cell(ref_name) {
            if let Some(n) = cell.numeric {
                range_values.push(n);
            }
        }
    } else if let Some(EvalResult::Number(n)) = eval_node(ctx, range_arg) {
        range_values.push(n);
    }
    let order = if args.len() > 2 {
        match eval_node(ctx, &args[2]) {
            Some(EvalResult::Number(n)) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let mut rank = 1.0;
    for v in &range_values {
        if order == 0.0 {
            // Descending (default): rank increases for values greater than target
            if *v > target {
                rank += 1.0;
            }
        } else {
            // Ascending: rank increases for values less than target
            if *v < target {
                rank += 1.0;
            }
        }
    }
    Some(EvalResult::Number(rank))
}

// ===========================================================================
// Error handling / type checking
// ===========================================================================

fn eval_iserror(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("ISERROR requires 1 arg".to_string()));
    }
    match eval_node(ctx, &args[0]) {
        Some(EvalResult::Error(_)) => Some(EvalResult::Number(1.0)),
        _ => Some(EvalResult::Number(0.0)),
    }
}

fn eval_isnumber(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("ISNUMBER requires 1 arg".to_string()));
    }
    match eval_node(ctx, &args[0]) {
        Some(EvalResult::Number(n)) if n.is_finite() => Some(EvalResult::Number(1.0)),
        _ => Some(EvalResult::Number(0.0)),
    }
}

fn eval_istext(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("ISTEXT requires 1 arg".to_string()));
    }
    match eval_node(ctx, &args[0]) {
        Some(EvalResult::Text(_)) => Some(EvalResult::Number(1.0)),
        _ => Some(EvalResult::Number(0.0)),
    }
}

fn eval_ifna(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("IFNA requires 2 args".to_string()));
    }
    let val = eval_node(ctx, &args[0])?;
    match &val {
        EvalResult::Number(n) if n.is_nan() || *n == -1.0 => eval_node(ctx, &args[1]),
        EvalResult::Error(e) if e.to_uppercase().contains("N/A") || e.to_uppercase() == "NA" => {
            eval_node(ctx, &args[1])
        }
        _ => Some(val),
    }
}

// ===========================================================================
// Lookup functions
// ===========================================================================

/// Helper: evaluate the first argument as a LookupValue for comparison.
fn eval_lookup_value(
    ctx: &dyn FormulaContext,
    node: &FormulaNode,
) -> Option<(LookupValue, EvalResult)> {
    let result = eval_node(ctx, node)?;
    let lv = match &result {
        EvalResult::Number(n) => LookupValue::Number(*n),
        EvalResult::Text(t) => LookupValue::Text(t.clone()),
        EvalResult::Bool(b) => LookupValue::Bool(*b),
        EvalResult::Error(_) => return Some((LookupValue::Error, result)),
    };
    Some((lv, result))
}

/// Helper: extract range values and column count for lookup functions.
/// Returns (flat_values, num_cols).
fn eval_range_for_lookup(
    ctx: &dyn FormulaContext,
    node: &FormulaNode,
) -> Option<(Vec<CachedCellValue>, usize)> {
    match node {
        FormulaNode::Range(range) => {
            let values = ctx.get_range(range)?;
            // Infer columns from the range string format "Col1Row1:Col2Row2"
            let cols = parse_range_cols(range)?;
            Some((values, cols))
        }
        FormulaNode::CellRef(ref_name) => {
            // Treat single cell ref as 1x1 range
            ctx.get_cell(ref_name).map(|cell| (vec![cell], 1))
        }
        _ => {
            // Try evaluating the expression as a range
            eval_node(ctx, node)?;
            Some((vec![], 0))
        }
    }
}

/// Parse the column count from a range string like "A1:C5".
fn parse_range_cols(range: &str) -> Option<usize> {
    let colon = range.find(':')?;
    let start: &str = &range[..colon];
    let end: &str = &range[colon + 1..];
    let start_col = parse_column_letter(start)?;
    let end_col = parse_column_letter(end)?;
    if end_col < start_col {
        return Some(1);
    }
    Some(end_col - start_col + 1)
}

/// Parse the column number from a column prefix like "A", "Z", "AA".
fn parse_column_letter(s: &str) -> Option<usize> {
    let mut col = 0usize;
    for ch in s.chars() {
        if !ch.is_ascii_alphabetic() {
            break;
        }
        col = col * 26 + (ch as usize - 'A' as usize + 1);
    }
    if col == 0 {
        return None;
    }
    Some(col)
}

/// Get the value at position (row, col) in the flat grid, returning its evaluated result.
/// row/col are 1-based.
fn grid_value_at(
    values: &[CachedCellValue],
    cols: usize,
    row: usize,
    col: usize,
) -> Option<EvalResult> {
    let idx = (row - 1) * cols + (col - 1);
    if idx >= values.len() {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    Some(cached_to_eval(&values[idx]))
}

/// Check if a lookup value matches the target.
/// Returns true if values are considered equal.
fn lookup_values_equal(lv: &LookupValue, cv: &EvalResult) -> bool {
    match (lv, cv) {
        (LookupValue::Number(a), EvalResult::Number(b)) => *a == *b,
        (LookupValue::Text(a), EvalResult::Text(b)) => a.eq_ignore_ascii_case(b),
        (LookupValue::Bool(a), EvalResult::Bool(b)) => a == b,
        (LookupValue::Text(a), EvalResult::Number(b)) => a.trim() == b.to_string().trim(),
        (LookupValue::Number(a), EvalResult::Text(b)) => a.to_string().trim() == b.trim(),
        _ => false,
    }
}

/// Compare lookup value with a grid value, returning -1, 0, or 1.
fn lookup_compare(lv: &LookupValue, result: &EvalResult) -> i32 {
    let a = match lv {
        LookupValue::Number(n) => *n,
        LookupValue::Text(t) => t.trim().parse::<f64>().unwrap_or(0.0),
        LookupValue::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        LookupValue::Error => 0.0,
    };
    let b = match result {
        EvalResult::Number(n) => *n,
        EvalResult::Text(t) => t.trim().parse::<f64>().unwrap_or(0.0),
        EvalResult::Bool(b) if *b => 1.0,
        EvalResult::Bool(_) => 0.0,
        _ => 0.0,
    };
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal) as i32
}

/// Search in a 1D slice for the lookup value.
/// Returns 1-based index or None if not found.
fn search_1d(
    values: &[CachedCellValue],
    lookup_val: &LookupValue,
    match_type: MatchType,
) -> Option<usize> {
    match match_type {
        MatchType::Exact => {
            for (i, cell) in values.iter().enumerate() {
                let result = cached_to_eval(cell);
                if lookup_values_equal(lookup_val, &result) {
                    return Some(i + 1); // 1-based
                }
            }
            None
        }
        MatchType::NextSmaller => {
            // Find largest value <= lookup_val
            // For match_type=1, values must be sorted ascending
            let mut best: Option<usize> = None;
            for (i, cell) in values.iter().enumerate() {
                let result = cached_to_eval(cell);
                let cmp = lookup_compare(lookup_val, &result);
                if cmp >= 0 {
                    best = Some(i + 1);
                } else if cmp < 0 {
                    break;
                }
            }
            best
        }
        MatchType::NextLarger => {
            // For descending order: find smallest value >= lookup_val
            // (i.e., the last value >= lookup in descending order)
            let mut best: Option<usize> = None;
            for (i, cell) in values.iter().enumerate() {
                let result = cached_to_eval(cell);
                let cmp = lookup_compare(lookup_val, &result);
                if cmp <= 0 {
                    // lookup <= cell, i.e., cell >= lookup
                    best = Some(i + 1);
                } else {
                    // cell < lookup, no more values will be >= lookup in descending order
                    break;
                }
            }
            best
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum LookupValue {
    Number(f64),
    Text(String),
    Bool(bool),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchType {
    Exact,
    NextSmaller,
    NextLarger,
}

fn eval_vlookup(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "VLOOKUP requires at least 3 args".to_string(),
        ));
    }
    let (lookup_val, _eval_result) = eval_lookup_value(ctx, &args[0])?;
    let (values, cols) = eval_range_for_lookup(ctx, &args[1])?;
    if values.is_empty() {
        return Some(EvalResult::Error("#N/A".to_string()));
    }
    let col_index = match eval_node(ctx, &args[2])? {
        EvalResult::Number(n) if n > 0.0 => n as usize,
        _ => return Some(EvalResult::Error("#VALUE!".to_string())),
    };
    if col_index < 1 || col_index > cols {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    let exact_match = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n == 0.0,
            EvalResult::Bool(b) => !b,
            EvalResult::Text(t) => t.eq_ignore_ascii_case("FALSE"),
            _ => true,
        }
    } else {
        true
    };
    let match_type = if exact_match {
        MatchType::Exact
    } else {
        MatchType::NextSmaller
    };
    let first_col_values: Vec<CachedCellValue> = (0..values.len())
        .step_by(cols)
        .filter(|&i| i < values.len())
        .map(|i| values[i].clone())
        .collect();
    let pos = match search_1d(&first_col_values, &lookup_val, match_type) {
        Some(p) => p,
        None => return Some(EvalResult::Error("#N/A".to_string())),
    };
    let row = pos; // pos is already 1-based row index
    grid_value_at(&values, cols, row, col_index)
}

fn eval_hlookup(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "HLOOKUP requires at least 3 args".to_string(),
        ));
    }
    let (lookup_val, _eval_result) = eval_lookup_value(ctx, &args[0])?;
    let (values, cols) = eval_range_for_lookup(ctx, &args[1])?;
    if values.is_empty() {
        return Some(EvalResult::Error("#N/A".to_string()));
    }
    let rows = values.len().checked_div(cols).unwrap_or(0);
    if rows == 0 {
        return Some(EvalResult::Error("#N/A".to_string()));
    }
    let row_index = match eval_node(ctx, &args[2])? {
        EvalResult::Number(n) if n > 0.0 => n as usize,
        _ => return Some(EvalResult::Error("#VALUE!".to_string())),
    };
    if row_index == 0 || row_index > rows {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    let exact_match = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n == 0.0,
            EvalResult::Bool(b) => !b,
            EvalResult::Text(t) => t.eq_ignore_ascii_case("FALSE"),
            _ => true,
        }
    } else {
        true
    };
    let match_type = if exact_match {
        MatchType::Exact
    } else {
        MatchType::NextSmaller
    };
    let first_row_values: Vec<CachedCellValue> = values.iter().take(cols).cloned().collect();
    let pos = match search_1d(&first_row_values, &lookup_val, match_type) {
        Some(p) => p,
        None => return Some(EvalResult::Error("#N/A".to_string())),
    };
    let col = pos; // pos is already 1-based column index
    grid_value_at(&values, cols, row_index, col)
}

fn eval_match(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "MATCH requires at least 2 args".to_string(),
        ));
    }
    let (lookup_val, _eval_result) = eval_lookup_value(ctx, &args[0])?;
    // Extract range values for the 1D search
    let values = match &args[1] {
        FormulaNode::Range(range) => ctx.get_range(range).unwrap_or_default(),
        FormulaNode::CellRef(ref_name) => {
            ctx.get_cell(ref_name).map(|c| vec![c]).unwrap_or_default()
        }
        _ => return Some(EvalResult::Error("#VALUE!".to_string())),
    };
    if values.is_empty() {
        return Some(EvalResult::Error("#N/A".to_string()));
    }
    let match_type = if args.len() > 2 {
        match eval_node(ctx, &args[2])? {
            EvalResult::Number(1.0) => MatchType::NextSmaller,
            EvalResult::Number(-1.0) => MatchType::NextLarger,
            EvalResult::Number(0.0) => MatchType::Exact,
            EvalResult::Number(n) if n > 0.0 => MatchType::NextSmaller,
            EvalResult::Number(n) if n < -1.0 => MatchType::NextLarger,
            _ => MatchType::Exact,
        }
    } else {
        MatchType::NextSmaller
    };
    search_1d(&values, &lookup_val, match_type)
        .map(|pos| EvalResult::Number(pos as f64))
        .or(Some(EvalResult::Error("#N/A".to_string())))
}

fn eval_index(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "INDEX requires at least 2 args".to_string(),
        ));
    }
    let (values, cols) = eval_range_for_lookup(ctx, &args[0])?;
    if values.is_empty() {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    let rows = values.len().checked_div(cols).unwrap_or(1);
    let row_arg = &args[1];
    let explicit_col: Option<usize> = if args.len() > 2 {
        Some(match eval_node(ctx, &args[2])? {
            EvalResult::Number(n) if n > 0.0 => n as usize,
            _ => return Some(EvalResult::Error("#VALUE!".to_string())),
        })
    } else {
        None
    };
    let row_arg_val = match eval_node(ctx, row_arg)? {
        EvalResult::Number(n) if n > 0.0 => n as usize,
        _ => return Some(EvalResult::Error("#VALUE!".to_string())),
    };
    // Determine (final_row, final_col) based on array shape and args
    let (final_row, final_col) = if let Some(c) = explicit_col {
        // Both row and col explicitly provided
        (row_arg_val, c)
    } else if rows == 1 && cols > 1 {
        // Single row array: row arg is actually the column index
        (1, row_arg_val)
    } else if cols == 1 {
        // Single column array: col defaults to 1
        (row_arg_val, 1)
    } else {
        return Some(EvalResult::Error("#VALUE!".to_string()));
    };
    // Validate bounds
    if final_row < 1 || final_row > rows {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    if final_col < 1 || final_col > cols {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    let idx = (final_row - 1) * cols + (final_col - 1);
    if idx >= values.len() {
        return Some(EvalResult::Error("#REF!".to_string()));
    }
    Some(cached_to_eval(&values[idx]))
}

// ===========================================================================
// F1: Financial functions
// ===========================================================================

fn eval_npv(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("NPV requires at least 1 arg".to_string()));
    }
    let rate = eval_fn_arg(ctx, &args[0])?;
    if args.len() < 2 {
        return Some(EvalResult::Error(
            "NPV requires at least 2 args".to_string(),
        ));
    }
    let mut sum = 0.0;
    for (i, arg) in args[1..].iter().enumerate() {
        let val = match eval_node(ctx, arg)? {
            EvalResult::Number(n) => n,
            EvalResult::Text(t) => t.trim().parse::<f64>().ok()?,
            _ => return Some(EvalResult::Error("NPV: non-numeric value".to_string())),
        };
        let power = math_pow(1.0 + rate, (i + 1) as i32);
        sum += val / power;
    }
    Some(EvalResult::Number(sum))
}

fn eval_irr(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("IRR requires at least 1 arg".to_string()));
    }
    let guess = if args.len() > 1 {
        match eval_node(ctx, &args[1])? {
            EvalResult::Number(n) => n,
            _ => 0.1,
        }
    } else {
        0.1
    };
    let values: Vec<f64> = args
        .iter()
        .filter_map(|a| match eval_node(ctx, a)? {
            EvalResult::Number(n) => Some(n),
            EvalResult::Text(t) => t.trim().parse::<f64>().ok(),
            _ => None,
        })
        .collect();
    if values.is_empty() {
        return Some(EvalResult::Error("IRR: no numeric values".to_string()));
    }
    // Newton-Raphson iteration
    let rate = newton_irr(&values, guess);
    Some(EvalResult::Number(rate))
}

fn newton_irr(cashflows: &[f64], guess: f64) -> f64 {
    let mut rate = guess;
    let max_iter = 100;
    let tolerance = 1e-8;
    for _ in 0..max_iter {
        let mut npv = 0.0;
        let mut dpv = 0.0;
        for (i, &cf) in cashflows.iter().enumerate() {
            let p = math_pow(1.0 + rate, i as i32);
            if p.abs() < 1e-15 {
                break;
            }
            npv += cf / p;
            dpv -= (i as f64) * cf / (p * (1.0 + rate));
        }
        if (dpv).abs() < 1e-15 {
            break;
        }
        let new_rate = rate - npv / dpv;
        if (new_rate - rate).abs() < tolerance {
            return new_rate;
        }
        rate = new_rate;
    }
    rate
}

fn eval_pmt(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("PMT requires at least 1 arg".to_string()));
    }
    let rate = eval_fn_arg(ctx, &args[0])?;
    if args.len() < 2 {
        return Some(EvalResult::Error(
            "PMT requires at least 2 args".to_string(),
        ));
    }
    let nper = eval_fn_arg(ctx, &args[1])?;
    if args.len() < 3 {
        return Some(EvalResult::Error(
            "PMT requires at least 3 args".to_string(),
        ));
    }
    let pv = eval_fn_arg(ctx, &args[2])?;
    let fv = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let type_flag = if args.len() > 4 {
        match eval_node(ctx, &args[4])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    eval_pmt_fn(rate, nper, pv, fv, type_flag)
}

fn eval_pmt_fn(rate: f64, nper: f64, pv: f64, fv: f64, type_flag: f64) -> Option<EvalResult> {
    if nper == 0.0 {
        return Some(EvalResult::Number(-pv - fv));
    }
    if (rate - 0.0).abs() < 1e-12 {
        // Zero rate: -(pv + fv) / nper
        return Some(EvalResult::Number(-(pv + fv) / nper));
    }
    let r = rate;
    let n = nper as i32;
    let factor = math_pow(1.0 + r, n);
    let pmt_val = -(r * (pv * factor + fv)) / ((1.0 + r * type_flag) * (factor - 1.0));
    Some(EvalResult::Number(pmt_val))
}

fn eval_pv(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("PV requires at least 1 arg".to_string()));
    }
    let rate = eval_fn_arg(ctx, &args[0])?;
    if args.len() < 2 {
        return Some(EvalResult::Error("PV requires at least 2 args".to_string()));
    }
    let nper = eval_fn_arg(ctx, &args[1])?;
    if args.len() < 3 {
        return Some(EvalResult::Error("PV requires at least 3 args".to_string()));
    }
    let pmt = eval_fn_arg(ctx, &args[2])?;
    let fv = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let type_flag = if args.len() > 4 {
        match eval_node(ctx, &args[4])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    eval_pv_fn(rate, nper, pmt, fv, type_flag)
}

fn eval_pv_fn(rate: f64, nper: f64, pmt: f64, fv: f64, type_flag: f64) -> Option<EvalResult> {
    if (rate - 0.0).abs() < 1e-12 {
        return Some(EvalResult::Number(-(pmt * nper + fv)));
    }
    let r = rate;
    let n = nper as i32;
    let factor = math_pow(1.0 + r, n);
    let pv_val = ((pmt * (1.0 + r * type_flag) * (1.0 - 1.0 / factor)) + fv / factor) / r;
    Some(EvalResult::Number(pv_val))
}

fn eval_fv(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("FV requires at least 1 arg".to_string()));
    }
    let rate = eval_fn_arg(ctx, &args[0])?;
    if args.len() < 2 {
        return Some(EvalResult::Error("FV requires at least 2 args".to_string()));
    }
    let nper = eval_fn_arg(ctx, &args[1])?;
    if args.len() < 3 {
        return Some(EvalResult::Error("FV requires at least 3 args".to_string()));
    }
    let pmt = eval_fn_arg(ctx, &args[2])?;
    let pv = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let type_flag = if args.len() > 4 {
        match eval_node(ctx, &args[4])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    eval_fv_fn(rate, nper, pmt, pv, type_flag)
}

fn eval_fv_fn(rate: f64, nper: f64, pmt: f64, pv: f64, type_flag: f64) -> Option<EvalResult> {
    if (rate - 0.0).abs() < 1e-12 {
        return Some(EvalResult::Number(-(pv * nper + pmt)));
    }
    let r = rate;
    let n = nper as i32;
    let factor = math_pow(1.0 + r, n);
    let fv_val = -(pv * factor + pmt * (1.0 + r * type_flag) * (factor - 1.0) / r);
    Some(EvalResult::Number(fv_val))
}

fn eval_rate(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 3 {
        return Some(EvalResult::Error(
            "RATE requires at least 3 args".to_string(),
        ));
    }
    let nper = eval_fn_arg(ctx, &args[0])?;
    let pmt = eval_fn_arg(ctx, &args[1])?;
    let pv = eval_fn_arg(ctx, &args[2])?;
    let fv = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let type_flag = if args.len() > 4 {
        match eval_node(ctx, &args[4])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let guess = if args.len() > 5 {
        match eval_node(ctx, &args[5])? {
            EvalResult::Number(n) => n,
            _ => 0.1,
        }
    } else {
        0.1
    };
    let rate = newton_rate(nper, pmt, pv, fv, type_flag, guess);
    Some(EvalResult::Number(rate))
}

fn newton_rate(nper: f64, pmt: f64, pv: f64, fv: f64, type_flag: f64, guess: f64) -> f64 {
    let mut rate = guess;
    let max_iter = 100;
    let tolerance = 1e-10;
    for _ in 0..max_iter {
        let (f, df) = fv_at_rate(nper, pmt, pv, fv, type_flag, rate);
        if df.abs() < 1e-15 {
            break;
        }
        let new_rate = rate - f / df;
        if (new_rate - rate).abs() < tolerance {
            return new_rate;
        }
        rate = new_rate;
    }
    rate
}

fn fv_at_rate(nper: f64, pmt: f64, pv: f64, fv: f64, type_flag: f64, rate: f64) -> (f64, f64) {
    let r = rate;
    let n = nper as i32;
    let one_plus_r = 1.0 + r;
    let factor = if r > -1.0 && n >= 0 {
        math_pow(one_plus_r, n)
    } else if n < 0 {
        let p = math_pow(one_plus_r, -n);
        if p.abs() > 1e-15 {
            1.0 / p
        } else {
            0.0
        }
    } else {
        1.0
    };
    // FV = -PV * (1+r)^n - PMT * (1 + r*type) * ((1+r)^n - 1) / r
    let fv_calc = if r.abs() > 1e-12 {
        -(pv * factor + pmt * (1.0 + r * type_flag) * (factor - 1.0) / r)
    } else {
        -(pv * factor + pmt * (1.0 + r * type_flag) * n as f64)
    };
    let fv_err = fv_calc - fv;
    // Analytical derivative of FV w.r.t. rate
    // dFV/dr = PV * n * (1+r)^(n-1) + ...
    // Simplified: use numerical derivative safely
    let dp = 1e-7;
    let factor_plus = if r + dp > -1.0 && n >= 0 {
        math_pow(1.0 + r + dp, n)
    } else {
        0.0
    };
    let fv_plus = if (r + dp).abs() > 1e-12 {
        -(pv * factor_plus + pmt * (1.0 + (r + dp) * type_flag) * (factor_plus - 1.0) / (r + dp))
    } else {
        -(pv * factor_plus + pmt * (1.0 + (r + dp) * type_flag) * n as f64)
    };
    let df = (fv_plus - fv_err) / dp;
    (fv_err, df)
}

fn eval_nper(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 3 {
        return Some(EvalResult::Error(
            "NPER requires at least 3 args".to_string(),
        ));
    }
    let rate = eval_fn_arg(ctx, &args[0])?;
    let pmt = eval_fn_arg(ctx, &args[1])?;
    let pv = eval_fn_arg(ctx, &args[2])?;
    let fv = if args.len() > 3 {
        match eval_node(ctx, &args[3])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let type_flag = if args.len() > 4 {
        match eval_node(ctx, &args[4])? {
            EvalResult::Number(n) => n,
            _ => 0.0,
        }
    } else {
        0.0
    };
    eval_nper_fn(rate, pmt, pv, fv, type_flag)
}

fn eval_nper_fn(rate: f64, pmt: f64, pv: f64, fv: f64, type_flag: f64) -> Option<EvalResult> {
    if (rate - 0.0).abs() < 1e-12 {
        let denom = -(pmt + fv / f64::MAX.min((pv - fv).abs().max(1e-10)));
        if denom.abs() < 1e-15 {
            return Some(EvalResult::Error("NPER: division by zero".to_string()));
        }
        return Some(EvalResult::Number(-(pv + fv) / pmt));
    }
    let r = rate;
    let denominator = pv * r + pmt * (1.0 + r * type_flag);
    if denominator.abs() < 1e-15 {
        return Some(EvalResult::Error("NPER: division by zero".to_string()));
    }
    let n = -((1.0 + r * pv / (fv + pmt * (1.0 + r * type_flag))).ln() / (1.0 + r).ln());
    Some(EvalResult::Number(n))
}

// ===========================================================================
// F2: Advanced date/time functions
// ===========================================================================

fn eval_workday(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "WORKDAY requires at least 2 args".to_string(),
        ));
    }
    let start_serial = eval_node(ctx, &args[0])?;
    let start = match start_serial {
        EvalResult::Number(n) => n,
        EvalResult::Text(t) => t.trim().parse::<f64>().ok()?,
        _ => return Some(EvalResult::Error("WORKDAY: invalid start date".to_string())),
    };
    let days = eval_fn_arg(ctx, &args[1])?;
    let days_int = days as i64;
    // Collect holidays
    let holidays: Vec<i64> = if args.len() > 2 {
        let mut result = Vec::new();
        for arg in &args[2..] {
            if let FormulaNode::Range(range) = arg {
                if let Some(cells) = ctx.get_range(range) {
                    for cell in &cells {
                        if let Some(n) = cell.numeric {
                            result.push(n as i64);
                        }
                    }
                }
            } else if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
                result.push(n as i64);
            }
        }
        result
    } else {
        Vec::new()
    };

    let (y, m, d) = serial_to_ymd(start)?;
    let step = if days_int >= 0 { 1i64 } else { -1 };
    let mut current = NaiveDate::from_ymd_opt(y, m as u32, d as u32)?;
    // Skip past the start date - WORKDAY adds working days FROM the start, not including it
    current = match current.checked_add_signed(chrono::TimeDelta::days(step)) {
        Some(d) => d,
        None => return Some(EvalResult::Error("WORKDAY: date overflow".to_string())),
    };
    let mut count = 0i64;
    let target = days_int.abs();
    let max_days = target * 3;
    let mut day = 0i64;
    while day < max_days {
        let dow = current.weekday();
        let holiday_serial =
            ymd_to_serial(current.year(), current.month() as i32, current.day() as i32);
        let is_holiday = holidays.contains(&(holiday_serial as i64));
        if dow != chrono::Weekday::Sat && dow != chrono::Weekday::Sun && !is_holiday {
            count += step;
            if count.abs() == target {
                return Some(EvalResult::Number(holiday_serial));
            }
        }
        current = match current.checked_add_signed(chrono::TimeDelta::days(step)) {
            Some(d) => d,
            None => break,
        };
        day += 1;
    }
    Some(EvalResult::Number(ymd_to_serial(
        current.year(),
        current.month() as i32,
        current.day() as i32,
    )))
}

fn eval_networkdays(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error(
            "NETWORKDAYS requires at least 2 args".to_string(),
        ));
    }
    let start = eval_node(ctx, &args[0])?;
    let start_serial = match start {
        EvalResult::Number(n) => n,
        EvalResult::Text(t) => t.trim().parse::<f64>().ok()?,
        _ => {
            return Some(EvalResult::Error(
                "NETWORKDAYS: invalid start date".to_string(),
            ))
        }
    };
    let end = eval_node(ctx, &args[1])?;
    let end_serial = match end {
        EvalResult::Number(n) => n,
        EvalResult::Text(t) => t.trim().parse::<f64>().ok()?,
        _ => {
            return Some(EvalResult::Error(
                "NETWORKDAYS: invalid end date".to_string(),
            ))
        }
    };

    let holidays: Vec<i64> = if args.len() > 2 {
        let mut result = Vec::new();
        for arg in &args[2..] {
            if let FormulaNode::Range(range) = arg {
                if let Some(cells) = ctx.get_range(range) {
                    for cell in &cells {
                        if let Some(n) = cell.numeric {
                            result.push(n as i64);
                        }
                    }
                }
            } else if let Some(EvalResult::Number(n)) = eval_node(ctx, arg) {
                result.push(n as i64);
            }
        }
        result
    } else {
        Vec::new()
    };

    let (sy, sm, sd) = serial_to_ymd(start_serial)?;
    let (ey, em, ed) = serial_to_ymd(end_serial)?;
    let start_date = NaiveDate::from_ymd_opt(sy, sm as u32, sd as u32)?;
    let end_date = NaiveDate::from_ymd_opt(ey, em as u32, ed as u32)?;

    let total_days = (end_date - start_date).num_days();
    if total_days < 0 {
        return Some(EvalResult::Error(
            "NETWORKDAYS: end date before start date".to_string(),
        ));
    }

    let mut count = 0i64;
    let mut current = start_date;
    for _ in 0..=total_days {
        let dow = current.weekday();
        let is_holiday = holidays.contains(
            &(ymd_to_serial(current.year(), current.month() as i32, current.day() as i32) as i64),
        );
        if dow != chrono::Weekday::Sat && dow != chrono::Weekday::Sun && !is_holiday {
            count += 1;
        }
        current = match current.checked_add_signed(chrono::TimeDelta::days(1)) {
            Some(d) => d,
            None => break,
        };
    }
    Some(EvalResult::Number(count as f64))
}

fn eval_weeknum(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "WEEKNUM requires at least 1 arg".to_string(),
        ));
    }
    let date_serial = eval_node(ctx, &args[0])?;
    let serial = match date_serial {
        EvalResult::Number(n) => n,
        EvalResult::Text(t) => t.trim().parse::<f64>().ok()?,
        _ => return Some(EvalResult::Error("WEEKNUM: invalid date".to_string())),
    };
    let (y, m, d) = serial_to_ymd(serial)?;
    let date = NaiveDate::from_ymd_opt(y, m as u32, d as u32)?;
    // ISO week: Monday is first day, week 1 contains Jan 4
    let iso_week = date.iso_week();
    Some(EvalResult::Number(iso_week.week() as f64))
}

fn eval_datevalue(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error("DATEVALUE requires 1 arg".to_string()));
    }
    let date_text = eval_result_to_string(&eval_node(ctx, &args[0])?);
    // Parse "YYYY-MM-DD" format
    let trimmed = date_text.trim();
    if let Some((y, m, d)) = parse_date_str(trimmed) {
        let serial = ymd_to_serial(y, m, d);
        Some(EvalResult::Number(serial))
    } else {
        Some(EvalResult::Error(format!(
            "DATEVALUE: cannot parse '{}'",
            date_text
        )))
    }
}

fn parse_date_str(s: &str) -> Option<(i32, i32, i32)> {
    // Handle "YYYY-MM-DD" or "YYYY/MM/DD"
    let normalized = s.replace('/', "-");
    let parts: Vec<&str> = normalized.split('-').filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let m: i32 = parts[1].parse().ok()?;
    let d: i32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=9999).contains(&y) {
        return None;
    }
    Some((y, m, d))
}

// ===========================================================================
// F3: Advanced text functions
// ===========================================================================

fn eval_text_format(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error("TEXT requires 2 args".to_string()));
    }
    let value = eval_node(ctx, &args[0])?;
    let format_str = eval_fn_text(ctx, &args[1])?;
    let num = match &value {
        EvalResult::Number(n) => *n,
        EvalResult::Text(t) => t.trim().parse::<f64>().ok()?,
        _ => return Some(EvalResult::Error("TEXT: non-numeric value".to_string())),
    };
    let formatted = format_number(&format_str, num);
    Some(EvalResult::Text(formatted))
}

fn format_number(fmt: &str, value: f64) -> String {
    let fmt_upper = fmt.to_uppercase();
    match fmt_upper.as_str() {
        "0" => format!("{}", value as i64),
        "0.00" => format!("{:.2}", value),
        "0.0" => format!("{:.1}", value),
        "0.000" => format!("{:.3}", value),
        "#,##0" => {
            let int_part = value as i64;
            format_int_with_commas(int_part)
        }
        "#,##0.00" => format!("{:.2}", value)
            .chars()
            .rev()
            .enumerate()
            .map(|(i, c)| {
                if c == '-' {
                    return String::from("-");
                }
                if c == '.' {
                    return String::from(".");
                }
                if i > 0 && i % 3 == 0 {
                    format!(
                        "{}{}",
                        c,
                        if (i / 3) % 10 == 0 && i / 3 > 1 {
                            ","
                        } else {
                            ""
                        }
                    )
                } else {
                    c.to_string()
                }
            })
            .collect::<Vec<String>>()
            .into_iter()
            .rev()
            .collect(),
        "General" | "general" => {
            if value == value.floor() && value.abs() < 1e15 {
                format!("{}", value as i64)
            } else {
                let s = format!("{:.10}", value);
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        _ => {
            // Fallback: just return the number as string
            if value == value.floor() && value.abs() < 1e15 {
                format!("{}", value as i64)
            } else {
                format!("{:.2}", value)
            }
        }
    }
}

fn format_int_with_commas(n: i64) -> String {
    let s = n.abs().to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let first_comma = len % 3;
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && i % 3 == first_comma {
            result.push(',');
        }
        result.push(c);
    }
    if n < 0 {
        result.insert(0, '-');
    }
    result
}

fn eval_textjoin(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 3 {
        return Some(EvalResult::Error(
            "TEXTJOIN requires at least 3 args".to_string(),
        ));
    }
    let delimiter = eval_result_to_string(&eval_node(ctx, &args[0])?);
    let ignore_empty = eval_result_to_numeric(&eval_node(ctx, &args[1])?);
    let mut parts = Vec::new();
    for arg in &args[2..] {
        // Handle ranges
        if let FormulaNode::Range(range) = arg {
            if let Some(cells) = ctx.get_range(range) {
                for cell in &cells {
                    if let Some(n) = cell.numeric {
                        let s = n.to_string();
                        if ignore_empty > 0.0 && s.is_empty() {
                            continue;
                        }
                        parts.push(s);
                    } else if let Some(ref t) = cell.text {
                        if ignore_empty > 0.0 && t.is_empty() {
                            continue;
                        }
                        parts.push(t.clone());
                    }
                }
            }
        } else if let FormulaNode::CellRef(ref_name) = arg {
            if let Some(cell) = ctx.get_cell(ref_name) {
                if let Some(n) = cell.numeric {
                    let s = n.to_string();
                    if ignore_empty > 0.0 && s.is_empty() {
                        continue;
                    }
                    parts.push(s);
                } else if let Some(ref t) = cell.text {
                    if ignore_empty > 0.0 && t.is_empty() {
                        continue;
                    }
                    parts.push(t.clone());
                }
            }
        } else {
            let val = eval_node(ctx, arg)?;
            let s = eval_result_to_string(&val);
            if ignore_empty > 0.0 && s.is_empty() {
                continue;
            }
            parts.push(s);
        }
    }
    Some(EvalResult::Text(parts.join(&delimiter)))
}

fn eval_search_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error(
            "SEARCH requires at least 2 args".to_string(),
        ));
    }
    let find_text = eval_result_to_string(&eval_node(ctx, &args[0])?).to_lowercase();
    let within_text = eval_result_to_string(&eval_node(ctx, &args[1])?).to_lowercase();
    let start = if args.len() > 2 {
        match eval_node(ctx, &args[2])? {
            EvalResult::Number(n) => n as usize,
            _ => 1,
        }
    } else {
        1
    };
    let search_in = &within_text[(start - 1)..];
    search_in
        .find(&find_text)
        .map(|pos| EvalResult::Number((pos + start) as f64))
        .or(Some(EvalResult::Error("#VALUE!".to_string())))
}

fn eval_find_fn(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.len() < 2 {
        return Some(EvalResult::Error(
            "FIND requires at least 2 args".to_string(),
        ));
    }
    let find_text = eval_result_to_string(&eval_node(ctx, &args[0])?);
    let within_text = eval_result_to_string(&eval_node(ctx, &args[1])?);
    let start = if args.len() > 2 {
        match eval_node(ctx, &args[2])? {
            EvalResult::Number(n) => n as usize,
            _ => 1,
        }
    } else {
        1
    };
    within_text
        .char_indices()
        .skip(start - 1)
        .find_map(|(idx, _ch)| {
            if within_text[idx..].starts_with(&find_text) {
                Some(EvalResult::Number((idx + 1) as f64))
            } else {
                None
            }
        })
        .or_else(|| {
            // Fallback: use direct find from start position
            within_text
                .get(start - 1..)?
                .find(&find_text)
                .map(|pos| EvalResult::Number((pos + start) as f64))
        })
        .or(Some(EvalResult::Error("#VALUE!".to_string())))
}

// ===========================================================================
// F4: Extended logical functions
// ===========================================================================

fn eval_xor(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    let mut count = 0u32;
    for arg in args {
        let val = eval_node(ctx, arg)?;
        if is_truthy(&val) {
            count += 1;
        }
    }
    Some(EvalResult::Bool(count % 2 == 1))
}

fn eval_ifs(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "IFS requires at least 2 args".to_string(),
        ));
    }
    let mut i = 0;
    while i + 1 < args.len() {
        let cond = eval_node(ctx, &args[i])?;
        if is_truthy(&cond) {
            return eval_node(ctx, &args[i + 1]);
        }
        i += 2;
    }
    Some(EvalResult::Error("#N/A".to_string()))
}

fn eval_switch(ctx: &dyn FormulaContext, args: &[FormulaNode]) -> Option<EvalResult> {
    if args.is_empty() {
        return Some(EvalResult::Error(
            "SWITCH requires at least 2 args".to_string(),
        ));
    }
    let expr = eval_node(ctx, &args[0])?;
    let mut i = 1;
    while i + 1 < args.len() {
        let val = eval_node(ctx, &args[i])?;
        if values_equal(&expr, &val) {
            return eval_node(ctx, &args[i + 1]);
        }
        i += 2;
    }
    // Check for default (odd number of remaining args)
    if i < args.len() {
        return eval_node(ctx, &args[i]);
    }
    Some(EvalResult::Error("#N/A".to_string()))
}

fn values_equal(a: &EvalResult, b: &EvalResult) -> bool {
    match (a, b) {
        (EvalResult::Number(x), EvalResult::Number(y)) => (*x - *y).abs() < 1e-9,
        (EvalResult::Text(x), EvalResult::Text(y)) => x == y,
        (EvalResult::Bool(x), EvalResult::Bool(y)) => x == y,
        _ => false,
    }
}

fn math_pow(base: f64, exp: i32) -> f64 {
    base.powi(exp)
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
#[allow(unused_variables)]
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

    // ========================================================================
    // P7-3 Tranche A: Date/Time, Statistical, Error handling tests
    // ========================================================================

    #[test]
    fn test_today() {
        let ctx = TestContext::default();
        let node = parse_formula("TODAY()").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(45672.0)));
    }

    #[test]
    fn test_now() {
        let ctx = TestContext::default();
        let node = parse_formula("NOW()").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(45672.0)));
    }

    #[test]
    fn test_year() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15
        let node = parse_formula("YEAR(45672)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(2025.0)));
        // 44927 = 2023-01-15
        let node = parse_formula("YEAR(44927)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(2023.0)));
        // 1 = 1900-01-01
        let node = parse_formula("YEAR(1)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1900.0)));
    }

    #[test]
    fn test_month() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15
        let node = parse_formula("MONTH(45672)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
        // 2025-06-15
        let node = parse_formula("MONTH(45824)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(6.0)));
    }

    #[test]
    fn test_day_fn() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15
        let node = parse_formula("DAY(45672)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(15.0)));
        // 2025-03-31
        let node = parse_formula("DAY(45747)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(31.0)));
    }

    #[test]
    fn test_datedif_days() {
        let ctx = TestContext::default();
        // 44927 = 2023-01-15, 45292 = 2024-01-15
        let node = parse_formula("DATEDIF(44927, 45292, \"d\")").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(365.0)));
    }

    #[test]
    fn test_datedif_months() {
        let ctx = TestContext::default();
        // Same period in months
        let node = parse_formula("DATEDIF(44927, 45292, \"m\")").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(12.0)));
    }

    #[test]
    fn test_datedif_years() {
        let ctx = TestContext::default();
        let node = parse_formula("DATEDIF(44927, 45292, \"y\")").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
    }

    #[test]
    fn test_eomonth() {
        let ctx = TestContext::default();
        // Jan 2025 (45672) + 0 months = last day of Jan 2025 = 45688
        let node = parse_formula("EOMONTH(45672, 0)").unwrap();
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(45688.0));
        // Feb 2025 + 0 months = last day of Feb 2025 = 45716
        let node = parse_formula("EOMONTH(45702, 0)").unwrap();
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(45716.0));
    }

    #[test]
    fn test_counta() {
        let ctx = TestContext::default();
        let node = parse_formula("COUNTA(1, \"\", \"x\")").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(2.0)));
    }

    #[test]
    fn test_counta_with_cell() {
        let ctx = TestContext::default()
            .with_value("A1", 42.0)
            .with_text("A2", "hello")
            .with_text("A3", "");
        let node = parse_formula("COUNTA(A1, A2, A3)").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(2.0)));
    }

    #[test]
    fn test_countblank() {
        let ctx = TestContext::default();
        let node = parse_formula("COUNTBLANK(1, \"\", \"x\")").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(1.0)));
    }

    #[test]
    fn test_stdev() {
        let ctx = TestContext::default();
        let node = parse_formula("STDEV(1,2,3,4,5)").unwrap();
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(1.5811388300841898));
    }

    #[test]
    fn test_stdev_range() {
        let ctx = TestContext::default();
        let node = parse_formula("STDEV(A1:A3)").unwrap();
        // A1:A3 = {1, 2, 3}, sample std dev = 1.0
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(1.0));
    }

    #[test]
    fn test_stdevp() {
        let ctx = TestContext::default();
        let node = parse_formula("STDEVP(2,4,4,4,5,5,7,9)").unwrap();
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(2.0));
    }

    #[test]
    fn test_var() {
        let ctx = TestContext::default();
        let node = parse_formula("VAR(1,2,3,4,5)").unwrap();
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(2.5));
    }

    #[test]
    fn test_varp() {
        let ctx = TestContext::default();
        let node = parse_formula("VARP(2,4,4,4,5,5,7,9)").unwrap();
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(4.0));
    }

    #[test]
    fn test_rank_descending() {
        let ctx = TestContext::default();
        let node = parse_formula("RANK(3, A1:A10, 0)").unwrap();
        // A1:A10 = {1,2,3,...,10}, rank of 3 in descending = 8 (8 values less than 3... wait, descending means 1 = highest)
        // In descending: 10=rank1, 9=rank2, 8=rank3, 7=rank4, 6=rank5, 5=rank6, 4=rank7, 3=rank8
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(8.0));
    }

    #[test]
    fn test_rank_ascending() {
        let ctx = TestContext::default();
        let node = parse_formula("RANK(3, A1:A10, 1)").unwrap();
        // In ascending: 1=rank1, 2=rank2, 3=rank3
        let result = evaluate(&node, &ctx).unwrap();
        assert_eq!(result, EvalResult::Number(3.0));
    }

    #[test]
    fn test_iserror() {
        let ctx = TestContext::default();
        let node = parse_formula("ISERROR(1/0)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
        let node = parse_formula("ISERROR(42)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(0.0)));
    }

    #[test]
    fn test_isnumber() {
        let ctx = TestContext::default();
        let node = parse_formula("ISNUMBER(42)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
        let node = parse_formula("ISNUMBER(\"hello\")").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(0.0)));
    }

    #[test]
    fn test_istext() {
        let ctx = TestContext::default();
        let node = parse_formula("ISTEXT(\"hello\")").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
        let node = parse_formula("ISTEXT(42)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(0.0)));
    }

    #[test]
    fn test_ifna_with_value() {
        let ctx = TestContext::default();
        let node = parse_formula("IFNA(42, \"fallback\")").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_ifna_with_text() {
        let ctx = TestContext::default();
        let node = parse_formula(r#"IFNA("hello", "fallback")"#).unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Text("hello".to_string())));
    }

    // ===== P7-3 Tranche A tests =====

    // Cell reference with $ prefix (parsed correctly)
    #[test]
    fn test_abs_cell_ref_dollar_dollar() {
        let ctx = TestContext::default().with_value("A1", 42.0);
        let node = parse_formula("$A$1").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_abs_cell_ref_dollar_col() {
        let ctx = TestContext::default().with_value("A1", 42.0);
        let node = parse_formula("$A1").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_abs_cell_ref_dollar_row() {
        let ctx = TestContext::default().with_value("A1", 42.0);
        let node = parse_formula("A$1").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(42.0)));
    }

    // Exponentiation
    #[test]
    fn test_pow_literal() {
        assert_eq!(evaluate_formula("2^3"), Some(EvalResult::Number(8.0)));
    }

    #[test]
    fn test_pow_cell_ref() {
        let ctx = TestContext::default().with_value("A1", 3.0);
        let node = parse_formula("A1^2").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(9.0)));
    }

    #[test]
    fn test_pow_right_associative() {
        // 2^3^2 = 2^(3^2) = 2^9 = 512
        assert_eq!(evaluate_formula("2^3^2"), Some(EvalResult::Number(512.0)));
    }

    // Percent operator
    #[test]
    fn test_percent_literal() {
        assert_eq!(evaluate_formula("10%"), Some(EvalResult::Number(0.1)));
    }

    #[test]
    fn test_percent_in_expression() {
        assert_eq!(evaluate_formula("100*10%"), Some(EvalResult::Number(10.0)));
    }

    #[test]
    fn test_percent_with_cell() {
        let ctx = TestContext::default().with_value("A1", 50.0);
        let node = parse_formula("A1*10%").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(5.0)));
    }

    // Math functions
    #[test]
    fn test_abs() {
        assert_eq!(evaluate_formula("ABS(-5)"), Some(EvalResult::Number(5.0)));
        assert_eq!(evaluate_formula("ABS(5)"), Some(EvalResult::Number(5.0)));
    }

    #[test]
    fn test_round() {
        assert_eq!(
            evaluate_formula("ROUND(2.7183,2)"),
            Some(EvalResult::Number(2.72))
        );
        assert_eq!(
            evaluate_formula("ROUND(2.5,0)"),
            Some(EvalResult::Number(3.0))
        );
    }

    #[test]
    fn test_roundup() {
        assert_eq!(
            evaluate_formula("ROUNDUP(3.11,1)"),
            Some(EvalResult::Number(3.2))
        );
        assert_eq!(
            evaluate_formula("ROUNDUP(-3.11,1)"),
            Some(EvalResult::Number(-3.2))
        );
    }

    #[test]
    fn test_rounddown() {
        assert_eq!(
            evaluate_formula("ROUNDDOWN(3.9,1)"),
            Some(EvalResult::Number(3.9))
        );
        assert_eq!(
            evaluate_formula("ROUNDDOWN(-3.9,1)"),
            Some(EvalResult::Number(-3.9))
        );
    }

    #[test]
    fn test_mod() {
        assert_eq!(evaluate_formula("MOD(10,3)"), Some(EvalResult::Number(1.0)));
        assert_eq!(evaluate_formula("MOD(10,4)"), Some(EvalResult::Number(2.0)));
    }

    #[test]
    fn test_int() {
        assert_eq!(evaluate_formula("INT(3.9)"), Some(EvalResult::Number(3.0)));
        assert_eq!(
            evaluate_formula("INT(-3.9)"),
            Some(EvalResult::Number(-4.0))
        );
    }

    #[test]
    fn test_power() {
        assert_eq!(
            evaluate_formula("POWER(2,3)"),
            Some(EvalResult::Number(8.0))
        );
        assert_eq!(
            evaluate_formula("POWER(9,0.5)"),
            Some(EvalResult::Number(3.0))
        );
    }

    // Text functions
    #[test]
    fn test_len() {
        assert_eq!(
            evaluate_formula(r#"LEN("hello")"#),
            Some(EvalResult::Number(5.0))
        );
        assert_eq!(
            evaluate_formula(r#"LEN("")"#),
            Some(EvalResult::Number(0.0))
        );
    }

    #[test]
    fn test_trim() {
        assert_eq!(
            evaluate_formula(r#"TRIM("  x  ")"#),
            Some(EvalResult::Text("x".to_string()))
        );
    }

    #[test]
    fn test_upper() {
        assert_eq!(
            evaluate_formula(r#"UPPER("hi")"#),
            Some(EvalResult::Text("HI".to_string()))
        );
    }

    #[test]
    fn test_lower() {
        assert_eq!(
            evaluate_formula(r#"LOWER("HI")"#),
            Some(EvalResult::Text("hi".to_string()))
        );
    }

    #[test]
    fn test_rept() {
        assert_eq!(
            evaluate_formula(r#"REPT("a",3)"#),
            Some(EvalResult::Text("aaa".to_string()))
        );
    }

    #[test]
    fn test_substitute_3args() {
        assert_eq!(
            evaluate_formula(r#"SUBSTITUTE("a-b-c","-","|")"#),
            Some(EvalResult::Text("a|b|c".to_string()))
        );
    }

    #[test]
    fn test_substitute_4args() {
        assert_eq!(
            evaluate_formula(r#"SUBSTITUTE("a-b-c","-","|",1)"#),
            Some(EvalResult::Text("a|b-c".to_string()))
        );
    }

    #[test]
    fn test_left() {
        assert_eq!(
            evaluate_formula(r#"LEFT("hello",2)"#),
            Some(EvalResult::Text("he".to_string()))
        );
        assert_eq!(
            evaluate_formula(r#"LEFT("hello")"#),
            Some(EvalResult::Text("hello".to_string()))
        );
    }

    #[test]
    fn test_right() {
        assert_eq!(
            evaluate_formula(r#"RIGHT("hello",2)"#),
            Some(EvalResult::Text("lo".to_string()))
        );
    }

    #[test]
    fn test_mid() {
        assert_eq!(
            evaluate_formula(r#"MID("hello",2,3)"#),
            Some(EvalResult::Text("ell".to_string()))
        );
    }

    // Error cases
    #[test]
    fn test_wrong_arg_count_abs() {
        let result = evaluate_formula("ABS()");
        assert!(matches!(result, Some(EvalResult::Error(_))));
    }

    #[test]
    fn test_wrong_arg_count_round() {
        let result = evaluate_formula("ROUND(1.5)");
        assert!(matches!(result, Some(EvalResult::Error(_))));
    }

    #[test]
    fn test_non_numeric_abs() {
        let result = evaluate_formula(r#"ABS("abc")"#);
        assert!(matches!(result, Some(EvalResult::Error(_))));
    }

    // ========================================================================
    // P7-3B Tranche B: Lookup function tests
    // ========================================================================

    /// Extended test context supporting 2D grid ranges for lookup functions.
    #[derive(Default)]
    struct LookupTestContext {
        cells: std::collections::HashMap<String, CachedCellValue>,
        ranges: std::collections::HashMap<String, (Vec<CachedCellValue>, usize)>,
    }

    impl LookupTestContext {
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

        fn with_range(mut self, range: &str, values: Vec<CachedCellValue>, cols: usize) -> Self {
            self.ranges.insert(range.to_string(), (values, cols));
            self
        }

        /// Build a rectangular grid range from row-major values.
        fn with_grid(mut self, range: &str, rows: Vec<Vec<f64>>) -> Self {
            let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            let mut values = Vec::new();
            for row in &rows {
                for &v in row {
                    values.push(CachedCellValue {
                        numeric: Some(v),
                        text: None,
                        bool: None,
                    });
                }
            }
            self.ranges.insert(range.to_string(), (values, cols));
            self
        }

        /// Build a text grid range from row-major values.
        fn with_text_grid(mut self, range: &str, rows: Vec<Vec<&str>>) -> Self {
            let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            let mut values = Vec::new();
            for row in &rows {
                for &v in row {
                    values.push(CachedCellValue {
                        numeric: None,
                        text: Some(v.to_string()),
                        bool: None,
                    });
                }
            }
            self.ranges.insert(range.to_string(), (values, cols));
            self
        }

        /// Build a mixed grid (first row text headers, rest numeric).
        fn with_data_grid(
            mut self,
            range: &str,
            headers: Vec<&str>,
            data_rows: Vec<Vec<f64>>,
        ) -> Self {
            let cols = headers.len();
            let mut values = Vec::new();
            // First row: headers
            for &h in &headers {
                values.push(CachedCellValue {
                    numeric: None,
                    text: Some(h.to_string()),
                    bool: None,
                });
            }
            // Data rows
            for row in &data_rows {
                for &v in row {
                    values.push(CachedCellValue {
                        numeric: Some(v),
                        text: None,
                        bool: None,
                    });
                }
            }
            let _total_rows = 1 + data_rows.len();
            self.ranges.insert(range.to_string(), (values, cols));
            self
        }
    }

    impl FormulaContext for LookupTestContext {
        fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
            self.cells.get(ref_name).cloned()
        }

        fn get_range(&self, range: &str) -> Option<Vec<CachedCellValue>> {
            self.ranges.get(range).map(|(v, _)| v.clone())
        }

        fn get_cell_at(
            &self,
            _sheet: Option<&str>,
            _row: usize,
            _col: usize,
        ) -> Option<CachedCellValue> {
            None
        }

        fn analysis_date(&self) -> f64 {
            45672.0
        }
    }

    fn evaluate_lookup_formula(formula: &str, ctx: &LookupTestContext) -> Option<EvalResult> {
        let node = parse_formula(formula).ok()?;
        evaluate(&node, ctx)
    }

    // VLOOKUP tests
    #[test]
    fn test_vlookup_exact_match() {
        let ctx = LookupTestContext::default().with_text_grid(
            "A1:B5",
            vec![
                vec!["A", "10"],
                vec!["B", "20"],
                vec!["C", "30"],
                vec!["D", "40"],
                vec!["E", "50"],
            ],
        );
        // VLOOKUP("B", A1:B5, 2, FALSE) = 20
        let result = evaluate_lookup_formula(r#"VLOOKUP("B", A1:B5, 2, FALSE)"#, &ctx);
        assert_eq!(result, Some(EvalResult::Number(20.0)));
        // VLOOKUP("D", A1:B5, 2, 0) = 40
        let result = evaluate_lookup_formula(r#"VLOOKUP("D", A1:B5, 2, 0)"#, &ctx);
        assert_eq!(result, Some(EvalResult::Number(40.0)));
    }

    #[test]
    fn test_vlookup_approximate_match() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:B5",
            vec![
                vec![1.0, 10.0],
                vec![2.0, 20.0],
                vec![3.0, 30.0],
                vec![4.0, 40.0],
                vec![5.0, 50.0],
            ],
        );
        // VLOOKUP(3, A1:B5, 2, FALSE) = 30
        let result = evaluate_lookup_formula("VLOOKUP(3, A1:B5, 2, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(30.0)));
        // VLOOKUP(2.5, A1:B5, 2, TRUE) = 20 (next smaller = 2 -> 20)
        let result = evaluate_lookup_formula("VLOOKUP(2.5, A1:B5, 2, TRUE)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(20.0)));
    }

    #[test]
    fn test_vlookup_not_found() {
        let ctx = LookupTestContext::default().with_text_grid(
            "A1:B3",
            vec![vec!["A", "10"], vec!["B", "20"], vec!["C", "30"]],
        );
        // VLOOKUP("Z", A1:B3, 2, FALSE) = #N/A
        let result = evaluate_lookup_formula(r#"VLOOKUP("Z", A1:B3, 2, FALSE)"#, &ctx);
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    #[test]
    fn test_vlookup_col_index_error() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:B5",
            vec![
                vec![1.0, 10.0],
                vec![2.0, 20.0],
                vec![3.0, 30.0],
                vec![4.0, 40.0],
                vec![5.0, 50.0],
            ],
        );
        // VLOOKUP(3, A1:B5, 3, FALSE) = #REF! (col 3 > 2 columns)
        let result = evaluate_lookup_formula("VLOOKUP(3, A1:B5, 3, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#REF!".to_string())));
    }

    #[test]
    fn test_vlookup_with_cell_refs() {
        let ctx = LookupTestContext::default()
            .with_value("B1", 1.5)
            .with_text_grid(
                "A1:B5",
                vec![
                    vec!["A", "10"],
                    vec!["B", "20"],
                    vec!["C", "30"],
                    vec!["D", "40"],
                    vec!["E", "50"],
                ],
            );
        // VLOOKUP(B1, A1:B5, 2, FALSE) should search for 1.5 as text "1.5"
        // which won't match, so expect #N/A
        let result = evaluate_lookup_formula("VLOOKUP(B1, A1:B5, 2, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    // HLOOKUP tests
    #[test]
    fn test_hlookup_exact_match() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:E2",
            vec![
                vec![1.0, 2.0, 3.0, 4.0, 5.0],
                vec![10.0, 20.0, 30.0, 40.0, 50.0],
            ],
        );
        // HLOOKUP(3, A1:E2, 2, FALSE) = 30
        let result = evaluate_lookup_formula("HLOOKUP(3, A1:E2, 2, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(30.0)));
        // HLOOKUP(1, A1:E2, 2, FALSE) = 10
        let result = evaluate_lookup_formula("HLOOKUP(1, A1:E2, 2, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(10.0)));
    }

    #[test]
    fn test_hlookup_approximate_match() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:E2",
            vec![
                vec![1.0, 2.0, 3.0, 4.0, 5.0],
                vec![10.0, 20.0, 30.0, 40.0, 50.0],
            ],
        );
        // HLOOKUP(2.5, A1:E2, 2, TRUE) = 20 (next smaller = 2 -> col 2 -> 20)
        let result = evaluate_lookup_formula("HLOOKUP(2.5, A1:E2, 2, TRUE)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(20.0)));
    }

    #[test]
    fn test_hlookup_not_found() {
        let ctx =
            LookupTestContext::default().with_grid("A1:B2", vec![vec![1.0, 2.0], vec![10.0, 20.0]]);
        // HLOOKUP(99, A1:B2, 2, FALSE) = #N/A
        let result = evaluate_lookup_formula("HLOOKUP(99, A1:B2, 2, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    #[test]
    fn test_hlookup_text_header() {
        let ctx = LookupTestContext::default().with_data_grid(
            "A1:C4",
            vec!["Item", "Price", "Qty"],
            vec![
                vec![100.0, 10.0, 5.0],
                vec![200.0, 20.0, 3.0],
                vec![300.0, 30.0, 8.0],
            ],
        );
        // HLOOKUP("Price", A1:C4, 2, FALSE) = 100 (row 2, col "Price"=2)
        // Wait, Price is in column C (col 3). Row 2 is data row 1.
        // Actually HLOOKUP searches the first row (headers), finds "Price" at col 3
        // Returns value from row 2 at col 3 = 10.0
        let result = evaluate_lookup_formula(r#"HLOOKUP("Price", A1:C4, 2, FALSE)"#, &ctx);
        assert_eq!(result, Some(EvalResult::Number(10.0)));
    }

    // MATCH tests
    #[test]
    fn test_match_exact() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A5",
            vec![
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
            ],
            1,
        );
        // MATCH(30, A1:A5, 0) = 3
        let result = evaluate_lookup_formula("MATCH(30, A1:A5, 0)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(3.0)));
    }

    #[test]
    fn test_match_next_smaller() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A5",
            vec![
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
            ],
            1,
        );
        // MATCH(25, A1:A5, 1) = 2 (next smaller is 20 at position 2)
        let result = evaluate_lookup_formula("MATCH(25, A1:A5, 1)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(2.0)));
    }

    #[test]
    fn test_match_next_larger() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A5",
            vec![
                CachedCellValue {
                    numeric: Some(50.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(40.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(30.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(20.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(10.0),
                    text: None,
                    bool: None,
                },
            ],
            1,
        );
        // MATCH(35, A1:A5, -1) = 2 (next larger is 40 at position 2)
        let result = evaluate_lookup_formula("MATCH(35, A1:A5, -1)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(2.0)));
    }

    #[test]
    fn test_match_not_found() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A3",
            vec![
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
            ],
            1,
        );
        // MATCH(99, A1:A3, 0) = #N/A
        let result = evaluate_lookup_formula("MATCH(99, A1:A3, 0)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    #[test]
    fn test_match_text() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A4",
            vec![
                CachedCellValue {
                    numeric: None,
                    text: Some("Apple".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: None,
                    text: Some("Banana".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: None,
                    text: Some("Cherry".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: None,
                    text: Some("Date".to_string()),
                    bool: None,
                },
            ],
            1,
        );
        // MATCH("Cherry", A1:A4, 0) = 3
        let result = evaluate_lookup_formula(r#"MATCH("Cherry", A1:A4, 0)"#, &ctx);
        assert_eq!(result, Some(EvalResult::Number(3.0)));
    }

    #[test]
    fn test_match_default_type() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A5",
            vec![
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
            ],
            1,
        );
        // MATCH(35, A1:A5) without match_type defaults to 1 (next smaller) = 30 at pos 3
        let result = evaluate_lookup_formula("MATCH(35, A1:A5)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(3.0)));
    }

    // INDEX tests
    #[test]
    fn test_index_cell() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:C3",
            vec![
                vec![1.0, 2.0, 3.0],
                vec![4.0, 5.0, 6.0],
                vec![7.0, 8.0, 9.0],
            ],
        );
        // INDEX(A1:C3, 2, 3) = 6
        let result = evaluate_lookup_formula("INDEX(A1:C3, 2, 3)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(6.0)));
    }

    #[test]
    fn test_index_out_of_bounds() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:C3",
            vec![
                vec![1.0, 2.0, 3.0],
                vec![4.0, 5.0, 6.0],
                vec![7.0, 8.0, 9.0],
            ],
        );
        // INDEX(A1:C3, 4, 1) = #REF! (row 4 > 3 rows)
        let result = evaluate_lookup_formula("INDEX(A1:C3, 4, 1)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#REF!".to_string())));
        // INDEX(A1:C3, 1, 4) = #REF! (col 4 > 3 cols)
        let result = evaluate_lookup_formula("INDEX(A1:C3, 1, 4)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#REF!".to_string())));
    }

    #[test]
    fn test_index_single_column() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A5",
            vec![
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
            ],
            1,
        );
        // INDEX(A1:A5, 3) = 30 (single column, col defaults to 1)
        let result = evaluate_lookup_formula("INDEX(A1:A5, 3)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(30.0)));
    }

    #[test]
    fn test_index_single_row() {
        let ctx = LookupTestContext::default().with_range(
            "A1:C1",
            vec![
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
            ],
            3,
        );
        // INDEX(A1:C1, 2) = 20 (single row: row arg = column index)
        let result = evaluate_lookup_formula("INDEX(A1:C1, 2)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(20.0)));
    }

    #[test]
    fn test_index_edge_values() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:C3",
            vec![
                vec![1.0, 2.0, 3.0],
                vec![4.0, 5.0, 6.0],
                vec![7.0, 8.0, 9.0],
            ],
        );
        // INDEX(A1:C3, 1, 1) = 1
        let result = evaluate_lookup_formula("INDEX(A1:C3, 1, 1)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(1.0)));
        // INDEX(A1:C3, 3, 3) = 9
        let result = evaluate_lookup_formula("INDEX(A1:C3, 3, 3)", &ctx);
        assert_eq!(result, Some(EvalResult::Number(9.0)));
    }

    #[test]
    fn test_vlookup_text_lookup() {
        let ctx = LookupTestContext::default().with_range(
            "A1:C5",
            vec![
                CachedCellValue {
                    numeric: None,
                    text: Some("Apple".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(100.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(5.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: None,
                    text: Some("Banana".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(200.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(3.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: None,
                    text: Some("Cherry".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(300.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(8.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: None,
                    text: Some("Date".to_string()),
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(400.0),
                    text: None,
                    bool: None,
                },
                CachedCellValue {
                    numeric: Some(2.0),
                    text: None,
                    bool: None,
                },
            ],
            3,
        );
        // VLOOKUP("Cherry", A1:C5, 2, FALSE) = 300 (searches column A for "Cherry",
        // finds at row 3, returns col 2 = 300)
        let result = evaluate_lookup_formula(r#"VLOOKUP("Cherry", A1:C5, 2, FALSE)"#, &ctx);
        assert_eq!(result, Some(EvalResult::Number(300.0)));
    }

    #[test]
    fn test_vlookup_zero_col() {
        let ctx = LookupTestContext::default().with_grid(
            "A1:B3",
            vec![vec![1.0, 10.0], vec![2.0, 20.0], vec![3.0, 30.0]],
        );
        // VLOOKUP(2, A1:B3, 0, FALSE) = #VALUE! (col 0 is invalid)
        let result = evaluate_lookup_formula("VLOOKUP(2, A1:B3, 0, FALSE)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#VALUE!".to_string())));
    }

    #[test]
    fn test_match_value_error() {
        let ctx = LookupTestContext::default().with_range(
            "A1:A5",
            vec![
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
            ],
            1,
        );
        // MATCH(3, A1:A5, 2) invalid match_type 2 defaults to exact
        let result = evaluate_lookup_formula("MATCH(3, A1:A5, 2)", &ctx);
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    #[test]
    fn test_index_text_grid() {
        let ctx = LookupTestContext::default().with_text_grid(
            "A1:C3",
            vec![
                vec!["A", "B", "C"],
                vec!["D", "E", "F"],
                vec!["G", "H", "I"],
            ],
        );
        // INDEX(A1:C3, 2, 2) = "E"
        let result = evaluate_lookup_formula("INDEX(A1:C3, 2, 2)", &ctx);
        assert_eq!(result, Some(EvalResult::Text("E".to_string())));
    }

    // ========================================================================
    // F1: Financial functions tests
    // ========================================================================

    #[test]
    fn test_npv() {
        let ctx = TestContext::default();
        // NPV(10%, 300, 400, 500) = 300/1.1 + 400/1.21 + 500/1.331
        let result = evaluate_formula("NPV(0.1,300,400,500)");
        assert_eq!(
            result,
            Some(EvalResult::Number(
                300.0 / 1.1 + 400.0 / 1.21 + 500.0 / 1.331
            ))
        );
    }

    #[test]
    fn test_npv_zero_rate() {
        let ctx = TestContext::default();
        // NPV(0, 100, 200, 300) = 100 + 200 + 300 = 600 (no discounting)
        let result = evaluate_formula("NPV(0,100,200,300)");
        assert_eq!(result, Some(EvalResult::Number(600.0)));
    }

    #[test]
    fn test_pmt() {
        let ctx = TestContext::default();
        // PMT(10%/12, 60, 100000)
        let result = evaluate_formula("PMT(0.1/12,60,100000)");
        assert!(matches!(result, Some(EvalResult::Number(_))));
    }

    #[test]
    fn test_pmt_zero_rate() {
        let ctx = TestContext::default();
        // PMT(0, 12, 1000) = -(0+1000)/12
        let result = evaluate_formula("PMT(0,12,1000)");
        assert_eq!(result, Some(EvalResult::Number(-1000.0 / 12.0)));
    }

    #[test]
    fn test_pv() {
        let ctx = TestContext::default();
        let result = evaluate_formula("PV(0.1/12,60,1000)");
        assert!(matches!(result, Some(EvalResult::Number(_))));
    }

    #[test]
    fn test_fv() {
        let ctx = TestContext::default();
        let result = evaluate_formula("FV(0.1/12,60,-1000,0,1)");
        assert!(matches!(result, Some(EvalResult::Number(_))));
    }

    #[test]
    fn test_rate() {
        let ctx = TestContext::default();
        let result = evaluate_formula("RATE(60,-1000,50000)");
        assert!(matches!(result, Some(EvalResult::Number(_))));
    }

    #[test]
    fn test_nper() {
        let ctx = TestContext::default();
        let result = evaluate_formula("NPER(0.1/12,-1000,10000)");
        assert!(matches!(result, Some(EvalResult::Number(_))));
    }

    // ========================================================================
    // F2: Advanced date/time tests
    // ========================================================================

    #[test]
    fn test_workday_basic() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15 (Wednesday), +5 working days = 2025-01-22 (Wednesday)
        let node = parse_formula("WORKDAY(45672,5)").unwrap();
        let result = evaluate(&node, &ctx);
        let end_serial = match result {
            Some(EvalResult::Number(n)) => n,
            _ => panic!("expected number"),
        };
        let (y, m, d) = serial_to_ymd(end_serial).expect("valid date");
        assert_eq!((y, m, d), (2025, 1, 22));
    }

    #[test]
    fn test_workday_skip_weekend() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15 (Wednesday), +10 working days = Jan 29, 2025 (Wednesday)
        let node = parse_formula("WORKDAY(45672,10)").unwrap();
        let result = evaluate(&node, &ctx);
        let end_serial = match result {
            Some(EvalResult::Number(n)) => n,
            _ => panic!("expected number"),
        };
        let (y, m, d) = serial_to_ymd(end_serial).expect("valid date");
        // 10 working days from Jan 15, 2025 = Jan 29, 2025
        assert_eq!((y, m, d), (2025, 1, 29));
    }

    #[test]
    fn test_networkdays_basic() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15 (Wednesday), 45678 = 2025-01-21 (Tuesday)
        // Wed, Thu, Fri, Mon, Tue = 5 working days
        let node = parse_formula("NETWORKDAYS(45672,45678)").unwrap();
        let result = evaluate(&node, &ctx);
        assert_eq!(result, Some(EvalResult::Number(5.0)));
    }

    #[test]
    fn test_weeknum() {
        let ctx = TestContext::default();
        // 45672 = 2025-01-15, which is in week 3 of 2025 (ISO)
        let node = parse_formula("WEEKNUM(45672)").unwrap();
        let result = evaluate(&node, &ctx);
        if let Some(EvalResult::Number(w)) = result {
            (1.0..=54.0).contains(&w);
        } else {
            panic!("expected week number");
        }
    }

    #[test]
    fn test_datevalue() {
        let ctx = TestContext::default();
        // "2025-01-15" should parse to serial date
        let node = parse_formula(r#"DATEVALUE("2025-01-15")"#).unwrap();
        let result = evaluate(&node, &ctx);
        if let Some(EvalResult::Number(serial)) = result {
            let (y, m, d) = serial_to_ymd(serial).expect("valid date");
            assert_eq!((y, m, d), (2025, 1, 15));
        } else {
            panic!("expected number");
        }
    }

    #[test]
    fn test_datevalue_slash() {
        let ctx = TestContext::default();
        let node = parse_formula(r#"DATEVALUE("2025/06/15")"#).unwrap();
        let result = evaluate(&node, &ctx);
        if let Some(EvalResult::Number(serial)) = result {
            let (y, m, d) = serial_to_ymd(serial).expect("valid date");
            assert_eq!((y, m, d), (2025, 6, 15));
        } else {
            panic!("expected number");
        }
    }

    // ========================================================================
    // F3: Advanced text tests
    // ========================================================================

    #[test]
    fn test_text_format_basic() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"TEXT(3.14159,"0.00")"#),
            Some(EvalResult::Text("3.14".to_string()))
        );
    }

    #[test]
    fn test_text_format_integer() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"TEXT(42,"0")"#),
            Some(EvalResult::Text("42".to_string()))
        );
    }

    #[test]
    fn test_text_format_commas() {
        let ctx = TestContext::default();
        let result = evaluate_formula(r##"TEXT(1234567,"#,##0")"##);
        if let Some(EvalResult::Text(s)) = result {
            assert_eq!(s, "1,234,567");
        } else {
            panic!("expected text");
        }
    }

    #[test]
    fn test_text_format_general() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"TEXT(42,"General")"#),
            Some(EvalResult::Text("42".to_string()))
        );
    }

    #[test]
    fn test_textjoin_basic() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"TEXTJOIN(",",TRUE,"a","b","c")"#),
            Some(EvalResult::Text("a,b,c".to_string()))
        );
    }

    #[test]
    fn test_textjoin_ignore_empty() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"TEXTJOIN(",",TRUE,"a","","c")"#),
            Some(EvalResult::Text("a,c".to_string()))
        );
    }

    #[test]
    fn test_textjoin_no_ignore() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"TEXTJOIN(",",FALSE,"a","","c")"#),
            Some(EvalResult::Text("a,,c".to_string()))
        );
    }

    #[test]
    fn test_search_case_insensitive() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"SEARCH("world","Hello World")"#),
            Some(EvalResult::Number(7.0))
        );
        // "WORLD" should also match "Hello World" (case insensitive)
        assert_eq!(
            evaluate_formula(r#"SEARCH("WORLD","Hello World")"#),
            Some(EvalResult::Number(7.0))
        );
    }

    #[test]
    fn test_search_not_found() {
        let ctx = TestContext::default();
        let result = evaluate_formula(r#"SEARCH("xyz","Hello World")"#);
        assert_eq!(result, Some(EvalResult::Error("#VALUE!".to_string())));
    }

    #[test]
    fn test_search_with_start() {
        let ctx = TestContext::default();
        // Search for "o" starting at position 5 in "Hello World"
        // "Hello World"[4:] = "o World", "o" found at index 0, so position = 5+0 = 5
        assert_eq!(
            evaluate_formula(r#"SEARCH("o","Hello World",5)"#),
            Some(EvalResult::Number(5.0))
        );
    }

    #[test]
    fn test_find_case_sensitive() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula(r#"FIND("W","Hello World")"#),
            Some(EvalResult::Number(7.0))
        );
        // "w" should NOT match "Hello World" (case sensitive)
        let result = evaluate_formula(r#"FIND("w","Hello World")"#);
        assert_eq!(result, Some(EvalResult::Error("#VALUE!".to_string())));
    }

    #[test]
    fn test_find_not_found() {
        let ctx = TestContext::default();
        let result = evaluate_formula(r#"FIND("xyz","Hello World")"#);
        assert_eq!(result, Some(EvalResult::Error("#VALUE!".to_string())));
    }

    // ========================================================================
    // F4: Extended logical tests
    // ========================================================================

    #[test]
    fn test_xor_all_false() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("XOR(FALSE,FALSE)"),
            Some(EvalResult::Bool(false))
        );
    }

    #[test]
    fn test_xor_one_true() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("XOR(TRUE,FALSE,FALSE)"),
            Some(EvalResult::Bool(true))
        );
    }

    #[test]
    fn test_xor_two_true() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("XOR(TRUE,TRUE,FALSE)"),
            Some(EvalResult::Bool(false))
        );
    }

    #[test]
    fn test_xor_three_true() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("XOR(TRUE,TRUE,TRUE)"),
            Some(EvalResult::Bool(true))
        );
    }

    #[test]
    fn test_ifs_first_match() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("IFS(1>2,10,3>2,20)"),
            Some(EvalResult::Number(20.0))
        );
    }

    #[test]
    fn test_ifs_no_match() {
        let ctx = TestContext::default();
        let result = evaluate_formula("IFS(1<0,10,2<0,20)");
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    #[test]
    fn test_switch_match() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("SWITCH(2,1,\"one\",2,\"two\",3,\"three\")"),
            Some(EvalResult::Text("two".to_string()))
        );
    }

    #[test]
    fn test_switch_default() {
        let ctx = TestContext::default();
        assert_eq!(
            evaluate_formula("SWITCH(5,1,\"one\",2,\"two\",\"default\")"),
            Some(EvalResult::Text("default".to_string()))
        );
    }

    #[test]
    fn test_switch_no_match_no_default() {
        let ctx = TestContext::default();
        let result = evaluate_formula("SWITCH(99,1,\"one\",2,\"two\")");
        assert_eq!(result, Some(EvalResult::Error("#N/A".to_string())));
    }

    // ========================================================================
    // F5: Named range resolution tests
    // ========================================================================

    #[derive(Default)]
    struct NamedRangeContext {
        cells: std::collections::HashMap<String, CachedCellValue>,
        named_ranges: std::collections::HashMap<String, CachedCellValue>,
    }

    impl NamedRangeContext {
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

        fn with_named_range(mut self, name: &str, value: f64) -> Self {
            self.named_ranges.insert(
                name.to_string(),
                CachedCellValue {
                    numeric: Some(value),
                    text: None,
                    bool: None,
                },
            );
            self
        }
    }

    impl FormulaContext for NamedRangeContext {
        fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
            self.cells.get(ref_name).cloned()
        }

        fn get_named_range(&self, name: &str) -> Option<CachedCellValue> {
            self.named_ranges.get(name).cloned()
        }

        fn get_range(&self, _range: &str) -> Option<Vec<CachedCellValue>> {
            None
        }
    }

    #[test]
    fn test_named_range_resolves() {
        let ctx = NamedRangeContext::default().with_named_range("RevenueTotal", 500.0);
        let node = parse_formula("RevenueTotal").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(500.0)));
    }

    #[test]
    fn test_named_range_in_expression() {
        let ctx = NamedRangeContext::default()
            .with_named_range("TaxRate", 0.15)
            .with_value("Price", 100.0);
        let node = parse_formula("Price*TaxRate").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(15.0)));
    }

    #[test]
    fn test_named_range_in_sum() {
        let ctx = NamedRangeContext::default()
            .with_named_range("Q1Sales", 100.0)
            .with_named_range("Q2Sales", 200.0);
        let node = parse_formula("SUM(Q1Sales,Q2Sales)").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(300.0)));
    }

    #[test]
    fn test_named_range_no_match_falls_through() {
        let ctx = NamedRangeContext::default();
        let node = parse_formula("UnknownRef").unwrap();
        // Neither cell nor named range exists -> None
        assert_eq!(evaluate(&node, &ctx), None);
    }

    #[test]
    fn test_named_range_text_value() {
        let ctx = NamedRangeContext::default().with_named_range("Status", 1.0);
        let node = parse_formula("Status").unwrap();
        assert_eq!(evaluate(&node, &ctx), Some(EvalResult::Number(1.0)));
    }

    // ========================================================================
    // F6: 3-D reference tests
    // ========================================================================

    #[derive(Default)]
    #[allow(dead_code)]
    struct ThreeDContext {
        cells: std::collections::HashMap<String, CachedCellValue>,
        named_ranges: std::collections::HashMap<String, CachedCellValue>,
        three_d_refs: std::collections::HashMap<String, Vec<CachedCellValue>>,
    }

    #[allow(dead_code)]
    impl ThreeDContext {
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

    impl FormulaContext for ThreeDContext {
        fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
            self.cells.get(ref_name).cloned()
        }

        fn get_range(&self, _range: &str) -> Option<Vec<CachedCellValue>> {
            None
        }

        fn get_cell_at_3d(
            &self,
            _sheet_start: &str,
            _sheet_end: &str,
            cell: &str,
        ) -> Option<Vec<CachedCellValue>> {
            // Simulate 3D ref: return same cell value from each sheet in range
            // For "A1" across "Sheet1:Sheet3", return 3 values
            let count = match cell {
                "A1" => 3,
                _ => return None,
            };
            Some(
                [
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
                ]
                .iter()
                .take(count)
                .cloned()
                .collect(),
            )
        }
    }

    #[test]
    fn test_3d_ref_resolved() {
        let ctx = ThreeDContext::default();
        let node = parse_formula("Sheet1:Sheet3!A1").unwrap();
        let result = evaluate(&node, &ctx);
        // SUM of 10+20+30 = 60
        assert_eq!(result, Some(EvalResult::Number(60.0)));
    }

    #[test]
    fn test_3d_ref_not_supported_returns_error() {
        let ctx = ThreeDContext::default();
        let node = parse_formula("Sheet1:Sheet3!A1").unwrap();
        // Use default context which doesn't support 3D refs
        let default_ctx = TestContext::default();
        let result = evaluate(&node, &default_ctx);
        assert!(matches!(result, Some(EvalResult::Error(ref e)) if e.contains("GP-F-022")));
    }

    // ========================================================================
    // F7: Structured table reference tests
    // ========================================================================

    #[derive(Default)]
    #[allow(dead_code)]
    struct StructuredRefContext {
        cells: std::collections::HashMap<String, CachedCellValue>,
        named_ranges: std::collections::HashMap<String, CachedCellValue>,
        structured_refs: std::collections::HashMap<String, Vec<CachedCellValue>>,
    }

    impl StructuredRefContext {
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

    impl FormulaContext for StructuredRefContext {
        fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
            self.cells.get(ref_name).cloned()
        }

        fn get_range(&self, _range: &str) -> Option<Vec<CachedCellValue>> {
            None
        }

        fn get_structured_ref(&self, _table: &str, column: &str) -> Option<Vec<CachedCellValue>> {
            self.structured_refs.get(column).cloned()
        }
    }

    #[test]
    fn test_structured_ref_simple() {
        let ctx = StructuredRefContext::default()
            .with_value("A1", 10.0)
            .with_value("A2", 20.0)
            .with_value("A3", 30.0);
        let node = parse_formula("[Revenue]").unwrap();
        // StructuredRef with empty table resolves via get_structured_ref("", "Revenue")
        // which won't match our context that only has cells
        // Let's test with a proper structured ref context
        let structured_ctx = StructuredRefContext::default();
        let structured_ctx = {
            let mut ctx = structured_ctx;
            ctx.structured_refs.insert(
                "Revenue".to_string(),
                vec![
                    CachedCellValue {
                        numeric: Some(100.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(200.0),
                        text: None,
                        bool: None,
                    },
                    CachedCellValue {
                        numeric: Some(300.0),
                        text: None,
                        bool: None,
                    },
                ],
            );
            ctx
        };
        let result = evaluate(&node, &structured_ctx);
        // SUM of 100+200+300 = 600
        assert_eq!(result, Some(EvalResult::Number(600.0)));
    }

    #[test]
    fn test_structured_ref_table_column() {
        let ctx = {
            let mut c = StructuredRefContext::default();
            c.structured_refs.insert(
                "ColumnName".to_string(),
                vec![CachedCellValue {
                    numeric: Some(42.0),
                    text: None,
                    bool: None,
                }],
            );
            c
        };
        let node = parse_formula("[ColumnName]").unwrap();
        let result = evaluate(&node, &ctx);
        // Should resolve to 42 via get_structured_ref("", "ColumnName")
        assert_eq!(result, Some(EvalResult::Number(42.0)));
    }

    #[test]
    fn test_structured_ref_not_supported_returns_error() {
        let ctx = StructuredRefContext::default();
        let node = parse_formula("[Revenue]").unwrap();
        let result = evaluate(&node, &ctx);
        assert!(matches!(result, Some(EvalResult::Error(ref e)) if e.contains("GP-F-023")));
    }
}
