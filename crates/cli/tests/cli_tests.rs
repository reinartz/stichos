//! Integration tests for the stichos CLI.
//!
//! These tests verify:
//! - `apply` subcommand exists (Phase 2 Wave E)
//! - Help output contains all expected commands
//! - JSON output is valid for inspect, diff, doctor
//! - Transport validation on serve
//! - Server startup for stdio/HTTP modes
//! - Exit codes

use std::io::Write;
use std::process::Command;

/// Run the CLI with the given arguments and return (exit_code, stdout, stderr).
fn run_stichos(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_stichos"))
        .args(args)
        .output()
        .expect("failed to run stichos");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.code().unwrap_or(-1), stdout, stderr)
}

/// Run the CLI with stdio input and return (exit_code, stdout, stderr).
fn run_stichos_with_stdin(args: &[&str], stdin: &[u8]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stichos"))
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn stichos");

    {
        let stdin_handle = child.stdin.as_mut().expect("failed to get stdin");
        stdin_handle
            .write_all(stdin)
            .expect("failed to write to stdin");
    }

    let output = child.wait_with_output().expect("failed to wait on child");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.code().unwrap_or(-1), stdout, stderr)
}

/// `apply` MUST be a subcommand (Phase 2 Wave E).
#[test]
fn test_apply_command() {
    let (_code, stdout, stderr) = run_stichos(&["--help"]);
    let output = format!("{}{}", stdout, stderr);
    assert!(
        output.contains("apply"),
        "apply must be a CLI verb in Phase 2 Wave E"
    );
}

/// Help output lists all 6 expected commands including apply.
#[test]
fn test_help_snapshot() {
    let (code, stdout, stderr) = run_stichos(&["--help"]);
    let output = format!("{}{}", stdout, stderr);
    assert_eq!(code, 0);
    assert!(output.contains("inspect"));
    assert!(output.contains("render"));
    assert!(output.contains("diff"));
    assert!(output.contains("doctor"));
    assert!(output.contains("serve"));
    assert!(output.contains("apply"));
}

/// `inspect --json` outputs valid JSON with tool/status fields.
#[test]
fn test_inspect_json() {
    // Use a corpus file that exists
    let corpus_path = format!(
        "{}/../../corpus/blobs/cjk-text-docx-001.docx",
        env!("CARGO_MANIFEST_DIR")
    );
    let (code, stdout, _stderr) = run_stichos(&["inspect", &corpus_path, "--json"]);
    assert_eq!(code, 0, "inspect should succeed with valid docx");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("inspect --json did not produce valid JSON");
    // Verify required fields per P1-48 contract
    assert_eq!(parsed["tool"].as_str().unwrap(), "inspect_document");
    assert!(parsed["path"].is_string());
    assert_eq!(parsed["ir_version"].as_str().unwrap(), "1.0.0");
    assert!(parsed["document_type"].is_string());
    assert!(parsed["structure"].is_object());
    assert!(parsed["element_ids"].is_array());
    assert!(parsed["total_elements"].is_number());
    assert!(parsed["coordinates"].is_array());
    assert!(parsed["fidelity_budget"].is_object());

    // C2 contract: element IDs must have kind/path/id
    for entry in parsed["element_ids"].as_array().unwrap() {
        assert!(entry["kind"].is_string(), "Missing kind in: {}", entry);
        assert!(entry["path"].is_string(), "Missing path in: {}", entry);
        assert!(entry["id"].is_string(), "Missing id in: {}", entry);
    }

    // C4 contract: fidelity_budget must be present
    assert!(parsed["fidelity_budget"]["confidence"].is_number());
}

/// `inspect` without `--json` outputs text to stderr.
#[test]
fn test_inspect_text() {
    // Use a corpus file that exists
    let corpus_path = format!(
        "{}/../../corpus/blobs/cjk-text-docx-001.docx",
        env!("CARGO_MANIFEST_DIR")
    );
    let (code, _stdout, stderr) = run_stichos(&["inspect", &corpus_path]);
    assert_eq!(code, 0, "inspect should succeed with valid docx");
    assert!(
        stderr.contains("inspect_document") || stderr.contains("elements"),
        "inspect text output should contain tool name or element count: {}",
        stderr
    );
}

/// `doctor --json` outputs valid JSON with engines and system info.
#[test]
fn test_doctor_json() {
    let (code, stdout, _stderr) = run_stichos(&["doctor", "--json"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("doctor --json did not produce valid JSON");
    // engines is an array of engine capability entries
    assert!(parsed["engines"].is_array());
    assert!(!parsed["engines"].as_array().unwrap().is_empty());
    // Each engine has id, version, available, capabilities
    let engines = parsed["engines"].as_array().unwrap();
    let first = &engines[0];
    assert!(first["id"].is_string());
    assert!(first["available"].is_boolean());
    assert!(first["capabilities"].is_array());
    // system info present
    assert!(parsed["system"].is_object());
    assert!(parsed["fonts"].is_object());
}

/// `diff --json` outputs valid JSON with IR structural diff.
#[test]
fn test_diff_json() {
    let corpus_path = format!(
        "{}/../../corpus/blobs/cjk-text-docx-001.docx",
        env!("CARGO_MANIFEST_DIR")
    );
    // Diff a file with itself — should be identical
    let (code, stdout, _stderr) = run_stichos(&["diff", &corpus_path, &corpus_path, "--json"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("diff --json did not produce valid JSON");
    assert_eq!(parsed["tool"].as_str().unwrap(), "diff_documents");
    assert!(parsed["path_a"].is_string());
    assert!(parsed["path_b"].is_string());
    assert!(parsed["ir_diff"].is_object());
    assert!(parsed["ir_diff"]["identical"].as_bool().unwrap());
    assert_eq!(parsed["ir_diff"]["total_ops"].as_u64().unwrap(), 0);
}

/// `serve --transport stdio` starts the MCP server and exits on EOF.
#[test]
fn test_serve_stdio() {
    // Pipe empty stdin — server prints startup message then exits on EOF
    let (code, _stdout, stderr) = run_stichos_with_stdin(&["serve", "--transport", "stdio"], &[]);
    assert_eq!(code, 0, "stdio server should start and exit cleanly on EOF");
    assert!(
        stderr.contains("stdio"),
        "should mention stdio mode: {}",
        stderr
    );
    assert!(
        stderr.contains("MCP server"),
        "should start MCP server: {}",
        stderr
    );
}

/// `serve --transport unknown` exits with code 1 and error message.
#[test]
fn test_serve_invalid_transport() {
    let (code, _stdout, stderr) = run_stichos(&["serve", "--transport", "unknown"]);
    assert_eq!(code, 1);
    assert!(
        stderr.contains("unknown transport"),
        "error message should mention unknown transport: {}",
        stderr
    );
}

/// `serve --transport invalid` exits with code 1.
#[test]
fn test_exit_code_error() {
    let (code, _stdout, _stderr) = run_stichos(&["serve", "--transport", "invalid"]);
    assert_eq!(code, 1);
}

/// A successful command exits 0.
#[test]
fn test_exit_code_success() {
    let (code, _stdout, _stderr) = run_stichos(&["doctor"]);
    assert_eq!(code, 0);
}
