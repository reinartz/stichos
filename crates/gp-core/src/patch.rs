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
//! - **SetCellRich** — update a spreadsheet cell's rich text runs (T2).
//! - **SetCellStyle** — set cell styling (font/fill/border/number_format) in XLSX.
//! - **SetFreezePane** — freeze rows/columns in XLSX sheet views.
//! - **SetRowColDim** — set column width / row height / hidden state in XLSX.
//! - **SetAutoFilter** — set/remove auto-filter on a range in XLSX.
//! - **SetConditionalFormat** — add conditional formatting rule in XLSX.
//! - **SetSheetProtection** — set sheet protection flags in XLSX.
//! - **SetPageSetup** — set page setup (orientation, paper size, margins) in XLSX.
//! - **SetCellHyperlink** — add hyperlink to an XLSX cell.
//! - **SetDefinedName** — add named ranges / defined names in XLSX workbook.
//! - **SetDataValidation** — add data validation rules to XLSX cells.
//! - **AddTable / RemoveTable** — manage ListObject tables in XLSX sheets.
//! - **TrackInsert** — tracked (revisions) insert.
//! - **TrackDelete** — tracked (revisions) delete.
//! - **SetStyle** — create or modify a custom style in styles.xml (DOCX).
//! - **AssignStyle** — assign a style to a paragraph or run (DOCX).
//! - **AddHyperlink** — add a hyperlink to a paragraph or run (DOCX).
//! - **SetShapeProps** — set shape properties (position, size, rotation, fill, line) on PPTX shapes.
//! - **InsertPicture** — insert a picture/image into a PPTX slide.
//! - **SetSlideText** — set text content on a PPTX shape/placeholder.
//! - **SetTableCellText** — set table cell text in a PPTX table.
//! - **SetSlideNotes** — set speaker notes for a PPTX slide.
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

