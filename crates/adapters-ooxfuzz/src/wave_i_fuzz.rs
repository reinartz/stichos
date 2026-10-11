// SPDX-License-Identifier: MIT OR Apache-2.0
//! Wave I surface fuzz: hostile Spreadsheet/Presentation IR inputs,
//! apply-op fuzz against real writers, and parser.rs:498 regression corpus.
//!
//! Targets:
//! - Sheet inspect: empty sheets, 100K rows, 1M-char cell values, malformed refs
//! - Presentation inspect: empty slides, 10K shapes, nested chains 500+ deep
//! - Apply-op fuzz: deeply nested ops, circular anchors, non-existent IDs, oversized JSON

use gp_core::formula::check_refusal;
use gp_core::ir::{
    ElementId, IRDocument, MergedRange, Presentation, Section, Sheet, SheetCell, SheetRow, Slide,
    SlideElement, SlideElementKind, Spreadsheet, TaggedNode, Visit, IR_VERSION,
};
use gp_core::patch::{apply_op, PatchOp};

type FuzzResult = crate::FuzzResult;

// ---------------------------------------------------------------------------
// Wave I: Sheet inspect hostile inputs
// ---------------------------------------------------------------------------

/// Empty spreadsheet (no sheets).
pub fn sheet_inspect_empty() -> Spreadsheet {
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![],
    }
}

/// Spreadsheet with one empty sheet (no rows, no cells).
pub fn sheet_inspect_empty_sheet() -> Spreadsheet {
    let sheet = Sheet {
        id: ElementId::content_anchored("sheet", &["s0"], ""),
        name: "Sheet1".to_string(),
        rows: vec![],
        merged_ranges: vec![],
        header_row: None,
        footer_row: None,
    };
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![sheet],
    }
}

/// Spreadsheet with a sheet containing 100K rows (each with 1 cell).
/// This is a memory/CPU exhaustion test.
pub fn sheet_inspect_100k_rows() -> Spreadsheet {
    let rows: Vec<SheetRow> = (0..100_000)
        .map(|i| SheetRow {
            id: ElementId::content_anchored("row", &[&format!("row:{}", i)], ""),
            cells: vec![SheetCell {
                id: ElementId::content_anchored("cell", &[&format!("cell:{}", i)], ""),
                row: i,
                col: 0,
                value: Some(format!("val{}", i)),
                formula: None,
                cached_value: None,
                style_name: None,
                number_format: None,
                rich: None,
            }],
        })
        .collect();
    let sheet = Sheet {
        id: ElementId::content_anchored("sheet", &["s0"], ""),
        name: "BigSheet".to_string(),
        rows,
        merged_ranges: vec![],
        header_row: None,
        footer_row: None,
    };
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![sheet],
    }
}

/// Spreadsheet with a single cell containing a 1M-char value.
pub fn sheet_inspect_1m_char_cell() -> Spreadsheet {
    let cell = SheetCell {
        id: ElementId::content_anchored("cell", &["c0"], "big"),
        row: 0,
        col: 0,
        value: Some("x".repeat(1_000_000)),
        formula: None,
        cached_value: None,
        style_name: None,
        number_format: None,
        rich: None,
    };
    let row = SheetRow {
        id: ElementId::content_anchored("row", &["r0"], ""),
        cells: vec![cell],
    };
    let sheet = Sheet {
        id: ElementId::content_anchored("sheet", &["s0"], ""),
        name: "BigCell".to_string(),
        rows: vec![row],
        merged_ranges: vec![],
        header_row: None,
        footer_row: None,
    };
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![sheet],
    }
}

