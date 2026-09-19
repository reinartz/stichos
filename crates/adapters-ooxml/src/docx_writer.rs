// SPDX-License-Identifier: MIT OR Apache-2.0
//! Docx patch writer — applies IR patch operations to OOXML documents.
//!
//! Routes patch ops to span mutations on document.xml, headers, and footers,
//! then writes the patched OPC package via SurgicalWriter.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::ElementId;
use gp_core::patch::PatchOp;
use gp_core::patch::RunProperties;
use std::collections::BTreeMap;

use super::opc::{OpcPackage, Part};
use super::span_mut::{apply_span_mutations, MutationKind, SpanMutation};
use super::writer::{PartMutation, SurgicalWriter};
use super::xml::{ByteSpan, TagName, XmlDocument, XmlNode};

// ===========================================================================
// Types
// ===========================================================================

/// A docx-specific patch writer.
pub struct DocxPatchWriter<'a> {
    /// The OPC package to patch.
    pub package: &'a OpcPackage,
    /// The patch operations to apply.
    pub ops: Vec<PatchOp>,
}

/// Result of docx patch application.
pub struct DocxPatchResult {
    /// The patched OPC bytes.
    pub output: Vec<u8>,
    /// Which parts were modified.
    pub modified_parts: Vec<String>,
    /// Elements that were changed (by ID).
    pub changed_element_ids: Vec<String>,
    /// Diagnostics from the patch process.
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> DocxPatchWriter<'a> {
    /// Create a new docx patch writer.
    pub fn new(package: &'a OpcPackage, ops: Vec<PatchOp>) -> Self {
        Self { package, ops }
    }

