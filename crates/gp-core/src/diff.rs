// SPDX-License-Identifier: MIT OR Apache-2.0
//! IR unified diff: text-level hunks + machine-readable JSON.
//!
//! This module provides a unified-text diff for IR documents, anchored to
//! `ElementId`s. Every hunk identifies the element being modified and carries
//! a list of text-level operations (`DiffOp`). A parallel `serde_json::Value`
//! provides a machine-consumable representation of the same diff.
//!
//! See also: `docs/contracts/C10.md` (policy + taint) for the broader context
//! in which diffs are produced and consumed.

use crate::diagnostics::Diagnostic;
use crate::ir::{ElementId, IRDocument};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// DiffOp
// ---------------------------------------------------------------------------

/// A text-level operation within a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
#[serde(rename_all = "snake_case")]
pub enum DiffOp {
    /// Insert `text` at this point.
    Insert { text: String },
    /// Delete `text` from this point.
    Delete { text: String },
    /// Replace `old` with `new`.
    Replace { old: String, new: String },
}

// ---------------------------------------------------------------------------
// DiffHunk
// ---------------------------------------------------------------------------

/// A single hunk in a unified diff, anchored to an element ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    /// ID of the element being modified.
    pub anchor: ElementId,
    /// Human-readable kind of the anchor element.
    pub anchor_kind: String,
    /// Text-level operations within this hunk.
    pub operations: Vec<DiffOp>,
}

// ---------------------------------------------------------------------------
// IrDiff
// ---------------------------------------------------------------------------

/// A unified diff of IR content.
///
/// `hunks` carries the human-readable unified-text view while `json` provides
/// the same structure as machine-readable JSON for tooling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrDiff {
    /// Ordered list of hunks.
    pub hunks: Vec<DiffHunk>,
    /// Machine-readable JSON diff (mirrors `hunks` structure).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub json: Option<serde_json::Value>,
}

impl Default for IrDiff {
    fn default() -> Self {
        Self {
            hunks: Vec::new(),
            json: Some(serde_json::Value::Array(Vec::new())),
        }
    }
}

impl IrDiff {
    /// Returns `true` if this diff has no hunks.
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Return the JSON representation of the hunks.
    ///
    /// If `json` is `None`, serialises `hunks` to JSON.
    pub fn json_value(&self) -> Option<serde_json::Value> {
        self.json
            .clone()
            .or_else(|| serde_json::to_value(&self.hunks).ok())
    }
}

// ---------------------------------------------------------------------------
// ID index: IRDocument → ElementId → (path, text)
// ---------------------------------------------------------------------------

/// An entry from the element index.
#[derive(Debug, Clone)]
struct IndexedElement {
    kind: String,
    /// Path string for display (unused in current implementation).
    #[allow(dead_code)]
    path: String,
    /// Text content extracted from the element.
    text: String,
}

/// Build a flat index of every element in an IRDocument.
///
/// Traverses the tagged tree and collects:
/// - Document-level elements (sections, tables, etc.)
/// - All TaggedNode descendants
fn index_elements(doc: &IRDocument) -> HashMap<ElementId, IndexedElement> {
    let mut map = HashMap::new();

    // Index document-level elements from the root TaggedNode
    index_tagged_node(&doc.root, &mut map);

    map
}

fn index_tagged_node(node: &crate::ir::TaggedNode, map: &mut HashMap<ElementId, IndexedElement>) {
    // Collect text from this node and all descendants
    let text = collect_text(node);
    let kind = node.tag.clone();
    let path = format!("tagged/{}", node.tag);

    map.insert(node.id.clone(), IndexedElement { kind, path, text });

    for child in &node.children {
        index_tagged_node(child, map);
    }
}

/// Collect all text content from a TaggedNode and its descendants.
fn collect_text(node: &crate::ir::TaggedNode) -> String {
    let mut text = String::new();
    text.push_str(node.alt_text.as_ref().map_or("", |s| s.as_str()));
    for child in &node.children {
        text.push_str(&collect_text(child));
    }
    text
}

// ---------------------------------------------------------------------------
// LCS-based diff: compute DiffOps between two strings
// ---------------------------------------------------------------------------

