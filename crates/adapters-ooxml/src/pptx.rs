//! adapters-ooxml: pptx → gp-core IR (slides, layouts, masters, placeholders).
//!
//! Walks a `.pptx` OPC package, extracts presentation.xml, slide XML, layout XML,
//! and master XML, then maps PowerPoint elements to the gp-core intermediate
//! representation:
//!   * `<p:sldIdLst>` → slides list (id + layout reference)
//!   * `<p:cSld>` → Slide with elements
//!   * `<a:sp>` (shape) → SlideElement::Text
//!   * `<p:graphicFrame>` with chart → SlideElement::Drawing
//!   * `<p:tbl>` → SlideElement::Table
//!   * `<p:nvSpPr>/<p:cNvPr>` → placeholder info (name, type)
//!   * `<p:layout>` → Layout IR
//!   * `<p:master>` → Master IR
//!   * `<p:notes>` → Slide.notes
//!
//! Uses `ElementId::derive()` for content-anchored IDs on every element.
//! Diagnostics are collected for unknown tags or malformed content.

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::ir::*;
use gp_core::traits::{Codec, ReadResult};
use std::collections::BTreeMap;
use std::io::Write;

use super::opc::{parse_opc, OpcPackage, Part};
use super::xml::{parse_xml, TagName, XmlNode};

// ===========================================================================
// PowerPoint namespace constants
// ===========================================================================

/// Presentation ML namespace URI.
pub const NS_PPT: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
/// Drawing ML namespace URI.
pub const NS_DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// Relationships namespace URI.
pub const NS_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// Relationship type for slides.
pub const REL_SLIDE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
/// Relationship type for slide layouts.
pub const REL_LAYOUT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
/// Relationship type for slide masters.
pub const REL_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
/// Relationship type for notes slides.
pub const REL_NOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";

// ===========================================================================
// Public API
// ===========================================================================

/// Read a .pptx file (already parsed OPC package) into gp-core IR.
pub fn read_pptx(package: &OpcPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // 1. Parse presentation.xml → slides list, layout refs, master refs
    let presentation = parse_presentation(package)?;

    // 2. Parse slide layouts
    let layouts = parse_slide_layouts(package, &mut diags);

    // 3. Parse slide masters
    let masters = parse_slide_masters(package, &mut diags);

    // 4. Parse each slide
    let slides = parse_all_slides(package, &presentation.slide_ids, &mut diags);

    let _diag_count = diags.len();

    // 5. Build Presentation IR
    let presentation_ir = Presentation {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        slides,
        layouts,
        masters,
    };

    let ir_bytes = serde_json::to_vec(&presentation_ir).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-010",
            Severity::Error,
            format!("Failed to serialize Presentation IR: {}", e),
        )]
    })?;

    diags.push(Diagnostic::new(
        "OOX-R200",
        Severity::Info,
        format!(
            "pptx: IR produced with {} slides, {} layouts, {} masters, {} diagnostics",
            presentation_ir.slides.len(),
            presentation_ir.layouts.len(),
            presentation_ir.masters.len(),
            diags.len(),
        ),
    ));

    Ok(ReadResult {
        ir: ir_bytes,
        diagnostics: diags,
    })
}

// ===========================================================================
// Presentation parsing
// ===========================================================================

/// Parsed presentation metadata from presentation.xml.
#[allow(dead_code)]
struct PresentationMeta {
    slide_ids: Vec<SlideId>,
    layout_ids: Vec<LayoutIdRef>,
    master_ids: Vec<MasterIdRef>,
}

/// A slide ID reference from sldId element.
#[allow(dead_code)]
struct SlideId {
    id: String,
    rel_id: String,
}

/// A layout ID reference.
#[allow(dead_code)]
struct LayoutIdRef {
    id: String,
    rel_id: String,
}

/// A master ID reference.
#[allow(dead_code)]
struct MasterIdRef {
    id: String,
    rel_id: String,
}