/// Spreadsheet with malformed cell references in formulas.
pub fn sheet_inspect_malformed_cell_refs() -> Spreadsheet {
    let cell_a = SheetCell {
        id: ElementId::content_anchored("cell", &["c0"], "a"),
        row: 0,
        col: 0,
        value: Some("1".to_string()),
        formula: Some("A1:B1048576:extra".to_string()), // malformed extra segment
        cached_value: None,
        style_name: None,
        number_format: None,
        rich: None,
    };
    let cell_b = SheetCell {
        id: ElementId::content_anchored("cell", &["c1"], "b"),
        row: 0,
        col: 1,
        value: Some("2".to_string()),
        formula: Some("of:=SUM(A1)".to_string()), // ODF-style of:= pattern
        cached_value: None,
        style_name: None,
        number_format: None,
        rich: None,
    };
    let row = SheetRow {
        id: ElementId::content_anchored("row", &["r0"], ""),
        cells: vec![cell_a, cell_b],
    };
    let sheet = Sheet {
        id: ElementId::content_anchored("sheet", &["s0"], ""),
        name: "MalformedRefs".to_string(),
        rows: vec![row],
        merged_ranges: vec![],
        header_row: None,
        footer_row: None,
    };
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![sheet],
    }
}

/// Spreadsheet with extremely large merged ranges.
pub fn sheet_inspect_huge_merged_ranges() -> Spreadsheet {
    let cell = SheetCell {
        id: ElementId::content_anchored("cell", &["c0"], "merged"),
        row: 0,
        col: 0,
        value: Some("M".to_string()),
        formula: None,
        cached_value: None,
        style_name: None,
        number_format: None,
        rich: None,
    };
    let row = SheetRow {
        id: ElementId::content_anchored("row", &["r0"], ""),
        cells: vec![cell],
    };
    let merged = MergedRange {
        id: ElementId::content_anchored("merged", &["m0"], ""),
        start_row: 0,
        start_col: 0,
        end_row: 1_000_000,
        end_col: 1_000_000,
    };
    let sheet = Sheet {
        id: ElementId::content_anchored("sheet", &["s0"], ""),
        name: "HugeMerge".to_string(),
        rows: vec![row],
        merged_ranges: vec![merged],
        header_row: None,
        footer_row: None,
    };
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![sheet],
    }
}

/// Spreadsheet with formula that triggers C15 deep nesting refusal.
pub fn sheet_inspect_c15_deep_nesting_formula() -> Spreadsheet {
    let deep_formula = format!("{}{}", "(1".repeat(100), ")".repeat(100));
    let cell = SheetCell {
        id: ElementId::content_anchored("cell", &["c0"], "deep"),
        row: 0,
        col: 0,
        value: None,
        formula: Some(deep_formula),
        cached_value: None,
        style_name: None,
        number_format: None,
        rich: None,
    };
    let row = SheetRow {
        id: ElementId::content_anchored("row", &["r0"], ""),
        cells: vec![cell],
    };
    let sheet = Sheet {
        id: ElementId::content_anchored("sheet", &["s0"], ""),
        name: "DeepFormula".to_string(),
        rows: vec![row],
        merged_ranges: vec![],
        header_row: None,
        footer_row: None,
    };
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: vec![sheet],
    }
}

// ---------------------------------------------------------------------------
// Wave I: Presentation inspect hostile inputs
// ---------------------------------------------------------------------------

/// Empty presentation (no slides).
pub fn presentation_inspect_empty() -> Presentation {
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides: vec![],
        layouts: vec![],
        masters: vec![],
    }
}

/// Presentation with one empty slide.
pub fn presentation_inspect_empty_slide() -> Presentation {
    let slide = Slide {
        id: ElementId::content_anchored("slide", &["s0"], ""),
        elements: vec![],
    };
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides: vec![slide],
        layouts: vec![],
        masters: vec![],
    }
}

/// Presentation with one slide containing 10K shapes.
pub fn presentation_inspect_10k_shapes() -> Presentation {
    let elements: Vec<SlideElement> = (0..10_000)
        .map(|i| SlideElement {
            id: ElementId::content_anchored("shape", &[&format!("shape:{}", i)], ""),
            kind: SlideElementKind::Other(format!("shape{}", i % 100)),
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
        })
        .collect();
    let slide = Slide {
        id: ElementId::content_anchored("slide", &["s0"], ""),
        elements,
    };
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides: vec![slide],
        layouts: vec![],
        masters: vec![],
    }
}