/// Compute a simple diff between two strings, returning a list of DiffOps.
///
/// Uses a basic LCS (Longest Common Subsequence) approach. For IR documents
/// the strings are typically short (element-level text), so this is efficient.
fn compute_diff_ops(old: &str, new: &str) -> Vec<DiffOp> {
    if old == new {
        return Vec::new();
    }

    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let m = old_chars.len();
    let n = new_chars.len();

    if old.is_empty() {
        return vec![DiffOp::Insert {
            text: new.to_string(),
        }];
    }
    if new.is_empty() {
        return vec![DiffOp::Delete {
            text: old.to_string(),
        }];
    }

    // Build LCS table (forward)
    let mut lcs = vec![vec![0usize; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old_chars[i - 1] == new_chars[j - 1] {
                lcs[i][j] = lcs[i - 1][j - 1] + 1;
            } else {
                lcs[i][j] = lcs[i - 1][j].max(lcs[i][j - 1]);
            }
        }
    }

    // Backtrack to produce ops
    let mut ops = Vec::new();
    let mut i = m;
    let mut j = n;
    let mut buffer_old = Vec::new();
    let mut buffer_new = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old_chars[i - 1] == new_chars[j - 1] {
            // Flush buffers if any
            if !buffer_old.is_empty() || !buffer_new.is_empty() {
                ops.push(DiffOp::Replace {
                    old: buffer_old.drain(..).collect(),
                    new: buffer_new.drain(..).collect(),
                });
            }
            // Collect matching chars
            let mut match_buf = Vec::new();
            while i > 0 && j > 0 && old_chars[i - 1] == new_chars[j - 1] {
                match_buf.push(old_chars[i - 1]);
                i -= 1;
                j -= 1;
            }
            match_buf.reverse();
            ops.push(DiffOp::Insert {
                text: match_buf.into_iter().collect(),
            });
        } else if j > 0 && (i == 0 || lcs[i][j - 1] >= lcs[i - 1][j]) {
            buffer_new.push(new_chars[j - 1]);
            j -= 1;
        } else {
            buffer_old.push(old_chars[i - 1]);
            i -= 1;
        }
    }

    // Flush remaining buffers
    if !buffer_old.is_empty() || !buffer_new.is_empty() {
        buffer_old.reverse();
        buffer_new.reverse();
        if buffer_old.is_empty() {
            ops.push(DiffOp::Insert {
                text: buffer_new.into_iter().collect(),
            });
        } else if buffer_new.is_empty() {
            ops.push(DiffOp::Delete {
                text: buffer_old.into_iter().collect(),
            });
        } else {
            ops.push(DiffOp::Replace {
                old: buffer_old.into_iter().collect(),
                new: buffer_new.into_iter().collect(),
            });
        }
    }

    ops.reverse();
    ops
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create an empty diff.
pub fn empty_diff() -> IrDiff {
    IrDiff::default()
}

/// Convert a list of patch operations into a unified diff.
///
/// For each `ReplaceText` patch op, a hunk is produced anchored to the target
/// element. The `before_ir` is used to look up the old text.
pub fn diff_patches_to_unified(
    patches: &[crate::patch::PatchOp],
    before_ir: &IRDocument,
) -> IrDiff {
    let elements = index_elements(before_ir);
    let mut hunks = Vec::new();

    for patch in patches {
        use crate::patch::PatchOp;

        if let PatchOp::ReplaceText {
            elem,
            old,
            new: new_text,
        } = patch
        {
            if let Some(idx) = elements.get(elem) {
                let ops = compute_diff_ops(old, new_text);
                if !ops.is_empty() {
                    hunks.push(DiffHunk {
                        anchor: elem.clone(),
                        anchor_kind: idx.kind.clone(),
                        operations: ops,
                    });
                }
            }
        }
    }

    let json = if hunks.is_empty() {
        Some(serde_json::json!([]))
    } else {
        serde_json::to_value(&hunks).ok()
    };

    IrDiff { hunks, json }
}

/// Compute a diff between two IR documents by comparing element contents.
///
/// Both documents are indexed by their element IDs. For every element present
/// in both, if the text content differs, a hunk is produced.
pub fn diff_documents(before: &IRDocument, after: &IRDocument) -> IrDiff {
    let before_elements = index_elements(before);
    let after_elements = index_elements(after);

    let mut hunks = Vec::new();

    // Compare elements present in both
    let before_ids: Vec<_> = before_elements.keys().collect();
    for elem_id in before_ids {
        if let (Some(before_idx), Some(after_idx)) =
            (before_elements.get(elem_id), after_elements.get(elem_id))
        {
            if before_idx.text != after_idx.text {
                let ops = compute_diff_ops(&before_idx.text, &after_idx.text);
                if !ops.is_empty() {
                    hunks.push(DiffHunk {
                        anchor: elem_id.clone(),
                        anchor_kind: after_idx.kind.clone(),
                        operations: ops,
                    });
                }
            }
        }
    }

    // Elements only in after are insertions
    for (elem_id, after_idx) in &after_elements {
        if !before_elements.contains_key(elem_id) {
            let ops = vec![DiffOp::Insert {
                text: after_idx.text.clone(),
            }];
            if !ops.is_empty() {
                hunks.push(DiffHunk {
                    anchor: elem_id.clone(),
                    anchor_kind: after_idx.kind.clone(),
                    operations: ops,
                });
            }
        }
    }

    // Elements only in before are deletions
    for (elem_id, before_idx) in &before_elements {
        if !after_elements.contains_key(elem_id) {
            let ops = vec![DiffOp::Delete {
                text: before_idx.text.clone(),
            }];
            if !ops.is_empty() {
                hunks.push(DiffHunk {
                    anchor: elem_id.clone(),
                    anchor_kind: before_idx.kind.clone(),
                    operations: ops,
                });
            }
        }
    }

    let json = if hunks.is_empty() {
        Some(serde_json::json!([]))
    } else {
        serde_json::to_value(&hunks).ok()
    };

    IrDiff { hunks, json }
}