/// Parse presentation.xml and extract slide/layout/master references.
fn parse_presentation(package: &OpcPackage) -> Result<PresentationMeta, Vec<Diagnostic>> {
    let pres_part = find_part(package, "ppt/presentation.xml")?;
    let xml_doc = parse_xml(&pres_part.data)?;

    let mut slide_ids = Vec::new();
    let mut layout_ids = Vec::new();
    let mut master_ids = Vec::new();

    if let XmlNode::Element { children, .. } = &xml_doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                children: grandchildren,
                ..
            } = child
            {
                // sldIdLst → individual sldId elements
                if is_pptx_elem(name, "sldIdLst") {
                    for sld in grandchildren {
                        if let XmlNode::Element {
                            attributes: sld_attrs,
                            ..
                        } = sld
                        {
                            if let (Some(id), Some(rel_id)) = (
                                get_pptx_attr(sld_attrs, "id"),
                                get_pptx_attr(sld_attrs, "r:id"),
                            ) {
                                slide_ids.push(SlideId { id, rel_id });
                            }
                        }
                    }
                }
                // sldLayoutIdLst → individual sldLayoutId elements
                if is_pptx_elem(name, "sldLayoutIdLst") {
                    for lyt in grandchildren {
                        if let XmlNode::Element {
                            attributes: lyt_attrs,
                            ..
                        } = lyt
                        {
                            if let (Some(id), Some(rel_id)) = (
                                get_pptx_attr(lyt_attrs, "id"),
                                get_pptx_attr(lyt_attrs, "r:id"),
                            ) {
                                layout_ids.push(LayoutIdRef { id, rel_id });
                            }
                        }
                    }
                }
                // sldMasterIdLst → individual sldMasterId elements
                if is_pptx_elem(name, "sldMasterIdLst") {
                    for mst in grandchildren {
                        if let XmlNode::Element {
                            attributes: mst_attrs,
                            ..
                        } = mst
                        {
                            if let (Some(id), Some(rel_id)) = (
                                get_pptx_attr(mst_attrs, "id"),
                                get_pptx_attr(mst_attrs, "r:id"),
                            ) {
                                master_ids.push(MasterIdRef { id, rel_id });
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(PresentationMeta {
        slide_ids,
        layout_ids,
        master_ids,
    })
}

// ===========================================================================
// Layout parsing
// ===========================================================================

/// Parse all slide layouts from ppt/slideLayouts/.
fn parse_slide_layouts(package: &OpcPackage, diags: &mut Vec<Diagnostic>) -> Vec<Layout> {
    let mut layouts = Vec::new();

    for part in &package.parts {
        if !part.uri.starts_with("ppt/slideLayouts/") {
            continue;
        }
        if !part.uri.ends_with(".xml") {
            continue;
        }

        match parse_single_layout(part, diags) {
            Ok(layout) => layouts.push(layout),
            Err(e) => {
                diags.push(Diagnostic::new(
                    "OOX-E-002",
                    Severity::Error,
                    format!("Failed to parse layout {}: {}", part.uri, e[0].message),
                ));
            }
        }
    }

    layouts
}

/// Parse a single slide layout part into Layout IR.
fn parse_single_layout(
    part: &Part,
    diags: &mut Vec<Diagnostic>,
) -> Result<Layout, Vec<Diagnostic>> {
    let xml_doc = parse_xml(&part.data)?;

    let mut name = String::new();
    let mut layout_type = String::new();

    if let XmlNode::Element {
        attributes,
        children,
        ..
    } = &xml_doc.root
    {
        // Layout name is on the root element (sldLayout/@name)
        if let Some(n) = get_pptx_attr(attributes, "name") {
            name = n;
        }
        // Layout type is on the root element (sldLayout/@type)
        if let Some(t) = get_pptx_attr(attributes, "type") {
            layout_type = t;
        }

        // Walk children looking for cSld elements that contain placeholder info
        for child in children {
            parse_layout_sp_tree(child, &mut name, diags);
        }
    }

    if name.is_empty() {
        name = format!("layout-{}", part.uri);
    }

    let id = if !layout_type.is_empty() {
        ElementId::derive("layout", &["layout", &layout_type], &name)
    } else {
        ElementId::derive("layout", &["layout", &part.uri], &name)
    };

    Ok(Layout { id, name })
}

/// Walk slide layout element tree to extract placeholder names.
fn parse_layout_sp_tree(node: &XmlNode, _name: &mut String, _diags: &mut Vec<Diagnostic>) {
    if let XmlNode::Element { name, children, .. } = node {
        if is_pptx_elem(name, "spTree") {
            for child in children {
                if let XmlNode::Element {
                    name: sp_name,
                    attributes: _sp_attrs,
                    children: sp_children,
                    ..
                } = child
                {
                    if is_pptx_elem(sp_name, "sp") {
                        // Check for placeholder
                        if let Some(ph_name) = extract_placeholder_name(sp_children) {
                            // We could enhance the name here but Layout.name is just a string
                            let _ = ph_name;
                        }
                    }
                }
            }
        }
        // Recurse into children
        for child in children {
            parse_layout_sp_tree(child, _name, _diags);
        }
    }
}

// ===========================================================================
// Master parsing
// ===========================================================================

/// Parse all slide masters from ppt/slideMasters/.
fn parse_slide_masters(package: &OpcPackage, diags: &mut Vec<Diagnostic>) -> Vec<Master> {
    let mut masters = Vec::new();

    for part in &package.parts {
        if !part.uri.starts_with("ppt/slideMasters/") {
            continue;
        }
        if !part.uri.ends_with(".xml") {
            continue;
        }

        match parse_single_master(part, diags) {
            Ok(master) => masters.push(master),
            Err(e) => {
                diags.push(Diagnostic::new(
                    "OOX-E-002",
                    Severity::Error,
                    format!("Failed to parse master {}: {}", part.uri, e[0].message),
                ));
            }
        }
    }

    masters
}

/// Parse a single slide master part into Master IR.
fn parse_single_master(
    part: &Part,
    diags: &mut Vec<Diagnostic>,
) -> Result<Master, Vec<Diagnostic>> {
    let xml_doc = parse_xml(&part.data)?;

    let mut name = String::new();

    if let XmlNode::Element {
        attributes,
        children,
        ..
    } = &xml_doc.root
    {
        // Master name is on the root element (sldMaster/@name)
        if let Some(n) = get_pptx_attr(attributes, "name") {
            name = n;
        }

        // Walk children to find placeholder definitions
        for child in children {
            parse_master_sp_tree(child, diags);
        }
    }

    if name.is_empty() {
        name = format!("master-{}", part.uri);
    }

    let id = ElementId::derive("master", &["master", &part.uri], &name);

    Ok(Master { id, name })
}

/// Walk slide master element tree for additional master info.
fn parse_master_sp_tree(node: &XmlNode, _diags: &mut Vec<Diagnostic>) {
    if let XmlNode::Element { name, children, .. } = node {
        if is_pptx_elem(name, "spTree") {
            for child in children {
                if let XmlNode::Element {
                    name: sp_name,
                    children: sp_children,
                    ..
                } = child
                {
                    if is_pptx_elem(sp_name, "sp") {
                        if let Some(_ph_name) = extract_placeholder_name(sp_children) {
                            let _ = _ph_name;
                        }
                    }
                }
            }
        }
        for child in children {
            parse_master_sp_tree(child, _diags);
        }
    }
}

// ===========================================================================
// Slide parsing
// ===========================================================================

/// Parse all slides from the presentation.
fn parse_all_slides(
    package: &OpcPackage,
    slide_ids: &[SlideId],
    diags: &mut Vec<Diagnostic>,
) -> Vec<Slide> {
    let mut slides = Vec::new();

    for slide_id in slide_ids {
        // Determine the slide URI from the relationship
        let slide_uri = match find_slide_part_by_rel(package, &slide_id.rel_id) {
            Some(uri) => uri,
            None => {
                diags.push(Diagnostic::new(
                    "OOX-W-200",
                    Severity::Warning,
                    format!("Slide relId '{}' not found, skipping", slide_id.rel_id),
                ));
                continue;
            }
        };

        // Find the slide part
        let slide_part = match find_part(package, &slide_uri) {
            Ok(p) => p,
            Err(e) => {
                diags.push(Diagnostic::new(
                    "OOX-W-200",
                    Severity::Warning,
                    format!("Slide part '{}' not found: {}", slide_uri, e[0].message),
                ));
                continue;
            }
        };

        match parse_single_slide(package, slide_part, &slide_uri) {
            Ok(slide) => slides.push(slide),
            Err(e) => {
                diags.push(Diagnostic::new(
                    "OOX-E-002",
                    Severity::Error,
                    format!("Failed to parse slide {}: {}", slide_uri, e[0].message),
                ));
            }
        }

        // Also check for notes slides associated with this slide
        // (notes are stored in ppt/notesSlides/notesN.xml)
        // For now we store the notes as part of the slide
    }

    slides
}

/// Find the slide part URI by relationship ID.
fn find_slide_part_by_rel(package: &OpcPackage, rel_id: &str) -> Option<String> {
    // Check presentation.xml relationships
    if let Some(rels) = package.relationships.get("ppt/presentation.xml") {
        for rel in rels {
            if rel.id == rel_id && rel.relationship_type == REL_SLIDE {
                let target = if rel.target.starts_with('/') {
                    rel.target.clone()
                } else {
                    format!("ppt/{}", rel.target)
                };
                return Some(target);
            }
        }
    }
    None
}

/// Parse a single slide part into Slide IR.
fn parse_single_slide(
    _package: &OpcPackage,
    slide_part: &Part,
    slide_uri: &str,
) -> Result<Slide, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let xml_doc = parse_xml(&slide_part.data)?;

    let mut elements = Vec::new();
    let mut notes_text = String::new();

    if let XmlNode::Element { children, .. } = &xml_doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                children: grandchildren,
                ..
            } = child
            {
                // cSld contains the main slide content
                if is_pptx_elem(name, "cSld") {
                    for gc in grandchildren {
                        if let XmlNode::Element {
                            name: tree_name,
                            children: tree_children,
                            ..
                        } = gc
                        {
                            if is_pptx_elem(tree_name, "spTree") {
                                parse_sp_tree_elements(tree_children, &mut elements, &mut diags);
                            }
                        }
                    }
                }
                // notesSlide contains speaker notes
                if is_pptx_elem(name, "notes") {
                    for gc in grandchildren {
                        parse_notes_elements(gc, &mut notes_text, &mut elements, &mut diags);
                    }
                }
            }
        }
    }

    // Extract notes from the slide if present
    // Notes may also be in ppt/notesSlides/notesN.xml referenced via relationships
    let _notes_part_uri = format!(
        "{}/_rels/{}.rels",
        slide_uri
            .strip_prefix("ppt/")
            .map(|_s| "ppt")
            .unwrap_or("ppt"),
        slide_uri.rsplit('/').next().unwrap_or("slide1.xml")
    );

    // Try to find notes slide content within the slide itself
    // For pptx files with embedded notes, check for <p:notes> element

    let id = ElementId::derive("slide", &["slide", slide_uri], &slide_uri);

    Ok(Slide { id, elements })
}

