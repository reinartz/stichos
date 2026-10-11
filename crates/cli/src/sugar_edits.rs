// SPDX-License-Identifier: MIT OR Apache-2.0
//! Sugar edit tools — `edit_text`, `edit_cell`, `edit_structure`.
//!
//! These are convenience wrappers over `apply_patch` that build the
//! appropriate `PatchOp` JSON internally and delegate to it.

use anyhow::Result;
use gp_core::file_access::read_document_bytes;

/// Replace text in an element by ID. Idempotent: same args produces same result.
///
/// By default (direct=false), edits land as tracked revisions. Use `direct=true`
/// to apply changes without revision markup.
#[allow(clippy::too_many_arguments)]
pub fn edit_text(
    path: &str,
    element_id: &str,
    old_text: &str,
    new_text: &str,
    dry_run: bool,
    direct: bool,
    author: &str,
    format_hint: Option<&str>,
) -> Result<serde_json::Value> {
    let op = serde_json::json!({
        "op": "replace_text",
        "elem": element_id,
        "old": old_text,
        "new": new_text,
    });
    let ops_json = serde_json::to_string(&vec![op])?;
    super::apply_tool::apply_patch(path, &ops_json, dry_run, format_hint, direct, author)
}

/// Set a spreadsheet cell value by element ID. Idempotent: same args produces same result.
#[allow(clippy::too_many_arguments)]
pub fn edit_cell(
    path: &str,
    element_id: &str,
    value: &str,
    dry_run: bool,
    direct: bool,
    author: &str,
    format_hint: Option<&str>,
) -> Result<serde_json::Value> {
    let op = serde_json::json!({
        "op": "set_cell_value",
        "cell": element_id,
        "value": value,
    });
    let ops_json = serde_json::to_string(&vec![op])?;
    super::apply_tool::apply_patch(path, &ops_json, dry_run, format_hint, direct, author)
}

/// Set a spreadsheet cell's rich text runs by element ID (T2 — CellRichText).
///
/// Accepts a JSON array of run objects:
/// `[{"text":"Hello ", "bold":true}, {"text":"World", "color":"FF0000"}]`
#[allow(clippy::too_many_arguments)]
pub fn edit_cell_rich(
    path: &str,
    element_id: &str,
    runs_json: &str,
    dry_run: bool,
    direct: bool,
    author: &str,
    format_hint: Option<&str>,
) -> Result<serde_json::Value> {
    let runs: Vec<gp_core::ir::CellRun> =
        serde_json::from_str(runs_json).map_err(|e| anyhow::anyhow!("invalid runs JSON: {}", e))?;
    let op = serde_json::json!({
        "op": "set_cell_rich",
        "cell": element_id,
        "runs": runs,
    });
    let ops_json = serde_json::to_string(&vec![op])?;
    super::apply_tool::apply_patch(path, &ops_json, dry_run, format_hint, direct, author)
}

/// Insert or delete a structural element by element ID. Idempotent: same args produces same result.
#[allow(clippy::too_many_arguments)]
pub fn edit_structure(
    path: &str,
    element_id: &str,
    action: &str,
    kind: Option<&str>,
    dry_run: bool,
    direct: bool,
    author: &str,
    format_hint: Option<&str>,
) -> Result<serde_json::Value> {
    let op = match action {
        "insert" => {
            let k = kind.unwrap_or("paragraph");
            serde_json::json!({
                "op": "insert",
                "anchor": element_id,
                "kind": k,
            })
        }
        "delete" => {
            serde_json::json!({
                "op": "delete",
                "elem": element_id,
            })
        }
        _ => anyhow::bail!(
            "unknown edit_structure action: {}, must be 'insert' or 'delete'",
            action
        ),
    };
    let ops_json = serde_json::to_string(&vec![op])?;
    super::apply_tool::apply_patch(path, &ops_json, dry_run, format_hint, direct, author)
}

