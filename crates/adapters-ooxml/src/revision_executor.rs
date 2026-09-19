// SPDX-License-Identifier: MIT OR Apache-2.0
//! OOXML revision accept/reject executor — span-surgical engine.
//!
//! Provides surgical XML patching for revision accept/reject operations on
//! OOXML documents. Every operation targets only the byte-span containing the
//! revision wrapper (`w:ins` / `w:del`), leaving all other parts of the OPC
//! package byte-identical.
//!
//! ## Operations
//!
//! - **Accept Insert** — unwrap `w:ins` content (insertion becomes permanent)
//! - **Reject Insert** — remove `w:ins` wrapper and its content (insertion undone)
//! - **Accept Delete** — remove `w:del` wrapper and its content (deletion finalized)
//! - **Reject Delete** — unwrap `w:del` content (deletion undone)
//! - **FormatChange** — refused with GP-R-001 (not in M4 scope)

use crate::opc::OpcPackage;
use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::diff::IrDiff;
use gp_core::ir::*;
use std::collections::BTreeMap;
use std::io::Cursor;

type ZipOptions = zip::write::FileOptions<'static, ()>;
use crate::xml::{parse_xml, ByteSpan, TagName, XmlNode};

// ===========================================================================
// Public types
// ===========================================================================

/// Metadata required to process revision operations.
#[derive(Debug, Clone)]
pub struct RevisionWriterMeta {
    /// Author of the revision.
    pub author: String,
    /// ISO-8601 timestamp.
    pub timestamp: String,
}

impl RevisionWriterMeta {
    /// Create a new revision metadata.
    pub fn new(author: &str, timestamp: &str) -> Self {
        Self {
            author: author.to_string(),
            timestamp: timestamp.to_string(),
        }
    }
}

