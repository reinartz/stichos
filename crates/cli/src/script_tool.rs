// SPDX-License-Identifier: MIT OR Apache-2.0
//! Script tool: execute a sandboxed Rhai script against the document store.

use anyhow::{bail, Context, Result};
use gp_core::ir::IRDocument;
use gp_core::script::{run_script, ScriptConfig, ScriptResult};
use serde_json::Value;

/// Execute a sandboxed Rhai script and return the result as JSON.
pub fn run_script_cli(
    script_path: &std::path::Path,
    ir_json: Option<&str>,
    max_ops: u64,
    max_size: usize,
) -> Result<Value> {
    // Read the script file
    let script_source = std::fs::read_to_string(script_path)
        .with_context(|| format!("failed to read script: {}", script_path.display()))?;

    // Parse optional IR document
    let initial_ir: Option<IRDocument> = ir_json.and_then(|s| serde_json::from_str(s).ok());

    // Build config
    let config = ScriptConfig {
        max_operations: max_ops,
        max_script_size: max_size,
        ..ScriptConfig::default()
    };

    // Execute
    let result = run_script(&config, &script_source, initial_ir.as_ref(), 100);

    // Serialize result
    Ok(script_result_to_json(result, initial_ir))
}

fn script_result_to_json(result: ScriptResult, _initial_ir: Option<IRDocument>) -> Value {
    serde_json::json!({
        "success": result.success,
        "output": result.output,
        "operations_used": result.operations_used,
        "journal_ids": result.journal_ids,
        "diagnostics": result.diagnostics,
    })
}

/// Try to parse IR from a file path.
pub fn try_parse_ir(path: &std::path::Path) -> Result<String> {
    let ir_str = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read IR file: {}", path.display()))?;
    // Validate it parses as IR
    if serde_json::from_str::<IRDocument>(&ir_str).is_ok() {
        Ok(ir_str)
    } else {
        bail!("invalid IR JSON in file: {}", path.display());
    }
}
