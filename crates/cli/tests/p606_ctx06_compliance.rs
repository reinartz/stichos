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

fn corpus(name: &str) -> String {
    format!("{}/../../corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), name)
}

fn require_envelope(parsed: &serde_json::Value, tool: &str) {
    assert!(
        parsed.get("__budget").is_some(),
        "{tool}: missing __budget envelope"
    );
    assert!(
        parsed.get("diagnostics").is_some(),
        "{tool}: missing diagnostics envelope"
    );
    assert!(
        parsed.get("rendered_by").is_some(),
        "{tool}: missing rendered_by envelope"
    );
}

/// C14 §7.4: default `inspect` summary fits one reasoning turn (<500 tokens).
#[test]
fn ctx06_summary_budget_under_500_tokens() {
    let path = corpus("long-tables-docx-001.docx");
    let (code, stdout, _stderr) = run_stichos(&["inspect", &path, "--json"]);
    assert_eq!(code, 0, "inspect (default summary) must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("inspect --json must be valid JSON");
    require_envelope(&parsed, "inspect_document");
    let tokens = parsed["__budget"]["tokens_est"]
        .as_u64()
        .expect("__budget.tokens_est must be a number") as usize;
    assert!(
        tokens < 500,
        "summary budget {tokens} must be < 500 tokens (C14 §7.4)"
    );
    assert!(
        stdout.len() < 64 * 1024,
        "summary must stay small in bytes too, got {}",
        stdout.len()
    );
    assert!(
        parsed.get("hasMore").is_some(),
        "summary must carry hasMore for pagination"
    );
}

/// C14 §7.4: URL-mode image JSON contains zero base64; the file is real.
#[test]
fn ctx06_url_mode_zero_base64() {
    let path = corpus("cjk-text-docx-001.docx");
    let (code, stdout, _stderr) =
        run_stichos(&["region", &path, "--bbox", "0,0,72,72", "--output", "urls"]);
    assert_eq!(code, 0, "region --output urls must exit 0");
    assert!(
        !stdout.contains("base64"),
        "URL-mode JSON must contain zero base64 (gate-greppable)"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("region must print valid JSON");
    require_envelope(&parsed, "inspect_region");
    let url = parsed["image"]["url"]
        .as_str()
        .expect("urls mode must carry image.url");
    assert!(
        parsed["image"].get("expiresAt").is_some(),
        "urls mode must carry image.expiresAt"
    );
    assert!(
        std::path::Path::new(url).exists(),
        "url file must exist on disk: {url}"
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
    require_envelope(&parsed, "diff_documents");
    let ir = &parsed["ir_diff"];
    let ops = ir["ops"].as_array().expect("ir_diff.ops must be an array");
    let total = ir["totalOperations"]
        .as_u64()
        .expect("ir_diff.totalOperations must be a number") as usize;
    assert!(total > 20, "fixture pair must exceed the cap, got {total}");
    assert_eq!(
        ops.len(),
        20,
        "default diff must return exactly the 20-op cap, got {}",
        ops.len()
    );
    assert_eq!(ir["hasMore"], true, "capped diff must set hasMore");
    assert_eq!(
        ir["summary"]["totalOperations"], total as u64,
        "summary.totalOperations must report the full count"
    );

    // Journal cap on an isolated copy (fresh session: assert the mechanism —
    // bounded length, count consistency, envelope — not pre-seeded state).
    let dir = tempfile::TempDir::new().expect("tempdir");
    let doc = dir.path().join("ctx06_journal.docx");
    std::fs::copy(corpus("long-tables-docx-001.docx"), &doc).expect("copy fixture");
    let doc = doc.to_string_lossy().to_string();
    let (code, stdout, _stderr) = run_stichos(&["journal", &doc, "--json"]);
    assert_eq!(code, 0, "journal (default caps) must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("journal --json must be valid JSON");
    require_envelope(&parsed, "journal");
    let entries = parsed["entries"]
        .as_array()
        .expect("journal.entries must be an array");
    assert!(
        entries.len() <= 10,
        "default journal must return at most 10 entries, got {}",
        entries.len()
    );
    assert_eq!(
        parsed["entry_count"].as_u64().unwrap() as usize,
        entries.len(),
        "entry_count must match returned length"
    );
    assert!(
        parsed["totalEntries"].as_u64().unwrap() as usize >= entries.len(),
        "totalEntries must cover the returned entries"
    );
    assert!(
        parsed.get("hasMore").is_some(),
        "journal must carry hasMore"
    );
}

/// C14 §7.4: omitted `dry_run` previews (default true) and writes nothing.
#[test]
fn ctx06_dry_run_default_zero_writes() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let doc = dir.path().join("ctx06_apply.docx");
    std::fs::copy(corpus("cjk-text-docx-001.docx"), &doc).expect("copy fixture");
    let doc = doc.to_string_lossy().to_string();
    let ops = r#"[{"op":"replace_text","elem":"id-abc123","old":"hello","new":"world"}]"#;
    let (code, stdout, _stderr) = run_stichos(&["apply", &doc, "--ops", ops, "--json"]);
    assert_eq!(code, 0, "apply with omitted flags must exit 0 (preview)");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("apply --json must be valid JSON");
    require_envelope(&parsed, "apply_patch");
    let preview = parsed
        .get("preview")
        .expect("default apply must return preview");
    // P6-01 PreviewSummary wire shape: snake_case keys, capped at 20 ops.
    for key in [
        "estimated_token_change",
        "elements_affected",
        "diff_summary",
        "operations",
        "has_more",
        "total_operations",
    ] {
        assert!(
            preview.get(key).is_some(),
            "preview must carry snake_case key '{key}' (P6-06 wire freeze)"
        );
    }
    assert!(
        preview["operations"].as_array().unwrap().len() <= 20,
        "preview operations must be capped at 20"
    );
    assert!(
        parsed.get("output_path").unwrap().is_null(),
        "dry_run default must not set output_path"
    );
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
    // The source file itself must be byte-identical (no in-place mutation).
    let before = std::fs::read(corpus("cjk-text-docx-001.docx")).expect("read fixture");
    let after = std::fs::read(&doc).expect("read target");
    assert_eq!(
        before, after,
        "dry_run default must leave the input untouched"
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
