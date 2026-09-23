// SPDX-License-Identifier: MIT OR Apache-2.0
//! Spreadsheet and Presentation → IRDocument tagged-tree converters.
//!
//! These functions produce a `TaggedNode` tree suitable for downstream
//! fidelity inspection (C2 — stable IDs) and diff analysis.

use crate::ir::*;

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
    }
}

fn build_spreadsheet_node(spreadsheet: &Spreadsheet) -> TaggedNode {
    let mut children = Vec::new();
    for (si, sheet) in spreadsheet.sheets.iter().enumerate() {
        let _si = si; // used for ID derivation
        let sheet_node = build_sheet_node(sheet, si);
        children.push(sheet_node);
    }
    TaggedNode {
        id: ElementId::content_anchored("spreadsheet", &["spreadsheet:0"], &spreadsheet.ir_version),
        tag: "spreadsheet".to_string(),
        alt_text: None,
        bbox: None,
        children,
    }
}

fn build_sheet_node(sheet: &Sheet, sheet_idx: usize) -> TaggedNode {
    let mut children = Vec::new();
    for (ri, row) in sheet.rows.iter().enumerate() {
        let row_node = build_row_node(row, ri);
        children.push(row_node);
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

    TaggedNode {
        id: ElementId::content_anchored(
            "presentation",
            &["presentation:0"],
            &presentation.ir_version,
        ),
        tag: "presentation".to_string(),
        alt_text: None,
        bbox: None,
        children,
    }
}

fn build_slide_node(slide: &Slide, slide_idx: usize) -> TaggedNode {
    let mut children = Vec::new();
    for el in &slide.elements {
        let shape_node = TaggedNode {
            id: ElementId::derive("shape", &[&format!("shape:{}", el.kind)], el),
            tag: format!("shape:{}", el.kind),
            alt_text: None,
            bbox: None,
            children: vec![],
        };
        children.push(shape_node);
    }
    let tag = format!("slide:{}", slide_idx);
    TaggedNode {
        id: ElementId::derive("slide", &[&tag], slide),
        tag,
        alt_text: None,
        bbox: None,
        children,
    }
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
        };
        let cell_b = SheetCell {
            id: ElementId::content_anchored("cell", &["b"], "v2"),
            row: 0,
            col: 1,
            value: Some("World".to_string()),
            formula: None,
            cached_value: None,
            style_name: None,
        };
        let cell_c = SheetCell {
            id: ElementId::content_anchored("cell", &["c"], "v3"),
            row: 0,
            col: 2,
            value: None,
            formula: Some("=A1+B1".to_string()),
            cached_value: None,
            style_name: None,
        };
        let row = SheetRow {
            id: ElementId::content_anchored("row", &["r0"], ""),
            cells: vec![cell_a, cell_b, cell_c],
        };
        let sheet = Sheet {
            id: ElementId::content_anchored("sheet", &["s0"], ""),
            name: "Sheet1".to_string(),
            rows: vec![row],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
        };

        let doc = ir_document_from_spreadsheet(&ss);
        // document → spreadsheet → sheet:Sheet1 → row:0 → cell:0,0,0 / cell:1,0,0 / cell:2,0,0
        let spreadsheet_node = &doc.root.children[0];
        assert_eq!(spreadsheet_node.tag, "spreadsheet");
        assert_eq!(spreadsheet_node.children.len(), 1);

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
        };
        assert_eq!(cell_display_value(&cell4), None);
    }

    /// Presentation with one slide, two shapes.
    #[test]
    fn test_presentation_with_slide_shapes() {
        let el1 = SlideElement {
            id: ElementId::content_anchored("shape", &["shape:text"], ""),
            kind: "text".to_string(),
        };
        let el2 = SlideElement {
            id: ElementId::content_anchored("shape", &["shape:image"], ""),
            kind: "image".to_string(),
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

        // No layouts or masters, only slide
        assert_eq!(pres_node.children.len(), 1);
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
        };
        let master = Master {
            id: ElementId::content_anchored("master", &["m0"], ""),
            name: "Office Theme".to_string(),
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
        };
        let master = Master {
            id: ElementId::content_anchored("master", &["m0"], ""),
            name: "Default".to_string(),
        };
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![SlideElement {
                id: ElementId::content_anchored("shape", &["s0"], ""),
                kind: "title".to_string(),
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
        assert_eq!(pres_node.children.len(), 3);
        assert_eq!(pres_node.children[0].tag, "layout:Title");
        assert_eq!(pres_node.children[1].tag, "master:Default");
        assert_eq!(pres_node.children[2].tag, "slide:0");
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
        };
        let row = SheetRow {
            id: ElementId::content_anchored("row", &["r0"], ""),
            cells: vec![cell],
        };
        let sheet = Sheet {
            id: ElementId::content_anchored("sheet", &["s0"], ""),
            name: "Data".to_string(),
            rows: vec![row],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
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
                    kind: "text".to_string(),
                },
                SlideElement {
                    id: ElementId::content_anchored("shape", &["i"], ""),
                    kind: "image".to_string(),
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
                }],
            }],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
        };
        let sheet2 = Sheet {
            id: ElementId::content_anchored("sheet", &["s1"], ""),
            name: "Sheet2".to_string(),
            rows: vec![],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet1, sheet2],
        };

        let doc = ir_document_from_spreadsheet(&ss);
        let pres = &doc.root.children[0];
        assert_eq!(pres.children.len(), 2);
        assert_eq!(pres.children[0].tag, "sheet:Sheet1");
        assert_eq!(pres.children[1].tag, "sheet:Sheet2");
    }

    /// Multiple slides, multiple shapes.
    #[test]
    fn test_presentation_multiple_slides() {
        let slide1 = Slide {
            id: ElementId::content_anchored("slide", &["s0"], ""),
            elements: vec![
                SlideElement {
                    id: ElementId::content_anchored("shape", &["t1"], ""),
                    kind: "title".to_string(),
                },
                SlideElement {
                    id: ElementId::content_anchored("shape", &["t2"], ""),
                    kind: "body".to_string(),
                },
            ],
        };
        let slide2 = Slide {
            id: ElementId::content_anchored("slide", &["s1"], ""),
            elements: vec![SlideElement {
                id: ElementId::content_anchored("shape", &["img1"], ""),
                kind: "image".to_string(),
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
        assert_eq!(pres_node.children.len(), 2);
        assert_eq!(pres_node.children[0].tag, "slide:0");
        assert_eq!(pres_node.children[1].tag, "slide:1");

        let s0 = &pres_node.children[0];
        assert_eq!(s0.children.len(), 2);
        assert_eq!(s0.children[0].tag, "shape:title");
        assert_eq!(s0.children[1].tag, "shape:body");

        let s1 = &pres_node.children[1];
        assert_eq!(s1.children.len(), 1);
        assert_eq!(s1.children[0].tag, "shape:image");
    }
}
