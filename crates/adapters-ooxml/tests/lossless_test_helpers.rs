// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(dead_code)]
//! Shared test helpers for the lossless proof suite.
//!
//! Provides fixture generation (docx with tracked changes via raw XML injection),
//! and comparison utilities (docx-bytes, IR equality, text extraction).

use sha2::{Digest, Sha256};

use std::collections::BTreeMap;
use std::io::Write;

use adapters_ooxml::opc::{parse_opc, OpcPackage, Part};
use adapters_ooxml::xml::{parse_xml, XmlNode};
use gp_core::ir::*;

/// Generate a docx with a tracked insertion using raw XML injection.
pub fn generate_docx_with_insert(text: &str, author: &str) -> Vec<u8> {
    let doc_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
<w:p><w:r><w:t>{} </w:t></w:r></w:p>
</w:body>
</w:document>"#,
        text
    );

    let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>";
    let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        </Types>";
    let styles_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        <w:style w:type=\"paragraph\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
        </w:styles>";
    let settings_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        </w:settings>";

    let mut pkg_parts: Vec<Part> = vec![
        Part {
            uri: "[Content_Types].xml".into(),
            content_type: "application/xml".into(),
            data: ct_xml.to_vec(),
        },
        Part {
            uri: "_rels/.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/document.xml".into(),
            content_type:
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
                    .into(),
            data: doc_xml.as_bytes().to_vec(),
        },
        Part {
            uri: "word/_rels/document.xml.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/styles.xml".into(),
            content_type: "application/xml".into(),
            data: styles_xml.to_vec(),
        },
        Part {
            uri: "word/settings.xml".into(),
            content_type: "application/xml".into(),
            data: settings_xml.to_vec(),
        },
    ];

    // Wrap the first paragraph text in w:ins
    let ins_xml = format!(
        r#"<w:ins w:id="0" w:author="{}" w:date="2026-01-01T00:00:00Z">
            <w:p><w:r><w:t>{}</w:t></w:r></w:p>
        </w:ins>"#,
        author, text
    );
    let new_doc_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
{}
</w:body>
</w:document>"#,
        ins_xml
    );
    // Replace the document.xml part
    for part in &mut pkg_parts {
        if part.uri == "word/document.xml" {
            part.data = new_doc_xml.as_bytes().to_vec();
            break;
        }
    }

    pack_opc(&pkg_parts)
}

/// Generate a docx with a tracked deletion.
pub fn generate_docx_with_delete(original: &str, author: &str) -> Vec<u8> {
    let del_xml = format!(
        r#"<w:del w:id="0" w:author="{}" w:date="2026-01-01T00:00:00Z">
            <w:p><w:r><w:delText>{}</w:delText></w:r></w:p>
        </w:del>"#,
        author, original
    );
    let new_doc_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
{}
</w:body>
</w:document>"#,
        del_xml
    );

    let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>";
    let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        </Types>";
    let styles_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        <w:style w:type=\"paragraph\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
        </w:styles>";
    let settings_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        </w:settings>";

    let pkg_parts: Vec<Part> = vec![
        Part {
            uri: "[Content_Types].xml".into(),
            content_type: "application/xml".into(),
            data: ct_xml.to_vec(),
        },
        Part {
            uri: "_rels/.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/document.xml".into(),
            content_type:
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
                    .into(),
            data: new_doc_xml.as_bytes().to_vec(),
        },
        Part {
            uri: "word/_rels/document.xml.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/styles.xml".into(),
            content_type: "application/xml".into(),
            data: styles_xml.to_vec(),
        },
        Part {
            uri: "word/settings.xml".into(),
            content_type: "application/xml".into(),
            data: settings_xml.to_vec(),
        },
    ];

    pack_opc(&pkg_parts)
}