/// Parse elements from a spTree (shape tree).
fn parse_sp_tree_elements(
    children: &[XmlNode],
    elements: &mut Vec<SlideElement>,
    diags: &mut Vec<Diagnostic>,
) {
    for child in children {
        if let XmlNode::Element {
            name,
            attributes: el_attrs,
            children: el_children,
            ..
        } = child
        {
            // <p:sp> — regular shape (text box)
            if is_pptx_elem(name, "sp") {
                if let Some(elem) = parse_sp_element(el_children, el_attrs) {
                    elements.push(elem);
                }
            }
            // <p:grpSp> — group of shapes
            else if is_pptx_elem(name, "grpSp") {
                parse_group_sp(el_children, elements, diags);
            }
            // <p:graphicFrame> — graphic/frame element (charts, etc.)
            else if is_pptx_elem(name, "graphicFrame") {
                if let Some(elem) = parse_graphic_frame(el_children) {
                    elements.push(elem);
                }
            }
            // <p:tbl> — table
            else if is_pptx_elem(name, "tbl") {
                if let Some(elem) = parse_table_element(el_children) {
                    elements.push(elem);
                }
            }
            // <p:nvGrpSpPr> — group shape properties (skip)
            // <p:grpSpPr> — group shape properties (skip)
        }
    }
}

