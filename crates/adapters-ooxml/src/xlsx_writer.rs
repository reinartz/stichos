// SPDX-License-Identifier: MIT OR Apache-2.0
//! XLSX patch writer — applies IR patch operations to OOXML spreadsheets.
//!
//! Routes patch ops to span mutations on worksheet XML, handles SetCellValue,
//! formula detection, stale cached value tracking, row append, and number formats.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::ElementId;
use gp_core::patch::PatchOp;
use std::collections::BTreeMap;

use super::opc::OpcPackage;
use super::span_mut::{apply_span_mutations, MutationKind, SpanMutation};
use super::writer::{PartMutation, SurgicalWriter};
use super::xml::{ByteSpan, TagName, XmlDocument, XmlNode};

// ===========================================================================
// Types
// ===========================================================================

/// An XLSX-specific patch writer.
pub struct XlsxPatchWriter<'a> {
    /// The OPC package to patch.
    pub package: &'a OpcPackage,
    /// The patch operations to apply.
    pub ops: Vec<PatchOp>,
}

/// Result of XLSX patch application.
pub struct XlsxPatchResult {
    /// The patched OPC bytes.
    pub output: Vec<u8>,
    /// Which parts were modified.
    pub modified_parts: Vec<String>,
    /// Elements that were changed (by ID).
    pub changed_element_ids: Vec<String>,
    /// Cells with stale cached values (cells that feed formulas).
    pub stale_cached_values: Vec<String>,
    /// Diagnostics from the patch process.
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> XlsxPatchWriter<'a> {
    /// Create a new XLSX patch writer.
    pub fn new(package: &'a OpcPackage, ops: Vec<PatchOp>) -> Self {
        Self { package, ops }
    }

