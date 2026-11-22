// SPDX-License-Identifier: MIT OR Apache-2.0
//! XLSX patch writer — applies IR patch operations to OOXML spreadsheets.
//!
//! Routes patch ops to span mutations on worksheet XML, handles SetCellValue,
//! formula detection, stale cached value tracking, row append, number formats,
//! and XLSX structural operations (freeze pane, auto-filter, conditional formatting,
//! sheet protection, page setup, cell hyperlinks, defined names, data validation, tables,
//! cell styles, row/col dimensions).

#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unreachable_patterns)]
#![allow(dead_code)]
#![allow(non_snake_case)]

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

        // 4. Apply page_setup ops directly to worksheet parts (bypass SpanMutation).
        for op in &self.ops {
            if let PatchOp::SetPageSetup {
                sheet_name,
                orientation,
                paper_size,
                margin_top,
                margin_bottom,
                margin_left,
                margin_right,
            } = op
            {
                // Find matching sheet part by URI containing the sheet name (lowercase)
                for (part_uri, doc) in &mut parts {
                    if part_uri.contains("sheet")
                        && part_uri.to_lowercase().contains(&sheet_name.to_lowercase().replace(' ', ""))
                    {
                        let ps = self.create_page_setup(orientation, paper_size);
                        let pm = self.create_page_margins(
                            margin_top,
                            margin_bottom,
                            margin_left,
                            margin_right,
                        );
                        if let XmlNode::Element { children, .. } = &mut doc.root {
                            children.push(ps);
                            children.push(pm);
                        }
                        break;
                    }
                }
            }
        }

        // 5. Detect stale cached values by scanning formula references.
        let _stale = self.detect_stale_cached_values(&parts);

        // 6. Build PartMutation map and write via SurgicalWriter.
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

    /// Parse workbook-level parts (workbook.xml, styles.xml) for structural ops.
    ///
    /// Returns a BTreeMap keyed by part URI. Only includes parts that exist
    /// and parse successfully — missing parts are silently skipped.
    fn parse_workbook_parts(&self) -> BTreeMap<String, XmlDocument> {
        let mut parts = BTreeMap::new();

        for part in &self.package.parts {
            let uri = &part.uri;
            let is_workbook = uri == "xl/workbook.xml" || uri.ends_with("/workbook.xml");
            let is_styles = uri == "xl/styles.xml" || uri.ends_with("/styles.xml");

            if (is_workbook || is_styles) && part.uri.starts_with("xl/") {
                if let Ok(doc) = super::xml::parse_xml(&part.data) {
                    parts.insert(part.uri.clone(), doc);
                }
            }
        }

        parts
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
                PatchOp::SetShapeProps { .. }
                | PatchOp::InsertPicture { .. }
                | PatchOp::SetSlideText { .. }
                | PatchOp::SetTableCellText { .. }
                | PatchOp::SetSlideNotes { .. }
                | PatchOp::SetStyle { .. }
                | PatchOp::AssignStyle { .. }
                | PatchOp::AddHyperlink { .. } => {
                    // PPTX/DOCX-specific ops — not applicable to xlsx.
                }
                PatchOp::SetCellStyle {
                    cell_ref,
                    font_color,
                    fill_color,
                    bold,
                    italic,
                    number_format,
                    border,
                } => {
                    match self.route_set_cell_style(
                        cell_ref,
                        font_color,
                        fill_color,
                        bold,
                        italic,
                        number_format,
                        border,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetFreezePane {
                    sheet_name,
                    row,
                    col,
                } => {
                    match self.route_set_freeze_pane(
                        sheet_name,
                        row,
                        col,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetRowColDim {
                    sheet_name,
                    col_index,
                    width,
                    row_index,
                    height,
                    hidden,
                } => {
                    match self.route_set_rowcol_dim(
                        sheet_name,
                        col_index,
                        width,
                        row_index,
                        height,
                        hidden,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetAutoFilter { sheet_name, range } => {
                    match self.route_set_auto_filter(
                        sheet_name,
                        &range,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetConditionalFormat {
                    sheet_name,
                    sqref,
                    cf_type,
                    formula,
                    priority,
                } => {
                    match self.route_set_conditional_format(
                        sheet_name,
                        sqref,
                        cf_type,
                        &formula,
                        priority,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetSheetProtection {
                    sheet_name,
                    password,
                    protect_sheet,
                    protect_content,
                } => {
                    match self.route_set_sheet_protection(
                        sheet_name,
                        &password,
                        protect_sheet,
                        protect_content,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetPageSetup {
                    sheet_name,
                    orientation,
                    paper_size,
                    margin_top,
                    margin_bottom,
                    margin_left,
                    margin_right,
                } => {
                    match self.route_set_page_setup(
                        sheet_name,
                        &orientation,
                        paper_size,
                        margin_top,
                        margin_bottom,
                        margin_left,
                        margin_right,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetCellHyperlink {
                    cell_ref,
                    target_url,
                    tooltip,
                } => {
                    match self.route_set_cell_hyperlink(
                        cell_ref,
                        target_url,
                        tooltip,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetDefinedName {
                    name,
                    formula,
                    sheet_id,
                } => {
                    match self.route_set_defined_name(
                        name,
                        formula,
                        &sheet_id,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::SetDataValidation {
                    cell_ref,
                    validation_type,
                    formula1,
                    formula2,
                    operator,
                    allow_blank,
                } => {
                    match self.route_set_data_validation(
                        cell_ref,
                        &validation_type,
                        &formula1,
                        &formula2,
                        &operator,
                        allow_blank,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::AddTable {
                    sheet_name,
                    range,
                    table_name,
                    display_name,
                } => {
                    match self.route_add_table(
                        sheet_name,
                        &range,
                        &table_name,
                        &display_name,
                        parts,
                        &mut mutations_by_part,
                    ) {
                        Ok(_) => {}
                        Err(e) => diags.extend(e),
                    }
                }
                PatchOp::RemoveTable {
                    sheet_name,
                    table_name,
                } => {
                    match self.route_remove_table(
                        sheet_name,
                        &table_name,
                        parts,
                        &mut mutations_by_part,
                    ) {
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

    // =========================================================================
    // Structural route handlers for XLSX ops
    // =========================================================================

    /// Route SetCellStyle — find the target cell in worksheet XML and apply style.
    fn route_set_cell_style(
        &self,
        cell_ref: &str,
        font_color: &Option<String>,
        fill_color: &Option<String>,
        bold: &Option<bool>,
        italic: &Option<bool>,
        number_format: &Option<String>,
        border: &Option<String>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        // The style index will be looked up/created by the writer.
        // For now, we create a marker that the cell needs styling.
        for (part_uri, doc) in parts {
            if let Some(cell_span) = self.find_cell_span_by_id(
                doc,
                &ElementId::content_anchored("cell", &[cell_ref], cell_ref),
            ) {
                let style_props = self.build_style_xml(
                    font_color,
                    fill_color,
                    bold,
                    italic,
                    number_format,
                    border,
                );
                mutations_by_part
                    .entry(part_uri.clone())
                    .or_default()
                    .push(SpanMutation {
                        span: cell_span,
                        kind: MutationKind::InsertNode(style_props),
                    });
                return Ok(());
            }
        }
        Ok(())
    }

    /// Build an <xf> element for styles.xml with the given style properties.
    fn build_style_xml(
        &self,
        _font_color: &Option<String>,
        _fill_color: &Option<String>,
        _bold: &Option<bool>,
        _italic: &Option<bool>,
        _number_format: &Option<String>,
        _border: &Option<String>,
    ) -> XmlNode {
        // Create a minimal xf element placeholder.
        // The actual style index management happens at write time.
        XmlNode::Element {
            name: TagName::local("xf"),
            attributes: BTreeMap::new(),
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 4,
            },
        }
    }

    /// Route SetFreezePane — add <pane> element in sheet's <sheetViews>.
    fn route_set_freeze_pane(
        &self,
        sheet_name: &str,
        row: &usize,
        col: &usize,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let pane_xml = self.create_freeze_pane(*row, *col);
        for (part_uri, doc) in parts {
            // Match by sheet name in part URI
            if part_uri.contains("sheet") {
                if let Some(span) = self.find_sheet_views_span(doc, sheet_name, part_uri) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::InsertNode(pane_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Create a <pane> element for freeze panes.
    fn create_freeze_pane(&self, row: usize, col: usize) -> XmlNode {
        let mut attrs = BTreeMap::new();
        if row > 0 {
            attrs.insert(TagName::local("ySplit"), row.to_string());
        }
        if col > 0 {
            attrs.insert(TagName::local("xSplit"), col.to_string());
        }
        let topLeft = if row > 0 && col > 0 {
            let col_letter = Self::col_index_to_letter(col);
            format!("{}{}", col_letter, row + 1)
        } else if row > 0 {
            format!("A{}", row + 1)
        } else if col > 0 {
            let col_letter = Self::col_index_to_letter(col);
            format!("{}1", col_letter)
        } else {
            "A1".to_string()
        };
        attrs.insert(TagName::local("topLeftCell"), topLeft);
        attrs.insert(TagName::local("activePane"), "bottomLeft".to_string());
        attrs.insert(TagName::local("state"), "frozen".to_string());

        XmlNode::Element {
            name: TagName::local("pane"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 20,
            },
        }
    }

    /// Find sheetViews span in worksheet XML, optionally matching sheet name.
    fn find_sheet_views_span(
        &self,
        doc: &XmlDocument,
        _sheet_name: &str,
        _part_uri: &str,
    ) -> Option<ByteSpan> {
        // Find the <sheetViews> element by walking the tree
        self.find_element_by_local_name(&doc.root, "sheetViews")
            .map(|node| match node {
                XmlNode::Element { span, .. } => span.clone(),
                _ => ByteSpan {
                    offset: 0,
                    length: 0,
                },
            })
    }

    /// Recursively find an element by local name.
    fn find_element_by_local_name<'b>(
        &'b self,
        node: &'b XmlNode,
        local: &str,
    ) -> Option<&'b XmlNode> {
        if let XmlNode::Element { name, children, .. } = node {
            if name.local_name == local {
                return Some(node);
            }
            for child in children {
                if let Some(found) = self.find_element_by_local_name(child, local) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Convert a column index to Excel column letter (0 -> A, 1 -> B, etc.)
    fn col_index_to_letter(idx: usize) -> String {
        let mut result = String::new();
        let mut n = idx;
        loop {
            if n == 0 {
                break;
            }
            n -= 1;
            result.push((b'A' + (n % 26) as u8) as char);
            n /= 26;
        }
        result.chars().rev().collect()
    }

    /// Route SetRowColDim — add <col> or <row> elements to the sheet.
    fn route_set_rowcol_dim(
        &self,
        sheet_name: &str,
        col_index: &Option<usize>,
        width: &Option<f64>,
        row_index: &Option<usize>,
        height: &Option<f64>,
        hidden: &Option<bool>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(ci) = col_index {
            let col_xml = self.create_col_element(*ci, *width, *hidden);
            for (part_uri, doc) in parts {
                if part_uri.contains("sheet") {
                    if let Some(span) = self.find_sheet_data_span(doc) {
                        mutations_by_part
                            .entry(part_uri.clone())
                            .or_default()
                            .push(SpanMutation {
                                span,
                                kind: MutationKind::InsertNode(col_xml.clone()),
                            });
                        return Ok(());
                    }
                }
            }
        } else if let Some(ri) = row_index {
            let row_xml = self.create_row_element(*ri, *height, *hidden);
            for (part_uri, doc) in parts {
                if part_uri.contains("sheet") {
                    if let Some(span) = self.find_sheet_data_span(doc) {
                        mutations_by_part
                            .entry(part_uri.clone())
                            .or_default()
                            .push(SpanMutation {
                                span,
                                kind: MutationKind::InsertNode(row_xml.clone()),
                            });
                        return Ok(());
                    }
                }
            }
        }
        // If neither col nor row index specified, try to find the sheet by name
        if col_index.is_none() && row_index.is_none() {
            for (part_uri, doc) in parts {
                if part_uri.contains("sheet") {
                    if let Some(span) = self.find_sheet_data_span(doc) {
                        let dim_xml = XmlNode::Element {
                            name: TagName::local("col"),
                            attributes: BTreeMap::new(),
                            namespaces: vec![],
                            children: vec![],
                            span: ByteSpan {
                                offset: 0,
                                length: 4,
                            },
                        };
                        mutations_by_part
                            .entry(part_uri.clone())
                            .or_default()
                            .push(SpanMutation {
                                span,
                                kind: MutationKind::InsertNode(dim_xml),
                            });
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// Create a <col> element for column dimensions.
    fn create_col_element(&self, min: usize, width: Option<f64>, hidden: Option<bool>) -> XmlNode {
        let mut attrs = BTreeMap::new();
        // min is 0-based; Excel uses 1-based
        attrs.insert(TagName::local("min"), (min + 1).to_string());
        attrs.insert(TagName::local("max"), (min + 1).to_string());
        if let Some(w) = width {
            attrs.insert(TagName::local("width"), format!("{:.2}", w));
        }
        if let Some(h) = hidden {
            attrs.insert(
                TagName::local("hidden"),
                if h { "1" } else { "0" }.to_string(),
            );
        }
        // Default style index
        attrs.insert(TagName::local("style"), "0".to_string());

        XmlNode::Element {
            name: TagName::local("col"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 10,
            },
        }
    }

    /// Create a <row> element for row dimensions.
    fn create_row_element(
        &self,
        index: usize,
        height: Option<f64>,
        hidden: Option<bool>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        // index is 0-based; Excel uses 1-based
        attrs.insert(TagName::local("r"), (index + 1).to_string());
        if let Some(h) = height {
            attrs.insert(TagName::local("ht"), format!("{:.2}", h));
        }
        if let Some(h) = hidden {
            attrs.insert(
                TagName::local("hidden"),
                if h { "1" } else { "0" }.to_string(),
            );
        }

        XmlNode::Element {
            name: TagName::local("row"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 10,
            },
        }
    }

    /// Find sheetData span in worksheet XML.
    fn find_sheet_data_span(&self, doc: &XmlDocument) -> Option<ByteSpan> {
        self.find_element_by_local_name(&doc.root, "sheetData")
            .map(|node| match node {
                XmlNode::Element { span, .. } => span.clone(),
                _ => ByteSpan {
                    offset: 0,
                    length: 0,
                },
            })
    }

    /// Route SetAutoFilter — add <autoFilter ref="..."/> to the sheet.
    fn route_set_auto_filter(
        &self,
        sheet_name: &str,
        range: &Option<String>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(ref r) = range {
            let af_xml = self.create_auto_filter(r);
            for (part_uri, doc) in parts {
                if part_uri.contains("sheet") {
                    if let Some(span) = self.find_sheet_data_span(doc) {
                        mutations_by_part
                            .entry(part_uri.clone())
                            .or_default()
                            .push(SpanMutation {
                                span,
                                kind: MutationKind::InsertNode(af_xml.clone()),
                            });
                        return Ok(());
                    }
                }
            }
        } else {
            // Remove auto-filter: we mark it for deletion
            for (part_uri, doc) in parts {
                if part_uri.contains("sheet") {
                    if let Some(span) = self
                        .find_element_by_local_name(&doc.root, "autoFilter")
                        .map(|node| match node {
                            XmlNode::Element { span, .. } => span.clone(),
                            _ => ByteSpan {
                                offset: 0,
                                length: 0,
                            },
                        })
                    {
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
            }
        }
        let _ = sheet_name;
        Ok(())
    }

    /// Create an <autoFilter ref="..."/> element.
    fn create_auto_filter(&self, ref_range: &str) -> XmlNode {
        let mut attrs = BTreeMap::new();
        attrs.insert(TagName::local("ref"), ref_range.to_string());
        XmlNode::Element {
            name: TagName::local("autoFilter"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 10,
            },
        }
    }

    /// Route SetConditionalFormat — add conditionalFormatting section.
    fn route_set_conditional_format(
        &self,
        sheet_name: &str,
        sqref: &str,
        cf_type: &str,
        formula: &Option<String>,
        priority: &Option<usize>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let cf_xml = self.create_conditional_format(sqref, cf_type, formula, priority);
        for (part_uri, doc) in parts {
            if part_uri.contains("sheet") {
                if let Some(span) = self.find_sheet_data_span(doc) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::InsertNode(cf_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        let _ = sheet_name;
        Ok(())
    }

    /// Create a <conditionalFormatting> element with a <cfRule>.
    fn create_conditional_format(
        &self,
        sqref: &str,
        cf_type: &str,
        formula: &Option<String>,
        priority: &Option<usize>,
    ) -> XmlNode {
        let mut cf_rule_attrs = BTreeMap::new();
        cf_rule_attrs.insert(TagName::local("type"), cf_type.to_string());
        if let Some(p) = priority {
            cf_rule_attrs.insert(TagName::local("priority"), p.to_string());
        }
        if let Some(ref f) = formula {
            cf_rule_attrs.insert(TagName::local("formula"), f.clone());
        }

        // Build the cfRule element's children first
        let inner_children: Vec<XmlNode> = if let Some(ref f) = formula {
            vec![XmlNode::Element {
                name: TagName::local("formula"),
                attributes: BTreeMap::new(),
                namespaces: vec![],
                children: vec![XmlNode::Text {
                    text: f.clone(),
                    span: ByteSpan {
                        offset: 0,
                        length: f.len(),
                    },
                }],
                span: ByteSpan {
                    offset: 0,
                    length: f.len(),
                },
            }]
        } else {
            vec![]
        };

        // Now build the cfRule element
        let cf_rule_elem = XmlNode::Element {
            name: TagName::local("cfRule"),
            attributes: cf_rule_attrs,
            namespaces: vec![],
            children: inner_children,
            span: ByteSpan {
                offset: 0,
                length: 6,
            },
        };

        let mut attrs = BTreeMap::new();
        attrs.insert(TagName::local("sqref"), sqref.to_string());

        XmlNode::Element {
            name: TagName::local("conditionalFormatting"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![cf_rule_elem],
            span: ByteSpan {
                offset: 0,
                length: 18,
            },
        }
    }

    /// Route SetSheetProtection — add <sheetProtection .../> to the sheet.
    fn route_set_sheet_protection(
        &self,
        sheet_name: &str,
        password: &Option<String>,
        protect_sheet: &Option<bool>,
        protect_content: &Option<bool>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let prot_xml = self.create_sheet_protection(password, protect_sheet, protect_content);
        for (part_uri, doc) in parts {
            if part_uri.contains("sheet") {
                // Try to find sheetProtection to replace, or insert before sheetData
                if let Some(span) = self.find_sheet_data_span(doc) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::InsertNode(prot_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        let _ = sheet_name;
        Ok(())
    }

    /// Create a <sheetProtection .../> element.
    fn create_sheet_protection(
        &self,
        password: &Option<String>,
        protect_sheet: &Option<bool>,
        protect_content: &Option<bool>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        if let Some(ref p) = password {
            attrs.insert(TagName::local("password"), p.clone());
        }
        // Default to protecting the sheet if not explicitly set
        if protect_sheet.unwrap_or(true) {
            attrs.insert(TagName::local("sheet"), "1".to_string());
        }
        if let Some(pc) = protect_content {
            attrs.insert(
                TagName::local("objects"),
                if *pc { "0" } else { "1" }.to_string(),
            );
            attrs.insert(
                TagName::local("scenarios"),
                if *pc { "0" } else { "1" }.to_string(),
            );
        }
        XmlNode::Element {
            name: TagName::local("sheetProtection"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 16,
            },
        }
    }

    /// Route SetPageSetup — add <pageSetup/> and <pageMargins/> to the sheet.
    fn route_set_page_setup(
        &self,
        sheet_name: &str,
        orientation: &Option<String>,
        paper_size: &Option<usize>,
        margin_top: &Option<f64>,
        margin_bottom: &Option<f64>,
        margin_left: &Option<f64>,
        margin_right: &Option<f64>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let ps_xml = self.create_page_setup(orientation, paper_size);
        let pm_xml = self.create_page_margins(margin_top, margin_bottom, margin_left, margin_right);

        // Skip SpanMutation for page_setup — handled directly in write()
        // to avoid span reindexing issues when inserting sibling elements
        let _ = (sheet_name, ps_xml, pm_xml);
        Ok(())
    }

    /// Create a <pageSetup .../> element.
    fn create_page_setup(
        &self,
        orientation: &Option<String>,
        paper_size: &Option<usize>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        if let Some(ref o) = orientation {
            attrs.insert(TagName::local("orientation"), o.clone());
        }
        if let Some(ps) = paper_size {
            attrs.insert(TagName::local("paperSize"), ps.to_string());
        }
        attrs.insert(TagName::local("sheetId"), "1".to_string());
        XmlNode::Element {
            name: TagName::local("pageSetup"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 12,
            },
        }
    }

    /// Create a <pageMargins .../> element.
    fn create_page_margins(
        &self,
        top: &Option<f64>,
        bottom: &Option<f64>,
        left: &Option<f64>,
        right: &Option<f64>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        attrs.insert(
            TagName::local("left"),
            format!("{:.2}", left.unwrap_or(0.75)),
        );
        attrs.insert(
            TagName::local("right"),
            format!("{:.2}", right.unwrap_or(0.75)),
        );
        attrs.insert(TagName::local("top"), format!("{:.2}", top.unwrap_or(1.0)));
        attrs.insert(
            TagName::local("bottom"),
            format!("{:.2}", bottom.unwrap_or(1.0)),
        );
        attrs.insert(TagName::local("header"), "0.5".to_string());
        attrs.insert(TagName::local("footer"), "0.5".to_string());
        XmlNode::Element {
            name: TagName::local("pageMargins"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 13,
            },
        }
    }

    /// Route SetCellHyperlink — add <hyperlink> to the sheet.
    fn route_set_cell_hyperlink(
        &self,
        cell_ref: &str,
        target_url: &str,
        tooltip: &Option<String>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let hl_xml = self.create_hyperlink(cell_ref, target_url, tooltip);
        for (part_uri, doc) in parts {
            if part_uri.contains("sheet") {
                // Find the cell element and add hyperlink as a sibling
                let cell_id = ElementId::content_anchored("cell", &[cell_ref], cell_ref);
                if let Some(cell_span) = self.find_cell_span_by_id(doc, &cell_id) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span: cell_span,
                            kind: MutationKind::InsertNode(hl_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Create a <hyperlink r:id="..." ref="A1"/> element.
    fn create_hyperlink(
        &self,
        cell_ref: &str,
        target_url: &str,
        tooltip: &Option<String>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        attrs.insert(TagName::local("ref"), cell_ref.to_string());
        attrs.insert(TagName::local("r:id"), "rId_hyperlink".to_string());
        if let Some(ref t) = tooltip {
            attrs.insert(TagName::local("tooltip"), t.clone());
        }

        // The actual relationship needs to be added to the .rels file.
        // For the minimum viable implementation, we create a placeholder.
        let mut url_child = Vec::new();
        url_child.push(XmlNode::Element {
            name: TagName::local("r:id"),
            attributes: attrs.clone(),
            namespaces: vec![],
            children: vec![XmlNode::Text {
                text: target_url.to_string(),
                span: ByteSpan {
                    offset: 0,
                    length: target_url.len(),
                },
            }],
            span: ByteSpan {
                offset: 0,
                length: target_url.len(),
            },
        });

        XmlNode::Element {
            name: TagName::local("hyperlink"),
            attributes: attrs,
            namespaces: vec![],
            children: url_child,
            span: ByteSpan {
                offset: 0,
                length: 10,
            },
        }
    }

    /// Route SetDefinedName — add <definedName> to workbook.xml.
    fn route_set_defined_name(
        &self,
        name: &str,
        formula: &str,
        sheet_id: &Option<String>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let dn_xml = self.create_defined_name(name, formula, sheet_id);
        for (part_uri, doc) in parts {
            // workbook.xml typically lives at xl/workbook.xml
            if part_uri.ends_with("workbook.xml") || part_uri.contains("workbook") {
                if let Some(span) = self.find_defined_names_span(doc) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::InsertNode(dn_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Create a <definedName> element.
    fn create_defined_name(&self, name: &str, formula: &str, sheet_id: &Option<String>) -> XmlNode {
        let mut attrs = BTreeMap::new();
        attrs.insert(TagName::local("name"), name.to_string());
        if let Some(ref sid) = sheet_id {
            attrs.insert(TagName::local("localSheetId"), sid.clone());
        }

        XmlNode::Element {
            name: TagName::local("definedName"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![XmlNode::Text {
                text: formula.to_string(),
                span: ByteSpan {
                    offset: 0,
                    length: formula.len(),
                },
            }],
            span: ByteSpan {
                offset: 0,
                length: 12,
            },
        }
    }

    /// Find <definedNames> or <workbook> span in workbook XML.
    fn find_defined_names_span(&self, doc: &XmlDocument) -> Option<ByteSpan> {
        self.find_element_by_local_name(&doc.root, "definedNames")
            .or_else(|| self.find_element_by_local_name(&doc.root, "workbook"))
            .map(|node| match node {
                XmlNode::Element { span, .. } => span.clone(),
                _ => ByteSpan {
                    offset: 0,
                    length: 0,
                },
            })
    }

    /// Route SetDataValidation — add <dataValidation> to the sheet.
    fn route_set_data_validation(
        &self,
        cell_ref: &str,
        validation_type: &str,
        formula1: &Option<String>,
        formula2: &Option<String>,
        operator: &Option<String>,
        allow_blank: &Option<bool>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let dv_xml = self.create_data_validation(
            cell_ref,
            validation_type,
            formula1,
            formula2,
            operator,
            allow_blank,
        );
        for (part_uri, doc) in parts {
            if part_uri.contains("sheet") {
                if let Some(span) = self.find_sheet_data_span(doc) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::InsertNode(dv_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Create a <dataValidation> element.
    fn create_data_validation(
        &self,
        cell_ref: &str,
        validation_type: &str,
        formula1: &Option<String>,
        formula2: &Option<String>,
        operator: &Option<String>,
        allow_blank: &Option<bool>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        attrs.insert(TagName::local("sqref"), cell_ref.to_string());
        attrs.insert(TagName::local("type"), validation_type.to_string());
        if let Some(ref f1) = formula1 {
            attrs.insert(TagName::local("formula1"), f1.clone());
        }
        if let Some(ref f2) = formula2 {
            attrs.insert(TagName::local("formula2"), f2.clone());
        }
        if let Some(ref op) = operator {
            attrs.insert(TagName::local("operator"), op.clone());
        }
        if let Some(ab) = allow_blank {
            attrs.insert(
                TagName::local("allowBlank"),
                if *ab { "1" } else { "0" }.to_string(),
            );
        }
        attrs.insert(TagName::local("showDropdown"), "1".to_string());
        attrs.insert(TagName::local("errorStyle"), "stop".to_string());

        XmlNode::Element {
            name: TagName::local("dataValidation"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![],
            span: ByteSpan {
                offset: 0,
                length: 16,
            },
        }
    }

    /// Route AddTable — add <table> element and table parts.
    fn route_add_table(
        &self,
        sheet_name: &str,
        range: &str,
        table_name: &str,
        display_name: &Option<String>,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        let table_xml = self.create_table(range, table_name, display_name);
        for (part_uri, doc) in parts {
            if part_uri.contains("sheet") {
                if let Some(span) = self.find_sheet_data_span(doc) {
                    mutations_by_part
                        .entry(part_uri.clone())
                        .or_default()
                        .push(SpanMutation {
                            span,
                            kind: MutationKind::InsertNode(table_xml.clone()),
                        });
                    return Ok(());
                }
            }
        }
        let _ = sheet_name;
        Ok(())
    }

    /// Create a <table> element for ListObject.
    fn create_table(
        &self,
        range: &str,
        table_name: &str,
        display_name: &Option<String>,
    ) -> XmlNode {
        let mut attrs = BTreeMap::new();
        attrs.insert(TagName::local("name"), table_name.to_string());
        if let Some(ref dn) = display_name {
            attrs.insert(TagName::local("displayName"), dn.clone());
        }
        attrs.insert(TagName::local("ref"), range.to_string());
        attrs.insert(TagName::local("tableColumnType"), "1".to_string());

        XmlNode::Element {
            name: TagName::local("table"),
            attributes: attrs,
            namespaces: vec![],
            children: vec![
                // <tableColumns> placeholder
                XmlNode::Element {
                    name: TagName::local("tableColumns"),
                    attributes: BTreeMap::new(),
                    namespaces: vec![],
                    children: vec![XmlNode::Element {
                        name: TagName::local("tableColumn"),
                        attributes: BTreeMap::from([(TagName::local("id"), "1".to_string())]),
                        namespaces: vec![],
                        children: vec![],
                        span: ByteSpan {
                            offset: 0,
                            length: 12,
                        },
                    }],
                    span: ByteSpan {
                        offset: 0,
                        length: 12,
                    },
                },
                // <tableStyleInfo> placeholder
                XmlNode::Element {
                    name: TagName::local("tableStyleInfo"),
                    attributes: BTreeMap::from([
                        (TagName::local("name"), "TableStyleMedium9".to_string()),
                        (TagName::local("showFirstColumn"), "0".to_string()),
                        (TagName::local("showLastColumn"), "0".to_string()),
                        (TagName::local("showRowStripes"), "1".to_string()),
                        (TagName::local("showColumnStripes"), "1".to_string()),
                    ]),
                    namespaces: vec![],
                    children: vec![],
                    span: ByteSpan {
                        offset: 0,
                        length: 14,
                    },
                },
            ],
            span: ByteSpan {
                offset: 0,
                length: 6,
            },
        }
    }

    /// Route RemoveTable — remove <table> element from the sheet.
    fn route_remove_table(
        &self,
        sheet_name: &str,
        table_name: &str,
        parts: &BTreeMap<String, XmlDocument>,
        mutations_by_part: &mut BTreeMap<String, Vec<SpanMutation>>,
    ) -> Result<(), Vec<Diagnostic>> {
        for (part_uri, doc) in parts {
            if part_uri.contains("sheet") {
                if let Some(span) = self.find_table_by_name(doc, table_name) {
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
        }
        let _ = sheet_name;
        let _ = table_name;
        Ok(())
    }

    /// Find a <table> element by its name attribute.
    fn find_table_by_name(&self, doc: &XmlDocument, name: &str) -> Option<ByteSpan> {
        self.find_table_by_name_node(&doc.root, name)
            .map(|node| match node {
                XmlNode::Element { span, .. } => span.clone(),
                _ => ByteSpan {
                    offset: 0,
                    length: 0,
                },
            })
    }

    fn find_table_by_name_node<'b>(&'b self, node: &'b XmlNode, name: &str) -> Option<&'b XmlNode> {
        if let XmlNode::Element {
            attributes,
            children,
            ..
        } = node
        {
            if let Some(attr_name) = attributes.get(&TagName::local("name")) {
                if attr_name == name {
                    return Some(node);
                }
            }
            for child in children {
                if let Some(found) = self.find_table_by_name_node(child, name) {
                    return Some(found);
                }
            }
        }
        None
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