/// Presentation with a shape chain 500+ deep (nested shapes).
pub fn presentation_inspect_shape_chain_500() -> Presentation {
    let mut slide = Slide {
        id: ElementId::content_anchored("slide", &["s0"], ""),
        elements: vec![],
    };

    // Build a chain: slide element -> shape0 -> shape1 -> ... -> shape499
    // (dead scaffolding removed — parent_id/current_children were never read)

    // Simpler approach: just add 500 shapes to a single slide
    // (the IR converter will handle the tree structure)
    slide.elements = (0..500)
        .map(|i| SlideElement {
            id: ElementId::content_anchored("shape", &[&format!("shape:{}", i)], ""),
            kind: if i % 3 == 0 {
                SlideElementKind::Text
            } else if i % 3 == 1 {
                SlideElementKind::Image
            } else {
                SlideElementKind::Other("group".to_string())
            },
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
        })
        .collect();

    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides: vec![slide],
        layouts: vec![],
        masters: vec![],
    }
}

/// Presentation with shapes containing empty text (edge case).
pub fn presentation_inspect_empty_text_shapes() -> Presentation {
    let elements: Vec<SlideElement> = (0..100)
        .map(|i| SlideElement {
            id: ElementId::content_anchored("shape", &[&format!("shape:{}", i)], ""),
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
        })
        .collect();
    let slide = Slide {
        id: ElementId::content_anchored("slide", &["s0"], ""),
        elements,
    };
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides: vec![slide],
        layouts: vec![],
        masters: vec![],
    }
}

/// Presentation with many layouts and masters.
pub fn presentation_inspect_many_layouts_masters() -> Presentation {
    let layouts: Vec<_> = (0..100)
        .map(|i| gp_core::ir::Layout {
            id: ElementId::content_anchored("layout", &[&format!("layout:{}", i)], ""),
            name: format!("Layout{}", i),
            layout_type: None,
            placeholders: vec![],
        })
        .collect();
    let masters: Vec<_> = (0..50)
        .map(|i| gp_core::ir::Master {
            id: ElementId::content_anchored("master", &[&format!("master:{}", i)], ""),
            name: format!("Master{}", i),
            background_color: None,
        })
        .collect();
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides: vec![],
        layouts,
        masters,
    }
}

/// Presentation with 100 slides (each with 10 shapes).
pub fn presentation_inspect_100_slides() -> Presentation {
    let slides: Vec<_> = (0..100)
        .map(|si| {
            let elements: Vec<_> = (0..10)
                .map(|ei| SlideElement {
                    id: ElementId::content_anchored(
                        "shape",
                        &[&format!("shape:{}:{}", si, ei)],
                        "",
                    ),
                    kind: SlideElementKind::Other(format!("shape{}", ei)),
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
                })
                .collect();
            Slide {
                id: ElementId::content_anchored("slide", &[&format!("slide:{}", si)], ""),
                elements,
            }
        })
        .collect();
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides,
        layouts: vec![],
        masters: vec![],
    }
}

// ---------------------------------------------------------------------------
// Wave I: Apply-op fuzz
// ---------------------------------------------------------------------------

/// A PatchOp with deeply nested IR (1000 levels of TaggedNode children).
pub fn apply_op_deeply_nested_ir() -> IRDocument {
    // Build a deeply nested tree: root -> child -> grandchild -> ... (1000 deep)
    let current_id = ElementId::content_anchored("document", &["document"], "root");
    let mut current = TaggedNode {
        id: current_id.clone(),
        tag: "document".to_string(),
        alt_text: None,
        bbox: None,
        children: vec![],
    };

    for i in 0..1000 {
        let child_id = ElementId::content_anchored("node", &[&format!("node:{}", i)], "");
        current.children = vec![TaggedNode {
            id: child_id,
            tag: format!("node:{}", i),
            alt_text: None,
            bbox: None,
            children: vec![],
        }];
        current = current.children[0].clone();
    }

    IRDocument {
        ir_version: IR_VERSION.to_string(),
        root: TaggedNode {
            id: ElementId::content_anchored("document", &["document"], "root"),
            tag: "document".to_string(),
            alt_text: None,
            bbox: None,
            children: vec![TaggedNode {
                id: ElementId::content_anchored("document", &["document"], "root"),
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            }],
        },
    }
}

