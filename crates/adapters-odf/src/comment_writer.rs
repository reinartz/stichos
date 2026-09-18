//! ODF comment write-back — add comments to odt files.
//!
//! Comments are written as <office:annotation> elements in content.xml,
//! preceded by <dc:creator>, <dc:date>, and <text:p> elements.

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::Comment;
use std::collections::BTreeMap;
use std::io::Read;

use crate::writer::{OdfMutation, SurgicalWriter};

/// Result of adding a comment to an ODT.
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

/// Add a comment to an ODT package.
///
/// The comment is inserted as <office:annotation> in content.xml,
/// preceded by <dc:creator>, <dc:date>, and <text:p> elements.
pub fn add_comment_to_odt(
    bytes: &[u8],
    comment: &Comment,
    element_id: &gp_core::ir::ElementId,
    dry_run: bool,
) -> Result<AddCommentResult, Vec<Diagnostic>> {
    let annotation_xml = format_annotation_xml(comment, element_id);

    let content_before = find_content_part_size(bytes);

    if dry_run {
        return Ok(AddCommentResult {
            output: bytes.to_vec(),
            accepted: true,
            diagnostics: vec![Diagnostic::new(
                "ODF-I001",
                gp_core::diagnostics::Severity::Info,
                format!(
                    "DRY RUN: annotation would be added to ODT (id={})",
                    comment.comment_id
                ),
            )],
            changed_parts: vec!["content.xml".to_string()],
            unchanged_parts: vec![],
            changed_element_ids: vec![element_id.to_string()],
            stale_cached_values: vec![],
            byte_diffs: vec![],
            format: "odt".to_string(),
            journal_id: None,
            ir_diff: Some(format!("annotation {} added", comment.comment_id)),
            invalidated_signatures: vec![],
            tainted_parts: vec!["content.xml".to_string()],
            policy_violations: vec![],
            comment_id: comment.comment_id.clone(),
        });
    }

    // Use the surgical writer to insert annotation into content.xml.
    let mut mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();

    // We'll read content.xml, find where to insert, and replace it.
    let new_content = insert_annotation_into_content(bytes, &annotation_xml);

    mutations.insert(
        "content.xml".to_string(),
        OdfMutation::Replace { data: new_content },
    );

    let writer = SurgicalWriter::new(bytes);
    let result = writer.write(mutations).map_err(|e| e.clone())?;
    let bytes_len = result.bytes.len();

    Ok(AddCommentResult {
        output: result.bytes,
        accepted: true,
        diagnostics: vec![],
        changed_parts: vec!["content.xml".to_string()],
        unchanged_parts: result
            .report
            .parts_unchanged
            .iter()
            .filter(|p| *p != "content.xml")
            .cloned()
            .collect(),
        changed_element_ids: vec![element_id.to_string()],
        stale_cached_values: vec![],
        byte_diffs: vec![PartByteDiff {
            part: "content.xml".to_string(),
            before: content_before,
            after: bytes_len as u64,
        }],
        format: "odt".to_string(),
        journal_id: None,
        ir_diff: Some(format!(
            "annotation {} (author={}) added to ODT",
            comment.comment_id,
            escape_odf_attr(&comment.author)
        )),
        invalidated_signatures: vec![],
        tainted_parts: vec!["content.xml".to_string()],
        policy_violations: vec![],
        comment_id: comment.comment_id.clone(),
    })
}

/// Find the content.xml part size in the ODT package.
fn find_content_part_size(bytes: &[u8]) -> u64 {
    use std::io::Cursor;

    if let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) {
        if let Ok(mut reader) = archive.by_name("content.xml") {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            return buf.len() as u64;
        }
    }
    0
}

/// Insert the annotation XML into the content.xml of an ODT package.
fn insert_annotation_into_content(bytes: &[u8], annotation: &str) -> Vec<u8> {
    use std::io::Cursor;

    // Try to read content.xml from the package first.
    let content_bytes = if let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) {
        if let Ok(mut reader) = archive.by_name("content.xml") {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            if !buf.is_empty() {
                return insert_into_xml(&buf, annotation);
            }
        }
        Vec::new()
    } else {
        Vec::new()
    };

    if content_bytes.is_empty() {
        // Treat bytes as raw content.xml data.
        insert_into_xml(bytes, annotation)
    } else {
        insert_into_xml(&content_bytes, annotation)
    }
}

