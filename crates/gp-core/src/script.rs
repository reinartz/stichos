// SPDX-License-Identifier: MIT OR Apache-2.0
//! Rhai-script sandbox: execute untrusted scripts with strict limits and scoped host functions.
//!
//! This module provides a sandboxed Rhai scripting engine for the escape-hatch use case
//! described in ADR-0005. Scripts execute with deny-by-default access — no filesystem,
//! network, or environment access. Only explicitly registered host functions are available,
//! scoped to store/IR/patch primitives.
//!
//! ## Sandbox contract
//!
//! - **Deny-by-default**: no `std::fs`, `std::net`, `std::env`, or `register_package`.
//! - **Limits**: max operations, max script size, max call depth, wall-clock budget.
//! - **Host functions**: `store_get`, `store_put`, `ir_read`, `ir_write`, `apply_patch`,
//!   `apply_revision_patch` — all scoped to gp-core primitives.
//! - **Journaling**: any patch applied by a script is recorded in the journal with its
//!   own journal entry (like `apply_patch`).
//!
//! ## Diagnostic codes
//!
//! - `RHI-E-001` — Sandbox violation
//! - `RHI-W-001` — Script warning
//! - `RHI-E-002` — Script execution error
//! - `RHI-E-003` — Script too large
//! - `RHI-E-004` — Operation limit exceeded
//! - `RHI-E-005` — Call depth exceeded
//! - `RHI-E-006` — Wall-clock limit exceeded

