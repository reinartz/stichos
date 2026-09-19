// SPDX-License-Identifier: MIT OR Apache-2.0
//! OOXML revision enumeration — walk `w:ins`/`w:del` trees into C11 `Revision` objects.
//!
//! Provides `enumerate_revisions()` which depth-first traverses a parsed
//! Word document XML, extracting insertions, deletions, and refusing
//! unsupported kinds (moveTo, moveFrom, rPrChange, pPrChange).

use crate::xml::{TagName, XmlDocument, XmlNode};
use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;

/// Anchor context: tracks paragraph-level anchors seen during traversal.
#[derive(Debug, Default)]
pub struct AnchorContext {
    /// Last-seen paragraph element ID.
    last_para_id: Option<ElementId>,
    /// Last-seen section item ID (for table cells, lists).
    last_section_item_id: Option<ElementId>,
    /// Map from paraId attribute value to ElementId.
    paratd_map: std::collections::HashMap<String, ElementId>,
}

/// Enumerate revisions from a parsed Word document XML.
///
/// Walks the XML tree depth-first, extracting `w:ins` and `w:del` elements
/// into `Revision` objects. Unsupported kinds (`moveTo`, `moveFrom`, `rPrChange`,
/// `pPrChange`) produce `GP-R-001` diagnostics.
pub fn enumerate_revisions(xml_doc: &XmlDocument, diags: &mut Vec<Diagnostic>) -> Vec<Revision> {
    let mut revisions: Vec<Revision> = Vec::new();
    let mut ctx = AnchorContext::default();
    enumerate_revisions_recursive(&xml_doc.root, &mut revisions, &mut ctx, diags);
    revisions
}

fn enumerate_revisions_recursive(
    node: &XmlNode,
    revisions: &mut Vec<Revision>,
    ctx: &mut AnchorContext,
    diags: &mut Vec<Diagnostic>,
) {
    if let XmlNode::Element {
        name,
        attributes,
        children,
        ..
    } = node
    {
        // Track paragraph anchors: w:p elements.
        if is_word_elem(name, "p") {
            let para_id = ElementId::derive("paragraph", &["paragraph"], &node.to_bytes());
            ctx.last_para_id = Some(para_id);
            // Map paraId if present on the paragraph.
            if let Some(paranum) = get_revs_attr(attributes, "paraId") {
                ctx.paratd_map
                    .insert(paranum, ctx.last_para_id.clone().unwrap());
            }
            for child in children {
                enumerate_revisions_recursive(child, revisions, ctx, diags);
            }
            return;
        }

        // Track section items: w:tc, w:tr, etc.
        if is_word_elem(name, "tc")
            || is_word_elem(name, "tr")
            || is_word_elem(name, "list")
            || is_word_elem(name, "tbl")
        {
            let sid = ElementId::derive(
                "section_item",
                &[name.local_name.as_str()],
                &node.to_bytes(),
            );
            ctx.last_section_item_id = Some(sid);
        }

        // Handle ins — Insert revision.
        if is_word_elem(name, "ins") {
            handle_revision(
                RevisionKind::Insert,
                node,
                revisions,
                ctx,
                diags,
                attributes,
                children,
            );
        }
        // Handle del — Delete revision.
        else if is_word_elem(name, "del") {
            handle_revision(
                RevisionKind::Delete,
                node,
                revisions,
                ctx,
                diags,
                attributes,
                children,
            );
        }
        // Refuse moveTo / moveFrom.
        else if is_word_elem(name, "moveTo") {
            diags.push(Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                "Unsupported revision kind: moveTo".to_string(),
            ));
        } else if is_word_elem(name, "moveFrom") {
            diags.push(Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                "Unsupported revision kind: moveFrom".to_string(),
            ));
        }
        // Refuse rPrChange / pPrChange.
        else if is_word_elem(name, "rPrChange") {
            diags.push(Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                "Unsupported revision kind: rPrChange".to_string(),
            ));
        } else if is_word_elem(name, "pPrChange") {
            diags.push(Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                "Unsupported revision kind: pPrChange".to_string(),
            ));
        }

        // Recurse into children for nested revisions.
        for child in children {
            enumerate_revisions_recursive(child, revisions, ctx, diags);
        }
    }
}

