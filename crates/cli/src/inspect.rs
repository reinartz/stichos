// SPDX-License-Identifier: MIT OR Apache-2.0
//! inspect_document tool: read a document and return structure + IDs + coordinates + fidelity budget.
//!
//! This module implements the `inspect_document` CLI command, which reads a document file
//! (docx, odt, pdf) and returns a JSON summary containing:
//! - Document structure (sections, pages, etc.)
//! - Element IDs with kind/path/id triples (C2 contract)
//! - Coordinate manifest (bounding boxes for PDF)
//! - Fidelity budget (C4 contract)

use gp_core::fidelity::FidelityBudget;
use gp_core::file_access::read_document_bytes;
use gp_core::ir::*;
use gp_core::ir_convert;
use gp_core::traits::Codec;
use serde_json::json;
use std::collections::BTreeMap;

/// Inspect a document and return its IR, element IDs, coordinate manifest, and fidelity budget.
pub fn inspect_document(path: &str) -> Result<serde_json::Value, anyhow::Error> {
    let bytes = read_document_bytes(path)
        .map_err(|e| anyhow::anyhow!("failed to read document: {}: {}", path, e))?;

    let ir = parse_document(&bytes, path)?;

    let result = build_inspect_result(&ir, path);
    Ok(result)
}

/// Parse a document from bytes based on file extension.
fn parse_document(bytes: &[u8], path: &str) -> Result<IRDocument, anyhow::Error> {
    let ext = path.to_lowercase();

    if ext.ends_with(".docx") {
        let reader = adapters_ooxml::docx::DocxReader::new();
        let result = reader
            .read(bytes)
            .map_err(|diags| anyhow::anyhow!("docx parse failed: {:?}", diags))?;
        let doc: Document = serde_json::from_slice(&result.ir)
            .map_err(|e| anyhow::anyhow!("failed to deserialize docx IR: {}", e))?;
        Ok(ir_document_from_document(&doc))
    } else if ext.ends_with(".xlsx") {
        let package = adapters_ooxml::opc::parse_opc(bytes)
            .map_err(|diags| anyhow::anyhow!("xlsx OPC parse failed: {:?}", diags))?;
        let result = adapters_ooxml::xlsx::read_xlsx(&package)
            .map_err(|diags| anyhow::anyhow!("xlsx parse failed: {:?}", diags))?;
        let ss: Spreadsheet = serde_json::from_slice(&result.ir)
            .map_err(|e| anyhow::anyhow!("failed to deserialize xlsx IR: {}", e))?;
        Ok(ir_convert::ir_document_from_spreadsheet(&ss))
    } else if ext.ends_with(".pptx") {
        let package = adapters_ooxml::opc::parse_opc(bytes)
            .map_err(|diags| anyhow::anyhow!("pptx OPC parse failed: {:?}", diags))?;
        let result = adapters_ooxml::pptx::read_pptx(&package)
            .map_err(|diags| anyhow::anyhow!("pptx parse failed: {:?}", diags))?;
        let pres: Presentation = serde_json::from_slice(&result.ir)
            .map_err(|e| anyhow::anyhow!("failed to deserialize pptx IR: {}", e))?;
        Ok(ir_convert::ir_document_from_presentation(&pres))
    } else if ext.ends_with(".odt") {
        let reader = adapters_odf::odt::OdtReader;
        let result = reader
            .read(bytes)
            .map_err(|diags| anyhow::anyhow!("odt parse failed: {:?}", diags))?;
        let doc: Document = serde_json::from_slice(&result.ir)
            .map_err(|e| anyhow::anyhow!("failed to deserialize odt IR: {}", e))?;
        Ok(ir_document_from_document(&doc))
    } else if ext.ends_with(".ods") {
        let package = adapters_odf::manifest::parse_odf(bytes)
            .map_err(|diags| anyhow::anyhow!("ods package parse failed: {:?}", diags))?;
        let result = adapters_odf::ods::read_ods(&package)
            .map_err(|diags| anyhow::anyhow!("ods parse failed: {:?}", diags))?;
        let ss: Spreadsheet = serde_json::from_slice(&result.ir)
            .map_err(|e| anyhow::anyhow!("failed to deserialize ods IR: {}", e))?;
        Ok(ir_convert::ir_document_from_spreadsheet(&ss))
    } else if ext.ends_with(".odp") {
        let package = adapters_odf::manifest::parse_odf(bytes)
            .map_err(|diags| anyhow::anyhow!("odp package parse failed: {:?}", diags))?;
        let result = adapters_odf::odp::read_odp(&package)
            .map_err(|diags| anyhow::anyhow!("odp parse failed: {:?}", diags))?;
        let pres: Presentation = serde_json::from_slice(&result.ir)
            .map_err(|e| anyhow::anyhow!("failed to deserialize odp IR: {}", e))?;
        Ok(ir_convert::ir_document_from_presentation(&pres))
    } else if ext.ends_with(".pdf") {
        let reader = adapters_pdf::parser::PdfReader::new();
        let result = reader
            .read(bytes)
            .map_err(|diags| anyhow::anyhow!("pdf parse failed: {:?}", diags))?;
        Ok(result)
    } else {
        anyhow::bail!("unsupported format: {}", path);
    }
}

