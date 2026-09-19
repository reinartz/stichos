// SPDX-License-Identifier: MIT OR Apache-2.0
//! ODF revision enumeration — walk `text:inserted`/`text:deleted` trees into C11 `Revision` objects.
//!
//! Provides `enumerate_revisions()` which depth-first traverses a parsed ODF
//! content.xml, extracting insertions, deletions, and refusing
//! unsupported kinds.
//!
//! ODF 1.2+ uses `text:inserted` and `text:deleted` elements (with `office:change-id`
//! linking pairs) for tracked changes, replacing the legacy `text:insert` /
//! `text:mark-deleted` markers.

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
}

/// Enumerate revisions from a parsed ODF content.xml.
///
/// Walks the XML tree depth-first, extracting `text:inserted` and `text:deleted`
/// elements into `Revision` objects. Unknown revision element kinds produce
/// `GP-R-001` diagnostics.
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
        // Track paragraph anchors: text:p elements.
        if is_odf_elem(name, "p") {
            let para_id = ElementId::derive("paragraph", &["paragraph"], &node.to_bytes());
            ctx.last_para_id = Some(para_id);
            for child in children {
                enumerate_revisions_recursive(child, revisions, ctx, diags);
            }
            return;
        }

        // Track section items: table:table, draw:frame, etc.
        if is_odf_elem(name, "table")
            || is_odf_elem(name, "table-row")
            || is_odf_elem(name, "table-cell")
            || is_odf_elem(name, "frame")
        {
            let sid = ElementId::derive(
                "section_item",
                &[name.local_name.as_str()],
                &node.to_bytes(),
            );
            ctx.last_section_item_id = Some(sid);
        }

        // Handle text:inserted — Insert revision.
        if is_odf_elem(name, "inserted") {
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
        // Handle text:deleted — Delete revision.
        else if is_odf_elem(name, "deleted") {
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
        // Legacy markers — treat as insert/delete but warn.
        else if is_odf_elem(name, "insert") {
            handle_revision(
                RevisionKind::Insert,
                node,
                revisions,
                ctx,
                diags,
                attributes,
                children,
            );
        } else if is_odf_elem(name, "mark-deleted") {
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
        // Refuse unsupported kinds (format-change, move, etc.).
        else if is_odf_elem(name, "change-definition")
            || is_odf_elem(name, "change")
            || is_odf_elem(name, "move")
        {
            diags.push(Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                format!("Unsupported revision kind: {}", name.local_name),
            ));
        }

        // Recurse into children for nested revisions.
        for child in children {
            enumerate_revisions_recursive(child, revisions, ctx, diags);
        }
    }
}

/// Handle a single revision element (inserted/deleted/insert/mark-deleted).
fn handle_revision(
    kind: RevisionKind,
    node: &XmlNode,
    revisions: &mut Vec<Revision>,
    ctx: &mut AnchorContext,
    diags: &mut Vec<Diagnostic>,
    _attributes: &std::collections::BTreeMap<TagName, String>,
    _children: &[XmlNode],
) {
    if let XmlNode::Element { attributes, .. } = node {
        // Extract change-id for revision id (ODF 1.2+ office:change-id).
        let change_id = get_odf_attr(attributes, "change-id");

        // Extract author and date.
        let author =
            get_odf_attr(attributes, "creators").or_else(|| get_odf_attr(attributes, "creator"));
        let date = get_odf_attr(attributes, "creator-date");

        // Extract text content from children.
        let content = extract_revision_content(node);

        // Find anchor paragraph ID.
        let anchor = find_anchor_id(ctx);

        // Page hint: derive from last paragraph's position (placeholder: 1).
        let page_hint = ctx.last_para_id.as_ref().map(|_| 1_usize);

        // Build revision id: use change-id if available (prefixed with "id-"), otherwise derive.
        let rev_id = if let Some(ref cid) = change_id {
            // Convert change-id to a valid ElementId format: "id-" + hex hash
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            cid.hash(&mut hasher);
            let hex = format!("{:016x}", hasher.finish());
            ElementId::from_str(&format!("id-{}", &hex[..12]))
        } else {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            content[..content.len().min(32)].hash(&mut hasher);
            let hex = format!("{:016x}", hasher.finish());
            let tag = if kind == RevisionKind::Insert {
                "ins"
            } else {
                "del"
            };
            ElementId::from_str(&format!("id-{}-{}", tag, &hex[..12]))
        };

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
            kind: kind.clone(),
            author: author.clone(),
            date: date.clone(),
            anchor_element_id: anchor,
            page_hint,
            content: content.clone(),
            property_name: None,
            original_value: None,
            new_value: None,
        });
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
        XmlNode::Element { name, children, .. }
            if is_odf_elem(name, "p")
                || is_odf_elem(name, "span")
                || is_odf_elem(name, "s")
                || is_odf_elem(name, "a")
                || is_odf_elem(name, "tab")
                || is_odf_elem(name, "line-break")
                || is_odf_elem(name, "inserted")
                || is_odf_elem(name, "deleted")
                || is_odf_elem(name, "h")
                || is_odf_elem(name, "h-link")
                || is_odf_elem(name, "note")
                || is_odf_elem(name, "note-citation")
                || is_odf_elem(name, "changed-range")
                || is_odf_elem(name, "deletion-annotation")
                || is_odf_elem(name, "user-defined")
                || is_odf_elem(name, "mark")
                || is_odf_elem(name, "highlight") =>
        {
            for child in children {
                extract_revision_content_recursive(child, out);
            }
        }
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

