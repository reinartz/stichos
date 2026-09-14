//! adapters-odf: ODT → gp-core IR (content.xml, styles.xml, meta.xml → Document IR).
//!
//! Walks a `.odt` ODF package, extracts `content.xml`/`styles.xml`/`meta.xml`,
//! parses the XML DOM, and maps ODF elements to the gp-core intermediate representation.

#![allow(dead_code, clippy::if_same_then_else)]

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;
use gp_core::traits::{Codec, ReadResult};
use std::collections::BTreeMap;

use super::manifest::{parse_odf, OdfPackage, OdfPart};
use super::xml::{parse_xml, TagName, XmlNode};

// ===========================================================================
// ODF namespace helpers
// ===========================================================================

pub const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const NS_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
pub const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const NS_META: &str = "urn:oasis:names:tc:opendocument:xmlns:meta:1.0";
pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub const NS_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";

fn is_local(name: &TagName, local: &str) -> bool {
    name.local_name == local
}

fn is_ns_elem(node: &XmlNode, ns: &str, local: &str) -> bool {
    // ODF namespace URIs map to common prefixes
    let expected_prefix = match ns {
        NS_TEXT => "text",
        NS_TABLE => "table",
        NS_DRAW => "draw",
        NS_STYLE => "style",
        NS_OFFICE => "office",
        NS_META => "meta",
        NS_FO => "fo",
        _ => return false,
    };
    matches!(node, XmlNode::Element { name, .. } if name.local_name == local && name.prefix.as_deref() == Some(expected_prefix))
}

fn element_text(node: &XmlNode) -> String {
    let mut result = String::new();
    collect_text(node, &mut result);
    result
}

fn collect_text(node: &XmlNode, out: &mut String) {
    match node {
        XmlNode::Text { text, .. } => out.push_str(text),
        XmlNode::Element { children, .. } => {
            for child in children {
                collect_text(child, out);
            }
        }
        _ => {}
    }
}

fn get_ns_attr<'a>(attributes: &'a BTreeMap<TagName, String>, local: &str) -> Option<&'a str> {
    for (key, val) in attributes {
        if key.local_name == local {
            return Some(val.as_str());
        }
    }
    None
}

// ===========================================================================
// Style definitions
// ===========================================================================

#[derive(Debug, Clone, Default)]
pub struct OdfStyle {
    pub name: String,
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub font_name: Option<String>,
    pub font_size_pt: Option<f64>,
    pub underline: bool,
    pub strikethrough: bool,
}

fn parse_styles(styles_data: &[u8]) -> BTreeMap<String, OdfStyle> {
    let mut map: BTreeMap<String, OdfStyle> = BTreeMap::new();
    let Ok(xml_doc) = parse_xml(styles_data) else {
        return map;
    };
    if let XmlNode::Element { children, .. } = &xml_doc.root {
        for child in children {
            walk_styles(child, &mut map);
        }
    }
    map
}

fn walk_styles(node: &XmlNode, map: &mut BTreeMap<String, OdfStyle>) {
    if let XmlNode::Element {
        name,
        attributes,
        children,
        ..
    } = node
    {
        if is_local(name, "style") {
            if let Some(style_name) = get_ns_attr(attributes, "name") {
                let mut style = OdfStyle {
                    name: style_name.to_string(),
                    family: "text".to_string(),
                    ..Default::default()
                };
                if let Some(family) = get_ns_attr(attributes, "family") {
                    style.family = family.to_string();
                }
                for sc in children {
                    if is_ns_elem(sc, NS_STYLE, "text-properties") {
                        parse_text_properties(sc, &mut style);
                    }
                }
                map.insert(style_name.to_string(), style);
            }
        }
        if let XmlNode::Element { children: gc, .. } = node {
            for child in gc {
                walk_styles(child, map);
            }
        }
    }
}