/// Build the inspect result JSON with structure, IDs, coordinates, and fidelity budget.
fn build_inspect_result(ir: &IRDocument, path: &str) -> serde_json::Value {
    // Collect element IDs
    let mut ids: Vec<serde_json::Value> = Vec::new();
    collect_ids(ir, "", &mut ids);

    // Build compact representation: element kind counts + first 5 samples
    let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &ids {
        if let Some(kind) = entry.get("kind").and_then(|k| k.as_str()) {
            *kind_counts.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    let sample_len = ids.len().min(5);
    let element_ids_sample: Vec<serde_json::Value> = ids[..sample_len].to_vec();

    // Build coordinate manifest
    let coordinates = build_coordinate_manifest(ir);

    // Build fidelity budget
    let budget = build_fidelity_budget(ir);

    let result = json!({
        "tool": "inspect_document",
        "path": path,
        "ir_version": IR_VERSION,
        "document_type": document_type_name(ir),
        "structure": build_structure_summary(ir),
        "element_kinds": kind_counts,
        "element_ids_sample": element_ids_sample,
        "total_elements": ids.len(),
        "coordinates": coordinates,
        "fidelity_budget": budget,
    });

    result
}

/// Create an IRDocument wrapper from a Document.
fn ir_document_from_document(doc: &Document) -> IRDocument {
    let root = build_tagged_root_from_document(doc);
    IRDocument {
        ir_version: doc.ir_version.clone(),
        root,
    }
}

/// Collect element IDs from an IRDocument into the result vector.
///
/// Single tagged-tree walk: every element (document, section, paragraph, run,
/// table, cell, drawing, list, ...) gets a `{kind, path, id}` entry exactly
/// once. Previous versions re-walked the same tree via
/// `collect_ids_from_tagged_tree`/`collect_item_from_tagged_node`, roughly
/// doubling `element_ids` (and `id_map` added a third full copy). That extra
/// payload is what pushed `inspect_document` tool results past LLM context
/// windows (see `budget_enforcer::enforce_inspect_budget` for the cap).
fn collect_ids(ir: &IRDocument, prefix: &str, ids: &mut Vec<serde_json::Value>) {
    collect_tagged_node(&ir.root, prefix, ids);
}

/// Recursively collect IDs from a TaggedNode tree.
fn collect_tagged_node(node: &TaggedNode, prefix: &str, ids: &mut Vec<serde_json::Value>) {
    let node_path = if prefix.is_empty() || prefix == "/" {
        format!("/{}", node.tag)
    } else {
        format!("{}/{}", prefix.trim_end_matches('/'), node.tag)
    };
    ids.push(json!({
        "kind": &node.tag,
        "path": &node_path,
        "id": node.id.as_str(),
    }));
    for child in &node.children {
        collect_tagged_node(child, &node_path, ids);
    }
}

/// Build coordinate manifest from PDF pages.
fn build_coordinate_manifest(ir: &IRDocument) -> Vec<serde_json::Value> {
    // Look for a PDF document by checking the tagged tree for page nodes
    let mut result: Vec<serde_json::Value> = Vec::new();

    for node in &ir.root.children {
        if node.tag == "page" {
            let bboxes: Vec<serde_json::Value> = node
                .children
                .iter()
                .take(100) // cap for JSON size
                .map(|b| {
                    json!({
                        "id": b.id.as_str(),
                        "text": b.alt_text.as_deref().unwrap_or(""),
                        "bbox_pt": b.bbox,
                        "type": "text",
                    })
                })
                .collect();

            // Try to parse page dimensions from alt_text
            let alt = node.alt_text.as_deref().unwrap_or("");
            let width_pt = alt
                .split('x')
                .next()
                .and_then(|s| s.split_whitespace().last())
                .and_then(|s| {
                    s.trim_end_matches('p')
                        .trim_end_matches('t')
                        .parse::<f64>()
                        .ok()
                })
                .unwrap_or(0.0);
            let height_pt = alt
                .split('x')
                .nth(1)
                .and_then(|s| s.split('p').next())
                .and_then(|s| s.trim_end_matches('t').parse::<f64>().ok())
                .unwrap_or(0.0);

            // Prefer explicit page bbox from the tagged node when present
            let (width_pt, height_pt) = match node.bbox {
                Some([_, _, w, h]) if w > 0.0 && h > 0.0 => (w, h),
                _ => (width_pt, height_pt),
            };

            let page_num: usize = node
                .id
                .as_str()
                .split(':')
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            result.push(json!({
                "page": page_num,
                "width_pt": width_pt,
                "height_pt": height_pt,
                "bbox_count": node.children.len(),
                "bboxes": bboxes,
            }));
        }
    }

    if !result.is_empty() {
        return result;
    }

    // For document types: build section-level coordinate info
    let mut doc_result: Vec<serde_json::Value> = Vec::new();
    for child in &ir.root.children {
        if child.tag.starts_with("section:") {
            // Collect drawing coordinates inside this section (drawings carry bbox_pt)
            let drawn: Vec<serde_json::Value> = collect_drawing_coords(child)
                .into_iter()
                .take(100)
                .collect();
            doc_result.push(json!({
                "section": child.id.as_str(),
                "kind": child.tag.replace("section:", ""),
                "drawing_bboxes": drawn,
            }));
        }
    }
    doc_result
}

/// Collect drawing bounding boxes from a tagged section subtree.
fn collect_drawing_coords(node: &TaggedNode) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if node.tag == "drawing" {
        if let Some(bbox_pt) = node.bbox {
            out.push(json!({
                "id": node.id.as_str(),
                "bbox_pt": bbox_pt,
            }));
        }
        return out;
    }
    for child in &node.children {
        out.extend(collect_drawing_coords(child));
    }
    out
}

/// Build a fidelity budget for the document.
fn build_fidelity_budget(_ir: &IRDocument) -> serde_json::Value {
    let budget = FidelityBudget::default();
    serde_json::to_value(&budget).unwrap_or_default()
}

/// Get the document type name for the IR.
fn document_type_name(ir: &IRDocument) -> &'static str {
    if ir.root.children.iter().any(|c| c.tag == "page") {
        return "pdf";
    }
    if ir.root.children.iter().any(|c| c.tag == "spreadsheet") {
        return "spreadsheet";
    }
    if ir.root.children.iter().any(|c| c.tag == "presentation") {
        return "presentation";
    }
    if ir
        .root
        .children
        .iter()
        .any(|c| c.tag.starts_with("section:"))
    {
        return "document";
    }
    "unknown"
}

