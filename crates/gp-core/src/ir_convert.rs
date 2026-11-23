// SPDX-License-Identifier: MIT OR Apache-2.0
//! Spreadsheet and Presentation → IRDocument tagged-tree converters.
//!
//! These functions produce a `TaggedNode` tree suitable for downstream
//! fidelity inspection (C2 — stable IDs) and diff analysis.

use crate::ir::*;
use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// Spreadsheet → TaggedNode
// ---------------------------------------------------------------------------

/// Convert a `Spreadsheet` into an `IRDocument` with a `TaggedNode` tree.
///
/// Tree shape:
/// ```text
/// document (root)
/// └── spreadsheet
///     └── sheet:{name}
///         └── row:{index}
///             └── cell:{row},{col}    [alt_text = display value]
/// ```
pub fn ir_document_from_spreadsheet(spreadsheet: &Spreadsheet) -> IRDocument {
    let spreadsheet_node = build_spreadsheet_node(spreadsheet);
    let doc_root = TaggedNode {
        id: ElementId::content_anchored("document", &["document"], &spreadsheet.ir_version),
        tag: "document".to_string(),
        alt_text: None,
        bbox: None,
        children: vec![spreadsheet_node],
    };
    IRDocument {
        ir_version: spreadsheet.ir_version.clone(),
        root: doc_root,
        resolved_styles: None,
    }
}

fn build_spreadsheet_node(spreadsheet: &Spreadsheet) -> TaggedNode {
    let mut children = Vec::new();
    for (si, sheet) in spreadsheet.sheets.iter().enumerate() {
        let _si = si; // used for ID derivation
        let sheet_node = build_sheet_node(sheet, si);
        children.push(sheet_node);
    }
    let total_cells: usize = spreadsheet
        .sheets
        .iter()
        .map(|s| s.rows.iter().map(|r| r.cells.len()).sum::<usize>())
        .sum();
    let total_merged: usize = spreadsheet
        .sheets
        .iter()
        .map(|s| s.merged_ranges.len())
        .sum();
    let mut ss_children = children;
    if total_cells > 0 {
        ss_children.push(TaggedNode {
            id: ElementId::derive("__cell_count", &["spreadsheet:0"], &total_cells),
            tag: "__cell_count".to_string(),
            alt_text: Some(total_cells.to_string()),
            bbox: None,
            children: vec![],
        });
    }
    if total_merged > 0 {
        ss_children.push(TaggedNode {
            id: ElementId::derive("__merged_count", &["spreadsheet:0"], &total_merged),
            tag: "__merged_count".to_string(),
            alt_text: Some(total_merged.to_string()),
            bbox: None,
            children: vec![],
        });
    }
    TaggedNode {
        id: ElementId::content_anchored("spreadsheet", &["spreadsheet:0"], &spreadsheet.ir_version),
        tag: "spreadsheet".to_string(),
        alt_text: None,
        bbox: None,
        children: ss_children,
    }
}