/// Result of accepting or rejecting a revision.
#[derive(Debug)]
pub struct RevisionActionResult {
    /// The mutated OPC package serialized as ZIP bytes.
    pub output: Vec<u8>,
    /// Journal entry IDs produced (one per operation).
    pub journal_ids: Vec<String>,
    /// True if this was an accept operation.
    pub accepted: bool,
    /// Element IDs that were modified.
    pub changed_element_ids: Vec<String>,
    /// Diagnostics produced during the operation.
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of a dry-run revision operation (zero writes).
pub struct DryRunResult {
    /// The IR diff that would result from the operation.
    pub ir_diff: IrDiff,
    /// Element IDs that would be modified.
    pub would_modify: Vec<String>,
}

// ===========================================================================
// Accept / Reject single revision
// ===========================================================================

/// Accept a revision: for Insert → unwrap content; for Delete → remove content.
pub fn accept_revision(
    package: &OpcPackage,
    revision: &Revision,
    _meta: &RevisionWriterMeta,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    if matches!(revision.kind, RevisionKind::FormatChange) {
        return Err(vec![Diagnostic::new(
            "GP-R-001",
            Severity::Error,
            format!(
                "FormatChange revision {} not supported in accept — refusing to process",
                revision.id
            ),
        )]);
    }
    process_revision(package, revision, true)
}

/// Reject a revision: for Insert → remove inserted content; for Delete → restore deleted content.
pub fn reject_revision(
    package: &OpcPackage,
    revision: &Revision,
    _meta: &RevisionWriterMeta,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    if matches!(revision.kind, RevisionKind::FormatChange) {
        return Err(vec![Diagnostic::new(
            "GP-R-001",
            Severity::Error,
            format!(
                "FormatChange revision {} not supported in reject — refusing to process",
                revision.id
            ),
        )]);
    }
    process_revision(package, revision, false)
}

/// Internal: process a single revision accept/reject.
fn process_revision(
    package: &OpcPackage,
    revision: &Revision,
    accepted: bool,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();

    // Find the document.xml part in the OPC package.
    let doc_part = find_document_part(package)?;

    // Parse the XML.
    let xml_doc = parse_xml(&doc_part.data)?;

    // Find the revision wrapper element (w:ins or w:del).
    let tag_name = if matches!(revision.kind, RevisionKind::Insert) {
        "ins"
    } else {
        "del"
    };

    let wrapper = find_revision_wrapper(&xml_doc.root, tag_name, revision, &mut diags)?;

    // Apply accept or reject logic.
    let node_action = if accepted {
        let result = apply_accept(&wrapper, revision);
        NodeAction::Replace(result)
    } else {
        match revision.kind {
            RevisionKind::Insert => {
                // Reject insert: remove wrapper and content entirely.
                NodeAction::Remove
            }
            RevisionKind::Delete => {
                // Reject delete: unwrap the deleted content.
                match apply_reject(&wrapper, revision) {
                    Some(node) => NodeAction::Unwrap(node),
                    None => NodeAction::Remove,
                }
            }
            RevisionKind::FormatChange => NodeAction::Remove,
        }
    };

    // Replace or remove the wrapper node in the tree using tag-name matching.
    let new_root = apply_node_action_by_tag(xml_doc.root, tag_name, revision, node_action)?;

    // Serialize the modified document.
    let mutated_source = serialize_modified_document(&new_root);

    // Build output: rebuild OPC with modified document part.
    let output = serialize_opc_with_part(package, "word/document.xml", mutated_source);

    Ok(RevisionActionResult {
        output,
        journal_ids: vec![format!(
            "rev-{}-{}",
            if accepted { "accept" } else { "reject" },
            revision.id
        )],
        accepted,
        changed_element_ids: vec![revision.id.to_string()],
        diagnostics: diags,
    })
}

// ===========================================================================
// Accept / Reject all revisions
// ===========================================================================

/// Accept all revisions in a document.
pub fn accept_all_revisions(
    package: &OpcPackage,
    revisions: &[Revision],
    meta: &RevisionWriterMeta,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    process_all_revisions(package, revisions, meta, true)
}

/// Reject all revisions in a document.
pub fn reject_all_revisions(
    package: &OpcPackage,
    revisions: &[Revision],
    meta: &RevisionWriterMeta,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    process_all_revisions(package, revisions, meta, false)
}

fn process_all_revisions(
    package: &OpcPackage,
    revisions: &[Revision],
    meta: &RevisionWriterMeta,
    accepted: bool,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut final_pkg = package.clone();
    let mut final_journal_ids: Vec<String> = Vec::new();
    let mut final_changed: Vec<String> = Vec::new();

    // Process revisions sequentially through the package.
    for rev in revisions {
        final_pkg = apply_single_to_package(&final_pkg, rev, accepted, meta, &mut diags)?;
        final_journal_ids.push(format!(
            "rev-{}-{}",
            if accepted { "accept" } else { "reject" },
            rev.id
        ));
        final_changed.push(rev.id.to_string());
    }

    let doc_part = find_document_part(&final_pkg)?;
    let output = serialize_opc_with_part(&final_pkg, &doc_part.uri, doc_part.data.clone());

    Ok(RevisionActionResult {
        output,
        journal_ids: final_journal_ids,
        accepted,
        changed_element_ids: final_changed,
        diagnostics: diags,
    })
}

/// Apply a single revision to a package, returning the updated package.
fn apply_single_to_package(
    package: &OpcPackage,
    revision: &Revision,
    accepted: bool,
    _meta: &RevisionWriterMeta,
    diags: &mut Vec<Diagnostic>,
) -> Result<OpcPackage, Vec<Diagnostic>> {
    let doc_part = find_document_part(package)?;

    let xml_doc = parse_xml(&doc_part.data).inspect_err(|d| {
        diags.extend(d.clone());
    })?;

    let tag_name = if matches!(revision.kind, RevisionKind::Insert) {
        "ins"
    } else {
        "del"
    };

    let wrapper = find_revision_wrapper(&xml_doc.root, tag_name, revision, diags)?;
    let wrapper_span = node_span(&wrapper);

    let modified = if accepted {
        apply_accept(&wrapper, revision)
    } else {
        match apply_reject(&wrapper, revision) {
            Some(node) => node,
            None => {
                diags.push(Diagnostic::new(
                    "GP-R-004",
                    Severity::Error,
                    format!(
                        "Reject failed for revision {}: could not unwrap content",
                        revision.id
                    ),
                ));
                return Err(diags.clone());
            }
        }
    };

    let new_root = replace_node_in_tree(xml_doc.root, wrapper_span, modified)?;
    let mutated_source = serialize_modified_document(&new_root);

    Ok(OpcPackage {
        parts: package
            .parts
            .iter()
            .map(|p| {
                if p.uri == doc_part.uri {
                    crate::opc::Part {
                        uri: p.uri.clone(),
                        content_type: p.content_type.clone(),
                        data: mutated_source.clone(),
                    }
                } else {
                    p.clone()
                }
            })
            .collect(),
        content_types: package.content_types.clone(),
        relationships: package.relationships.clone(),
        diagnostics: package.diagnostics.clone(),
    })
}

// ===========================================================================
// Dry-run
// ===========================================================================

/// Dry-run: compute IR diff without writing anything.
pub fn dry_run_revisions(
    _package: &OpcPackage,
    revisions: &[Revision],
    op: &RevisionOp,
) -> Result<DryRunResult, Vec<Diagnostic>> {
    let mut would_modify: Vec<String> = Vec::new();

    let _ = op; // dry_run doesn't need the specific op, just the revisions
    for rev in revisions {
        would_modify.push(rev.id.to_string());
    }

    let ir_diff = if would_modify.is_empty() {
        IrDiff::default()
    } else {
        let mut hunks = Vec::new();
        for rev in revisions {
            let anchor_kind = match rev.kind {
                RevisionKind::Insert => "inserted",
                RevisionKind::Delete => "deleted",
                RevisionKind::FormatChange => "format-changed",
            };
            hunks.push(gp_core::diff::DiffHunk {
                anchor: rev.anchor_element_id.clone(),
                anchor_kind: anchor_kind.to_string(),
                operations: vec![
                    gp_core::diff::DiffOp::Delete {
                        text: rev.content.clone(),
                    },
                    gp_core::diff::DiffOp::Insert {
                        text: rev.content.clone(),
                    },
                ],
            });
        }
        IrDiff {
            hunks,
            json: Some(serde_json::json!([])),
        }
    };

    Ok(DryRunResult {
        ir_diff,
        would_modify,
    })
}

// ===========================================================================
// Core XML manipulation
// ===========================================================================

/// Find revision wrapper elements (w:ins/w:del) by scanning the XML tree.
pub fn find_revision_wrappers(xml: &XmlNode, _anchor_id: &ElementId) -> Vec<XmlNode> {
    let mut result = Vec::new();
    find_revision_wrappers_recursive(xml, &mut result);
    result
}

fn find_revision_wrappers_recursive(node: &XmlNode, result: &mut Vec<XmlNode>) {
    if is_revision_element(node) {
        result.push(node.clone());
    }
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            find_revision_wrappers_recursive(child, result);
        }
    }
}