/// Build a structure summary for the IR.
fn build_structure_summary(ir: &IRDocument) -> serde_json::Value {
    if ir.root.children.iter().any(|c| c.tag == "page") {
        let page_count = ir.root.children.iter().filter(|c| c.tag == "page").count();
        return json!({ "pages": page_count });
    }
    if ir.root.children.iter().any(|c| c.tag == "spreadsheet") {
        let sheet_count = ir.root.children[0].children.len();
        return json!({ "sheet_count": sheet_count });
    }
    if ir.root.children.iter().any(|c| c.tag == "presentation") {
        let slide_count = ir.root.children[0]
            .children
            .iter()
            .filter(|c| c.tag.starts_with("slide:"))
            .count();
        let layout_count = ir.root.children[0]
            .children
            .iter()
            .filter(|c| c.tag.starts_with("layout:"))
            .count();
        return json!({ "slide_count": slide_count, "layout_count": layout_count });
    }
    let section_count = ir
        .root
        .children
        .iter()
        .filter(|c| c.tag.starts_with("section:"))
        .count();
    json!({ "sections": section_count })
}

/// Build a tagged tree root from a Document.
fn build_tagged_root_from_document(doc: &Document) -> TaggedNode {
    let children: Vec<TaggedNode> = doc
        .sections
        .iter()
        .map(|section| {
            let section_children: Vec<TaggedNode> =
                section.items.iter().map(section_item_to_tagged).collect();
            TaggedNode {
                id: section.id.clone(),
                tag: format!("section:{}", section_kind_name(&section.kind)),
                alt_text: None,
                bbox: None,
                children: section_children,
            }
        })
        .collect();

    TaggedNode {
        id: ElementId::derive("document", &["document"], &doc.ir_version),
        tag: "document".to_string(),
        alt_text: None,
        bbox: None,
        children,
    }
}