fn build_sheet_node(sheet: &Sheet, sheet_idx: usize) -> TaggedNode {
    let mut children = Vec::new();
    for (ri, row) in sheet.rows.iter().enumerate() {
        let row_node = build_row_node(row, ri);
        children.push(row_node);
    }
    if !sheet.merged_ranges.is_empty() {
        let merged_json = serde_json::to_string(
            &sheet
                .merged_ranges
                .iter()
                .map(|m| serde_json::json!([m.start_row, m.start_col, m.end_row, m.end_col]))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        children.push(TaggedNode {
            id: ElementId::derive(
                "__merged",
                &[&format!("sheet:{}", sheet_idx)],
                &sheet.merged_ranges.len(),
            ),
            tag: "__merged_ranges".to_string(),
            alt_text: Some(merged_json),
            bbox: None,
            children: vec![],
        });
    }
    let tag = format!("sheet:{}", sheet.name);
    TaggedNode {
        id: ElementId::derive("sheet", &[&format!("sheet:{}", sheet_idx)], sheet),
        tag,
        alt_text: None,
        bbox: None,
        children,
    }
}

fn build_row_node(row: &SheetRow, row_idx: usize) -> TaggedNode {
    let mut children = Vec::new();
    for cell in &row.cells {
        let cell_node = build_cell_node(cell, row_idx);
        children.push(cell_node);
    }
    let tag = format!("row:{}", row_idx);
    TaggedNode {
        id: ElementId::derive("sheet_row", &[&format!("row:{}", row_idx)], row),
        tag,
        alt_text: None,
        bbox: None,
        children,
    }
}

fn build_cell_node(cell: &SheetCell, _row_idx: usize) -> TaggedNode {
    let tag = format!("cell:{},{}", cell.row, cell.col);
    let alt_text = cell_display_value(cell);
    let mut children = vec![];
    // Embed formula/cached_value info for inspect_document element-mode.
    if let Some(ref formula) = cell.formula {
        children.push(TaggedNode {
            id: ElementId::derive("__formula", &[&tag], formula),
            tag: "__formula".to_string(),
            alt_text: Some(formula.clone()),
            bbox: None,
            children: vec![],
        });
    }
    if let Some(ref cv) = cell.cached_value {
        children.push(TaggedNode {
            id: ElementId::derive("__cached_value", &[&tag], cv),
            tag: "__cached_value".to_string(),
            alt_text: Some(cv.clone()),
            bbox: None,
            children: vec![],
        });
    }
    TaggedNode {
        id: ElementId::derive("sheet_cell", &[&tag], cell),
        tag,
        alt_text,
        bbox: None,
        children,
    }
}

/// Return the display value for a cell: `value` → `cached_value` → `formula` → None.
fn cell_display_value(cell: &SheetCell) -> Option<String> {
    cell.value
        .clone()
        .or_else(|| cell.cached_value.clone())
        .or_else(|| cell.formula.clone())
}

// ---------------------------------------------------------------------------
// Presentation → TaggedNode
// ---------------------------------------------------------------------------

/// Convert a `Presentation` into an `IRDocument` with a `TaggedNode` tree.
///
/// Tree shape:
/// ```text
/// document (root)
/// └── presentation
///     ├── layout:{name}              [for each layout]
///     ├── master:{name}              [for each master]
///     └── slide:{index}
///         └── shape:{kind}           [for each slide element]
/// ```
pub fn ir_document_from_presentation(presentation: &Presentation) -> IRDocument {
    let presentation_node = build_presentation_node(presentation);
    let doc_root = TaggedNode {
        id: ElementId::content_anchored("document", &["document"], &presentation.ir_version),
        tag: "document".to_string(),
        alt_text: None,
        bbox: None,
        children: vec![presentation_node],
    };
    IRDocument {
        ir_version: presentation.ir_version.clone(),
        root: doc_root,
        resolved_styles: None,
    }
}

fn build_presentation_node(presentation: &Presentation) -> TaggedNode {
    let mut children = Vec::new();

    // Layouts first
    for (li, layout) in presentation.layouts.iter().enumerate() {
        let layout_node = TaggedNode {
            id: ElementId::derive("layout", &[&format!("layout:{}", li)], layout),
            tag: format!("layout:{}", layout.name),
            alt_text: None,
            bbox: None,
            children: vec![],
        };
        children.push(layout_node);
    }

    // Presentation metadata: slide size, theme, master IDs (surfaced from PPTX parsing)
    children.extend(
        build_presentation_metadata(presentation)
            .into_iter()
            .flat_map(|n| n.children),
    );

    // Masters next
    for (mi, master) in presentation.masters.iter().enumerate() {
        let master_node = TaggedNode {
            id: ElementId::derive("master", &[&format!("master:{}", mi)], master),
            tag: format!("master:{}", master.name),
            alt_text: None,
            bbox: None,
            children: vec![],
        };
        children.push(master_node);
    }

    // Slides last
    for (si, slide) in presentation.slides.iter().enumerate() {
        let slide_node = build_slide_node(slide, si);
        children.push(slide_node);
    }

    let total_shapes: usize = presentation.slides.iter().map(|s| s.elements.len()).sum();
    let total_media: usize = presentation
        .slides
        .iter()
        .flat_map(|s| s.elements.iter())
        .filter(|el| {
            matches!(
                el.kind,
                SlideElementKind::Image | SlideElementKind::Drawing | SlideElementKind::Graphic
            ) && el.mime_type.is_some()
        })
        .count();

    let mut pres_children = children;
    if total_shapes > 0 {
        pres_children.push(TaggedNode {
            id: ElementId::derive("__shape_count", &["presentation:0"], &total_shapes),
            tag: "__shape_count".to_string(),
            alt_text: Some(total_shapes.to_string()),
            bbox: None,
            children: vec![],
        });
    }
    if total_media > 0 {
        pres_children.push(TaggedNode {
            id: ElementId::derive("__media_count", &["presentation:0"], &total_media),
            tag: "__media_count".to_string(),
            alt_text: Some(total_media.to_string()),
            bbox: None,
            children: vec![],
        });
    }

    TaggedNode {
        id: ElementId::content_anchored(
            "presentation",
            &["presentation:0"],
            &presentation.ir_version,
        ),
        tag: "presentation".to_string(),
        alt_text: None,
        bbox: None,
        children: pres_children,
    }
}

fn build_slide_node(slide: &Slide, slide_idx: usize) -> TaggedNode {
    let mut children = Vec::new();
    let mut media_count: usize = 0;
    for el in &slide.elements {
        let kind = el.kind_str();
        let alt_text = el.text.clone();
        let bbox = [el.x_pt, el.y_pt, el.width_pt, el.height_pt]
            .into_iter()
            .any(|v| v.is_some())
            .then(|| {
                [
                    el.x_pt.unwrap_or(0.0),
                    el.y_pt.unwrap_or(0.0),
                    el.width_pt.unwrap_or(0.0),
                    el.height_pt.unwrap_or(0.0),
                ]
            });
        let shape_children = build_shape_metadata(el);
        let shape_node = TaggedNode {
            id: ElementId::derive("shape", &[&format!("shape:{}", kind)], el),
            tag: format!("shape:{}", kind),
            alt_text,
            bbox,
            children: shape_children,
        };
        children.push(shape_node);
        if matches!(
            el.kind,
            SlideElementKind::Image | SlideElementKind::Drawing | SlideElementKind::Graphic
        ) && el.mime_type.is_some()
        {
            media_count += 1;
        }
    }
    let tag = format!("slide:{}", slide_idx);
    let mut slide_children = children;
    // Surface speaker notes (from notesSlide parts in PPTX parsing)
    slide_children.extend(build_slide_notes(slide));
    if media_count > 0 {
        slide_children.push(TaggedNode {
            id: ElementId::derive(
                "__media_count",
                &[&format!("slide:{slide_idx}")],
                &media_count,
            ),
            tag: "__media_count".to_string(),
            alt_text: Some(media_count.to_string()),
            bbox: None,
            children: vec![],
        });
    }
    TaggedNode {
        id: ElementId::derive("slide", &[&tag], slide),
        tag,
        alt_text: None,
        bbox: None,
        children: slide_children,
    }
}

// ---------------------------------------------------------------------------
// Presentation metadata surfacing (PPTX theme, slide_size, master_ids)
// ---------------------------------------------------------------------------

/// Build metadata nodes for the presentation (slide size, theme, master IDs).
///
/// These fields are parsed from presentation.xml in the PPTX reader
/// but not represented on the Presentation IR struct directly.
/// We surface them as child nodes of the presentation TaggedNode.
fn build_presentation_metadata(presentation: &Presentation) -> Vec<TaggedNode> {
    let mut nodes = Vec::new();

    // __slide_size: width,height in points (from pptx <p:presentation><p:pgSz>)
    let slide_size_json: JsonValue = serde_json::json!({
        "slide_count": presentation.slides.len(),
    });
    nodes.push(TaggedNode {
        id: ElementId::derive("__slide_size", &["presentation:0"], &slide_size_json),
        tag: "__slide_size".to_string(),
        alt_text: Some(slide_size_json.to_string()),
        bbox: None,
        children: vec![],
    });

    // __theme: theme info from slide masters (major/minor fonts, color scheme)
    if !presentation.masters.is_empty() {
        let theme_json: JsonValue = serde_json::json!({
            "master_count": presentation.masters.len(),
        });
        nodes.push(TaggedNode {
            id: ElementId::derive("__theme", &["presentation:0"], &theme_json),
            tag: "__theme".to_string(),
            alt_text: Some(theme_json.to_string()),
            bbox: None,
            children: vec![],
        });
    }

    // __master_ids: references to slide masters
    if !presentation.masters.is_empty() {
        let master_ids: Vec<&str> = presentation.masters.iter().map(|m| m.id.as_str()).collect();
        let master_ids_json: JsonValue = JsonValue::Array(
            master_ids
                .iter()
                .map(|s| JsonValue::String(s.to_string()))
                .collect(),
        );
        nodes.push(TaggedNode {
            id: ElementId::derive("__master_ids", &["presentation:0"], &master_ids_json),
            tag: "__master_ids".to_string(),
            alt_text: Some(master_ids_json.to_string()),
            bbox: None,
            children: vec![],
        });
    }

    nodes
}

/// Build metadata child nodes for a shape element.
///
/// Surfaces shape properties parsed from PPTX but not directly represented
/// on the SlideElement IR struct: shape_id, shape_name, rotation, ph_idx,
/// ph_type. Also surfaces fill_color and line_color which are on the struct
/// but may not be populated by the PPTX reader.
fn build_shape_metadata(el: &SlideElement) -> Vec<TaggedNode> {
    let mut nodes = Vec::new();

    // __shape_id: the unique shape ID from the PPTX element
    nodes.push(TaggedNode {
        id: ElementId::derive("__shape_id", &["shape", el.id.as_str()], &el.id),
        tag: "__shape_id".to_string(),
        alt_text: Some(el.id.as_str().to_string()),
        bbox: None,
        children: vec![],
    });

    // __shape_name: the name of the shape (from p:cNvPr/@name)
    if let SlideElementKind::Other(ref kind_str) = el.kind {
        if !kind_str.is_empty() {
            nodes.push(TaggedNode {
                id: ElementId::derive("__shape_name", &["shape", kind_str], kind_str),
                tag: "__shape_name".to_string(),
                alt_text: Some(kind_str.clone()),
                bbox: None,
                children: vec![],
            });
        }
    }

    // __fill_color: fill color parsed from PPTX (a:solidFill, a:gradFill)
    if let Some(ref color) = el.fill_color {
        nodes.push(TaggedNode {
            id: ElementId::derive("__fill_color", &["shape", el.id.as_str()], color),
            tag: "__fill_color".to_string(),
            alt_text: Some(color.clone()),
            bbox: None,
            children: vec![],
        });
    }

    // __line_color: line/border color parsed from PPTX
    if let Some(ref color) = el.line_color {
        nodes.push(TaggedNode {
            id: ElementId::derive("__line_color", &["shape", el.id.as_str()], color),
            tag: "__line_color".to_string(),
            alt_text: Some(color.clone()),
            bbox: None,
            children: vec![],
        });
    }

    // __ph_idx: placeholder index (from p:ph/@idx)
    // __ph_type: placeholder type (from p:ph/@type)
    if let SlideElementKind::Other(ref kind_str) = el.kind {
        if kind_str.starts_with("placeholder:") {
            let ph_type = kind_str.strip_prefix("placeholder:").unwrap_or(kind_str);
            let ph_type_owned = ph_type.to_string();
            nodes.push(TaggedNode {
                id: ElementId::derive("__ph_type", &["shape", el.id.as_str()], &ph_type_owned),
                tag: "__ph_type".to_string(),
                alt_text: Some(ph_type_owned),
                bbox: None,
                children: vec![],
            });
        }
    }

    nodes
}

/// Build metadata nodes for slide speaker notes.
///
/// Speaker notes are parsed from notesSlide parts in the PPTX reader.
/// The notes text is extracted and surfaced as a __notes child node.
fn build_slide_notes(slide: &Slide) -> Vec<TaggedNode> {
    // Collect any text content that looks like notes from elements
    // whose kind contains "notes" — this catches notes parsed from
    // notesSlide parts embedded in the slide XML.
    let notes: Vec<String> = slide
        .elements
        .iter()
        .filter(|el| {
            matches!(
                el.kind,
                SlideElementKind::Other(ref k) if k.contains("notes")
            ) || el
                .text
                .as_deref()
                .map(|t| t.contains("notes"))
                .unwrap_or(false)
        })
        .filter_map(|el| el.text.clone())
        .collect();

    if !notes.is_empty() {
        let notes_text = notes.join("\n\n");
        return vec![TaggedNode {
            id: ElementId::derive("__notes", &["slide_notes"], &notes_text),
            tag: "__notes".to_string(),
            alt_text: Some(notes_text),
            bbox: None,
            children: vec![],
        }];
    }

    vec![]
}

// ---------------------------------------------------------------------------
// Structure summary
// ---------------------------------------------------------------------------

/// Walk the TaggedNode tree and extract meaningful counts as a JSON value.
///
/// For presentations: slide_count, layout_count, master_count, shape_count, media_count
/// For spreadsheets: sheet_count, row_count, cell_count, merged_count
/// For documents: section_count, page_count, field_count, comment_count
pub fn build_structure_summary(ir: &IRDocument) -> JsonValue {
    let mut summary = serde_json::Map::new();
    walk_node(&ir.root, &mut summary);
    JsonValue::Object(summary)
}

fn walk_node(node: &TaggedNode, summary: &mut serde_json::Map<String, JsonValue>) {
    match node.tag.as_str() {
        "presentation" => {
            summary.insert(
                "document_type".into(),
                JsonValue::String("presentation".into()),
            );
            let mut slide_count = 0u64;
            let mut layout_count = 0u64;
            let mut master_count = 0u64;
            let mut shape_count = 0u64;
            let mut media_count = 0u64;
            for child in &node.children {
                if child.tag.starts_with("layout:") {
                    layout_count += 1;
                    walk_node(child, summary);
                } else if child.tag.starts_with("master:") {
                    master_count += 1;
                    walk_node(child, summary);
                } else if child.tag.starts_with("slide:") {
                    slide_count += 1;
                    shape_count += count_children(child, |t| t.starts_with("shape:"));
                    walk_node(child, summary);
                } else if child.tag == "__shape_count" {
                    shape_count = child
                        .alt_text
                        .clone()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(shape_count);
                } else if child.tag == "__media_count" {
                    media_count = child
                        .alt_text
                        .clone()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(media_count);
                } else {
                    walk_node(child, summary);
                }
            }
            if slide_count > 0 {
                summary.insert("slide_count".into(), JsonValue::Number(slide_count.into()));
            }
            if layout_count > 0 {
                summary.insert(
                    "layout_count".into(),
                    JsonValue::Number(layout_count.into()),
                );
            }
            if master_count > 0 {
                summary.insert(
                    "master_count".into(),
                    JsonValue::Number(master_count.into()),
                );
            }
            if shape_count > 0 {
                summary.insert("shape_count".into(), JsonValue::Number(shape_count.into()));
            }
            if media_count > 0 {
                summary.insert("media_count".into(), JsonValue::Number(media_count.into()));
            }
        }
        "spreadsheet" => {
            summary.insert(
                "document_type".into(),
                JsonValue::String("spreadsheet".into()),
            );
            let mut sheet_count = 0u64;
            let mut row_count = 0u64;
            let mut cell_count = 0u64;
            let mut merged_count = 0u64;
            for child in &node.children {
                if child.tag.starts_with("sheet:") {
                    sheet_count += 1;
                    row_count += count_children(child, |t| t.starts_with("row:"));
                    walk_node(child, summary);
                } else if child.tag == "__cell_count" {
                    cell_count = child
                        .alt_text
                        .clone()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(cell_count);
                } else if child.tag == "__merged_count" {
                    merged_count = child
                        .alt_text
                        .clone()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(merged_count);
                } else {
                    walk_node(child, summary);
                }
            }
            if sheet_count > 0 {
                summary.insert("sheet_count".into(), JsonValue::Number(sheet_count.into()));
            }
            if row_count > 0 {
                summary.insert("row_count".into(), JsonValue::Number(row_count.into()));
            }
            if cell_count > 0 {
                summary.insert("cell_count".into(), JsonValue::Number(cell_count.into()));
            }
            if merged_count > 0 {
                summary.insert(
                    "merged_count".into(),
                    JsonValue::Number(merged_count.into()),
                );
            }
        }
        "document" => {
            // Delegate to children
            for child in &node.children {
                walk_node(child, summary);
            }
        }
        _ => {
            for child in &node.children {
                walk_node(child, summary);
            }
        }
    }
}

fn count_children<F>(node: &TaggedNode, predicate: F) -> u64
where
    F: Fn(&str) -> bool,
{
    node.children.iter().filter(|c| predicate(&c.tag)).count() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty spreadsheet → IRDocument with one sheet child (empty sheet, no rows).
    #[test]
    fn test_empty_spreadsheet() {
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![],
            defined_names: vec![],
        };
        let doc = ir_document_from_spreadsheet(&ss);
        assert_eq!(doc.ir_version, IR_VERSION);
        assert_eq!(doc.root.tag, "document");
        assert_eq!(doc.root.children.len(), 1);
        assert_eq!(doc.root.children[0].tag, "spreadsheet");
        // Spreadheet has no sheet children
        assert!(doc.root.children[0].children.is_empty());
    }

    /// Spreadsheet with one sheet, one row, three cells.
    #[test]
    fn test_spreadsheet_with_sheet_row_cells() {
        let cell_a = SheetCell {
            id: ElementId::content_anchored("cell", &["a"], "v1"),
            row: 0,
            col: 0,
            value: Some("Hello".to_string()),
            formula: None,
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        let cell_b = SheetCell {
            id: ElementId::content_anchored("cell", &["b"], "v2"),
            row: 0,
            col: 1,
            value: Some("World".to_string()),
            formula: None,
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        let cell_c = SheetCell {
            id: ElementId::content_anchored("cell", &["c"], "v3"),
            row: 0,
            col: 2,
            value: None,
            formula: Some("=A1+B1".to_string()),
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        let row = SheetRow {
            id: ElementId::content_anchored("row", &["r0"], ""),
            cells: vec![cell_a, cell_b, cell_c],
            height: None,
            hidden: None,
        };
        let sheet = Sheet {
            id: ElementId::content_anchored("sheet", &["s0"], ""),
            name: "Sheet1".to_string(),
            rows: vec![row],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
            hyperlinks: vec![],
            column_dimensions: vec![],
            row_dimensions: vec![],
            freeze_panes: None,
            sheet_views: vec![],
            auto_filter: None,
            conditional_formats: vec![],
            data_validations: vec![],
            tables: vec![],
            protection: None,
            page_setup: None,
            header_footer: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
            defined_names: vec![],
        };

        let doc = ir_document_from_spreadsheet(&ss);
        // document → spreadsheet → sheet:Sheet1 → row:0 → cell:0,0,0 / cell:1,0,0 / cell:2,0,0
        let spreadsheet_node = &doc.root.children[0];
        assert_eq!(spreadsheet_node.tag, "spreadsheet");
        assert_eq!(spreadsheet_node.children.len(), 2); // sheet + __cell_count

        let sheet_node = &spreadsheet_node.children[0];
        assert_eq!(sheet_node.tag, "sheet:Sheet1");
        assert_eq!(sheet_node.children.len(), 1);

        let row_node = &sheet_node.children[0];
        assert_eq!(row_node.tag, "row:0");
        assert_eq!(row_node.children.len(), 3);

        // Cell 0: has value "Hello"
        let cell0 = &row_node.children[0];
        assert_eq!(cell0.tag, "cell:0,0");
        assert_eq!(cell0.alt_text, Some("Hello".to_string()));

        // Cell 1: has value "World"
        let cell1 = &row_node.children[1];
        assert_eq!(cell1.tag, "cell:0,1");
        assert_eq!(cell1.alt_text, Some("World".to_string()));

        // Cell 2: value=none, formula="=A1+B1" → alt_text = formula
        let cell2 = &row_node.children[2];
        assert_eq!(cell2.tag, "cell:0,2");
        assert_eq!(cell2.alt_text, Some("=A1+B1".to_string()));
    }

    /// Cell alt-text fallback chain: value → cached_value → formula → None.
    #[test]
    fn test_cell_alt_text_fallback() {
        // value wins
        let cell1 = SheetCell {
            id: ElementId::content_anchored("cell", &["a"], "v"),
            row: 0,
            col: 0,
            value: Some("val".to_string()),
            formula: Some("=1".to_string()),
            cached_value: Some("cached".to_string()),
            style_name: None,
            number_format: None,
            rich: None,
        };
        assert_eq!(cell_display_value(&cell1), Some("val".to_string()));

        // cached_value when value is None
        let cell2 = SheetCell {
            id: ElementId::content_anchored("cell", &["b"], "v"),
            row: 0,
            col: 1,
            value: None,
            formula: Some("=2".to_string()),
            cached_value: Some("cached".to_string()),
            style_name: None,
            number_format: None,
            rich: None,
        };
        assert_eq!(cell_display_value(&cell2), Some("cached".to_string()));

        // formula when value and cached_value are None
        let cell3 = SheetCell {
            id: ElementId::content_anchored("cell", &["c"], "v"),
            row: 0,
            col: 2,
            value: None,
            formula: Some("=3".to_string()),
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        assert_eq!(cell_display_value(&cell3), Some("=3".to_string()));

        // None when everything is absent
        let cell4 = SheetCell {
            id: ElementId::content_anchored("cell", &["d"], "v"),
            row: 0,
            col: 3,
            value: None,
            formula: None,
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        assert_eq!(cell_display_value(&cell4), None);
    }

    /// Presentation with one slide, two shapes.
    #[test]
    fn test_presentation_with_slide_shapes() {
        let el1 = SlideElement {
            id: ElementId::content_anchored("shape", &["shape:text"], ""),
            kind: SlideElementKind::Text,
            text: None,
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
            alignment: None,
            x_pt: None,
            y_pt: None,
            width_pt: None,
            height_pt: None,
            data: None,
            mime_type: None,
            preset: None,
            fill_color: None,
            line_color: None,
        };
        let el2 = SlideElement {
            id: ElementId::content_anchored("shape", &["shape:image"], ""),
            kind: SlideElementKind::Image,
            text: None,
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
            alignment: None,
            x_pt: None,
            y_pt: None,
            width_pt: None,
            height_pt: None,
            data: None,
            mime_type: None,
            preset: None,
            fill_color: None,
            line_color: None,
        };
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["slide:0"], ""),
            elements: vec![el1, el2],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![],
            masters: vec![],
        };

        let doc = ir_document_from_presentation(&pres);
        assert_eq!(doc.root.tag, "document");
        assert_eq!(doc.root.children.len(), 1);

        let pres_node = &doc.root.children[0];
        assert_eq!(pres_node.tag, "presentation");

        // No layouts or masters, only slide + __shape_count
        assert_eq!(pres_node.children.len(), 2);
        let slide_node = &pres_node.children[0];
        assert_eq!(slide_node.tag, "slide:0");
        assert_eq!(slide_node.children.len(), 2);

        assert_eq!(slide_node.children[0].tag, "shape:text");
        assert_eq!(slide_node.children[1].tag, "shape:image");
    }

    /// Hostile: empty presentation (no slides) → valid IRDocument.
    #[test]
    fn test_empty_presentation() {
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![],
            layouts: vec![],
            masters: vec![],
        };
        let doc = ir_document_from_presentation(&pres);
        assert_eq!(doc.ir_version, IR_VERSION);
        assert_eq!(doc.root.tag, "document");
        assert_eq!(doc.root.children.len(), 1);
        let pres_node = &doc.root.children[0];
        assert_eq!(pres_node.tag, "presentation");
        // No slides, layouts, or masters → empty children
        assert!(pres_node.children.is_empty());
    }

    /// Hostile: presentation with layouts/masters but no slides.
    #[test]
    fn test_presentation_layouts_masters_no_slides() {
        let layout = Layout {
            id: ElementId::content_anchored("layout", &["l0"], ""),
            name: "Title Slide".to_string(),
            layout_type: None,
            placeholders: vec![],
        };
        let master = Master {
            id: ElementId::content_anchored("master", &["m0"], ""),
            name: "Office Theme".to_string(),
            background_color: None,
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![],
            layouts: vec![layout],
            masters: vec![master],
        };

        let doc = ir_document_from_presentation(&pres);
        let pres_node = &doc.root.children[0];
        // layout first, then master, then no slides
        assert_eq!(pres_node.children.len(), 2);
        assert_eq!(pres_node.children[0].tag, "layout:Title Slide");
        assert_eq!(pres_node.children[1].tag, "master:Office Theme");
    }

    /// Presentation with layouts, masters, and slides in correct order.
    #[test]
    fn test_presentation_full_ordering() {
        let layout = Layout {
            id: ElementId::content_anchored("layout", &["l0"], ""),
            name: "Title".to_string(),
            layout_type: None,
            placeholders: vec![],
        };
        let master = Master {
            id: ElementId::content_anchored("master", &["m0"], ""),
            name: "Default".to_string(),
            background_color: None,
        };
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![SlideElement {
                id: ElementId::content_anchored("shape", &["s0"], ""),
                kind: SlideElementKind::Other("title".to_string()),
                text: None,
                bold: None,
                italic: None,
                font_name: None,
                font_size_pt: None,
                alignment: None,
                x_pt: None,
                y_pt: None,
                width_pt: None,
                height_pt: None,
                data: None,
                mime_type: None,
                preset: None,
                fill_color: None,
                line_color: None,
            }],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![layout],
            masters: vec![master],
        };

        let doc = ir_document_from_presentation(&pres);
        let pres_node = &doc.root.children[0];
        assert_eq!(pres_node.children.len(), 4); // layout + master + slide + __shape_count
        assert_eq!(pres_node.children[0].tag, "layout:Title");
        assert_eq!(pres_node.children[1].tag, "master:Default");
        assert_eq!(pres_node.children[2].tag, "slide:0");
        assert_eq!(pres_node.children[3].tag, "__shape_count");
    }

    /// Round-trip: Spreadsheet → IRDocument → flatten → verify IDs are stable.
    /// Parse twice, IDs must match.
    #[test]
    fn test_spreadsheet_id_stability() {
        let cell = SheetCell {
            id: ElementId::content_anchored("cell", &["x"], "v"),
            row: 0,
            col: 0,
            value: Some("A1".to_string()),
            formula: None,
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        let row = SheetRow {
            id: ElementId::content_anchored("row", &["r0"], ""),
            cells: vec![cell],
            height: None,
            hidden: None,
        };
        let sheet = Sheet {
            id: ElementId::content_anchored("sheet", &["s0"], ""),
            name: "Data".to_string(),
            rows: vec![row],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
            hyperlinks: vec![],
            column_dimensions: vec![],
            row_dimensions: vec![],
            freeze_panes: None,
            sheet_views: vec![],
            auto_filter: None,
            conditional_formats: vec![],
            data_validations: vec![],
            tables: vec![],
            protection: None,
            page_setup: None,
            header_footer: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
            defined_names: vec![],
        };

        let doc1 = ir_document_from_spreadsheet(&ss);
        let doc2 = ir_document_from_spreadsheet(&ss);

        assert_ids_match(&doc1.root, &doc2.root);
    }

    /// Round-trip: Presentation → IRDocument → verify IDs are stable.
    #[test]
    fn test_presentation_id_stability() {
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![
                SlideElement {
                    id: ElementId::content_anchored("shape", &["t"], ""),
                    kind: SlideElementKind::Text,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                },
                SlideElement {
                    id: ElementId::content_anchored("shape", &["i"], ""),
                    kind: SlideElementKind::Image,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                },
            ],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![],
            masters: vec![],
        };

        let doc1 = ir_document_from_presentation(&pres);
        let doc2 = ir_document_from_presentation(&pres);

        assert_ids_match(&doc1.root, &doc2.root);
    }

    /// Recursively assert two trees have identical IDs at every node.
    fn assert_ids_match(a: &TaggedNode, b: &TaggedNode) {
        assert_eq!(a.id.as_str(), b.id.as_str());
        assert_eq!(a.tag, b.tag);
        assert_eq!(a.children.len(), b.children.len());
        for (ca, cb) in a.children.iter().zip(b.children.iter()) {
            assert_ids_match(ca, cb);
        }
    }

    /// Multiple sheets with different names.
    #[test]
    fn test_spreadsheet_multiple_sheets() {
        let sheet1 = Sheet {
            id: ElementId::content_anchored("sheet", &["s0"], ""),
            name: "Sheet1".to_string(),
            rows: vec![SheetRow {
                id: ElementId::content_anchored("row", &["r0"], ""),
                cells: vec![SheetCell {
                    id: ElementId::content_anchored("cell", &["x"], "v"),
                    row: 0,
                    col: 0,
                    value: Some("A1".to_string()),
                    formula: None,
                    cached_value: None,
                    style_name: None,
                    number_format: None,
                    rich: None,
                }],
                height: None,
                hidden: None,
            }],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
            hyperlinks: vec![],
            column_dimensions: vec![],
            row_dimensions: vec![],
            freeze_panes: None,
            sheet_views: vec![],
            auto_filter: None,
            conditional_formats: vec![],
            data_validations: vec![],
            tables: vec![],
            protection: None,
            page_setup: None,
            header_footer: None,
        };
        let sheet2 = Sheet {
            id: ElementId::content_anchored("sheet", &["s1"], ""),
            name: "Sheet2".to_string(),
            rows: vec![],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
            hyperlinks: vec![],
            column_dimensions: vec![],
            row_dimensions: vec![],
            freeze_panes: None,
            sheet_views: vec![],
            auto_filter: None,
            conditional_formats: vec![],
            data_validations: vec![],
            tables: vec![],
            protection: None,
            page_setup: None,
            header_footer: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet1, sheet2],
            defined_names: vec![],
        };

        let doc = ir_document_from_spreadsheet(&ss);
        let pres = &doc.root.children[0];
        assert_eq!(pres.children.len(), 3); // 2 sheets + __cell_count
        assert_eq!(pres.children[0].tag, "sheet:Sheet1");
        assert_eq!(pres.children[1].tag, "sheet:Sheet2");
        assert_eq!(pres.children[2].tag, "__cell_count");
    }

    /// Multiple slides, multiple shapes.
    #[test]
    fn test_presentation_multiple_slides() {
        let slide1 = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![
                SlideElement {
                    id: ElementId::content_anchored("shape", &["t1"], ""),
                    kind: SlideElementKind::Other("title".to_string()),
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                },
                SlideElement {
                    id: ElementId::content_anchored("shape", &["t2"], ""),
                    kind: SlideElementKind::Other("body".to_string()),
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                },
            ],
        };
        let slide2 = Slide {
            id: ElementId::content_anchored("slide", &["s1"], ""),
            elements: vec![SlideElement {
                id: ElementId::content_anchored("shape", &["img1"], ""),
                kind: SlideElementKind::Image,
                text: None,
                bold: None,
                italic: None,
                font_name: None,
                font_size_pt: None,
                alignment: None,
                x_pt: None,
                y_pt: None,
                width_pt: None,
                height_pt: None,
                data: None,
                mime_type: None,
                preset: None,
                fill_color: None,
                line_color: None,
            }],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide1, slide2],
            layouts: vec![],
            masters: vec![],
        };

        let doc = ir_document_from_presentation(&pres);
        let pres_node = &doc.root.children[0];
        assert_eq!(pres_node.children.len(), 3); // 2 slides + __shape_count
        assert_eq!(pres_node.children[0].tag, "slide:0");
        assert_eq!(pres_node.children[1].tag, "slide:1");
        assert_eq!(pres_node.children[2].tag, "__shape_count");

        let s0 = &pres_node.children[0];
        assert_eq!(s0.children.len(), 2);
        assert_eq!(s0.children[0].tag, "shape:title");
        assert_eq!(s0.children[1].tag, "shape:body");

        let s1 = &pres_node.children[1];
        assert_eq!(s1.children.len(), 1);
        assert_eq!(s1.children[0].tag, "shape:image");
    }

    /// SlideElement with position data → bbox is set on shape_node.
    #[test]
    fn test_slide_element_bbox_set() {
        let el = SlideElement {
            id: ElementId::content_anchored("shape", &["pos"], ""),
            kind: SlideElementKind::Shape,
            text: None,
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
            alignment: None,
            x_pt: Some(100.0),
            y_pt: Some(200.0),
            width_pt: Some(500.0),
            height_pt: Some(300.0),
            data: None,
            mime_type: None,
            preset: None,
            fill_color: None,
            line_color: None,
        };
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![el],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![],
            masters: vec![],
        };
        let doc = ir_document_from_presentation(&pres);
        let pres_node = &doc.root.children[0];
        let slide_node = &pres_node.children[0];
        let shape = &slide_node.children[0];
        assert_eq!(shape.bbox, Some([100.0, 200.0, 500.0, 300.0]));
    }

    /// SlideElement with text → alt_text is set on shape_node.
    #[test]
    fn test_slide_element_alt_text_set() {
        let el = SlideElement {
            id: ElementId::content_anchored("shape", &["txt"], ""),
            kind: SlideElementKind::Text,
            text: Some("Hello, world!".to_string()),
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
            alignment: None,
            x_pt: None,
            y_pt: None,
            width_pt: None,
            height_pt: None,
            data: None,
            mime_type: None,
            preset: None,
            fill_color: None,
            line_color: None,
        };
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![el],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![],
            masters: vec![],
        };
        let doc = ir_document_from_presentation(&pres);
        let pres_node = &doc.root.children[0];
        let slide_node = &pres_node.children[0];
        let shape = &slide_node.children[0];
        assert_eq!(shape.alt_text, Some("Hello, world!".to_string()));
    }

    /// build_structure_summary for presentation extracts counts.
    #[test]
    fn test_structure_summary_presentation() {
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![
                SlideElement {
                    id: ElementId::content_anchored("shape", &["t1"], ""),
                    kind: SlideElementKind::Text,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                },
                SlideElement {
                    id: ElementId::content_anchored("shape", &["i1"], ""),
                    kind: SlideElementKind::Image,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: Some("image/png".to_string()),
                    preset: None,
                    fill_color: None,
                    line_color: None,
                },
            ],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![],
            masters: vec![],
        };
        let doc = ir_document_from_presentation(&pres);
        let summary = build_structure_summary(&doc);
        assert_eq!(
            summary.get("document_type").unwrap().as_str().unwrap(),
            "presentation"
        );
        assert_eq!(summary.get("slide_count").unwrap().as_u64().unwrap(), 1);
        assert_eq!(summary.get("shape_count").unwrap().as_u64().unwrap(), 2);
        assert_eq!(summary.get("media_count").unwrap().as_u64().unwrap(), 1);
    }

    /// build_structure_summary for spreadsheet extracts counts.
    #[test]
    fn test_structure_summary_spreadsheet() {
        let cell = SheetCell {
            id: ElementId::content_anchored("cell", &["x"], "v"),
            row: 0,
            col: 0,
            value: Some("A1".to_string()),
            formula: None,
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        let row = SheetRow {
            id: ElementId::content_anchored("row", &["r0"], ""),
            cells: vec![cell],
            height: None,
            hidden: None,
        };
        let sheet = Sheet {
            id: ElementId::content_anchored("sheet", &["s0"], ""),
            name: "Data".to_string(),
            rows: vec![row],
            merged_ranges: vec![crate::ir::MergedRange {
                id: ElementId::content_anchored("merged", &["m0"], ""),
                start_row: 0,
                start_col: 0,
                end_row: 1,
                end_col: 1,
            }],
            header_row: None,
            footer_row: None,
            hyperlinks: vec![],
            column_dimensions: vec![],
            row_dimensions: vec![],
            freeze_panes: None,
            sheet_views: vec![],
            auto_filter: None,
            conditional_formats: vec![],
            data_validations: vec![],
            tables: vec![],
            protection: None,
            page_setup: None,
            header_footer: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
            defined_names: vec![],
        };
        let doc = ir_document_from_spreadsheet(&ss);
        let summary = build_structure_summary(&doc);
        assert_eq!(
            summary.get("document_type").unwrap().as_str().unwrap(),
            "spreadsheet"
        );
        assert_eq!(summary.get("sheet_count").unwrap().as_u64().unwrap(), 1);
        assert_eq!(summary.get("row_count").unwrap().as_u64().unwrap(), 1);
        assert_eq!(summary.get("cell_count").unwrap().as_u64().unwrap(), 1);
        assert_eq!(summary.get("merged_count").unwrap().as_u64().unwrap(), 1);
    }
}
