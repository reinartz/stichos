// SPDX-License-Identifier: MIT OR Apache-2.0
//! PPTX patch writer — applies IR patch operations to OOXML presentations.
//!
//! Routes patch ops to span mutations on slide XML, handles ReplaceText on shape text,
//! Insert/Delete of shapes, and skips incompatible operations with diagnostics.

#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unreachable_patterns)]

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

/// A new OPC part to be added during patch application.
#[derive(Debug, Clone)]
pub struct NewPart {
    /// The URI of the new part (e.g., "ppt/media/image1.png").
    pub uri: String,
    /// The content type (e.g., "image/png").
    pub content_type: String,
    /// The raw bytes of the part.
    pub data: Vec<u8>,
}

/// A relationship update for an existing part.
#[derive(Debug, Clone)]
pub struct RelUpdate {
    /// The source part URI (e.g., "ppt/presentation.xml").
    pub source_uri: String,
    /// The relationship ID (e.g., "rId3").
    pub rel_id: String,
    /// The target part URI (e.g., "ppt/notesSlides/notesSlide1.xml").
    pub target_uri: String,
    /// The relationship type (e.g., "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide").
    pub rel_type: String,
}

/// A PPTX-specific patch writer.
pub struct PptxPatchWriter<'a> {
    /// The OPC package to patch.
    pub package: &'a OpcPackage,
    /// The patch operations to apply.
    pub ops: Vec<PatchOp>,
}

/// Collector for new parts and relationship updates discovered during routing.
pub struct NewPartsCollector {
    /// New parts to be added.
    pub new_parts: Vec<NewPart>,
    /// Relationship updates for existing parts.
    pub rel_updates: Vec<RelUpdate>,
}

impl NewPartsCollector {
    pub fn new() -> Self {
        Self {
            new_parts: Vec::new(),
            rel_updates: Vec::new(),
        }
    }
}

/// Result of PPTX patch application.
pub struct PptxPatchResult {
    /// The patched OPC bytes.
    pub output: Vec<u8>,
    /// Which parts were modified.
    pub modified_parts: Vec<String>,
    /// Elements that were changed (by ID).
    pub changed_element_ids: Vec<String>,
    /// Diagnostics from the patch process.
    pub diagnostics: Vec<Diagnostic>,
}

#[allow(clippy::collapsible_match)]
impl<'a> PptxPatchWriter<'a> {
    /// Create a new PPTX patch writer.
    pub fn new(package: &'a OpcPackage, ops: Vec<PatchOp>) -> Self {
        Self { package, ops }
    }