/// Convert a SectionItem to a TaggedNode.
fn section_item_to_tagged(item: &SectionItem) -> TaggedNode {
    match item {
        SectionItem::Paragraph(p) => {
            let children: Vec<TaggedNode> = p
                .runs
                .iter()
                .map(|run| TaggedNode {
                    id: run.id.clone(),
                    tag: "run".to_string(),
                    alt_text: Some(run.text.clone()),
                    bbox: None,
                    children: vec![],
                })
                .collect();
            TaggedNode {
                id: p.id.clone(),
                tag: "paragraph".to_string(),
                alt_text: None,
                bbox: None,
                children,
            }
        }
        SectionItem::Table(t) => {
            let children: Vec<TaggedNode> = t
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .map(|cell| TaggedNode {
                    id: cell.id.clone(),
                    tag: "cell".to_string(),
                    alt_text: None,
                    bbox: None,
                    children: vec![],
                })
                .collect();
            TaggedNode {
                id: t.id.clone(),
                tag: "table".to_string(),
                alt_text: None,
                bbox: None,
                children,
            }
        }
        SectionItem::Drawing(d) => TaggedNode {
            id: d.id.clone(),
            tag: "drawing".to_string(),
            alt_text: Some(format!("{}x{}", d.mime_type, d.data.len())),
            bbox: d.bbox_pt,
            children: vec![],
        },
        SectionItem::List(l) => {
            let children: Vec<TaggedNode> = l
                .items
                .iter()
                .map(|list_item| TaggedNode {
                    id: list_item.id.clone(),
                    tag: "list_item".to_string(),
                    alt_text: None,
                    bbox: None,
                    children: vec![],
                })
                .collect();
            TaggedNode {
                id: l.id.clone(),
                tag: "list".to_string(),
                alt_text: None,
                bbox: None,
                children,
            }
        }
    }
}