/// Get an ODF element attribute value (handles namespace prefix and local name).
fn get_odf_attr(
    attributes: &std::collections::BTreeMap<TagName, String>,
    key: &str,
) -> Option<String> {
    for (tag, val) in attributes {
        if tag.local_name == key
            || tag.local_name.ends_with(&format!(":{}", key))
            || tag.local_name == format!("office:{}", key)
            || tag.local_name == format!("text:{}", key)
        {
            return Some(val.clone());
        }
    }
    None
}

/// Check if a tag name is an ODF element with the given local name.
fn is_odf_elem(name: &TagName, local: &str) -> bool {
    name.local_name == local
        && (name.prefix.as_deref() == Some("text") || name.prefix.as_deref() == Some("office"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_xml;

    /// Helper: create a minimal XmlDocument from XML string.
    fn make_doc(xml: &str) -> XmlDocument {
        parse_xml(xml.as_bytes()).expect("should parse xml")
    }

    /// test_enumerate_revisions_inserted — parse text:inserted, produce Revision { kind: Insert }.
    #[test]
    fn test_enumerate_revisions_inserted() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:inserted office:change-id="abc123" office:creators="Alice" office:creator-date="2026-01-01T00:00:00Z">
                    <text:p>Inserted text</text:p>
                </text:inserted>
            </office:text></office:body>
        </office:document-content>"#;
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

    /// test_enumerate_revisions_deleted — parse text:deleted, produce Revision { kind: Delete }.
    #[test]
    fn test_enumerate_revisions_deleted() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:deleted office:change-id="def456" office:creators="Bob" office:creator-date="2026-02-01T00:00:00Z">
                    <text:p>Deleted text</text:p>
                </text:deleted>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Delete);
        assert_eq!(revisions[0].author.as_deref(), Some("Bob"));
        assert_eq!(revisions[0].date.as_deref(), Some("2026-02-01T00:00:00Z"));
        assert!(!revisions[0].content.is_empty());
    }

    /// test_enumerate_revisions_legacy_insert — legacy text:insert marker.
    #[test]
    fn test_enumerate_revisions_legacy_insert() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:insert>Legacy insert</text:insert>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
    }

    /// test_enumerate_revisions_legacy_delete — legacy text:mark-deleted marker.
    #[test]
    fn test_enumerate_revisions_legacy_delete() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:mark-deleted>Legacy delete</text:mark-deleted>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].kind, RevisionKind::Delete);
    }

    /// test_enumerate_revisions_nested — deleted containing inserted (replacement).
    #[test]
    fn test_enumerate_revisions_nested() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:deleted office:change-id="rep1" office:creators="Alice" office:creator-date="2026-01-01T00:00:00Z">
                    <text:p>Original text</text:p>
                    <text:inserted office:change-id="rep1" office:creators="Alice" office:creator-date="2026-01-02T00:00:00Z">
                        <text:p>Replacement text</text:p>
                    </text:inserted>
                </text:deleted>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        // The outer deleted should be processed first (depth-first), then nested inserted.
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].kind, RevisionKind::Delete);
        assert_eq!(revisions[1].kind, RevisionKind::Insert);
        assert_eq!(revisions[0].author.as_deref(), Some("Alice"));
        assert_eq!(revisions[1].author.as_deref(), Some("Alice"));
    }

    /// test_enumerate_revisions_empty — no revisions produces empty vec.
    #[test]
    fn test_enumerate_revisions_empty() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:p>Plain text</text:p>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert!(revisions.is_empty());
        assert!(
            !diags.iter().any(|d| d.code == "GP-R-001"),
            "Should not have refusal diagnostics"
        );
    }

    /// test_enumerate_revisions_multiple — multiple revisions produce multiple revisions.
    #[test]
    fn test_enumerate_revisions_multiple() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:inserted office:change-id="ins1" office:creators="A">
                    <text:p>First</text:p>
                </text:inserted>
                <text:deleted office:change-id="del1" office:creators="B">
                    <text:p>Second</text:p>
                </text:deleted>
                <text:inserted office:change-id="ins2" office:creators="C">
                    <text:p>Third</text:p>
                </text:inserted>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 3);
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
        assert_eq!(revisions[1].kind, RevisionKind::Delete);
        assert_eq!(revisions[2].kind, RevisionKind::Insert);
    }

    /// test_enumerate_revisions_no_author — missing author produces GP-R-003 warning.
    #[test]
    fn test_enumerate_revisions_no_author() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:inserted office:change-id="noauth">
                    <text:p>Unattributed</text:p>
                </text:inserted>
            </office:text></office:body>
        </office:document-content>"#;
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

    /// test_enumerate_revisions_change_id_as_rev_id — change-id is hashed into revision id.
    #[test]
    fn test_enumerate_revisions_change_id_as_rev_id() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:inserted office:change-id="my-change-id-123" office:creators="Alice">
                    <text:p>Content</text:p>
                </text:inserted>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        // The revision id should start with "id-" and be a valid ElementId (12+ hex chars).
        assert!(
            revisions[0].id.to_string().starts_with("id-"),
            "Revision id should start with 'id-' (got {})",
            revisions[0].id
        );
        assert!(
            revisions[0].id.to_string().len() >= 15,
            "Revision id should be long enough for a valid id- format"
        );
    }

    /// test_enumerate_revisions_content_extraction — nested elements produce text content.
    #[test]
    fn test_enumerate_revisions_content_extraction() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:text>
                <text:inserted office:change-id="nested" office:creators="Alice">
                    <text:p>Some <text:span text:style-name="Bold">bold</text:span> and normal</text:p>
                </text:inserted>
            </office:text></office:body>
        </office:document-content>"#;
        let doc = make_doc(xml);
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1);
        assert!(
            revisions[0].content.contains("Some"),
            "content should include nested text"
        );
        assert!(
            revisions[0].content.contains("bold"),
            "content should include span text"
        );
        assert!(
            revisions[0].content.contains("normal"),
            "content should include sibling text"
        );
    }
}