    /// Apply all patch operations, return patched OPC bytes.
    pub fn write(&self) -> Result<PptxPatchResult, Vec<Diagnostic>> {
        let mut diags = Vec::new();

        // 1. Parse all slide parts.
        let mut parts = match self.parse_slide_parts() {
            Ok(p) => p,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        // 2. Collect new parts and rel updates during routing.
        let mut collector = NewPartsCollector::new();

        // 3. Route patch ops to span mutations per part.
        let mutations_by_part = match self.route_ops(&parts, &mut collector) {
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

        // 4. Build PartMutation map and write via SurgicalWriter.
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

        // 5. Add new parts as PartMutation::Add.
        for new_part in &collector.new_parts {
            mutations.insert(
                new_part.uri.clone(),
                PartMutation::Add {
                    uri: new_part.uri.clone(),
                    content_type: new_part.content_type.clone(),
                    data: new_part.data.clone(),
                    rels: None,
                },
            );
        }

        // 6. Add rel updates as PartMutation::UpdateRels.
        for rel_update in &collector.rel_updates {
            mutations.insert(
                format!("{}._rels", rel_update.source_uri),
                PartMutation::UpdateRels {
                    source_uri: rel_update.source_uri.clone(),
                    rels: vec![super::writer::Relationship {
                        id: rel_update.rel_id.clone(),
                        target: rel_update.target_uri.clone(),
                        relationship_type: rel_update.rel_type.clone(),
                        target_mode: None,
                    }],
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

        Ok(PptxPatchResult {
            output: write_result.bytes,
            modified_parts,
            changed_element_ids,
            diagnostics: diags,
        })
    }

    /// Parse all slide parts from the OPC package.
    fn parse_slide_parts(&self) -> Result<BTreeMap<String, XmlDocument>, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut parts = BTreeMap::new();

        for part in &self.package.parts {
            if part.uri.starts_with("ppt/slides/") && part.uri.ends_with(".xml") {
                match super::xml::parse_xml(&part.data) {
                    Ok(doc) => {
                        parts.insert(part.uri.clone(), doc);
                    }
                    Err(e) => {
                        diags.push(Diagnostic::new(
                            "OOX-E-002",
                            Severity::Error,
                            format!("Failed to parse slide {}: {}", part.uri, e[0].message),
                        ));
                    }
                }
            }
        }

        if parts.is_empty() {
            diags.push(Diagnostic::new(
                "OOX-W-022",
                Severity::Warning,
                "No slide parts found to patch".to_string(),
            ));
        }

        Ok(parts)
    }

    /// Route patch ops to span mutations.
    fn route_ops(
        &self,
        parts: &BTreeMap<String, XmlDocument>,
        collector: &mut NewPartsCollector,
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
                PatchOp::Insert { anchor, kind } => {
                    match self.route_insert(anchor, kind, parts, &mut mutations_by_part) {
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
                PatchOp::SetRunProps { .. } => {
                    // Not applicable to pptx — skip with warning.
                    diags.push(Diagnostic::new(
                        "OOX-W-023",
                        Severity::Warning,
                        "SetRunProps not applicable to pptx".to_string(),
                    ));
                }
                PatchOp::SetCellValue { .. } => {
                    // Not applicable to pptx — skip with warning.
                    diags.push(Diagnostic::new(
                        "OOX-W-024",
                        Severity::Warning,
                        "SetCellValue not applicable to pptx".to_string(),
                    ));
                }
                PatchOp::SetCellRich { .. } => {
                    // Spreadsheet operation — not applicable to pptx.
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
                    line_width_pt,
                } => {
                    match self.route_set_shape_props(
                        slide_id,
                        shape_id,
                        *x_pt,
                        *y_pt,
                        *width_pt,
                        *height_pt,
                        *rotation,
                        fill_color.as_deref(),
                        line_color.as_deref(),
                        *line_width_pt,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::InsertPicture {
                    slide_id,
                    data,
                    mime_type,
                    alt_text,
                    x_pt,
                    y_pt,
                    width_pt,
                    height_pt,
                } => {
                    match self.route_insert_picture(
                        slide_id,
                        data,
                        mime_type,
                        alt_text.as_deref(),
                        *x_pt,
                        *y_pt,
                        *width_pt,
                        *height_pt,
                        parts,
                        &mut mutations_by_part,
                        collector,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetSlideText {
                    slide_id,
                    shape_id,
                    text,
                } => {
                    match self.route_set_slide_text(
                        slide_id,
                        shape_id,
                        text,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetTableCellText {
                    slide_id,
                    table_row,
                    table_col,
                    text,
                    bold,
                    italic,
                } => {
                    match self.route_set_table_cell_text(
                        slide_id,
                        *table_row,
                        *table_col,
                        text,
                        *bold,
                        *italic,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetSlideNotes { slide_id, notes } => {
                    match self.route_set_slide_notes(
                        slide_id,
                        notes,
                        parts,
                        &mut mutations_by_part,
                        collector,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetStyle { .. }
                | PatchOp::AssignStyle { .. }
                | PatchOp::AddHyperlink { .. }
                | PatchOp::SetCellValue { .. }
                | PatchOp::SetCellRich { .. }
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
                    // DOCX/XLSX-specific ops — not applicable to pptx.
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

    /// Route SetShapeProps op — mutate spPr/a:xfrm and fill/line elements.
    ///
    /// Finds the shape by cNvPr/@id or cNvPr/@name, then mutates:
    ///  - a:off @x/@y for position
    ///  - a:ext @cx/@cy for size
    ///  - a:xfrm @rot for rotation
    ///  - adds a:solidFill under spPr for fill
    ///  - adds a:ln under spPr for line styling
    #[allow(clippy::too_many_arguments)]
    fn route_set_shape_props(
        &self,
        slide_id: &str,
        shape_id: &str,
        x_pt: Option<f64>,
        y_pt: Option<f64>,
        width_pt: Option<f64>,
        height_pt: Option<f64>,
        rotation: Option<f64>,
        fill_color: Option<&str>,
        line_color: Option<&str>,
        line_width_pt: Option<f64>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(shape_node) = self.find_shape_node(doc, shape_id) {
                let mut shape_muts: Vec<SpanMutation> = Vec::new();

                if let XmlNode::Element { children, .. } = shape_node {
                    for child in children {
                        if let XmlNode::Element {
                            name: sp_name,
                            children: sp_children,
                            ..
                        } = child
                        {
                            if is_pptx_elem(sp_name, "spPr") || is_pptx_elem(sp_name, "grpSpPr") {
                                for sp_child in sp_children {
                                    if let XmlNode::Element {
                                        name: xfrm_name,
                                        children: xfrm_children,
                                        span: xfrm_span,
                                        ..
                                    } = sp_child
                                    {
                                        if is_pptx_elem(xfrm_name, "xfrm") {
                                            // Set x and y via a:off
                                            for xfrm_child in xfrm_children {
                                                if let XmlNode::Element {
                                                    name: off_name,
                                                    attributes: off_attrs,
                                                    span: off_span,
                                                    ..
                                                } = xfrm_child
                                                {
                                                    if is_pptx_elem(off_name, "off") {
                                                        if let Some(x) = x_pt {
                                                            let x_emu = (x * 914400.0) as i64;
                                                            shape_muts.push(SpanMutation {
                                                                span: off_span.clone(),
                                                                kind: MutationKind::SetAttribute {
                                                                    name: TagName::local("x"),
                                                                    value: x_emu.to_string(),
                                                                },
                                                            });
                                                        }
                                                        if let Some(y) = y_pt {
                                                            let y_emu = (y * 914400.0) as i64;
                                                            shape_muts.push(SpanMutation {
                                                                span: off_span.clone(),
                                                                kind: MutationKind::SetAttribute {
                                                                    name: TagName::local("y"),
                                                                    value: y_emu.to_string(),
                                                                },
                                                            });
                                                        }
                                                    }
                                                    if is_pptx_elem(off_name, "ext") {
                                                        if let Some(w) = width_pt {
                                                            let w_emu = (w * 914400.0) as i64;
                                                            shape_muts.push(SpanMutation {
                                                                span: off_span.clone(),
                                                                kind: MutationKind::SetAttribute {
                                                                    name: TagName::local("cx"),
                                                                    value: w_emu.to_string(),
                                                                },
                                                            });
                                                        }
                                                        if let Some(h) = height_pt {
                                                            let h_emu = (h * 914400.0) as i64;
                                                            shape_muts.push(SpanMutation {
                                                                span: off_span.clone(),
                                                                kind: MutationKind::SetAttribute {
                                                                    name: TagName::local("cy"),
                                                                    value: h_emu.to_string(),
                                                                },
                                                            });
                                                        }
                                                    }
                                                }
                                            }

                                            // Set rotation via @rot on a:xfrm
                                            if let Some(rot) = rotation {
                                                let rot_cdeg = (rot * 100000.0).round() as i64;
                                                shape_muts.push(SpanMutation {
                                                    span: xfrm_span.clone(),
                                                    kind: MutationKind::SetAttribute {
                                                        name: TagName::local("rot"),
                                                        value: rot_cdeg.to_string(),
                                                    },
                                                });
                                            }

                                            // Set fill via a:solidFill child of spPr
                                            if let Some(color) = fill_color {
                                                let solid_fill = XmlNode::Element {
                                                    name: TagName::local("a:solidFill"),
                                                    attributes: BTreeMap::new(),
                                                    namespaces: vec![],
                                                    children: vec![XmlNode::Element {
                                                        name: TagName::local("a:srgbClr"),
                                                        attributes: {
                                                            let mut attrs = BTreeMap::new();
                                                            attrs.insert(
                                                                TagName::local("val"),
                                                                color.to_string(),
                                                            );
                                                            attrs
                                                        },
                                                        namespaces: vec![],
                                                        children: vec![],
                                                        span: ByteSpan {
                                                            offset: 0,
                                                            length: 0,
                                                        },
                                                    }],
                                                    span: ByteSpan {
                                                        offset: 0,
                                                        length: 0,
                                                    },
                                                };
                                                // Remove existing a:noFill if present
                                                for sp_child2 in sp_children {
                                                    if let XmlNode::Element {
                                                        name: nf_name, ..
                                                    } = sp_child2
                                                    {
                                                        if is_pptx_elem(nf_name, "noFill") {
                                                            if let XmlNode::Element {
                                                                span: nf_span,
                                                                ..
                                                            } = sp_child2
                                                            {
                                                                shape_muts.push(SpanMutation {
                                                                    span: nf_span.clone(),
                                                                    kind: MutationKind::DeleteNode,
                                                                });
                                                            }
                                                        }
                                                    }
                                                }
                                                shape_muts.push(SpanMutation {
                                                    span: ByteSpan {
                                                        offset: 0,
                                                        length: 0,
                                                    },
                                                    kind: MutationKind::InsertNode(solid_fill),
                                                });
                                            }

                                            // Set line via a:ln child of spPr
                                            if let Some(color) = line_color {
                                                let ln_w = if let Some(lw) = line_width_pt {
                                                    ((lw * 914400.0) as i64).to_string()
                                                } else {
                                                    "9525".to_string() // 1pt default in EMU
                                                };
                                                let ln_elem = XmlNode::Element {
                                                    name: TagName::local("a:ln"),
                                                    attributes: {
                                                        let mut attrs = BTreeMap::new();
                                                        attrs.insert(TagName::local("w"), ln_w);
                                                        attrs
                                                    },
                                                    namespaces: vec![],
                                                    children: vec![XmlNode::Element {
                                                        name: TagName::local("a:solidFill"),
                                                        attributes: BTreeMap::new(),
                                                        namespaces: vec![],
                                                        children: vec![XmlNode::Element {
                                                            name: TagName::local("a:srgbClr"),
                                                            attributes: {
                                                                let mut attrs = BTreeMap::new();
                                                                attrs.insert(
                                                                    TagName::local("val"),
                                                                    color.to_string(),
                                                                );
                                                                attrs
                                                            },
                                                            namespaces: vec![],
                                                            children: vec![],
                                                            span: ByteSpan {
                                                                offset: 0,
                                                                length: 0,
                                                            },
                                                        }],
                                                        span: ByteSpan {
                                                            offset: 0,
                                                            length: 0,
                                                        },
                                                    }],
                                                    span: ByteSpan {
                                                        offset: 0,
                                                        length: 0,
                                                    },
                                                };
                                                shape_muts.push(SpanMutation {
                                                    span: ByteSpan {
                                                        offset: 0,
                                                        length: 0,
                                                    },
                                                    kind: MutationKind::InsertNode(ln_elem),
                                                });
                                            }

                                            break;
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }

                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .extend(shape_muts);
                return Ok(());
            }
        }
        Ok(())
    }

    /// Find a shape node by matching cNvPr/@id or cNvPr/@name.
    fn find_shape_node<'b>(&self, doc: &'b XmlDocument, shape_id: &str) -> Option<&'b XmlNode> {
        self.search_for_shape(&doc.root, shape_id)
    }

    fn search_for_shape<'c>(&self, node: &'c XmlNode, shape_id: &str) -> Option<&'c XmlNode> {
        if let XmlNode::Element {
            name: elem_name,
            children,
            ..
        } = node
        {
            // Check if this is a <p:sp> or <p:pic> whose cNvPr has matching id or name
            if is_pptx_elem(elem_name, "sp") || is_pptx_elem(elem_name, "pic") {
                if self.shape_matches_id(children, shape_id) {
                    return Some(node);
                }
            }
            // Recurse into children
            for child in children {
                if let Some(found) = self.search_for_shape(child, shape_id) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn shape_matches_id(&self, children: &[XmlNode], shape_id: &str) -> bool {
        for child in children {
            if let XmlNode::Element {
                name: nv_name,
                children: nv_children,
                ..
            } = child
            {
                if is_pptx_elem(nv_name, "nvSpPr") || is_pptx_elem(nv_name, "nvPicPr") {
                    for nv_child in nv_children {
                        if let XmlNode::Element {
                            name: cnv_name,
                            attributes: cnv_attrs,
                            children: cnv_children,
                            ..
                        } = nv_child
                        {
                            if is_pptx_elem(cnv_name, "cNvPr") {
                                // Check @id
                                if let Some(id) = cnv_attrs.get(&TagName::local("id")) {
                                    if id == shape_id {
                                        return true;
                                    }
                                }
                                // Check @name
                                if let Some(name) = cnv_attrs.get(&TagName::local("name")) {
                                    if name == shape_id {
                                        return true;
                                    }
                                }
                                // Also check nvPr -> ph/@idx
                                for cnv_child2 in cnv_children {
                                    if let XmlNode::Element {
                                        name: npr_name,
                                        children: npr_children,
                                        ..
                                    } = cnv_child2
                                    {
                                        if is_pptx_elem(npr_name, "nvPr") {
                                            for npr_child in npr_children {
                                                if let XmlNode::Element {
                                                    name: ph_name,
                                                    attributes: ph_attrs,
                                                    ..
                                                } = npr_child
                                                {
                                                    if is_pptx_elem(ph_name, "ph") {
                                                        if let Some(idx) =
                                                            ph_attrs.get(&TagName::local("idx"))
                                                        {
                                                            if idx == shape_id {
                                                                return true;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Route InsertPicture op — build a proper pic:pic element with blipFill, spPr.
    ///
    /// The pic:pic element contains:
    ///  - nvPicPr/cNvPr/@id, /cNvPr/@name, cNvPicPr
    ///  - blipFill/a:blip @r:embed="rIdN" (relationship ID placeholder)
    ///  - spPr/a:xfrm @off/@ext for position/size
    ///  - spPr/a:prstGeom for shape
    #[allow(clippy::too_many_arguments)]
    fn route_insert_picture(
        &self,
        slide_id: &str,
        data: &[u8],
        mime_type: &str,
        alt_text: Option<&str>,
        x_pt: Option<f64>,
        y_pt: Option<f64>,
        width_pt: Option<f64>,
        height_pt: Option<f64>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
        collector: &mut NewPartsCollector,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_sp_tree_span(doc) {
                let x_emu = x_pt.map(|x| (x * 914400.0) as i64).unwrap_or(914400);
                let y_emu = y_pt.map(|y| (y * 914400.0) as i64).unwrap_or(548640);
                let w_emu = width_pt.map(|w| (w * 914400.0) as i64).unwrap_or(3048000);
                let h_emu = height_pt.map(|h| (h * 914400.0) as i64).unwrap_or(3048000);
                let alt = alt_text.unwrap_or("Image");

                let rel_id = format!("rId{}", 100 + (slide_id.chars().count() % 100));
                let file_ext = match mime_type {
                    "image/png" => "png",
                    "image/jpeg" | "image/jpg" => "jpg",
                    "image/gif" => "gif",
                    "image/bmp" => "bmp",
                    "image/svg+xml" => "svg",
                    _ => "png",
                };
                let media_uri = format!("ppt/media/image{}.{}", slide_id, file_ext);
                let content_type = match mime_type {
                    "image/png" => "image/png".to_string(),
                    "image/jpeg" | "image/jpg" => "image/jpeg".to_string(),
                    "image/gif" => "image/gif".to_string(),
                    "image/bmp" => "image/bmp".to_string(),
                    "image/svg+xml" => "image/svg+xml".to_string(),
                    _ => "application/octet-stream".to_string(),
                };
                let rel_type =
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
                let slide_rels_uri = format!("{}._rels", part_uri);

                collector.new_parts.push(NewPart {
                    uri: media_uri.clone(),
                    content_type,
                    data: data.to_vec(),
                });

                collector.rel_updates.push(RelUpdate {
                    source_uri: slide_rels_uri.clone(),
                    rel_id: rel_id.clone(),
                    target_uri: media_uri,
                    rel_type: rel_type.to_string(),
                });

                let new_node =
                    self.create_picture_element(slide_id, &rel_id, alt, x_emu, y_emu, w_emu, h_emu);
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::InsertNode(new_node),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route SetSlideText op — replace text in txBody/t.
    fn route_set_slide_text(
        &self,
        slide_id: &str,
        shape_id: &str,
        text: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_text_span(doc, shape_id) {
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::ReplaceText(text.to_string()),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route SetTableCellText op — replace text in table tc element.
    ///
    /// Uses the table cell span to replace text, and adds <a:b/> and <a:i/>
    /// children to the first <a:rPr> found in the cell's txBody for bold/italic.
    #[allow(clippy::too_many_arguments)]
    fn route_set_table_cell_text(
        &self,
        slide_id: &str,
        table_row: usize,
        table_col: usize,
        text: &str,
        bold: Option<bool>,
        italic: Option<bool>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if let Some(span) = self.find_table_cell_span(doc, table_row, table_col) {
                let mut cell_muts: Vec<SpanMutation> = Vec::new();

                cell_muts.push(SpanMutation {
                    span: span.clone(),
                    kind: MutationKind::ReplaceText(text.to_string()),
                });

                // Add bold/italic formatting if requested
                if bold == Some(true) {
                    cell_muts.push(SpanMutation {
                        span: span.clone(),
                        kind: MutationKind::SetAttribute {
                            name: TagName::local("a:b"),
                            value: "1".to_string(),
                        },
                    });
                }
                if italic == Some(true) {
                    cell_muts.push(SpanMutation {
                        span,
                        kind: MutationKind::SetAttribute {
                            name: TagName::local("a:i"),
                            value: "1".to_string(),
                        },
                    });
                }

                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .extend(cell_muts);
                return Ok(());
            }
        }
        Ok(())
    }

    /// Route SetSlideNotes op — build a notesSlide part with txBody containing the notes text.
    ///
    /// If a notesSlide part already exists (ppt/notesSlides/notesN.xml), replaces its text content.
    /// If it does not exist, creates a new notesSlide part and registers it via the collector.
    fn route_set_slide_notes(
        &self,
        slide_id: &str,
        notes: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
        collector: &mut NewPartsCollector,
    ) -> Result<(), Vec<Diagnostic>> {
        let notes_uri = format!("ppt/notesSlides/notes{}.xml", slide_id);

        if let Some(doc) = parts.get(&notes_uri) {
            // Existing notesSlide — find the first <a:t> text node and replace it
            if let Some(span) = self.find_notes_text_span(doc) {
                mutations_by_part
                    .entry(notes_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span,
                        kind: MutationKind::ReplaceText(notes.to_string()),
                    });
                return Ok(());
            }
        }

        // No notesSlide exists — create a new part via the collector
        let notes_xml = self.build_notes_slide_document(slide_id, notes);
        let notes_bytes = notes_xml.to_bytes();

        // Add the new notesSlide part
        collector.new_parts.push(NewPart {
            uri: notes_uri.clone(),
            content_type: "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml".into(),
            data: notes_bytes,
        });

        // Add the relationship from presentation.xml to the notesSlide part
        let rel_id = format!("rId{}", 100 + slide_id.chars().count());
        collector.rel_updates.push(RelUpdate {
            source_uri: "ppt/presentation.xml".to_string(),
            rel_id: rel_id.clone(),
            target_uri: notes_uri,
            rel_type:
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide"
                    .into(),
        });

        Ok(())
    }

    /// Build a complete notesSlide XmlDocument root element with txBody containing notes.
    fn build_notes_slide_document(&self, slide_id: &str, notes: &str) -> XmlNode {
        XmlNode::Element {
            name: TagName {
                prefix: Some("p".into()),
                local_name: "notesSlide".to_string(),
                namespace_uri: None,
            },
            attributes: BTreeMap::new(),
            namespaces: vec![
                super::xml::Namespace {
                    prefix: "p".into(),
                    uri: super::pptx::NS_PPT.into(),
                },
                super::xml::Namespace {
                    prefix: "a".into(),
                    uri: super::pptx::NS_DRAWING.into(),
                },
                super::xml::Namespace {
                    prefix: "r".into(),
                    uri: super::pptx::NS_RELS.into(),
                },
            ],
            children: vec![
                // notesBg
                XmlNode::Element {
                    name: TagName {
                        prefix: Some("p".into()),
                        local_name: "notesBg".to_string(),
                        namespace_uri: None,
                    },
                    attributes: [(TagName::local("show"), "false".to_string())]
                        .into_iter()
                        .collect(),
                    namespaces: vec![],
                    children: vec![],
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                },
                // spTree (placeholder for notes slide objects)
                XmlNode::Element {
                    name: TagName {
                        prefix: Some("p".into()),
                        local_name: "spTree".to_string(),
                        namespace_uri: None,
                    },
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: vec![],
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                },
                // txBody with notes text
                XmlNode::Element {
                    name: TagName {
                        prefix: Some("p".into()),
                        local_name: "txBody".to_string(),
                        namespace_uri: None,
                    },
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: vec![
                        XmlNode::Element {
                            name: TagName {
                                prefix: Some("a".into()),
                                local_name: "bodyPr".to_string(),
                                namespace_uri: None,
                            },
                            attributes: BTreeMap::new(),
                            namespaces: vec![],
                            children: vec![],
                            span: ByteSpan {
                                offset: 0,
                                length: 0,
                            },
                        },
                        XmlNode::Element {
                            name: TagName {
                                prefix: Some("a".into()),
                                local_name: "p".to_string(),
                                namespace_uri: None,
                            },
                            attributes: BTreeMap::new(),
                            namespaces: vec![],
                            children: vec![
                                XmlNode::Element {
                                    name: TagName {
                                        prefix: Some("a".into()),
                                        local_name: "rPr".to_string(),
                                        namespace_uri: None,
                                    },
                                    attributes: {
                                        let mut attrs = BTreeMap::new();
                                        attrs.insert(TagName::local("lang"), "en-US".to_string());
                                        attrs
                                    },
                                    namespaces: vec![],
                                    children: vec![],
                                    span: ByteSpan {
                                        offset: 0,
                                        length: 0,
                                    },
                                },
                                XmlNode::Text {
                                    text: notes.to_string(),
                                    span: ByteSpan {
                                        offset: 0,
                                        length: notes.len(),
                                    },
                                },
                            ],
                            span: ByteSpan {
                                offset: 0,
                                length: 0,
                            },
                        },
                    ],
                    span: ByteSpan {
                        offset: 0,
                        length: 0,
                    },
                },
            ],
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        }
    }

    /// Find a shape element by shape_id (cNvPr/@id or nvPr/@idx).
    #[allow(dead_code)]
    fn find_shape_span(&self, doc: &XmlDocument, shape_id: &str) -> Option<ByteSpan> {
        let sid = shape_id.to_string();
        self.find_node_by_id(&doc.root, &ElementId::derive("shape_id", &["shape"], &sid))
            .or_else(|| self.find_shape_by_name(&doc.root, shape_id))
            .map(|node| match node {
                XmlNode::Element { span, .. }
                | XmlNode::Text { span, .. }
                | XmlNode::Comment { span, .. }
                | XmlNode::CData { span, .. }
                | XmlNode::ProcessingInstruction { span, .. } => span.clone(),
            })
    }

    /// Find a shape by matching cNvPr/@name attribute.
    fn find_shape_by_name<'b>(&'b self, node: &'b XmlNode, name: &str) -> Option<&'b XmlNode> {
        if let XmlNode::Element {
            name: elem_name,
            children,
            ..
        } = node
        {
            if is_pptx_elem(elem_name, "sp")
                || is_pptx_elem(elem_name, "graphicFrame")
                || is_pptx_elem(elem_name, "tbl")
            {
                // Look for cNvPr with matching name attribute
                if self.search_cnvp_name(children, name).is_some() {
                    return Some(node);
                }
            }
            for child in children {
                if self.find_shape_by_name(child, name).is_some() {
                    return Some(node);
                }
            }
        }
        None
    }

    /// Search children for a cNvPr element with matching name attribute.
    fn search_cnvp_name<'b>(
        &'b self,
        children: &'b [XmlNode],
        target: &str,
    ) -> Option<&'b XmlNode> {
        for child in children {
            if let XmlNode::Element {
                name: cnvpr_name,
                attributes: _attrs,
                children: _gc,
                ..
            } = child
            {
                if is_pptx_elem(cnvpr_name, "cNvPr") {
                    if let XmlNode::Element { attributes, .. } = child {
                        if let Some(n) = attributes.get(&TagName::local("name")) {
                            if n == target {
                                return Some(child);
                            }
                        }
                    }
                }
            }
            if let XmlNode::Element { children: gc, .. } = child {
                if let Some(_found) = self.search_cnvp_name(gc, target) {
                    return Some(_found);
                }
            }
        }
        None
    }

    /// Find a text element in a shape's txBody.
    fn find_text_span(&self, doc: &XmlDocument, shape_id: &str) -> Option<ByteSpan> {
        if let Some(shape_node) = self.find_shape_by_name(&doc.root, shape_id) {
            if let XmlNode::Element { children, .. } = shape_node {
                if let Some(text_node) = self.find_first_text_node(children) {
                    return match text_node {
                        XmlNode::Text { span, .. } | XmlNode::Element { span, .. } => {
                            Some(span.clone())
                        }
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// Find a table cell span at the given row/col.
    fn find_table_cell_span(&self, doc: &XmlDocument, row: usize, col: usize) -> Option<ByteSpan> {
        if let Some(tbl_node) = self.find_first_tbl(&doc.root) {
            if let XmlNode::Element { children, .. } = tbl_node {
                if let Some(cell_node) = self.find_table_cell(children, row, col) {
                    return match cell_node {
                        XmlNode::Text { span, .. } | XmlNode::Element { span, .. } => {
                            Some(span.clone())
                        }
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// Find the first tbl element in an XmlNode tree.
    fn find_first_tbl<'b>(&'b self, node: &'b XmlNode) -> Option<&'b XmlNode> {
        if let XmlNode::Element { name, children, .. } = node {
            if is_pptx_elem(name, "tbl") {
                return Some(node);
            }
            for child in children {
                if let Some(found) = self.find_first_tbl(child) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Find a table cell at the given row/col index.
    fn find_table_cell<'b>(
        &'b self,
        children: &'b [XmlNode],
        row: usize,
        col: usize,
    ) -> Option<&'b XmlNode> {
        let mut current_row = 0;
        for child in children {
            if let XmlNode::Element {
                name, children: gc, ..
            } = child
            {
                if is_pptx_elem(name, "tr") && current_row == row {
                    for (j, gc_child) in gc.iter().enumerate() {
                        if let XmlNode::Element { name: tc_name, .. } = gc_child {
                            if is_pptx_elem(tc_name, "tc") && j == col {
                                return Some(gc_child);
                            }
                        }
                    }
                    current_row += 1;
                }
            }
        }
        None
    }

    /// Find the first text node in children.
    fn find_first_text_node<'b>(&'b self, children: &'b [XmlNode]) -> Option<&'b XmlNode> {
        for child in children {
            if let XmlNode::Element {
                name, children: gc, ..
            } = child
            {
                if is_pptx_elem(name, "t") {
                    if let Some(tn) = gc.first() {
                        return Some(tn);
                    }
                }
                if let Some(found) = self.find_first_text_node(gc) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Find the notes text span.
    fn find_notes_text_span(&self, doc: &XmlDocument) -> Option<ByteSpan> {
        self.find_notes_text_node(&doc.root).map(|node| match node {
            XmlNode::Text { span, .. } | XmlNode::Element { span, .. } => span.clone(),
            _ => ByteSpan {
                offset: 0,
                length: 0,
            },
        })
    }

    /// Find a text node in the notes slide.
    fn find_notes_text_node<'b>(&'b self, node: &'b XmlNode) -> Option<&'b XmlNode> {
        if let XmlNode::Text { text, .. } = node {
            if !text.is_empty() {
                return Some(node);
            }
        }
        if let XmlNode::Element { children, .. } = node {
            for child in children {
                if let Some(found) = self.find_notes_text_node(child) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Find a spTree element in a slide.
    fn find_sp_tree_span(&self, doc: &XmlDocument) -> Option<ByteSpan> {
        self.find_sp_tree(&doc.root).map(|node| match node {
            XmlNode::Element { span, .. }
            | XmlNode::Text { span, .. }
            | XmlNode::Comment { span, .. }
            | XmlNode::CData { span, .. }
            | XmlNode::ProcessingInstruction { span, .. } => span.clone(),
        })
    }

    fn find_sp_tree<'b>(&'b self, node: &'b XmlNode) -> Option<&'b XmlNode> {
        if let XmlNode::Element { name, children, .. } = node {
            if is_pptx_elem(name, "spTree") {
                return Some(node);
            }
            for child in children {
                if let Some(found) = self.find_sp_tree(child) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Create a proper pic:pic element for picture insertion.
    ///
    /// The element includes:
    ///  - nvPicPr with cNvPr (id/name) and cNvPicPr
    ///  - blipFill with a:blip @r:embed="rIdN"
    ///  - spPr with a:xfrm @off/@ext and prstGeom
    fn create_picture_element(
        &self,
        slide_id: &str,
        rel_id: &str,
        alt_text: &str,
        x_emu: i64,
        y_emu: i64,
        w_emu: i64,
        h_emu: i64,
    ) -> XmlNode {
        let pic_id = format!("{}pic", slide_id);

        XmlNode::Element {
            name: TagName {
                prefix: Some("p".into()),
                local_name: "pic".to_string(),
                namespace_uri: None,
            },
            attributes: BTreeMap::new(),
            namespaces: vec![super::xml::Namespace {
                prefix: "pic".into(),
                uri: super::pptx::NS_DRAWING.into(),
            }],
            children: vec![
                // nvPicPr
                XmlNode::Element {
                    name: TagName {
                        prefix: Some("p".into()),
                        local_name: "nvPicPr".to_string(),
                        namespace_uri: None,
                    },
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: vec![
                        XmlNode::Element {
                            name: TagName::local("cNvPicPr"),
                            attributes: {
                                let mut attrs = BTreeMap::new();
                                attrs.insert(TagName::local("anchor"), "1".to_string());
                                if !alt_text.is_empty() {
                                    attrs.insert(TagName::local("title"), alt_text.to_string());
                                }
                                attrs
                            },
                            namespaces: vec![],
                            children: vec![],
                            span: ByteSpan {
                                offset: 0,
                                length: 12,
                            },
                        },
                        XmlNode::Element {
                            name: TagName::local("cNvPr"),
                            attributes: [
                                (TagName::local("id"), pic_id.clone()),
                                (TagName::local("name"), format!("picture_{}", slide_id)),
                            ]
                            .into_iter()
                            .collect(),
                            namespaces: vec![],
                            children: vec![],
                            span: ByteSpan {
                                offset: 0,
                                length: 30,
                            },
                        },
                    ],
                    span: ByteSpan {
                        offset: 0,
                        length: 50,
                    },
                },
                // blipFill with a:blip r:embed
                XmlNode::Element {
                    name: TagName {
                        prefix: Some("p".into()),
                        local_name: "blipFill".to_string(),
                        namespace_uri: None,
                    },
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: vec![XmlNode::Element {
                        name: TagName {
                            prefix: Some("a".into()),
                            local_name: "blip".to_string(),
                            namespace_uri: None,
                        },
                        attributes: {
                            let mut attrs = BTreeMap::new();
                            attrs.insert(
                                TagName {
                                    prefix: Some("r".into()),
                                    local_name: "embed".to_string(),
                                    namespace_uri: None,
                                },
                                rel_id.to_string(),
                            );
                            attrs.insert(TagName::local("cstate"), "print".to_string());
                            attrs
                        },
                        namespaces: vec![],
                        children: vec![],
                        span: ByteSpan {
                            offset: 0,
                            length: 10,
                        },
                    }],
                    span: ByteSpan {
                        offset: 0,
                        length: 10,
                    },
                },
                // spPr
                XmlNode::Element {
                    name: TagName {
                        prefix: Some("p".into()),
                        local_name: "spPr".to_string(),
                        namespace_uri: None,
                    },
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: vec![
                        XmlNode::Element {
                            name: TagName {
                                prefix: Some("a".into()),
                                local_name: "xfrm".to_string(),
                                namespace_uri: None,
                            },
                            attributes: BTreeMap::new(),
                            namespaces: vec![],
                            children: vec![
                                XmlNode::Element {
                                    name: TagName {
                                        prefix: Some("a".into()),
                                        local_name: "off".to_string(),
                                        namespace_uri: None,
                                    },
                                    attributes: {
                                        let mut attrs = BTreeMap::new();
                                        attrs.insert(TagName::local("x"), x_emu.to_string());
                                        attrs.insert(TagName::local("y"), y_emu.to_string());
                                        attrs
                                    },
                                    namespaces: vec![],
                                    children: vec![],
                                    span: ByteSpan {
                                        offset: 0,
                                        length: 10,
                                    },
                                },
                                XmlNode::Element {
                                    name: TagName {
                                        prefix: Some("a".into()),
                                        local_name: "ext".to_string(),
                                        namespace_uri: None,
                                    },
                                    attributes: {
                                        let mut attrs = BTreeMap::new();
                                        attrs.insert(TagName::local("cx"), w_emu.to_string());
                                        attrs.insert(TagName::local("cy"), h_emu.to_string());
                                        attrs
                                    },
                                    namespaces: vec![],
                                    children: vec![],
                                    span: ByteSpan {
                                        offset: 0,
                                        length: 10,
                                    },
                                },
                            ],
                            span: ByteSpan {
                                offset: 0,
                                length: 10,
                            },
                        },
                        XmlNode::Element {
                            name: TagName::local("a:prstGeom"),
                            attributes: {
                                let mut attrs = BTreeMap::new();
                                attrs.insert(TagName::local("prst"), "rect".to_string());
                                attrs
                            },
                            namespaces: vec![],
                            children: vec![XmlNode::Element {
                                name: TagName::local("a:avLst"),
                                attributes: BTreeMap::new(),
                                namespaces: vec![],
                                children: vec![],
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
                    ],
                    span: ByteSpan {
                        offset: 0,
                        length: 10,
                    },
                },
            ],
            span: ByteSpan {
                offset: 0,
                length: 100,
            },
        }
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

/// Check if an XML element name matches a local name with the PowerPoint namespace prefix.
fn is_pptx_elem(name: &TagName, local: &str) -> bool {
    name.local_name == local
        && (name.prefix.as_deref() == Some("p")
            || name.prefix.as_deref() == Some("p14")
            || name.prefix.as_deref() == Some("p15")
            || name.prefix.as_deref() == Some("p16")
            || name.prefix.as_deref() == Some("a")
            || name.prefix.as_deref() == Some("pic")
            || name.prefix.as_deref() == Some("r")
            || name.prefix.as_deref() == Some("a14")
            || name.prefix.as_deref() == Some("p12"))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::super::pptx;
    use super::*;

    /// test_pptx_writer_creates.
    #[test]
    fn test_pptx_writer_creates() {
        let pres_xml = pptx::build_presentation_xml(1);
        let slide_xml = pptx::build_slide_with_text(&["Content"]);
        let layout_xml = pptx::build_layout_xml("Title Slide", "title");
        let master_xml = pptx::build_master_xml("Office Theme");

        let pkg = pptx::build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);
        let package = super::super::opc::parse_opc(&pkg).expect("should parse pptx");
        let writer = PptxPatchWriter::new(&package, Vec::new());
        assert!(writer.ops.is_empty());
    }

    /// test_pptx_write_empty_ops.
    #[test]
    fn test_pptx_write_empty_ops() {
        let pres_xml = pptx::build_presentation_xml(1);
        let slide_xml = pptx::build_slide_with_text(&["Content"]);
        let layout_xml = pptx::build_layout_xml("Title Slide", "title");
        let master_xml = pptx::build_master_xml("Office Theme");

        let pkg = pptx::build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);
        let package = super::super::opc::parse_opc(&pkg).expect("should parse pptx");
        let writer = PptxPatchWriter::new(&package, Vec::new());
        let result = writer.write();
        assert!(result.is_ok());
    }
}