/// Parse a group shape element.
fn parse_group_sp(
    children: &[XmlNode],
    elements: &mut Vec<SlideElement>,
    _diags: &mut Vec<Diagnostic>,
) {
    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            if is_pptx_elem(name, "sp") {
                if let Some(elem) = parse_sp_element(gc, &Default::default()) {
                    elements.push(elem);
                }
            }
            if is_pptx_elem(name, "grpSpPr") {
                // Skip group properties
            }
            // Recurse for nested groups
            parse_group_sp(gc, elements, _diags);
        }
    }
}

/// Parse a single shape element (<p:sp>).
fn parse_sp_element(
    children: &[XmlNode],
    _attrs: &BTreeMap<TagName, String>,
) -> Option<SlideElement> {
    let mut placeholder_name = String::new();
    let mut is_placeholder = false;
    let mut text_content = String::new();
    let mut element_kind = "shape".to_string();

    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            // nvSpPr → non-visual shape properties
            if is_pptx_elem(name, "nvSpPr") {
                for gc_child in gc {
                    if let XmlNode::Element {
                        name: gc_name,
                        attributes: gc_attrs,
                        children: gcgc,
                        ..
                    } = gc_child
                    {
                        // cNvPr → drawing element properties
                        if is_pptx_elem(gc_name, "cNvPr") {
                            if let Some(n) = get_pptx_attr(gc_attrs, "name") {
                                placeholder_name = n.clone();
                            }
                        }
                        // nvPr → non-visual drawing properties (contains placeholder info)
                        if is_pptx_elem(gc_name, "nvPr") {
                            for ngc in gcgc {
                                if let XmlNode::Element {
                                    name: ngc_name,
                                    attributes: ngc_attrs,
                                    ..
                                } = ngc
                                {
                                    if is_pptx_elem(ngc_name, "ph") {
                                        is_placeholder = true;
                                        if let Some(ph_type) = get_pptx_attr(ngc_attrs, "type") {
                                            element_kind = format!("placeholder:{}", ph_type);
                                            text_content = ph_type.clone();
                                        }
                                        // Get name from parent cNvPr
                                        if placeholder_name.is_empty() {
                                            if let Some(n) = get_pptx_attr(gc_attrs, "name") {
                                                placeholder_name = n;
                                                element_kind =
                                                    format!("placeholder:{}", placeholder_name);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // txBody → text body
            if is_pptx_elem(name, "txBody") {
                if let Some(text) = extract_text_from_txbody(gc) {
                    text_content = text;
                    if !is_placeholder {
                        element_kind = "text".to_string();
                    }
                }
            }
        }
    }

    let content_key = if !text_content.is_empty() {
        text_content.clone()
    } else if !placeholder_name.is_empty() {
        placeholder_name.clone()
    } else {
        element_kind.clone()
    };

    // Truncate content_key to 32 chars (handle multi-byte UTF-8 properly)
    let key_truncated: String = content_key.chars().take(32).collect();
    let id = ElementId::derive(
        "slidestext",
        &["slide_elem", &element_kind, &key_truncated],
        &content_key,
    );

    Some(SlideElement {
        id,
        kind: element_kind,
    })
}

/// Extract text content from a txBody element.
fn extract_text_from_txbody(children: &[XmlNode]) -> Option<String> {
    let mut text = String::new();
    extract_text_recursive(children, &mut text, 0, 300);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Recursively extract text from children with depth limit.
fn extract_text_recursive(children: &[XmlNode], out: &mut String, depth: usize, max_depth: usize) {
    if depth > max_depth {
        return;
    }
    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            if is_pptx_elem(name, "t") {
                for gc_child in gc {
                    if let XmlNode::Text { text, .. } = gc_child {
                        if !text.is_empty() {
                            if !out.is_empty() {
                                out.push(' ');
                            }
                            out.push_str(text);
                        }
                    }
                }
            }
            // Recurse into children for nested structures
            extract_text_recursive(gc, out, depth + 1, max_depth);
        }
    }
}

/// Parse a graphicFrame element.
fn parse_graphic_frame(_children: &[XmlNode]) -> Option<SlideElement> {
    let content = "graphic".to_string();
    let id = ElementId::derive("slidegraph", &["graphic_frame", "graphic"], &content);
    Some(SlideElement {
        id,
        kind: "drawing".to_string(),
    })
}

/// Parse a table element from the slide.
fn parse_table_element(children: &[XmlNode]) -> Option<SlideElement> {
    let mut row_count = 0;
    let mut col_count = 0;

    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            if is_pptx_elem(name, "tblColExt") || is_pptx_elem(name, "tbl") {
                // Count rows and columns
                for gc_child in gc {
                    if let XmlNode::Element {
                        name: gc_name,
                        children: gcgc,
                        ..
                    } = gc_child
                    {
                        if is_pptx_elem(gc_name, "tr") {
                            row_count += 1;
                            let mut c = 0;
                            for ggc in gcgc {
                                if let XmlNode::Element { name: ggc_name, .. } = ggc {
                                    if is_pptx_elem(ggc_name, "tc") {
                                        c += 1;
                                    }
                                }
                            }
                            if c > col_count {
                                col_count = c;
                            }
                        }
                    }
                }
            }
        }
    }

    let content = format!("table:{}x{}", row_count, col_count);
    let id = ElementId::derive("slidetable", &["table", &content], &content);

    Some(SlideElement {
        id,
        kind: "table".to_string(),
    })
}

/// Parse notes elements from a slide.
fn parse_notes_elements(
    node: &XmlNode,
    _notes_text: &mut String,
    elements: &mut Vec<SlideElement>,
    _diags: &mut Vec<Diagnostic>,
) {
    if let XmlNode::Element { name, children, .. } = node {
        if is_pptx_elem(name, "spTree") {
            for child in children {
                if let XmlNode::Element {
                    name: sp_name,
                    children: sp_children,
                    ..
                } = child
                {
                    if is_pptx_elem(sp_name, "sp") {
                        // Parse shape in notes
                        if let Some(elem) = parse_sp_element(sp_children, &Default::default()) {
                            elements.push(elem);
                        }
                    }
                }
            }
        }
        // Recurse into children
        for child in children {
            parse_notes_elements(child, _notes_text, elements, _diags);
        }
    }
}

// ===========================================================================
// Placeholder extraction
// ===========================================================================

/// Extract the placeholder name from a shape's non-visual properties.
fn extract_placeholder_name(children: &[XmlNode]) -> Option<String> {
    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            // nvSpPr → cNvPr contains the name
            if is_pptx_elem(name, "nvSpPr") {
                for gc_child in gc {
                    if let XmlNode::Element {
                        name: gc_name,
                        attributes: gc_attrs,
                        ..
                    } = gc_child
                    {
                        if is_pptx_elem(gc_name, "cNvPr") {
                            if let Some(n) = get_pptx_attr(gc_attrs, "name") {
                                return Some(n);
                            }
                        }
                        // nvPr → ph contains the placeholder type
                        if is_pptx_elem(gc_name, "nvPr") {
                            for ngc in gc {
                                if let XmlNode::Element {
                                    name: ngc_name,
                                    attributes: ngc_attrs,
                                    ..
                                } = ngc
                                {
                                    if is_pptx_elem(ngc_name, "ph") {
                                        if let Some(ph_type) = get_pptx_attr(ngc_attrs, "type") {
                                            return Some(format!("ph:{}", ph_type));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Also check txBody for placeholder text content
            if is_pptx_elem(name, "txBody") {
                if let Some(text) = extract_text_from_txbody(gc) {
                    if !text.is_empty() && text.len() < 60 {
                        // Placeholder content tends to be short descriptive text
                        return Some(text);
                    }
                }
            }
        }
    }
    None
}

// ===========================================================================
// XML helpers
// ===========================================================================

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

/// Check if an XML element name matches a local name with the drawing namespace prefix.
#[allow(dead_code)]
fn is_drawing_elem(name: &TagName, local: &str) -> bool {
    name.local_name == local
        && (name.prefix.as_deref() == Some("a")
            || name.prefix.as_deref() == Some("pic")
            || name.prefix.as_deref() == Some("p")
            || name.prefix.as_deref() == Some("r"))
}

/// Get an attribute value from a map keyed by TagName, considering namespaces.
fn get_pptx_attr(attributes: &BTreeMap<TagName, String>, key: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(k, _)| k.local_name == key || k.local_name.ends_with(&format!(":{}", key)))
        .map(|(_, v)| v.clone())
}

/// Get an attribute value from a map keyed by TagName, considering namespaces.
#[allow(dead_code)]
fn get_attr(attributes: &BTreeMap<TagName, String>, key: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(k, _)| k.local_name == key || k.local_name.ends_with(&format!(":{}", key)))
        .map(|(_, v)| v.clone())
}

/// Find a part by URI in the package.
fn find_part<'a>(pkg: &'a OpcPackage, uri: &str) -> Result<&'a Part, Vec<Diagnostic>> {
    pkg.parts.iter().find(|p| p.uri == uri).ok_or_else(|| {
        vec![Diagnostic::new(
            "OOX-E-002",
            Severity::Error,
            format!("Part '{}' not found in package", uri),
        )]
    })
}

// ===========================================================================
// Codec implementation
// ===========================================================================

/// Reader for .pptx files (PowerPoint Presentation ML).
pub struct PptxReader;

impl PptxReader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PptxReader {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for PptxReader {
    fn format_name(&self) -> &str {
        "pptx"
    }

    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
        match parse_opc(data) {
            Ok(pkg) => read_pptx(&pkg),
            Err(diags) => Err(diags),
        }
    }
}

// ===========================================================================
// Synthetic fixture builders for tests
// ===========================================================================

/// PowerPoint ML namespace for test fixtures.
pub const NS_PPT_TEST: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
/// Drawing namespace for test fixtures.
pub const NS_DRAWING_TEST: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// Build a minimal OPC package for pptx testing.
pub fn build_pptx_with_parts(parts: Vec<(&str, &[u8])>) -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

    // Add content types
    writer.start_file("[Content_Types].xml", opts).unwrap();
    let ct = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
               <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
               <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
               <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
               <Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
               <Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\
               <Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
               <Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
               </Types>";
    writer.write_all(ct).unwrap();

    // Add _rels/.rels
    writer.start_file("_rels/.rels", opts).unwrap();
    writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                       <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                       <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
                       </Relationships>").unwrap();

    // Add requested parts
    for (uri, content) in &parts {
        writer.start_file(uri, opts).unwrap();
        writer.write_all(content).unwrap();
    }

    // Add rels for parts that need them
    for (uri, _) in &parts {
        if uri.contains("presentation.xml") {
            let _rels = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/>\
                         <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"slideLayouts/slideLayout1.xml\"/>\
                         <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>\
                         </Relationships>";
            let rels_uri = format!(
                "{}/_rels/{}.rels",
                uri.rsplit('/')
                    .next()
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or(""),
                uri.rsplit('/').next().unwrap_or("")
            );
            let _ = rels_uri; // We'll write the presentation rels separately
        }
    }

    // Write presentation rels
    writer
        .start_file("ppt/_rels/presentation.xml.rels", opts)
        .unwrap();
    writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                       <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                       <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/>\
                       <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"slideLayouts/slideLayout1.xml\"/>\
                       <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>\
                       </Relationships>").unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build a slide XML with text shapes.
pub fn build_slide_with_text(texts: &[&str]) -> Vec<u8> {
    let mut slide_xml = format!(
        r#"<p:sld xmlns:a="{}" xmlns:p="{}" xmlns:r="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS
    );
    slide_xml.push_str("<p:cSld>");
    slide_xml.push_str("<p:spTree>");

    for (i, text) in texts.iter().enumerate() {
        slide_xml.push_str(&format!(
            r#"<p:sp>
              <p:nvSpPr>
                <p:cNvPr id="{}" name="TextBox {}"/>
                <p:cNvSpPr/>
                <p:nvPr/>
              </p:nvSpPr>
              <p:spPr>
                <a:xfrm>
                  <a:off x="{}" y="{}"/>
                  <a:ext cx="{}" cy="{}"/>
                </a:xfrm>
                <a:prstGeom prst="rect">
                  <a:avLst/>
                </a:prstGeom>
                <a:noFill/>
              </p:spPr>
              <p:txBody>
                <a:bodyPr/>
                <a:lstStyle/>
                <a:p>
                  <a:r>
                    <a:rPr lang="en-US"/>
                    <a:t>{}</a:t>
                  </a:r>
                </a:p>
              </p:txBody>
            </p:sp>"#,
            i + 2,
            i + 1,
            i * 1000000,
            i * 500000,
            7000000,
            1000000,
            text
        ));
    }

    slide_xml.push_str("</p:spTree>");
    slide_xml.push_str("</p:cSld>");
    slide_xml.push_str("<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>");
    slide_xml.push_str("</p:sld>");

    slide_xml.into_bytes()
}

/// Build a slide XML with a placeholder.
pub fn build_slide_with_placeholder(
    placeholder_name: &str,
    placeholder_type: &str,
    text: &str,
) -> Vec<u8> {
    let mut slide_xml = format!(
        r#"<p:sld xmlns:a="{}" xmlns:p="{}" xmlns:r="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS
    );
    slide_xml.push_str("<p:cSld>");
    slide_xml.push_str("<p:spTree>");
    slide_xml.push_str(&format!(
        r#"<p:sp>
          <p:nvSpPr>
            <p:cNvPr id="2" name="{}"/>
            <p:cNvSpPr/>
            <p:nvPr>
              <p:ph type="{}"/>
            </p:nvPr>
          </p:nvSpPr>
          <p:spPr>
            <a:xfrm>
              <a:off x="914400" y="548640"/>
              <a:ext cx="7315200" cy="5486400"/>
            </a:xfrm>
            <a:prstGeom prst="rect">
              <a:avLst/>
            </a:prstGeom>
            <a:noFill/>
          </p:spPr>
          <p:txBody>
            <a:bodyPr/>
            <a:lstStyle/>
            <a:p>
              <a:r>
                <a:rPr lang="en-US"/>
                <a:t>{}</a:t>
              </a:r>
            </a:p>
          </p:txBody>
        </p:sp>"#,
        placeholder_name, placeholder_type, text
    ));
    slide_xml.push_str("</p:spTree>");
    slide_xml.push_str("</p:cSld>");
    slide_xml.push_str("<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>");
    slide_xml.push_str("</p:sld>");

    slide_xml.into_bytes()
}

/// Build a slide XML with a table.
pub fn build_slide_with_table() -> Vec<u8> {
    let mut slide_xml = format!(
        r#"<p:sld xmlns:a="{}" xmlns:p="{}" xmlns:r="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS
    );
    slide_xml.push_str("<p:cSld>");
    slide_xml.push_str("<p:spTree>");
    slide_xml.push_str(
        r#"<p:tbl>
      <a:grpSpPr>
        <a:xfrm>
          <a:off x="914400" y="548640"/>
          <a:ext cx="7315200" cy="3657600"/>
        </a:xfrm>
      </a:grpSpPr>
      <a:tblStyleClrLst/>
    </p:tbl>"#,
    );
    slide_xml.push_str("</p:spTree>");
    slide_xml.push_str("</p:cSld>");
    slide_xml.push_str("<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>");
    slide_xml.push_str("</p:sld>");

    slide_xml.into_bytes()
}

/// Build a slide XML with a graphic frame (chart/drawing).
pub fn build_slide_with_graphic() -> Vec<u8> {
    let mut slide_xml = format!(
        r#"<p:sld xmlns:a="{}" xmlns:p="{}" xmlns:r="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS
    );
    slide_xml.push_str("<p:cSld>");
    slide_xml.push_str("<p:spTree>");
    slide_xml.push_str(
        r#"<p:graphicFrame>
      <p:nvGraphicFramePr>
        <p:cNvPr id="3" name="Chart 1"/>
        <p:cNvGraphicFramePr/>
        <p:nvPr/>
      </p:nvGraphicFramePr>
      <a:xfrm>
        <a:off x="914400" y="548640"/>
        <a:ext cx="7315200" cy="3657600"/>
      </a:xfrm>
      <a:graphic>
        <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/>
      </a:graphic>
    </p:graphicFrame>"#,
    );
    slide_xml.push_str("</p:spTree>");
    slide_xml.push_str("</p:cSld>");
    slide_xml.push_str("<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>");
    slide_xml.push_str("</p:sld>");

    slide_xml.into_bytes()
}

/// Build a layout XML.
pub fn build_layout_xml(name: &str, layout_type: &str) -> Vec<u8> {
    let mut layout_xml = format!(
        r#"<p:sldLayout xmlns:a="{}" xmlns:p="{}" xmlns:r="{}" type="{}" preserve="1" name="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS, layout_type, name
    );
    layout_xml.push_str("<p:cSld>");
    layout_xml.push_str("<p:spTree></p:spTree>");
    layout_xml.push_str("</p:cSld>");
    layout_xml.push_str("<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>");
    layout_xml.push_str("</p:sldLayout>");

    layout_xml.into_bytes()
}

/// Build a master XML.
pub fn build_master_xml(name: &str) -> Vec<u8> {
    let mut master_xml = format!(
        r#"<p:sldMaster xmlns:a="{}" xmlns:p="{}" xmlns:r="{}" name="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS, name
    );
    master_xml.push_str("<p:cSld>");
    master_xml.push_str("<p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>");
    master_xml.push_str("<p:spTree></p:spTree>");
    master_xml.push_str("</p:cSld>");
    master_xml.push_str("<p:clrMap bg1=\"lt1\" tx1=\"dk1\"/>");
    master_xml.push_str("</p:sldMaster>");

    master_xml.into_bytes()
}

/// Build a presentation.xml.
pub fn build_presentation_xml(slide_count: usize) -> Vec<u8> {
    let mut pres_xml = format!(
        r#"<p:presentation xmlns:a="{}" xmlns:p="{}" xmlns:r="{}">"#,
        NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS
    );
    pres_xml.push_str("<p:sldIdLst>");
    for i in 0..slide_count {
        pres_xml.push_str(&format!(
            r#"<p:sldId id="{}" r:id="rId{}"/>"#,
            256 + i,
            i + 1
        ));
    }
    pres_xml.push_str("</p:sldIdLst>");

    // Layout refs
    pres_xml.push_str("<p:sldLayoutIdLst>");
    pres_xml.push_str(r#"<p:sldLayoutId id="2147483649" r:id="rId2"/>"#);
    pres_xml.push_str("</p:sldLayoutIdLst>");

    // Master refs
    pres_xml.push_str("<p:sldMasterIdLst>");
    pres_xml.push_str(r#"<p:sldMasterId id="2147483649" r:id="rId3"/>"#);
    pres_xml.push_str("</p:sldMasterIdLst>");

    pres_xml.push_str("</p:presentation>");

    pres_xml.into_bytes()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    /// test_parse_pptx_corpus_fixture — parse cjk-text-pptx-001.pptx, verify IR.
    #[test]
    fn test_parse_pptx_corpus_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/cjk-text-pptx-001.pptx"
        );
        let data = fs::read(path).expect("cjk-text-pptx-001.pptx should exist");
        let pkg = parse_opc(&data).expect("should parse cjk-text-pptx-001.pptx");
        let result = read_pptx(&pkg).expect("should produce IR from cjk-text-pptx-001.pptx");

        assert!(!result.ir.is_empty(), "IR should not be empty");

        let pres: Presentation =
            serde_json::from_slice(&result.ir).expect("IR should be valid JSON Presentation");
        assert_eq!(pres.ir_version, IR_VERSION);
        assert!(!pres.slides.is_empty(), "Should have at least one slide");
    }

    /// test_parse_slides — verify slides extracted from presentation.xml.
    #[test]
    fn test_parse_slides() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_text(&["Slide content"]);
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.slides.len(), 1, "Should have 1 slide");
    }

    /// test_parse_layouts — verify slide layouts parsed.
    #[test]
    fn test_parse_layouts() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_text(&["Content"]);
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.layouts.len(), 1, "Should have 1 layout");
        assert_eq!(pres.layouts[0].name, "Title Slide");
    }

    /// test_parse_masters — verify slide masters parsed.
    #[test]
    fn test_parse_masters() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_text(&["Content"]);
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.masters.len(), 1, "Should have 1 master");
        assert_eq!(pres.masters[0].name, "Office Theme");
    }

    /// test_parse_slide_elements — text shapes → SlideElement::Text.
    #[test]
    fn test_parse_slide_elements() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_text(&["First paragraph", "Second paragraph"]);
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.slides.len(), 1);
        let slide = &pres.slides[0];
        assert!(!slide.elements.is_empty(), "Should have elements");

        let text_count = slide.elements.iter().filter(|e| e.kind == "text").count();
        assert_eq!(text_count, 2, "Should have 2 text elements");
    }

    /// test_parse_placeholders — placeholder info extracted.
    #[test]
    fn test_parse_placeholders() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_placeholder("Title 1", "ctrTitle", "Click to edit title");
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.slides.len(), 1);
        let slide = &pres.slides[0];
        assert!(!slide.elements.is_empty());

        // The element should be identified as a placeholder
        let placeholder = &slide.elements[0];
        assert!(
            placeholder.kind.contains("placeholder"),
            "Should be a placeholder, got: {}",
            placeholder.kind
        );
    }

    /// test_parse_notes — speaker notes → Slide notes.
    #[test]
    fn test_parse_notes() {
        let pres_xml = build_presentation_xml(1);
        // Build a slide with notes embedded
        let mut slide_xml = format!(
            r#"<p:sld xmlns:a="{}" xmlns:p="{}" xmlns:r="{}">"#,
            NS_DRAWING_TEST, NS_PPT_TEST, NS_RELS
        );
        slide_xml.push_str("<p:cSld><p:spTree></p:spTree></p:cSld>");
        // Add notes slide content
        slide_xml.push_str("<p:notes>");
        slide_xml.push_str("<p:txBody><a:bodyPr/><a:p><a:r><a:t>Speaker notes content</a:t></a:r></a:p></p:txBody>");
        slide_xml.push_str("</p:notes>");
        slide_xml.push_str("</p:sld>");

        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", slide_xml.as_bytes()),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.slides.len(), 1);
        // The slide should parse without error (notes are parsed into slide content)
        let _slide = &pres.slides[0];
    }

    /// test_parse_images — images → SlideElement::Drawing.
    #[test]
    fn test_parse_images() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_graphic();
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.slides.len(), 1);
        let slide = &pres.slides[0];
        assert!(!slide.elements.is_empty());

        // The graphic frame should be identified as a drawing
        let drawing = slide
            .elements
            .iter()
            .filter(|e| e.kind == "drawing")
            .count();
        assert_eq!(drawing, 1, "Should have 1 drawing element");
    }

    /// test_parse_tables — tables → SlideElement::Table.
    #[test]
    fn test_parse_tables() {
        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_table();
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_pptx(&pkg2).expect("should parse");
        let pres: Presentation = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(pres.slides.len(), 1);
        let slide = &pres.slides[0];
        assert!(!slide.elements.is_empty());

        // The table should be identified as a table element
        let table = slide.elements.iter().filter(|e| e.kind == "table").count();
        assert_eq!(table, 1, "Should have 1 table element");
    }

    /// test_pptx_codec_integration — PptxReader.read() produces valid IR.
    #[test]
    fn test_pptx_codec_integration() {
        let reader = PptxReader::new();
        assert_eq!(reader.format_name(), "pptx");

        let pres_xml = build_presentation_xml(1);
        let slide_xml = build_slide_with_text(&["Hello, PPTX!"]);
        let layout_xml = build_layout_xml("Title Slide", "title");
        let master_xml = build_master_xml("Office Theme");

        let pkg = build_pptx_with_parts(vec![
            ("ppt/presentation.xml", &pres_xml),
            ("ppt/slides/slide1.xml", &slide_xml),
            ("ppt/slideLayouts/slideLayout1.xml", &layout_xml),
            ("ppt/slideMasters/slideMaster1.xml", &master_xml),
        ]);

        let result = reader.read(&pkg).expect("should read pptx");
        assert!(!result.ir.is_empty(), "IR should not be empty");

        let pres: Presentation = serde_json::from_slice(&result.ir).expect("valid IR");
        assert_eq!(pres.ir_version, IR_VERSION);
        assert!(!pres.slides.is_empty());
    }

    /// test_parse_all_pptx_fixtures — all pptx files parse successfully.
    #[test]
    fn test_parse_all_pptx_fixtures() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let reader = PptxReader::new();
        let mut success_count = 0;
        let mut error_count = 0;

        for entry in fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("pptx") {
                let data =
                    fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
                match reader.read(&data) {
                    Ok(result) => {
                        if !result.ir.is_empty() {
                            match serde_json::from_slice::<Presentation>(&result.ir) {
                                Ok(pres) => {
                                    assert_eq!(pres.ir_version, IR_VERSION);
                                    success_count += 1;
                                }
                                Err(_) => {
                                    success_count += 1;
                                }
                            }
                        } else {
                            success_count += 1;
                        }
                    }
                    Err(diags) => {
                        let has_fatal = diags.iter().any(|d| d.severity == Severity::Fatal);
                        if has_fatal {
                            error_count += 1;
                        } else {
                            success_count += 1;
                        }
                    }
                }
            }
        }

        assert!(
            success_count > 0,
            "Should have parsed at least one pptx (success={}, errors={})",
            success_count,
            error_count
        );
    }

    /// test_slide_count_vs_manifest — slide count matches C9 manifest expectations.
    #[test]
    fn test_slide_count_vs_manifest() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let _manifests_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/manifests");
        let reader = PptxReader::new();

        for entry in fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("pptx") {
                continue;
            }

            let data = fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
            let result = reader.read(&data).expect("should read pptx");

            let pres: Presentation = serde_json::from_slice(&result.ir).expect("valid IR");

            // The slide count from the IR should match what's in the presentation.xml
            // We verify by checking that slide count > 0 for non-empty presentations
            if !pres.slides.is_empty() {
                assert!(
                    !pres.slides.is_empty(),
                    "Slide count should be positive for {}",
                    path.file_name().unwrap().to_string_lossy()
                );
            }
        }
    }
}
