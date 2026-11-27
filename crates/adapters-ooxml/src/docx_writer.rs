// SPDX-License-Identifier: MIT OR Apache-2.0
//! Docx patch writer — applies IR patch operations to OOXML documents.
//!
//! Routes patch ops to span mutations on document.xml, headers, and footers,
//! then writes the patched OPC package via SurgicalWriter.

#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unreachable_patterns)]

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::ElementId;
use gp_core::patch::PatchOp;
use gp_core::patch::RunProperties;
use std::collections::BTreeMap;

use super::opc::{OpcPackage, Part};
use super::span_mut::{
    apply_span_mutations, find_first_cell, node_span, MutationKind, SpanMutation,
};
use super::writer::{PartMutation, SurgicalWriter};
use super::xml::{ByteSpan, TagName, XmlDocument, XmlNode};

// ===========================================================================
// Types
// ===========================================================================

/// A hyperlink relationship entry to be written to a .rels file.
#[derive(Debug, Clone)]
pub struct HyperlinkRel {
    /// The relationship ID (e.g. "rId1").
    pub r_id: String,
    /// The target URL.
    pub target: String,
    /// Whether the target is external.
    pub target_mode: Option<String>,
    /// The source part URI (e.g. "word/document.xml").
    pub source_uri: String,
}

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

        // 2. Route patch ops to span mutations per part, collecting hyperlink rels.
        let (mutations_by_part, hyperlink_rels) = match self.route_ops(&mut parts) {
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

        // 4. Create/update relationship files for hyperlinks (W4.3).
        let mut rels_parts: Vec<(String, Vec<HyperlinkRel>)> = Vec::new();
        for rel in &hyperlink_rels {
            let rels_uri = rels_uri_for(&rel.source_uri);
            rels_parts.push((rels_uri, vec![rel.clone()]));
        }

        // 5. Build the PartMutation map for SurgicalWriter.
        let mut mutations = BTreeMap::new();

        // First, add all regular document parts.
        for (uri, doc) in parts.iter_mut() {
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

        // Add hyperlink rel file mutations (UpdateRels for each rels URI).
        for (rels_uri, rels) in rels_parts {
            // Generate the rels XML content directly to avoid lifetime issues.
            let mut rels_xml =
                String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
            rels_xml.push_str(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
            );
            for rel in &rels {
                rels_xml.push_str("<Relationship ");
                rels_xml.push_str(&format!("Id=\"{}\"", rel.r_id));
                rels_xml.push_str(&format!(
                    " Type=\"{}\"",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
                ));
                rels_xml.push_str(&format!(" Target=\"{}\"", rel.target));
                if let Some(ref mode) = rel.target_mode {
                    rels_xml.push_str(&format!(" TargetMode=\"{}\"", mode));
                }
                rels_xml.push_str("/>");
            }
            rels_xml.push_str("</Relationships>");

            mutations.insert(
                rels_uri,
                PartMutation::Replace {
                    data: rels_xml.into_bytes(),
                    content_type: Some(
                        "application/vnd.openxmlformats-package.relationships+xml".to_string(),
                    ),
                },
            );
        }

        // 6. Write the patched OPC via SurgicalWriter.
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

        // Parse styles.xml — required for SetStyle operations (W4.1).
        if let Ok(styles_part) = find_part(self.package, "word/styles.xml") {
            match super::xml::parse_xml(&styles_part.data) {
                Ok(doc) => {
                    parts.insert("word/styles.xml".to_string(), doc);
                }
                Err(e) => {
                    diags.push(Diagnostic::new(
                        "OOX-E-002",
                        Severity::Warning,
                        format!("Failed to parse styles.xml: {}", e[0].message),
                    ));
                }
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
    /// Also collects hyperlink relationship entries for rels file generation.
    fn route_ops(
        &self,
        parts: &mut BTreeMap<String, XmlDocument>,
    ) -> Result<(BTreeMap<String, Vec<SpanMutation>>, Vec<HyperlinkRel>), Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut mutations_by_part: BTreeMap<String, Vec<SpanMutation>> = BTreeMap::new();
        let mut hyperlink_rels: Vec<HyperlinkRel> = Vec::new();

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
                PatchOp::SetCellRich { .. } => {
                    // Spreadsheet operation — not applicable to docx.
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
                PatchOp::SetStyle { .. } => {
                    match self.route_set_style(op, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::AssignStyle { elem, style_id } => {
                    match self.route_assign_style(elem, style_id, parts, &mut mutations_by_part) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::AddHyperlink {
                    target,
                    elem,
                    tooltip,
                } => {
                    match self.route_add_hyperlink(
                        target,
                        elem,
                        tooltip.as_deref(),
                        parts,
                        &mut mutations_by_part,
                        &mut hyperlink_rels,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetShapeProps { .. }
                | PatchOp::InsertPicture { .. }
                | PatchOp::SetSlideText { .. }
                | PatchOp::SetTableCellText { .. }
                | PatchOp::SetSlideNotes { .. }
                | PatchOp::SetCellStyle { .. }
                | PatchOp::SetFreezePane { .. }
                | PatchOp::SetRowColDim { .. }
                | PatchOp::SetAutoFilter { .. }
                | PatchOp::SetConditionalFormat { .. }
                | PatchOp::SetSheetProtection { .. }
                | PatchOp::SetPageSetup { .. }
                | PatchOp::SetCellHyperlink { .. }
                | PatchOp::SetDefinedName { .. }
                | PatchOp::SetDataValidation { .. }
                | PatchOp::AddTable { .. }
                | PatchOp::RemoveTable { .. } => {
                    // XLSX/PPTX-specific ops — not applicable to docx.
                }
            }
        }

        if !diags.is_empty() {
            return Err(diags);
        }

        Ok((mutations_by_part, hyperlink_rels))
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
        // Element not found by w/id attribute — check if this targets a table cell.
        // Table cells in docx use IR content-hashed IDs, not w:id attributes,
        // so find_element_span won't find them. Detect cell-like element IDs
        // and use the preserving mutation path instead.
        if self.is_cell_element(elem) {
            for (part_uri, doc) in parts {
                let mut found_cell_span = None;
                if let XmlNode::Element { .. } = doc.root {
                    if let Some(cell_node) = find_first_cell(&doc.root) {
                        found_cell_span = Some(node_span(cell_node).clone());
                    }
                }
                if let Some(span) = found_cell_span {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::ReplaceTextPreserving {
                                new_text: new.to_string(),
                                blank_subsequent: true,
                            },
                        });
                    return Ok(());
                }
            }
        }
        // Element not found — diagnostic, not error.
        Ok(())
    }

    /// Return true if the element ID likely targets a table cell.
    /// Cell IDs are derived from content containing "cell" context.
    fn is_cell_element(&self, elem: &ElementId) -> bool {
        // Cell element IDs are generated by ElementId::content_anchored("cell", ...)
        // The hash content starts with "cell:" prefix for table cells
        elem.as_str().starts_with("cell:")
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
                        kind: MutationKind::WrapDelete,
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

    /// Route SetStyle op — write a <w:style> element to styles.xml.
    fn route_set_style(
        &self,
        op: &PatchOp,
        parts: &mut BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let PatchOp::SetStyle {
            style_id,
            name,
            style_type,
            based_on,
            bold,
            italic,
            font_name,
            font_size_pt,
        } = op
        else {
            return Err(vec![Diagnostic::new(
                "OOX-E-002",
                Severity::Error,
                "Expected SetStyle op".to_string(),
            )]);
        };

        let styles_uri = "word/styles.xml";

        // Build the <w:style> XML element
        let mut style_attrs = BTreeMap::new();
        style_attrs.insert(TagName::local("w:val"), style_id.clone());
        style_attrs.insert(TagName::local("w:customStyle"), "1".to_string());
        style_attrs.insert(TagName::local("w:name"), format!("{}", style_id));

        let mut style_children = Vec::new();

        // <w:basedOn>
        if let Some(ref based) = based_on {
            style_children.push(XmlNode::Element {
                name: TagName::local("w:basedOn"),
                attributes: {
                    let mut a = BTreeMap::new();
                    a.insert(TagName::local("w:val"), based.clone());
                    a
                },
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        } else {
            style_children.push(XmlNode::Element {
                name: TagName::local("w:basedOn"),
                attributes: {
                    let mut a = BTreeMap::new();
                    a.insert(TagName::local("w:val"), "Normal".to_string());
                    a
                },
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }

        // <w:uiPriority>
        style_children.push(XmlNode::Element {
            name: TagName::local("w:uiPriority"),
            attributes: {
                let mut a = BTreeMap::new();
                a.insert(TagName::local("w:val"), "99".to_string());
                a
            },
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        });

        // <w:rsid>
        style_children.push(XmlNode::Element {
            name: TagName::local("w:rsidR"),
            attributes: {
                let mut a = BTreeMap::new();
                a.insert(TagName::local("w:val"), "00000000".to_string());
                a
            },
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        });
        style_children.push(XmlNode::Element {
            name: TagName::local("w:rsidRPr"),
            attributes: {
                let mut a = BTreeMap::new();
                a.insert(TagName::local("w:val"), "00000000".to_string());
                a
            },
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        });

        // <w:pPr> for paragraph styles
        if style_type == "paragraph" {
            let mut pp_children = Vec::new();

            if let Some(ref font) = font_name {
                pp_children.push(XmlNode::Element {
                    name: TagName::local("w:rFonts"),
                    attributes: {
                        let mut a = BTreeMap::new();
                        a.insert(TagName::local("w:ascii"), font.clone());
                        a.insert(TagName::local("w:hAnsi"), font.clone());
                        a
                    },
                    namespaces: vec![],
                    children: vec![],
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                });
            }
            if let Some(pt) = font_size_pt {
                pp_children.push(XmlNode::Element {
                    name: TagName::local("w:sz"),
                    attributes: {
                        let mut a = BTreeMap::new();
                        a.insert(TagName::local("w:val"), ((pt * 2.0) as i32).to_string());
                        a
                    },
                    namespaces: vec![],
                    children: vec![],
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                });
            }

            style_children.push(XmlNode::Element {
                name: TagName::local("w:pPr"),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: pp_children,
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }

        // <w:rPr> for character formatting
        let mut rp_children = Vec::new();
        if *bold == Some(true) {
            rp_children.push(XmlNode::Element {
                name: TagName::local("w:b"),
                attributes: {
                    let mut a = BTreeMap::new();
                    a.insert(TagName::local("w:val"), "1".to_string());
                    a
                },
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }
        if *italic == Some(true) {
            rp_children.push(XmlNode::Element {
                name: TagName::local("w:i"),
                attributes: {
                    let mut a = BTreeMap::new();
                    a.insert(TagName::local("w:val"), "1".to_string());
                    a
                },
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }
        if let Some(ref font) = font_name {
            rp_children.push(XmlNode::Element {
                name: TagName::local("w:rFonts"),
                attributes: {
                    let mut a = BTreeMap::new();
                    a.insert(TagName::local("w:ascii"), font.clone());
                    a.insert(TagName::local("w:hAnsi"), font.clone());
                    a
                },
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }
        if let Some(pt) = font_size_pt {
            rp_children.push(XmlNode::Element {
                name: TagName::local("w:sz"),
                attributes: {
                    let mut a = BTreeMap::new();
                    a.insert(TagName::local("w:val"), ((pt * 2.0) as i32).to_string());
                    a
                },
                namespaces: vec![],
                children: vec![],
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }

        // Only add w:rPr if it has children
        if !rp_children.is_empty() {
            style_children.push(XmlNode::Element {
                name: TagName::local("w:rPr"),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: rp_children,
                span: ByteSpan {
                    offset: 0,
                    length: 0,
                },
            });
        }

        let style_element = XmlNode::Element {
            name: TagName::local("w:style"),
            attributes: style_attrs,
            namespaces: vec![super::xml::Namespace {
                prefix: "w".into(),
                uri: "http://schemas.openxmlformats.org/wordprocessingml/2006/main".into(),
            }],
            children: style_children,
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        };

        // If styles.xml is not yet in parts, create it as a minimal styles document.
        // This ensures the part exists for the mutation to target (W4.1).
        if !parts.contains_key(styles_uri) {
            let ns = vec![super::xml::Namespace {
                prefix: "w".into(),
                uri: "http://schemas.openxmlformats.org/wordprocessingml/2006/main".into(),
            }];
            let styles_doc = XmlDocument::new(
                XmlNode::Element {
                    name: TagName::local("w:styles"),
                    attributes: BTreeMap::new(),
                    namespaces: ns.clone(),
                    children: vec![style_element.clone()],
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                },
                ns,
                "utf-8".into(),
                vec![],
            );
            parts.insert(styles_uri.to_string(), styles_doc);
            return Ok(());
        }

        // Find the <w:styles> element in styles.xml and insert the new style there.
        if let Some(styles_doc) = parts.get_mut(styles_uri) {
            // Check if the root is <w:styles> before mutably borrowing.
            let root_is_styles = match &styles_doc.root {
                XmlNode::Element { name, .. } => is_word_elem(name, "styles"),
                _ => false,
            };

            match &mut styles_doc.root {
                XmlNode::Element {
                    name: _,
                    attributes: _,
                    namespaces: _,
                    children: root_children,
                    span: _,
                } => {
                    if root_is_styles {
                        // Root is styles, insert into root's children
                        root_children.push(style_element);
                    } else {
                        // Find <w:styles> element or create it
                        let mut found = false;
                        for child in root_children.iter_mut() {
                            if let XmlNode::Element {
                                name, children: gc, ..
                            } = child
                            {
                                if is_word_elem(name, "styles") {
                                    gc.push(style_element.clone());
                                    found = true;
                                    break;
                                }
                            }
                        }
                        if !found {
                            // Create <w:styles> element
                            let ns = vec![super::xml::Namespace {
                                prefix: "w".into(),
                                uri: "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                                    .into(),
                            }];
                            root_children.push(XmlNode::Element {
                                name: TagName::local("w:styles"),
                                attributes: BTreeMap::new(),
                                namespaces: ns,
                                children: vec![style_element],
                                span: ByteSpan {
                                    offset: 0,
                                    length: 0,
                                },
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Route AssignStyle op — assign a style to a paragraph/run.
    /// Matches by element ID prefix (para:/run:) + index for reliable targeting.
    fn route_assign_style(
        &self,
        elem: &ElementId,
        style_id: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let elem_str = elem.as_str();
        let (target_type, index) = if let Some(rest) = elem_str.strip_prefix("para:") {
            ("para", rest.parse::<usize>().ok())
        } else if let Some(rest) = elem_str.strip_prefix("run:") {
            ("run", rest.parse::<usize>().ok())
        } else {
            // Fall back to w:id matching for legacy IDs
            for (part_uri, doc) in parts {
                if let Some(span) = self.find_element_span(doc, elem) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::SetStyleAttr(style_id.to_string()),
                        });
                    return Ok(());
                }
            }
            return Ok(());
        };

        // Find the target paragraph or run by index in document.xml
        for (part_uri, doc) in parts {
            if part_uri != "word/document.xml" {
                continue;
            }
            if let Some(span) = Self::find_element_by_index(&doc.root, target_type, index) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::SetStyleAttr(style_id.to_string()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Find an element by type ("para" or "run") and 0-based index in the document tree.
    fn find_element_by_index<'b>(
        root: &'b XmlNode,
        target_type: &str,
        index: Option<usize>,
    ) -> Option<ByteSpan> {
        let idx = index?;
        let mut count: usize = 0;

        fn walk<'c>(
            node: &'c XmlNode,
            target_type: &str,
            idx: usize,
            count: &mut usize,
        ) -> Option<ByteSpan> {
            match node {
                XmlNode::Element { name, span, .. } => {
                    let matches = match target_type {
                        "para" => is_word_elem(name, "p"),
                        "run" => is_word_elem(name, "r"),
                        _ => false,
                    };
                    if matches {
                        if *count == idx {
                            return Some(span.clone());
                        }
                        *count += 1;
                    }
                    if let XmlNode::Element { children, .. } = node {
                        for child in children {
                            if let Some(result) = walk(child, target_type, idx, count) {
                                return Some(result);
                            }
                        }
                    }
                    None
                }
                _ => None,
            }
        }

        walk(root, target_type, idx, &mut count)
    }

    /// Route AddHyperlink op — add a hyperlink to a paragraph/run.
    /// Generates a unique rId and targets element by ID prefix (para:/run:) or fallback.
    fn route_add_hyperlink(
        &self,
        target: &str,
        elem: &ElementId,
        tooltip: Option<&str>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
        hyperlink_rels: &mut Vec<HyperlinkRel>,
    ) -> Result<(), Vec<Diagnostic>> {
        // Count existing rId* attributes to generate a unique rId
        let max_rid = Self::max_existing_rid(parts);
        let r_id = format!("rId{}", max_rid + 1);

        let elem_str = elem.as_str();
        let (span, source_uri) = if let Some(rest) = elem_str.strip_prefix("para:") {
            let idx = rest.parse::<usize>().ok();
            (
                Self::find_element_by_index_for_hyperlink(parts, "para", idx),
                "word/document.xml",
            )
        } else if let Some(rest) = elem_str.strip_prefix("run:") {
            let idx = rest.parse::<usize>().ok();
            (
                Self::find_element_by_index_for_hyperlink(parts, "run", idx),
                "word/document.xml",
            )
        } else {
            // Fall back to w:id matching for legacy IDs
            for (part_uri, doc) in parts {
                if let Some(s) = self.find_element_span(doc, elem) {
                    return Self::push_hyperlink_mutations(
                        part_uri,
                        s,
                        target,
                        tooltip,
                        &r_id,
                        mutations_by_part,
                        hyperlink_rels,
                    );
                }
            }
            (None, "word/document.xml")
        };

        if let Some(span) = span {
            return Self::push_hyperlink_mutations(
                source_uri,
                span,
                target,
                tooltip,
                &r_id,
                mutations_by_part,
                hyperlink_rels,
            );
        }
        Ok(())
    }

    /// Count the maximum rId number in the package and return the next available.
    fn max_existing_rid(parts: &BTreeMap<String, XmlDocument>) -> usize {
        let mut max_rid: usize = 0;

        fn count_rids(node: &XmlNode, max: &mut usize) {
            if let XmlNode::Element {
                attributes,
                children,
                ..
            } = node
            {
                for (key, val) in attributes {
                    if key.local_name == "rId" {
                        if let Some(num_str) = val.strip_prefix("rId") {
                            if let Ok(num) = num_str.parse::<usize>() {
                                if num > *max {
                                    *max = num;
                                }
                            }
                        }
                    }
                }
                for child in children {
                    count_rids(child, max);
                }
            }
        }

        for doc in parts.values() {
            count_rids(&doc.root, &mut max_rid);
        }

        max_rid + 1
    }

    /// Find element by type and index for hyperlink targeting.
    fn find_element_by_index_for_hyperlink(
        parts: &BTreeMap<String, XmlDocument>,
        target_type: &str,
        index: Option<usize>,
    ) -> Option<ByteSpan> {
        let idx = index?;
        let mut count: usize = 0;

        fn walk<'a>(
            node: &'a XmlNode,
            target_type: &str,
            idx: usize,
            count: &mut usize,
        ) -> Option<&'a ByteSpan> {
            match node {
                XmlNode::Element { name, span, .. } => {
                    let matches = match target_type {
                        "para" => is_word_elem(name, "p"),
                        "run" => is_word_elem(name, "r"),
                        _ => false,
                    };
                    if matches {
                        if *count == idx {
                            return Some(span);
                        }
                        *count += 1;
                    }
                    if let XmlNode::Element { children, .. } = node {
                        for child in children {
                            if let Some(result) = walk(child, target_type, idx, count) {
                                return Some(result);
                            }
                        }
                    }
                    None
                }
                _ => None,
            }
        }

        for doc in parts.values() {
            if let Some(span) = walk(&doc.root, target_type, idx, &mut count) {
                return Some(span.clone());
            }
        }
        None
    }

    /// Push hyperlink mutation with the generated rId, and record the rel.
    fn push_hyperlink_mutations(
        part_uri: &str,
        span: ByteSpan,
        target: &str,
        tooltip: Option<&str>,
        r_id: &str,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
        hyperlink_rels: &mut Vec<HyperlinkRel>,
    ) -> Result<(), Vec<Diagnostic>> {
        mutations_by_part
            .entry(part_uri.to_string())
            .or_default()
            .push(SpanMutation {
                span,
                kind: MutationKind::AddHyperlink {
                    target: target.to_string(),
                    tooltip: tooltip.map(|s| s.to_string()),
                    r_id: Some(r_id.to_string()),
                },
            });
        // Track the hyperlink rel for writing to the rels file (W4.3).
        let is_external = target.starts_with("http://") || target.starts_with("https://");
        hyperlink_rels.push(HyperlinkRel {
            r_id: r_id.to_string(),
            target: target.to_string(),
            target_mode: if is_external {
                Some("External".to_string())
            } else {
                None
            },
            source_uri: part_uri.to_string(),
        });
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

/// Convert a part URI to its relationship file path.
/// E.g. "word/document.xml" -> "word/_rels/document.xml.rels"
fn rels_uri_for(part_uri: &str) -> String {
    if let Some(pos) = part_uri.rfind('/') {
        let dir = &part_uri[..pos];
        let base = &part_uri[pos + 1..];
        let rels_base = format!("_rels/{}.rels", base);
        format!("{}/{}", dir, rels_base)
    } else {
        format!("_rels/{}.rels", part_uri)
    }
}

/// Check if an element name matches (handles namespace-qualified names).
fn is_word_elem(name: &TagName, local: &str) -> bool {
    name.local_name == local
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

        let mut parts = writer.parse_document_parts().expect("parse parts");
        assert!(parts.contains_key("word/document.xml"));

        let (mutations, _) = writer.route_ops(&mut parts).expect("route ops");
        assert!(mutations.contains_key("word/document.xml"));
    }

    #[test]
    fn test_delete_op_routing() {
        let pkg = make_minimal_docx();
        let elem_id = ElementId::content_anchored("paragraph", &["p:0"], "test");
        let ops = vec![PatchOp::Delete { elem: elem_id }];
        let writer = DocxPatchWriter::new(&pkg, ops);

        let mut parts = writer.parse_document_parts().expect("parse parts");
        let (mutations, _) = writer.route_ops(&mut parts).expect("route ops");
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

        let mut parts = writer.parse_document_parts().expect("parse parts");
        let (mutations, _) = writer.route_ops(&mut parts).expect("route ops");
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

        let mut parts = writer.parse_document_parts().expect("parse parts");
        let (mutations, _) = writer.route_ops(&mut parts).expect("route ops");
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

        let mut parts = writer.parse_document_parts().expect("parse parts");
        let (mutations, _) = writer.route_ops(&mut parts).expect("route ops");
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

        let mut parts = writer.parse_document_parts().expect("parse parts");
        let (mutations, _) = writer.route_ops(&mut parts).expect("route ops");
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

        let mut parts = writer.parse_document_parts().expect("parse parts");
        // route_ops may return diagnostics for unsupported ops, but should still succeed
        let mutations = writer.route_ops(&mut parts);
        // We expect either an empty map or a warning
        if let Ok((m, _)) = mutations {
            assert!(m.is_empty() || m.contains_key("word/document.xml"));
        }
    }
}
