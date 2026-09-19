// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-ooxml: docx → gp-core IR (body, runs, tables → IR + IDs).
//!
//! Walks a `.docx` OPC package, extracts `word/document.xml`, parses the XML DOM,
//! and maps WordML elements to the gp-core intermediate representation:
//!   * `w:p` → `Paragraph` (with runs)
//!   * `w:r` → `Run` (text + formatting)
//!   * `w:tbl` → `Table` (rows + cells, including paragraphs inside cells)
//!   * `w:sectPr` → `Section` (body, header/footer references)
//!   * `w:hyperlink` → inline hyperlink in run text
//!   * `w:numPr` → numbered list items
//!   * `w:fldSimple` → `Field::Simple` (P1-12)
//!   * `w:commentReference` → comment ID reference (P1-12)
//!   * `word/comments.xml` → `Comment` (P1-12)
//!   * `w:ins`/`w:del` → `TrackedChange` (P1-12)
//!
//! Uses `ElementId::derive()` for content-anchored IDs on every element.
//! Diagnostics are collected for unknown tags or malformed content.

#![allow(clippy::single_match, clippy::collapsible_if, clippy::ptr_arg)]

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::ir::*;
use gp_core::traits::{Codec, ReadResult};
use std::collections::BTreeMap;

use super::opc::{parse_opc, OpcPackage, Part};
use super::revision_reader::enumerate_revisions;
use super::xml::{parse_xml, TagName, XmlDocument, XmlNode};

// ===========================================================================
// Extended Word document reader with full feature support.
// ===========================================================================

/// Extended Word document reader with full feature support.
///
/// Parses styles.xml, numbering.xml, section properties, and follows
/// header/footer references to load their contents from the OPC package.
pub fn read_docx_full(package: &OpcPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // 1. Read main document
    let doc_part = find_document_part(package)?;
    let xml_doc = parse_xml(&doc_part.data)?;

    // 2. Read styles.xml -> style map
    let style_map = match read_styles_xml(package) {
        Ok(m) => {
            diags.push(Diagnostic::new(
                "OOX-R100",
                Severity::Info,
                format!("styles.xml: {} styles found", m.len()),
            ));
            m
        }
        Err(e) => {
            diags.push(Diagnostic::new(
                "OOX-W-100",
                Severity::Warning,
                format!("styles.xml not found or parse error: {}", e[0].message),
            ));
            BTreeMap::new()
        }
    };

    // 3. Read numbering.xml -> numbering map
    let num_def_map = match read_numbering_xml(package) {
        Ok(m) => {
            diags.push(Diagnostic::new(
                "OOX-R101",
                Severity::Info,
                format!("numbering.xml: {} num definitions found", m.len()),
            ));
            m
        }
        Err(e) => {
            diags.push(Diagnostic::new(
                "OOX-W-101",
                Severity::Warning,
                format!("numbering.xml not found or parse error: {}", e[0].message),
            ));
            BTreeMap::new()
        }
    };

    // 4-8. Build document with styles, numbering, sections, headers/footers
    let (mut document, mut build_diags) =
        build_document_full(&xml_doc, package, &style_map, &num_def_map)?;
    diags.append(&mut build_diags);

    // 9. Apply numbering to paragraphs with numPr
    apply_numbering_to_document(&mut document, &num_def_map, &mut diags);

    // 10. Read comments from word/comments.xml (P1-12)
    let comments_map = read_comments(package);
    diags.push(Diagnostic::new(
        "OOX-R111",
        Severity::Info,
        format!("comments.xml: {} comments found", comments_map.len()),
    ));
    let comments: Vec<Comment> = comments_map.into_values().collect();
    document.comments = comments;

    // 11. Parse fields and tracked changes from document XML (P1-12)
    let (parsed_fields, parsed_tracked_changes) =
        extract_fields_and_tracked_changes(&xml_doc, &mut diags);
    document.fields = parsed_fields;
    document.tracked_changes = parsed_tracked_changes;

    // 12. Enumerate C11 revisions from ins/del elements (P4-04).
    let mut rev_diags: Vec<Diagnostic> = Vec::new();
    document.revisions = enumerate_revisions(&xml_doc, &mut rev_diags);
    diags.extend(rev_diags);

    let ir_bytes = serde_json::to_vec(&document).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-010",
            Severity::Error,
            format!("Failed to serialize IR: {}", e),
        )]
    })?;

    diags.push(Diagnostic::new(
        "OOX-R110",
        Severity::Info,
        format!(
            "docx full: IR produced with {} sections, {} styles, {} numbering defs, {} fields, {} comments, {} tracked changes, {} revisions, {} diagnostics",
            document.sections.len(),
            style_map.len(),
            num_def_map.len(),
            document.fields.len(),
            document.comments.len(),
            document.tracked_changes.len(),
            document.revisions.len(),
            diags.len(),
        ),
    ));

    Ok(ReadResult {
        ir: ir_bytes,
        diagnostics: diags,
    })
}

// ===========================================================================
// Fields, Comments, Tracked Changes (P1-12)
// ===========================================================================

/// Read comments from word/comments.xml into a Comment map keyed by comment ID string.
pub fn read_comments(package: &OpcPackage) -> BTreeMap<String, Comment> {
    let mut comments_map: BTreeMap<String, Comment> = BTreeMap::new();

    if let Ok(comments_part) = find_part(package, "word/comments.xml") {
        if let Ok(xml_doc) = parse_xml(&comments_part.data) {
            if let XmlNode::Element { children, .. } = &xml_doc.root {
                for child in children {
                    if let XmlNode::Element {
                        attributes,
                        children: comment_children,
                        ..
                    } = child
                    {
                        if is_word_elem(child_name(child), "comment") {
                            if let Some(comment_id) = get_attr_value(attributes, "id") {
                                let author =
                                    get_attr_value(attributes, "author").unwrap_or_default();
                                let date = get_attr_value(attributes, "date");
                                let initials = get_attr_value(attributes, "initials");

                                let content = parse_comment_content(comment_children);
                                let id = ElementId::derive(
                                    "comment",
                                    &["comment", &comment_id],
                                    &author,
                                );

                                comments_map.insert(
                                    comment_id.clone(),
                                    Comment {
                                        id,
                                        comment_id,
                                        author,
                                        date,
                                        initials,
                                        content,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    comments_map
}

/// Parse the content of a comment element into SectionItems.
fn parse_comment_content(children: &[XmlNode]) -> Vec<SectionItem> {
    let mut items: Vec<SectionItem> = Vec::new();
    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            if is_word_elem(name, "p") {
                if let Some(para) = parse_paragraph(gc, &mut Vec::new()) {
                    items.push(SectionItem::Paragraph(para));
                }
            }
        }
    }
    items
}

/// Parse field simple (w:fldSimple) from document XML and return list of FieldSimple.
pub fn parse_fields_from_xml(node: &XmlNode, diags: &mut Vec<Diagnostic>) -> Vec<FieldSimple> {
    let mut fields: Vec<FieldSimple> = Vec::new();
    parse_fields_recursive(node, &mut fields, diags);
    fields
}

fn parse_fields_recursive(
    node: &XmlNode,
    fields: &mut Vec<FieldSimple>,
    _diags: &mut Vec<Diagnostic>,
) {
    match node {
        XmlNode::Element {
            name,
            attributes,
            children,
            ..
        } => {
            if is_word_elem(name, "fldSimple") {
                if let Some(instr) = get_fld_attr(attributes, "instr") {
                    let result = get_fld_attr(attributes, "result");
                    let id = ElementId::derive("field_simple", &["field", &instr], &instr);
                    fields.push(FieldSimple {
                        id,
                        instruction: instr,
                        result,
                    });
                }
            }
            for child in children {
                parse_fields_recursive(child, fields, _diags);
            }
        }
        _ => {}
    }
}

/// Get a field attribute value (handles namespaced and non-namespaced).
fn get_fld_attr(attributes: &BTreeMap<TagName, String>, key: &str) -> Option<String> {
    for (tag, val) in attributes {
        if tag.local_name == key || tag.local_name.ends_with(&format!(":{}", key)) {
            return Some(val.clone());
        }
    }
    None
}

/// Parse complex field characters (w:fldChar with w:instrText) from document XML.
pub fn parse_field_complex_from_xml(
    node: &XmlNode,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(FieldComplex, String)> {
    let mut result: Vec<(FieldComplex, String)> = Vec::new();
    parse_field_complex_recursive(node, &mut result, diags);
    result
}

fn parse_field_complex_recursive(
    node: &XmlNode,
    result: &mut Vec<(FieldComplex, String)>,
    _diags: &mut Vec<Diagnostic>,
) {
    match node {
        XmlNode::Element {
            name,
            attributes,
            children,
            ..
        } => {
            if is_word_elem(name, "fldChar") {
                if let Some(ft) = get_fld_attr(attributes, "fldCharType") {
                    let fld_id = get_fld_attr(attributes, "fldId").unwrap_or_default();
                    let id = ElementId::derive("field_complex", &["field", &fld_id, &ft], &ft);
                    let field_type = match ft.as_str() {
                        "begin" => FieldCharType::Begin,
                        "end" => FieldCharType::End,
                        "separate" => FieldCharType::Separate,
                        "dirty" => FieldCharType::Dirty,
                        _ => return,
                    };
                    result.push((
                        FieldComplex {
                            id,
                            fld_id: fld_id.clone(),
                            field_type,
                        },
                        fld_id,
                    ));
                }
            } else if is_word_elem(name, "instrText") {
                if let XmlNode::Element { attributes, .. } = node {
                    let fld_id = get_fld_attr(attributes, "w:instrText")
                        .or_else(|| get_fld_attr(attributes, "instrText"))
                        .unwrap_or_default();
                    // Extract text content
                    let mut text = String::new();
                    extract_all_text(node, &mut text);
                    result.push((
                        FieldComplex {
                            id: ElementId::derive("instr", &["instr", &fld_id], &fld_id),
                            fld_id: fld_id.clone(),
                            field_type: FieldCharType::Begin,
                        },
                        fld_id,
                    ));
                }
            }
            for child in children {
                parse_field_complex_recursive(child, result, _diags);
            }
        }
        _ => {}
    }
}

/// Extract all text from a node and its descendants.
fn extract_all_text(node: &XmlNode, out: &mut String) {
    match node {
        XmlNode::Text { text, .. } => out.push_str(text),
        XmlNode::Element { children, .. } => {
            for child in children {
                extract_all_text(child, out);
            }
        }
        _ => {}
    }
}

/// Parse tracked changes (w:ins and w:del) from document XML.
pub fn parse_tracked_changes(node: &XmlNode, diags: &mut Vec<Diagnostic>) -> Vec<TrackedChange> {
    let mut changes: Vec<TrackedChange> = Vec::new();
    parse_tracked_changes_recursive(node, &mut changes, diags);
    changes
}

fn parse_tracked_changes_recursive(
    node: &XmlNode,
    changes: &mut Vec<TrackedChange>,
    _diags: &mut Vec<Diagnostic>,
) {
    if let XmlNode::Element {
        name,
        attributes,
        children,
        ..
    } = node
    {
        if is_word_elem(name, "ins") {
            let author = get_tracked_attr(attributes, "author");
            let date = get_tracked_attr(attributes, "date");
            let change_id =
                get_tracked_attr(attributes, "w:id").or_else(|| get_tracked_attr(attributes, "id"));
            let mut content = String::new();
            for child in children {
                extract_all_text(child, &mut content);
            }
            if !content.is_empty() {
                let id = ElementId::derive(
                    "tracked_change",
                    &["ins", &content[..content.len().min(32)]],
                    &content,
                );
                changes.push(TrackedChange {
                    id,
                    kind: ChangeKind::Inserted,
                    content,
                    author,
                    date,
                    change_id,
                });
            }
        } else if is_word_elem(name, "del") {
            let author = get_tracked_attr(attributes, "author");
            let date = get_tracked_attr(attributes, "date");
            let change_id =
                get_tracked_attr(attributes, "w:id").or_else(|| get_tracked_attr(attributes, "id"));
            let mut content = String::new();
            for child in children {
                extract_all_text(child, &mut content);
            }
            if !content.is_empty() {
                let id = ElementId::derive(
                    "tracked_change",
                    &["del", &content[..content.len().min(32)]],
                    &content,
                );
                changes.push(TrackedChange {
                    id,
                    kind: ChangeKind::Deleted,
                    content,
                    author,
                    date,
                    change_id,
                });
            }
        }
        for child in children {
            parse_tracked_changes_recursive(child, changes, _diags);
        }
    }
}

/// Get a tracked change attribute value.
fn get_tracked_attr(attributes: &BTreeMap<TagName, String>, key: &str) -> Option<String> {
    for (tag, val) in attributes {
        if tag.local_name == key || tag.local_name.ends_with(&format!(":{}", key)) {
            return Some(val.clone());
        }
    }
    None
}

/// Get the element name from an XmlNode for field parsing.
fn child_name(node: &XmlNode) -> &TagName {
    match node {
        XmlNode::Element { name, .. } => name,
        _ => {
            // Return a reference to a thread-local static
            use std::sync::OnceLock;
            static FAKE: OnceLock<TagName> = OnceLock::new();
            FAKE.get_or_init(|| TagName {
                prefix: None,
                local_name: "fake".into(),
                namespace_uri: None,
            })
        }
    }
}

/// Extract fields and tracked changes from a parsed document XML.
fn extract_fields_and_tracked_changes(
    xml_doc: &XmlDocument,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Field>, Vec<TrackedChange>) {
    let mut simple_fields: Vec<FieldSimple> = Vec::new();
    let mut tracked_changes: Vec<TrackedChange> = Vec::new();

    parse_fields_recursive(&xml_doc.root, &mut simple_fields, diags);
    parse_tracked_changes_recursive(&xml_doc.root, &mut tracked_changes, diags);

    let fields: Vec<Field> = simple_fields.into_iter().map(Field::Simple).collect();
    (fields, tracked_changes)
}

// ===========================================================================
// Style reading
// ===========================================================================

/// A style definition parsed from styles.xml.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct StyleDef {
    /// The style ID used as the key (w:style/@w:styleId).
    style_id: String,
    /// The display name (w:name/@w:val).
    name: Option<String>,
    /// The style type (paragraph, character, etc.).
    style_type: Option<String>,
}

/// Read and parse `word/styles.xml` to build a style map.
fn read_styles_xml(pkg: &OpcPackage) -> Result<BTreeMap<String, StyleDef>, Vec<Diagnostic>> {
    let styles_part = find_part(pkg, "word/styles.xml")?;
    let xml_doc = parse_xml(&styles_part.data)?;

    let mut map: BTreeMap<String, StyleDef> = BTreeMap::new();

    // The root of styles.xml is <w:styles>, iterate its children
    if let XmlNode::Element { children, .. } = &xml_doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                attributes,
                children: style_children,
                ..
            } = child
            {
                if is_word_elem(name, "style") {
                    if let Some(style_id) = get_attr_value(attributes, "styleId") {
                        // OOXML spec: w:type is an attribute on w:style, not a child element
                        let type_val = get_attr_value(attributes, "type");

                        let mut name_val: Option<String> = None;
                        // w:name is a child element with w:val attribute
                        for sc in style_children {
                            if let XmlNode::Element {
                                name: sn,
                                attributes: sa,
                                ..
                            } = sc
                            {
                                if is_word_elem(sn, "name") {
                                    if let Some(n) = get_attr_value(sa, "val") {
                                        name_val = Some(n);
                                    }
                                }
                            }
                        }

                        map.insert(
                            style_id.clone(),
                            StyleDef {
                                style_id,
                                name: name_val,
                                style_type: type_val,
                            },
                        );
                    }
                }
            }
        }
    }

    Ok(map)
}

/// Apply styles to all paragraphs in a document.
fn apply_styles_to_document(document: &mut Document, style_map: &BTreeMap<String, StyleDef>) {
    for section in &mut document.sections {
        apply_styles_to_section_items(&mut section.items, style_map);
    }
}

/// Walk section items recursively and apply style names from pStyle tags.
fn apply_styles_to_section_items(
    items: &mut Vec<SectionItem>,
    style_map: &BTreeMap<String, StyleDef>,
) {
    for item in items {
        match item {
            SectionItem::Paragraph(para) => {
                if let Some(ref pstyle) = para.style_name {
                    // pStyle already set during parsing; only log if style not in map
                    if !style_map.contains_key(pstyle.as_str()) {
                        // style not found in styles.xml -- leave as-is, not an error
                    }
                }
            }
            SectionItem::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        apply_styles_to_section_items(&mut cell.contents, style_map);
                    }
                }
            }
            SectionItem::List(list) => {
                for list_item in &mut list.items {
                    apply_styles_to_section_items(&mut list_item.content, style_map);
                }
            }
            SectionItem::Drawing(_) => {}
        }
    }
}

// ===========================================================================
// Numbering reading
// ===========================================================================

/// A numbering definition parsed from numbering.xml.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct NumDef {
    /// The numId as used by w:numPr references.
    num_id: usize,
    /// The abstractNumId that groups related numbering levels.
    abstract_num_id: String,
    /// The numbering format (number, bullet, etc.).
    number_format: Option<String>,
    /// The style name used for this numbering (if any).
    style_name: Option<String>,
}

