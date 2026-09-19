// SPDX-License-Identifier: MIT OR Apache-2.0
//! Patch operations for IR transformation.
//!
//! This module provides the `PatchOp` enum, the `apply_op` engine function,
//! and JSON serialization support for patch operations.
//!
//! ## Supported operations
//!
//! - **Insert** — insert a new element at an anchor point.
//! - **ReplaceText** — replace text content in an element.
//! - **Delete** — remove an element from the IR.
//! - **SetRunProps** — update formatting properties on a run.
//! - **SetCellValue** — update a spreadsheet cell value.
//! - **TrackInsert** — tracked (revisions) insert.
//! - **TrackDelete** — tracked (revisions) delete.
//!
//! ## Diagnostic codes
//!
//! - `GP-E-001` — Invalid patch operation (per C3 contract).

use crate::diagnostics::Diagnostic;
use crate::fidelity::FidelityBudget;
use crate::ir::{ElementId, IRDocument, TaggedNode};
use serde::{Deserialize, Serialize};

/// Formatting properties for a text run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunProperties {
    pub bold: bool,
    pub italic: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub font_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub font_size_pt: Option<f32>,
}

/// A single patch operation that transforms an IR document.
///
/// Each variant identifies the target element (or anchor point) and carries
/// the data needed to perform the transformation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
#[serde(rename_all = "snake_case")]
pub enum PatchOp {
    /// Insert a new element at the given anchor point.
    Insert { anchor: ElementId, kind: String },
    /// Replace the text content of an element.
    ReplaceText {
        elem: ElementId,
        old: String,
        new: String,
    },
    /// Delete an element from the IR.
    Delete { elem: ElementId },
    /// Update formatting properties on a run element.
    SetRunProps {
        elem: ElementId,
        props: RunProperties,
    },
    /// Update a spreadsheet cell value.
    SetCellValue { cell: ElementId, value: String },
    /// Tracked (revisions) insert — used by collaboration engines.
    TrackInsert {
        anchor: ElementId,
        kind: String,
        author: String,
        timestamp: String,
    },
    /// Tracked (revisions) delete — used by collaboration engines.
    TrackDelete {
        elem: ElementId,
        author: String,
        timestamp: String,
    },
}

/// Result of applying a patch operation to an IR document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyResult {
    /// Diagnostics produced during the apply.
    pub diagnostics: Vec<Diagnostic>,
    /// Element IDs that were modified or inserted.
    pub changed_element_ids: Vec<ElementId>,
    /// Fidelity budget reflecting what was preserved.
    pub fidelity_budget: FidelityBudget,
}

/// Apply a single patch operation to an IR document.
///
/// Returns `Ok(ApplyResult)` with any diagnostics, changed IDs, and
/// fidelity budget. Unknown operations return a `GP-E-001` diagnostic.
pub fn apply_op(ir: &mut IRDocument, op: &PatchOp) -> Result<ApplyResult, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut changed: Vec<ElementId> = Vec::new();

    match op {
        PatchOp::Insert { .. } => {
            diagnostics.push(Diagnostic::new(
                "GP-I-001",
                crate::diagnostics::Severity::Info,
                "Insert operation not yet implemented",
            ));
        }
        PatchOp::ReplaceText { elem, old: _, new } => {
            replace_text_in_tree(&mut ir.root, elem, new, &mut changed);
        }
        PatchOp::Delete { elem } => {
            delete_from_tree(&mut ir.root, elem, &mut changed);
        }
        PatchOp::SetRunProps { elem, props } => {
            set_run_props_in_tree(&mut ir.root, elem, props, &mut changed);
        }
        PatchOp::SetCellValue { cell, value } => {
            set_cell_value_in_tree(&mut ir.root, cell, value, &mut changed);
        }
        PatchOp::TrackInsert { .. } => {
            diagnostics.push(Diagnostic::new(
                "GP-I-002",
                crate::diagnostics::Severity::Info,
                "TrackInsert operation not yet implemented",
            ));
        }
        PatchOp::TrackDelete { .. } => {
            diagnostics.push(Diagnostic::new(
                "GP-I-003",
                crate::diagnostics::Severity::Info,
                "TrackDelete operation not yet implemented",
            ));
        }
    }

    let fidelity = FidelityBudget::default();
    Ok(ApplyResult {
        diagnostics,
        changed_element_ids: changed,
        fidelity_budget: fidelity,
    })
}