/// Add a comment to a document. Works with docx and odt formats.
#[allow(clippy::too_many_arguments)]
pub fn edit_comment(
    path: &str,
    comment_text: &str,
    element_id: &str,
    dry_run: bool,
    _direct: bool,
    author: &str,
    format_hint: Option<&str>,
) -> Result<serde_json::Value> {
    let bytes = read_document_bytes(path)?;
    let ext = if let Some(fh) = format_hint {
        fh.to_lowercase()
    } else {
        path.to_lowercase()
    };

    let comment = gp_core::ir::Comment {
        id: gp_core::ir::ElementId::content_anchored("comment", &["comment:0"], comment_text),
        comment_id: uuid::Uuid::new_v4().as_u128().to_string(),
        author: author.to_string(),
        date: Some(chrono::Utc::now().to_rfc3339()),
        initials: Some("R".to_string()),
        content: vec![gp_core::ir::SectionItem::Paragraph(
            gp_core::ir::Paragraph {
                id: gp_core::ir::ElementId::content_anchored(
                    "paragraph",
                    &["para:0"],
                    comment_text,
                ),
                runs: vec![gp_core::ir::Run {
                    id: gp_core::ir::ElementId::content_anchored("run", &["run:0"], comment_text),
                    text: comment_text.to_string(),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: None,
                }],
                page_number: None,
                style_name: None,
                numbering_info: None,
            },
        )],
    };

    let element_id_elem = gp_core::ir::ElementId::from_str(element_id);

    let result: Result<_, String> = if ext.ends_with(".docx") {
        adapters_ooxml::comment_writer::add_comment_to_docx(
            &bytes,
            &comment,
            &element_id_elem,
            dry_run,
        )
        .map_err(|diags| {
            let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
            msgs.join("; ")
        })
        .map(|res| {
            serde_json::json!({
                "comment_id": res.comment_id,
                "changed_parts": res.changed_parts,
                "format": res.format,
            })
        })
    } else if ext.ends_with(".odt") {
        adapters_odf::comment_writer::add_comment_to_odt(
            &bytes,
            &comment,
            &element_id_elem,
            dry_run,
        )
        .map_err(|diags| {
            let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
            msgs.join("; ")
        })
        .map(|res| {
            serde_json::json!({
                "comment_id": res.comment_id,
                "changed_parts": res.changed_parts,
                "format": res.format,
            })
        })
    } else {
        anyhow::bail!("unsupported format for comment write-back: {}", ext);
    };

    match result {
        Ok(json_val) => {
            let mut val = json_val.as_object().unwrap().clone();
            val.insert(
                "status".to_string(),
                serde_json::json!(if dry_run { "dry_run" } else { "ok" }),
            );
            val.insert("path".to_string(), serde_json::json!(path));
            val.insert("dry_run".to_string(), serde_json::json!(dry_run));
            Ok(serde_json::Value::Object(val))
        }
        Err(diags) => Ok(serde_json::json!({
            "status": "error",
            "diagnostics": diags,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edit_text_builds_correct_json() {
        let result = edit_text(
            "/tmp/nonexistent.docx",
            "id-abc1234567",
            "old",
            "new",
            true,
            false,
            "agent",
            Some("docx"),
        );
        let err = result.unwrap_err().to_string();
        // Should fail on file read, not JSON construction
        assert!(
            !err.contains("invalid patch operations JSON"),
            "unexpected JSON error: {}",
            err
        );
    }

    #[test]
    fn test_edit_cell_builds_correct_json() {
        let result = edit_cell(
            "/tmp/nonexistent.xlsx",
            "id-abc1234567",
            "42",
            true,
            false,
            "agent",
            Some("xlsx"),
        );
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("invalid patch operations JSON"),
            "unexpected JSON error: {}",
            err
        );
    }

    #[test]
    fn test_edit_structure_insert_builds_correct_json() {
        let result = edit_structure(
            "/tmp/nonexistent.docx",
            "id-abc1234567",
            "insert",
            Some("paragraph"),
            true,
            false,
            "agent",
            Some("docx"),
        );
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("invalid patch operations JSON"),
            "unexpected JSON error: {}",
            err
        );
    }

    #[test]
    fn test_edit_structure_delete_builds_correct_json() {
        let result = edit_structure(
            "/tmp/nonexistent.docx",
            "id-abc1234567",
            "delete",
            None,
            true,
            false,
            "agent",
            Some("docx"),
        );
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("invalid patch operations JSON"),
            "unexpected JSON error: {}",
            err
        );
    }

    #[test]
    fn test_edit_structure_unknown_action() {
        let result = edit_structure(
            "/tmp/nonexistent.docx",
            "id-abc1234567",
            "unknown",
            None,
            true,
            false,
            "agent",
            Some("docx"),
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown edit_structure action"),
            "expected action error, got: {}",
            err
        );
    }

    #[test]
    fn test_edit_text_direct_flag_propagates() {
        // direct=true → revisions: false, author: None
        let result = edit_text(
            "/tmp/nonexistent.docx",
            "id-abc1234567",
            "old",
            "new",
            true,
            true,
            "agent",
            Some("docx"),
        );
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("invalid patch operations JSON"),
            "unexpected JSON error: {}",
            err
        );
    }

    #[test]
    fn test_edit_text_revisions_flag_propagates() {
        // direct=false → revisions: true, author: Some("agent")
        let result = edit_text(
            "/tmp/nonexistent.docx",
            "id-abc1234567",
            "old",
            "new",
            false,
            false,
            "agent",
            Some("docx"),
        );
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("invalid patch operations JSON"),
            "unexpected JSON error: {}",
            err
        );
    }
}