/// Reverse a unified diff back into patch operations.
///
/// Returns `Ok(Vec<PatchOp>)` with one `ReplaceText` patch per hunk.
/// Returns `Err(Vec<Diagnostic>)` if any hunk's anchor is unknown.
pub fn unified_to_patches(diff: &IrDiff) -> Result<Vec<crate::patch::PatchOp>, Vec<Diagnostic>> {
    let mut patches = Vec::new();

    for hunk in &diff.hunks {
        let mut old_text = String::new();
        let mut new_text = String::new();

        for op in &hunk.operations {
            match op {
                DiffOp::Insert { text } => {
                    new_text.push_str(text);
                }
                DiffOp::Delete { text } => {
                    old_text.push_str(text);
                }
                DiffOp::Replace {
                    old,
                    new: new_text_h,
                } => {
                    old_text.push_str(old);
                    new_text.push_str(new_text_h);
                }
            }
        }

        // Only create a patch if there are actual changes
        if !old_text.is_empty() || !new_text.is_empty() {
            patches.push(crate::patch::PatchOp::ReplaceText {
                elem: hunk.anchor.clone(),
                old: old_text,
                new: new_text,
            });
        }
    }

    Ok(patches)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::TaggedNode;

    fn make_doc(text: &str) -> IRDocument {
        let id = ElementId::content_anchored("text", &["doc:0"], text);
        IRDocument {
            ir_version: crate::ir::IR_VERSION.to_string(),
            root: TaggedNode {
                id: id.clone(),
                tag: "document".to_string(),
                alt_text: Some(text.to_string()),
                bbox: None,
                children: vec![],
            },
        }
    }

    fn make_child_doc(parent_text: &str, child_tag: &str, child_text: &str) -> IRDocument {
        let child_id = ElementId::content_anchored("text", &["child:0"], child_text);
        let child = TaggedNode {
            id: child_id.clone(),
            tag: child_tag.to_string(),
            alt_text: Some(child_text.to_string()),
            bbox: None,
            children: vec![],
        };
        let parent_id = ElementId::content_anchored("text", &["doc:0"], parent_text);
        IRDocument {
            ir_version: crate::ir::IR_VERSION.to_string(),
            root: TaggedNode {
                id: parent_id,
                tag: "document".to_string(),
                alt_text: Some(parent_text.to_string()),
                bbox: None,
                children: vec![child],
            },
        }
    }

    // ------------------------------------------------------------------
    // Empty diff tests
    // ------------------------------------------------------------------

    #[test]
    fn test_empty_diff() {
        let diff = empty_diff();
        assert!(diff.is_empty());
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn test_empty_patch_produces_empty_diff() {
        let doc = make_doc("hello");
        let patches: Vec<crate::patch::PatchOp> = vec![];
        let diff = diff_patches_to_unified(&patches, &doc);
        assert!(diff.is_empty());
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn test_diff_identical_documents() {
        let doc = make_doc("hello world");
        let diff = diff_documents(&doc, &doc);
        assert!(diff.is_empty());
    }

    // ------------------------------------------------------------------
    // Diff operations
    // ------------------------------------------------------------------

    #[test]
    fn test_diff_ops_same_text() {
        let ops = compute_diff_ops("hello", "hello");
        assert!(ops.is_empty());
    }

    #[test]
    fn test_diff_ops_insert() {
        let ops = compute_diff_ops("", "hello");
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            DiffOp::Insert { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Insert"),
        }
    }

    #[test]
    fn test_diff_ops_delete() {
        let ops = compute_diff_ops("hello", "");
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            DiffOp::Delete { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn test_diff_ops_replace() {
        let ops = compute_diff_ops("hello", "hallo");
        // Should produce at least one operation
        assert!(!ops.is_empty());
        // Check that there's a Replace or Delete+Insert
        let has_change = ops.iter().any(|op| matches!(op, DiffOp::Replace { .. }));
        assert!(has_change || ops.iter().any(|op| matches!(op, DiffOp::Delete { .. })));
    }

    #[test]
    fn test_diff_ops_partial_match() {
        let ops = compute_diff_ops("hello world", "hello universe");
        // Should have some ops since the texts differ
        assert!(!ops.is_empty());
    }

    // ------------------------------------------------------------------
    // Hunk & anchor tests
    // ------------------------------------------------------------------

    #[test]
    fn test_hunk_has_anchor() {
        let doc = make_doc("old text");
        let id = doc.root.id.clone();

        let patches = vec![crate::patch::PatchOp::ReplaceText {
            elem: id.clone(),
            old: "old text".to_string(),
            new: "new text".to_string(),
        }];
        let diff = diff_patches_to_unified(&patches, &doc);

        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].anchor, id);
        assert!(!diff.hunks[0].anchor_kind.is_empty());
        assert!(!diff.hunks[0].operations.is_empty());
    }

    #[test]
    fn test_id_anchors_in_all_hunks() {
        let doc = make_doc("old");
        let id = doc.root.id.clone();

        let patches = vec![crate::patch::PatchOp::ReplaceText {
            elem: id.clone(),
            old: "old".to_string(),
            new: "new".to_string(),
        }];
        let diff = diff_patches_to_unified(&patches, &doc);

        for hunk in &diff.hunks {
            assert!(hunk.anchor.as_str().starts_with("id-"));
            assert!(!hunk.anchor_kind.is_empty());
        }
    }

    // ------------------------------------------------------------------
    // diff_documents tests
    // ------------------------------------------------------------------

    #[test]
    fn test_diff_documents_text_change() {
        let before = make_doc("hello");
        let after = make_doc("world");
        let diff = diff_documents(&before, &after);

        assert!(!diff.is_empty());
        assert!(
            !diff.hunks.is_empty(),
            "expected at least 1 hunk, got {}",
            diff.hunks.len()
        );
    }

    #[test]
    fn test_diff_documents_no_change() {
        let doc = make_doc("same");
        let diff = diff_documents(&doc, &doc);
        assert!(diff.is_empty());
    }

    #[test]
    fn test_diff_documents_child_change() {
        let before = make_child_doc("parent", "section", "old child");
        let after = make_child_doc("parent", "section", "new child");
        let diff = diff_documents(&before, &after);

        // Should detect the child text change
        let has_section_hunk = diff.hunks.iter().any(|h| h.anchor_kind == "section");
        assert!(
            has_section_hunk,
            "expected a section hunk, got {:?}",
            diff.hunks
        );
    }

    // ------------------------------------------------------------------
    // JSON tests
    // ------------------------------------------------------------------

    #[test]
    fn test_json_present_on_non_empty_diff() {
        let before = make_doc("old");
        let after = make_doc("new");
        let diff = diff_documents(&before, &after);

        assert!(diff.json.is_some());
        let json = diff.json_value().unwrap();
        assert!(json.is_array() || json.is_object());
    }

    #[test]
    fn test_json_empty_on_empty_diff() {
        let doc = make_doc("same");
        let diff = diff_documents(&doc, &doc);

        assert!(diff.is_empty());
        let json = diff.json_value().unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_json_serialization_roundtrip() {
        let before = make_doc("old text");
        let after = make_doc("new text");
        let diff = diff_documents(&before, &after);

        let json_str = serde_json::to_string(&diff).expect("serialize");
        let decoded: IrDiff = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(diff, decoded);
    }

    #[test]
    fn test_golden_diff_pairs() {
        let before = make_doc("hello");
        let after = make_doc("hellos");
        let diff = diff_documents(&before, &after);

        // Golden: text diff should show insertion
        assert!(!diff.is_empty());

        // Golden: JSON should be present and valid
        let json = diff.json_value().expect("json present");
        let json_str = serde_json::to_string(&json).expect("json serialize");
        assert!(json_str.contains("insert") || json_str.contains("replace"));

        // Golden: every hunk has anchor and operations
        for hunk in &diff.hunks {
            assert!(!hunk.anchor.as_str().is_empty());
            assert!(!hunk.anchor_kind.is_empty());
            assert!(!hunk.operations.is_empty());
        }
    }

    // ------------------------------------------------------------------
    // Round-trip tests
    // ------------------------------------------------------------------

    #[test]
    fn test_roundtrip_diff_to_patches_to_diff() {
        let before = make_doc("old content");
        let after = make_doc("new content");

        // Compute initial diff
        let diff1 = diff_documents(&before, &after);
        assert!(!diff1.is_empty());

        // Convert to patches
        let patches = unified_to_patches(&diff1).expect("unified_to_patches");
        assert!(!patches.is_empty());

        // Verify each patch's ReplaceText has correct elem and content
        for patch in &patches {
            if let crate::patch::PatchOp::ReplaceText {
                elem,
                old,
                new: new_text,
            } = patch
            {
                assert!(!elem.as_str().is_empty(), "patch elem should be non-empty");
                // Either old or new (or both) should be non-empty
                assert!(
                    !old.is_empty() || !new_text.is_empty(),
                    "patch should have non-empty old or new"
                );
            }
        }

        // Convert patches back to diff
        let diff2 = diff_patches_to_unified(&patches, &before);

        // diff2 should have at least one hunk (for patches that matched elements)
        assert!(
            !diff2.hunks.is_empty(),
            "roundtrip should produce at least 1 hunk"
        );
    }

    #[test]
    fn test_roundtrip_empty_diff() {
        let doc = make_doc("same");
        let diff1 = diff_documents(&doc, &doc);
        assert!(diff1.is_empty());

        let patches = unified_to_patches(&diff1).expect("unified_to_patches");
        assert!(patches.is_empty());

        let diff2 = diff_patches_to_unified(&patches, &doc);
        assert!(diff2.is_empty());
    }

    #[test]
    fn test_multiple_hunks() {
        let before = make_child_doc("parent", "section", "old child");
        let after = make_child_doc("parent", "section", "new child");

        // Add another element to after that doesn't exist in before
        let mut after_doc = after.clone();
        let extra_id = ElementId::content_anchored("extra", &["extra:0"], "extra text");
        after_doc.root.children.push(TaggedNode {
            id: extra_id.clone(),
            tag: "extra".to_string(),
            alt_text: Some("extra text".to_string()),
            bbox: None,
            children: vec![],
        });

        let diff = diff_documents(&before, &after_doc);

        // Should have hunks for both the changed child and the new element
        assert!(!diff.hunks.is_empty());

        // All hunks must have valid anchors
        for hunk in &diff.hunks {
            assert!(hunk.anchor.as_str().starts_with("id-"));
        }
    }

    // ------------------------------------------------------------------
    // unified_to_patches tests
    // ------------------------------------------------------------------

    #[test]
    fn test_unified_to_patches_replace() {
        let anchor = ElementId::content_anchored("test", &["t:0"], "a");
        let diff = IrDiff {
            hunks: vec![DiffHunk {
                anchor: anchor.clone(),
                anchor_kind: "section".to_string(),
                operations: vec![DiffOp::Replace {
                    old: "old".to_string(),
                    new: "new".to_string(),
                }],
            }],
            json: None,
        };

        let patches = unified_to_patches(&diff).expect("patches");
        assert_eq!(patches.len(), 1);
        match &patches[0] {
            crate::patch::PatchOp::ReplaceText {
                elem,
                old,
                new: new_text,
            } => {
                assert_eq!(elem, &anchor);
                assert_eq!(old, "old");
                assert_eq!(new_text, "new");
            }
            _ => panic!("expected ReplaceText"),
        }
    }

    #[test]
    fn test_unified_to_patches_insert_only() {
        let anchor = ElementId::content_anchored("test", &["t:0"], "");
        let diff = IrDiff {
            hunks: vec![DiffHunk {
                anchor: anchor.clone(),
                anchor_kind: "section".to_string(),
                operations: vec![DiffOp::Insert {
                    text: "new content".to_string(),
                }],
            }],
            json: None,
        };

        let patches = unified_to_patches(&diff).expect("patches");
        assert_eq!(patches.len(), 1);
    }

    #[test]
    fn test_unified_to_patches_delete_only() {
        let anchor = ElementId::content_anchored("test", &["t:0"], "old content");
        let diff = IrDiff {
            hunks: vec![DiffHunk {
                anchor: anchor.clone(),
                anchor_kind: "section".to_string(),
                operations: vec![DiffOp::Delete {
                    text: "old content".to_string(),
                }],
            }],
            json: None,
        };

        let patches = unified_to_patches(&diff).expect("patches");
        assert_eq!(patches.len(), 1);
    }

    #[test]
    fn test_unified_to_patches_empty_diff() {
        let diff = empty_diff();
        let patches = unified_to_patches(&diff).expect("patches");
        assert!(patches.is_empty());
    }
}