/// Recursively search a tagged tree for the target element and replace its text.
fn replace_text_in_tree(
    node: &mut TaggedNode,
    target_id: &ElementId,
    new_text: &str,
    changed: &mut Vec<ElementId>,
) {
    if node.id == *target_id {
        node.tag = new_text.to_string();
        changed.push(node.id.clone());
    } else {
        for child in &mut node.children {
            replace_text_in_tree(child, target_id, new_text, changed);
        }
    }
}

/// Recursively search a tagged tree for the target element and delete it.
fn delete_from_tree(parent: &mut TaggedNode, target_id: &ElementId, changed: &mut Vec<ElementId>) {
    let to_remove: Vec<usize> = parent
        .children
        .iter()
        .enumerate()
        .filter_map(|(i, child)| {
            if child.id == *target_id {
                changed.push(child.id.clone());
                Some(i)
            } else {
                None
            }
        })
        .rev()
        .collect();
    for i in to_remove {
        parent.children.remove(i);
    }
    // Recurse into remaining children
    for child in &mut parent.children {
        delete_from_tree(child, target_id, changed);
    }
}

/// Recursively search a tagged tree for a run element and set its properties.
fn set_run_props_in_tree(
    node: &mut TaggedNode,
    target_id: &ElementId,
    props: &RunProperties,
    changed: &mut Vec<ElementId>,
) {
    if node.id == *target_id {
        if props.bold {
            node.alt_text = Some(format!(
                "bold:{}",
                node.alt_text.clone().unwrap_or_default()
            ));
        }
        if props.italic {
            node.alt_text = node.alt_text.as_ref().map(|a| {
                if a.is_empty() {
                    format!("italic:{}", node.alt_text.clone().unwrap_or_default())
                } else {
                    format!("{},italic", a)
                }
            });
        }
        if let Some(ref font) = props.font_name {
            node.alt_text = node.alt_text.as_ref().map(|a| {
                if a.is_empty() {
                    format!("font:{font}")
                } else {
                    format!("{},font:{}", a, font)
                }
            });
        }
        if let Some(pt) = props.font_size_pt {
            node.alt_text = node.alt_text.as_ref().map(|a| {
                if a.is_empty() {
                    format!("size:{pt}")
                } else {
                    format!("{},size:{}", a, pt)
                }
            });
        }
        changed.push(node.id.clone());
    } else {
        for child in &mut node.children {
            set_run_props_in_tree(child, target_id, props, changed);
        }
    }
}

/// Recursively search a tagged tree for a cell element and set its value.
fn set_cell_value_in_tree(
    node: &mut TaggedNode,
    cell_id: &ElementId,
    value: &str,
    changed: &mut Vec<ElementId>,
) {
    if node.id == *cell_id {
        node.tag = value.to_string();
        changed.push(node.id.clone());
    } else {
        for child in &mut node.children {
            set_cell_value_in_tree(child, cell_id, value, changed);
        }
    }
}

/// Check if a patch operation is known.
///
/// Returns `Ok(())` for all recognized ops in the `PatchOp` enum.
pub fn validate_op(_op: &PatchOp) -> Result<(), Vec<Diagnostic>> {
    Ok(())
}

/// Serialize a patch operation to JSON.
pub fn op_to_json(op: &PatchOp) -> serde_json::Result<String> {
    serde_json::to_string(op)
}