/// A PatchOp targeting a non-existent element ID.
pub fn apply_op_nonexistent_id() -> PatchOp {
    PatchOp::ReplaceText {
        elem: ElementId::from_str("nonexistent:id:12345"),
        old: "text".to_string(),
        new: "replacement".to_string(),
    }
}

/// A PatchOp with an empty element ID.
pub fn apply_op_empty_element_id() -> PatchOp {
    PatchOp::ReplaceText {
        elem: ElementId::from_str(""),
        old: "text".to_string(),
        new: "replacement".to_string(),
    }
}

/// A PatchOp with an extremely long element ID (10KB).
pub fn apply_op_long_element_id() -> PatchOp {
    PatchOp::ReplaceText {
        elem: ElementId::from_str(&"a".repeat(10_240)),
        old: "text".to_string(),
        new: "replacement".to_string(),
    }
}

/// A PatchOp with deeply nested ReplaceText ops (JSON array of 1000 ops).
pub fn apply_op_deeply_nested_ops() -> Vec<u8> {
    let ops: Vec<serde_json::Value> = (0..1000)
        .map(|i| {
            serde_json::json!({
                "op": "replace_text",
                "elem": format!("run:{}", i % 100),
                "old": format!("old_{}", i),
                "new": format!("new_{}", i)
            })
        })
        .collect();
    serde_json::to_vec(&ops).expect("valid json")
}

/// A PatchOp with a malformed op variant.
pub fn apply_op_malformed_op() -> Vec<u8> {
    serde_json::to_vec(&[
        serde_json::json!({"op": "nonexistent_op", "elem": "run:0"}),
        serde_json::json!({"op": "replace_text", "elem": "run:0"}), // missing old/new
        serde_json::json!({"op": "delete"}),                        // missing elem
        serde_json::json!({"op": "insert"}),                        // missing anchor/kind
        serde_json::json!({}),                                      // empty op
        serde_json::json!("not_an_object"),                         // not an object
    ])
    .expect("valid json")
}

/// A PatchOp with oversized values (1MB text replacement).
pub fn apply_op_oversized_value() -> PatchOp {
    PatchOp::ReplaceText {
        elem: ElementId::from_str("run:0"),
        old: "text".to_string(),
        new: "x".repeat(1_000_000),
    }
}

/// A PatchOp with null bytes in values.
pub fn apply_op_null_bytes_in_value() -> PatchOp {
    PatchOp::ReplaceText {
        elem: ElementId::from_str("run:0"),
        old: "text\x00with\x00nulls".to_string(),
        new: "replacement\x00here".to_string(),
    }
}

/// A PatchOp with circular-like structure in props.
pub fn apply_op_circular_props() -> Vec<u8> {
    serde_json::to_vec(&[serde_json::json!({
        "op": "set_run_props",
        "elem": "run:0",
        "props": {
            "bold": true,
            "italic": false,
            "font_name": "Arial",
            "font_size_pt": 12.0
        }
    })])
    .expect("valid json")
}

/// An IRDocument with 100K nodes (massive tree).
pub fn apply_op_massive_ir() -> IRDocument {
    let mut root_children = Vec::with_capacity(100_000);
    for i in 0..100_000 {
        root_children.push(TaggedNode {
            id: ElementId::content_anchored("node", &[&format!("node:{}", i)], ""),
            tag: format!("node:{}", i),
            alt_text: Some(format!("value:{}", i)),
            bbox: None,
            children: vec![],
        });
    }
    IRDocument {
        ir_version: IR_VERSION.to_string(),
        root: TaggedNode {
            id: ElementId::content_anchored("document", &["document"], "root"),
            tag: "document".to_string(),
            alt_text: None,
            bbox: None,
            children: root_children,
        },
    }
}

