//! Sugar edit tools — `edit_text`, `edit_cell`, `edit_structure`.
//!
//! These are convenience wrappers over `apply_patch` that build the
//! appropriate `PatchOp` JSON internally and delegate to it.

use anyhow::Result;

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