/// Get the name of a section kind.
fn section_kind_name(kind: &SectionKind) -> &'static str {
    match kind {
        SectionKind::Body => "body",
        SectionKind::Header => "header",
        SectionKind::Footer => "footer",
        SectionKind::Footnotes => "footnotes",
        SectionKind::Endnotes => "endnotes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_section_kind_name — verify all SectionKind variants.
    #[test]
    fn test_section_kind_name() {
        assert_eq!(section_kind_name(&SectionKind::Body), "body");
        assert_eq!(section_kind_name(&SectionKind::Header), "header");
        assert_eq!(section_kind_name(&SectionKind::Footer), "footer");
        assert_eq!(section_kind_name(&SectionKind::Footnotes), "footnotes");
        assert_eq!(section_kind_name(&SectionKind::Endnotes), "endnotes");
    }

    /// test_build_tagged_root_from_document — verify tagged tree structure.
    #[test]
    fn test_build_tagged_root_from_document() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![
                Section {
                    id: ElementId::derive("section", &["section:0"], &"body"),
                    kind: SectionKind::Body,
                    items: vec![SectionItem::Paragraph(Paragraph {
                        id: ElementId::derive("paragraph", &["para:0"], &"hello"),
                        runs: vec![Run {
                            id: ElementId::derive("run", &["run:0"], &"hello"),
                            text: "hello".into(),
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
                    })],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                },
                Section {
                    id: ElementId::derive("section", &["section:1"], &"footer"),
                    kind: SectionKind::Footer,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                },
            ],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let root = build_tagged_root_from_document(&doc);
        assert_eq!(root.tag, "document");
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].tag, "section:body");
        assert_eq!(root.children[1].tag, "section:footer");
        // Check paragraph child of first section
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].tag, "paragraph");
    }

    /// test_collect_ids_document — verify ID collection from a document.
    #[test]
    fn test_collect_ids_document() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::derive("paragraph", &["para:0"], &"test"),
                    runs: vec![Run {
                        id: ElementId::derive("run", &["run:0"], &"test"),
                        text: "test".into(),
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
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let mut ids: Vec<serde_json::Value> = Vec::new();
        collect_ids(&ir, "", &mut ids);
        assert!(!ids.is_empty(), "Should have collected some IDs");
        // Check that all IDs have the expected fields
        for entry in &ids {
            assert!(entry["kind"].is_string());
            assert!(entry["path"].is_string());
            assert!(entry["id"].is_string());
        }
    }

    /// test_build_fidelity_budget — verify default fidelity budget serialization.
    #[test]
    fn test_build_fidelity_budget() {
        let budget = build_fidelity_budget(&IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::default(),
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        });
        assert!(budget["confidence"].is_number());
        assert_eq!(budget["confidence"].as_f64().unwrap(), 1.0);
        assert!(budget["dropped_features"].is_array());
        assert!(budget["part_warnings"].is_array());
        assert!(budget["unknown_parts"].is_array());
        assert!(budget["rendered_by"].is_null());
        assert!(!budget["has_potentially_unsafe"].as_bool().unwrap());
    }

    /// test_unsupported_format — verify unsupported format returns error.
    #[test]
    fn test_unsupported_format() {
        let result = parse_document(b"fake", "test.txt");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unsupported format"));
    }

    /// test_element_ids_have_valid_format — all collected IDs match the expected pattern.
    #[test]
    fn test_element_ids_have_valid_format() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::derive("paragraph", &["para:0"], &"test"),
                    runs: vec![Run {
                        id: ElementId::derive("run", &["run:0"], &"test"),
                        text: "test".into(),
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
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let mut ids: Vec<serde_json::Value> = Vec::new();
        collect_ids(&ir, "", &mut ids);
        // All IDs should match the ELEMENT_ID_PATTERN: id-[0-9a-f]{12,}
        for entry in &ids {
            let id_str = entry["id"].as_str().expect("id should be string");
            assert!(
                id_str.starts_with("id-"),
                "ID should start with 'id-': {}",
                id_str
            );
            let hex_part = &id_str[3..];
            assert!(
                hex_part.len() >= 12,
                "ID hex part should be at least 12 chars: {}",
                id_str
            );
            assert!(
                hex_part.chars().all(|c| c.is_ascii_hexdigit()),
                "ID hex part should be hex: {}",
                id_str
            );
        }
    }

    /// test_collect_ids_single_walk — each tagged tree node appears exactly once.
    #[test]
    fn test_collect_ids_single_walk() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::derive("paragraph", &["para:0"], &"test"),
                    runs: vec![Run {
                        id: ElementId::derive("run", &["run:0"], &"test"),
                        text: "hello".into(),
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
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let mut ids = Vec::new();
        collect_ids(&ir, "", &mut ids);
        // Exactly 4 nodes: document root, section:body, paragraph, run
        assert_eq!(
            ids.len(),
            4,
            "collect_ids should yield exactly 4 entries, got {}",
            ids.len()
        );
        let paths: Vec<&str> = ids.iter().map(|e| e["path"].as_str().unwrap()).collect();
        assert_eq!(paths[0], "/document");
        assert_eq!(paths[1], "/document/section:body");
        assert_eq!(paths[2], "/document/section:body/paragraph");
        assert_eq!(paths[3], "/document/section:body/paragraph/run");
    }

    /// test_inspect_result_no_id_map — id_map removed (was a third full tree copy).
    #[test]
    fn test_inspect_result_no_id_map() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let result = build_inspect_result(&ir, "/test/path.docx");
        assert!(
            result.get("id_map").is_none() || result["id_map"].is_null(),
            "id_map must not be present in inspect result to avoid a third full-tree copy"
        );
    }

    /// test_inspect_result_json_shape — verify the full result structure.
    #[test]
    fn test_inspect_result_json_shape() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::derive("paragraph", &["para:0"], &"test"),
                    runs: vec![Run {
                        id: ElementId::derive("run", &["run:0"], &"test"),
                        text: "test".into(),
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
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let result = build_inspect_result(&ir, "/test/path.docx");

        // Verify top-level fields
        assert_eq!(result["tool"].as_str().unwrap(), "inspect_document");
        assert_eq!(result["path"].as_str().unwrap(), "/test/path.docx");
        assert_eq!(result["ir_version"].as_str().unwrap(), IR_VERSION);
        assert_eq!(result["document_type"].as_str().unwrap(), "document");
        assert!(result["structure"].is_object());
        assert!(result["element_kinds"].is_object());
        assert!(result["element_ids_sample"].is_array());
        assert!(result["total_elements"].is_number());
        assert!(result["coordinates"].is_array());
        assert!(result["fidelity_budget"].is_object());

        // Verify element_ids_sample entries have required fields
        for entry in result["element_ids_sample"].as_array().unwrap() {
            assert!(entry["kind"].is_string(), "Missing kind in: {}", entry);
            assert!(entry["path"].is_string(), "Missing path in: {}", entry);
            assert!(entry["id"].is_string(), "Missing id in: {}", entry);
        }

        // Verify element_kinds counts sum to total_elements
        let total = result["total_elements"].as_u64().unwrap() as usize;
        let kinds_sum: usize = result["element_kinds"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_u64().unwrap() as usize)
            .sum();
        assert_eq!(
            total, kinds_sum,
            "element_kinds counts must sum to total_elements"
        );
    }

    /// test_inspect_table_document — verify table ID collection.
    #[test]
    fn test_inspect_table_document() {
        let cell = Cell {
            id: ElementId::derive("cell", &["cell:0"], &"data"),
            contents: vec![],
            colspan: None,
            rowspan: None,
        };
        let row = Row { cells: vec![cell] };
        let table = Table {
            id: ElementId::derive("table", &["table:0"], &"test"),
            rows: vec![row],
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Table(table)],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let mut ids: Vec<serde_json::Value> = Vec::new();
        collect_ids(&ir, "", &mut ids);
        // Should have section, table, cell
        let kinds: Vec<&str> = ids.iter().filter_map(|e| e["kind"].as_str()).collect();
        assert!(
            kinds.contains(&"Table") || kinds.contains(&"table"),
            "Should have Table: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&"Cell") || kinds.contains(&"cell"),
            "Should have Cell: {:?}",
            kinds
        );
    }

    /// test_inspect_list_document — verify list ID collection.
    #[test]
    fn test_inspect_list_document() {
        let list_item = ListItem {
            id: ElementId::derive("listitem", &["item:0"], &"item"),
            content: vec![],
            level: 0,
        };
        let list = List {
            id: ElementId::derive("list", &["list:0"], &"test"),
            kind: ListKind::Numbered,
            items: vec![list_item],
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::List(list)],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let mut ids: Vec<serde_json::Value> = Vec::new();
        collect_ids(&ir, "", &mut ids);
        let kinds: Vec<&str> = ids.iter().filter_map(|e| e["kind"].as_str()).collect();
        assert!(
            kinds.contains(&"List") || kinds.contains(&"list"),
            "Should have List: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&"ListItem") || kinds.contains(&"list_item"),
            "Should have ListItem: {:?}",
            kinds
        );
    }

    /// test_document_type_names — verify document type detection for different IR types.
    #[test]
    fn test_document_type_names() {
        // Document type
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        assert_eq!(document_type_name(&ir), "document");

        // PDF type
        let pdf_ir = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::derive("pdf", &["pdf:0"], &"test"),
                tag: "pdf".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![TaggedNode {
                    id: ElementId::derive("page", &["page:0"], &"p0"),
                    tag: "page".to_string(),
                    alt_text: Some("Page 0: 612x792pt".to_string()),
                    bbox: None,
                    children: vec![],
                }],
            },
        };
        assert_eq!(document_type_name(&pdf_ir), "pdf");

        // Unknown type
        let unknown_ir = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::default(),
                tag: "unknown".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        };
        assert_eq!(document_type_name(&unknown_ir), "unknown");
    }

    /// test_structure_summary — verify structure summary for documents and PDFs.
    #[test]
    fn test_structure_summary() {
        // Document with 2 sections
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![
                Section {
                    id: ElementId::derive("section", &["section:0"], &"s0"),
                    kind: SectionKind::Body,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                },
                Section {
                    id: ElementId::derive("section", &["section:1"], &"s1"),
                    kind: SectionKind::Body,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                },
            ],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let summary = build_structure_summary(&ir);
        assert_eq!(summary["sections"].as_u64().unwrap(), 2);

        // PDF with 3 pages
        let pdf_ir = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::derive("pdf", &["pdf:0"], &"test"),
                tag: "pdf".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![
                    TaggedNode {
                        id: ElementId::derive("page", &["page:0"], &"p0"),
                        tag: "page".to_string(),
                        alt_text: Some("Page 0: 612x792pt".to_string()),
                        bbox: None,
                        children: vec![],
                    },
                    TaggedNode {
                        id: ElementId::derive("page", &["page:1"], &"p1"),
                        tag: "page".to_string(),
                        alt_text: Some("Page 1: 612x792pt".to_string()),
                        bbox: None,
                        children: vec![],
                    },
                    TaggedNode {
                        id: ElementId::derive("page", &["page:2"], &"p2"),
                        tag: "page".to_string(),
                        alt_text: Some("Page 2: 612x792pt".to_string()),
                        bbox: None,
                        children: vec![],
                    },
                ],
            },
        };
        let summary = build_structure_summary(&pdf_ir);
        assert_eq!(summary["pages"].as_u64().unwrap(), 3);
    }

    /// test_coordinate_manifest_document — verify coordinate manifest for documents.
    #[test]
    fn test_coordinate_manifest_document() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![
                Section {
                    id: ElementId::derive("section", &["section:0"], &"s0"),
                    kind: SectionKind::Body,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                },
                Section {
                    id: ElementId::derive("section", &["section:1"], &"s1"),
                    kind: SectionKind::Header,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                },
            ],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let coords = build_coordinate_manifest(&ir);
        assert_eq!(coords.len(), 2);
        assert_eq!(
            coords[0]["section"].as_str().unwrap(),
            ir.root.children[0].id.as_str()
        );
    }

    /// test_coordinate_manifest_pdf — verify coordinate manifest for PDFs.
    #[test]
    fn test_coordinate_manifest_pdf() {
        let pdf_ir = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::derive("pdf", &["pdf:0"], &"test"),
                tag: "pdf".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![TaggedNode {
                    id: ElementId::derive("page", &["page:0"], &"p0"),
                    tag: "page".to_string(),
                    alt_text: Some("Page 0: 612x792pt".to_string()),
                    bbox: Some([0.0, 0.0, 612.0, 792.0]),
                    children: vec![
                        TaggedNode {
                            id: ElementId::derive("text", &["text:0"], &"hello"),
                            tag: "text".to_string(),
                            alt_text: Some("hello".to_string()),
                            bbox: Some([100.0, 700.0, 150.0, 688.0]),
                            children: vec![],
                        },
                        TaggedNode {
                            id: ElementId::derive("text", &["text:1"], &"world"),
                            tag: "text".to_string(),
                            alt_text: Some("world".to_string()),
                            bbox: Some([160.0, 700.0, 220.0, 688.0]),
                            children: vec![],
                        },
                    ],
                }],
            },
        };
        let coords = build_coordinate_manifest(&pdf_ir);
        assert_eq!(coords.len(), 1);
        assert_eq!(coords[0]["page"].as_u64().unwrap(), 0);
        assert_eq!(coords[0]["bbox_count"].as_u64().unwrap(), 2);
        assert_eq!(coords[0]["width_pt"].as_f64().unwrap(), 612.0);
        assert_eq!(coords[0]["height_pt"].as_f64().unwrap(), 792.0);
        // Per-text bbox coordinates are surfaced (actual coordinates, not alt_text only)
        let bboxes = coords[0]["bboxes"].as_array().unwrap();
        assert_eq!(bboxes[0]["text"].as_str().unwrap(), "hello");
        assert_eq!(bboxes[0]["bbox_pt"][0].as_f64().unwrap(), 100.0);
        assert_eq!(bboxes[0]["bbox_pt"][2].as_f64().unwrap(), 150.0);
        assert_eq!(bboxes[0]["type"].as_str().unwrap(), "text");
    }

    /// test_coordinate_manifest_drawing — document drawings carry bbox_pt.
    #[test]
    fn test_coordinate_manifest_drawing() {
        let drawing = Drawing {
            id: ElementId::derive("drawing", &["drawing:0"], &"img"),
            mime_type: "image/png".into(),
            data: "abcd".into(),
            bbox_pt: Some([20.0, 30.0, 220.0, 130.0]),
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &"s0"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Drawing(drawing)],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        };
        let ir = ir_document_from_document(&doc);
        let coords = build_coordinate_manifest(&ir);
        assert_eq!(coords.len(), 1);
        let drawn = coords[0]["drawing_bboxes"].as_array().unwrap();
        assert_eq!(drawn.len(), 1, "drawing bbox should be surfaced");
        assert_eq!(drawn[0]["bbox_pt"][0].as_f64().unwrap(), 20.0);
        assert_eq!(drawn[0]["bbox_pt"][3].as_f64().unwrap(), 130.0);
    }
}