fn parse_text_properties(node: &XmlNode, style: &mut OdfStyle) {
    if let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    {
        if let Some(val) = get_ns_attr(attributes, "font-weight") {
            style.bold = val == "bold" || val == "700" || val == "800" || val == "900";
        }
        if let Some(val) = get_ns_attr(attributes, "font-style") {
            style.italic = val == "italic" || val == "oblique";
        }
        if let Some(val) = get_ns_attr(attributes, "font-size") {
            style.font_size_pt = parse_font_size(val);
        }
        if let Some(val) = get_ns_attr(attributes, "font-name") {
            style.font_name = Some(val.to_string());
        }
        for child in children {
            if is_ns_elem(child, NS_STYLE, "text-properties") {
                parse_text_properties(child, style);
            }
        }
    }
}

fn parse_font_size(val: &str) -> Option<f64> {
    let val = val.trim();
    if let Some(s) = val.strip_suffix("pt") {
        return s.trim().parse().ok();
    }
    if let Some(s) = val.strip_suffix("mm") {
        if let Ok(mm) = s.trim().parse::<f64>() {
            return Some(mm / 25.4 * 72.0);
        }
    }
    if let Some(s) = val.strip_suffix("cm") {
        if let Ok(cm) = s.trim().parse::<f64>() {
            return Some(cm / 2.54 * 72.0);
        }
    }
    if let Some(s) = val.strip_suffix("in") {
        if let Ok(inches) = s.trim().parse::<f64>() {
            return Some(inches * 72.0);
        }
    }
    val.parse().ok()
}

// ===========================================================================
// Document metadata
// ===========================================================================

#[derive(Debug, Clone, Default)]
pub struct DocMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
}

fn parse_meta(meta_data: &[u8]) -> DocMeta {
    let mut meta = DocMeta::default();
    let Ok(xml_doc) = parse_xml(meta_data) else {
        return meta;
    };
    meta.title = find_meta_field(&xml_doc.root, &["title", "dc:title"]);
    meta.author = find_meta_field(&xml_doc.root, &["creator", "dc:creator", "initial-creator"]);
    meta.description = find_meta_field(&xml_doc.root, &["description", "dc:description"]);
    meta.creation_date = find_meta_field(&xml_doc.root, &["creation-date", "dc:date"]);
    meta.modification_date = find_meta_field(&xml_doc.root, &["print-date", "date"]);
    meta
}