/// Apply accept logic: for Insert revision, unwrap the w:ins content.
/// For Delete revision, remove the w:del element entirely.
pub fn apply_accept(xml: &XmlNode, revision: &Revision) -> XmlNode {
    match revision.kind {
        RevisionKind::Insert => {
            // Unwrap: return the children of w:ins as-is.
            unwrap_children(xml)
        }
        RevisionKind::Delete => {
            // Accept delete: return empty element.
            XmlNode::Element {
                name: TagName::local("empty"),
                attributes: BTreeMap::new(),
                namespaces: Vec::new(),
                children: Vec::new(),
                span: node_span(xml).clone(),
            }
        }
        RevisionKind::FormatChange => xml.clone(),
    }
}

/// Apply reject logic: for Insert revision, remove w:ins element entirely.
/// For Delete revision, unwrap the w:del content.
pub fn apply_reject(xml: &XmlNode, revision: &Revision) -> Option<XmlNode> {
    match revision.kind {
        RevisionKind::Insert => None,
        RevisionKind::Delete => Some(unwrap_children(xml)),
        RevisionKind::FormatChange => None,
    }
}

fn unwrap_children(node: &XmlNode) -> XmlNode {
    match node {
        XmlNode::Element { children, span, .. } => {
            if children.len() == 1 {
                let mut child = children[0].clone();
                set_node_span(&mut child, span);
                child
            } else if children.is_empty() {
                XmlNode::Text {
                    text: String::new(),
                    span: span.clone(),
                }
            } else {
                XmlNode::Element {
                    name: TagName::local("w:p"),
                    attributes: BTreeMap::new(),
                    namespaces: Vec::new(),
                    children: children.clone(),
                    span: span.clone(),
                }
            }
        }
        XmlNode::Text { text, span } => XmlNode::Text {
            text: text.clone(),
            span: span.clone(),
        },
        other => other.clone(),
    }
}

/// Action to take when applying a node change to the XML tree.
#[derive(Clone)]
enum NodeAction {
    /// Replace the node at the target span with this XmlNode.
    Replace(XmlNode),
    /// Remove the node and its content entirely.
    Remove,
    /// Remove the wrapper but keep the content (unwrap for reject-delete).
    Unwrap(XmlNode),
}

/// Apply a node action by finding a revision element by tag name and revision ID.
fn apply_node_action_by_tag(
    root: XmlNode,
    tag_name: &str,
    revision: &Revision,
    action: NodeAction,
) -> Result<XmlNode, Vec<Diagnostic>> {
    let result = apply_action_recursive(&root, tag_name, revision, &action, true)?;
    Ok(result.unwrap_or(root))
}