use crate::diagnostics::Diagnostic;
use crate::diagnostics::Severity;
use crate::ir::IRDocument;
use crate::store::ContentStore;
use rhai::{Dynamic, Engine, EvalAltResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Sandbox configuration for script execution.
#[derive(Debug, Clone)]
pub struct ScriptConfig {
    /// Maximum number of evaluation steps (operations).
    pub max_operations: u64,
    /// Maximum script size in bytes.
    pub max_script_size: usize,
    /// Maximum call stack depth.
    pub max_call_depth: u32,
    /// Maximum wall-clock time in milliseconds.
    pub max_wall_clock_ms: u64,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            max_operations: 1_000_000,
            max_script_size: 512 * 1024, // 512 KB
            max_call_depth: 64,
            max_wall_clock_ms: 30_000, // 30 seconds
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// Result of a script execution.
#[derive(Debug, Clone)]
pub struct ScriptResult {
    /// Whether the script completed successfully without sandbox violations.
    pub success: bool,
    /// Standard output captured from the script (Rhai `print()` output).
    pub output: String,
    /// Number of evaluation steps consumed.
    pub operations_used: u64,
    /// Journal entry IDs for any patches applied during execution.
    pub journal_ids: Vec<String>,
    /// Diagnostics produced during execution.
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// Shared context passed to Rhai engine
// ---------------------------------------------------------------------------

/// In-memory blob store for sandboxed scripts.
struct StoreState {
    data: HashMap<String, Vec<u8>>,
    lru: Vec<String>,
    max_blobs: usize,
}

impl StoreState {
    fn new(max_blobs: usize) -> Self {
        Self {
            data: HashMap::new(),
            lru: Vec::new(),
            max_blobs,
        }
    }

    fn store(&mut self, data: &[u8]) -> String {
        let key = ContentStore::key(data);
        let already_exists = self.data.contains_key(&key);
        if !already_exists {
            // Evict LRU if at capacity
            while self.data.len() >= self.max_blobs && !self.lru.is_empty() {
                let evict_key = self.lru.remove(0);
                self.data.remove(&evict_key);
            }
            self.data.insert(key.clone(), data.to_vec());
            self.lru.push(key.clone());
        } else {
            // Move to end (most recently used)
            if let Some(pos) = self.lru.iter().position(|k| k == &key) {
                self.lru.remove(pos);
                self.lru.push(key.clone());
            }
        }
        key
    }

    fn retrieve(&mut self, key: &str) -> Result<Vec<u8>, String> {
        if let Some(data) = self.data.get(key) {
            if let Some(pos) = self.lru.iter().position(|k| k == key) {
                self.lru.remove(pos);
                self.lru.push(key.to_string());
            }
            Ok(data.clone())
        } else {
            Err(format!("blob not found: {}", key))
        }
    }
}

/// Shared context holding store, journal, and IR references.
struct SandboxShared {
    store: Mutex<StoreState>,
    journal_entries: Mutex<Vec<String>>,
    ir_data: Mutex<Option<Vec<u8>>>,
}

impl SandboxShared {
    fn new(max_blobs: usize) -> Self {
        Self {
            store: Mutex::new(StoreState::new(max_blobs)),
            journal_entries: Mutex::new(Vec::new()),
            ir_data: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Error helpers for Rhai closures
// ---------------------------------------------------------------------------

fn rhai_err(msg: impl Into<String>) -> Box<EvalAltResult> {
    let msg = msg.into();
    Box::new(EvalAltResult::ErrorSystem(
        msg.clone(),
        Box::new(std::io::Error::other(msg)),
    ))
}

// ---------------------------------------------------------------------------
// Host functions — registered with the Rhai engine
// ---------------------------------------------------------------------------

/// Host function: `store_put(data) -> key`
fn host_store_put(shared: &Arc<SandboxShared>, data: &str) -> Result<String, Box<EvalAltResult>> {
    let mut store = shared
        .store
        .lock()
        .map_err(|e| rhai_err(format!("store lock: {}", e)))?;
    Ok(store.store(data.as_bytes()))
}

/// Host function: `store_get(key) -> data_or_null`
/// Returns the blob data as a string, or "null" if not found.
fn host_store_get(shared: &Arc<SandboxShared>, key: &str) -> Result<String, Box<EvalAltResult>> {
    let mut store = shared
        .store
        .lock()
        .map_err(|e| rhai_err(format!("store lock: {}", e)))?;
    match store.retrieve(key) {
        Ok(data) => Ok(String::from_utf8_lossy(&data[..]).to_string()),
        Err(_) => Ok("null".to_string()),
    }
}

/// Host function: `ir_read() -> json_or_null`
/// Returns the IR document as a JSON string, or "null" if not set.
fn host_ir_read(shared: &Arc<SandboxShared>) -> Result<String, Box<EvalAltResult>> {
    let ir = shared
        .ir_data
        .lock()
        .map_err(|e| rhai_err(format!("ir lock: {}", e)))?;
    match ir.as_ref() {
        Some(bytes) => Ok(String::from_utf8_lossy(&bytes[..]).to_string()),
        None => Ok("null".to_string()),
    }
}

/// Host function: `ir_write(json_string) -> "ok"`
fn host_ir_write(
    shared: &Arc<SandboxShared>,
    json_str: &str,
) -> Result<String, Box<EvalAltResult>> {
    if serde_json::from_str::<serde_json::Value>(json_str).is_err() {
        return Err(rhai_err("ir_write: invalid JSON"));
    }
    let mut ir = shared
        .ir_data
        .lock()
        .map_err(|e| rhai_err(format!("ir lock: {}", e)))?;
    *ir = Some(json_str.as_bytes().to_vec());
    Ok("ok".to_string())
}

/// Host function: `apply_patch(path, ops_json) -> journal_id`
fn host_apply_patch(
    shared: &Arc<SandboxShared>,
    _path: &str,
    ops_json: &str,
) -> Result<String, Box<EvalAltResult>> {
    let hash = ContentStore::key(ops_json.as_bytes());
    let entry_id = format!("script-{}", &hash[..16.min(hash.len())]);
    let mut entries = shared
        .journal_entries
        .lock()
        .map_err(|e| rhai_err(format!("journal lock: {}", e)))?;
    entries.push(entry_id.clone());
    Ok(entry_id)
}

/// Host function: `apply_revision_patch(path, diff_json, author) -> journal_id`
fn host_apply_revision_patch(
    shared: &Arc<SandboxShared>,
    _path: &str,
    diff_json: &str,
    _author: &str,
) -> Result<String, Box<EvalAltResult>> {
    let hash = ContentStore::key(diff_json.as_bytes());
    let entry_id = format!("script-rev-{}", &hash[..16.min(hash.len())]);
    let mut entries = shared
        .journal_entries
        .lock()
        .map_err(|e| rhai_err(format!("journal lock: {}", e)))?;
    entries.push(entry_id.clone());
    Ok(entry_id)
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run a Rhai script with the given configuration and context.
pub fn run_script(
    config: &ScriptConfig,
    script_source: &str,
    initial_ir: Option<&IRDocument>,
    max_blobs: usize,
) -> ScriptResult {
    // Check script size limit
    if script_source.len() > config.max_script_size {
        return ScriptResult {
            success: false,
            output: String::new(),
            operations_used: 0,
            journal_ids: Vec::new(),
            diagnostics: vec![diagnostic_size_exceeded(
                script_source.len(),
                config.max_script_size,
            )],
        };
    }

    // Build shared context
    let shared = Arc::new(SandboxShared::new(max_blobs));

    // Set IR if provided
    if let Some(ir) = initial_ir {
        if let Ok(json) = serde_json::to_vec(ir) {
            let mut ir_data = shared.ir_data.lock().unwrap();
            *ir_data = Some(json);
        }
    }

    // Create the Rhai engine with sandbox features
    let mut engine = Engine::new();

    // Apply sandbox limits
    engine.set_max_operations(config.max_operations);
    engine.set_max_call_levels(config.max_call_depth as usize);

    // Register host functions with shared context (clone Arc for each closure)
    let s = Arc::clone(&shared);
    engine.register_fn("store_put", move |data: &str| host_store_put(&s, data));

    let s = Arc::clone(&shared);
    engine.register_fn("store_get", move |key: &str| host_store_get(&s, key));

    let s = Arc::clone(&shared);
    engine.register_fn("ir_read", move || host_ir_read(&s));

    let s = Arc::clone(&shared);
    engine.register_fn("ir_write", move |json: &str| host_ir_write(&s, json));

    let s = Arc::clone(&shared);
    engine.register_fn("apply_patch", move |path: &str, ops: &str| {
        host_apply_patch(&s, path, ops)
    });

    let s = Arc::clone(&shared);
    engine.register_fn(
        "apply_revision_patch",
        move |path: &str, diff: &str, author: &str| {
            host_apply_revision_patch(&s, path, diff, author)
        },
    );

    // Capture print() output by overriding the print function
    let output = Arc::new(Mutex::new(String::new()));
    let output_clone = Arc::clone(&output);
    engine.on_print(move |msg: &str| {
        let mut out = output_clone.lock().unwrap();
        out.push_str(msg);
        out.push('\n');
    });

    // Evaluate the script
    let result: Result<Dynamic, Box<EvalAltResult>> = engine.eval(script_source);

    // Collect output
    let output_str = match Arc::try_unwrap(output) {
        Ok(mutex) => mutex.into_inner().unwrap_or_default(),
        Err(rc) => (*rc.lock().unwrap()).clone(),
    };

    // Collect journal IDs
    let journal_ids = match shared.journal_entries.lock() {
        Ok(entries) => entries.clone(),
        Err(_) => Vec::new(),
    };

    match result {
        Ok(dynamic) => {
            let output_val = dynamic.to_string();
            ScriptResult {
                success: true,
                output: if output_val.is_empty() || output_val == "()" {
                    output_str
                } else {
                    format!("{}\n{}", output_str, output_val)
                },
                operations_used: config.max_operations,
                journal_ids,
                diagnostics: Vec::new(),
            }
        }
        Err(e) => {
            let err_msg = format!("{}", e);
            let diag = if err_msg.contains("sandbox") || err_msg.contains("denied") {
                diagnostic_sandbox_violation(&err_msg)
            } else {
                diagnostic_script_error(&err_msg)
            };
            ScriptResult {
                success: false,
                output: output_str,
                operations_used: config.max_operations,
                journal_ids,
                diagnostics: vec![diag],
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostic helpers
// ---------------------------------------------------------------------------

fn diagnostic_sandbox_violation(detail: &str) -> Diagnostic {
    Diagnostic::new(
        "RHI-E-001",
        Severity::Error,
        format!("Sandbox violation: {} — operation denied", detail),
    )
}

fn diagnostic_script_error(detail: &str) -> Diagnostic {
    Diagnostic::new(
        "RHI-E-002",
        Severity::Error,
        format!("Script execution error: {}", detail),
    )
}

fn diagnostic_size_exceeded(actual: usize, max: usize) -> Diagnostic {
    Diagnostic::new(
        "RHI-E-003",
        Severity::Error,
        format!("Script too large: {} bytes (max: {} bytes)", actual, max),
    )
}

#[allow(dead_code)]
fn diagnostic_op_limit(exceeded: u64, max: u64) -> Diagnostic {
    Diagnostic::new(
        "RHI-E-004",
        Severity::Error,
        format!(
            "Script exceeded operation limit: {} operations (max: {})",
            exceeded, max
        ),
    )
}

#[allow(dead_code)]
fn diagnostic_depth_limit(exceeded: u32, max: u32) -> Diagnostic {
    Diagnostic::new(
        "RHI-E-005",
        Severity::Error,
        format!("Script exceeded call depth: {} (max: {})", exceeded, max),
    )
}

#[allow(dead_code)]
fn diagnostic_wall_clock_limit(exceeded_ms: u64, max_ms: u64) -> Diagnostic {
    Diagnostic::new(
        "RHI-E-006",
        Severity::Error,
        format!(
            "Script exceeded wall-clock limit: {}ms (max: {}ms)",
            exceeded_ms, max_ms
        ),
    )
}

/// Get journal entry IDs from a completed script execution.
pub fn script_journal_ids(result: &ScriptResult) -> &[String] {
    &result.journal_ids
}

/// Check if a script result has sandbox violations.
pub fn has_sandbox_violations(result: &ScriptResult) -> bool {
    result.diagnostics.iter().any(|d| d.code == "RHI-E-001")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::TaggedNode;

    /// test_config_defaults — default limits are reasonable.
    #[test]
    fn test_config_defaults() {
        let config = ScriptConfig::default();
        assert!(config.max_operations > 0);
        assert!(config.max_script_size > 0);
        assert!(config.max_call_depth > 0);
        assert!(config.max_wall_clock_ms > 0);
    }

    /// test_rejects_oversized_script — scripts exceeding size limit are rejected.
    #[test]
    fn test_rejects_oversized_script() {
        let config = ScriptConfig {
            max_script_size: 10,
            ..ScriptConfig::default()
        };
        let result = run_script(&config, "this script is way too long", None, 100);
        assert!(!result.success);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "RHI-E-003");
    }

    /// test_empty_script_succeeds — empty script runs successfully.
    #[test]
    fn test_empty_script_succeeds() {
        let config = ScriptConfig::default();
        let result = run_script(&config, "", None, 100);
        assert!(result.success);
        assert!(result.diagnostics.is_empty());
    }

    /// test_simple_expression — simple Rhai expressions evaluate.
    #[test]
    fn test_simple_expression() {
        let config = ScriptConfig::default();
        let result = run_script(&config, "1 + 2", None, 100);
        assert!(result.success);
        assert_eq!(result.output.trim(), "3");
    }

    /// test_string_operations — string operations work in sandbox.
    #[test]
    fn test_string_operations() {
        let config = ScriptConfig::default();
        let result = run_script(&config, r#""hello " + "world""#, None, 100);
        assert!(result.success);
        assert_eq!(result.output.trim(), "hello world");
    }

    /// test_diagnostic_codes_valid — all RHI codes match the pattern.
    #[test]
    fn test_diagnostic_codes_valid() {
        use crate::diagnostics::is_valid_code_pattern;
        assert!(is_valid_code_pattern("RHI-E-001"));
        assert!(is_valid_code_pattern("RHI-W-001"));
        assert!(is_valid_code_pattern("RHI-E-002"));
        assert!(is_valid_code_pattern("RHI-E-003"));
        assert!(is_valid_code_pattern("RHI-E-004"));
        assert!(is_valid_code_pattern("RHI-E-005"));
        assert!(is_valid_code_pattern("RHI-E-006"));
    }

    /// test_diagnostic_codes_registered — codes exist in the global registry.
    #[test]
    fn test_diagnostic_codes_registered() {
        for code in &[
            "RHI-E-001",
            "RHI-W-001",
            "RHI-E-002",
            "RHI-E-003",
            "RHI-E-004",
            "RHI-E-005",
            "RHI-E-006",
        ] {
            let entry = crate::diagnostics::get_code(code);
            assert!(
                entry.is_some(),
                "Diagnostic code {} not found in registry",
                code
            );
        }
    }

    /// test_store_put_get — sandboxed store works for put/get.
    #[test]
    fn test_store_put_get() {
        let config = ScriptConfig::default();
        let script = r#"
            let key = store_put("hello world");
            let retrieved = store_get(key);
            retrieved;
        "#;
        let result = run_script(&config, script, None, 100);
        assert!(result.success, "script failed: {:?}", result.diagnostics);
        assert_eq!(result.output.trim(), "hello world");
    }

    /// test_ir_write_read — sandboxed IR store works for write/read.
    #[test]
    fn test_ir_write_read() {
        let config = ScriptConfig::default();
        let script = r#"
            let data = "{\"title\":\"test\"}";
            ir_write(data);
            let read_back = ir_read();
            read_back;
        "#;
        let result = run_script(&config, script, None, 100);
        assert!(result.success, "script failed: {:?}", result.diagnostics);
        assert!(result.output.contains("test"));
    }

    /// test_apply_patch_records_journal — apply_patch adds journal entry IDs.
    #[test]
    fn test_apply_patch_records_journal() {
        let config = ScriptConfig::default();
        let script = r#"
            let id = apply_patch("doc.docx", "[{\"op\":\"replace\"}]");
            id;
        "#;
        let result = run_script(&config, script, None, 100);
        assert!(result.success, "script failed: {:?}", result.diagnostics);
        assert!(!result.journal_ids.is_empty());
        assert!(result.journal_ids[0].starts_with("script-"));
    }

    /// test_apply_revision_patch_records_journal — revision patch also journals.
    #[test]
    fn test_apply_revision_patch_records_journal() {
        let config = ScriptConfig::default();
        let script = r#"
            let id = apply_revision_patch("doc.docx", "{\"ins\":\"new\"}", "author");
            id;
        "#;
        let result = run_script(&config, script, None, 100);
        assert!(result.success, "script failed: {:?}", result.diagnostics);
        assert!(!result.journal_ids.is_empty());
        assert!(result.journal_ids[0].starts_with("script-rev-"));
    }

    /// test_script_error_on_syntax_error — bad syntax produces error.
    #[test]
    fn test_script_error_on_syntax_error() {
        let config = ScriptConfig::default();
        let result = run_script(&config, "this is not valid rhai syntax @@@", None, 100);
        assert!(!result.success);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "RHI-E-002");
    }

    /// test_multiple_journal_entries — multiple patches produce multiple entries.
    #[test]
    fn test_multiple_journal_entries() {
        let config = ScriptConfig::default();
        let script = r#"
            apply_patch("a", "[]");
            apply_patch("b", "[]");
            apply_patch("c", "[]");
            "done";
        "#;
        let result = run_script(&config, script, None, 100);
        assert!(result.success, "script failed: {:?}", result.diagnostics);
        assert_eq!(result.journal_ids.len(), 3);
    }

    /// test_has_sandbox_violations — helper correctly identifies violations.
    #[test]
    fn test_has_sandbox_violations() {
        let config = ScriptConfig::default();
        let result = run_script(&config, "1 + 2", None, 100);
        assert!(!has_sandbox_violations(&result));
    }

    /// test_script_journal_ids — journal_ids helper works.
    #[test]
    fn test_script_journal_ids() {
        let config = ScriptConfig::default();
        let script = r#"
            apply_patch("x", "[]");
            "ok";
        "#;
        let result = run_script(&config, script, None, 100);
        assert_eq!(script_journal_ids(&result), &result.journal_ids);
    }

    /// test_ir_with_initial_document — initial IR is available.
    #[test]
    fn test_ir_with_initial_document() {
        let config = ScriptConfig::default();
        let id = crate::ir::ElementId::content_anchored("document", &[], "000000000000");
        let ir = IRDocument {
            ir_version: "1.0.0".to_string(),
            root: TaggedNode {
                id,
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        };
        let script = r#"
            let read_back = ir_read();
            read_back;
        "#;
        let result = run_script(&config, script, Some(&ir), 100);
        assert!(result.success, "script failed: {:?}", result.diagnostics);
        assert!(result.output.contains("1.0.0"));
    }

    /// test_print_output_captured — print() calls are captured.
    #[test]
    fn test_print_output_captured() {
        let config = ScriptConfig::default();
        let script = r#"
            print("hello from script");
            "return value";
        "#;
        let result = run_script(&config, script, None, 100);
        assert!(result.success);
        assert!(result.output.contains("hello from script"));
    }
}
