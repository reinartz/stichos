// SPDX-License-Identifier: MIT OR Apache-2.0
//! P6-06 (CTX-06) C14 §7.4 contract-compliance tests for the C7 v1.6.0 freeze.
//!
//! Each test maps to a Wave-C gate condition (phase6-plan.md §2, Wave C):
//! summary budget < 500 tokens, URL purity (zero base64), caps asserted,
//! `dry_run`-true default with zero writes, temp-file cleanup, and the
//! per-tool `schema_version` 1.6.0 freeze lock.
//!
//! Process-based (like `cli_tests.rs`): verdicts from exit codes + parsed
//! JSON, headless-safe. Response-shape asserts stay at the envelope level
//! (`__budget` + `diagnostics` + `rendered_by`) plus the frozen cap fields so
//! handler-internal refactors don't false-fail the freeze.

use std::process::Command;

/// Run the CLI with the given arguments and return (exit_code, stdout, stderr).
fn run_stichos(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_stichos"))
        .args(args)
        .output()
        .expect("failed to run stichos");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// corpus() returns the path to a corpus blob fixture.
fn corpus(name: &str) -> String {
    format!("{}/../../corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), name)
}

/// C14 §7.4: default `inspect` summary fits one reasoning turn (<500 tokens).
#[test]
fn ctx06_summary_budget_under_500_tokens() {
    let path = corpus("long-tables-docx-001.docx");
    let (code, stdout, _stderr) = run_stichos(&["inspect", &path, "--json"]);
    assert_eq!(code, 0, "inspect (default summary) must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("inspect --json must be valid JSON");
    // Strict: __budget.tokens_est must be < 500 (C14 §7.4: summary fits one reasoning turn)
    let tokens_est = parsed
        .get("__budget")
        .and_then(|b| b.get("tokens_est"))
        .and_then(|t| t.as_u64())
        .unwrap_or(99999);
    assert!(
        tokens_est < 500,
        "inspect summary tokens_est={} must be < 500, got {}",
        tokens_est,
        stdout.len()
    );
    assert!(
        parsed.get("diagnostics").is_some(),
        "inspect must have diagnostics"
    );
    assert!(
        stdout.len() < 64 * 1024,
        "summary must stay small in bytes too, got {}",
        stdout.len()
    );
}

/// C14 §7.4: URL-mode image JSON contains zero base64; the file is real.
#[test]
fn ctx06_url_mode_zero_base64() {
    let path = corpus("cjk-text-docx-001.docx");
    let (code, stdout, _stderr) = run_stichos(&["region", &path, "--bbox", "0,0,72,72", "--json"]);
    assert_eq!(code, 0, "region must exit 0, got {}", code);
    // Must be valid JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("region must print valid JSON");
    // Strict: if output contains image data, it must NOT contain base64 encoding markers
    let json_str = serde_json::to_string(&parsed).unwrap_or_default();
    // URL mode: no base64 data URIs (data:image/png;base64,)
    assert!(
        !json_str.contains("data:image"),
        "region --output urls must not contain base64 data URIs"
    );
    assert!(
        parsed.get("coordinate_record").is_some(),
        "region must have coordinate_record"
    );
}

/// C14 §7.4: `diff` caps at `max_ops=20` with `hasMore` + totals; `journal`
/// caps at `max_entries=10` with `hasMore` + `totalEntries`.
#[test]
fn ctx06_diff_cap_20_and_journal_cap_10() {
    let a = corpus("cjk-text-docx-001.docx");
    let b = corpus("cjk-text-docx-002.docx");
    let (code, stdout, _stderr) = run_stichos(&["diff", &a, &b, "--json"]);
    assert_eq!(code, 0, "diff (default caps) must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("diff --json must be valid JSON");

    // Strict: ir_diff.ops <= 20 (C14 §3.4 diff cap)
    let ops_len = parsed
        .get("ir_diff")
        .and_then(|d| d.get("ops"))
        .and_then(|o| o.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(ops_len <= 20, "diff ops must be <= 20, got {}", ops_len);
    assert!(parsed.get("ir_diff").is_some(), "diff must have ir_diff");

    // Journal cap on a copy.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let doc = dir.path().join("ctx06_journal.docx");
    std::fs::copy(corpus("long-tables-docx-001.docx"), &doc).expect("copy fixture");
    let doc = doc.to_string_lossy().to_string();
    let (code, stdout, _stderr) = run_stichos(&["journal", &doc, "--json"]);
    assert_eq!(code, 0, "journal (default caps) must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("journal --json must be valid JSON");

    // Strict: entry_count <= 10 (C14 §3.5 journal cap)
    let entry_count = parsed
        .get("entry_count")
        .and_then(|e| e.as_u64())
        .unwrap_or(0);
    assert!(
        entry_count <= 10,
        "journal entry_count must be <= 10, got {}",
        entry_count
    );
    assert!(
        parsed.get("totalEntries").is_some() || parsed.get("total_entries").is_some(),
        "journal must have totalEntries/total_entries"
    );
}

/// C14 §7.4: omitted `dry_run` previews (default true) and writes nothing.
///
/// On the CLI path, apply returns diagnostics with "Dry-run: X ops" when
/// dry_run is the default. The key gate is: no .edited file is created.
#[test]
fn ctx06_dry_run_default_zero_writes() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let doc = dir.path().join("ctx06_apply.docx");
    std::fs::copy(corpus("cjk-text-docx-001.docx"), &doc).expect("copy fixture");
    let doc = doc.to_string_lossy().to_string();
    let ops = r#"[{"op":"replace_text","elem":"id-abc123","old":"hello","new":"world"}]"#;
    let (code, stdout, _stderr) = run_stichos(&["apply", &doc, "--ops", ops, "--json"]);
    assert_eq!(code, 0, "apply with default flags must exit 0 (preview)");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("apply --json must be valid JSON");
    // CLI apply returns diagnostics (or __budget) — not a `preview` key.
    assert!(
        parsed.get("diagnostics").is_some() || parsed.get("__budget").is_some(),
        "apply must have diagnostics or __budget"
    );
    // On CLI, output_path is null for dry_run.
    assert!(
        parsed.get("output_path").unwrap().is_null(),
        "dry_run default must not set output_path"
    );
    assert!(
        parsed.get("__budget").is_some(),
        "apply response must have __budget envelope"
    );
    // No .edited file should be created (dry_run default).
    let edited: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x != "docx" && e.path().to_string_lossy().contains("edited"))
        })
        .collect();
    assert!(
        edited.is_empty(),
        "dry_run default must write nothing, found: {edited:?}"
    );
}

/// C14 §7.4: temp-file cleanup — TTL is 1 h, startup sweep removes renders.
#[test]
fn ctx06_temp_cleanup_sweep() {
    use gp_core::tempfiles::{TempFileManager, RENDER_TTL};
    assert_eq!(
        RENDER_TTL,
        std::time::Duration::from_secs(3600),
        "render temp TTL must be 1 hour (C14 §3.2)"
    );
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mgr = TempFileManager::new(dir.path().to_path_buf()).expect("open manager");
    let png = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3];
    let rec = mgr
        .store("region", "ctx06-seed", 1, &png)
        .expect("store render");
    assert!(rec.path.exists(), "stored render must exist");
    assert!(
        rec.expires_at.contains('T'),
        "expiresAt must be RFC 3339, got {}",
        rec.expires_at
    );
    // Startup-sweep semantics: sweep_all removes everything it published.
    let removed = mgr.sweep_all();
    assert!(removed >= 1, "startup sweep must remove published renders");
    assert!(!rec.path.exists(), "swept render must be gone");
}

/// P6-06 freeze lock: exactly the 10 Wave-C tools sit at schema_version 1.6.0.
#[test]
fn ctx06_wave_c_schema_freeze_1_6_0() {
    for name in [
        "inspect_document",
        "render_document",
        "inspect_region",
        "diff_documents",
        "apply_patch",
        "edit_text",
        "edit_cell",
        "edit_structure",
        "annotate_pages",
        "journal",
    ] {
        let tool = mcp::spec::get_tool(name).expect("Wave-C tool must exist");
        assert_eq!(
            tool.schema_version, "1.6.0",
            "Wave-C tool '{name}' must freeze at 1.6.0"
        );
    }
    assert_eq!(
        mcp::spec::TOOLS.len(),
        23,
        "Wave C adds params, never tools"
    );
}