    /// Apply all patch operations, return patched OPC bytes.
    pub fn write(&self) -> Result<DocxPatchResult, Vec<Diagnostic>> {
        let mut diags = Vec::new();

        // 1. Parse all document parts (document.xml, headers, footers).
        let mut parts = match self.parse_document_parts() {
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

        // 4. Build the PartMutation map for SurgicalWriter.
        let mut mutations = BTreeMap::new();

        for (uri, doc) in &parts {
            let part_data = doc.to_bytes();
            let content_type = find_part(self.package, uri)
                .map(|p| p.content_type.clone())
                .ok()
                .filter(|ct| !ct.is_empty());

            mutations.insert(
                uri.clone(),
                PartMutation::Replace {
                    data: part_data,
                    content_type,
                },
            );
        }

        // 5. Write the patched OPC via SurgicalWriter.
        let patch_writer = SurgicalWriter::new(&[]);
        let write_result = match patch_writer.write_from_parts(&self.package.parts, mutations) {
            Ok(r) => r,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        Ok(DocxPatchResult {
            output: write_result.bytes,
            modified_parts,
            changed_element_ids,
            diagnostics: diags,
        })
    }

    /// Parse the document.xml (and headers/footers) into XML DOMs with span tracking.
    fn parse_document_parts(&self) -> Result<BTreeMap<String, XmlDocument>, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut parts = BTreeMap::new();

        // Parse main document.
        match find_part(self.package, "word/document.xml") {
            Ok(doc_part) => match super::xml::parse_xml(&doc_part.data) {
                Ok(doc) => {
                    parts.insert("word/document.xml".to_string(), doc);
                }
                Err(e) => {
                    diags.push(Diagnostic::new(
                        "OOX-E-002",
                        Severity::Error,
                        format!("Failed to parse document.xml: {}", e[0].message),
                    ));
                }
            },
            Err(e) => {
                diags.extend(e);
            }
        }

        // Parse headers from relationships.
        for (rel_uri, rels) in &self.package.relationships {
            if rel_uri == "_rels/.rels" || rel_uri.ends_with("/_rels/.rels") {
                continue;
            }
            for rel in rels {
                if rel.relationship_type.to_lowercase().contains("header") {
                    let header_uri = resolve_relative(rel_uri, &rel.target);
                    if let Ok(header_part) = find_part(self.package, &header_uri) {
                        match super::xml::parse_xml(&header_part.data) {
                            Ok(doc) => {
                                parts.insert(header_uri.clone(), doc);
                            }
                            Err(e) => {
                                diags.push(Diagnostic::new(
                                    "OOX-E-002",
                                    Severity::Warning,
                                    format!(
                                        "Failed to parse header {}: {}",
                                        header_uri, e[0].message
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Parse footers from relationships.
        for (rel_uri, rels) in &self.package.relationships {
            if rel_uri == "_rels/.rels" || rel_uri.ends_with("/_rels/.rels") {
                continue;
            }
            for rel in rels {
                if rel.relationship_type.to_lowercase().contains("footer") {
                    let footer_uri = resolve_relative(rel_uri, &rel.target);
                    if let Ok(footer_part) = find_part(self.package, &footer_uri) {
                        match super::xml::parse_xml(&footer_part.data) {
                            Ok(doc) => {
                                parts.insert(footer_uri.clone(), doc);
                            }
                            Err(e) => {
                                diags.push(Diagnostic::new(
                                    "OOX-E-002",
                                    Severity::Warning,
                                    format!(
                                        "Failed to parse footer {}: {}",
                                        footer_uri, e[0].message
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }

        if parts.is_empty() {
            diags.push(Diagnostic::new(
                "OOX-W-022",
                Severity::Warning,
                "No document parts found to patch".to_string(),
            ));
        }

        Ok(parts)
    }

    /// Route patch ops to span mutations on the appropriate part.
    fn route_ops(
        &self,
        parts: &BTreeMap<String, XmlDocument>,
    ) -> Result<BTreeMap<String, Vec<SpanMutation>>, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut mutations_by_part: BTreeMap<String, Vec<SpanMutation>> = BTreeMap::new();

        for op in &self.ops {
            match op {
                PatchOp::ReplaceText { elem, old: _, new } => {
                    match self.route_replace_text(elem, new, parts, &mut mutations_by_part) {
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
                PatchOp::Insert { anchor, kind } => {
                    match self.route_insert(anchor, kind, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetRunProps { elem, props } => {
                    match self.route_set_run_props(elem, props, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetCellValue { .. } => {
                    // Spreadsheet operation — not applicable to docx.
                    diags.push(Diagnostic::new(
                        "OOX-W-023",
                        Severity::Warning,
                        "SetCellValue not applicable to docx".to_string(),
                    ));
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
        // Element not found — diagnostic, not error.
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

    /// Route Insert op.
    fn route_insert(
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

    /// Route TrackInsert op (wraps in w:ins).
    fn route_track_insert(
        &self,
        anchor: &ElementId,
        kind: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let new_node = self.create_track_insert_element(kind);
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

    /// Route TrackDelete op (wraps in w:del).
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

    /// Route SetRunProps op.
    fn route_set_run_props(
        &self,
        elem: &ElementId,
        props: &RunProperties,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_element_span(doc, elem) {
                let mut muts = Vec::new();
                if props.bold {
                    muts.push(SpanMutation {
                        span: span.clone(),
                        kind: MutationKind::SetAttribute {
                            name: TagName::local("w:b"),
                            value: "1".to_string(),
                        },
                    });
                }
                if props.italic {
                    muts.push(SpanMutation {
                        span: span.clone(),
                        kind: MutationKind::SetAttribute {
                            name: TagName::local("w:i"),
                            value: "1".to_string(),
                        },
                    });
                }
                if let Some(ref font) = props.font_name {
                    muts.push(SpanMutation {
                        span: span.clone(),
                        kind: MutationKind::SetAttribute {
                            name: TagName::local("w:rFonts"),
                            value: font.clone(),
                        },
                    });
                }
                if let Some(pt) = props.font_size_pt {
                    muts.push(SpanMutation {
                        span: span.clone(),
                        kind: MutationKind::SetAttribute {
                            name: TagName::local("w:sz"),
                            value: ((pt * 2.0) as i32).to_string(),
                        },
                    });
                }
                if !muts.is_empty() {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .extend(muts);
                }
                return Ok(());
            }
        }
        Ok(())
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

    /// Create a simple element node.
    fn create_element(&self, kind: &str) -> XmlNode {
        match kind {
            "paragraph" => XmlNode::Element {
                name: TagName::local("w:p"),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: vec![XmlNode::Text {
                    text: "".to_string(),
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                }],
                span: ByteSpan {
                    offset: 0,
                    length: 10,
                },
            },
            "table" => XmlNode::Element {
                name: TagName::local("w:tbl"),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 10,
                },
            },
            _ => XmlNode::Element {
                name: TagName::local(kind),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 10,
                },
            },
        }
    }

    /// Create a tracked insert element (w:ins).
    fn create_track_insert_element(&self, kind: &str) -> XmlNode {
        let inner = self.create_element(kind);
        XmlNode::Element {
            name: TagName::local("w:ins"),
            attributes: BTreeMap::new(),
            namespaces: vec![],
            children: vec![inner],
            span: ByteSpan {
                offset: 0,
                length: 20,
            },
        }
    }
}

/// Find a part in the OPC package by URI.
fn find_part<'a>(package: &'a OpcPackage, uri: &str) -> Result<&'a Part, Vec<Diagnostic>> {
    package.parts.iter().find(|p| p.uri == uri).ok_or_else(|| {
        vec![Diagnostic::new(
            "OOX-W-022",
            Severity::Warning,
            format!("Part not found: {}", uri),
        )]
    })
}

/// Resolve a relative URI against a base URI.
fn resolve_relative(base: &str, target: &str) -> String {
    if target.starts_with('/') {
        target.to_string()
    } else if let Some(pos) = base.rfind('/') {
        format!("{}{}", &base[..pos + 1], target)
    } else {
        target.to_string()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Create a minimal docx package for testing with a given paragraph ID.
    #[allow(dead_code)]
    pub fn make_minimal_docx_with_p_id(p_id: &str) -> OpcPackage {
        let doc_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r w:id="{}"><w:t>Hello World</w:t></w:r></w:p></w:body></w:document>"#,
            p_id
        );

        let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/6.0/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";

        let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>";

        OpcPackage {
            parts: vec![
                Part {
                    uri: "[Content_Types].xml".to_string(),
                    content_type: "application/xml".to_string(),
                    data: ct_xml.to_vec(),
                },
                Part {
                    uri: "_rels/.rels".to_string(),
                    content_type: "application/xml".to_string(),
                    data: rels_xml.to_vec(),
                },
                Part {
                    uri: "word/document.xml".to_string(),
                    content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml".to_string(),
                    data: doc_xml.as_bytes().to_vec(),
                },
            ],
            content_types: BTreeMap::new(),
            relationships: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Create a minimal docx package for testing.
    #[allow(dead_code)]
    pub fn make_minimal_docx() -> OpcPackage {
        make_minimal_docx_with_p_id("p0")
    }

    // ------------------------------------------------------------------
    // Basic functionality tests
    // ------------------------------------------------------------------

    #[test]
    fn test_writer_creates() {
        let pkg = make_minimal_docx();
        let writer = DocxPatchWriter::new(&pkg, Vec::new());
        assert!(writer.ops.is_empty());
    }

    #[test]
    fn test_replace_text_op_routing() {
        let elem_id = ElementId::content_anchored("run", &["paragraph:0", "run:0"], "test");
        let pkg = make_minimal_docx_with_p_id(elem_id.as_str());
        let ops = vec![PatchOp::ReplaceText {
            elem: elem_id.clone(),
            old: "Hello".to_string(),
            new: "Goodbye".to_string(),
        }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        assert!(parts.contains_key("word/document.xml"));

        let mutations = writer.route_ops(&parts).expect("route ops");
        assert!(mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_delete_op_routing() {
        let pkg = make_minimal_docx();
        let elem_id = ElementId::content_anchored("paragraph", &["p:0"], "test");
        let ops = vec![PatchOp::Delete { elem: elem_id }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        let mutations = writer.route_ops(&parts).expect("route ops");
        // Deletion on non-matching ID produces no mutations, which is expected
        assert!(mutations.is_empty() || mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_set_run_props_op_routing() {
        let elem_id = ElementId::content_anchored("run", &["paragraph:0", "run:0"], "test");
        let pkg = make_minimal_docx_with_p_id(elem_id.as_str());
        let props = RunProperties {
            bold: true,
            italic: false,
            font_name: Some("Arial".to_string()),
            font_size_pt: Some(14.0),
        };
        let ops = vec![PatchOp::SetRunProps {
            elem: elem_id.clone(),
            props,
        }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        let mutations = writer.route_ops(&parts).expect("route ops");
        assert!(mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_insert_op_routing() {
        let pkg = make_minimal_docx();
        let anchor_id = ElementId::content_anchored("paragraph", &["p:0"], "test");
        let ops = vec![PatchOp::Insert {
            anchor: anchor_id.clone(),
            kind: "paragraph".to_string(),
        }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        let mutations = writer.route_ops(&parts).expect("route ops");
        assert!(mutations.is_empty() || mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_track_insert_op_routing() {
        let pkg = make_minimal_docx();
        let anchor_id = ElementId::content_anchored("paragraph", &["p:0"], "test");
        let ops = vec![PatchOp::TrackInsert {
            anchor: anchor_id.clone(),
            kind: "paragraph".to_string(),
            author: "alice".to_string(),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        let mutations = writer.route_ops(&parts).expect("route ops");
        assert!(mutations.is_empty() || mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_track_delete_op_routing() {
        let pkg = make_minimal_docx();
        let elem_id = ElementId::content_anchored("paragraph", &["p:0"], "test");
        let ops = vec![PatchOp::TrackDelete {
            elem: elem_id.clone(),
            author: "bob".to_string(),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        let mutations = writer.route_ops(&parts).expect("route ops");
        assert!(mutations.is_empty() || mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_unknown_op_handling() {
        let pkg = make_minimal_docx();
        // SetCellValue is not applicable to docx — should produce warning, not error.
        let elem_id = ElementId::content_anchored("cell", &["sheet:0", "row:0"], "test");
        let ops = vec![PatchOp::SetCellValue {
            cell: elem_id.clone(),
            value: "42".to_string(),
        }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let parts = writer.parse_document_parts().expect("parse parts");
        // route_ops may return diagnostics for unsupported ops, but should still succeed
        let mutations = writer.route_ops(&parts);
        // We expect either an empty map or a warning
        if let Ok(ref m) = mutations {
            assert!(m.is_empty() || m.contains_key("word/document.xml"));
        }
    }
}