/// Insert annotation XML into content.xml content.
fn insert_into_xml(content: &[u8], annotation: &str) -> Vec<u8> {
    let content = String::from_utf8_lossy(content).to_string();

    // Insert before </office:body> or </office:text> for inline annotations.
    if let Some(pos) = content.rfind("</office:body>") {
        let mut new_content = content.clone();
        new_content.insert_str(pos, annotation);
        return new_content.into_bytes();
    }

    // Fallback: append at the end before </office:document-content>.
    if let Some(pos) = content.rfind("</office:document-content>") {
        let mut new_content = content.clone();
        new_content.insert_str(pos, annotation);
        return new_content.into_bytes();
    }

    // Last resort: append.
    content
        .into_bytes()
        .iter()
        .chain(annotation.as_bytes())
        .cloned()
        .collect()
}

/// Build the office:annotation XML for a comment.
fn format_annotation_xml(comment: &Comment, _element_id: &gp_core::ir::ElementId) -> String {
    let creator = escape_odf_attr(&comment.author);
    let date = comment.date.as_deref().unwrap_or("");
    let text = build_annotation_text(&comment.content);

    format!(
        r#"
<office:annotation>
  <dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">{}</dc:creator>
  <dc:date xmlns:dc="http://purl.org/dc/elements/1.1/">{}</dc:date>
  <text:p xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">{}</text:p>
</office:annotation>"#,
        creator, date, text
    )
}

/// Build the text content for an annotation.
fn build_annotation_text(content_items: &[gp_core::ir::SectionItem]) -> String {
    if content_items.is_empty() {
        return "Comment added".to_string();
    }

    let mut text = String::new();
    for item in content_items {
        match item {
            gp_core::ir::SectionItem::Paragraph(p) => {
                let run_text = p
                    .runs
                    .iter()
                    .map(|r| escape_odf_text(&r.text))
                    .collect::<Vec<_>>()
                    .join(" ");
                if !run_text.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&run_text);
                }
            }
            gp_core::ir::SectionItem::Table(_)
            | gp_core::ir::SectionItem::Drawing(_)
            | gp_core::ir::SectionItem::List(_) => {}
        }
    }

    if text.is_empty() {
        "Comment added".to_string()
    } else {
        text
    }
}

fn escape_odf_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_odf_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_odf_attr() {
        assert_eq!(escape_odf_attr("a\"b"), "a&quot;b");
        assert_eq!(escape_odf_attr("a<b&c>d"), "a&lt;b&amp;c&gt;d");
    }

    #[test]
    fn test_escape_odf_text() {
        assert_eq!(escape_odf_text("<b>&</b>"), "&lt;b&gt;&amp;&lt;/b&gt;");
    }

    #[test]
    fn test_format_annotation_xml() {
        let comment = Comment {
            id: gp_core::ir::ElementId::from_str("test-1"),
            comment_id: "test-1".to_string(),
            author: "Reviewer".to_string(),
            date: Some("2025-01-01T00:00:00Z".to_string()),
            initials: Some("RV".to_string()),
            content: vec![],
        };
        let xml = format_annotation_xml(&comment, &comment.id);
        assert!(xml.contains("Reviewer"));
        assert!(xml.contains("2025-01-01T00:00:00Z"));
        assert!(xml.contains("</office:annotation>"));
    }

    #[test]
    fn test_build_annotation_text_empty() {
        let text = build_annotation_text(&[]);
        assert_eq!(text, "Comment added");
    }

    #[test]
    fn test_build_annotation_text_with_content() {
        let text = build_annotation_text(&[gp_core::ir::SectionItem::Paragraph(
            gp_core::ir::Paragraph {
                id: gp_core::ir::ElementId::from_str("p:0"),
                runs: vec![gp_core::ir::Run {
                    id: gp_core::ir::ElementId::from_str("r:0"),
                    text: "Hello world".to_string(),
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
            },
        )]);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_insert_annotation_into_content() {
        let annotation = r#"
<office:annotation>
  <dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">Reviewer</dc:creator>
</office:annotation>"#;

        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:body><office:text>Body content</office:text></office:body></office:document-content>";
        let result = insert_annotation_into_content(content, annotation);
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("Body content"));
        assert!(result_str.contains("Reviewer"));
        assert!(result_str.contains("</office:body>"));
    }

    #[test]
    fn test_insert_annotation_fallback() {
        let annotation = "<office:annotation><text:p>test</text:p></office:annotation>";
        // Content without office:body tag.
        let content = b"<office:document-content>no body</office:document-content>";
        let result = insert_annotation_into_content(content, annotation);
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("no body"));
        assert!(result_str.contains("test"));
    }
}