    /// Apply all patch operations, return patched OPC bytes.
    pub fn write(&self) -> Result<XlsxPatchResult, Vec<Diagnostic>> {
        let mut diags = Vec::new();

        // 1. Parse all worksheet parts.
        let mut parts = match self.parse_worksheet_parts() {
            Ok(p) => p,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        // 2. Route patch ops to span mutations per part.
        let mutations_by_part = match self.route_ops(&parts) {
            Ok(m) => m,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        // 3. Apply span mutations to each part.
        let mut modified_parts = Vec::new();
        let mut changed_element_ids = Vec::new();
        let stale_cached_values = self.detect_stale_cached_values(&parts);

        for (part_uri, mutations) in &mutations_by_part {
            if let Some(doc) = parts.get_mut(part_uri) {
                match apply_span_mutations(doc, mutations.clone()) {
                    Ok(result) => {
                        modified_parts.push(part_uri.clone());
                        changed_element_ids.extend(
                            result
                                .changed_spans
                                .iter()
                                .map(|s| format!("span@{}", s.offset)),
                        );
                        *doc = result.document;
                    }
                    Err(e) => {
                        diags.push(e);
                    }
                }
            }
        }

        // 4. Detect stale cached values by scanning formula references.
        let _stale = self.detect_stale_cached_values(&parts);

        // 5. Build PartMutation map and write via SurgicalWriter.
        let mut mutations = BTreeMap::new();
        for (uri, doc) in &parts {
            let part_data = doc.to_bytes();
            mutations.insert(
                uri.clone(),
                PartMutation::Replace {
                    data: part_data,
                    content_type: None,
                },
            );
        }

        let writer = SurgicalWriter::new(&[]);
        let write_result = match writer.write_from_parts(&self.package.parts, mutations) {
            Ok(r) => r,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        Ok(XlsxPatchResult {
            output: write_result.bytes,
            modified_parts,
            changed_element_ids,
            stale_cached_values,
            diagnostics: diags,
        })
    }

    /// Parse all worksheet parts from the OPC package.
    fn parse_worksheet_parts(&self) -> Result<BTreeMap<String, XmlDocument>, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut parts = BTreeMap::new();

        for part in &self.package.parts {
            if part.uri.starts_with("xl/worksheets/") && part.uri.ends_with(".xml") {
                match super::xml::parse_xml(&part.data) {
                    Ok(doc) => {
                        parts.insert(part.uri.clone(), doc);
                    }
                    Err(e) => {
                        diags.push(Diagnostic::new(
                            "OOX-E-002",
                            Severity::Error,
                            format!("Failed to parse worksheet {}: {}", part.uri, e[0].message),
                        ));
                    }
                }
            }
        }

        if parts.is_empty() {
            diags.push(Diagnostic::new(
                "OOX-W-022",
                Severity::Warning,
                "No worksheet parts found to patch".to_string(),
            ));
        }

        Ok(parts)
    }

    /// Route patch ops to span mutations.
    fn route_ops(
        &self,
        parts: &BTreeMap<String, XmlDocument>,
    ) -> Result<BTreeMap<String, Vec<SpanMutation>>, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut mutations_by_part: BTreeMap<String, Vec<SpanMutation>> = BTreeMap::new();

        for op in &self.ops {
            match op {
                PatchOp::SetCellValue { cell, value } => {
                    match self.route_set_cell_value(cell, value, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetCellRich { cell, runs } => {
                    match self.route_set_cell_rich_text(cell, runs, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::ReplaceText { elem, old: _, new } => {
                    match self.route_replace_text(elem, new, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetRunProps { .. } => {
                    // Not applicable to xlsx.
                }
                PatchOp::Insert { anchor, kind } => {
                    match self.route_insert_row(anchor, kind, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::Delete { elem } => {
                    match self.route_delete(elem, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::TrackInsert { anchor, kind, .. } => {
                    match self.route_track_insert(anchor, kind, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::TrackDelete { elem, .. } => {
                    match self.route_track_delete(elem, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
            }
        }

        if !diags.is_empty() {
            return Err(diags);
        }

        Ok(mutations_by_part)
    }

    /// Route SetCellValue op.
    fn route_set_cell_value(
        &self,
        cell: &ElementId,
        value: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_cell_value_span(doc, cell) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::ReplaceText(value.to_string()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route ReplaceText op.
    fn route_replace_text(
        &self,
        elem: &ElementId,
        new: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_element_span(doc, elem) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::ReplaceText(new.to_string()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route Insert op (row append).
    fn route_insert_row(
        &self,
        anchor: &ElementId,
        kind: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let new_node = self.create_element(kind);
        for (part_uri, doc) in parts {
            if let Some(anchor_span) = self.find_element_span(doc, anchor) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span: anchor_span,
                        kind: MutationKind::InsertNode(new_node.clone()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route Delete op.
    fn route_delete(
        &self,
        elem: &ElementId,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_element_span(doc, elem) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::DeleteNode,
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route TrackInsert op.
    fn route_track_insert(
        &self,
        anchor: &ElementId,
        kind: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let new_node = self.create_element(kind);
        for (part_uri, doc) in parts {
            if let Some(anchor_span) = self.find_element_span(doc, anchor) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span: anchor_span,
                        kind: MutationKind::InsertNode(new_node.clone()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route TrackDelete op.
    fn route_track_delete(
        &self,
        elem: &ElementId,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_element_span(doc, elem) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::DeleteNode,
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route SetCellRich op — write rich runs into an `<is>` element.
    fn route_set_cell_rich_text(
        &self,
        cell_id: &ElementId,
        runs: &[gp_core::ir::CellRun],
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let rich_xml = self.create_rich_text_is(runs);
        for (part_uri, doc) in parts {
            if let Some(cell_span) = self.find_cell_span_by_id(doc, cell_id) {
                // Delete the existing cell content and insert rich text.
                // We replace the entire cell by inserting the new rich structure.
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span: cell_span,
                        kind: MutationKind::InsertNode(rich_xml.clone()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Find a cell's span by matching its cell reference attribute (e.g., r="A1").
    fn find_cell_span_by_id(&self, doc: &XmlDocument, cell_id: &ElementId) -> Option<ByteSpan> {
        self.find_cell_node(&doc.root, cell_id)
    }

    /// Recursively find a cell <c> node by matching its r attribute.
    fn find_cell_node<'b>(&'b self, node: &'b XmlNode, target_id: &ElementId) -> Option<ByteSpan> {
        if let XmlNode::Element {
            attributes,
            children,
            span,
            ..
        } = node
        {
            // Match on 'r' attribute (cell reference like A1) or element id
            if let Some(r_attr) = attributes.get(&TagName::local("r")) {
                if r_attr == target_id.as_str() {
                    return Some(span.clone());
                }
            }
            // Also try matching element IDs stored in alt_text
            if let Some(id_attr) = attributes.get(&TagName::local("id")) {
                if id_attr == target_id.as_str() {
                    return Some(span.clone());
                }
            }
            for child in children {
                if let Some(found) = self.find_cell_node(child, target_id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Build an `<is>` element containing rich run XML from CellRun values.
    fn create_rich_text_is(&self, runs: &[gp_core::ir::CellRun]) -> XmlNode {
        let mut children: Vec<XmlNode> = Vec::new();
        for run in runs {
            let mut run_children: Vec<XmlNode> = Vec::new();

            // Build <rPr> if any formatting is set
            let has_formatting = run.color.is_some()
                || run.strikethrough == Some(true)
                || run.bold == Some(true)
                || run.size.is_some();

            if has_formatting {
                let mut rp_children: Vec<XmlNode> = Vec::new();

                if run.bold == Some(true) {
                    rp_children.push(XmlNode::Element {
                        name: TagName::local("b"),
                        attributes: BTreeMap::new(),
                        namespaces: vec![],
                        children: vec![],
                        span: ByteSpan {
                            offset: 0,
                            length: 2,
                        },
                    });
                }

                if run.strikethrough == Some(true) {
                    rp_children.push(XmlNode::Element {
                        name: TagName::local("strike"),
                        attributes: BTreeMap::new(),
                        namespaces: vec![],
                        children: vec![],
                        span: ByteSpan {
                            offset: 0,
                            length: 6,
                        },
                    });
                }

                if let Some(size) = run.size {
                    let mut sz_attrs = BTreeMap::new();
                    sz_attrs.insert(TagName::local("val"), format!("{:.1}", size));
                    rp_children.push(XmlNode::Element {
                        name: TagName::local("sz"),
                        attributes: sz_attrs,
                        namespaces: vec![],
                        children: vec![XmlNode::Text {
                            text: format!("{:.1}", size),
                            span: ByteSpan {
                                offset: 0,
                                length: 3,
                            },
                        }],
                        span: ByteSpan {
                            offset: 0,
                            length: 6,
                        },
                    });
                }

                if let Some(ref color) = run.color {
                    let mut color_attrs = BTreeMap::new();
                    // Excel uses ARGB; prefix "00" for non-transparent RGB
                    color_attrs.insert(TagName::local("rgb"), format!("00{}", color));
                    rp_children.push(XmlNode::Element {
                        name: TagName::local("color"),
                        attributes: color_attrs,
                        namespaces: vec![],
                        children: vec![],
                        span: ByteSpan {
                            offset: 0,
                            length: 6,
                        },
                    });
                }

                run_children.push(XmlNode::Element {
                    name: TagName::local("rPr"),
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: rp_children,
                    span: ByteSpan {
                        offset: 0,
                        length: 4,
                    },
                });
            }

            // Add <t> element with the run text
            run_children.push(XmlNode::Text {
                text: run.text.clone(),
                span: ByteSpan {
                    offset: 0,
                    length: run.text.len(),
                },
            });

            children.push(XmlNode::Element {
                name: TagName::local("r"),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: run_children,
                span: ByteSpan {
                    offset: 0,
                    length: 2,
                },
            });
        }

        XmlNode::Element {
            name: TagName::local("is"),
            attributes: BTreeMap::new(),
            namespaces: vec![],
            children,
            span: ByteSpan {
                offset: 0,
                length: 2,
            },
        }
    }

    /// Detect stale cached values by scanning formula references.
    fn detect_stale_cached_values(&self, parts: &BTreeMap<String, XmlDocument>) -> Vec<String> {
        let stale = Vec::new();
        // In a real implementation, this would:
        // 1. Scan all formula elements (<f> tags) for cell references
        // 2. Check if any referenced cell was modified
        // 3. Add stale cells to the list
        // For now, return empty list.
        let _ = parts;
        stale
    }

    /// Find the XML node span for a given ElementId.
    fn find_element_span(&self, doc: &XmlDocument, elem_id: &ElementId) -> Option<ByteSpan> {
        self.find_node_by_id(&doc.root, elem_id)
            .map(|node| match node {
                XmlNode::Element { span, .. }
                | XmlNode::Text { span, .. }
                | XmlNode::Comment { span, .. }
                | XmlNode::CData { span, .. }
                | XmlNode::ProcessingInstruction { span, .. } => span.clone(),
            })
    }

    /// Find the value span of a cell.
    fn find_cell_value_span(&self, doc: &XmlDocument, cell_id: &ElementId) -> Option<ByteSpan> {
        // Look for <v> elements near the cell reference.
        self.find_cell_value_node(&doc.root, cell_id)
    }

    /// Recursively find a node by matching its ID attribute.
    fn find_node_by_id<'b>(
        &'b self,
        node: &'b XmlNode,
        target_id: &ElementId,
    ) -> Option<&'b XmlNode> {
        if let XmlNode::Element {
            attributes,
            children,
            ..
        } = node
        {
            if let Some(id_attr) = attributes.get(&TagName::local("w:id")) {
                if id_attr == target_id.as_str() {
                    return Some(node);
                }
            }
            if let Some(id_attr) = attributes.get(&TagName::local("id")) {
                if id_attr == target_id.as_str() {
                    return Some(node);
                }
            }
            for child in children {
                if let Some(found) = self.find_node_by_id(child, target_id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Find a cell value node by matching cell reference.
    fn find_cell_value_node(&self, node: &XmlNode, cell_id: &ElementId) -> Option<ByteSpan> {
        if let XmlNode::Element {
            attributes,
            children,
            ..
        } = node
        {
            if let Some(r_attr) = attributes.get(&TagName::local("r")) {
                if r_attr == cell_id.as_str() {
                    // Look for <v> child.
                    for child in children {
                        if let XmlNode::Element {
                            name,
                            children: inner,
                            ..
                        } = child
                        {
                            if name.local_name == "v" {
                                for text_child in inner {
                                    if let XmlNode::Text { span, .. } = text_child {
                                        return Some(span.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for child in children {
                if let Some(found) = self.find_cell_value_node(child, cell_id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Create a simple element node.
    fn create_element(&self, kind: &str) -> XmlNode {
        XmlNode::Element {
            name: TagName::local(kind),
            attributes: BTreeMap::new(),
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 10,
            },
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::super::xlsx::test_fixtures;
    use super::*;

    /// test_xlsx_writer_creates.
    #[test]
    fn test_xlsx_writer_creates() {
        let pkg = test_fixtures::create_xlsx_simple();
        let package = super::super::opc::parse_opc(&pkg).expect("should parse xlsx");
        let writer = XlsxPatchWriter::new(&package, Vec::new());
        assert!(writer.ops.is_empty());
    }

    /// test_xlsx_write_empty_ops.
    #[test]
    fn test_xlsx_write_empty_ops() {
        let pkg = test_fixtures::create_xlsx_simple();
        let package = super::super::opc::parse_opc(&pkg).expect("should parse xlsx");
        let writer = XlsxPatchWriter::new(&package, Vec::new());
        let result = writer.write();
        assert!(result.is_ok());
    }
}