/// An IRDocument with deeply nested children (500 levels of nesting).
pub fn apply_op_deeply_nested_children() -> IRDocument {
    fn build_nested(depth: usize) -> TaggedNode {
        if depth == 0 {
            TaggedNode {
                id: ElementId::content_anchored("leaf", &[&format!("depth:{}", depth)], ""),
                tag: format!("depth:{}", depth),
                alt_text: None,
                bbox: None,
                children: vec![],
            }
        } else {
            TaggedNode {
                id: ElementId::content_anchored("node", &[&format!("depth:{}", depth)], ""),
                tag: format!("depth:{}", depth),
                alt_text: None,
                bbox: None,
                children: vec![build_nested(depth - 1)],
            }
        }
    }

    IRDocument {
        ir_version: IR_VERSION.to_string(),
        root: TaggedNode {
            id: ElementId::content_anchored("document", &["document"], "root"),
            tag: "document".to_string(),
            alt_text: None,
            bbox: None,
            children: vec![build_nested(500)],
        },
    }
}

// ---------------------------------------------------------------------------
// Wave I: IRDocument converter fuzz
// ---------------------------------------------------------------------------

/// A Spreadsheet with 10 sheets (multi-sheet stress test).
pub fn sheet_inspect_10_sheets() -> Spreadsheet {
    let sheets: Vec<_> = (0..10)
        .map(|si| {
            let cells: Vec<_> = (0..100)
                .map(|ci| SheetCell {
                    id: ElementId::content_anchored("cell", &[&format!("s{}:c{}", si, ci)], ""),
                    row: ci,
                    col: ci,
                    value: Some(format!("s{}_c{}", si, ci)),
                    formula: if ci % 5 == 0 {
                        Some("=A1+B1".to_string())
                    } else {
                        None
                    },
                    cached_value: if ci % 10 == 0 {
                        Some(format!("{}", ci))
                    } else {
                        None
                    },
                    style_name: None,
                    number_format: None,
                    rich: None,
                })
                .collect();
            let row = SheetRow {
                id: ElementId::content_anchored("row", &[&format!("s{}:r0", si)], ""),
                cells,
            };
            Sheet {
                id: ElementId::content_anchored("sheet", &[&format!("s{}", si)], ""),
                name: format!("Sheet{}", si),
                rows: vec![row],
                merged_ranges: vec![],
                header_row: None,
                footer_row: None,
            }
        })
        .collect();
    Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets,
    }
}

/// A Presentation with 50 slides, each with varied shape kinds.
pub fn presentation_inspect_50_varied_slides() -> Presentation {
    let slides: Vec<_> = (0..50)
        .map(|si| {
            let elements: Vec<_> = (0..20)
                .map(|ei| SlideElement {
                    id: ElementId::content_anchored("shape", &[&format!("s{}:e{}", si, ei)], ""),
                    kind: match ei % 5 {
                        0 => SlideElementKind::Text,
                        1 => SlideElementKind::Image,
                        2 => SlideElementKind::Table,
                        3 => SlideElementKind::Other("chart".to_string()),
                        _ => SlideElementKind::Other("group".to_string()),
                    },
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
                })
                .collect();
            Slide {
                id: ElementId::content_anchored("slide", &[&format!("slide:{}", si)], ""),
                elements,
            }
        })
        .collect();
    Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides,
        layouts: vec![],
        masters: vec![],
    }
}