/// Deserialize a patch operation from JSON bytes.
pub fn op_from_json(data: &[u8]) -> serde_json::Result<PatchOp> {
    serde_json::from_slice(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample_doc() -> IRDocument {
        let para_id = ElementId::content_anchored("paragraph", &["section:0"], "hello");
        let run_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:0", "run:0"], "hello");

        IRDocument {
            ir_version: "1.0.0".to_string(),
            root: TaggedNode {
                id: ElementId::content_anchored("document", &["document"], "root"),
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![TaggedNode {
                    id: para_id.clone(),
                    tag: "paragraph".to_string(),
                    alt_text: None,
                    bbox: None,
                    children: vec![TaggedNode {
                        id: run_id.clone(),
                        tag: "hello".to_string(),
                        alt_text: None,
                        bbox: None,
                        children: Vec::new(),
                    }],
                }],
            },
        }
    }

    // ------------------------------------------------------------------
    // JSON round-trip tests
    // ------------------------------------------------------------------

    #[test]
    fn json_roundtrip_insert() {
        let anchor = ElementId::content_anchored("paragraph", &["section:0"], "anchor");
        let op = PatchOp::Insert {
            anchor: anchor.clone(),
            kind: "paragraph".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_replace_text() {
        let elem = ElementId::content_anchored("run", &["section:0"], "elem");
        let op = PatchOp::ReplaceText {
            elem: elem.clone(),
            old: "old".to_string(),
            new: "new".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_delete() {
        let elem = ElementId::content_anchored("paragraph", &["section:0"], "elem");
        let op = PatchOp::Delete { elem: elem.clone() };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_set_run_props() {
        let elem = ElementId::content_anchored("run", &["section:0"], "elem");
        let props = RunProperties {
            bold: true,
            italic: false,
            font_name: Some("Arial".to_string()),
            font_size_pt: Some(14.0),
        };
        let op = PatchOp::SetRunProps {
            elem: elem.clone(),
            props: props.clone(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetRunProps {
            elem: back_elem,
            props: back_props,
        } = back
        {
            assert_eq!(elem, back_elem);
            assert_eq!(props, back_props);
        } else {
            panic!("expected SetRunProps");
        }
    }

    #[test]
    fn json_roundtrip_set_cell_value() {
        let cell = ElementId::content_anchored("cell", &["sheet:0", "row:0", "cell:0"], "cell");
        let op = PatchOp::SetCellValue {
            cell: cell.clone(),
            value: "42".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_track_insert() {
        let anchor = ElementId::content_anchored("run", &["section:0"], "anchor");
        let op = PatchOp::TrackInsert {
            anchor: anchor.clone(),
            kind: "paragraph".to_string(),
            author: "alice".to_string(),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_track_delete() {
        let elem = ElementId::content_anchored("paragraph", &["section:0"], "elem");
        let op = PatchOp::TrackDelete {
            elem: elem.clone(),
            author: "bob".to_string(),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    // ------------------------------------------------------------------
    // Apply-on-IR tests
    // ------------------------------------------------------------------

    #[test]
    fn apply_replace_text_in_tree() {
        let mut ir = make_sample_doc();
        let run_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:0", "run:0"], "hello");
        let op = PatchOp::ReplaceText {
            elem: run_id.clone(),
            old: "hello".to_string(),
            new: "world".to_string(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert_eq!(result.changed_element_ids[0], run_id);
        assert_eq!(ir.root.children[0].children[0].tag, "world".to_string());
    }

    #[test]
    fn apply_replace_text_not_found() {
        let mut ir = make_sample_doc();
        let fake_id = ElementId::content_anchored("run", &["fake"], "nonexistent");
        let op = PatchOp::ReplaceText {
            elem: fake_id,
            old: "x".to_string(),
            new: "y".to_string(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert!(result.changed_element_ids.is_empty());
    }

    #[test]
    fn apply_delete_node() {
        let mut ir = make_sample_doc();
        let para_id = ElementId::content_anchored("paragraph", &["section:0"], "hello");
        let op = PatchOp::Delete {
            elem: para_id.clone(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert!(ir.root.children.is_empty());
    }

    #[test]
    fn apply_delete_not_found() {
        let mut ir = make_sample_doc();
        let fake_id = ElementId::content_anchored("paragraph", &["fake"], "nonexistent");
        let op = PatchOp::Delete { elem: fake_id };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert!(result.changed_element_ids.is_empty());
        assert_eq!(ir.root.children.len(), 1);
    }

    #[test]
    fn apply_set_run_props() {
        let mut ir = make_sample_doc();
        let run_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:0", "run:0"], "hello");
        let props = RunProperties {
            bold: true,
            italic: true,
            font_name: Some("Serif".to_string()),
            font_size_pt: Some(12.0),
        };
        let op = PatchOp::SetRunProps {
            elem: run_id.clone(),
            props: props.clone(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert_eq!(result.changed_element_ids[0], run_id);
    }

    #[test]
    fn apply_set_cell_value() {
        let mut ir = make_sample_doc();
        let cell_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:0", "run:0"], "hello");
        let op = PatchOp::SetCellValue {
            cell: cell_id.clone(),
            value: "cell_value".to_string(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert_eq!(ir.root.children[0].children[0].tag, "cell_value");
    }

    #[test]
    fn apply_unimplemented_insert() {
        let mut ir = make_sample_doc();
        let anchor = ElementId::content_anchored("paragraph", &["section:0"], "anchor");
        let op = PatchOp::Insert {
            anchor: anchor.clone(),
            kind: "paragraph".to_string(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert!(result.changed_element_ids.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "GP-I-001");
    }

    // ------------------------------------------------------------------
    // Diagnostic / validate op tests
    // ------------------------------------------------------------------

    #[test]
    fn validate_op_all_known() {
        let anchor = ElementId::content_anchored("run", &["section:0"], "anchor");
        let elem = ElementId::content_anchored("run", &["section:0"], "elem");

        assert!(validate_op(&PatchOp::Insert {
            anchor: anchor.clone(),
            kind: "p".to_string(),
        })
        .is_ok());
        assert!(validate_op(&PatchOp::ReplaceText {
            elem: elem.clone(),
            old: "x".to_string(),
            new: "y".to_string(),
        })
        .is_ok());
        assert!(validate_op(&PatchOp::Delete { elem: elem.clone() }).is_ok());
        assert!(validate_op(&PatchOp::SetRunProps {
            elem: elem.clone(),
            props: RunProperties {
                bold: false,
                italic: false,
                font_name: None,
                font_size_pt: None,
            },
        })
        .is_ok());
        assert!(validate_op(&PatchOp::SetCellValue {
            cell: elem.clone(),
            value: "v".to_string(),
        })
        .is_ok());
        assert!(validate_op(&PatchOp::TrackInsert {
            anchor: anchor.clone(),
            kind: "p".to_string(),
            author: "a".to_string(),
            timestamp: "t".to_string(),
        })
        .is_ok());
        assert!(validate_op(&PatchOp::TrackDelete {
            elem: elem.clone(),
            author: "a".to_string(),
            timestamp: "t".to_string(),
        })
        .is_ok());
    }

    #[test]
    fn fidelity_budget_default() {
        let fb = FidelityBudget::default();
        assert!(fb.dropped_features.is_empty());
        assert!(fb.part_warnings.is_empty());
        assert!(fb.unknown_parts.is_empty());
        assert_eq!(fb.confidence, 1.0);
    }

    #[test]
    fn apply_result_serializes() {
        let result = ApplyResult {
            diagnostics: vec![Diagnostic::new(
                "GP-I-001",
                crate::diagnostics::Severity::Info,
                "test",
            )],
            changed_element_ids: vec![ElementId::content_anchored("run", &["test"], "id")],
            fidelity_budget: FidelityBudget::default(),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let back: ApplyResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result.diagnostics.len(), back.diagnostics.len());
        assert_eq!(
            result.changed_element_ids.len(),
            back.changed_element_ids.len()
        );
    }

    #[test]
    fn run_properties_serializes() {
        let props = RunProperties {
            bold: true,
            italic: false,
            font_name: Some("Arial".into()),
            font_size_pt: Some(14.0),
        };
        let json = serde_json::to_string(&props).expect("serialize");
        let back: RunProperties = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(props, back);

        let props_no_opts = RunProperties {
            bold: false,
            italic: false,
            font_name: None,
            font_size_pt: None,
        };
        let json2 = serde_json::to_string(&props_no_opts).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json2).expect("parse");
        assert!(parsed["font_name"].is_null());
        assert!(parsed["font_size_pt"].is_null());
    }

    #[test]
    fn apply_op_preserves_ir_version() {
        let mut ir = make_sample_doc();
        ir.ir_version = "2.0.0".to_string();
        let result_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:0", "run:0"], "hello");
        let op = PatchOp::ReplaceText {
            elem: result_id,
            old: "hello".to_string(),
            new: "world".to_string(),
        };
        let _ = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(ir.ir_version, "2.0.0");
    }

    #[test]
    fn apply_delete_preserves_siblings() {
        let mut ir = make_sample_doc();
        // Add a second sibling to verify it's preserved
        let sibling_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:1", "run:0"], "sibling");
        ir.root.children.push(TaggedNode {
            id: sibling_id.clone(),
            tag: "sibling".to_string(),
            alt_text: None,
            bbox: None,
            children: Vec::new(),
        });
        assert_eq!(ir.root.children.len(), 2);

        let para_id = ElementId::content_anchored("paragraph", &["section:0"], "hello");
        let op = PatchOp::Delete {
            elem: para_id.clone(),
        };
        let _ = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(ir.root.children.len(), 1);
        assert_eq!(ir.root.children[0].id, sibling_id);
    }
}