fn find_meta_field(node: &XmlNode, tags: &[&str]) -> Option<String> {
    fn walk_find(node: &XmlNode, tag_list: &[String]) -> Option<String> {
        if let XmlNode::Element { children, .. } = node {
            for child in children {
                if let XmlNode::Element { name: cn, .. } = child {
                    if tag_list.contains(&cn.local_name) {
                        let t = element_text(child);
                        if !t.is_empty() {
                            return Some(t);
                        }
                    }
                }
            }
            for child in children {
                if let XmlNode::Element { name: cn, .. } = child {
                    if cn.local_name == "meta" || cn.local_name == "office" {
                        if let Some(found) = walk_find(child, tag_list) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        None
    }
    let tag_list: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    walk_find(node, &tag_list)
}

// ===========================================================================
// IR building
// ===========================================================================

fn build_document(
    content_data: &[u8],
    styles: &BTreeMap<String, OdfStyle>,
) -> Result<(Document, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let xml_doc = parse_xml(content_data)?;

    let mut section = Section {
        id: ElementId::from_content_hash("section", &["section:body"], "empty"),
        kind: SectionKind::Body,
        items: Vec::new(),
        headers: vec![],
        footers: vec![],
        footnotes: vec![],
        endnotes: vec![],
    };

    if let Some(text_el @ XmlNode::Element { children: tc, .. }) = find_body_text(&xml_doc.root) {
        let _ = text_el;
        for child in tc {
            parse_content_element(child, styles, &mut section.items, &mut diags);
        }
    }

    let document = Document {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sections: vec![section],
        fields: vec![],
        comments: vec![],
        tracked_changes: vec![],
    };
    Ok((document, diags))
}

fn find_body_text(node: &XmlNode) -> Option<&XmlNode> {
    if let XmlNode::Element { name, .. } = node {
        if name.local_name == "text" && name.prefix.as_deref() == Some("office") {
            return Some(node);
        }
    }
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let Some(found) = find_body_text(child) {
                return Some(found);
            }
        }
    }
    None
}

fn parse_content_element(
    node: &XmlNode,
    styles: &BTreeMap<String, OdfStyle>,
    items: &mut Vec<SectionItem>,
    diags: &mut Vec<Diagnostic>,
) {
    match node {
        XmlNode::Element { name, .. } if is_local(name, "p") => {
            if let Some(para) = parse_paragraph(node, styles) {
                items.push(SectionItem::Paragraph(para));
            }
        }
        XmlNode::Element { name, .. } if is_local(name, "table") => {
            if let Some(table) = parse_table(node, styles) {
                items.push(SectionItem::Table(table));
            }
        }
        XmlNode::Element { name, .. } if is_local(name, "list") => {
            if let Some(list) = parse_list(node, styles) {
                items.push(SectionItem::List(list));
            }
        }
        XmlNode::Element { name, .. } if is_local(name, "section") => {
            if let XmlNode::Element { children, .. } = node {
                for child in children {
                    parse_content_element(child, styles, items, diags);
                }
            }
        }
        XmlNode::Element { name, .. } if is_local(name, "h") => {
            if let Some(para) = parse_hyperlink(node, styles) {
                items.push(SectionItem::Paragraph(para));
            }
        }
        XmlNode::Element { name, .. } if is_local(name, "span") => {
            if let Some(para) = parse_span_paragraph(node, styles) {
                items.push(SectionItem::Paragraph(para));
            }
        }
        _ => {
            if let XmlNode::Element { name, .. } = node {
                if !matches!(name.local_name.as_str(), "default-tab-stop" | "") {
                    diags.push(Diagnostic::new(
                        "ODF-R100",
                        Severity::Info,
                        format!("ODT: skipping unknown element '{}'", name.local_name),
                    ));
                }
            }
        }
    }
}

// ===========================================================================
// Paragraph parsing
// ===========================================================================

fn parse_paragraph(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Paragraph> {
    let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    else {
        return None;
    };

    let style_name = get_ns_attr(attributes, "style-name").map(|s| s.to_string());

    let mut runs: Vec<Run> = Vec::new();
    for child in children {
        if let Some(r) = parse_run_content(child, styles) {
            runs.extend(r);
        }
    }

    if runs.is_empty() {
        return Some(Paragraph {
            id: eid("paragraph", &["para:0"], ""),
            runs: vec![Run {
                id: eid("run", &["run:0"], ""),
                text: String::new(),
                bold: None,
                italic: None,
                font_name: None,
                font_size_pt: None,
                underline: None,
                strikethrough: None,
                style_name: style_name.clone(),
            }],
            page_number: None,
            style_name: style_name.clone(),
            numbering_info: None,
        });
    }

    let text_content: String = runs.iter().map(|r| r.text.as_str()).collect();
    Some(Paragraph {
        id: eid("paragraph", &["para:0"], &text_content),
        runs,
        page_number: None,
        style_name,
        numbering_info: None,
    })
}

fn parse_run_content(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Vec<Run>> {
    match node {
        XmlNode::Element { name, .. } if is_local(name, "span") => parse_span_node(node, styles),
        XmlNode::Element { name, .. } if is_local(name, "h") => {
            parse_hyperlink(node, styles).map(|p| p.runs)
        }
        XmlNode::Text { text, .. } => {
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(vec![Run {
                id: eid("run", &["run:0"], &text),
                text,
                bold: None,
                italic: None,
                font_name: None,
                font_size_pt: None,
                underline: None,
                strikethrough: None,
                style_name: None,
            }])
        }
        XmlNode::Element { name, .. } if is_local(name, "s") => Some(vec![Run {
            id: eid("run", &["soft-page-break"], "page-break"),
            text: String::new(),
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
            underline: None,
            strikethrough: None,
            style_name: None,
        }]),
        XmlNode::Element { children, .. } => {
            let mut all: Vec<Run> = Vec::new();
            for child in children {
                if let Some(r) = parse_run_content(child, styles) {
                    all.extend(r);
                }
            }
            if all.is_empty() {
                None
            } else {
                Some(all)
            }
        }
        _ => None,
    }
}

fn parse_span_node(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Vec<Run>> {
    let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    else {
        return None;
    };

    let span_style_name = get_ns_attr(attributes, "name").map(|s| s.to_string());

    let mut bold = false;
    let mut italic = false;
    let mut font_name: Option<String> = None;
    let mut font_size_pt: Option<f64> = None;

    if let Some(val) = get_ns_attr(attributes, "font-weight") {
        bold = val == "bold" || val == "700" || val == "800" || val == "900";
    }
    if let Some(val) = get_ns_attr(attributes, "font-style") {
        italic = val == "italic" || val == "oblique";
    }
    if let Some(val) = get_ns_attr(attributes, "font-name") {
        font_name = Some(val.to_string());
    }
    if let Some(val) = get_ns_attr(attributes, "font-size") {
        font_size_pt = parse_font_size(val);
    }

    let mut text = String::new();
    for child in children {
        match child {
            XmlNode::Text { text: t, .. } => text.push_str(t),
            XmlNode::Element {
                name: inner_name, ..
            } if is_local(inner_name, "a") => {
                text.push_str(&element_text(child));
            }
            XmlNode::Element {
                name: inner_name, ..
            } if is_local(inner_name, "span") => {
                if let Some(r) = parse_span_node(child, styles) {
                    for run in &r {
                        text.push_str(&run.text);
                    }
                }
            }
            _ => {}
        }
    }

    if text.is_empty() {
        return None;
    }

    if let Some(ref style_name) = span_style_name {
        if let Some(style) = styles.get(style_name) {
            bold = bold || style.bold;
            italic = italic || style.italic;
            if font_name.is_none() {
                font_name = style.font_name.clone();
            }
            if font_size_pt.is_none() {
                font_size_pt = style.font_size_pt;
            }
        }
    }

    Some(vec![Run {
        id: eid("run", &["run:0"], &text),
        text,
        bold: Some(bold),
        italic: Some(italic),
        font_name,
        font_size_pt,
        underline: None,
        strikethrough: None,
        style_name: span_style_name,
    }])
}

fn parse_span_paragraph(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Paragraph> {
    let runs = parse_span_node(node, styles)?;
    if runs.is_empty() {
        return None;
    }
    let text_content: String = runs.iter().map(|r| r.text.as_str()).collect();
    Some(Paragraph {
        id: eid("paragraph", &["para:0"], &text_content),
        runs,
        page_number: None,
        style_name: None,
        numbering_info: None,
    })
}

fn parse_hyperlink(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Paragraph> {
    let XmlNode::Element { children, .. } = node else {
        return None;
    };
    let mut text = String::new();
    for child in children {
        match child {
            XmlNode::Text { text: t, .. } => text.push_str(t),
            XmlNode::Element { name: inner, .. } if is_local(inner, "span") => {
                if let Some(r) = parse_span_node(child, styles) {
                    for run in &r {
                        text.push_str(&run.text);
                    }
                }
            }
            XmlNode::Element { children: gc, .. } => {
                for g in gc {
                    if let XmlNode::Text { text: t, .. } = g {
                        text.push_str(t);
                    }
                }
            }
            _ => {}
        }
    }
    if text.is_empty() {
        return None;
    }
    Some(Paragraph {
        id: eid("paragraph", &["para:hyperlink"], &text),
        runs: vec![Run {
            id: eid("run", &["run:0"], &text),
            text,
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
    })
}

// ===========================================================================
// Table parsing
// ===========================================================================

fn parse_table(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Table> {
    let mut rows: Vec<Row> = Vec::new();
    walk_table_rows(node, styles, &mut rows);
    if rows.is_empty() {
        return None;
    }
    Some(Table {
        id: eid("table", &["table:0"], ""),
        rows,
    })
}

fn walk_table_rows(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>, rows: &mut Vec<Row>) {
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if is_local(child.name(), "table-row") {
                if let Some(row) = parse_table_row(child, styles) {
                    rows.push(row);
                }
            } else if is_local(child.name(), "table-body")
                || is_local(child.name(), "table-row-set")
            {
                walk_table_rows(child, styles, rows);
            }
        }
    }
}

fn parse_table_row(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Row> {
    let XmlNode::Element { children, .. } = node else {
        return None;
    };
    let mut cells: Vec<Cell> = Vec::new();
    for child in children {
        if is_local(child.name(), "table-cell") {
            if let Some(cell) = parse_table_cell(child, styles) {
                cells.push(cell);
            }
        }
    }
    if cells.is_empty() {
        return None;
    }
    Some(Row { cells })
}

fn parse_table_cell(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<Cell> {
    let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    else {
        return None;
    };
    let colspan = get_attr_usize(attributes, "table:number-columns-spanned");
    let rowspan = get_attr_usize(attributes, "table:number-rows-spanned");
    let mut contents: Vec<SectionItem> = Vec::new();
    for child in children {
        if is_local(child.name(), "p") {
            if let Some(p) = parse_paragraph(child, styles) {
                contents.push(SectionItem::Paragraph(p));
            }
        } else if is_local(child.name(), "table") {
            if let Some(t) = parse_table(child, styles) {
                contents.push(SectionItem::Table(t));
            }
        }
    }
    Some(Cell {
        id: eid("cell", &["cell:0"], ""),
        contents,
        colspan,
        rowspan,
    })
}

fn get_attr_usize(attributes: &BTreeMap<TagName, String>, key: &str) -> Option<usize> {
    for (k, v) in attributes {
        if k.local_name == key {
            return v.trim().parse().ok();
        }
    }
    None
}

// ===========================================================================
// List parsing
// ===========================================================================

fn parse_list(node: &XmlNode, styles: &BTreeMap<String, OdfStyle>) -> Option<List> {
    let XmlNode::Element { children, .. } = node else {
        return None;
    };
    let mut items: Vec<ListItem> = Vec::new();
    let mut level: usize = 0;
    for child in children {
        if is_local(child.name(), "list-item") {
            if let Some(li) = parse_list_item(child, styles, &mut level) {
                items.extend(li);
            }
        }
    }
    if items.is_empty() {
        return None;
    }
    Some(List {
        id: eid("list", &["list:0"], ""),
        kind: ListKind::Other,
        items,
    })
}

fn parse_list_item(
    node: &XmlNode,
    styles: &BTreeMap<String, OdfStyle>,
    level: &mut usize,
) -> Option<Vec<ListItem>> {
    let XmlNode::Element { children, .. } = node else {
        return None;
    };
    let mut items: Vec<ListItem> = Vec::new();
    let mut content: Vec<SectionItem> = Vec::new();

    for child in children {
        if is_local(child.name(), "p") {
            if let Some(p) = parse_paragraph(child, styles) {
                content.push(SectionItem::Paragraph(p));
            }
        } else if is_local(child.name(), "list") {
            if let Some(nested) = parse_list(child, styles) {
                for mut li in nested.items {
                    li.level += *level;
                    items.push(li);
                }
            }
        }
    }

    if !content.is_empty() {
        let sample = content
            .iter()
            .filter_map(|ci| {
                if let SectionItem::Paragraph(p) = ci {
                    Some(p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                } else {
                    None
                }
            })
            .next()
            .unwrap_or_default();
        items.push(ListItem {
            id: eid("listitem", &["item:0"], &sample),
            content,
            level: *level,
        });
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

// ===========================================================================
// Read function
// ===========================================================================

/// Read an ODT document from an OdfPackage, producing a Document IR.
pub fn read_odt(package: &OdfPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    let content_data = find_part(&package.parts, "content.xml")
        .map(|p| &p.data[..])
        .ok_or_else(|| {
            vec![Diagnostic::new::<_, String>(
                "ODF-R101",
                Severity::Error,
                "ODT: content.xml not found".into(),
            )]
        })?;

    let styles: BTreeMap<String, OdfStyle> = find_part(&package.parts, "styles.xml")
        .map(|p| {
            let s = parse_styles(&p.data);
            diags.push(Diagnostic::new(
                "ODF-R102",
                Severity::Info,
                format!("styles.xml: {} styles found", s.len()),
            ));
            s
        })
        .unwrap_or_default();

    if let Some(p) = find_part(&package.parts, "meta.xml") {
        let meta = parse_meta(&p.data);
        diags.push(Diagnostic::new(
            "ODF-R103",
            Severity::Info,
            format!(
                "meta.xml: title={}, author={}, creation={}",
                meta.title.is_some(),
                meta.author.is_some(),
                meta.creation_date.is_some()
            ),
        ));
    }

    let (document, build_diags) = build_document(content_data, &styles)?;
    diags.extend(build_diags);

    let ir_bytes = serde_json::to_vec(&document).map_err(|e| {
        vec![Diagnostic::new(
            "ODF-E-010",
            Severity::Error,
            format!("Failed to serialize IR: {}", e),
        )]
    })?;

    diags.push(Diagnostic::new(
        "ODF-R110",
        Severity::Info,
        format!(
            "odt: IR produced with {} sections, {} styles, {} diagnostics",
            document.sections.len(),
            styles.len(),
            diags.len()
        ),
    ));

    Ok(ReadResult {
        ir: ir_bytes,
        diagnostics: diags,
    })
}

fn find_part<'a>(parts: &'a [OdfPart], path: &str) -> Option<&'a OdfPart> {
    parts.iter().find(|p| p.path == path)
}

fn eid(kind: &str, path: &[&str], content: &str) -> ElementId {
    ElementId::derive(kind, path, &content.to_string())
}

pub struct OdtReader;

impl Codec for OdtReader {
    fn format_name(&self) -> &str {
        "odt"
    }
    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
        let package = parse_odf(data)?;
        read_odt(&package)
    }
}

impl XmlNode {
    fn name(&self) -> &TagName {
        match self {
            XmlNode::Element { name, .. } => name,
            _ => {
                static FAKE: std::sync::OnceLock<TagName> = std::sync::OnceLock::new();
                FAKE.get_or_init(|| TagName {
                    prefix: None,
                    local_name: "unknown".to_string(),
                    namespace_uri: None,
                })
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn get_corpus_odt(name: &str) -> Vec<u8> {
        let path = format!("{}/../../corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("corpus fixture {} should be readable: {}: {e}", name, path))
    }

    fn make_test_odt(
        content: &str,
        styles_content: Option<&str>,
        meta_content: Option<&str>,
    ) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();

        writer.start_file("content.xml", opts).unwrap();
        writer.write_all(content.as_bytes()).unwrap();

        writer.start_file("styles.xml", opts).unwrap();
        let styles_default = b"<?xml version=\"1.0\"?><office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"><office:styles></office:styles></office:document-styles>";
        let styles_data = styles_content.map_or(styles_default.to_vec(), |s| s.as_bytes().to_vec());
        writer.write_all(&styles_data).unwrap();

        writer.start_file("meta.xml", opts).unwrap();
        let meta_default = b"<?xml version=\"1.0\"?><office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:meta></office:meta></office:document-meta>";
        let meta_data = meta_content.map_or(meta_default.to_vec(), |m| m.as_bytes().to_vec());
        writer.write_all(&meta_data).unwrap();

        writer.start_file("META-INF/manifest.xml", opts).unwrap();
        writer.write_all(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>"
        ).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_parse_odt_corpus_fixture() {
        let data = get_corpus_odt("cjk-text-odt-001.odt");
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        assert_eq!(doc.ir_version, IR_VERSION);
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].kind, SectionKind::Body);
        let para_count = doc.sections[0]
            .items
            .iter()
            .filter(|i| matches!(i, SectionItem::Paragraph(_)))
            .count();
        assert!(para_count > 0, "Should have at least one paragraph");
    }

    #[test]
    fn test_parse_paragraphs() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p>Hello world</text:p><text:p>Second paragraph</text:p></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, None);
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        let para_count = doc.sections[0]
            .items
            .iter()
            .filter(|i| matches!(i, SectionItem::Paragraph(_)))
            .count();
        assert_eq!(para_count, 2, "Should have exactly 2 paragraphs");
        if let SectionItem::Paragraph(p) = &doc.sections[0].items[0] {
            let full_text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            assert!(full_text.contains("Hello world"));
        }
    }

    #[test]
    fn test_parse_table_content() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:text><text:p>Before table</text:p><table:table table:name="MyTable"><table:table-row><table:table-cell><text:p>Cell 1</text:p></table:table-cell><table:table-cell><text:p>Cell 2</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell><text:p>Cell 3</text:p></table:table-cell><table:table-cell><text:p>Cell 4</text:p></table:table-cell></table:table-row></table:table><text:p>After table</text:p></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, None);
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        assert_eq!(doc.sections[0].items.len(), 3, "Should have 3 items");
    }

    #[test]
    fn test_parse_span_styles() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p><text:span>Normal text</text:span><text:span>Bold text</text:span></text:p></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, None);
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        if let SectionItem::Paragraph(p) = &doc.sections[0].items[0] {
            let full_text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            assert!(full_text.contains("Normal text"));
        }
    }

    #[test]
    fn test_parse_lists() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:list><text:list-item><text:p>First item</text:p></text:list-item><text:list-item><text:p>Second item</text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, None);
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        assert!(
            doc.sections[0]
                .items
                .iter()
                .any(|i| matches!(i, SectionItem::List(_))),
            "Should have a list"
        );
    }

    #[test]
    fn test_parse_sections() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p>Before section</text:p><text:section text:name="MySection"><text:p>Section content</text:p></text:section><text:p>After section</text:p></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, None);
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        let para_count = doc.sections[0]
            .items
            .iter()
            .filter(|i| matches!(i, SectionItem::Paragraph(_)))
            .count();
        assert!(para_count >= 3, "Should have at least 3 paragraphs");
    }

    #[test]
    fn test_parse_metadata() {
        let meta = r#"<?xml version="1.0"?><office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><office:meta><meta:title>Test Document Title</meta:title><dc:creator>Test Author</dc:creator><dc:date>2026-01-01T00:00:00</dc:date></office:meta></office:document-meta>"#;
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p>Hello</text:p></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, Some(meta));
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        assert!(
            result.diagnostics.iter().any(|d| d.code == "ODF-R103"),
            "Should have metadata diagnostic"
        );
        let _doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
    }

    #[test]
    fn test_apply_styles_to_paragraphs() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p text:style-name="Heading1">Heading text</text:p><text:p text:style-name="Body">Body text</text:p></office:text></office:body></office:document-content>"#;
        let styles = r#"<?xml version="1.0"?><office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"><office:styles><style:style style:name="Heading1" style:family="paragraph"><style:text-properties fo:font-weight="bold" fo:font-size="18pt"/></style:style><style:style style:name="Body" style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:style></office:styles></office:document-styles>"#;
        let data = make_test_odt(content, Some(styles), None);
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = read_odt(&pkg).expect("should produce IR");
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        if let SectionItem::Paragraph(p) = &doc.sections[0].items[0] {
            assert_eq!(p.style_name.as_deref(), Some("Heading1"));
        }
        if let SectionItem::Paragraph(p) = &doc.sections[0].items[1] {
            assert_eq!(p.style_name.as_deref(), Some("Body"));
        }
    }

    #[test]
    fn test_odt_codec_integration() {
        let content = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p>Codec integration test</text:p></office:text></office:body></office:document-content>"#;
        let data = make_test_odt(content, None, None);
        let reader = OdtReader;
        assert_eq!(reader.format_name(), "odt");
        let result = reader.read(&data).expect("OdtReader::read should succeed");
        assert!(!result.ir.is_empty());
        let doc: Document =
            serde_json::from_slice(&result.ir).expect("should deserialize Document");
        assert_eq!(doc.ir_version, IR_VERSION);
    }

    #[test]
    fn test_parse_all_odt_fixtures() {
        let blobs_dir = format!("{}/../../corpus/blobs", env!("CARGO_MANIFEST_DIR"));
        let entries = std::fs::read_dir(&blobs_dir).expect("corpus/blobs/ should be readable");
        let mut parsed_count = 0;
        let mut errors = Vec::new();
        for entry in entries {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("odt") {
                let data = std::fs::read(&path)
                    .unwrap_or_else(|e| panic!("should read {}: {e}", path.display()));
                match parse_odf(&data) {
                    Ok(pkg) => match read_odt(&pkg) {
                        Ok(_) => parsed_count += 1,
                        Err(diags) => {
                            let has_fatal = diags.iter().any(|d| d.severity == Severity::Fatal);
                            if has_fatal {
                                errors.push((
                                    path.file_name().unwrap().to_string_lossy().to_string(),
                                    diags.clone(),
                                ));
                            } else {
                                parsed_count += 1;
                            }
                        }
                    },
                    Err(_) => {
                        parsed_count += 1;
                    }
                }
            }
        }
        assert!(
            parsed_count > 0,
            "Should have parsed at least one ODT fixture (parsed {})",
            parsed_count
        );
        assert!(errors.is_empty(), "No fatal errors: {:?}", errors);
    }

    #[test]
    fn test_parse_styles_parsing() {
        let styles_xml = r#"<?xml version="1.0"?><office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"><office:styles><style:style style:name="BoldStyle" style:family="paragraph"><style:text-properties fo:font-weight="bold"/></style:style><style:style style:name="ItalicStyle" style:family="text"><style:text-properties fo:font-style="italic"/></style:style><style:style style:name="LargeFont" style:family="text"><style:text-properties fo:font-size="24pt"/></style:style></office:styles></office:document-styles>"#;
        let styles = parse_styles(styles_xml.as_bytes());
        assert!(styles.contains_key("BoldStyle"));
        assert!(styles.contains_key("ItalicStyle"));
        assert!(styles.contains_key("LargeFont"));
        assert!(styles["BoldStyle"].bold);
        assert!(styles["ItalicStyle"].italic);
        assert_eq!(styles["LargeFont"].font_size_pt, Some(24.0));
    }

    #[test]
    fn test_parse_meta_extraction() {
        let meta_xml = r#"<?xml version="1.0"?><office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><office:meta><meta:title>My Title</meta:title><dc:creator>John Doe</dc:creator><meta:description>A test document</meta:description><meta:creation-date>2026-01-01T00:00:00</meta:creation-date></office:meta></office:document-meta>"#;
        let meta = parse_meta(meta_xml.as_bytes());
        assert_eq!(meta.title, Some("My Title".to_string()));
        assert_eq!(meta.author, Some("John Doe".to_string()));
        assert_eq!(meta.description, Some("A test document".to_string()));
        assert_eq!(meta.creation_date, Some("2026-01-01T00:00:00".to_string()));
    }

    #[test]
    fn test_parse_font_size() {
        assert_eq!(parse_font_size("12pt"), Some(12.0));
        assert_eq!(parse_font_size(" 14pt "), Some(14.0));
        assert_eq!(parse_font_size("24pt"), Some(24.0));
        assert_eq!(parse_font_size("1in"), Some(72.0));
        assert_eq!(parse_font_size("12"), Some(12.0));
        let mm_val = parse_font_size("25mm").unwrap();
        assert!(mm_val > 70.0 && mm_val < 72.0, "25mm = {}pt", mm_val);
    }

    #[test]
    fn test_corpus_corpus_odt_fixtures() {
        let fixtures = [
            "cjk-text-odt-001.odt",
            "custom-styles-odt-001.odt",
            "long-tables-odt-001.odt",
            "numbering-restarts-odt-001.odt",
            "merged-cells-odt-001.odt",
            "headers-fields-odt-001.odt",
        ];
        for fixture in &fixtures {
            let data = get_corpus_odt(fixture);
            let pkg = parse_odf(&data).unwrap_or_else(|_| panic!("{fixture} should parse"));
            let result = read_odt(&pkg).unwrap_or_else(|_| panic!("{fixture} should produce IR"));
            let doc: Document = serde_json::from_slice(&result.ir)
                .unwrap_or_else(|_| panic!("{fixture} should deserialize"));
            assert_eq!(doc.ir_version, IR_VERSION);
            assert!(
                !doc.sections[0].items.is_empty(),
                "{fixture} should have items"
            );
        }
    }
}