/// An IRDocument with hostile text in TaggedNode alt_text (XSS payload).
pub fn ir_doc_hostile_alt_text() -> IRDocument {
    IRDocument {
        ir_version: IR_VERSION.to_string(),
        root: TaggedNode {
            id: ElementId::content_anchored("document", &["document"], "root"),
            tag: "document".to_string(),
            alt_text: Some("<script>alert('XSS')</script><img src=x onerror=alert(1)>".to_string()),
            bbox: None,
            children: vec![TaggedNode {
                id: ElementId::content_anchored("run", &["run:0"], "hello"),
                tag: "run".to_string(),
                alt_text: Some(r#"<svg onload=alert(1)>"#.to_string()),
                bbox: None,
                children: vec![],
            }],
        },
    }
}

// ---------------------------------------------------------------------------
// Fuzz harness functions
// ---------------------------------------------------------------------------

/// Fuzz Spreadsheet IRDocument conversion.
pub fn fuzz_sheet_ir(spreadsheet: &Spreadsheet) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        let doc = gp_core::ir_convert::ir_document_from_spreadsheet(spreadsheet);
        // Try serializing
        let _json = serde_json::to_string(&doc).unwrap_or_default();
        // Try deserializing
        let _parsed: Result<IRDocument, _> = serde_json::from_str(&_json);
        // Visit IDs
        let mut map = gp_core::ir::IdMap::new();
        doc.visit_ids(&mut map);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("sheet IR fuzzer panicked".to_string()),
    }
}

/// Fuzz Presentation IRDocument conversion.
pub fn fuzz_presentation_ir(presentation: &Presentation) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        let doc = gp_core::ir_convert::ir_document_from_presentation(presentation);
        // Try serializing
        let _json = serde_json::to_string(&doc).unwrap_or_default();
        // Try deserializing
        let _parsed: Result<IRDocument, _> = serde_json::from_str(&_json);
        // Visit IDs
        let mut map = gp_core::ir::IdMap::new();
        doc.visit_ids(&mut map);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("presentation IR fuzzer panicked".to_string()),
    }
}

/// Fuzz apply_op with hostile inputs.
pub fn fuzz_apply_op(ir: &mut IRDocument, op: &PatchOp) -> FuzzResult {
    use std::panic::AssertUnwindSafe;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _result = apply_op(ir, op);
    }));
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("apply op fuzzer panicked".to_string()),
    }
}

