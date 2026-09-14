//! PPTX patch writer — applies IR patch operations to OOXML presentations.
//!
//! Routes patch ops to span mutations on slide XML, handles ReplaceText on shape text,
//! Insert/Delete of shapes, and skips incompatible operations with diagnostics.

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

/// A PPTX-specific patch writer.
pub struct PptxPatchWriter<'a> {
    /// The OPC package to patch.
    pub package: &'a OpcPackage,
    /// The patch operations to apply.
    pub ops: Vec<PatchOp>,
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
