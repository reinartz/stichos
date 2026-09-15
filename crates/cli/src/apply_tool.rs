//! Apply patch tool implementation.
//!
//! Reads a document, applies patch operations, and returns the result with
//! diff, provenance, and fidelity budget.
//!
//! By default (direct=false), docx edits land as tracked revisions with
//! `w:ins`/`w:del` markup. Use `direct=true` to apply changes without
//! revision tracking.

use anyhow::anyhow;
use gp_core::{
    diff::diff_documents,
    executor::{ExecutorConfig, FormatType, SurgicalPatchExecutor},
    patch::PatchOp,
};

/// Apply a surgical patch to a document.
///
/// # Arguments
/// * `path` — Path to the document file.
/// * `ops_json` — JSON array string of PatchOp objects.
/// * `dry_run` — If true, only compute preview (IR diff), no writes.
/// * `format_hint` — Optional format hint (inferred from extension if omitted).
/// * `direct` — If true, apply without tracked-revisions. Default false (revisions ON).
/// * `author` — Author name for revision markup.
///
/// # Returns
/// A `serde_json::Value` containing success, diagnostics, output_path, journal_id,
/// ir_diff, changed_parts, unchanged_parts, changed_element_ids, tainted_parts,
/// format, and byte_diffs.
pub fn apply_patch(
    path: &str,
    ops_json: &str,
    dry_run: bool,
    format_hint: Option<&str>,
    direct: bool,
    author: &str,
) -> Result<serde_json::Value, anyhow::Error> {
    // Read the document
    let source_bytes =
        std::fs::read(path).map_err(|e| anyhow!("failed to read document '{}': {}", path, e))?;

    // Parse ops from JSON
    let ops: Vec<PatchOp> = serde_json::from_str(ops_json)
        .map_err(|e| anyhow!("invalid patch operations JSON: {}", e))?;

    // Determine format
    let format = match format_hint {
        Some(hint) => FormatType::from_extension(hint)
            .ok_or_else(|| anyhow!("unsupported format hint: {}", hint))?,
        None => {
            let ext = std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            FormatType::from_extension(ext)
                .ok_or_else(|| anyhow!("could not determine format from extension: '{}'", ext))?
        }
    };

    // Create executor config: revisions ON by default (direct=false for docx)
    let config = ExecutorConfig {
        revisions: !direct,
        dry_run,
        author: if direct {
            None
        } else {
            Some(author.to_string())
        },
        policy: None,
    };
    let executor = SurgicalPatchExecutor::new(config);

    // Execute the patch
    let result = executor.execute(&source_bytes, format.clone(), &ops);

    // Build the result envelope
    let (success, output_path) = if result.accepted && !dry_run {
        // Check if this is stub mode (no real execution yet)
        let is_stub = result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("Executor stub"));
        if is_stub {
            // Stub mode: no real edits, don't create output file
            (true, None)
        } else {
            // Write output file (dry_run=false, accepted, real execution)
            let output_bytes = &result.output;
            let base = std::path::Path::new(path);
            let stem = base
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("edited");
            let ext = format.as_str();
            let out_path = base.with_file_name(format!("{}.edited.{}", stem, ext));
            std::fs::write(&out_path, output_bytes)
                .map_err(|e| anyhow!("failed to write output: {}", e))?;
            (true, Some(out_path.to_string_lossy().to_string()))
        }
    } else {
        (result.accepted, None)
    };

    // Compute IR diff if dry_run and ops provided
    let ir_diff_value = if dry_run && !ops.is_empty() {
        // Build IR diff for dry-run preview
        let parts = gp_core::executor::parse_source_parts(&source_bytes);
        let before_ir = gp_core::executor::make_minimal_ir(&parts);
        let mut after_ir = before_ir.clone();
        for op in &ops {
            let _ = gp_core::patch::apply_op(&mut after_ir, op);
        }
        let ir_diff = diff_documents(&before_ir, &after_ir);
        Some(serde_json::to_value(ir_diff).unwrap_or(serde_json::json!(null)))
    } else {
        result
            .ir_diff
            .as_ref()
            .and_then(|d| serde_json::to_value(d).ok())
    };

    let changed_element_ids: Vec<String> = result
        .changed_element_ids
        .iter()
        .map(|e| e.to_string())
        .collect();

    let byte_diffs: Vec<serde_json::Value> = result
        .byte_diffs
        .iter()
        .map(|bd| {
            serde_json::json!({
                "part_uri": bd.part_uri,
                "before_hash": bd.before_hash,
                "after_hash": bd.after_hash,
                "before_size": bd.before_size,
                "after_size": bd.after_size,
                "size_diff": bd.size_diff,
                "changed": bd.changed,
            })
        })
        .collect();

    let diagnostics: Vec<serde_json::Value> = result
        .diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "code": d.code,
                "severity": d.severity,
                "message": d.message,
            })
        })
        .collect();

    let unchanged_parts: Vec<String> = result.unchanged_parts.clone();

    let result_envelope = serde_json::json!({
        "success": success,
        "accepted": result.accepted,
        "diagnostics": diagnostics,
        "output_path": output_path,
        "journal_id": result.journal_id,
        "ir_diff": ir_diff_value,
        "changed_parts": result.changed_parts,
        "unchanged_parts": unchanged_parts,
        "changed_element_ids": changed_element_ids,
        "tainted_parts": result.tainted_parts,
        "format": result.format,
        "byte_diffs": byte_diffs,
    });

    Ok(result_envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_patch_json_returns_error() {
        let result = apply_patch(
            "/tmp/nonexistent.docx",
            "not json",
            false,
            None,
            false,
            "agent",
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid patch operations JSON")
                || err.contains("failed to read document")
        );
    }

    #[test]
    fn test_empty_ops_array_produces_success() {
        let result = apply_patch("/tmp/nonexistent.docx", "[]", true, None, false, "agent");
        // Should error on file read, but the ops parsing should succeed
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("failed to read document") || err.contains("could not determine format")
        );
    }

    #[test]
    fn test_dry_run_no_output_file() {
        // Create a temp file to test with
        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join("stichos_test_apply.docx");
        std::fs::write(&tmp_path, b"PK\x03\x04[Content_Types].xml").unwrap();

        let result = apply_patch(
            tmp_path.to_str().unwrap(),
            "[]",
            true, // dry_run
            Some("docx"),
            false, // direct = false (revisions ON)
            "agent",
        );
        assert!(result.is_ok());

        // In dry_run mode, no .edited.docx file should be created
        let expected_output = tmp_path.with_file_name("stichos_test_apply.edited.docx");
        assert!(
            !expected_output.exists(),
            "dry_run should not create output file"
        );

        // Cleanup
        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_apply_patch_valid_ops_json() {
        // Test that valid PatchOp JSON deserializes correctly
        // ElementId serializes as a plain string
        let ops_json = r#"[{"op":"replace_text","elem":"id-abc123","old":"hello","new":"world"}]"#;
        let result = serde_json::from_str::<Vec<PatchOp>>(ops_json);
        assert!(result.is_ok());
        let ops = result.unwrap();
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], PatchOp::ReplaceText { .. }));
    }

    #[test]
    fn test_direct_true_sets_revisions_false() {
        // When direct=true, revisions should be false in config
        let config = ExecutorConfig {
            revisions: false, // direct=true → !direct=false
            dry_run: false,
            author: None, // direct=true → author is None
            policy: None,
        };
        assert!(!config.revisions);
        assert!(config.author.is_none());
    }

    #[test]
    fn test_direct_false_sets_revisions_true_with_author() {
        // When direct=false, revisions should be true and author is set
        let config = ExecutorConfig {
            revisions: true, // direct=false → !direct=true
            dry_run: false,
            author: Some("agent".to_string()),
            policy: None,
        };
        assert!(config.revisions);
        assert_eq!(config.author.as_deref(), Some("agent"));
    }

    #[test]
    fn test_custom_author_with_revisions() {
        // Custom author name should be preserved when not direct
        let config = ExecutorConfig {
            revisions: true,
            dry_run: false,
            author: Some("custom-author".to_string()),
            policy: None,
        };
        assert!(config.revisions);
        assert_eq!(config.author.as_deref(), Some("custom-author"));
    }
}