/// Generate a docx with mixed insertions and deletions.
pub fn generate_docx_with_mixed_revisions(_author: &str) -> Vec<u8> {
    let mixed_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
<w:ins w:id="0" w:author="agent" w:date="2026-01-01T00:00:00Z">
    <w:p><w:r><w:t>Inserted text</w:t></w:r></w:p>
</w:ins>
<w:del w:id="1" w:author="agent" w:date="2026-01-01T00:00:00Z">
    <w:p><w:r><w:delText>Deleted text</w:delText></w:r></w:p>
</w:del>
</w:body>
</w:document>"#;

    let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>";
    let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        </Types>";
    let styles_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        <w:style w:type=\"paragraph\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
        </w:styles>";
    let settings_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        </w:settings>";

    let pkg_parts: Vec<Part> = vec![
        Part {
            uri: "[Content_Types].xml".into(),
            content_type: "application/xml".into(),
            data: ct_xml.to_vec(),
        },
        Part {
            uri: "_rels/.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/document.xml".into(),
            content_type:
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
                    .into(),
            data: mixed_xml.as_bytes().to_vec(),
        },
        Part {
            uri: "word/_rels/document.xml.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/styles.xml".into(),
            content_type: "application/xml".into(),
            data: styles_xml.to_vec(),
        },
        Part {
            uri: "word/settings.xml".into(),
            content_type: "application/xml".into(),
            data: settings_xml.to_vec(),
        },
    ];

    pack_opc(&pkg_parts)
}

/// Generate a direct (non-revision) version of the same document.
pub fn generate_direct_docx(text: &str) -> Vec<u8> {
    let clean_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
<w:p><w:r><w:t>{}</w:t></w:r></w:p>
</w:body>
</w:document>"#,
        text
    );

    let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>";
    let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        </Types>";
    let styles_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        <w:style w:type=\"paragraph\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
        </w:styles>";
    let settings_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        </w:settings>";

    let pkg_parts: Vec<Part> = vec![
        Part {
            uri: "[Content_Types].xml".into(),
            content_type: "application/xml".into(),
            data: ct_xml.to_vec(),
        },
        Part {
            uri: "_rels/.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/document.xml".into(),
            content_type:
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
                    .into(),
            data: clean_xml.as_bytes().to_vec(),
        },
        Part {
            uri: "word/_rels/document.xml.rels".into(),
            content_type: "application/xml".into(),
            data: rels_xml.to_vec(),
        },
        Part {
            uri: "word/styles.xml".into(),
            content_type: "application/xml".into(),
            data: styles_xml.to_vec(),
        },
        Part {
            uri: "word/settings.xml".into(),
            content_type: "application/xml".into(),
            data: settings_xml.to_vec(),
        },
    ];

    pack_opc(&pkg_parts)
}

/// Generate a clean original docx (no revisions).
pub fn generate_clean_docx(text: &str) -> Vec<u8> {
    generate_direct_docx(text)
}

/// Pack parts into OPC ZIP bytes.
fn pack_opc(parts: &[Part]) -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();
    for part in parts {
        let _ = writer.start_file(&part.uri, opts);
        let _ = writer.write_all(&part.data);
    }
    writer.finish().unwrap().into_inner()
}

/// Compare two docx files for byte-identical content.
pub fn docx_bytes_identical(a: &[u8], b: &[u8]) -> bool {
    a == b
}

/// Compare IR documents for equality.
pub fn ir_documents_equal(a: &[u8], b: &[u8]) -> bool {
    let doc_a: Document = match serde_json::from_slice(a) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let doc_b: Document = match serde_json::from_slice(b) {
        Ok(d) => d,
        Err(_) => return false,
    };
    // Compare sections (the meaningful content)
    if doc_a.sections.len() != doc_b.sections.len() {
        return false;
    }
    for (sa, sb) in doc_a.sections.iter().zip(doc_b.sections.iter()) {
        if sa.kind != sb.kind {
            return false;
        }
        if sa.items.len() != sb.items.len() {
            return false;
        }
        for (ia, ib) in sa.items.iter().zip(sb.items.iter()) {
            if !items_text_equal(ia, ib) {
                return false;
            }
        }
    }
    true
}