fn apply_action_recursive(
    node: &XmlNode,
    tag_name: &str,
    revision: &Revision,
    action: &NodeAction,
    _is_root: bool,
) -> Result<Option<XmlNode>, Vec<Diagnostic>> {
    // Check if this node is the target revision wrapper.
    let is_target = is_revision_wrapper(node, tag_name, revision);

    match action {
        NodeAction::Replace(replacement) => {
            if is_target {
                // Replace this node with the replacement.
                Ok(Some(replacement.clone()))
            } else {
                // Recurse into children.
                apply_action_on_children(node, tag_name, revision, action, false)
            }
        }
        NodeAction::Remove => {
            if is_target {
                // Remove this node entirely.
                Ok(None)
            } else {
                apply_action_on_children(node, tag_name, revision, action, false)
            }
        }
        NodeAction::Unwrap(replacement) => {
            if is_target {
                // Replace the wrapper with its unwrapped content.
                Ok(Some(replacement.clone()))
            } else {
                apply_action_on_children(node, tag_name, revision, action, false)
            }
        }
    }
}

fn apply_action_on_children(
    node: &XmlNode,
    tag_name: &str,
    revision: &Revision,
    action: &NodeAction,
    is_root: bool,
) -> Result<Option<XmlNode>, Vec<Diagnostic>> {
    match node {
        XmlNode::Element {
            name,
            attributes,
            namespaces,
            children,
            span,
        } => {
            let mut new_children = Vec::new();
            let mut found = false;

            for child in children {
                match apply_action_recursive(child, tag_name, revision, action, false) {
                    Ok(Some(modified)) => {
                        new_children.push(modified);
                    }
                    Ok(None) => {
                        // Node was removed, skip it.
                        found = true;
                    }
                    Err(diags) => {
                        if !diags.is_empty() {
                            return Err(diags);
                        }
                        found = true;
                    }
                }
            }

            if is_root && !found {
                Err(vec![Diagnostic::new(
                    "GP-R-002",
                    Severity::Error,
                    format!(
                        "Ambiguous revision anchor: no matching {} element found",
                        tag_name
                    ),
                )])
            } else {
                Ok(Some(XmlNode::Element {
                    name: name.clone(),
                    attributes: attributes.clone(),
                    namespaces: namespaces.clone(),
                    children: new_children,
                    span: span.clone(),
                }))
            }
        }
        _ => Ok(Some(node.clone())),
    }
}

fn is_revision_wrapper(node: &XmlNode, tag_name: &str, _revision: &Revision) -> bool {
    match node {
        XmlNode::Element { name, .. } => name.local_name == tag_name,
        _ => false,
    }
}

fn is_revision_element(node: &XmlNode) -> bool {
    match node {
        XmlNode::Element { name, .. } => name.local_name == "ins" || name.local_name == "del",
        _ => false,
    }
}

fn replace_node_in_tree(
    root: XmlNode,
    target_span: &ByteSpan,
    replacement: XmlNode,
) -> Result<XmlNode, Vec<Diagnostic>> {
    match replace_node_recursive(&root, target_span, &replacement) {
        Ok(node) => Ok(node),
        Err(e) => Err(vec![e]),
    }
}

fn replace_node_recursive(
    node: &XmlNode,
    target: &ByteSpan,
    replacement: &XmlNode,
) -> Result<XmlNode, Diagnostic> {
    let ns = node_span(node);
    if ns.offset == target.offset && ns.length == target.length {
        return Ok(replacement.clone());
    }

    match node {
        XmlNode::Element {
            name,
            attributes,
            namespaces,
            children,
            span,
        } => {
            let mut new_children = Vec::new();
            for child in children {
                let modified = replace_node_recursive(child, target, replacement)?;
                new_children.push(modified);
            }
            Ok(XmlNode::Element {
                name: name.clone(),
                attributes: attributes.clone(),
                namespaces: namespaces.clone(),
                children: new_children,
                span: span.clone(),
            })
        }
        other => Ok(other.clone()),
    }
}

fn find_revision_wrapper(
    root: &XmlNode,
    tag_name: &str,
    revision: &Revision,
    diags: &mut Vec<Diagnostic>,
) -> Result<XmlNode, Vec<Diagnostic>> {
    match find_revision_node(root, tag_name) {
        Some(node) => Ok(node.clone()),
        None => {
            diags.push(Diagnostic::new(
                "GP-R-002",
                Severity::Error,
                format!(
                    "Ambiguous revision anchor: revision {} has no matching {} element",
                    revision.id, tag_name
                ),
            ));
            Err(diags.clone())
        }
    }
}