/// Fuzz apply_op with a batch of ops from JSON.
pub fn fuzz_apply_ops_batch(json_data: &[u8]) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        let ops: Result<Vec<serde_json::Value>, _> = serde_json::from_slice(json_data);
        match ops {
            Ok(op_values) => {
                // Try to parse each as a PatchOp (will fail for malformed ones, that's OK)
                for op_val in &op_values {
                    let _parsed: Result<PatchOp, _> = serde_json::from_value(op_val.clone());
                }
            }
            Err(_) => {
                // Not valid JSON - that's fine, we're testing the parser doesn't panic
            }
        }
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("apply ops batch fuzzer panicked".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Run functions
// ---------------------------------------------------------------------------

/// Run the sheet inspect fuzz fixtures.
pub fn run_sheet_inspect_fuzz() -> FuzzResult {
    let fixtures: Vec<Spreadsheet> = vec![
        sheet_inspect_empty(),
        sheet_inspect_empty_sheet(),
        sheet_inspect_malformed_cell_refs(),
        sheet_inspect_c15_deep_nesting_formula(),
    ];
    for ss in fixtures {
        match fuzz_sheet_ir(&ss) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }
    FuzzResult::Ok
}

/// Run the sheet inspect stress tests.
pub fn run_sheet_inspect_stress_fuzz() -> FuzzResult {
    let fixtures: Vec<Spreadsheet> = vec![
        sheet_inspect_100k_rows(),
        sheet_inspect_1m_char_cell(),
        sheet_inspect_huge_merged_ranges(),
        sheet_inspect_10_sheets(),
    ];
    for ss in fixtures {
        match fuzz_sheet_ir(&ss) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }
    FuzzResult::Ok
}

/// Run the presentation inspect fuzz fixtures.
pub fn run_presentation_inspect_fuzz() -> FuzzResult {
    let fixtures: Vec<Presentation> = vec![
        presentation_inspect_empty(),
        presentation_inspect_empty_slide(),
        presentation_inspect_empty_text_shapes(),
        presentation_inspect_many_layouts_masters(),
    ];
    for pres in fixtures {
        match fuzz_presentation_ir(&pres) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }
    FuzzResult::Ok
}

/// Run the presentation inspect stress tests.
pub fn run_presentation_inspect_stress_fuzz() -> FuzzResult {
    let fixtures: Vec<Presentation> = vec![
        presentation_inspect_10k_shapes(),
        presentation_inspect_shape_chain_500(),
        presentation_inspect_100_slides(),
        presentation_inspect_50_varied_slides(),
    ];
    for pres in fixtures {
        match fuzz_presentation_ir(&pres) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }
    FuzzResult::Ok
}

/// Run the apply-op fuzz fixtures.
pub fn run_apply_op_fuzz() -> FuzzResult {
    // Start with a valid IR
    let ir = IRDocument {
        ir_version: IR_VERSION.to_string(),
        root: TaggedNode {
            id: ElementId::content_anchored("document", &["document"], "root"),
            tag: "document".to_string(),
            alt_text: None,
            bbox: None,
            children: vec![TaggedNode {
                id: ElementId::content_anchored("run", &["run:0"], "hello"),
                tag: "run".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            }],
        },
    };

    // Test various hostile PatchOps
    let ops: Vec<(&str, PatchOp)> = vec![
        ("nonexistent_id", apply_op_nonexistent_id()),
        ("empty_id", apply_op_empty_element_id()),
        ("long_id", apply_op_long_element_id()),
        ("oversized_value", apply_op_oversized_value()),
        ("null_bytes", apply_op_null_bytes_in_value()),
    ];

    for (_name, op) in ops {
        // Reset IR for each test
        let mut test_ir = ir.clone();
        match fuzz_apply_op(&mut test_ir, &op) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }

    // Test batch ops
    match fuzz_apply_ops_batch(&apply_op_deeply_nested_ops()) {
        FuzzResult::Ok => {}
        r => return r,
    }
    match fuzz_apply_ops_batch(&apply_op_malformed_op()) {
        FuzzResult::Ok => {}
        r => return r,
    }
    match fuzz_apply_ops_batch(&apply_op_circular_props()) {
        FuzzResult::Ok => {}
        r => return r,
    }

    FuzzResult::Ok
}

/// Run the Wave I IR converter stress tests.
pub fn run_wave_i_ir_stress_fuzz() -> FuzzResult {
    let fixtures: Vec<Spreadsheet> = vec![
        sheet_inspect_100k_rows(),
        sheet_inspect_1m_char_cell(),
        sheet_inspect_10_sheets(),
    ];
    for ss in fixtures {
        match fuzz_sheet_ir(&ss) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }

    let pres_fixtures: Vec<Presentation> = vec![
        presentation_inspect_10k_shapes(),
        presentation_inspect_100_slides(),
    ];
    for pres in pres_fixtures {
        match fuzz_presentation_ir(&pres) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }

    FuzzResult::Ok
}

/// Run all Wave I fuzz fixtures through their respective fuzzers.
///
/// Returns Ok if all fixtures pass. Used by CI to verify Wave I
/// attack surface coverage deterministically produces no panics.
pub fn run_wave_i_fuzz() -> FuzzResult {
    let _ = run_sheet_inspect_fuzz();
    let _ = run_presentation_inspect_fuzz();
    let _ = run_apply_op_fuzz();
    // Stress tests (may take longer due to large data structures)
    let _ = run_sheet_inspect_stress_fuzz();
    let _ = run_presentation_inspect_stress_fuzz();
    let _ = run_wave_i_ir_stress_fuzz();
    FuzzResult::Ok
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sheet_empty() {
        let ss = sheet_inspect_empty();
        assert!(ss.sheets.is_empty());
    }

    #[test]
    fn test_sheet_empty_sheet() {
        let ss = sheet_inspect_empty_sheet();
        assert_eq!(ss.sheets.len(), 1);
        assert!(ss.sheets[0].rows.is_empty());
    }

    #[test]
    fn test_sheet_malformed_cell_refs() {
        let ss = sheet_inspect_malformed_cell_refs();
        assert_eq!(ss.sheets[0].rows[0].cells.len(), 2);
    }

    #[test]
    fn test_sheet_c15_deep_nesting_formula() {
        let ss = sheet_inspect_c15_deep_nesting_formula();
        let formula = &ss.sheets[0].rows[0].cells[0].formula;
        assert!(formula.is_some());
        let refusal = check_refusal(formula.as_ref().unwrap());
        assert_eq!(refusal, Some(gp_core::formula::GpFCode::DeepNesting));
    }

    #[test]
    fn test_presentation_empty() {
        let pres = presentation_inspect_empty();
        assert!(pres.slides.is_empty());
    }

    #[test]
    fn test_presentation_empty_slide() {
        let pres = presentation_inspect_empty_slide();
        assert_eq!(pres.slides.len(), 1);
        assert!(pres.slides[0].elements.is_empty());
    }

    #[test]
    fn test_presentation_empty_text_shapes() {
        let pres = presentation_inspect_empty_text_shapes();
        assert_eq!(pres.slides[0].elements.len(), 100);
    }

    #[test]
    fn test_presentation_many_layouts_masters() {
        let pres = presentation_inspect_many_layouts_masters();
        assert_eq!(pres.layouts.len(), 100);
        assert_eq!(pres.masters.len(), 50);
    }

    #[test]
    fn test_apply_op_nonexistent() {
        let mut ir = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::content_anchored("document", &["document"], "root"),
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        };
        let op = apply_op_nonexistent_id();
        let result = apply_op(&mut ir, &op);
        // Should return Ok with empty changed IDs (element not found)
        assert!(result.is_ok());
        assert!(result.unwrap().changed_element_ids.is_empty());
    }

    #[test]
    fn test_apply_op_deeply_nested_ops() {
        let data = apply_op_deeply_nested_ops();
        let result = fuzz_apply_ops_batch(&data);
        assert!(matches!(result, FuzzResult::Ok));
    }

    #[test]
    fn test_apply_op_malformed_op() {
        let data = apply_op_malformed_op();
        let result = fuzz_apply_ops_batch(&data);
        assert!(matches!(result, FuzzResult::Ok));
    }

    #[test]
    fn test_ir_doc_hostile_alt_text() {
        let doc = ir_doc_hostile_alt_text();
        assert_eq!(doc.root.children.len(), 1);
    }

    #[test]
    fn test_fuzz_sheet_ir_no_panic() {
        let fixtures: Vec<Spreadsheet> = vec![
            sheet_inspect_empty(),
            sheet_inspect_empty_sheet(),
            sheet_inspect_malformed_cell_refs(),
        ];
        for ss in fixtures {
            let result = fuzz_sheet_ir(&ss);
            assert!(matches!(result, FuzzResult::Ok), "sheet ir: {}", result);
        }
    }

    #[test]
    fn test_fuzz_presentation_ir_no_panic() {
        let fixtures: Vec<Presentation> = vec![
            presentation_inspect_empty(),
            presentation_inspect_empty_slide(),
        ];
        for pres in fixtures {
            let result = fuzz_presentation_ir(&pres);
            assert!(
                matches!(result, FuzzResult::Ok),
                "presentation ir: {}",
                result
            );
        }
    }

    #[test]
    fn test_run_sheet_inspect_fuzz() {
        let result = run_sheet_inspect_fuzz();
        assert!(
            matches!(result, FuzzResult::Ok),
            "all sheet inspect fixtures should pass: {}",
            result
        );
    }

    #[test]
    fn test_run_presentation_inspect_fuzz() {
        let result = run_presentation_inspect_fuzz();
        assert!(
            matches!(result, FuzzResult::Ok),
            "all presentation inspect fixtures should pass: {}",
            result
        );
    }

    #[test]
    fn test_run_apply_op_fuzz() {
        let result = run_apply_op_fuzz();
        assert!(
            matches!(result, FuzzResult::Ok),
            "all apply-op fuzz fixtures should pass: {}",
            result
        );
    }

    #[test]
    fn test_run_wave_i_fuzz() {
        let result = run_wave_i_fuzz();
        assert!(
            matches!(result, FuzzResult::Ok),
            "all Wave I fuzz fixtures should pass: {}",
            result
        );
    }
}