/// Handle a single revision element (ins or del).
fn handle_revision(
    kind: RevisionKind,
    node: &XmlNode,
    revisions: &mut Vec<Revision>,
    ctx: &mut AnchorContext,
    diags: &mut Vec<Diagnostic>,
    _attributes: &std::collections::BTreeMap<TagName, String>,
    _children: &[XmlNode],
) {
    if let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    {
        // Check for unsupported nested kinds inside this element.
        for child in children {
            if let XmlNode::Element { name, .. } = child {
                if is_word_elem(name, "rPrChange")
                    || is_word_elem(name, "pPrChange")
                    || is_word_elem(name, "moveTo")
                    || is_word_elem(name, "moveFrom")
                {
                    // Already handled by the outer check; skip to avoid duplicates.
                }
            }
        }

        let author = get_revs_attr(attributes, "author");
        let date = get_revs_attr(attributes, "date");

        // Extract text content from children.
        let content = extract_revision_content(node);

        // Find anchor paragraph ID.
        let anchor = find_anchor_id(ctx);

        // Page hint: derive from last paragraph's position (placeholder: 1).
        let page_hint = ctx.last_para_id.as_ref().map(|_| 1_usize);

        let tag = if kind == RevisionKind::Insert {
            "ins"
        } else {
            "del"
        };
        let rev_id = ElementId::derive(
            "revision",
            &[tag, &content[..content.len().min(32)]],
            &content,
        );

        // Warning if no author provided (GP-R-003).
        if author.is_none() {
            diags.push(Diagnostic::new(
                "GP-R-003",
                Severity::Warning,
                format!("Revision {} has no author and none provided", rev_id),
            ));
        }

        revisions.push(Revision {
            id: rev_id,
            kind,
            author: author.clone(),
            date: date.clone(),
            anchor_element_id: anchor,
            page_hint,
            content: content.clone(),
            property_name: None,
            original_value: None,
            new_value: None,
        });

        // Store paraId cross-links from attributes.
        if let Some(paranum) = get_revs_attr(attributes, "paraId") {
            if let Some(anchor_id) = &ctx.last_para_id {
                let _ = ctx.paratd_map.insert(paranum, anchor_id.clone());
            }
        }
    }
}

/// Extract text content from a revision element's children.
pub fn extract_revision_content(node: &XmlNode) -> String {
    let mut content = String::new();
    extract_revision_content_recursive(node, &mut content);
    content
}

fn extract_revision_content_recursive(node: &XmlNode, out: &mut String) {
    match node {
        XmlNode::Text { text, .. } => out.push_str(text),
        XmlNode::Element { children, .. } => {
            for child in children {
                extract_revision_content_recursive(child, out);
            }
        }
        _ => {}
    }
}

/// Find the nearest anchor ElementId (paragraph or section item).
pub fn find_anchor_id(ctx: &AnchorContext) -> ElementId {
    if let Some(ref pid) = ctx.last_para_id {
        pid.clone()
    } else if let Some(ref sid) = ctx.last_section_item_id {
        sid.clone()
    } else {
        ElementId::derive("revision_anchor", &["anonymous"], &())
    }
}

/// Get a revision element attribute value (handles w: prefix and local name).
fn get_revs_attr(
    attributes: &std::collections::BTreeMap<TagName, String>,
    key: &str,
) -> Option<String> {
    for (tag, val) in attributes {
        if tag.local_name == key
            || tag.local_name.ends_with(&format!(":{}", key))
            || tag.local_name == format!("w:{}", key)
        {
            return Some(val.clone());
        }
    }
    None
}

