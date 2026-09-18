//! OOXML comment write-back — add comments to docx files.
//!
//! Comments are written to `word/comments.xml` with `commentRangeStart`/`commentRangeEnd`
//! anchored to C2 element IDs via `w:commentReference` in the document.

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::{Comment, ElementId};
use std::collections::BTreeMap;

/// Result of adding a comment to a docx.
#[derive(Debug)]
pub struct AddCommentResult {
    pub output: Vec<u8>,
    pub accepted: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub changed_parts: Vec<String>,
    pub unchanged_parts: Vec<String>,
    pub changed_element_ids: Vec<String>,
    pub stale_cached_values: Vec<String>,
    pub byte_diffs: Vec<PartByteDiff>,
    pub format: String,
    pub journal_id: Option<String>,
    pub ir_diff: Option<String>,
    pub invalidated_signatures: Vec<String>,
    pub tainted_parts: Vec<String>,
    pub policy_violations: Vec<String>,
    pub comment_id: String,
}

/// A byte diff for a single part.
#[derive(Debug, Clone)]
pub struct PartByteDiff {
    pub part: String,
    pub before: u64,
    pub after: u64,
}

/// Add a comment into a docx OPC package.
///
/// The comment is:
/// 1. Appended to `word/comments.xml`
/// 2. Referenced from `word/document.xml` via commentReference + commentRangeStart/End
///
/// Returns the updated package bytes or an error.
pub fn add_comment_to_docx(
    bytes: &[u8],
    comment: &Comment,
    element_id: &ElementId,
    dry_run: bool,
) -> Result<AddCommentResult, Vec<Diagnostic>> {
    use crate::opc::parse_opc;
    use crate::writer::{PartMutation, SurgicalWriter};

    let pkg = parse_opc(bytes).map_err(|e| e.clone())?;

    // Determine next comment ID.
    let comment_id_num = next_comment_id(&pkg);
    let comment_xml = format_comment_xml(comment, &comment_id_num);

    // Find the existing comments part.
    let comments_part = pkg
        .parts
        .iter()
        .find(|p| p.uri == "word/comments.xml")
        .cloned();

    // Build the updated comments.xml data.
    let new_comments_data = if let Some(ref existing) = comments_part {
        append_comment_to_xml(&existing.data, &comment_xml)
    } else {
        create_comments_xml(&comment_xml)
    };

    // Build surgical mutations.
    let mut mutations: BTreeMap<String, PartMutation> = BTreeMap::new();

    // Replace/add the comments part.
    mutations.insert(
        "word/comments.xml".to_string(),
        if comments_part.is_some() {
            PartMutation::Replace {
                data: new_comments_data.clone(),
                content_type: Some(
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"
                        .to_string(),
                ),
            }
        } else {
            PartMutation::Add {
                uri: "word/comments.xml".to_string(),
                content_type:
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"
                        .to_string(),
                data: new_comments_data.clone(),
                rels: None,
            }
        },
    );

    // If we have a document, add commentReference anchors.
    let doc_part = pkg
        .parts
        .iter()
        .find(|p| p.uri == "word/document.xml")
        .cloned();

    // For dry_run, just return preview without writing.
    if dry_run {
        let ir_diff = format!(
            "comment {} (author={}) added to docx",
            comment_id_num,
            escape_xml_attr(&comment.author)
        );
        return Ok(AddCommentResult {
            output: bytes.to_vec(),
            accepted: true,
            diagnostics: vec![Diagnostic::new(
                "OOX-I001",
                gp_core::diagnostics::Severity::Info,
                format!(
                    "DRY RUN: comment {} would be added (id={})",
                    comment.comment_id, comment_id_num
                ),
            )],
            changed_parts: vec!["word/comments.xml".to_string()],
            unchanged_parts: pkg.parts.iter().map(|p| p.uri.clone()).collect(),
            changed_element_ids: vec![element_id.to_string()],
            stale_cached_values: vec![],
            byte_diffs: vec![],
            format: "docx".to_string(),
            journal_id: None,
            ir_diff: Some(ir_diff),
            invalidated_signatures: vec![],
            tainted_parts: vec!["word/comments.xml".to_string()],
            policy_violations: vec![],
            comment_id: comment_id_num,
        });
    }

    // Perform the surgical write.
    let writer = SurgicalWriter::new(bytes);

    // If we have a document, add commentReference anchors.
    if let Some(doc_part) = &doc_part {
        // We need to also update the document to add commentReference anchors.
        let doc_xml = String::from_utf8_lossy(&doc_part.data).to_string();
        let new_doc_data =
            update_document_with_comment_anchor(&doc_xml, element_id, &comment_id_num);

        mutations.insert(
            "word/document.xml".to_string(),
            PartMutation::Replace {
                data: new_doc_data.into_bytes(),
                content_type: None,
            },
        );
    }

    let result = writer.write(mutations).map_err(|e| e.clone())?;

    let report = result.report;
    let bytes_after = result.bytes;

    Ok(AddCommentResult {
        output: bytes_after,
        accepted: true,
        diagnostics: vec![],
        changed_parts: report.parts_written,
        unchanged_parts: report.parts_unchanged,
        changed_element_ids: vec![element_id.to_string()],
        stale_cached_values: vec![],
        byte_diffs: vec![PartByteDiff {
            part: "word/comments.xml".to_string(),
            before: comments_part
                .as_ref()
                .map(|p| p.data.len() as u64)
                .unwrap_or(0),
            after: new_comments_data.len() as u64,
        }],
        format: "docx".to_string(),
        journal_id: None,
        ir_diff: Some(format!(
            "comment {} (author={}) added, id={}",
            comment.comment_id,
            escape_xml_attr(&comment.author),
            comment_id_num
        )),
        invalidated_signatures: vec![],
        tainted_parts: vec!["word/comments.xml".to_string()],
        policy_violations: vec![],
        comment_id: comment_id_num,
    })
}

