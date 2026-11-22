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
pub fn inspect_document(
    path: &str,
    styles: bool,
    media: bool,
    annots: bool,
    ids: bool,
    coords: bool,
    bboxes: bool,
) -> Result<serde_json::Value, anyhow::Error> {
    let bytes = read_document_bytes(path)
        .map_err(|e| anyhow::anyhow!("failed to read document: {}: {}", path, e))?;

    let ir = parse_document(&bytes, path)?;

    let result = build_inspect_result(&ir, path, styles, media, annots, ids, coords, bboxes);
    Ok(result)
}

/// Parse a document from a path into an IRDocument (public entry point for tabular export).
pub fn parse_document_from_path(path: &str) -> Result<IRDocument, anyhow::Error> {
    let bytes = gp_core::file_access::read_document_bytes(path)
        .map_err(|e| anyhow::anyhow!("failed to read: {}: {}", path, e))?;
    parse_document(&bytes, path)
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
#[allow(clippy::too_many_arguments)]
fn build_inspect_result(
    ir: &IRDocument,
    path: &str,
    styles: bool,
    media: bool,
    annots: bool,
    ids: bool,
    coords: bool,
    bboxes: bool,
) -> serde_json::Value {
    // Collect element IDs
    let mut all_ids: Vec<serde_json::Value> = Vec::new();
    collect_ids(ir, "", &mut all_ids);

    // Build compact representation: element kind counts + first 5 samples
    let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &all_ids {
        if let Some(kind) = entry.get("kind").and_then(|k| k.as_str()) {
            *kind_counts.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    let sample_len = all_ids.len().min(5);
    let element_ids_sample: Vec<serde_json::Value> = all_ids[..sample_len].to_vec();

    // Build coordinates block conditionally
    let coordinates_value: serde_json::Value = if coords {
        json!(build_coordinate_manifest(ir))
    } else {
        json!([])
    };

    // Build bboxes block conditionally
    let bboxes_value: serde_json::Value = if bboxes {
        json!(build_bboxes(ir))
    } else {
        json!([])
    };

    // Build fidelity budget
    let budget = build_fidelity_budget(ir);

    // Build element_ids block conditionally
    let element_ids_value: serde_json::Value = if ids { json!(all_ids) } else { json!([]) };

    let mut result = json!({
        "tool": "inspect_document",
        "path": path,
        "ir_version": IR_VERSION,
        "document_type": document_type_name(ir),
        "structure": build_structure_summary(ir),
        "element_kinds": kind_counts,
        "element_ids_sample": element_ids_sample,
        "element_ids": element_ids_value,
        "total_elements": all_ids.len(),
        "coordinates": coordinates_value,
        "bboxes": bboxes_value,
        "fidelity_budget": budget,
    });

    if styles {
        if let Some(styles_block) = build_styles_block(ir) {
            result["styles"] = styles_block;
        }
    }
    if media {
        if let Some(media_block) = build_media_block(ir) {
            result["media"] = media_block;
        }
    }
    if annots {
        result["annotations"] = build_annotations_block(ir);
    }

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

/// Build bounding box manifest from the tagged tree.
fn build_bboxes(ir: &IRDocument) -> Vec<serde_json::Value> {
    let mut bboxes: Vec<serde_json::Value> = Vec::new();
    collect_bboxes(&ir.root, &mut bboxes);
    bboxes
}

/// Recursively collect bounding boxes from the tagged tree.
fn collect_bboxes(node: &TaggedNode, bboxes: &mut Vec<serde_json::Value>) {
    if let Some(bbox) = node.bbox {
        bboxes.push(json!({
            "id": node.id.as_str(),
            "tag": node.tag,
            "bbox_pt": bbox,
        }));
    }
    for child in &node.children {
        collect_bboxes(child, bboxes);
    }
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

/// Build styles block from a Document IR.
fn build_styles_block(ir: &IRDocument) -> Option<serde_json::Value> {
    // Walk the tagged tree to find document sections, then extract styles info
    for node in &ir.root.children {
        if node.tag == "document" {
            // Look for style-related children in the tree
            let mut style_entries: Vec<serde_json::Value> = Vec::new();
            let mut numbering_entries: Vec<serde_json::Value> = Vec::new();
            let mut doc_props: Option<serde_json::Value> = None;

            // Recursively collect style info from tagged tree
            collect_style_info(
                node,
                &mut style_entries,
                &mut numbering_entries,
                &mut doc_props,
            );

            if style_entries.is_empty() && numbering_entries.is_empty() && doc_props.is_none() {
                return None;
            }

            let mut result = serde_json::Map::new();
            if !style_entries.is_empty() {
                result.insert(
                    "styles".to_string(),
                    serde_json::Value::Array(style_entries),
                );
            }
            if !numbering_entries.is_empty() {
                result.insert(
                    "numbering".to_string(),
                    serde_json::Value::Array(numbering_entries),
                );
            }
            if let Some(props) = doc_props {
                result.insert("doc_props".to_string(), props);
            }
            if result.is_empty() {
                return None;
            }
            return Some(serde_json::Value::Object(result));
        }
    }
    None
}

/// Recursively collect style information from tagged tree nodes.
fn collect_style_info(
    node: &TaggedNode,
    styles: &mut Vec<serde_json::Value>,
    numbering: &mut Vec<serde_json::Value>,
    doc_props: &mut Option<serde_json::Value>,
) {
    // Check node content for style indicators
    if let Some(ref alt) = node.alt_text {
        if alt.contains("style:") || alt.contains("Style:") {
            styles.push(json!({
                "tag": node.tag,
                "alt_text": alt,
                "id": node.id.as_str(),
            }));
        }
        if alt.contains("numbering") || alt.contains("Numbering") {
            numbering.push(json!({
                "tag": node.tag,
                "alt_text": alt,
                "id": node.id.as_str(),
            }));
        }
        if alt.contains("docProps") || alt.contains("creator") || alt.contains("title") {
            *doc_props = Some(json!({
                "tag": node.tag,
                "alt_text": alt,
                "id": node.id.as_str(),
            }));
        }
    }

    for child in &node.children {
        collect_style_info(child, styles, numbering, doc_props);
    }
}

/// Build media block from Drawing/Image elements with data/mime_type.
fn build_media_block(ir: &IRDocument) -> Option<serde_json::Value> {
    let mut drawings: Vec<serde_json::Value> = Vec::new();
    let mut total_data_bytes: usize = 0;

    collect_drawings(&ir.root, &mut drawings, &mut total_data_bytes);

    let drawing_count = drawings.len();
    if drawing_count == 0 {
        return None;
    }

    Some(json!({
        "drawings": drawings,
        "image_count": drawing_count,
        "total_data_bytes": total_data_bytes,
    }))
}

/// Recursively collect drawing info from the tagged tree.
fn collect_drawings(
    node: &TaggedNode,
    drawings: &mut Vec<serde_json::Value>,
    total_bytes: &mut usize,
) {
    if node.tag == "drawing" {
        let data_len = node.alt_text.as_deref().map_or(0, |alt| {
            // Parse "mime_typexdata_len" format from alt_text
            alt.split('x')
                .nth(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0)
        });
        *total_bytes += data_len;

        drawings.push(json!({
            "id": node.id.as_str(),
            "alt_text": node.alt_text.as_deref().unwrap_or(""),
            "mime_type": extract_mime_type(&node.alt_text),
            "bbox_pt": node.bbox,
            "data_len": data_len,
        }));
    }
    for child in &node.children {
        collect_drawings(child, drawings, total_bytes);
    }
}

/// Extract MIME type from alt_text string.
fn extract_mime_type(alt: &Option<String>) -> String {
    alt.as_deref().map_or("unknown".to_string(), |a| {
        a.split('x').next().unwrap_or("unknown").to_string()
    })
}

/// Build annotations block counting comments, tracked changes, and revisions.
fn build_annotations_block(ir: &IRDocument) -> serde_json::Value {
    let mut comments_count: usize = 0;
    let mut tracked_changes_count: usize = 0;
    let mut revisions_count: usize = 0;

    // Walk the tree to count annotation-related elements
    collect_annotation_counts(
        &ir.root,
        &mut comments_count,
        &mut tracked_changes_count,
        &mut revisions_count,
    );

    json!({
        "comments": comments_count,
        "tracked_changes": tracked_changes_count,
        "revisions": revisions_count,
    })
}

/// Recursively count annotations from the tagged tree.
fn collect_annotation_counts(
    node: &TaggedNode,
    comments: &mut usize,
    tracked_changes: &mut usize,
    revisions: &mut usize,
) {
    match node.tag.as_str() {
        "comment" | "Comment" => *comments += 1,
        "tracked_change" | "TrackedChange" | "insertion" | "deletion" => *tracked_changes += 1,
        "revision" | "Revision" => *revisions += 1,
        _ => {}
    }
    for child in &node.children {
        collect_annotation_counts(child, comments, tracked_changes, revisions);
    }
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

/// Export spreadsheet data from an IRDocument as CSV.
///
/// Walks the tagged tree looking for spreadsheet nodes, extracts cell values
/// into a 2D grid, and serializes with proper CSV quoting.
pub fn export_tabular(ir: &IRDocument, format: &str) -> Result<String, anyhow::Error> {
    let is_tsv = format == "tsv";
    let delimiter = if is_tsv { '\t' } else { ',' };

    // Find spreadsheet nodes in the tagged tree
    let mut sheets: Vec<(String, Vec<Vec<Option<String>>>)> = Vec::new();

    for child in &ir.root.children {
        if child.tag == "spreadsheet" {
            for sheet_node in &child.children {
                if sheet_node.tag.starts_with("sheet:") {
                    let sheet_name = sheet_node
                        .tag
                        .strip_prefix("sheet:")
                        .unwrap_or("Sheet")
                        .to_string();
                    let grid = extract_sheet_grid(sheet_node);
                    if !grid.is_empty() {
                        sheets.push((sheet_name, grid));
                    }
                }
            }
        }
    }

    if sheets.is_empty() {
        return Ok(String::new());
    }

    // Build CSV output — use the first sheet by default
    let (sheet_name, grid) = &sheets[0];
    let mut output = String::new();

    // Add sheet name as first line comment
    if !sheet_name.is_empty() {
        output.push_str(&format!("// Sheet: {}\n", sheet_name));
    }

    for row in grid {
        let mut cells: Vec<String> = Vec::new();
        for cell in row {
            let val = cell.as_deref().unwrap_or("");
            cells.push(csv_quote(val, delimiter));
        }
        output.push_str(&cells.join(&delimiter.to_string()));
        output.push('\n');
    }

    Ok(output)
}

/// Extract cell values from a sheet tagged node into a 2D grid.
///
/// The grid is indexed [row][col] where the first row with data becomes row 0.
fn extract_sheet_grid(sheet_node: &TaggedNode) -> Vec<Vec<Option<String>>> {
    let mut max_col: usize = 0;
    let mut cells_by_pos: std::collections::BTreeMap<(usize, usize), String> =
        std::collections::BTreeMap::new();

    for row_node in &sheet_node.children {
        if !row_node.tag.starts_with("row:") {
            continue;
        }
        // Parse row index from tag
        let row_idx: usize = row_node
            .tag
            .strip_prefix("row:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        for cell_node in &row_node.children {
            if !cell_node.tag.starts_with("cell:") {
                continue;
            }
            // Parse col index from tag "cell:row,col" — col is the part AFTER the comma
            let col_idx = cell_node
                .tag
                .strip_prefix("cell:")
                .and_then(|s| s.find(',').and_then(|pos| s[pos + 1..].parse().ok()))
                .unwrap_or(0);

            if col_idx >= max_col {
                max_col = col_idx + 1;
            }

            // Get value from alt_text if available
            let value = cell_node.alt_text.clone();
            cells_by_pos.insert((row_idx, col_idx), value.unwrap_or_default());
        }
    }

    if cells_by_pos.is_empty() {
        return vec![];
    }

    // Determine number of rows
    let max_row = cells_by_pos.keys().map(|(r, _)| *r).max().unwrap_or(0);

    // Collect merged ranges from __merged_ranges child node (if present)
    let mut merged_ranges: Vec<(usize, usize, usize, usize)> = Vec::new();
    for child in &sheet_node.children {
        if child.tag == "__merged_ranges" {
            if let Some(ref alt) = child.alt_text {
                if let Ok(arr) = serde_json::from_str::<Vec<Vec<usize>>>(alt) {
                    for r in arr {
                        if r.len() >= 4 {
                            merged_ranges.push((r[0], r[1], r[2], r[3]));
                        }
                    }
                }
            }
        }
    }

    // Size the grid to cover both populated cells and any merged range extent.
    let mut needed_cols = max_col;
    let mut needed_rows = max_row + 1;
    for (_, _, er, ec) in &merged_ranges {
        if *ec + 1 > needed_cols {
            needed_cols = *ec + 1;
        }
        if *er + 1 > needed_rows {
            needed_rows = *er + 1;
        }
    }

    // Build grid
    let mut grid = vec![vec![None; needed_cols]; needed_rows];
    for ((row, col), value) in &cells_by_pos {
        if *row < needed_rows && *col < needed_cols {
            grid[*row][*col] = Some(value.clone());
        }
    }

    // Fill merged ranges: propagate top-left cell value into spanned cells.
    for (sr, sc, er, ec) in &merged_ranges {
        let top_left = if *sr < needed_rows && *sc < needed_cols {
            grid[*sr][*sc].clone()
        } else {
            None
        };
        let row_end = (*er + 1).min(needed_rows);
        let col_end = (*ec + 1).min(needed_cols);
        for (r, row) in grid.iter_mut().enumerate().take(row_end).skip(*sr) {
            for (c, cell) in row.iter_mut().enumerate().take(col_end).skip(*sc) {
                if r == *sr && c == *sc {
                    continue;
                }
                if cell.is_none() {
                    *cell = top_left.clone();
                }
            }
        }
    }

    grid
}

/// Quote a cell value for CSV/TSV output with proper escaping.
fn csv_quote(value: &str, delimiter: char) -> String {
    let needs_quoting = value.contains(&['"', '\n', '\r', delimiter][..])
        || !value
            .chars()
            .all(|c| (c as u32) >= 0x20 && (c as u32) != 0x7f || c == '\t');
    if !needs_quoting {
        return value.to_string();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        if c == '"' {
            quoted.push_str("\"\"");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('"');
    quoted
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
                        ..Default::default()
                    })],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                    ..Default::default()
                },
                Section {
                    id: ElementId::derive("section", &["section:1"], &"footer"),
                    kind: SectionKind::Footer,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                    ..Default::default()
                },
            ],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                    ..Default::default()
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                    ..Default::default()
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                    ..Default::default()
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
            ..Default::default()
        };
        let ir = ir_document_from_document(&doc);
        let result = build_inspect_result(
            &ir,
            "/test/path.docx",
            false,
            false,
            false,
            false,
            false,
            false,
        );
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
                    ..Default::default()
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
        };
        let ir = ir_document_from_document(&doc);
        let result = build_inspect_result(
            &ir,
            "/test/path.docx",
            false,
            false,
            false,
            false,
            false,
            false,
        );

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
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                    ..Default::default()
                },
                Section {
                    id: ElementId::derive("section", &["section:1"], &"s1"),
                    kind: SectionKind::Body,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                    ..Default::default()
                },
            ],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                    ..Default::default()
                },
                Section {
                    id: ElementId::derive("section", &["section:1"], &"s1"),
                    kind: SectionKind::Header,
                    items: vec![],
                    headers: vec![],
                    footers: vec![],
                    footnotes: vec![],
                    endnotes: vec![],
                    ..Default::default()
                },
            ],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
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
                ..Default::default()
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
            ..Default::default()
        };
        let ir = ir_document_from_document(&doc);
        let coords = build_coordinate_manifest(&ir);
        assert_eq!(coords.len(), 1);
        let drawn = coords[0]["drawing_bboxes"].as_array().unwrap();
        assert_eq!(drawn.len(), 1, "drawing bbox should be surfaced");
        assert_eq!(drawn[0]["bbox_pt"][0].as_f64().unwrap(), 20.0);
        assert_eq!(drawn[0]["bbox_pt"][3].as_f64().unwrap(), 130.0);
    }

    /// test_export_tabular_columns_and_merged — CSV columns use the real column
    /// index (not the row index) and merged ranges propagate the top-left value.
    #[test]
    fn test_export_tabular_columns_and_merged() {
        let cell = |row: usize, col: usize, value: &str| SheetCell {
            id: ElementId::derive("sheet_cell", &[&format!("{}:{}", row, col)], &value),
            row,
            col,
            value: Some(value.to_string()),
            formula: None,
            cached_value: None,
            style_name: None,
            number_format: None,
            rich: None,
        };
        let sheet = Sheet {
            id: ElementId::derive("sheet", &["sheet:0"], &"Main"),
            name: "Main".into(),
            rows: vec![
                SheetRow {
                    id: ElementId::derive("sheet_row", &["row:0"], &0),
                    cells: vec![cell(0, 0, "a"), cell(0, 2, "c")],
                    height: None,
                    hidden: None,
                },
                SheetRow {
                    id: ElementId::derive("sheet_row", &["row:1"], &1),
                    cells: vec![cell(1, 1, "b")],
                    height: None,
                    hidden: None,
                },
            ],
            merged_ranges: vec![MergedRange {
                id: ElementId::derive("merged", &["merged:0"], &"A1:B1"),
                start_row: 0,
                start_col: 0,
                end_row: 0,
                end_col: 1,
            }],
            ..Default::default()
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
            defined_names: vec![],
        };
        let ir = ir_convert::ir_document_from_spreadsheet(&ss);
        let csv = export_tabular(&ir, "csv").unwrap();
        // Column indices must be honoured: "c" stays in column C, "b" in column B.
        assert_eq!(csv, "// Sheet: Main\na,a,c\n,b,\n");
    }
}