/// Check if a tag name is a Word element with the given local name.
fn is_word_elem(name: &TagName, local: &str) -> bool {
    name.local_name == local
        && (name.prefix.as_deref() == Some("w")
            || name.prefix.as_deref() == Some("w14")
            || name.prefix.as_deref() == Some("w15")
            || name.prefix.as_deref() == Some("w16")
            || name.prefix.as_deref() == Some("w16ex"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_xml;

    /// Helper: create a minimal XmlDocument from XML string.
    fn make_doc(xml: &str) -> XmlDocument {
        parse_xml(xml.as_bytes()).expect("should parse xml")
    }

    /// test_enumerate_revisions_insert — parse w:ins, produce Revision { kind: Insert }.
    #[test]
    fn test_enumerate_revisions_insert() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:author="Alice" w:date="2026-01-01T00:00:00Z" w:id="0">
                    <w:p><w:r><w:t>Inserted text</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
        assert_eq!(revisions[0].author.as_deref(), Some("Alice"));
        assert_eq!(revisions[0].date.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert!(!revisions[0].content.is_empty());
        assert!(revisions[0].id.is_valid());
        assert!(revisions[0].anchor_element_id.is_valid());
    }

    /// test_enumerate_revisions_delete — parse w:del, produce Revision { kind: Delete }.
    #[test]
    fn test_enumerate_revisions_delete() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:del w:author="Bob" w:date="2026-02-01T00:00:00Z" w:id="1">
                    <w:p><w:r><w:t>Deleted text</w:t></w:r></w:p>
                </w:del>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Delete);
        assert_eq!(revisions[0].author.as_deref(), Some("Bob"));
        assert_eq!(revisions[0].date.as_deref(), Some("2026-02-01T00:00:00Z"));
        assert!(!revisions[0].content.is_empty());
    }

    /// test_enumerate_revisions_nested — ins inside del produces two Revision objects.
    #[test]
    fn test_enumerate_revisions_nested() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:del w:author="Alice" w:date="2026-01-01T00:00:00Z" w:id="0">
                    <w:p><w:r><w:t>Original</w:t></w:r></w:p>
                    <w:ins w:author="Alice" w:date="2026-01-02T00:00:00Z" w:id="1">
                        <w:p><w:r><w:t>Replacement</w:t></w:r></w:p>
                    </w:ins>
                </w:del>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        // The outer del should be processed first, then nested ins.
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].kind, RevisionKind::Delete);
        assert_eq!(revisions[1].kind, RevisionKind::Insert);
        assert_eq!(revisions[0].author.as_deref(), Some("Alice"));
        assert_eq!(revisions[1].author.as_deref(), Some("Alice"));
    }

    /// test_enumerate_revisions_refuse_format_change — w:rPrChange produces GP-R-001 diagnostic.
    #[test]
    fn test_enumerate_revisions_refuse_format_change() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:rPrChange w:author="Alice" w:date="2026-01-01T00:00:00Z">
                    <w:rPr><w:b/></w:rPr>
                </w:rPrChange>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 0);
        assert!(
            diags.iter().any(|d| d.code == "GP-R-001"),
            "Should have GP-R-001 for rPrChange"
        );
        assert!(
            diags.iter().any(|d| d.message.contains("rPrChange")),
            "Diagnostic should mention rPrChange"
        );
    }

    /// test_enumerate_revisions_refuse_move — w:moveTo produces GP-R-001 diagnostic.
    #[test]
    fn test_enumerate_revisions_refuse_move() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:moveTo w:author="Alice" w:id="0"/>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 0);
        assert!(
            diags.iter().any(|d| d.code == "GP-R-001"),
            "Should have GP-R-001 for moveTo"
        );
        assert!(
            diags.iter().any(|d| d.message.contains("moveTo")),
            "Diagnostic should mention moveTo"
        );
    }

    /// test_enumerate_revisions_author_date — author and date attributes captured.
    #[test]
    fn test_enumerate_revisions_author_date() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:author="Charlie" w:date="2026-03-15T12:30:00Z" w:id="5">
                    <w:p><w:r><w:t>Some content</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        let rev = &revisions[0];
        assert_eq!(rev.author.as_deref(), Some("Charlie"));
        assert_eq!(rev.date.as_deref(), Some("2026-03-15T12:30:00Z"));
        assert!(rev.id.is_valid());
    }

    /// test_enumerate_revisions_empty — no ins/del produces empty vec.
    #[test]
    fn test_enumerate_revisions_empty() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Plain text</w:t></w:r></w:p>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert!(revisions.is_empty());
        assert!(
            !diags.iter().any(|d| d.code == "GP-R-001"),
            "Should not have refusal diagnostics"
        );
    }

    /// test_enumerate_revisions_multiple — multiple ins/del produce multiple revisions.
    #[test]
    fn test_enumerate_revisions_multiple() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:author="A" w:id="0"><w:p><w:r><w:t>First</w:t></w:r></w:p></w:ins>
                <w:del w:author="B" w:id="1"><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:del>
                <w:ins w:author="C" w:id="2"><w:p><w:r><w:t>Third</w:t></w:r></w:p></w:ins>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 3);
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
        assert_eq!(revisions[1].kind, RevisionKind::Delete);
        assert_eq!(revisions[2].kind, RevisionKind::Insert);
    }

    /// test_enumerate_revisions_paraId — paraId attribute creates cross-link.
    #[test]
    fn test_enumerate_revisions_paraId() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p w:paraId="12345678">
                    <w:r><w:t>Paragraph</w:t></w:r>
                </w:p>
                <w:ins w:author="Alice" w:id="0" w:paraId="87654321">
                    <w:p><w:r><w:t>Inserted</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
        assert_eq!(revisions[0].author.as_deref(), Some("Alice"));
        assert!(revisions[0].anchor_element_id.is_valid());
    }

    /// test_enumerate_revisions_pPrChange — w:pPrChange produces GP-R-001 diagnostic.
    #[test]
    fn test_enumerate_revisions_pPrChange() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:pPrChange w:author="Alice" w:date="2026-01-01T00:00:00Z">
                    <w:pPr><w:jc w:val="center"/></w:pPr>
                </w:pPrChange>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 0);
        assert!(
            diags.iter().any(|d| d.code == "GP-R-001"),
            "Should have GP-R-001 for pPrChange"
        );
    }

    /// test_enumerate_revisions_moveFrom — w:moveFrom produces GP-R-001 diagnostic.
    #[test]
    fn test_enumerate_revisions_moveFrom() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:moveFrom w:author="Alice" w:id="0"/>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 0);
        assert!(
            diags.iter().any(|d| d.code == "GP-R-001"),
            "Should have GP-R-001 for moveFrom"
        );
    }

    /// test_enumerate_revisions_no_author — missing author produces GP-R-003 warning.
    #[test]
    fn test_enumerate_revisions_no_author() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:id="0"><w:p><w:r><w:t>Unattributed</w:t></w:r></w:p></w:ins>
            </w:body>
        </w:document>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
        assert_eq!(revisions[0].author, None);
        assert!(
            diags.iter().any(|d| d.code == "GP-R-003"),
            "Should have GP-R-003 for missing author"
        );
    }
}