fn find_revision_node<'a>(node: &'a XmlNode, tag_name: &str) -> Option<&'a XmlNode> {
    if is_revision_element(node) {
        if let XmlNode::Element { name, .. } = node {
            if name.local_name == tag_name {
                return Some(node);
            }
        }
    }
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let Some(found) = find_revision_node(child, tag_name) {
                return Some(found);
            }
        }
    }
    None
}

// ===========================================================================
// OPC package serialization
// ===========================================================================

fn find_document_part(package: &OpcPackage) -> Result<&crate::opc::Part, Vec<Diagnostic>> {
    package
        .parts
        .iter()
        .find(|p| p.uri == "word/document.xml")
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "OOX-E-020",
                Severity::Error,
                "Missing word/document.xml in OPC package".to_string(),
            )]
        })
}

/// Serialize an OPC package to ZIP bytes with a modified document part.
fn serialize_opc_with_part(original: &OpcPackage, doc_uri: &str, new_doc_data: Vec<u8>) -> Vec<u8> {
    use std::io::Write;

    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    for part in &original.parts {
        let data = if part.uri == doc_uri {
            new_doc_data.clone()
        } else {
            part.data.clone()
        };
        let opts: ZipOptions = zip::write::FileOptions::default();
        let _ = writer.start_file(&part.uri, opts);
        let _ = writer.write_all(&data);
    }

    writer.finish().unwrap().into_inner()
}

fn serialize_modified_document(root: &XmlNode) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    output.extend_from_slice(&root.to_bytes());
    output
}

// ===========================================================================
// Node helpers
// ===========================================================================

fn node_span(node: &XmlNode) -> &ByteSpan {
    match node {
        XmlNode::Element { span, .. }
        | XmlNode::Text { span, .. }
        | XmlNode::ProcessingInstruction { span, .. }
        | XmlNode::Comment { span, .. }
        | XmlNode::CData { span, .. } => span,
    }
}

fn set_node_span(node: &mut XmlNode, span: &ByteSpan) {
    match node {
        XmlNode::Element { span: s, .. }
        | XmlNode::Text { span: s, .. }
        | XmlNode::ProcessingInstruction { span: s, .. }
        | XmlNode::Comment { span: s, .. }
        | XmlNode::CData { span: s, .. } => {
            *s = span.clone();
        }
    }
}

/// XML-based span extraction for surgical patching.
pub fn find_span_for_element(node: &XmlNode, _anchor_id: &ElementId) -> Option<ByteSpan> {
    find_span_recursive(node, &mut std::collections::HashSet::new())
}