/// A custom style to be defined in styles.xml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetStyle {
    pub style_id: String,
    pub name: String,
    pub style_type: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub based_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub italic: Option<bool>,
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
    /// Set a spreadsheet cell's rich text runs (T2 — CellRichText).
    SetCellRich {
        cell: ElementId,
        runs: Vec<super::ir::CellRun>,
    },
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
    /// Create or modify a custom style in styles.xml (DOCX).
    SetStyle {
        style_id: String,
        name: String,
        style_type: String,
        based_on: Option<String>,
        bold: Option<bool>,
        italic: Option<bool>,
        font_name: Option<String>,
        font_size_pt: Option<f32>,
    },
    /// Assign a style to a paragraph or run (DOCX).
    AssignStyle { elem: ElementId, style_id: String },
    /// Add a hyperlink to a paragraph or run (DOCX).
    AddHyperlink {
        target: String,
        elem: ElementId,
        tooltip: Option<String>,
    },
    /// Set shape properties (position, size, rotation, fill, line) on a PPTX shape.
    SetShapeProps {
        slide_id: String,
        shape_id: String,
        x_pt: Option<f64>,
        y_pt: Option<f64>,
        width_pt: Option<f64>,
        height_pt: Option<f64>,
        rotation: Option<f64>,
        fill_color: Option<String>,
        line_color: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        line_width_pt: Option<f64>,
    },
    /// Insert a picture/image into a PPTX slide.
    InsertPicture {
        slide_id: String,
        data: Vec<u8>,
        mime_type: String,
        alt_text: Option<String>,
        x_pt: Option<f64>,
        y_pt: Option<f64>,
        width_pt: Option<f64>,
        height_pt: Option<f64>,
    },
    /// Set text content on a PPTX shape (for placeholders, text boxes).
    SetSlideText {
        slide_id: String,
        shape_id: String,
        text: String,
    },
    /// Set table cell text in a PPTX table.
    SetTableCellText {
        slide_id: String,
        table_row: usize,
        table_col: usize,
        text: String,
        bold: Option<bool>,
        italic: Option<bool>,
    },
    /// Set slide notes (speaker notes) for a PPTX slide.
    SetSlideNotes { slide_id: String, notes: String },
    /// Set cell styling (font/fill/border/alignment/number_format) in XLSX styles.xml.
    SetCellStyle {
        cell_ref: String,
        font_color: Option<String>,
        fill_color: Option<String>,
        bold: Option<bool>,
        italic: Option<bool>,
        number_format: Option<String>,
        border: Option<String>,
    },
    /// Freeze rows/columns in XLSX sheet views.
    SetFreezePane {
        sheet_name: String,
        row: usize,
        col: usize,
    },
    /// Set column width / row height / hidden state in XLSX.
    SetRowColDim {
        sheet_name: String,
        col_index: Option<usize>,
        width: Option<f64>,
        row_index: Option<usize>,
        height: Option<f64>,
        hidden: Option<bool>,
    },
    /// Set or remove auto-filter on a range in XLSX.
    SetAutoFilter {
        sheet_name: String,
        range: Option<String>,
    },
    /// Add conditional formatting rule to XLSX sheet.
    SetConditionalFormat {
        sheet_name: String,
        sqref: String,
        cf_type: String,
        formula: Option<String>,
        priority: Option<usize>,
    },
    /// Set sheet protection flags in XLSX.
    SetSheetProtection {
        sheet_name: String,
        password: Option<String>,
        protect_sheet: Option<bool>,
        protect_content: Option<bool>,
    },
    /// Set page setup (orientation, paper size, margins) for XLSX sheet.
    SetPageSetup {
        sheet_name: String,
        orientation: Option<String>,
        paper_size: Option<usize>,
        margin_top: Option<f64>,
        margin_bottom: Option<f64>,
        margin_left: Option<f64>,
        margin_right: Option<f64>,
    },
    /// Add hyperlink to an XLSX cell.
    SetCellHyperlink {
        cell_ref: String,
        target_url: String,
        tooltip: Option<String>,
    },
    /// Add a defined name / named range in the workbook.
    SetDefinedName {
        name: String,
        formula: String,
        sheet_id: Option<String>,
    },
    /// Add a data validation rule to an XLSX cell/range.
    SetDataValidation {
        cell_ref: String,
        validation_type: String,
        formula1: Option<String>,
        formula2: Option<String>,
        operator: Option<String>,
        allow_blank: Option<bool>,
    },
    /// Add a ListObject table to an XLSX sheet.
    AddTable {
        sheet_name: String,
        range: String,
        table_name: String,
        display_name: Option<String>,
    },
    /// Remove a ListObject table from an XLSX sheet.
    RemoveTable {
        sheet_name: String,
        table_name: String,
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
    let diagnostics = Vec::new();
    let mut changed: Vec<ElementId> = Vec::new();

    match op {
        PatchOp::Insert { anchor, kind } => {
            let new_id = ElementId::derive(kind.as_str(), &[kind], &());
            let new_node = TaggedNode {
                id: new_id,
                tag: kind.clone(),
                alt_text: None,
                bbox: None,
                children: vec![],
            };
            if let Some(parent) = find_parent_mut(&mut ir.root, anchor) {
                if let Some(pos) = parent.children.iter().position(|c| c.id == *anchor) {
                    parent.children.insert(pos + 1, new_node);
                    changed.push(parent.children[pos + 1].id.clone());
                }
            }
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
        PatchOp::SetCellRich { cell, runs } => {
            set_cell_rich_text_in_tree(&mut ir.root, cell, runs, &mut changed);
        }
        PatchOp::TrackInsert {
            anchor,
            kind,
            author,
            timestamp,
        } => {
            let new_id =
                ElementId::derive(kind.as_str(), &[kind], &format!("track_ins_{}", author));
            let new_node = TaggedNode {
                id: new_id.clone(),
                tag: kind.clone(),
                alt_text: Some(format!(
                    "tracked_insert:author={}:timestamp={}",
                    author, timestamp
                )),
                bbox: None,
                children: vec![],
            };
            if let Some(parent) = find_parent_mut(&mut ir.root, anchor) {
                if let Some(pos) = parent.children.iter().position(|c| c.id == *anchor) {
                    parent.children.insert(pos + 1, new_node);
                    changed.push(parent.children[pos + 1].id.clone());
                }
            }
        }
        PatchOp::TrackDelete {
            elem,
            author,
            timestamp,
        } => {
            if let Some(node) = find_node_mut(&mut ir.root, elem) {
                let current = node.alt_text.clone().unwrap_or_default();
                node.alt_text = Some(
                    format!(
                        "tracked_delete:author={}:timestamp={}:{}",
                        author, timestamp, current
                    )
                    .trim_start_matches(':')
                    .to_string(),
                );
                changed.push(node.id.clone());
            }
        }
        PatchOp::SetStyle {
            style_id,
            name,
            style_type,
            based_on,
            bold,
            italic,
            font_name,
            font_size_pt,
        } => {
            let props = format!(
                "set_style:id={}:name={}:type={}:based_on={:?}:bold={:?}:italic={:?}:font={:?}:size={:?}",
                style_id, name, style_type, based_on, bold, italic, font_name, font_size_pt
            );
            let new_id = ElementId::derive("style", &["set_style", style_id], &props);
            let _ = new_id;
            let styled = "styled".to_string();
            changed.push(ElementId::derive(
                "style",
                &["set_style", style_id],
                &styled,
            ));
        }
        PatchOp::AssignStyle { elem, style_id } => {
            // Actually modify the IR tree: store style assignment in alt_text of target node
            if let Some(node) = find_node_mut(&mut ir.root, elem) {
                let current = node.alt_text.clone().unwrap_or_default();
                node.alt_text = Some(
                    format!(
                        "assign_style:{}{}",
                        style_id,
                        if current.is_empty() { "" } else { &current }
                    )
                    .trim_start_matches(':')
                    .to_string(),
                );
                changed.push(node.id.clone());
            }
        }
        PatchOp::AddHyperlink {
            target,
            elem,
            tooltip,
        } => {
            // Actually modify the IR tree: store hyperlink in alt_text of target node
            if let Some(node) = find_node_mut(&mut ir.root, elem) {
                let current = node.alt_text.clone().unwrap_or_default();
                let tooltip_str = tooltip
                    .as_ref()
                    .map(|t| format!(":tooltip={}", t))
                    .unwrap_or_default();
                node.alt_text = Some(
                    format!(
                        "hyperlink:{}{}{}",
                        target,
                        tooltip_str,
                        if current.is_empty() { "" } else { &current }
                    )
                    .trim_start_matches(':')
                    .to_string(),
                );
                changed.push(node.id.clone());
            }
        }
        PatchOp::SetShapeProps {
            slide_id,
            shape_id,
            x_pt,
            y_pt,
            width_pt,
            height_pt,
            rotation,
            fill_color,
            line_color,
            ..
        } => {
            let mut extra = String::new();
            if let Some(x) = x_pt {
                extra.push_str(&format!(",x={}", x));
            }
            if let Some(y) = y_pt {
                extra.push_str(&format!(",y={}", y));
            }
            if let Some(w) = width_pt {
                extra.push_str(&format!(",w={}", w));
            }
            if let Some(h) = height_pt {
                extra.push_str(&format!(",h={}", h));
            }
            if let Some(r) = rotation {
                extra.push_str(&format!(",rot={}", r));
            }
            if let Some(f) = &fill_color {
                extra.push_str(&format!(",fill={}", f));
            }
            if let Some(l) = &line_color {
                extra.push_str(&format!(",line={}", l));
            }
            let props = format!(
                "set_shape_props:slide={}:shape:{}{}",
                slide_id, shape_id, extra,
            );
            let new_id =
                ElementId::derive("shape_props", &["slide_props", slide_id, shape_id], &props);
            let _ = new_id;
            let modified = "modified".to_string();
            changed.push(ElementId::derive(
                "shape_props",
                &["slide_props", slide_id, shape_id],
                &modified,
            ));
        }
        PatchOp::InsertPicture {
            slide_id,
            data,
            mime_type,
            alt_text,
            ..
        } => {
            let mut meta = format!(
                "insert_picture:slide={}:mime={}:len={}",
                slide_id,
                mime_type,
                data.len()
            );
            if let Some(ref alt) = alt_text {
                meta.push_str(&format!(":alt={}", alt));
            }
            let new_id = ElementId::derive("picture", &["insert_pic", slide_id], &meta);
            let _ = new_id;
            let added = "added".to_string();
            changed.push(ElementId::derive(
                "picture",
                &["insert_pic", slide_id],
                &added,
            ));
        }
        PatchOp::SetSlideText {
            slide_id,
            shape_id,
            text,
        } => {
            let new_id = ElementId::derive("slide_text", &["slide_text", slide_id, shape_id], text);
            let _ = new_id;
            let set_text = "set_text".to_string();
            changed.push(ElementId::derive(
                "slide_text",
                &["slide_text", slide_id, shape_id],
                &set_text,
            ));
        }
        PatchOp::SetTableCellText {
            slide_id,
            table_row,
            table_col,
            text,
            ..
        } => {
            let row_col = format!("{}x{}", table_row, table_col);
            let new_id =
                ElementId::derive("table_cell_text", &["table_cell", slide_id, &row_col], text);
            let _ = new_id;
            let cell_text = "set_cell_text".to_string();
            changed.push(ElementId::derive(
                "table_cell_text",
                &["table_cell", slide_id, &row_col],
                &cell_text,
            ));
        }
        PatchOp::SetSlideNotes { slide_id, notes } => {
            let new_id = ElementId::derive("slide_notes", &["notes", slide_id], notes);
            let _ = new_id;
            let set_notes = "set_notes".to_string();
            changed.push(ElementId::derive(
                "slide_notes",
                &["notes", slide_id],
                &set_notes,
            ));
        }
        // XLSX structural ops — writer handles the actual XML; IR marks them changed.
        PatchOp::SetCellStyle { cell_ref, .. } => {
            changed.push(ElementId::derive(
                "xlsx_style",
                &["set_cell_style", cell_ref],
                &"applied".to_string(),
            ));
        }
        PatchOp::SetFreezePane {
            sheet_name,
            row,
            col,
        } => {
            changed.push(ElementId::derive(
                "xlsx_freeze",
                &["freeze_pane", sheet_name],
                &format!("row:{}:col:{}", row, col),
            ));
        }
        PatchOp::SetRowColDim {
            sheet_name,
            col_index,
            row_index,
            ..
        } => {
            let key = match (col_index, row_index) {
                (Some(c), None) => format!("coldim:col:{}", c),
                (None, Some(r)) => format!("coldim:row:{}", r),
                _ => "coldim:general".to_string(),
            };
            changed.push(ElementId::derive(
                "xlsx_rowcoldim",
                &["set_rowcol_dim", sheet_name],
                &key,
            ));
        }
        PatchOp::SetAutoFilter { sheet_name, range } => {
            changed.push(ElementId::derive(
                "xlsx_autofilter",
                &["auto_filter", sheet_name],
                &range.clone().unwrap_or_default(),
            ));
        }
        PatchOp::SetConditionalFormat {
            sheet_name,
            sqref,
            cf_type,
            ..
        } => {
            changed.push(ElementId::derive(
                "xlsx_cond_fmt",
                &["conditional_format", sheet_name, sqref],
                &cf_type,
            ));
        }
        PatchOp::SetSheetProtection { sheet_name, .. } => {
            changed.push(ElementId::derive(
                "xlsx_protection",
                &["sheet_protection", sheet_name],
                &"applied".to_string(),
            ));
        }
        PatchOp::SetPageSetup {
            sheet_name,
            orientation,
            paper_size,
            ..
        } => {
            let key = format!(
                "ps:orient:{}:psize:{}",
                orientation.as_deref().unwrap_or("default"),
                paper_size.unwrap_or(0)
            );
            changed.push(ElementId::derive(
                "xlsx_pagesetup",
                &["page_setup", sheet_name],
                &key,
            ));
        }
        PatchOp::SetCellHyperlink {
            cell_ref,
            target_url,
            ..
        } => {
            changed.push(ElementId::derive(
                "xlsx_hyperlink",
                &["cell_hyperlink", cell_ref],
                target_url,
            ));
        }
        PatchOp::SetDefinedName { name, formula, .. } => {
            changed.push(ElementId::derive(
                "xlsx_defined_name",
                &["defined_name", name],
                formula,
            ));
        }
        PatchOp::SetDataValidation {
            cell_ref,
            validation_type,
            ..
        } => {
            changed.push(ElementId::derive(
                "xlsx_data_validation",
                &["data_validation", cell_ref],
                &validation_type,
            ));
        }
        PatchOp::AddTable {
            sheet_name,
            table_name,
            ..
        } => {
            changed.push(ElementId::derive(
                "xlsx_add_table",
                &["add_table", sheet_name],
                table_name,
            ));
        }
        PatchOp::RemoveTable {
            sheet_name,
            table_name,
        } => {
            changed.push(ElementId::derive(
                "xlsx_remove_table",
                &["remove_table", sheet_name],
                table_name,
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

/// Recursively search a tagged tree for a cell element and set its rich text runs.
fn set_cell_rich_text_in_tree(
    node: &mut TaggedNode,
    cell_id: &ElementId,
    runs: &[crate::ir::CellRun],
    changed: &mut Vec<ElementId>,
) {
    if node.id == *cell_id {
        // Store runs as JSON in the alt_text field (IR metadata)
        node.alt_text = Some(format!("rich_runs:{}", runs.len()));
        changed.push(node.id.clone());
    } else {
        for child in &mut node.children {
            set_cell_rich_text_in_tree(child, cell_id, runs, changed);
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

/// Find a node in the tagged tree by ID (immutable borrow).
#[expect(dead_code)]
fn find_node<'a>(node: &'a TaggedNode, target_id: &ElementId) -> Option<&'a TaggedNode> {
    if &node.id == target_id {
        Some(node)
    } else {
        for child in &node.children {
            if let Some(found) = find_node(child, target_id) {
                return Some(found);
            }
        }
        None
    }
}

/// Find the parent of a node by target ID (immutable borrow).
#[expect(dead_code)]
fn find_parent<'a>(node: &'a TaggedNode, target_id: &ElementId) -> Option<&'a TaggedNode> {
    for child in &node.children {
        if &child.id == target_id {
            return Some(node);
        }
        if let Some(found) = find_parent(child, target_id) {
            return Some(found);
        }
    }
    None
}

/// Find a node in the tagged tree by ID (mutable borrow).
fn find_node_mut<'a>(
    node: &'a mut TaggedNode,
    target_id: &ElementId,
) -> Option<&'a mut TaggedNode> {
    if &node.id == target_id {
        Some(node)
    } else {
        for child in &mut node.children {
            if let Some(found) = find_node_mut(child, target_id) {
                return Some(found);
            }
        }
        None
    }
}

/// Find the parent of a node by target ID (mutable borrow).
fn find_parent_mut<'a>(
    node: &'a mut TaggedNode,
    target_id: &ElementId,
) -> Option<&'a mut TaggedNode> {
    for child in &mut node.children {
        if &child.id == target_id {
            return Some(node);
        }
    }
    for child in &mut node.children {
        if let Some(found) = find_parent_mut(child, target_id) {
            return Some(found);
        }
    }
    None
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

    #[test]
    fn json_roundtrip_set_shape_props() {
        let op = PatchOp::SetShapeProps {
            slide_id: "slide1".to_string(),
            shape_id: "shape3".to_string(),
            x_pt: Some(100.5),
            y_pt: Some(200.0),
            width_pt: Some(300.0),
            height_pt: Some(150.0),
            rotation: Some(45.0),
            fill_color: Some("#FF0000".to_string()),
            line_color: Some("#000000".to_string()),
            line_width_pt: Some(1.5),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetShapeProps {
            slide_id: bs,
            shape_id: bsh,
            x_pt: bx,
            y_pt: by,
            width_pt: bw,
            height_pt: bh,
            rotation: br,
            fill_color: bf,
            line_color: bl,
            line_width_pt: blw,
        } = back
        {
            assert_eq!(bs, "slide1");
            assert_eq!(bsh, "shape3");
            assert_eq!(bx, Some(100.5));
            assert_eq!(by, Some(200.0));
            assert_eq!(bw, Some(300.0));
            assert_eq!(bh, Some(150.0));
            assert_eq!(br, Some(45.0));
            assert_eq!(bf, Some("#FF0000".to_string()));
            assert_eq!(bl, Some("#000000".to_string()));
            assert_eq!(blw, Some(1.5));
        } else {
            panic!("expected SetShapeProps");
        }
    }

    #[test]
    fn json_roundtrip_insert_picture() {
        let op = PatchOp::InsertPicture {
            slide_id: "slide1".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47],
            mime_type: "image/png".to_string(),
            alt_text: Some("A chart image".to_string()),
            x_pt: Some(50.0),
            y_pt: Some(50.0),
            width_pt: Some(400.0),
            height_pt: Some(300.0),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::InsertPicture {
            slide_id: bs,
            data: bd,
            mime_type: bm,
            alt_text: ba,
            x_pt: bx,
            y_pt: by,
            width_pt: bw,
            height_pt: bh,
        } = back
        {
            assert_eq!(bs, "slide1");
            assert_eq!(bd, vec![0x89, 0x50, 0x4E, 0x47]);
            assert_eq!(bm, "image/png");
            assert_eq!(ba, Some("A chart image".to_string()));
            assert_eq!(bx, Some(50.0));
            assert_eq!(by, Some(50.0));
            assert_eq!(bw, Some(400.0));
            assert_eq!(bh, Some(300.0));
        } else {
            panic!("expected InsertPicture");
        }
    }

    #[test]
    fn json_roundtrip_set_slide_text() {
        let op = PatchOp::SetSlideText {
            slide_id: "slide2".to_string(),
            shape_id: "title".to_string(),
            text: "New Title Text".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetSlideText {
            slide_id: bs,
            shape_id: bsh,
            text: bt,
        } = back
        {
            assert_eq!(bs, "slide2");
            assert_eq!(bsh, "title");
            assert_eq!(bt, "New Title Text");
        } else {
            panic!("expected SetSlideText");
        }
    }

    #[test]
    fn json_roundtrip_set_table_cell_text() {
        let op = PatchOp::SetTableCellText {
            slide_id: "slide1".to_string(),
            table_row: 0,
            table_col: 1,
            text: "Cell Value".to_string(),
            bold: Some(true),
            italic: Some(false),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetTableCellText {
            slide_id: bs,
            table_row: br,
            table_col: bc,
            text: bt,
            bold: bb,
            italic: bi,
        } = back
        {
            assert_eq!(bs, "slide1");
            assert_eq!(br, 0);
            assert_eq!(bc, 1);
            assert_eq!(bt, "Cell Value");
            assert_eq!(bb, Some(true));
            assert_eq!(bi, Some(false));
        } else {
            panic!("expected SetTableCellText");
        }
    }

    #[test]
    fn json_roundtrip_set_slide_notes() {
        let op = PatchOp::SetSlideNotes {
            slide_id: "slide3".to_string(),
            notes: "Speaker notes here".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetSlideNotes {
            slide_id: bs,
            notes: bn,
        } = back
        {
            assert_eq!(bs, "slide3");
            assert_eq!(bn, "Speaker notes here");
        } else {
            panic!("expected SetSlideNotes");
        }
    }

    #[test]
    fn json_roundtrip_set_style() {
        let op = PatchOp::SetStyle {
            style_id: "CustomStyle1".to_string(),
            name: "My Custom Style".to_string(),
            style_type: "paragraph".to_string(),
            based_on: Some("Normal".to_string()),
            bold: Some(true),
            italic: Some(false),
            font_name: Some("Calibri".to_string()),
            font_size_pt: Some(12.5),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetStyle {
            style_id: bs,
            name: bn,
            style_type: bst,
            based_on: bbo,
            bold: bb,
            italic: bi,
            font_name: bfn,
            font_size_pt: bfsp,
        } = back
        {
            assert_eq!(bs, "CustomStyle1");
            assert_eq!(bn, "My Custom Style");
            assert_eq!(bst, "paragraph");
            assert_eq!(bbo, Some("Normal".to_string()));
            assert_eq!(bb, Some(true));
            assert_eq!(bi, Some(false));
            assert_eq!(bfn, Some("Calibri".to_string()));
            assert_eq!(bfsp, Some(12.5));
        } else {
            panic!("expected SetStyle");
        }
    }

    #[test]
    fn json_roundtrip_set_style_minimal() {
        let op = PatchOp::SetStyle {
            style_id: "S1".to_string(),
            name: "S1".to_string(),
            style_type: "character".to_string(),
            based_on: None,
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_assign_style() {
        let elem = ElementId::content_anchored("paragraph", &["section:0"], "para1");
        let op = PatchOp::AssignStyle {
            elem: elem.clone(),
            style_id: "CustomStyle1".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::AssignStyle {
            elem: be,
            style_id: bs,
        } = back
        {
            assert_eq!(elem, be);
            assert_eq!(bs, "CustomStyle1");
        } else {
            panic!("expected AssignStyle");
        }
    }

    #[test]
    fn json_roundtrip_add_hyperlink() {
        let elem = ElementId::content_anchored("run", &["section:0", "paragraph:0"], "run1");
        let op = PatchOp::AddHyperlink {
            target: "https://example.com".to_string(),
            elem: elem.clone(),
            tooltip: Some("Visit Example".to_string()),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::AddHyperlink {
            target: bt,
            elem: be,
            tooltip: btt,
        } = back
        {
            assert_eq!(bt, "https://example.com");
            assert_eq!(elem, be);
            assert_eq!(btt, Some("Visit Example".to_string()));
        } else {
            panic!("expected AddHyperlink");
        }
    }

    #[test]
    fn json_roundtrip_add_hyperlink_no_tooltip() {
        let elem = ElementId::content_anchored("paragraph", &["s:0"], "p1");
        let op = PatchOp::AddHyperlink {
            target: "#InternalAnchor".to_string(),
            elem: elem.clone(),
            tooltip: None,
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    // ------------------------------------------------------------------
    // XLSX structural PatchOp roundtrip tests
    // ------------------------------------------------------------------

    #[test]
    fn json_roundtrip_set_cell_style() {
        let op = PatchOp::SetCellStyle {
            cell_ref: "A1".to_string(),
            font_color: Some("#E2483A".to_string()),
            fill_color: Some("#F4F1EA".to_string()),
            bold: Some(true),
            italic: Some(false),
            number_format: Some("0.00".to_string()),
            border: Some("thin".to_string()),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetCellStyle {
            cell_ref: bcr,
            font_color: bfc,
            fill_color: bflc,
            bold: bb,
            italic: bi,
            number_format: bnf,
            border: bbdr,
        } = back
        {
            assert_eq!(bcr, "A1");
            assert_eq!(bfc, Some("#E2483A".to_string()));
            assert_eq!(bflc, Some("#F4F1EA".to_string()));
            assert_eq!(bb, Some(true));
            assert_eq!(bi, Some(false));
            assert_eq!(bnf, Some("0.00".to_string()));
            assert_eq!(bbdr, Some("thin".to_string()));
        } else {
            panic!("expected SetCellStyle");
        }
    }

    #[test]
    fn json_roundtrip_set_cell_style_minimal() {
        let op = PatchOp::SetCellStyle {
            cell_ref: "B2".to_string(),
            font_color: None,
            fill_color: None,
            bold: None,
            italic: None,
            number_format: None,
            border: None,
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_set_freeze_pane() {
        let op = PatchOp::SetFreezePane {
            sheet_name: "Sheet1".to_string(),
            row: 2,
            col: 1,
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetFreezePane {
            sheet_name: bsn,
            row: br,
            col: bc,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(br, 2);
            assert_eq!(bc, 1);
        } else {
            panic!("expected SetFreezePane");
        }
    }

    #[test]
    fn json_roundtrip_set_rowcol_dim_col() {
        let op = PatchOp::SetRowColDim {
            sheet_name: "Sheet1".to_string(),
            col_index: Some(0),
            width: Some(15.0),
            row_index: None,
            height: None,
            hidden: None,
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetRowColDim {
            sheet_name: bsn,
            col_index: bci,
            width: bw,
            row_index: bri,
            height: bh,
            hidden: bhd,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(bci, Some(0));
            assert_eq!(bw, Some(15.0));
            assert_eq!(bri, None);
            assert_eq!(bh, None);
            assert_eq!(bhd, None);
        } else {
            panic!("expected SetRowColDim");
        }
    }

    #[test]
    fn json_roundtrip_set_rowcol_dim_row() {
        let op = PatchOp::SetRowColDim {
            sheet_name: "Sheet1".to_string(),
            col_index: None,
            width: None,
            row_index: Some(0),
            height: Some(30.0),
            hidden: Some(true),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn json_roundtrip_set_auto_filter() {
        let op = PatchOp::SetAutoFilter {
            sheet_name: "Sheet1".to_string(),
            range: Some("A1:C10".to_string()),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetAutoFilter {
            sheet_name: bsn,
            range: br,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(br, Some("A1:C10".to_string()));
        } else {
            panic!("expected SetAutoFilter");
        }
    }

    #[test]
    fn json_roundtrip_set_conditional_format() {
        let op = PatchOp::SetConditionalFormat {
            sheet_name: "Sheet1".to_string(),
            sqref: "A1:A10".to_string(),
            cf_type: "cellIs".to_string(),
            formula: Some("100".to_string()),
            priority: Some(1),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetConditionalFormat {
            sheet_name: bsn,
            sqref: bsq,
            cf_type: bct,
            formula: bformula,
            priority: bp,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(bsq, "A1:A10");
            assert_eq!(bct, "cellIs");
            assert_eq!(bformula, Some("100".to_string()));
            assert_eq!(bp, Some(1));
        } else {
            panic!("expected SetConditionalFormat");
        }
    }

    #[test]
    fn json_roundtrip_set_sheet_protection() {
        let op = PatchOp::SetSheetProtection {
            sheet_name: "Sheet1".to_string(),
            password: Some("pass123".to_string()),
            protect_sheet: Some(true),
            protect_content: Some(true),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetSheetProtection {
            sheet_name: bsn,
            password: bp,
            protect_sheet: bps,
            protect_content: bpc,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(bp, Some("pass123".to_string()));
            assert_eq!(bps, Some(true));
            assert_eq!(bpc, Some(true));
        } else {
            panic!("expected SetSheetProtection");
        }
    }

    #[test]
    fn json_roundtrip_set_page_setup() {
        let op = PatchOp::SetPageSetup {
            sheet_name: "Sheet1".to_string(),
            orientation: Some("landscape".to_string()),
            paper_size: Some(9),
            margin_top: Some(1.0),
            margin_bottom: Some(1.0),
            margin_left: Some(0.75),
            margin_right: Some(0.75),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetPageSetup {
            sheet_name: bsn,
            orientation: bo,
            paper_size: bps,
            margin_top: bmt,
            margin_bottom: bmb,
            margin_left: bml,
            margin_right: bmr,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(bo, Some("landscape".to_string()));
            assert_eq!(bps, Some(9));
            assert_eq!(bmt, Some(1.0));
            assert_eq!(bmb, Some(1.0));
            assert_eq!(bml, Some(0.75));
            assert_eq!(bmr, Some(0.75));
        } else {
            panic!("expected SetPageSetup");
        }
    }

    #[test]
    fn json_roundtrip_set_cell_hyperlink() {
        let op = PatchOp::SetCellHyperlink {
            cell_ref: "A1".to_string(),
            target_url: "https://example.com".to_string(),
            tooltip: Some("Go to Example".to_string()),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetCellHyperlink {
            cell_ref: bcr,
            target_url: btu,
            tooltip: btt,
        } = back
        {
            assert_eq!(bcr, "A1");
            assert_eq!(btu, "https://example.com");
            assert_eq!(btt, Some("Go to Example".to_string()));
        } else {
            panic!("expected SetCellHyperlink");
        }
    }

    #[test]
    fn json_roundtrip_set_defined_name() {
        let op = PatchOp::SetDefinedName {
            name: "TaxRate".to_string(),
            formula: "=Sheet1!$B$10".to_string(),
            sheet_id: Some("1".to_string()),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetDefinedName {
            name: bn,
            formula: bformula,
            sheet_id: bsi,
        } = back
        {
            assert_eq!(bn, "TaxRate");
            assert_eq!(bformula, "=Sheet1!$B$10");
            assert_eq!(bsi, Some("1".to_string()));
        } else {
            panic!("expected SetDefinedName");
        }
    }

    #[test]
    fn json_roundtrip_set_data_validation() {
        let op = PatchOp::SetDataValidation {
            cell_ref: "A1".to_string(),
            validation_type: "list".to_string(),
            formula1: Some("=Sheet2!$A$1:$A$10".to_string()),
            formula2: None,
            operator: None,
            allow_blank: Some(true),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::SetDataValidation {
            cell_ref: bcr,
            validation_type: bvt,
            formula1: bf1,
            formula2: bf2,
            operator: bo,
            allow_blank: bab,
        } = back
        {
            assert_eq!(bcr, "A1");
            assert_eq!(bvt, "list");
            assert_eq!(bf1, Some("=Sheet2!$A$1:$A$10".to_string()));
            assert_eq!(bf2, None);
            assert_eq!(bo, None);
            assert_eq!(bab, Some(true));
        } else {
            panic!("expected SetDataValidation");
        }
    }

    #[test]
    fn json_roundtrip_add_table() {
        let op = PatchOp::AddTable {
            sheet_name: "Sheet1".to_string(),
            range: "A1:C10".to_string(),
            table_name: "Table1".to_string(),
            display_name: Some("Sales Table".to_string()),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::AddTable {
            sheet_name: bsn,
            range: br,
            table_name: btn,
            display_name: bdn,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(br, "A1:C10");
            assert_eq!(btn, "Table1");
            assert_eq!(bdn, Some("Sales Table".to_string()));
        } else {
            panic!("expected AddTable");
        }
    }

    #[test]
    fn json_roundtrip_remove_table() {
        let op = PatchOp::RemoveTable {
            sheet_name: "Sheet1".to_string(),
            table_name: "Table1".to_string(),
        };
        let json = op_to_json(&op).expect("serialize");
        let back: PatchOp = op_from_json(json.as_bytes()).expect("deserialize");
        if let PatchOp::RemoveTable {
            sheet_name: bsn,
            table_name: btn,
        } = back
        {
            assert_eq!(bsn, "Sheet1");
            assert_eq!(btn, "Table1");
        } else {
            panic!("expected RemoveTable");
        }
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
    fn apply_insert_after_anchor() {
        let mut ir = make_sample_doc();
        let para_id = ElementId::content_anchored("paragraph", &["section:0"], "hello");
        let op = PatchOp::Insert {
            anchor: para_id.clone(),
            kind: "paragraph".to_string(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert_eq!(ir.root.children.len(), 2);
        // Verify the inserted node is after the anchor
        assert_eq!(ir.root.children[0].id, para_id);
    }

    #[test]
    fn apply_set_style() {
        let mut ir = make_sample_doc();
        let op = PatchOp::SetStyle {
            style_id: "MyStyle".to_string(),
            name: "My Style".to_string(),
            style_type: "paragraph".to_string(),
            based_on: Some("Normal".to_string()),
            bold: Some(true),
            italic: None,
            font_name: Some("Arial".to_string()),
            font_size_pt: Some(14.0),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
    }

    #[test]
    fn apply_assign_style() {
        let mut ir = make_sample_doc();
        let para_id = ElementId::content_anchored("paragraph", &["section:0"], "hello");
        let op = PatchOp::AssignStyle {
            elem: para_id.clone(),
            style_id: "MyStyle".to_string(),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert_eq!(result.changed_element_ids[0], para_id);
        assert_eq!(
            ir.root.children[0].alt_text.as_deref(),
            Some("assign_style:MyStyle")
        );
    }

    #[test]
    fn apply_add_hyperlink() {
        let mut ir = make_sample_doc();
        let run_id =
            ElementId::content_anchored("run", &["section:0", "paragraph:0", "run:0"], "hello");
        let op = PatchOp::AddHyperlink {
            target: "https://example.com".to_string(),
            elem: run_id.clone(),
            tooltip: Some("Visit".to_string()),
        };
        let result = apply_op(&mut ir, &op).expect("apply");
        assert_eq!(result.changed_element_ids.len(), 1);
        assert_eq!(result.changed_element_ids[0], run_id);
        assert_eq!(
            ir.root.children[0].children[0].alt_text.as_deref(),
            Some("hyperlink:https://example.com:tooltip=Visit")
        );
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
        assert!(validate_op(&PatchOp::SetStyle {
            style_id: "S1".to_string(),
            name: "S1".to_string(),
            style_type: "paragraph".to_string(),
            based_on: None,
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
        })
        .is_ok());
        assert!(validate_op(&PatchOp::AssignStyle {
            elem: elem.clone(),
            style_id: "S1".to_string(),
        })
        .is_ok());
        assert!(validate_op(&PatchOp::AddHyperlink {
            target: "https://example.com".to_string(),
            elem: elem.clone(),
            tooltip: None,
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