/// Get the next comment ID from existing comments.xml.
fn next_comment_id(pkg: &crate::opc::OpcPackage) -> String {
    let max_id = pkg
        .parts
        .iter()
        .filter(|p| p.uri == "word/comments.xml")
        .flat_map(|p| extract_comment_ids(&p.data))
        .filter_map(|s| s.parse::<i64>().ok())
        .max()
        .unwrap_or(-1);
    (max_id + 1).to_string()
}

/// Extract comment IDs from XML data.
fn extract_comment_ids(data: &[u8]) -> Vec<String> {
    let s = String::from_utf8_lossy(data);
    let mut ids = Vec::new();
    let mut search: &str = &s;
    while let Some(pos) = search.find(r#"w:id="#) {
        let rest = &search[pos + 6..];
        if let Some(end) = rest.find('"') {
            ids.push(rest[..end].to_string());
            search = &rest[end + 1..];
        } else {
            break;
        }
    }
    ids
}

/// Build the XML for a comment element.
fn format_comment_xml(comment: &Comment, id: &str) -> String {
    let author = escape_xml_attr(&comment.author);
    let date = comment.date.as_deref().unwrap_or("");
    let initials = comment.initials.as_deref().unwrap_or("");

    let mut xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#
        .to_string();

    xml.push_str(&format!(
        r#"<w:comment w:id="{}" w:author="{}""#,
        id, author
    ));
    if !date.is_empty() {
        xml.push_str(&format!(r#" w:date="{}""#, escape_xml_attr(date)));
    }
    if !initials.is_empty() {
        xml.push_str(&format!(r#" w:initials="{}""#, escape_xml_attr(initials)));
    }

    // Build content paragraphs from SectionItem.
    let content = build_comment_content_xml(&comment.content);

    xml.push_str(&format!(">{}{}</w:comment>", content, "</w:comments>"));
    xml
}

/// Build the paragraph content for a comment.
fn build_comment_content_xml(content_items: &[gp_core::ir::SectionItem]) -> String {
    if content_items.is_empty() {
        return String::new();
    }

    // For simplicity, render as text paragraphs.
    let mut xml = String::new();
    for item in content_items {
        match item {
            gp_core::ir::SectionItem::Paragraph(p) => {
                let text = p
                    .runs
                    .iter()
                    .map(|r| escape_xml(&r.text))
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    xml.push_str(&format!(r#"<w:p><w:r><w:t>{}</w:t></w:r></w:p>"#, text));
                }
            }
            gp_core::ir::SectionItem::Table(_)
            | gp_core::ir::SectionItem::Drawing(_)
            | gp_core::ir::SectionItem::List(_) => {
                xml.push_str(r#"<w:p><w:r><w:t>Comment</w:t></w:r></w:p>"#);
            }
        }
    }

    // Fallback: if content_items had no paragraph/text we couldn't render,
    // just use the comment.comment_id as placeholder.
    if xml.is_empty() {
        xml = r#"<w:p><w:r><w:t>Comment added</w:t></w:r></w:p>"#.to_string();
    }

    xml
}

/// Create initial comments.xml content.
fn create_comments_xml(first_comment: &str) -> Vec<u8> {
    first_comment.as_bytes().to_vec()
}

/// Append a comment element to existing comments.xml data.
fn append_comment_to_xml(existing: &[u8], new_comment: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(existing).to_string();
    if let Some(end_pos) = s.rfind("</w:comments>") {
        let mut result = s.clone();
        result.insert_str(end_pos, new_comment);
        result.into_bytes()
    } else {
        // No closing tag found — prepend XML declaration and append.
        let mut result = String::from_utf8_lossy(existing).to_string();
        result.push('\n');
        result.push_str(new_comment);
        result.into_bytes()
    }
}

/// Update document.xml to add commentReference anchor elements.
fn update_document_with_comment_anchor(
    doc_xml: &str,
    element_id: &ElementId,
    comment_id: &str,
) -> String {
    let mut result = doc_xml.to_string();

    // Try to find the anchor element by ID in the document XML.
    // Look for elements with w:id or w:anchorId attributes.
    let anchor_pattern = format!("w:anchorId=\"{}\"", element_id);
    if let Some(anchor_pos) = result.find(&anchor_pattern) {
        // Insert commentRangeStart before the matching element.
        let insert_pos = anchor_pos;
        let reference_xml = format!(r#"<w:commentRangeStart w:id="{}"/>"#, comment_id);
        let reference_end = format!(r#"<w:commentRangeEnd w:id="{}"/>"#, comment_id);
        let reference_ref = format!(r#"<w:commentReference w:id="{}"/>"#, comment_id);

        result.insert_str(insert_pos, &reference_xml);

        // Find a good insertion point for RangeEnd (after the anchor element).
        // For simplicity, append before the </w:body> tag.
        if let Some(body_end) = result.rfind("</w:body>") {
            result.insert_str(body_end, &reference_end);
        } else {
            // Fallback: append at end.
            result.push_str(&reference_end);
        }

        // Insert commentReference somewhere in the paragraph flow.
        if let Some(run_start) = result.find("<w:r>") {
            result.insert_str(run_start, &reference_ref);
        } else {
            result.push_str(&reference_ref);
        }
    } else {
        // No anchor found — just append comment markers at the end of body.
        let reference_xml = format!(r#"<w:commentRangeStart w:id="{}"/>"#, comment_id);
        let reference_end = format!(r#"<w:commentRangeEnd w:id="{}"/>"#, comment_id);

        if let Some(body_end) = result.rfind("</w:body>") {
            result.insert_str(body_end, &reference_xml);
            result.insert_str(body_end, &reference_end);
        }
    }

    result
}

/// Update [Content_Types].xml to include the comments part.
#[allow(dead_code)]
fn update_content_types_for_comments(ct_xml: &str) -> String {
    let override_entry = r#"<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>"#;

    if ct_xml.contains("word/comments.xml") {
        ct_xml.to_string()
    } else if let Some(end_tag) = ct_xml.rfind("</Types>") {
        let mut result = ct_xml.to_string();
        result.insert_str(end_tag, override_entry);
        result
    } else {
        // Fallback: append.
        let mut result = ct_xml.to_string();
        result.push_str(override_entry);
        result
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_comment_ids() {
        let data = b"<w:comments xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n<w:comment w:id=\"0\" w:author=\"A\">1</w:comment>\n<w:comment w:id=\"5\" w:author=\"B\">2</w:comment>\n<w:comment w:id=\"3\" w:author=\"C\">3</w:comment>\n</w:comments>";
        let ids = extract_comment_ids(data);
        assert_eq!(ids, vec!["0", "5", "3"]);
    }

    #[test]
    fn test_next_comment_id() {
        use crate::opc::OpcPackage;
        let pkg = OpcPackage {
            parts: vec![],
            content_types: Default::default(),
            relationships: Default::default(),
            diagnostics: vec![],
        };
        assert_eq!(next_comment_id(&pkg), "0");

        let pkg_with_comments = OpcPackage {
            parts: vec![crate::opc::Part {
                uri: "word/comments.xml".to_string(),
                content_type: "application/xml".to_string(),
                data: b"<w:comments xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:comment w:id=\"42\" w:author=\"X\">test</w:comment></w:comments>".to_vec(),
            }],
            content_types: Default::default(),
            relationships: Default::default(),
            diagnostics: vec![],
        };
        assert_eq!(next_comment_id(&pkg_with_comments), "43");
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("<b>&</b>"), "&lt;b&gt;&amp;&lt;/b&gt;");
        assert_eq!(escape_xml_attr("a\"b"), "a&quot;b");
    }

    #[test]
    fn test_create_comments_xml() {
        let comment = r#"<w:comment w:id="0" w:author="A"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:comment>"#;
        let data = create_comments_xml(comment);
        let s = String::from_utf8_lossy(&data);
        assert!(s.contains(comment));
    }

    #[test]
    fn test_append_comment_to_xml() {
        let existing = b"<w:comments xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n<w:comment w:id=\"0\" w:author=\"A\">existing</w:comment>\n</w:comments>";
        let new = r#"<w:comment w:id="1" w:author="B">new</w:comment>"#;
        let result = append_comment_to_xml(existing, new);
        let s = String::from_utf8_lossy(&result);
        assert!(s.contains("existing"));
        assert!(s.contains("new"));
        assert!(s.contains("</w:comments>"));
    }

    #[test]
    fn test_update_content_types_for_comments() {
        let ct = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/xml"/></Types>"#;
        let result = update_content_types_for_comments(ct);
        assert!(result.contains("word/comments.xml"));
        assert!(result.contains("comments+xml"));
    }

    #[test]
    fn test_format_comment_xml() {
        let comment = Comment {
            id: ElementId::from_str("test-1"),
            comment_id: "test-1".to_string(),
            author: "Reviewer".to_string(),
            date: Some("2025-01-01T00:00:00Z".to_string()),
            initials: Some("RV".to_string()),
            content: vec![],
        };
        let xml = format_comment_xml(&comment, "0");
        assert!(xml.contains("w:id=\"0\""));
        assert!(xml.contains("w:author=\"Reviewer\""));
        assert!(xml.contains("w:date=\"2025-01-01T00:00:00Z\""));
        assert!(xml.contains("w:initials=\"RV\""));
        assert!(xml.contains("</w:comment>"));
    }
}