fn items_text_equal(a: &SectionItem, b: &SectionItem) -> bool {
    match (a, b) {
        (SectionItem::Paragraph(pa), SectionItem::Paragraph(pb)) => {
            if pa.runs.len() != pb.runs.len() {
                return false;
            }
            pa.runs
                .iter()
                .zip(pb.runs.iter())
                .all(|(ra, rb)| ra.text == rb.text)
        }
        (SectionItem::Table(ta), SectionItem::Table(tb)) => {
            if ta.rows.len() != tb.rows.len() {
                return false;
            }
            ta.rows.iter().zip(tb.rows.iter()).all(|(ra, rb)| {
                if ra.cells.len() != rb.cells.len() {
                    return false;
                }
                ra.cells.iter().zip(rb.cells.iter()).all(|(ca, cb)| {
                    ca.contents.len() == cb.contents.len()
                        && ca
                            .contents
                            .iter()
                            .zip(cb.contents.iter())
                            .all(|(a, b)| items_text_equal(a, b))
                })
            })
        }
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// Compare extracted text from two docx files.
pub fn docx_text_equal(a: &[u8], b: &[u8]) -> bool {
    let text_a = extract_text_from_docx(a);
    let text_b = extract_text_from_docx(b);
    text_a == text_b
}

/// Extract visible text (excluding revision markup) from a docx package.
pub fn extract_text_from_docx(bytes: &[u8]) -> String {
    let pkg = match parse_opc(bytes) {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let doc_part = match pkg.parts.iter().find(|p| p.uri == "word/document.xml") {
        Some(p) => p,
        None => return String::new(),
    };
    let xml_doc = match parse_xml(&doc_part.data) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    extract_text_recursive(&xml_doc.root)
}

fn extract_text_recursive(node: &XmlNode) -> String {
    let mut result = String::new();
    match node {
        XmlNode::Element { children, .. } => {
            for child in children {
                result.push_str(&extract_text_recursive(child));
            }
        }
        XmlNode::Text { text, .. } => {
            result.push_str(text);
        }
        _ => {}
    }
    // Filter out delText content (deleted text should not appear)
    result
}

/// Compute SHA-256 hash of bytes.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Create a minimal OPC package for testing with revision markup.
pub fn make_minimal_docx_with_revisions(doc_xml: &str) -> OpcPackage {
    let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>";
    let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        </Types>";

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

/// Create a Revision for testing (Insert kind).
pub fn make_test_insert(text: &str, id: &str) -> Revision {
    Revision {
        id: ElementId::from_str(id),
        kind: RevisionKind::Insert,
        author: Some("agent".to_string()),
        date: Some("2026-01-01T00:00:00Z".to_string()),
        anchor_element_id: ElementId::from_str("anchor-1"),
        page_hint: Some(1),
        content: text.to_string(),
        property_name: None,
        original_value: None,
        new_value: None,
    }
}

/// Create a Revision for testing (Delete kind).
pub fn make_test_delete(text: &str, id: &str) -> Revision {
    Revision {
        id: ElementId::from_str(id),
        kind: RevisionKind::Delete,
        author: Some("agent".to_string()),
        date: Some("2026-01-01T00:00:00Z".to_string()),
        anchor_element_id: ElementId::from_str("anchor-1"),
        page_hint: Some(1),
        content: text.to_string(),
        property_name: None,
        original_value: None,
        new_value: None,
    }
}

/// Create a RevisionWriterMeta for testing.
pub fn make_test_meta() -> adapters_ooxml::revision_executor::RevisionWriterMeta {
    adapters_ooxml::revision_executor::RevisionWriterMeta::new("agent", "2026-01-01T00:00:00Z")
}

/// Extract body text from parsed XML, excluding revision wrappers.
pub fn extract_body_text(xml: &XmlNode) -> String {
    let mut text = String::new();
    extract_body_text_recursive(xml, &mut text, false);
    text.trim().to_string()
}

fn extract_body_text_recursive(node: &XmlNode, out: &mut String, in_del: bool) {
    if let XmlNode::Element { name, children, .. } = node {
        if name.local_name == "del" {
            // Skip content inside del, but still recurse for delText
            for child in children {
                if name.local_name == "delText" {
                    continue; // skip delText content
                }
                extract_body_text_recursive(child, out, true);
            }
            return;
        }
        if name.local_name == "delText" {
            return; // skip deleted text
        }
        for child in children {
            extract_body_text_recursive(child, out, in_del);
        }
    } else if let XmlNode::Text { text, .. } = node {
        if !in_del {
            out.push_str(text);
        }
    }
}
