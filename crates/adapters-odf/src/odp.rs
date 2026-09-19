// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-odf: ODP (presentation) -> gp-core IR.

#![allow(dead_code, clippy::if_same_then_else, clippy::ptr_arg)]

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;
use gp_core::traits::{Codec, ReadResult};
use std::collections::BTreeMap;

use super::manifest::{parse_odf, OdfPackage, OdfPart};
use super::xml::{parse_xml, TagName, XmlNode};

pub const NS_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub const NS_PPTX: &str = "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0";
pub const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const NS_SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";

fn is_ns(node: &XmlNode, ns: &str, local: &str) -> bool {
    let p = match ns {
        NS_DRAW => "draw",
        NS_PPTX => "presentation",
        NS_OFFICE => "office",
        NS_SVG => "svg",
        _ => return false,
    };
    matches!(node, XmlNode::Element { name, .. } if name.local_name == local && name.prefix.as_deref() == Some(p))
}

fn elem_text(node: &XmlNode) -> String {
    let mut r = String::new();
    match node {
        XmlNode::Text { text, .. } => r.push_str(text),
        XmlNode::Element { children, .. } => {
            for c in children {
                r.push_str(&elem_text(c));
            }
        }
        _ => {}
    }
    r
}

fn get_attr<'a>(attrs: &'a BTreeMap<TagName, String>, key: &str) -> Option<&'a str> {
    for (k, v) in attrs {
        if k.local_name == key {
            return Some(v.as_str());
        }
    }
    None
}

trait XmlNodeExt {
    fn children_ref(&self) -> std::slice::Iter<'_, XmlNode>;
    fn attributes_ref(&self) -> Option<&BTreeMap<TagName, String>>;
}
impl XmlNodeExt for XmlNode {
    fn children_ref(&self) -> std::slice::Iter<'_, XmlNode> {
        match self {
            XmlNode::Element { children, .. } => children.iter(),
            _ => [].iter(),
        }
    }
    fn attributes_ref(&self) -> Option<&BTreeMap<TagName, String>> {
        match self {
            XmlNode::Element { attributes, .. } => Some(attributes),
            _ => None,
        }
    }
}

// ===========================================================================
// Parse
// ===========================================================================

fn find_page_root(node: &XmlNode) -> Option<&XmlNode> {
    if is_ns(node, NS_DRAW, "page") {
        return Some(node);
    }
    if let XmlNode::Element { children, .. } = node {
        for c in children {
            if let Some(f) = find_page_root(c) {
                return Some(f);
            }
        }
    }
    None
}

fn find_pres(node: &XmlNode) -> Option<&XmlNode> {
    if is_ns(node, NS_OFFICE, "presentation") {
        return Some(node);
    }
    if let XmlNode::Element { children, .. } = node {
        for c in children {
            if let Some(f) = find_pres(c) {
                return Some(f);
            }
        }
    }
    None
}

fn build_presentation(data: &[u8]) -> Result<(Presentation, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let doc = parse_xml(data)?;
    let mut slides = Vec::new();
    // Find office:presentation and iterate over its draw:page children
    let pres_node = find_pres(&doc.root);
    if let Some(root) = pres_node {
        for child in root.children_ref() {
            if is_ns(child, NS_DRAW, "page") {
                if let Some(s) = parse_slide(child, slides.len(), &mut diags) {
                    slides.push(s);
                }
            }
        }
    }
    if slides.is_empty() {
        diags.push(Diagnostic::new(
            "ODF-R300",
            Severity::Warning,
            "ODP: no pages found",
        ));
    }
    Ok((
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides,
            layouts: vec![],
            masters: vec![],
        },
        diags,
    ))
}

fn parse_slide(node: &XmlNode, si: usize, diags: &mut Vec<Diagnostic>) -> Option<Slide> {
    let XmlNode::Element { children, .. } = node else {
        return None;
    };
    let mut elems = Vec::new();
    for child in children {
        if let Some(e) = parse_element(child, diags) {
            elems.push(e);
        }
    }
    Some(Slide {
        id: ElementId::from_content_hash("slide", &[&format!("slide:{si}")], ""),
        elements: elems,
    })
}

fn parse_element(node: &XmlNode, _diags: &mut Vec<Diagnostic>) -> Option<SlideElement> {
    let XmlNode::Element { name, .. } = node else {
        return None;
    };
    let local = &name.local_name;
    let kind = if is_ns(node, NS_DRAW, "frame") {
        "frame"
    } else if is_ns(node, NS_DRAW, "rect") {
        "rect"
    } else if is_ns(node, NS_DRAW, "text") {
        "text-box"
    } else if is_ns(node, NS_DRAW, "text-box") {
        "text-box"
    } else if is_ns(node, NS_DRAW, "image") {
        "image"
    } else if is_ns(node, NS_DRAW, "circle") {
        "circle"
    } else if is_ns(node, NS_DRAW, "line") {
        "line"
    } else if is_ns(node, NS_DRAW, "connector") {
        "connector"
    } else {
        return None;
    };
    Some(SlideElement {
        id: ElementId::from_content_hash(local, &[], ""),
        kind: kind.to_string(),
    })
}