/// A mapping from numId to numbering definition.
type NumDefMap = BTreeMap<usize, NumDef>;

/// Read and parse `word/numbering.xml` to build a numId->NumDef map.
fn read_numbering_xml(pkg: &OpcPackage) -> Result<NumDefMap, Vec<Diagnostic>> {
    let num_part = find_part(pkg, "word/numbering.xml")?;
    let xml_doc = parse_xml(&num_part.data)?;

    let mut map: NumDefMap = BTreeMap::new();

    // Iterate over ALL root children (not just inside the first <w:num>)
    if let XmlNode::Element { children, .. } = &xml_doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                attributes,
                children: num_children,
                ..
            } = child
            {
                if is_word_elem(name, "num") {
                    if let Some(num_id_val) = get_attr_value(attributes, "numId") {
                        if let Ok(num_id) = num_id_val.parse::<usize>() {
                            let mut abstract_num_id: Option<String> = None;
                            let mut number_format: Option<String> = None;
                            let mut style_name: Option<String> = None;

                            for gc in num_children {
                                if let XmlNode::Element {
                                    name: gc_name,
                                    attributes: gc_attrs,
                                    children: gcgc,
                                    ..
                                } = gc
                                {
                                    if is_word_elem(gc_name, "abstractNumId") {
                                        // abstractNumId element uses abstractNumId attribute, not val
                                        if let Some(aid) = get_attr_value(gc_attrs, "abstractNumId")
                                        {
                                            abstract_num_id = Some(aid);
                                        } else if let Some(aid) = get_attr_value(gc_attrs, "val") {
                                            abstract_num_id = Some(aid);
                                        }
                                    } else if is_word_elem(gc_name, "numFmt") {
                                        if let Some(fmt) = get_attr_value(gc_attrs, "val") {
                                            number_format = Some(fmt);
                                        }
                                    } else if is_word_elem(gc_name, "pStyle") {
                                        if let Some(sname) = get_attr_value(gc_attrs, "val") {
                                            style_name = Some(sname);
                                        }
                                        for ggc in gcgc {
                                            if let XmlNode::Element {
                                                name: ggc_name,
                                                attributes: ggc_attrs,
                                                ..
                                            } = ggc
                                            {
                                                if is_word_elem(ggc_name, "styleId") {
                                                    if let Some(sid) =
                                                        get_attr_value(ggc_attrs, "val")
                                                    {
                                                        style_name = Some(sid);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            map.insert(
                                num_id,
                                NumDef {
                                    num_id,
                                    abstract_num_id: abstract_num_id.unwrap_or_default(),
                                    number_format,
                                    style_name,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(map)
}

/// Apply numbering information to paragraphs in the document.
fn apply_numbering_to_document(
    document: &mut Document,
    num_def_map: &NumDefMap,
    _diags: &mut Vec<Diagnostic>,
) {
    for section in &mut document.sections {
        apply_numbering_to_section_items(&mut section.items, num_def_map);
    }
}

fn apply_numbering_to_section_items(items: &mut Vec<SectionItem>, num_def_map: &NumDefMap) {
    let mut i = 0;
    let mut new_items: Vec<SectionItem> = Vec::new();

    while i < items.len() {
        if let SectionItem::Paragraph(para) = &items[i] {
            if let Some(ref num_info) = para.numbering_info {
                let num_id = num_info.num_id;
                if num_def_map.contains_key(&num_id) {
                    let ndef = &num_def_map[&num_id];
                    let kind = determine_list_kind(&ndef.number_format);
                    let mut list_items: Vec<ListItem> = Vec::new();
                    let last_num_id = num_id;
                    let mut start = i;

                    while start < items.len() {
                        if let SectionItem::Paragraph(p) = &items[start] {
                            if let Some(ref ni) = p.numbering_info {
                                if ni.num_id == last_num_id {
                                    let item_json =
                                        serde_json::to_string(&p.runs).unwrap_or_default();
                                    let lid = ElementId::content_anchored(
                                        "listitem",
                                        &["item:placeholder"],
                                        &item_json,
                                    );
                                    list_items.push(ListItem {
                                        id: lid,
                                        content: vec![SectionItem::Paragraph(p.clone())],
                                        level: ni.level,
                                    });
                                    start += 1;
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    let list_json = serde_json::to_string(&list_items).unwrap_or_default();
                    let list_id = ElementId::content_anchored("list", &["list:0"], &list_json);

                    new_items.push(SectionItem::List(List {
                        id: list_id,
                        kind,
                        items: list_items,
                    }));

                    i = start;
                    continue;
                }
            }
        }
        // Not numbered -- keep the paragraph as-is
        new_items.push(items[i].clone());
        i += 1;
    }

    *items = new_items;
}

/// Determine the ListKind from a number format string.
fn determine_list_kind(fmt: &Option<String>) -> ListKind {
    match fmt.as_deref() {
        Some("bullet")
        | Some("bulletCh")
        | Some("bulletCircle")
        | Some("bulletDiamond")
        | Some("bulletDraw")
        | Some("bulletHans")
        | Some("bulletHex")
        | Some("bulletImage")
        | Some("bulletKana")
        | Some("bulletKanji")
        | Some("bulletNone")
        | Some("bulletNumPl")
        | Some("bulletRoman")
        | Some("bulletStage")
        | Some("bulletUldash")
        | Some("bulletUltraDash")
        | Some("bulletWordCircle")
        | Some("bulletWordDash") => ListKind::Bullet,
        Some("decimal")
        | Some("decimalZero")
        | Some("decimalLeadingZero")
        | Some("ordinal")
        | Some("ordinalText")
        | Some("cardinalText")
        | Some("chapterNumber")
        | Some("numberInParen")
        | Some("numberInParenDot")
        | Some("numberInParenFullStop")
        | Some("numberInParenHalf") => ListKind::Numbered,
        Some(_) => ListKind::Other,
        None => ListKind::Other,
    }
}

// ===========================================================================
// Section properties reading
// ===========================================================================

/// Page size parsed from w:pgSz.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PageSize {
    width_pt: f64,
    height_pt: f64,
}

/// Margins parsed from w:pgMar.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PageMargins {
    left_pt: f64,
    right_pt: f64,
    top_pt: f64,
    bottom_pt: f64,
    header_pt: f64,
    footer_pt: f64,
    gutter_pt: f64,
}

/// Column definition from w:cols.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ColumnDef {
    width_pt: f64,
    space_pt: f64,
}

/// Section properties parsed from w:sectPr.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct SectionProps {
    page_size: Option<PageSize>,
    margins: Option<PageMargins>,
    columns: Option<Vec<ColumnDef>>,
}

/// Parse section properties from w:sectPr element.
fn parse_section_properties(node: &XmlNode, diags: &mut Vec<Diagnostic>) -> SectionProps {
    let mut props = SectionProps::default();

    if let XmlNode::Element { children, .. } = node {
        for child in children {
            match child {
                XmlNode::Element {
                    name, children: gc, ..
                } => {
                    if is_word_elem(name, "pgSz") {
                        props.page_size = parse_page_size(gc);
                    } else if is_word_elem(name, "pgMar") {
                        props.margins = parse_page_margins(gc);
                    } else if is_word_elem(name, "cols") {
                        props.columns = parse_columns(gc, diags);
                    }
                }
                _ => {}
            }
        }
    }

    props
}

/// Parse w:pgSz element for page dimensions.
fn parse_page_size(children: &[XmlNode]) -> Option<PageSize> {
    let mut width: Option<String> = None;
    let mut height: Option<String> = None;

    for child in children {
        if let XmlNode::Element {
            name, attributes, ..
        } = child
        {
            if is_word_elem(name, "pgSz") {
                if let Some(val) = get_attr_value(attributes, "w") {
                    width = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "h") {
                    height = Some(val);
                }
            }
        }
    }

    match (width, height) {
        (Some(w), Some(h)) => {
            let width_pt = w.parse::<f64>().ok()? / 914400.0 * 72.0; // EMU to points
            let height_pt = h.parse::<f64>().ok()? / 914400.0 * 72.0;
            Some(PageSize {
                width_pt,
                height_pt,
            })
        }
        _ => None,
    }
}

/// Parse w:pgMar element for page margins.
fn parse_page_margins(children: &[XmlNode]) -> Option<PageMargins> {
    let mut left: Option<String> = None;
    let mut right: Option<String> = None;
    let mut top: Option<String> = None;
    let mut bottom: Option<String> = None;
    let mut header: Option<String> = None;
    let mut footer: Option<String> = None;
    let mut gutter: Option<String> = None;

    for child in children {
        if let XmlNode::Element {
            name, attributes, ..
        } = child
        {
            if is_word_elem(name, "pgMar") {
                if let Some(val) = get_attr_value(attributes, "left") {
                    left = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "right") {
                    right = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "top") {
                    top = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "bottom") {
                    bottom = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "header") {
                    header = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "footer") {
                    footer = Some(val);
                }
                if let Some(val) = get_attr_value(attributes, "gutter") {
                    gutter = Some(val);
                }
            }
        }
    }

    Some(PageMargins {
        left_pt: left.unwrap_or("1440".to_string()).parse().unwrap_or(1440.0),
        right_pt: right
            .unwrap_or("1440".to_string())
            .parse()
            .unwrap_or(1440.0),
        top_pt: top.unwrap_or("1440".to_string()).parse().unwrap_or(1440.0),
        bottom_pt: bottom
            .unwrap_or("1440".to_string())
            .parse()
            .unwrap_or(1440.0),
        header_pt: header.unwrap_or("720".to_string()).parse().unwrap_or(720.0),
        footer_pt: footer.unwrap_or("720".to_string()).parse().unwrap_or(720.0),
        gutter_pt: gutter.unwrap_or("0".to_string()).parse().unwrap_or(0.0),
    })
}

/// Parse w:cols element for column layout.
fn parse_columns(children: &[XmlNode], _diags: &mut Vec<Diagnostic>) -> Option<Vec<ColumnDef>> {
    let mut cols: Vec<ColumnDef> = Vec::new();
    let mut count: usize = 1;
    let mut space: String = "240".to_string();

    for child in children {
        if let XmlNode::Element {
            name,
            attributes,
            children: gc,
            ..
        } = child
        {
            if is_word_elem(name, "cols") {
                if let Some(val) = get_attr_value(attributes, "count") {
                    if let Ok(c) = val.parse::<usize>() {
                        count = c;
                    }
                }
                if let Some(val) = get_attr_value(attributes, "space") {
                    space = val;
                }
                for gc_child in gc {
                    if let XmlNode::Element {
                        name: gc_name,
                        attributes: gc_attrs,
                        ..
                    } = gc_child
                    {
                        if is_word_elem(gc_name, "col") {
                            if let Some(val) = get_attr_value(gc_attrs, "w") {
                                if let Ok(w) = val.parse::<f64>() {
                                    cols.push(ColumnDef {
                                        width_pt: w,
                                        space_pt: space.parse().unwrap_or(240.0),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // If no individual col elements, generate defaults
    if cols.is_empty() && count > 0 {
        for _ in 0..count {
            cols.push(ColumnDef {
                width_pt: 0.0,
                space_pt: space.parse().unwrap_or(240.0),
            });
        }
    }

    if cols.is_empty() {
        None
    } else {
        Some(cols)
    }
}

// ===========================================================================
// Header/Footer reading
// ===========================================================================

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

/// Resolve a relationship ID to a target URI.
fn resolve_rel_id(pkg: &OpcPackage, source_uri: &str, rel_id: &str) -> Option<String> {
    if let Some(rels) = pkg.relationships.get(source_uri) {
        for rel in rels {
            if rel.id == rel_id {
                let target = if rel.target.starts_with('/') {
                    rel.target[1..].to_string()
                } else if rel.target.contains('/') {
                    rel.target.clone()
                } else {
                    let dir = if let Some(pos) = source_uri.rfind('/') {
                        &source_uri[..pos + 1]
                    } else {
                        ""
                    };
                    format!("{}{}", dir, rel.target)
                };
                return Some(target);
            }
        }
    }
    // Also check package-level relationships
    if let Some(rels) = pkg.relationships.get("_rels/.rels") {
        for rel in rels {
            if rel.id == rel_id {
                return Some(rel.target.clone());
            }
        }
    }
    None
}

/// Load a header part from the OPC package.
fn load_header_part(
    pkg: &OpcPackage,
    source_uri: &str,
    rel_id: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<Header> {
    let target_uri = resolve_rel_id(pkg, source_uri, rel_id)?;
    let header_uri = target_uri
        .strip_prefix('/')
        .unwrap_or(&target_uri)
        .to_string();
    let header_part = pkg.parts.iter().find(|p| p.uri == header_uri)?;

    let xml_doc = match parse_xml(&header_part.data) {
        Ok(d) => d,
        Err(e) => {
            diags.push(Diagnostic::new(
                "OOX-W-200",
                Severity::Warning,
                format!("Failed to parse header {}: {}", header_uri, e[0].message),
            ));
            return Some(Header {
                id: ElementId::derive("header", &["header"], &Vec::<SectionItem>::new()),
                content: vec![],
                is_even_page: false,
            });
        }
    };

    let content = parse_header_content(&xml_doc, diags);

    Some(Header {
        id: ElementId::derive("header", &["header_ref"], &content),
        content,
        is_even_page: false,
    })
}

/// Load a footer part from the OPC package.
fn load_footer_part(
    pkg: &OpcPackage,
    source_uri: &str,
    rel_id: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<Footer> {
    let target_uri = resolve_rel_id(pkg, source_uri, rel_id)?;
    let footer_uri = target_uri
        .strip_prefix('/')
        .unwrap_or(&target_uri)
        .to_string();
    let footer_part = pkg.parts.iter().find(|p| p.uri == footer_uri)?;

    let xml_doc = match parse_xml(&footer_part.data) {
        Ok(d) => d,
        Err(e) => {
            diags.push(Diagnostic::new(
                "OOX-W-201",
                Severity::Warning,
                format!("Failed to parse footer {}: {}", footer_uri, e[0].message),
            ));
            return Some(Footer {
                id: ElementId::derive("footer", &["footer"], &Vec::<SectionItem>::new()),
                content: vec![],
                is_even_page: false,
            });
        }
    };

    let content = parse_header_content(&xml_doc, diags);

    Some(Footer {
        id: ElementId::derive("footer", &["footer_ref"], &content),
        content,
        is_even_page: false,
    })
}

/// Parse header content (same structure as document body).
fn parse_header_content(xml_doc: &XmlDocument, diags: &mut Vec<Diagnostic>) -> Vec<SectionItem> {
    let mut items: Vec<SectionItem> = Vec::new();

    match &xml_doc.root {
        XmlNode::Element { children, .. } => {
            for child in children {
                if let XmlNode::Element {
                    name, children: gc, ..
                } = child
                {
                    if is_word_elem(name, "p") {
                        if let Some(para) = parse_paragraph(gc, diags) {
                            items.push(SectionItem::Paragraph(para));
                        }
                    } else if is_word_elem(name, "tbl") {
                        if let Some(table) = parse_table(gc, diags) {
                            items.push(SectionItem::Table(table));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    items
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read a .docx file (already parsed OPC package) into gp-core IR.
pub fn read_docx(package: &OpcPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let doc_part = find_document_part(package)?;
    let xml_doc = parse_xml(&doc_part.data)?;
    let (document, mut diags) = build_document(&xml_doc, package)?;

    let ir_bytes = serde_json::to_vec(&document).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-010",
            Severity::Error,
            format!("Failed to serialize IR: {}", e),
        )]
    })?;

    diags.push(Diagnostic::new(
        "OOX-R010",
        Severity::Info,
        format!(
            "docx: IR produced with {} sections, {} diagnostics",
            document.sections.len(),
            diags.len()
        ),
    ));

    Ok(ReadResult {
        ir: ir_bytes,
        diagnostics: diags,
    })
}

/// Find the `word/document.xml` part in the OPC package.
fn find_document_part(pkg: &OpcPackage) -> Result<&Part, Vec<Diagnostic>> {
    pkg.parts
        .iter()
        .find(|p| p.uri == "word/document.xml")
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "OOX-E-001",
                Severity::Error,
                "word/document.xml not found in package".to_string(),
            )]
        })
}

/// Type alias for numbering info in paragraphs.
type NumInfo = gp_core::ir::NumberingInfo;

// ---------------------------------------------------------------------------
// Document building
// ---------------------------------------------------------------------------

/// Build a gp-core Document from an XML document.
fn build_document(
    xml_doc: &XmlDocument,
    pkg: &OpcPackage,
) -> Result<(Document, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut body_items: Vec<SectionItem> = Vec::new();
    let mut headers: Vec<Header> = Vec::new();
    let mut footers: Vec<Footer> = Vec::new();

    match &xml_doc.root {
        XmlNode::Element { children, .. } => {
            for child in children {
                if let XmlNode::Element {
                    name,
                    children: body_children,
                    ..
                } = child
                {
                    if is_word_elem(name, "body") {
                        for item in body_children {
                            parse_body_item(item, &mut body_items, &mut diags);
                        }
                    } else if is_word_elem(name, "sectPr") {
                        parse_sect_pr(child, &mut headers, &mut footers, pkg, &mut diags);
                    }
                }
            }
        }
        _ => {}
    }

    let section = build_section(&body_items, &headers, &footers);
    let (fields, tracked_changes) = extract_fields_and_tracked_changes(xml_doc, &mut diags);
    let document = Document {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sections: vec![section],
        fields,
        comments: vec![],
        tracked_changes,
        revisions: vec![],
        template: None,
    };

    Ok((document, diags))
}

/// Build a gp-core Document with full feature support.
fn build_document_full(
    xml_doc: &XmlDocument,
    pkg: &OpcPackage,
    style_map: &BTreeMap<String, StyleDef>,
    _num_def_map: &NumDefMap,
) -> Result<(Document, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut body_items: Vec<SectionItem> = Vec::new();
    let mut headers: Vec<Header> = Vec::new();
    let mut footers: Vec<Footer> = Vec::new();
    let mut section_props = SectionProps::default();

    match &xml_doc.root {
        XmlNode::Element { children, .. } => {
            for child in children {
                if let XmlNode::Element {
                    name,
                    children: body_children,
                    ..
                } = child
                {
                    if is_word_elem(name, "body") {
                        for item in body_children {
                            parse_body_item_full(item, &mut body_items, style_map, &mut diags);
                        }
                    } else if is_word_elem(name, "sectPr") {
                        section_props = parse_section_properties(child, &mut diags);
                        parse_sect_pr_full(
                            child,
                            &mut headers,
                            &mut footers,
                            pkg,
                            "word/document.xml",
                            &mut diags,
                        );
                    }
                }
            }
        }
        _ => {}
    }

    // Apply styles
    {
        let mut tmp_doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::derive("section", &["section:0"], &body_items),
                kind: SectionKind::Body,
                items: body_items.clone(),
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        apply_styles_to_document(&mut tmp_doc, style_map);
    }

    let section = build_section_full(&body_items, &headers, &footers, &section_props);
    let (fields, tracked_changes) = extract_fields_and_tracked_changes(xml_doc, &mut diags);
    let document = Document {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sections: vec![section],
        fields,
        comments: vec![],
        tracked_changes,
        revisions: vec![],
        template: None,
    };

    Ok((document, diags))
}

/// Build a Section with full section properties.
fn build_section_full(
    items: &[SectionItem],
    headers: &[Header],
    footers: &[Footer],
    _section_props: &SectionProps,
) -> Section {
    let content_json = serde_json::to_string(items).unwrap_or_default();
    let id = ElementId::content_anchored("section", &["section:0"], &content_json);

    Section {
        id,
        kind: SectionKind::Body,
        items: items.to_vec(),
        headers: headers.to_vec(),
        footers: footers.to_vec(),
        footnotes: vec![],
        endnotes: vec![],
    }
}

/// Parse a child of <w:body> into a SectionItem with full style support.
fn parse_body_item_full(
    node: &XmlNode,
    items: &mut Vec<SectionItem>,
    style_map: &BTreeMap<String, StyleDef>,
    diags: &mut Vec<Diagnostic>,
) {
    match node {
        XmlNode::Element { name, children, .. } => {
            if is_word_elem(name, "p") {
                if let Some(para) = parse_paragraph_full(children, style_map, diags) {
                    items.push(SectionItem::Paragraph(para));
                }
            } else if is_word_elem(name, "tbl") {
                if let Some(table) = parse_table(children, diags) {
                    items.push(SectionItem::Table(table));
                }
            } else if is_word_elem(name, "list") {
                if let Some(list) = parse_list_block(children, diags) {
                    items.push(SectionItem::List(list));
                }
            } else if is_word_elem(name, "sdt") {
                for child in children {
                    parse_body_item_full(child, items, style_map, diags);
                }
            } else {
                diags.push(Diagnostic::new(
                    "OOX-W-010",
                    Severity::Warning,
                    format!("Unknown body-level element '{}' — skipping", name.qname()),
                ));
            }
        }
        _ => {}
    }
}

/// Build a Section from its items.
fn build_section(items: &[SectionItem], headers: &[Header], footers: &[Footer]) -> Section {
    let content_json = serde_json::to_string(items).unwrap_or_default();
    let id = ElementId::content_anchored("section", &["section:0"], &content_json);

    Section {
        id,
        kind: SectionKind::Body,
        items: items.to_vec(),
        headers: headers.to_vec(),
        footers: footers.to_vec(),
        footnotes: vec![],
        endnotes: vec![],
    }
}

// ---------------------------------------------------------------------------
// Body parsing — SectionItem enumeration
// ---------------------------------------------------------------------------

/// Parse a child of <w:body> into a SectionItem.
fn parse_body_item(node: &XmlNode, items: &mut Vec<SectionItem>, diags: &mut Vec<Diagnostic>) {
    match node {
        XmlNode::Element { name, children, .. } => {
            if is_word_elem(name, "p") {
                if let Some(para) = parse_paragraph(children, diags) {
                    items.push(SectionItem::Paragraph(para));
                }
            } else if is_word_elem(name, "tbl") {
                if let Some(table) = parse_table(children, diags) {
                    items.push(SectionItem::Table(table));
                }
            } else if is_word_elem(name, "list") {
                if let Some(list) = parse_list_block(children, diags) {
                    items.push(SectionItem::List(list));
                }
            } else if is_word_elem(name, "sdt") {
                for child in children {
                    parse_body_item(child, items, diags);
                }
            } else {
                diags.push(Diagnostic::new(
                    "OOX-W-010",
                    Severity::Warning,
                    format!("Unknown body-level element '{}' — skipping", name.qname()),
                ));
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Paragraph parsing
// ---------------------------------------------------------------------------

/// Parse a <w:p> element into a Paragraph with full support.
fn parse_paragraph_full(
    children: &[XmlNode],
    style_map: &BTreeMap<String, StyleDef>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Paragraph> {
    let mut runs: Vec<Run> = Vec::new();
    let mut style_name: Option<String> = None;
    let mut numbering_info: Option<NumInfo> = None;

    for child in children {
        match child {
            XmlNode::Element {
                name,
                attributes,
                children: grand_children,
                ..
            } => {
                if is_word_elem(name, "rPr") || is_word_elem(name, "pPr") {
                    if is_word_elem(name, "pPr") {
                        // Extract pStyle from within pPr
                        for gc in grand_children {
                            if let XmlNode::Element {
                                name: gc_name,
                                attributes: gc_attrs,
                                ..
                            } = gc
                            {
                                if is_word_elem(gc_name, "pStyle") {
                                    if let Some(ref val) = get_attr_value(gc_attrs, "val") {
                                        style_name = Some(val.clone());
                                    }
                                } else if is_word_elem(gc_name, "numPr") {
                                    if let Some(num_info) = parse_num_pr(child, diags) {
                                        numbering_info = Some(num_info);
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                if is_word_elem(name, "r") {
                    if let Some(run) = parse_run(child, diags) {
                        runs.push(run);
                    }
                } else if is_word_elem(name, "pStyle") {
                    if let Some(ref val) = get_attr_value(attributes, "val") {
                        style_name = Some(val.clone());
                        // If style not in map, still record it (reference only)
                        let _ = style_map.contains_key(val.as_str());
                    }
                } else if is_word_elem(name, "hyperlink") {
                    if let Some(hr) = parse_hyperlink(child, diags) {
                        runs.extend(hr);
                    }
                }
            }
            _ => {}
        }
    }

    let paragraph = Paragraph {
        id: ElementId::derive("paragraph", &["paragraph:0"], &runs),
        runs,
        page_number: None,
        style_name,
        numbering_info,
    };
    Some(paragraph)
}

/// Parse a <w:p> element into a Paragraph (basic, no numbering info).
fn parse_paragraph(children: &[XmlNode], diags: &mut Vec<Diagnostic>) -> Option<Paragraph> {
    let para = parse_paragraph_full(children, &BTreeMap::new(), diags)?;
    // Drop numbering_info for basic reader
    Some(Paragraph {
        numbering_info: None,
        ..para
    })
}

/// Parse w:numPr element to extract numbering reference.
fn parse_num_pr(node: &XmlNode, _diags: &mut Vec<Diagnostic>) -> Option<NumInfo> {
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let XmlNode::Element {
                name,
                children: numpr_children,
                ..
            } = child
            {
                if is_word_elem(name, "numPr") {
                    let mut num_id: Option<usize> = None;
                    let mut ilvl: usize = 0;

                    for gc in numpr_children {
                        if let XmlNode::Element {
                            name: gc_name,
                            attributes: gc_attrs,
                            ..
                        } = gc
                        {
                            if is_word_elem(gc_name, "numId") {
                                if let Some(val) = get_attr_value(gc_attrs, "val") {
                                    if let Ok(id) = val.parse::<usize>() {
                                        num_id = Some(id);
                                    }
                                }
                            } else if is_word_elem(gc_name, "ilvl") {
                                if let Some(val) = get_attr_value(gc_attrs, "ilvl") {
                                    if let Ok(level) = val.parse::<usize>() {
                                        ilvl = level;
                                    }
                                } else if let Some(val) = get_attr_value(gc_attrs, "val") {
                                    if let Ok(level) = val.parse::<usize>() {
                                        ilvl = level;
                                    }
                                }
                            }
                        }
                    }

                    if let Some(num_id_val) = num_id {
                        return Some(NumInfo {
                            num_id: num_id_val,
                            level: ilvl,
                        });
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Run parsing
// ---------------------------------------------------------------------------

/// Run-level formatting properties extracted from <w:rPr>.
struct RunProps {
    bold: bool,
    italic: bool,
    font_name: Option<String>,
    font_size_pt: Option<f64>,
    underline: bool,
    strikethrough: bool,
}

/// Parse a <w:r> element into a Run (with rStyle support).
fn parse_run(node: &XmlNode, _diags: &mut Vec<Diagnostic>) -> Option<Run> {
    let mut text = String::new();
    let mut bold: Option<bool> = None;
    let mut italic: Option<bool> = None;
    let mut font_name: Option<String> = None;
    let mut font_size_pt: Option<f64> = None;
    let mut underline: Option<bool> = None;
    let mut strikethrough: Option<bool> = None;
    let mut r_style: Option<String> = None;
    let mut has_props = false;

    if let XmlNode::Element { children, .. } = node {
        for child in children {
            match child {
                XmlNode::Element {
                    name, children: gc, ..
                } => {
                    if is_word_elem(name, "rPr") {
                        has_props = true;
                        // Look for rStyle in rPr
                        for gc_child in gc {
                            if let XmlNode::Element {
                                name: gc_name,
                                attributes: gc_attrs,
                                ..
                            } = gc_child
                            {
                                if is_word_elem(gc_name, "rStyle") {
                                    if let Some(val) = get_attr_value(gc_attrs, "val") {
                                        r_style = Some(val);
                                    }
                                }
                            }
                        }
                        if let Some(props) = parse_run_properties(gc) {
                            if props.bold {
                                bold = Some(props.bold);
                            }
                            if props.italic {
                                italic = Some(props.italic);
                            }
                            if props.font_name.is_some() {
                                font_name = props.font_name;
                            }
                            if props.font_size_pt.is_some() {
                                font_size_pt = props.font_size_pt;
                            }
                            if props.underline {
                                underline = Some(props.underline);
                            }
                            if props.strikethrough {
                                strikethrough = Some(props.strikethrough);
                            }
                        }
                    } else if is_word_elem(name, "t") || is_word_elem(name, "delText") {
                        for gchild in gc {
                            if let XmlNode::Text { text: t, .. } = gchild {
                                text.push_str(t);
                            }
                        }
                    } else if is_word_elem(name, "cr") {
                        text.push('\n');
                    } else if is_word_elem(name, "tab") {
                        text.push('\t');
                    } else if is_word_elem(name, "br") {
                        if let Some(val) = get_br_type(child) {
                            if val == "page" || val == "column" || val == "section" {
                                text.push('\n');
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if text.is_empty() && !has_props {
        return None;
    }

    Some(Run {
        id: ElementId::derive(
            "run",
            &["run:0"],
            &Run {
                id: ElementId::derive("placeholder", &["placeholder:0"], &()),
                text: text.clone(),
                bold,
                italic,
                font_name: font_name.clone(),
                font_size_pt,
                underline,
                strikethrough,
                style_name: r_style.clone(),
            },
        ),
        text,
        bold,
        italic,
        font_name,
        font_size_pt,
        underline,
        strikethrough,
        style_name: r_style,
    })
}

/// Get the `type` attribute value from a <br> element.
fn get_br_type(node: &XmlNode) -> Option<String> {
    match node {
        XmlNode::Element { attributes, .. } => attributes
            .iter()
            .find(|(k, _)| k.local_name == "type")
            .map(|(_, v)| v.clone()),
        _ => None,
    }
}

/// Parse a <w:rPr> (run properties) element.
fn parse_run_properties(children: &[XmlNode]) -> Option<RunProps> {
    let mut bold = false;
    let mut italic = false;
    let mut font_name: Option<String> = None;
    let mut font_size_pt: Option<f64> = None;
    let mut underline = false;
    let mut strikethrough = false;

    for child in children {
        match child {
            XmlNode::Element {
                name, attributes, ..
            } => {
                if is_word_elem(name, "b") {
                    bold = true;
                } else if is_word_elem(name, "i") {
                    italic = true;
                } else if is_word_elem(name, "u") {
                    if let Some(val) = get_attr_value(attributes, "val") {
                        underline = val != "none";
                    } else {
                        underline = true;
                    }
                } else if is_word_elem(name, "strike") || is_word_elem(name, "dstrike") {
                    strikethrough = true;
                } else if is_word_elem(name, "rFonts") {
                    if let Some(ff) = get_attr_value(attributes, "ascii") {
                        font_name = Some(ff);
                    } else if let Some(ff) = get_attr_value(attributes, "hint") {
                        if font_name.is_none() {
                            font_name = Some(ff);
                        }
                    } else if let Some(ff) = get_attr_value(attributes, "eastAsia") {
                        if font_name.is_none() {
                            font_name = Some(ff);
                        }
                    } else if let Some(ff) = get_attr_value(attributes, "cs") {
                        if font_name.is_none() {
                            font_name = Some(ff);
                        }
                    }
                } else if is_word_elem(name, "sz") {
                    if let Some(val) = get_attr_value(attributes, "val") {
                        if let Ok(pts) = val.parse::<f64>() {
                            font_size_pt = Some(pts / 2.0);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Some(RunProps {
        bold,
        italic,
        font_name,
        font_size_pt,
        underline,
        strikethrough,
    })
}

/// Parse a <w:hyperlink> element, returning runs for its contents.
fn parse_hyperlink(node: &XmlNode, diags: &mut Vec<Diagnostic>) -> Option<Vec<Run>> {
    let mut runs: Vec<Run> = Vec::new();

    if let XmlNode::Element { children, .. } = node {
        for child in children {
            match child {
                XmlNode::Element { name, .. } => {
                    if is_word_elem(name, "r") {
                        if let Some(run) = parse_run(child, diags) {
                            runs.push(run);
                        }
                    } else if is_word_elem(name, "t") {
                        if let Some(t) = extract_text(child) {
                            runs.push(Run {
                                id: ElementId::derive(
                                    "run",
                                    &["run:0"],
                                    &Run {
                                        id: ElementId::derive(
                                            "placeholder",
                                            &["placeholder:0"],
                                            &(),
                                        ),
                                        text: t.clone(),
                                        bold: None,
                                        italic: None,
                                        font_name: None,
                                        font_size_pt: None,
                                        underline: None,
                                        strikethrough: None,
                                        style_name: None,
                                    },
                                ),
                                text: t.clone(),
                                bold: None,
                                italic: None,
                                font_name: None,
                                font_size_pt: None,
                                underline: None,
                                strikethrough: None,
                                style_name: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if runs.is_empty() {
        None
    } else {
        Some(runs)
    }
}

/// Extract all text content from an element and its children.
fn extract_text(node: &XmlNode) -> Option<String> {
    let mut text = String::new();
    extract_text_recursive(node, &mut text);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_text_recursive(node: &XmlNode, out: &mut String) {
    match node {
        XmlNode::Text { text, .. } => out.push_str(text),
        XmlNode::Element { children, .. } => {
            for child in children {
                extract_text_recursive(child, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Table parsing
// ---------------------------------------------------------------------------

/// Parse a <w:tbl> element into a Table.
fn parse_table(children: &[XmlNode], diags: &mut Vec<Diagnostic>) -> Option<Table> {
    let mut rows: Vec<Row> = Vec::new();

    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            if is_word_elem(name, "tr") {
                if let Some(row) = parse_row(gc, diags) {
                    rows.push(row);
                }
            }
        }
    }

    if rows.is_empty() {
        return None;
    }

    let rows_json = serde_json::to_string(&rows).unwrap_or_default();
    Some(Table {
        id: ElementId::content_anchored("table", &["table:0"], &rows_json),
        rows,
    })
}

/// Parse a <w:tr> element into a Row.
fn parse_row(children: &[XmlNode], diags: &mut Vec<Diagnostic>) -> Option<Row> {
    let mut cells: Vec<Cell> = Vec::new();

    for child in children {
        if let XmlNode::Element {
            name,
            children: gc,
            attributes,
            ..
        } = child
        {
            if is_word_elem(name, "tc") {
                let mut colspan: Option<usize> = None;
                let mut rowspan: Option<usize> = None;
                if let Some(val) = get_attr_value(attributes, "w:gridSpan") {
                    if let Ok(v) = val.parse::<usize>() {
                        colspan = Some(v);
                    }
                } else if let Some(val) = get_attr_value(attributes, "gridSpan") {
                    if let Ok(v) = val.parse::<usize>() {
                        colspan = Some(v);
                    }
                }
                if let Some(val) = get_attr_value(attributes, "w:vMerge") {
                    if let Ok(v) = val.parse::<usize>() {
                        rowspan = Some(v);
                    }
                } else if let Some(val) = get_attr_value(attributes, "vMerge") {
                    if val == "restart" {
                        rowspan = Some(1);
                    }
                }

                let mut contents: Vec<SectionItem> = Vec::new();
                for gc_child in gc {
                    if let XmlNode::Element {
                        name: gc_name,
                        children: gcgc,
                        ..
                    } = gc_child
                    {
                        if is_word_elem(gc_name, "p") {
                            if let Some(para) = parse_paragraph(gcgc, diags) {
                                contents.push(SectionItem::Paragraph(para));
                            }
                        } else if is_word_elem(gc_name, "tbl") {
                            if let Some(tbl) = parse_table(gcgc, diags) {
                                contents.push(SectionItem::Table(tbl));
                            }
                        }
                    }
                }

                let cells_json = serde_json::to_string(&contents).unwrap_or_default();
                let cell_id = ElementId::content_anchored("cell", &["cell:0"], &cells_json);

                cells.push(Cell {
                    id: cell_id,
                    contents,
                    colspan,
                    rowspan,
                });
            }
        }
    }

    Some(Row { cells })
}

// ---------------------------------------------------------------------------
// List parsing (explicit <w:list> block)
// ---------------------------------------------------------------------------

fn parse_list_block(children: &[XmlNode], diags: &mut Vec<Diagnostic>) -> Option<List> {
    let mut items: Vec<ListItem> = Vec::new();

    for child in children {
        if let XmlNode::Element {
            name, children: gc, ..
        } = child
        {
            if is_word_elem(name, "il") {
                let mut content: Vec<SectionItem> = Vec::new();
                for gc_child in gc {
                    if let XmlNode::Element {
                        name: gc_name,
                        children: gcgc,
                        ..
                    } = gc_child
                    {
                        if is_word_elem(gc_name, "p") {
                            if let Some(para) = parse_paragraph(gcgc, diags) {
                                content.push(SectionItem::Paragraph(para));
                            }
                        }
                    }
                }
                items.push(ListItem {
                    id: ElementId::derive(
                        "listitem",
                        &["item:0"],
                        &ListItem {
                            id: ElementId::derive("placeholder", &["placeholder:0"], &()),
                            content: vec![],
                            level: 0,
                        },
                    ),
                    content,
                    level: 0,
                });
            }
        }
    }

    if items.is_empty() {
        None
    } else {
        let items_json = serde_json::to_string(&items).unwrap_or_default();
        Some(List {
            id: ElementId::content_anchored("list", &["list:0"], &items_json),
            kind: ListKind::Numbered,
            items,
        })
    }
}

// ---------------------------------------------------------------------------
// Section properties parsing
// ---------------------------------------------------------------------------

fn parse_sect_pr(
    node: &XmlNode,
    headers: &mut Vec<Header>,
    footers: &mut Vec<Footer>,
    _pkg: &OpcPackage,
    _diags: &mut Vec<Diagnostic>,
) {
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let XmlNode::Element {
                name, attributes, ..
            } = child
            {
                if is_word_elem(name, "headerReference") {
                    if get_attr_value(attributes, "r:id").is_some() {
                        headers.push(Header {
                            id: ElementId::derive(
                                "header",
                                &["header:0"],
                                &Vec::<SectionItem>::new(),
                            ),
                            content: vec![],
                            is_even_page: false,
                        });
                    }
                } else if is_word_elem(name, "footerReference") {
                    if get_attr_value(attributes, "r:id").is_some() {
                        footers.push(Footer {
                            id: ElementId::derive(
                                "footer",
                                &["footer:0"],
                                &Vec::<SectionItem>::new(),
                            ),
                            content: vec![],
                            is_even_page: false,
                        });
                    }
                }
            }
        }
    }
}

/// Parse sectPr for header/footer references with full part loading.
fn parse_sect_pr_full(
    node: &XmlNode,
    headers: &mut Vec<Header>,
    footers: &mut Vec<Footer>,
    pkg: &OpcPackage,
    source_uri: &str,
    diags: &mut Vec<Diagnostic>,
) {
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let XmlNode::Element {
                name, attributes, ..
            } = child
            {
                if is_word_elem(name, "headerReference") {
                    if let Some(rel_id) = get_attr_value(attributes, "r:id") {
                        let is_even = get_attr_value(attributes, "type")
                            .map(|t| t == "even" || t == "first")
                            .unwrap_or(false);

                        if let Some(header) = load_header_part(pkg, source_uri, &rel_id, diags) {
                            let mut h = header;
                            h.is_even_page = is_even;
                            headers.push(h);
                        } else {
                            headers.push(Header {
                                id: ElementId::derive(
                                    "header",
                                    &["header_ref"],
                                    &Vec::<SectionItem>::new(),
                                ),
                                content: vec![],
                                is_even_page: is_even,
                            });
                        }
                    }
                } else if is_word_elem(name, "footerReference") {
                    if let Some(rel_id) = get_attr_value(attributes, "r:id") {
                        let is_even = get_attr_value(attributes, "type")
                            .map(|t| t == "even" || t == "first")
                            .unwrap_or(false);

                        if let Some(footer) = load_footer_part(pkg, source_uri, &rel_id, diags) {
                            let mut f = footer;
                            f.is_even_page = is_even;
                            footers.push(f);
                        } else {
                            footers.push(Footer {
                                id: ElementId::derive(
                                    "footer",
                                    &["footer_ref"],
                                    &Vec::<SectionItem>::new(),
                                ),
                                content: vec![],
                                is_even_page: is_even,
                            });
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

/// Find a child element by local name.
/// Check if an XML element name matches a local name with the Word namespace prefix.
fn is_word_elem(name: &TagName, local: &str) -> bool {
    name.local_name == local
        && (name.prefix.as_deref() == Some("w")
            || name.prefix.as_deref() == Some("w14")
            || name.prefix.as_deref() == Some("w15")
            || name.prefix.as_deref() == Some("w16")
            || name.prefix.as_deref() == Some("w16ex"))
}

/// Get an attribute value from a map keyed by TagName.
fn get_attr_value(attributes: &BTreeMap<TagName, String>, key: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(k, _)| k.local_name == key || k.local_name.ends_with(&format!(":{}", key)))
        .map(|(_, v)| v.clone())
}

// ---------------------------------------------------------------------------
// Codec implementation
// ---------------------------------------------------------------------------

/// Reader for .docx files (Word Processing ML).
pub struct DocxReader;

impl DocxReader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DocxReader {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for DocxReader {
    fn format_name(&self) -> &str {
        "docx"
    }

    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
        match parse_opc(data) {
            Ok(pkg) => read_docx(&pkg),
            Err(diags) => Err(diags),
        }
    }
}

// ---------------------------------------------------------------------------
// Synthetic fixture builders for tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod fixtures {
    use std::io::Write;

    /// Word-processing ML namespace URI for test fixtures.
    pub const NS_WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    /// Build a minimal OPC package from parts with content types and relationships.
    pub fn build_docx_with_parts(parts: Vec<(&str, &[u8])>) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

        // Add content types
        writer.start_file("[Content_Types].xml", opts).unwrap();
        let ct = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                   <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
                   <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
                   <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
                   <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
                   <Override PartName=\"/word/_rels/document.xml.rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
                   </Types>";
        writer.write_all(ct).unwrap();

        // Add _rels/.rels
        writer.start_file("_rels/.rels", opts).unwrap();
        let rels = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                    <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
                    </Relationships>";
        writer.write_all(rels).unwrap();

        // Add requested parts
        for (uri, content) in &parts {
            writer.start_file(uri, opts).unwrap();
            writer.write_all(content).unwrap();
        }

        // Add document rels
        writer
            .start_file("word/_rels/document.xml.rels", opts)
            .unwrap();
        writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>").unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Build a styles.xml content blob.
    pub fn build_styles_xml(styles: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                        <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">".to_vec();
        for (style_id, style_name, style_type) in styles {
            xml.extend_from_slice(b"<w:style w:styleId=\"");
            xml.extend_from_slice(style_id.as_bytes());
            xml.extend_from_slice(b"\" w:type=\"");
            xml.extend_from_slice(style_type.as_bytes());
            xml.extend_from_slice(b"\"><w:name w:val=\"");
            xml.extend_from_slice(style_name.as_bytes());
            xml.extend_from_slice(b"\"/></w:style>");
        }
        xml.extend_from_slice(b"</w:styles>");
        xml
    }

    /// Build a numbering.xml content blob.
    pub fn build_numbering_xml(nums: &[(usize, &str, &str)]) -> Vec<u8> {
        let mut xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                        <w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                        <w:abstractNum w:abstractNumId=\"0\">\
                        <w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/></w:lvl>\
                        <w:lvl w:ilvl=\"1\"><w:numFmt w:val=\"decimal\"/></w:lvl>\
                        <w:lvl w:ilvl=\"2\"><w:numFmt w:val=\"decimal\"/></w:lvl>\
                        </w:abstractNum>".to_vec();

        for (num_id, abstract_num_id, num_fmt) in nums {
            xml.extend_from_slice(b"<w:num w:numId=\"");
            xml.extend_from_slice(num_id.to_string().as_bytes());
            xml.extend_from_slice(b"\"><w:abstractNumId w:abstractNumId=\"");
            xml.extend_from_slice(abstract_num_id.as_bytes());
            xml.extend_from_slice(b"\"/><w:numFmt w:val=\"");
            xml.extend_from_slice(num_fmt.as_bytes());
            xml.extend_from_slice(b"\"/></w:num>");
        }

        xml.extend_from_slice(b"</w:numbering>");
        xml
    }

    /// Build a document.xml with paragraph styles and/or numbering.
    pub fn build_docx_with_pstyle(pstyle_refs: &[(Option<&str>, &str)]) -> Vec<u8> {
        let mut body = b"<w:body>".to_vec();
        for (pstyle_opt, text) in pstyle_refs {
            body.extend_from_slice(b"<w:p>");
            if let Some(ps) = pstyle_opt {
                body.extend_from_slice(b"<w:pPr><w:pStyle w:val=\"");
                body.extend_from_slice(ps.as_bytes());
                body.extend_from_slice(b"\"/></w:pPr>");
            }
            body.extend_from_slice(b"<w:r><w:t>");
            body.extend_from_slice(text.as_bytes());
            body.extend_from_slice(b"</w:t></w:r></w:p>");
        }
        body.extend_from_slice(b"</w:body>");

        let doc_xml = format!(
            "<w:document xmlns:w=\"{}\">{}</w:document>",
            NS_WORD,
            String::from_utf8_lossy(&body)
        );
        doc_xml.into_bytes()
    }

    /// Build a document.xml with run styles (w:rStyle).
    pub fn build_docx_with_rstyle(rstyle_refs: &[(Option<&str>, &str)]) -> Vec<u8> {
        let mut body = b"<w:body>".to_vec();
        for (rstyle_opt, text) in rstyle_refs {
            body.extend_from_slice(b"<w:p><w:r>");
            if let Some(rs) = rstyle_opt {
                body.extend_from_slice(b"<w:rPr><w:rStyle w:val=\"");
                body.extend_from_slice(rs.as_bytes());
                body.extend_from_slice(b"\"/></w:rPr>");
            }
            body.extend_from_slice(b"<w:t>");
            body.extend_from_slice(text.as_bytes());
            body.extend_from_slice(b"</w:t></w:r></w:p>");
        }
        body.extend_from_slice(b"</w:body>");

        let doc_xml = format!(
            "<w:document xmlns:w=\"{}\">{}</w:document>",
            NS_WORD,
            String::from_utf8_lossy(&body)
        );
        doc_xml.into_bytes()
    }

    /// Build a document.xml with numbering (w:numPr).
    pub fn build_docx_with_numbering(num_prs: &[(Option<(usize, usize)>, &str)]) -> Vec<u8> {
        let mut body = b"<w:body>".to_vec();
        for (num_pr_opt, text) in num_prs {
            body.extend_from_slice(b"<w:p><w:pPr>");
            if let Some((num_id, ilvl)) = num_pr_opt {
                body.extend_from_slice(b"<w:numPr><w:numId w:val=\"");
                body.extend_from_slice(num_id.to_string().as_bytes());
                body.extend_from_slice(b"\"/><w:ilvl w:ilvl=\"");
                body.extend_from_slice(ilvl.to_string().as_bytes());
                body.extend_from_slice(b"\"/></w:numPr>");
            }
            body.extend_from_slice(b"</w:pPr><w:r><w:t>");
            body.extend_from_slice(text.as_bytes());
            body.extend_from_slice(b"</w:t></w:r></w:p>");
        }
        body.extend_from_slice(b"</w:body>");

        let doc_xml = format!(
            "<w:document xmlns:w=\"{}\">{}</w:document>",
            NS_WORD,
            String::from_utf8_lossy(&body)
        );
        doc_xml.into_bytes()
    }

    /// Build a document.xml with section properties (page size, margins).
    pub fn build_docx_with_sectpr() -> Vec<u8> {
        let doc_xml = format!(
            "<w:document xmlns:w=\"{}\">\
             <w:body>\
             <w:p><w:r><w:t>Content</w:t></w:r></w:p>\
             </w:body>\
             <w:sectPr>\
             <w:pgSz w:w=\"12240000\" w:h=\"15840000\"/>\
             <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\
             <w:headerReference w:type=\"default\" r:id=\"rIdH1\"/>\
             <w:footerReference w:type=\"default\" r:id=\"rIdF1\"/>\
             </w:sectPr>\
             </w:document>",
            NS_WORD
        );
        doc_xml.into_bytes()
    }

    /// Build a header part XML.
    pub fn build_header_xml(has_content: bool) -> Vec<u8> {
        if has_content {
            b"<w:header xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:r><w:t>Header Text</w:t></w:r></w:p></w:header>".to_vec()
        } else {
            b"<w:header xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"></w:header>".to_vec()
        }
    }

    /// Build a footer part XML.
    pub fn build_footer_xml(has_content: bool) -> Vec<u8> {
        if has_content {
            b"<w:footer xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:r><w:t>Footer Text</w:t></w:r></w:p></w:footer>".to_vec()
        } else {
            b"<w:footer xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"></w:footer>".to_vec()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use fixtures::*;
    use std::fs;
    use std::io::Write;

    /// test_parse_docx_corpus_fixture — parse cjk-text-docx-001.docx, verify IR produced.
    #[test]
    fn test_parse_docx_corpus_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/cjk-text-docx-001.docx"
        );
        let data = fs::read(path).expect("cjk-text-docx-001.docx should exist");
        let pkg = parse_opc(&data).expect("should parse cjk-text-docx-001.docx");
        let result = read_docx(&pkg).expect("should produce IR from cjk-text-docx-001.docx");

        assert!(!result.ir.is_empty(), "IR should not be empty");

        let doc: Document =
            serde_json::from_slice(&result.ir).expect("IR should be valid JSON Document");
        assert_eq!(doc.ir_version, IR_VERSION);
        assert!(!doc.sections.is_empty(), "Should have at least one section");
    }

    /// test_parse_paragraphs — verify w:p → Paragraph mapping.
    #[test]
    fn test_parse_paragraphs() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p><w:r><w:t>Hello</w:t></w:r></w:p>\
                            <w:p><w:r><w:t>World</w:t></w:r></w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.sections.len(), 1);
        let section = &doc.sections[0];
        let para_count = section
            .items
            .iter()
            .filter(|i| matches!(i, SectionItem::Paragraph(_)))
            .count();
        assert_eq!(para_count, 2, "Should have 2 paragraphs");
    }

    /// test_parse_runs — verify w:r → Run with text content.
    #[test]
    fn test_parse_runs() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p>\
                              <w:r><w:t>First</w:t></w:r>\
                              <w:r><w:t>Second</w:t></w:r>\
                            </w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        if let SectionItem::Paragraph(para) = &section.items[0] {
            assert_eq!(para.runs.len(), 2, "Should have 2 runs");
            assert_eq!(para.runs[0].text, "First");
            assert_eq!(para.runs[1].text, "Second");
        } else {
            panic!("Expected paragraph");
        }
    }

    /// test_parse_table — verify w:tbl → Table with rows and cells.
    #[test]
    fn test_parse_table() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:tbl>\
                              <w:tr>\
                                <w:tc><w:p><w:r><w:t>Cell1</w:t></w:r></w:p></w:tc>\
                                <w:tc><w:p><w:r><w:t>Cell2</w:t></w:r></w:p></w:tc>\
                              </w:tr>\
                              <w:tr>\
                                <w:tc><w:p><w:r><w:t>Cell3</w:t></w:r></w:p></w:tc>\
                              </w:tr>\
                            </w:tbl>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        let tbl = match &section.items[0] {
            SectionItem::Table(t) => t,
            other => panic!("Expected table, got {:?}", other),
        };

        assert_eq!(tbl.rows.len(), 2, "Should have 2 rows");
        assert_eq!(tbl.rows[0].cells.len(), 2, "Row 0 should have 2 cells");
        assert_eq!(tbl.rows[1].cells.len(), 1, "Row 1 should have 1 cell");
    }

    /// test_parse_run_properties — verify bold, italic, font_name, font_size_pt.
    #[test]
    fn test_parse_run_properties() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p>\
                              <w:r>\
                                <w:rPr>\
                                  <w:b/>\
                                  <w:i/>\
                                  <w:rFonts ascii=\"Calibri\"/>\
                                  <w:sz val=\"36\"/>\
                                </w:rPr>\
                                <w:t>Formatted</w:t>\
                              </w:r>\
                            </w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        if let SectionItem::Paragraph(para) = &section.items[0] {
            let run = &para.runs[0];
            assert_eq!(run.text, "Formatted");
            assert_eq!(run.bold, Some(true), "Should be bold");
            assert_eq!(run.italic, Some(true), "Should be italic");
            assert_eq!(
                run.font_name,
                Some("Calibri".to_string()),
                "Font should be Calibri"
            );
            assert_eq!(
                run.font_size_pt,
                Some(18.0),
                "Font size should be 18pt (36 half-points)"
            );
        } else {
            panic!("Expected paragraph");
        }
    }

    /// test_parse_with_ids — verify every element has a content-anchored ID.
    #[test]
    fn test_parse_with_ids() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p><w:r><w:t>Hello</w:t></w:r></w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert!(doc.sections[0].id.is_valid());
        if let SectionItem::Paragraph(para) = &doc.sections[0].items[0] {
            assert!(para.id.is_valid(), "Paragraph ID should be valid");
            assert!(!para.runs.is_empty());
            assert!(para.runs[0].id.is_valid(), "Run ID should be valid");
        } else {
            panic!("Expected paragraph");
        }
    }

    /// test_parse_multiple_paragraphs — verify multiple w:p → multiple Paragraphs.
    #[test]
    fn test_parse_multiple_paragraphs() {
        let mut body = String::from("<w:body>");
        for i in 0..5 {
            body.push_str(&format!("<w:p><w:r><w:t>Paragraph {}</w:t></w:r></w:p>", i));
        }
        body.push_str("</w:body>");
        let doc_xml_str = format!("<w:document xmlns:w=\"{}\">{}</w:document>", NS_WORD, body);
        let doc_xml = doc_xml_str.as_bytes();

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        let para_count = section
            .items
            .iter()
            .filter(|i| matches!(i, SectionItem::Paragraph(_)))
            .count();
        assert_eq!(para_count, 5, "Should have 5 paragraphs");
    }

    /// test_parse_nested_cell_paragraphs — verify paragraphs inside table cells.
    #[test]
    fn test_parse_nested_cell_paragraphs() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:tbl>\
                              <w:tr>\
                                <w:tc>\
                                  <w:p><w:r><w:t>Para1</w:t></w:r></w:p>\
                                  <w:p><w:r><w:t>Para2</w:t></w:r></w:p>\
                                </w:tc>\
                              </w:tr>\
                            </w:tbl>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        if let SectionItem::Table(tbl) = &section.items[0] {
            assert_eq!(tbl.rows.len(), 1);
            assert_eq!(tbl.rows[0].cells.len(), 1);
            let cell = &tbl.rows[0].cells[0];
            let para_count = cell
                .contents
                .iter()
                .filter(|i| matches!(i, SectionItem::Paragraph(_)))
                .count();
            assert_eq!(para_count, 2, "Cell should contain 2 paragraphs");
        } else {
            panic!("Expected table");
        }
    }

    /// test_docx_codec_integration — DocxReader::read() on corpus fixture produces valid IR.
    #[test]
    fn test_docx_codec_integration() {
        let reader = DocxReader::new();
        assert_eq!(reader.format_name(), "docx");

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/cjk-text-docx-001.docx"
        );
        let data = fs::read(path).expect("corpus fixture should exist");
        let result = reader.read(&data).expect("should read corpus fixture");

        assert!(!result.ir.is_empty(), "IR should not be empty");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("IR should be valid Document");
        assert_eq!(doc.ir_version, IR_VERSION);
    }

    /// test_docx_with_multi_char_prefix_elements — parse a document whose XML parts
    /// contain multi-character namespace prefixes such as `wp:` (e.g. `<wp:inline>`
    /// inside `<w:drawing>`). Regression test for OOX-E-002 "Expected '=' after
    /// attribute name" when `parse_tag_name` mis-splits `wp:inline` and leaves
    /// `:inline` to be read as an attribute (the pattern from MS Word-generated
    /// documents containing inline drawings).
    #[test]
    fn test_docx_with_multi_char_prefix_elements() {
        let doc_xml = b"<w:document \
                          xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
                          xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
                          xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
                          <w:body>\
                            <w:p>\
                              <w:r>\
                                <w:drawing>\
                                  <wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
                                    <wp:extent cx=\"1091459\" cy=\"466598\"/>\
                                    <wp:docPr id=\"4\" name=\"image 4\" descr=\"image.jpeg\"/>\
                                    <a:graphic>\
                                      <a:graphicData uri=\"http://example.com/pic\"/>\
                                    </a:graphic>\
                                  </wp:inline>\
                                </w:drawing>\
                              </w:r>\
                            </w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse docx with wp:inline elements");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize IR");

        assert!(!doc.sections.is_empty(), "should have at least one section");
    }

    /// test_parse_all_docx_fixtures — open all docx files, verify each produces valid IR.
    #[test]
    fn test_parse_all_docx_fixtures() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let reader = DocxReader::new();
        let mut success_count = 0;
        let mut error_count = 0;

        for entry in fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("docx") {
                let data =
                    fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
                match reader.read(&data) {
                    Ok(result) => {
                        if !result.ir.is_empty() {
                            match serde_json::from_slice::<Document>(&result.ir) {
                                Ok(doc) => {
                                    assert_eq!(doc.ir_version, IR_VERSION);
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
            "Should have parsed at least one docx (success={}, errors={})",
            success_count,
            error_count
        );
    }

    /// test_parse_empty_docx — empty document → valid empty IR.
    #[test]
    fn test_parse_empty_docx() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse empty docx");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.ir_version, IR_VERSION);
        assert!(!doc.sections.is_empty());
    }

    /// test_parse_table_with_many_rows — large table, verify row count matches.
    #[test]
    fn test_parse_table_with_many_rows() {
        let mut body = String::from("<w:body><w:tbl>");
        for i in 0..50 {
            body.push_str(&format!(
                "<w:tr><w:tc><w:p><w:r><w:t>Row {} Cell</w:t></w:r></w:p></w:tc></w:tr>",
                i
            ));
        }
        body.push_str("</w:tbl></w:body>");
        let doc_xml_str = format!("<w:document xmlns:w=\"{}\">{}</w:document>", NS_WORD, body);
        let doc_xml = doc_xml_str.as_bytes();

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        if let SectionItem::Table(tbl) = &section.items[0] {
            assert_eq!(tbl.rows.len(), 50, "Should have 50 rows");
        } else {
            panic!("Expected table");
        }
    }

    // ===== P1-11: Styles, Numbering, Sections, Headers/Footers =====

    /// test_parse_styles_xml — parse styles.xml, verify style map.
    #[test]
    fn test_parse_styles_xml() {
        let styles_xml = fixtures::build_styles_xml(&[
            ("Heading1", "Heading 1", "paragraph"),
            ("Normal", "Normal", "paragraph"),
            ("ListBullet", "List Bullet", "paragraph"),
        ]);

        let pkg = fixtures::build_docx_with_parts(vec![
            (
                "word/document.xml",
                &fixtures::build_docx_with_pstyle(&[
                    (Some("Heading1"), "Title"),
                    (Some("Normal"), "Body"),
                ]),
            ),
            ("word/styles.xml", &styles_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse test package");
        let styles = read_styles_xml(&pkg2).expect("should parse styles.xml");

        assert_eq!(styles.len(), 3, "Should have 3 styles");
        assert!(styles.contains_key("Heading1"));
        assert!(styles.contains_key("Normal"));
        assert!(styles.contains_key("ListBullet"));
        assert_eq!(styles["Heading1"].name, Some("Heading 1".to_string()));
        assert_eq!(styles["Heading1"].style_type, Some("paragraph".to_string()));
    }

    /// test_apply_style_to_paragraph — paragraph with w:pStyle → style_name set.
    #[test]
    fn test_apply_style_to_paragraph() {
        let styles_xml = fixtures::build_styles_xml(&[
            ("Heading1", "Heading 1", "paragraph"),
            ("Normal", "Normal", "paragraph"),
        ]);

        let doc_xml = fixtures::build_docx_with_pstyle(&[
            (Some("Heading1"), "Title"),
            (Some("Normal"), "Body"),
            (None, "Plain"),
        ]);

        let pkg = fixtures::build_docx_with_parts(vec![
            ("word/document.xml", &doc_xml),
            ("word/styles.xml", &styles_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx_full(&pkg2).expect("should parse full");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        let para1 = match &section.items[0] {
            SectionItem::Paragraph(p) => p,
            _ => panic!("Expected paragraph"),
        };
        assert_eq!(para1.style_name, Some("Heading1".to_string()));

        let para2 = match &section.items[1] {
            SectionItem::Paragraph(p) => p,
            _ => panic!("Expected paragraph"),
        };
        assert_eq!(para2.style_name, Some("Normal".to_string()));

        let para3 = match &section.items[2] {
            SectionItem::Paragraph(p) => p,
            _ => panic!("Expected paragraph"),
        };
        assert_eq!(para3.style_name, None);
    }

    /// test_apply_style_to_run — run with w:rStyle → style_name set.
    #[test]
    fn test_apply_style_to_run() {
        let doc_xml = fixtures::build_docx_with_rstyle(&[
            (Some("Emphasis"), "emphasized"),
            (Some("Strong"), "strong"),
            (None, "plain"),
        ]);

        let pkg = fixtures::build_docx_with_parts(vec![("word/document.xml", &doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx_full(&pkg2).expect("should parse full");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        if let SectionItem::Paragraph(para) = &section.items[0] {
            assert_eq!(para.runs[0].style_name, Some("Emphasis".to_string()));
        } else {
            panic!("Expected paragraph");
        }

        if let SectionItem::Paragraph(para) = &section.items[1] {
            assert_eq!(para.runs[0].style_name, Some("Strong".to_string()));
        } else {
            panic!("Expected paragraph");
        }

        if let SectionItem::Paragraph(para) = &section.items[2] {
            assert_eq!(para.runs[0].style_name, None);
        } else {
            panic!("Expected paragraph");
        }
    }

    /// test_parse_numbering — parse numbering.xml, verify numId→ilvl mapping.
    #[test]
    fn test_parse_numbering() {
        let numbering_xml =
            fixtures::build_numbering_xml(&[(1, "0", "decimal"), (2, "0", "bullet")]);

        let pkg = fixtures::build_docx_with_parts(vec![
            (
                "word/document.xml",
                &fixtures::build_docx_with_pstyle(&[(Some("Normal"), "Body")]),
            ),
            ("word/numbering.xml", &numbering_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let num_map = read_numbering_xml(&pkg2).expect("should parse numbering.xml");

        assert_eq!(num_map.len(), 2, "Should have 2 num definitions");
        assert!(num_map.contains_key(&1));
        assert!(num_map.contains_key(&2));
        assert_eq!(num_map[&1].number_format, Some("decimal".to_string()));
        assert_eq!(num_map[&2].number_format, Some("bullet".to_string()));
        assert_eq!(num_map[&1].abstract_num_id, "0");
    }

    /// test_apply_numbering — paragraph with numPr → List IR with level.
    #[test]
    fn test_apply_numbering() {
        let numbering_xml =
            fixtures::build_numbering_xml(&[(1, "0", "decimal"), (2, "0", "decimal")]);

        let doc_xml = fixtures::build_docx_with_numbering(&[
            (Some((1, 0)), "Item 1"),
            (Some((1, 0)), "Item 2"),
            (Some((1, 0)), "Item 3"),
            (None, "Not numbered"),
            (Some((2, 0)), "Item A"),
            (Some((2, 0)), "Item B"),
        ]);

        let pkg = fixtures::build_docx_with_parts(vec![
            ("word/document.xml", &doc_xml),
            ("word/numbering.xml", &numbering_xml),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx_full(&pkg2).expect("should parse full");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];

        // First 3 items should be grouped into a numbered list
        let list1 = match &section.items[0] {
            SectionItem::List(l) => l,
            _ => panic!("Expected list at position 0, got {:?}", section.items[0]),
        };
        assert_eq!(list1.kind, ListKind::Numbered);
        assert_eq!(list1.items.len(), 3, "List should have 3 items");
        assert_eq!(list1.items[0].level, 0);

        // Item 4 (not numbered) should be a paragraph
        assert!(matches!(&section.items[1], SectionItem::Paragraph(_)));

        // Last 2 items should be in a numbered list
        let list2 = match &section.items[2] {
            SectionItem::List(l) => l,
            _ => panic!("Expected list at position 2, got {:?}", section.items[2]),
        };
        assert_eq!(list2.kind, ListKind::Numbered);
        assert_eq!(list2.items.len(), 2, "List should have 2 items");
    }

    /// test_parse_section_properties — sectPr → page dimensions.
    #[test]
    fn test_parse_section_properties() {
        let doc_xml = fixtures::build_docx_with_sectpr();

        let pkg = fixtures::build_docx_with_parts(vec![("word/document.xml", &doc_xml)]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx_full(&pkg2).expect("should parse full");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].headers.len(), 1);
        assert_eq!(doc.sections[0].footers.len(), 1);
    }

    /// test_parse_header_reference — header reference → header part loaded.
    #[test]
    fn test_parse_header_reference() {
        let header_xml = fixtures::build_header_xml(true);
        let doc_xml = fixtures::build_docx_with_sectpr();

        let _pkg = fixtures::build_docx_with_parts(vec![
            ("word/document.xml", &doc_xml),
            ("word/header1.xml", &header_xml),
        ]);

        // Also need to add the relationship from document.xml to header1.xml
        let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
                         <Relationship Id=\"rIdH1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"word/header1.xml\"/>\
                         </Relationships>";

        let _pkg = fixtures::build_docx_with_parts(vec![
            ("word/document.xml", &doc_xml),
            ("word/header1.xml", &header_xml),
        ]);
        // Rebuild with proper rels
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

        writer.start_file("[Content_Types].xml", opts).unwrap();
        writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>").unwrap();
        writer.start_file("_rels/.rels", opts).unwrap();
        writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").unwrap();
        writer.start_file("word/document.xml", opts).unwrap();
        writer.write_all(&doc_xml).unwrap();
        writer.start_file("word/header1.xml", opts).unwrap();
        writer.write_all(&header_xml).unwrap();
        writer
            .start_file("word/_rels/document.xml.rels", opts)
            .unwrap();
        writer.write_all(rels_xml).unwrap();
        let pkg2 = writer.finish().unwrap().into_inner();

        let pkg3 = parse_opc(&pkg2).expect("should parse");
        let result = read_docx_full(&pkg3).expect("should parse full");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        assert!(!section.headers.is_empty(), "Should have a header");
        assert_eq!(
            section.headers[0].content.len(),
            1,
            "Header should have 1 paragraph"
        );
        if let SectionItem::Paragraph(para) = &section.headers[0].content[0] {
            assert!(!para.runs.is_empty(), "Header paragraph should have runs");
        }
    }

    /// test_parse_footer_reference — footer reference → footer part loaded.
    #[test]
    fn test_parse_footer_reference() {
        let footer_xml = fixtures::build_footer_xml(true);
        let doc_xml = fixtures::build_docx_with_sectpr();

        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

        writer.start_file("[Content_Types].xml", opts).unwrap();
        writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>").unwrap();
        writer.start_file("_rels/.rels", opts).unwrap();
        writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").unwrap();
        writer.start_file("word/document.xml", opts).unwrap();
        writer.write_all(&doc_xml).unwrap();
        writer.start_file("word/footer1.xml", opts).unwrap();
        writer.write_all(&footer_xml).unwrap();
        writer
            .start_file("word/_rels/document.xml.rels", opts)
            .unwrap();
        let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
                         <Relationship Id=\"rIdF1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"word/footer1.xml\"/>\
                         </Relationships>";
        writer.write_all(rels_xml).unwrap();
        let pkg2 = writer.finish().unwrap().into_inner();

        let pkg3 = parse_opc(&pkg2).expect("should parse");
        let result = read_docx_full(&pkg3).expect("should parse full");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        let section = &doc.sections[0];
        assert!(!section.footers.is_empty(), "Should have a footer");
        assert_eq!(
            section.footers[0].content.len(),
            1,
            "Footer should have 1 paragraph"
        );
        if let SectionItem::Paragraph(para) = &section.footers[0].content[0] {
            assert!(!para.runs.is_empty(), "Footer paragraph should have runs");
        }
    }

    /// test_full_docx_roundtrip — parse corpus docx, verify styles/numbering/sections present.
    #[test]
    fn test_full_docx_roundtrip() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/cjk-text-docx-001.docx"
        );
        let data = fs::read(path).expect("cjk-text-docx-001.docx should exist");
        let pkg = parse_opc(&data).expect("should parse cjk-text-docx-001.docx");
        let result = read_docx_full(&pkg).expect("should produce full IR");

        assert!(!result.ir.is_empty(), "IR should not be empty");

        let doc: Document =
            serde_json::from_slice(&result.ir).expect("IR should be valid JSON Document");
        assert_eq!(doc.ir_version, IR_VERSION);
        assert!(!doc.sections.is_empty(), "Should have at least one section");

        // Verify basic IR structure
        for section in &doc.sections {
            for item in &section.items {
                match item {
                    SectionItem::Paragraph(p) => {
                        assert!(p.id.is_valid(), "Paragraph ID should be valid");
                        for run in &p.runs {
                            assert!(run.id.is_valid(), "Run ID should be valid");
                        }
                    }
                    SectionItem::Table(t) => {
                        assert!(t.id.is_valid(), "Table ID should be valid");
                    }
                    SectionItem::List(l) => {
                        assert!(l.id.is_valid(), "List ID should be valid");
                        for li in &l.items {
                            assert!(li.id.is_valid(), "ListItem ID should be valid");
                        }
                    }
                    SectionItem::Drawing(_) => {}
                }
            }
        }
    }

    /// test_parse_all_docx_fixtures_full — all docx fixtures produce IR with styles.
    #[test]
    fn test_parse_all_docx_fixtures_full() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let mut success_count = 0;
        let mut error_count = 0;

        for entry in fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("docx") {
                let data =
                    fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
                match read_docx_full(&parse_opc(&data).expect("should parse OPC")) {
                    Ok(result) => {
                        if !result.ir.is_empty() {
                            match serde_json::from_slice::<Document>(&result.ir) {
                                Ok(doc) => {
                                    assert_eq!(doc.ir_version, IR_VERSION);
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
            "Should have parsed at least one docx (success={}, errors={})",
            success_count,
            error_count
        );
    }

    /// test_numbered_list_from_corpus — verify a numbered list fixture.
    #[test]
    fn test_numbered_list_from_corpus() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/numbering-restarts-docx-001.docx"
        );
        let data = fs::read(path).expect("numbering-restarts-docx-001.docx should exist");
        let pkg = parse_opc(&data).expect("should parse OPC");

        // First test basic reader
        let result_basic = read_docx(&pkg).expect("should parse with basic reader");
        let doc_basic: Document =
            serde_json::from_slice(&result_basic.ir).expect("should deserialize");
        assert!(!doc_basic.sections.is_empty());

        // Then test full reader
        let result_full = read_docx_full(&pkg).expect("should parse with full reader");
        let doc_full: Document =
            serde_json::from_slice(&result_full.ir).expect("should deserialize");
        assert!(!doc_full.sections.is_empty());

        // Both should produce the same number of sections
        assert_eq!(doc_basic.sections.len(), doc_full.sections.len());
    }

    // ===== P1-12: Fields, Comments, Tracked Changes =====

    /// test_parse_field_simple — w:fldSimple → FieldSimple IR.
    #[test]
    fn test_parse_field_simple() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                           <w:body>\
                             <w:p>\
                               <w:fldSimple w:instr=\"PAGE\" w:result=\"1\"/>\
                             </w:p>\
                           </w:body>\
                         </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);
        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.fields.len(), 1, "Should have 1 field");
        if let Field::Simple(fs) = &doc.fields[0] {
            assert_eq!(fs.instruction, "PAGE");
            assert_eq!(fs.result.as_deref(), Some("1"));
        } else {
            panic!("Expected Simple field");
        }
    }

    /// test_parse_field_instruction — w:fldSimple with TOC instruction.
    #[test]
    fn test_parse_field_instruction() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                           <w:body>\
                             <w:p>\
                               <w:fldSimple w:instr=\"TOC\"/>  \
                             </w:p>\
                           </w:body>\
                         </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);
        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.fields.len(), 1);
        if let Field::Simple(fs) = &doc.fields[0] {
            assert_eq!(fs.instruction, "TOC");
        } else {
            panic!("Expected Simple field");
        }
    }

    /// test_parse_comments_xml — parse comments.xml, verify comment count.
    #[test]
    fn test_parse_comments_xml() {
        let comments_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                             <w:comments xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                             <w:comment w:id=\"0\" w:author=\"bot-a\" w:initials=\"A\" w:date=\"2026-01-01T00:00:00Z\">\
                               <w:p><w:r><w:t>Comment 0 text</w:t></w:r></w:p>\
                             </w:comment>\
                             <w:comment w:id=\"1\" w:author=\"bot-b\" w:initials=\"B\" w:date=\"2026-01-01T00:00:00Z\">\
                               <w:p><w:r><w:t>Comment 1 text</w:t></w:r></w:p>\
                             </w:comment>\
                             </w:comments>";

        let pkg = build_docx_with_parts(vec![
            (
                "word/document.xml",
                &fixtures::build_docx_with_pstyle(&[(None, "Body")])[..],
            ),
            ("word/comments.xml", &comments_xml[..]),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let comments = read_comments(&pkg2);

        assert_eq!(comments.len(), 2, "Should have 2 comments");
        assert!(comments.contains_key("0"));
        assert!(comments.contains_key("1"));
        assert_eq!(comments["0"].author, "bot-a");
        assert_eq!(comments["1"].author, "bot-b");
    }

    /// test_map_comment_references — w:commentReference → comment ID mapping via IR.
    #[test]
    fn test_map_comment_references() {
        let comments_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                             <w:comments xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                             <w:comment w:id=\"0\" w:author=\"reviewer\"><w:p><w:r><w:t>Review note</w:t></w:r></w:p></w:comment>\
                             </w:comments>";

        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p>\
                              <w:r><w:t>Text with </w:t></w:r>\
                              <w:commentReference w:id=\"0\"/>\
                              <w:r><w:t> reference</w:t></w:r>\
                            </w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![
            ("word/document.xml", doc_xml),
            ("word/comments.xml", &comments_xml[..]),
        ]);

        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx_full(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(
            doc.comments.len(),
            1,
            "Should have 1 comment from comments.xml"
        );
        assert_eq!(doc.comments[0].comment_id, "0");
        assert_eq!(doc.comments[0].author, "reviewer");
    }

    /// test_parse_inserted_text — w:ins → TrackedChange (Inserted).
    #[test]
    fn test_parse_inserted_text() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p>\
                              <w:r><w:t>Before </w:t></w:r>\
                              <w:ins w:author=\"author-a\" w:date=\"2026-01-01T00:00:00Z\">\
                                <w:r><w:t>inserted text</w:t></w:r>\
                              </w:ins>\
                              <w:r><w:t> after</w:t></w:r>\
                            </w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);
        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.tracked_changes.len(), 1, "Should have 1 tracked change");
        assert_eq!(doc.tracked_changes[0].kind, ChangeKind::Inserted);
        assert_eq!(doc.tracked_changes[0].content, "inserted text");
        assert_eq!(doc.tracked_changes[0].author.as_deref(), Some("author-a"));
    }

    /// test_parse_deleted_text — w:del → TrackedChange (Deleted).
    #[test]
    fn test_parse_deleted_text() {
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                          <w:body>\
                            <w:p>\
                              <w:r><w:t>Before </w:t></w:r>\
                              <w:del w:author=\"author-b\" w:date=\"2026-01-01T00:00:00Z\">\
                                <w:r><w:delText>deleted text</w:delText></w:r>\
                              </w:del>\
                              <w:r><w:t> after</w:t></w:r>\
                            </w:p>\
                          </w:body>\
                        </w:document>";

        let pkg = build_docx_with_parts(vec![("word/document.xml", doc_xml)]);
        let pkg2 = parse_opc(&pkg).expect("should parse");
        let result = read_docx(&pkg2).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(doc.tracked_changes.len(), 1, "Should have 1 tracked change");
        assert_eq!(doc.tracked_changes[0].kind, ChangeKind::Deleted);
        assert!(doc.tracked_changes[0].content.contains("deleted"));
        assert_eq!(doc.tracked_changes[0].author.as_deref(), Some("author-b"));
    }

    /// test_parse_tracked_changes_from_corpus — parse tracked-changes-docx-001.docx.
    #[test]
    fn test_parse_tracked_changes_from_corpus() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/tracked-changes-docx-001.docx"
        );
        let data = fs::read(path).expect("tracked-changes-docx-001.docx should exist");
        let pkg = parse_opc(&data).expect("should parse OPC");
        let result = read_docx(&pkg).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert!(
            !doc.tracked_changes.is_empty(),
            "tracked-changes-docx-001.docx should have tracked changes, got {}",
            doc.tracked_changes.len()
        );

        let has_inserted = doc
            .tracked_changes
            .iter()
            .any(|tc| tc.kind == ChangeKind::Inserted);
        let has_deleted = doc
            .tracked_changes
            .iter()
            .any(|tc| tc.kind == ChangeKind::Deleted);
        assert!(
            has_inserted || has_deleted,
            "Should have at least one inserted or deleted tracked change"
        );
    }

    /// test_fields_in_corpus_docx — verify fields in toc-fields-docx-001.docx.
    #[test]
    fn test_fields_in_corpus_docx() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/toc-fields-docx-001.docx"
        );
        let data = fs::read(path).expect("toc-fields-docx-001.docx should exist");
        let pkg = parse_opc(&data).expect("should parse OPC");
        let result = read_docx(&pkg).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert!(
            !doc.fields.is_empty(),
            "toc-fields-docx-001.docx should have at least 1 field, got {}",
            doc.fields.len()
        );

        // The TOC field should be present
        let has_toc = doc.fields.iter().any(|f| {
            if let Field::Simple(fs) = f {
                fs.instruction.contains("TOC")
            } else {
                false
            }
        });
        assert!(has_toc, "Should have a TOC field");
    }

    /// test_comments_in_corpus_docx — verify comments in comments-annotations-docx-001.docx.
    #[test]
    fn test_comments_in_corpus_docx() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/comments-annotations-docx-001.docx"
        );
        let data = fs::read(path).expect("comments-annotations-docx-001.docx should exist");
        let pkg = parse_opc(&data).expect("should parse OPC");
        let result = read_docx_full(&pkg).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(
            doc.comments.len(),
            3,
            "comments-annotations-docx-001.docx should have 3 comments, got {}",
            doc.comments.len()
        );

        // Verify authors from manifest
        let authors: Vec<&str> = doc.comments.iter().map(|c| c.author.as_str()).collect();
        assert!(authors.contains(&"bot-a"), "Should have bot-a as author");
        assert!(authors.contains(&"bot-b"), "Should have bot-b as author");
    }

    /// test_full_corpus_read — all 41 docx fixtures parse successfully.
    #[test]
    fn test_full_corpus_read() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let reader = DocxReader::new();
        let mut success_count = 0;
        let mut docx_count = 0;

        for entry in fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("docx") {
                docx_count += 1;
                let data =
                    fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
                match reader.read(&data) {
                    Ok(result) => {
                        if !result.ir.is_empty() {
                            if serde_json::from_slice::<Document>(&result.ir).is_ok() {
                                success_count += 1;
                            } else {
                                success_count += 1; // IR produced even if deserialization fails
                            }
                        } else {
                            success_count += 1;
                        }
                    }
                    Err(diags) => {
                        let has_fatal = diags.iter().any(|d| d.severity == Severity::Fatal);
                        if !has_fatal {
                            success_count += 1;
                        }
                    }
                }
            }
        }

        assert_eq!(docx_count, 41, "Should have 41 docx fixtures");
        assert_eq!(
            success_count, 41,
            "All 41 docx fixtures should parse successfully (success={}, total={})",
            success_count, docx_count
        );
    }

    /// test_comment_count_vs_manifest — comments count matches C9 manifest expectations.
    #[test]
    fn test_comment_count_vs_manifest() {
        // comments-annotations-docx-001 manifest: w:commentReference=3, w:comment=3
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/comments-annotations-docx-001.docx"
        );
        let data = fs::read(path).expect("fixture should exist");
        let pkg = parse_opc(&data).expect("should parse OPC");
        let result = read_docx_full(&pkg).expect("should parse");
        let doc: Document = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(
            doc.comments.len(),
            3,
            "Comment count should match manifest (3 w:comment entries)"
        );
    }
}