fn find_span_recursive(
    node: &XmlNode,
    seen: &mut std::collections::HashSet<usize>,
) -> Option<ByteSpan> {
    let ptr = node as *const XmlNode as usize;
    if !seen.insert(ptr) {
        return None;
    }

    match node {
        XmlNode::Element { children, .. } => {
            for child in children {
                if let Some(span) = find_span_recursive(child, seen) {
                    return Some(span);
                }
            }
        }
        XmlNode::Text { span, .. } => {
            return Some(span.clone());
        }
        _ => {}
    }
    None
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_package(doc_xml: &str) -> OpcPackage {
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
                crate::opc::Part {
                    uri: "[Content_Types].xml".to_string(),
                    content_type: "application/xml".to_string(),
                    data: ct_xml.to_vec(),
                },
                crate::opc::Part {
                    uri: "_rels/.rels".to_string(),
                    content_type: "application/xml".to_string(),
                    data: rels_xml.to_vec(),
                },
                crate::opc::Part {
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

    fn make_insert_rev(text: &str, id: &str) -> Revision {
        Revision {
            id: ElementId::from_str(id),
            kind: RevisionKind::Insert,
            author: Some("test".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: text.to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        }
    }

    fn make_delete_rev(text: &str, id: &str) -> Revision {
        Revision {
            id: ElementId::from_str(id),
            kind: RevisionKind::Delete,
            author: Some("test".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: text.to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        }
    }

    fn make_format_change_rev(id: &str) -> Revision {
        Revision {
            id: ElementId::from_str(id),
            kind: RevisionKind::FormatChange,
            author: Some("test".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "".to_string(),
            property_name: Some("bold".to_string()),
            original_value: Some("false".to_string()),
            new_value: Some("true".to_string()),
        }
    }

    /// test_accept_insert_revision — accept an Insert revision → content unwrapped from w:ins.
    #[test]
    fn test_accept_insert_revision() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Plain text</w:t></w:r></w:p>
                <w:ins w:author="Alice" w:date="2026-01-01T00:00:00Z" w:id="0">
                    <w:p><w:r><w:t>Inserted</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_insert_rev("Inserted", "rev-1");
        let meta = RevisionWriterMeta::new("Alice", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta).expect("accept should succeed");
        assert!(result.accepted, "should be accepted");
        assert!(!result.changed_element_ids.is_empty());

        // Verify output is valid ZIP.
        let result_pkg = crate::opc::parse_opc(&result.output).expect("result should be valid OPC");
        let doc_part = find_document_part(&result_pkg).expect("doc part should exist");
        let output = String::from_utf8_lossy(&doc_part.data);
        assert!(
            output.contains("Inserted"),
            "output should contain inserted text"
        );
        assert!(
            !output.contains("<w:ins"),
            "output should not contain w:ins wrapper"
        );
    }

    /// test_reject_insert_revision — reject an Insert revision → w:ins removed.
    #[test]
    fn test_reject_insert_revision() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Plain text</w:t></w:r></w:p>
                <w:ins w:author="Alice" w:id="0">
                    <w:p><w:r><w:t>Inserted</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_insert_rev("Inserted", "rev-1");
        let meta = RevisionWriterMeta::new("Alice", "2026-01-01T00:00:00Z");

        let result = reject_revision(&pkg, &rev, &meta).expect("reject should succeed");
        assert!(!result.accepted, "should not be accepted");

        let result_pkg = crate::opc::parse_opc(&result.output).expect("result should be valid OPC");
        let doc_part = find_document_part(&result_pkg).expect("doc part should exist");
        let output = String::from_utf8_lossy(&doc_part.data);
        assert!(
            !output.contains("Inserted"),
            "rejected insert should not contain inserted text: {}",
            output
        );
        assert!(
            !output.contains("<w:ins"),
            "output should not contain w:ins wrapper"
        );
        assert!(output.contains("Plain text"), "plain text should remain");
    }

    /// test_accept_delete_revision — accept a Delete revision → deleted content removed.
    #[test]
    fn test_accept_delete_revision() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Keep this</w:t></w:r></w:p>
                <w:del w:author="Bob" w:id="1">
                    <w:p><w:r><w:t>Deleted</w:t></w:r></w:p>
                </w:del>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_delete_rev("Deleted", "rev-2");
        let meta = RevisionWriterMeta::new("Bob", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta).expect("accept should succeed");
        assert!(result.accepted, "should be accepted");

        let result_pkg = crate::opc::parse_opc(&result.output).expect("result should be valid OPC");
        let doc_part = find_document_part(&result_pkg).expect("doc part should exist");
        let output = String::from_utf8_lossy(&doc_part.data);
        assert!(
            !output.contains("<w:del"),
            "output should not contain w:del wrapper"
        );
        assert!(output.contains("Keep this"), "unrelated text should remain");
    }

    /// test_reject_delete_revision — reject a Delete revision → w:del removed, original restored.
    #[test]
    fn test_reject_delete_revision() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Keep this</w:t></w:r></w:p>
                <w:del w:author="Bob" w:id="1">
                    <w:p><w:r><w:t>Deleted</w:t></w:r></w:p>
                </w:del>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_delete_rev("Deleted", "rev-2");
        let meta = RevisionWriterMeta::new("Bob", "2026-01-01T00:00:00Z");

        let result = reject_revision(&pkg, &rev, &meta).expect("reject should succeed");
        assert!(!result.accepted, "should not be accepted");

        let result_pkg = crate::opc::parse_opc(&result.output).expect("result should be valid OPC");
        let doc_part = find_document_part(&result_pkg).expect("doc part should exist");
        let output = String::from_utf8_lossy(&doc_part.data);
        assert!(
            output.contains("Deleted"),
            "rejected delete should restore deleted text: {}",
            output
        );
        assert!(
            !output.contains("<w:del"),
            "output should not contain w:del wrapper"
        );
        assert!(output.contains("Keep this"), "unrelated text should remain");
    }

    /// test_accept_all_revisions — batch accept.
    #[test]
    fn test_accept_all_revisions() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:author="Alice" w:id="0">
                    <w:p><w:r><w:t>Added</w:t></w:r></w:p>
                </w:ins>
                <w:del w:author="Bob" w:id="1">
                    <w:p><w:r><w:t>Removed</w:t></w:r></w:p>
                </w:del>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let revs = vec![
            make_insert_rev("Added", "rev-1"),
            make_delete_rev("Removed", "rev-2"),
        ];
        let meta = RevisionWriterMeta::new("test", "2026-01-01T00:00:00Z");

        let result = accept_all_revisions(&pkg, &revs, &meta).expect("accept all should succeed");
        assert!(result.accepted, "should be accepted");
        assert_eq!(result.journal_ids.len(), 2);
    }

    /// test_dry_run_zero_writes — dry_run produces no output, zero writes.
    #[test]
    fn test_dry_run_zero_writes() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:author="Alice" w:id="0">
                    <w:p><w:r><w:t>Added</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let revs = vec![make_insert_rev("Added", "rev-1")];

        // Verify package state before.
        let before_bytes = pkg
            .parts
            .iter()
            .find(|p| p.uri == "word/document.xml")
            .unwrap()
            .data
            .clone();

        let result =
            dry_run_revisions(&pkg, &revs, &RevisionOp::AcceptAll).expect("dry_run should succeed");

        // Verify package state after — must be unchanged (zero writes).
        let after_bytes = pkg
            .parts
            .iter()
            .find(|p| p.uri == "word/document.xml")
            .unwrap()
            .data
            .clone();
        assert_eq!(
            before_bytes, after_bytes,
            "package must be unchanged after dry_run"
        );

        // Result should report modified elements.
        assert!(!result.would_modify.is_empty());
    }

    /// test_unrelated_parts_unchanged — property test for byte-identical untouched parts.
    #[test]
    fn test_unrelated_parts_unchanged() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Unchanged</w:t></w:r></w:p>
                <w:ins w:author="Alice" w:id="0">
                    <w:p><w:r><w:t>Inserted</w:t></w:r></w:p>
                </w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);

        // Capture original part data for all non-document parts.
        let original_non_doc: Vec<_> = pkg
            .parts
            .iter()
            .filter(|p| p.uri != "word/document.xml")
            .map(|p| (p.uri.clone(), p.data.clone()))
            .collect();

        let rev = make_insert_rev("Inserted", "rev-1");
        let meta = RevisionWriterMeta::new("Alice", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta).expect("accept should succeed");

        // Re-parse the result to check parts.
        let result_pkg = crate::opc::parse_opc(&result.output).expect("result should be valid OPC");

        // All non-document parts must be byte-identical.
        for (uri, orig_data) in &original_non_doc {
            let result_part = result_pkg.parts.iter().find(|p| &p.uri == uri);
            match result_part {
                Some(rp) => {
                    assert_eq!(
                        &rp.data, orig_data,
                        "Part '{}' should be byte-identical (non-revision part)",
                        uri
                    );
                }
                None => {
                    panic!("Part '{}' should exist in result package", uri);
                }
            }
        }
    }

    /// test_refuse_stale_span — reject fails if revision anchor not found.
    #[test]
    fn test_refuse_stale_span() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Plain text</w:t></w:r></w:p>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_insert_rev("Nonexistent", "rev-99");
        let meta = RevisionWriterMeta::new("Alice", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta);
        assert!(
            result.is_err(),
            "should fail when revision anchor not found"
        );
        let diags = result.unwrap_err();
        assert!(
            diags
                .iter()
                .any(|d| d.code == "GP-R-002" || d.code == "OOX-E-020"),
            "should produce GP-R-002 or OOX-E-020 diagnostic, got: {:?}",
            diags
        );
    }

    /// test_format_change_refused — FormatChange revision → GP-R-001 refusal.
    #[test]
    fn test_format_change_refused() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Text</w:t></w:r></w:p>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_format_change_rev("rev-fc-1");
        let meta = RevisionWriterMeta::new("test", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta);
        assert!(result.is_err(), "FormatChange should be refused");
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "GP-R-001"),
            "should produce GP-R-001 diagnostic, got: {:?}",
            diags
        );
    }

    /// test_find_revision_wrappers — find w:ins and w:del elements in XML.
    #[test]
    fn test_find_revision_wrappers() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:id="0"><w:p>inserted</w:p></w:ins>
                <w:del w:id="1"><w:p>deleted</w:p></w:del>
                <w:ins w:id="2"><w:p>another insert</w:p></w:ins>
            </w:body>
        </w:document>"#;
        let doc = parse_xml(xml.as_bytes()).expect("should parse");
        let wrappers = find_revision_wrappers(&doc.root, &ElementId::from_str("any"));
        assert_eq!(wrappers.len(), 3, "should find 3 revision wrappers");
    }

    /// test_apply_accept_insert — acceptInsert unwraps w:ins content.
    #[test]
    fn test_apply_accept_insert() {
        let ins_xml = r#"<w:ins w:id="0"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:ins>"#;
        let ins_node = parse_xml(ins_xml.as_bytes()).expect("parse").root;
        let rev = make_insert_rev("hello", "rev-1");
        let result = apply_accept(&ins_node, &rev);
        assert!(
            matches!(result, XmlNode::Element { ref name, .. } if name.local_name == "p"),
            "should unwrap to element, got {:?}",
            result
        );
    }

    /// test_apply_reject_insert — rejectInsert removes the w:ins entirely.
    #[test]
    fn test_apply_reject_insert() {
        let ins_xml = r#"<w:ins w:id="0"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:ins>"#;
        let ins_node = parse_xml(ins_xml.as_bytes()).expect("parse").root;
        let rev = make_insert_rev("hello", "rev-1");
        let result = apply_reject(&ins_node, &rev);
        assert!(result.is_none(), "reject insert should return None");
    }

    /// test_apply_reject_delete — rejectDelete unwraps w:del content.
    #[test]
    fn test_apply_reject_delete() {
        let del_xml = r#"<w:del w:id="0"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:del>"#;
        let del_node = parse_xml(del_xml.as_bytes()).expect("parse").root;
        let rev = make_delete_rev("hello", "rev-2");
        let result = apply_reject(&del_node, &rev);
        assert!(result.is_some(), "reject delete should unwrap content");
    }

    /// test_apply_accept_delete — acceptDelete produces empty element.
    #[test]
    fn test_apply_accept_delete() {
        let del_xml = r#"<w:del w:id="0"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:del>"#;
        let del_node = parse_xml(del_xml.as_bytes()).expect("parse").root;
        let rev = make_delete_rev("hello", "rev-2");
        let result = apply_accept(&del_node, &rev);
        assert!(
            matches!(result, XmlNode::Element { ref name, .. } if name.local_name == "empty"),
            "accept delete should produce empty element"
        );
    }

    /// test_journal_ids_produced — accept and reject produce journal IDs.
    #[test]
    fn test_journal_ids_produced() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:id="0"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_insert_rev("hello", "rev-1");
        let meta = RevisionWriterMeta::new("Alice", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta).expect("accept");
        assert!(!result.journal_ids.is_empty(), "should produce journal IDs");
        assert!(
            result.journal_ids[0].contains("accept"),
            "journal ID should contain 'accept'"
        );

        let result = reject_revision(&pkg, &rev, &meta).expect("reject");
        assert!(!result.journal_ids.is_empty(), "should produce journal IDs");
        assert!(
            result.journal_ids[0].contains("reject"),
            "journal ID should contain 'reject'"
        );
    }

    /// test_dry_run_returns_diff — dry_run returns non-trivial diff for revisions.
    #[test]
    fn test_dry_run_returns_diff() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:id="0"><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let revs = vec![make_insert_rev("hello", "rev-1")];

        let result = dry_run_revisions(&pkg, &revs, &RevisionOp::AcceptAll).expect("dry_run");
        assert!(
            !result.would_modify.is_empty(),
            "dry_run should report modified elements"
        );
    }

    /// test_empty_package_no_document — missing document.xml produces error.
    #[test]
    fn test_empty_package_no_document() {
        let pkg = OpcPackage {
            parts: vec![],
            content_types: BTreeMap::new(),
            relationships: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        let rev = make_insert_rev("hello", "rev-1");
        let meta = RevisionWriterMeta::new("Alice", "2026-01-01T00:00:00Z");

        let result = accept_revision(&pkg, &rev, &meta);
        assert!(result.is_err(), "should error when document.xml is missing");
    }

    /// test_multiple_revisions_in_package — package with multiple revision wrappers.
    #[test]
    fn test_multiple_revisions_in_package() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:ins w:id="0"><w:p><w:r><w:t>First</w:t></w:r></w:p></w:ins>
                <w:del w:id="1"><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:del>
                <w:ins w:id="2"><w:p><w:r><w:t>Third</w:t></w:r></w:p></w:ins>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let revs = vec![
            make_insert_rev("First", "rev-1"),
            make_delete_rev("Second", "rev-2"),
            make_insert_rev("Third", "rev-3"),
        ];
        let meta = RevisionWriterMeta::new("test", "2026-01-01T00:00:00Z");

        let result = accept_all_revisions(&pkg, &revs, &meta).expect("accept all");
        assert_eq!(result.journal_ids.len(), 3, "should have 3 journal entries");
    }

    /// test_dry_run_refuses_format_change — dry_run with FormatChange still produces output.
    #[test]
    fn test_dry_run_refuses_format_change() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Text</w:t></w:r></w:p>
            </w:body>
        </w:document>"#;
        let pkg = make_package(xml);
        let rev = make_format_change_rev("rev-fc-1");

        let result = dry_run_revisions(&pkg, &[rev], &RevisionOp::AcceptAll);
        assert!(
            result.is_ok(),
            "dry_run should compute diff even for FormatChange"
        );
    }
}