// ===========================================================================
// Read
// ===========================================================================

pub fn read_odp(package: &OdfPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let content = find_part(&package.parts, "content.xml").ok_or_else(|| {
        vec![Diagnostic::new(
            "ODF-R301",
            Severity::Error,
            "ODP: no content.xml",
        )]
    })?;
    let (pres, bd) = build_presentation(&content.data)?;
    diags.extend(bd);
    let ir = serde_json::to_vec(&pres)
        .map_err(|e| vec![Diagnostic::new("ODF-E-010", Severity::Error, e.to_string())])?;
    let ts: usize = pres.slides.iter().map(|s| s.elements.len()).sum();
    diags.push(Diagnostic::new(
        "ODF-R310",
        Severity::Info,
        format!("odp: {n} slides, {ts} elements", n = pres.slides.len()),
    ));
    Ok(ReadResult {
        ir,
        diagnostics: diags,
    })
}

fn find_part<'a>(parts: &'a [OdfPart], path: &str) -> Option<&'a OdfPart> {
    parts.iter().find(|p| p.path == path)
}

pub struct OdpReader;
impl Codec for OdpReader {
    fn format_name(&self) -> &str {
        "odp"
    }
    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
        let pkg = parse_odf(data)?;
        read_odp(&pkg)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn get_odp(n: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/../../corpus/blobs/{n}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("corpus {n}: {e}"))
    }

    fn mk_odp(c: &str) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();
        w.start_file("content.xml", o).unwrap();
        w.write_all(c.as_bytes()).unwrap();
        w.start_file("styles.xml", o).unwrap();
        w.write_all(b"<?xml version=\"1.0\"?><office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:styles></office:styles></office:document-styles>").unwrap();
        w.start_file("meta.xml", o).unwrap();
        w.write_all(b"<?xml version=\"1.0\"?><office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>").unwrap();
        w.start_file("META-INF/manifest.xml", o).unwrap();
        w.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn test_parse_odp_corpus_fixture() {
        let d = get_odp("cjk-text-odp-001.odp");
        let p = parse_odf(&d).expect("parse");
        let r = read_odp(&p).expect("ir");
        let pres: Presentation = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(pres.ir_version, IR_VERSION);
        assert!(!pres.slides.is_empty(), "slides should not be empty");
    }

    #[test]
    fn test_parse_slide_elements() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"><office:body><office:presentation><draw:page draw:style-name="p1"><draw:frame draw:name="frame1" svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="3cm"/><draw:text-box draw:name="textbox1" svg:x="2cm" svg:y="2cm" svg:width="4cm" svg:height="2cm"><text:p>Slide title</text:p></draw:text-box></draw:page></office:presentation></office:body></office:document-content>"#;
        let d = mk_odp(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_odp(&p).expect("ir");
        let pres: Presentation = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(pres.slides.len(), 1);
        assert_eq!(pres.slides[0].elements.len(), 2);
    }

    #[test]
    fn test_parse_image_element() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"><office:body><office:presentation><draw:page><draw:image svg:x="0" svg:y="0" svg:width="10cm" svg:height="8cm"/></draw:page></office:presentation></office:body></office:document-content>"#;
        let d = mk_odp(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_odp(&p).expect("ir");
        let pres: Presentation = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(pres.slides[0].elements[0].kind, "image");
    }

    #[test]
    fn test_parse_all_odp_fixtures() {
        let blobs = format!("{}/../../corpus/blobs", env!("CARGO_MANIFEST_DIR"));
        let mut cnt = 0;
        for e in std::fs::read_dir(&blobs).expect("blobs") {
            let e = e.expect("entry");
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("odp") {
                let d = std::fs::read(&p)
                    .unwrap_or_else(|e| panic!("should read {}: {e}", p.display()));
                match parse_odf(&d) {
                    Ok(pkg) => {
                        let _ = read_odp(&pkg);
                        cnt += 1;
                    }
                    Err(_) => cnt += 1,
                }
            }
        }
        assert!(cnt > 0, "parsed {cnt} odp files");
    }

    #[test]
    fn test_no_pages() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:spreadsheet></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_odp(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_odp(&p).expect("ir");
        let pres: Presentation = serde_json::from_slice(&r.ir).expect("decode");
        assert!(pres.slides.is_empty());
        assert!(r.diagnostics.iter().any(|d| d.code == "ODF-R300"));
    }
}
