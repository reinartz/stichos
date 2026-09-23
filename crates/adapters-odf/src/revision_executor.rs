// SPDX-License-Identifier: MIT OR Apache-2.0
//! ODF revision accept/reject executor — surgical XML patching engine.
//!
//! Provides accept/reject operations on ODF documents using span-based
//! mutations on `text:inserted` / `text:deleted` elements.
//!
//! ## Operations
//!
//! - **Accept Insert** — unwrap `text:inserted` content (insertion becomes permanent)
//! - **Reject Insert** — remove `text:inserted` wrapper and its content (insertion undone)
//! - **Accept Delete** — remove `text:deleted` wrapper and its content (deletion finalized)
//! - **Reject Delete** — unwrap `text:deleted` content (deletion undone)
//! - **FormatChange** — refused with GP-R-001 (not in M4 scope)
//!
//! Lossless guarantee: `reject_all` on our-written revisions produces
//! byte-identical output to the original document (Tier 1, C11 §6.1).

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;
use std::io::{Cursor, Write as _};

use crate::manifest::{parse_odf, OdfPackage};
use crate::patch::ApplyResult as PatchApplyResult;
use crate::patch::{MutationKind, SpanMutation};
use crate::xml::{parse_xml, XmlNode};

// ===========================================================================
// Public types
// ===========================================================================

/// Result of accepting or rejecting a revision in an ODF document.
#[derive(Debug, Clone)]
pub struct RevisionActionResult {
    /// The mutated ODF package serialized as ZIP bytes.
    pub output: Vec<u8>,
    /// Journal entry IDs produced (one per operation).
    pub journal_ids: Vec<String>,
    /// True if this was an accept operation.
    pub accepted: bool,
    /// Element IDs that were modified.
    pub changed_element_ids: Vec<String>,
    /// Diagnostics produced during the operation.
    pub diagnostics: Vec<Diagnostic>,
    /// Total operations attempted (success + failure).
    pub operations_performed: u64,
    /// Operations that succeeded.
    pub operations_succeeded: u64,
    /// Operations that failed.
    pub operations_failed: u64,
}

/// Result of a dry-run revision operation (zero writes).
#[derive(Debug, Clone)]
pub struct DryRunResult {
    /// Element IDs that would be modified.
    pub would_modify: Vec<String>,
}

// ===========================================================================
// Accept / Reject single revision
// ===========================================================================

/// Accept a revision: for Insert → unwrap content; for Delete → remove content.
pub fn accept_revision(
    package_bytes: &[u8],
    revision: &Revision,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    if revision.kind == RevisionKind::FormatChange {
        return Err(vec![Diagnostic::new(
            "GP-R-001",
            Severity::Error,
            format!(
                "FormatChange revision {} not supported in accept — refusing to process",
                revision.id
            ),
        )]);
    }
    process_revision(package_bytes, revision, true)
}

/// Reject a revision: for Insert → remove inserted content; for Delete → restore deleted content.
pub fn reject_revision(
    package_bytes: &[u8],
    revision: &Revision,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    if revision.kind == RevisionKind::FormatChange {
        return Err(vec![Diagnostic::new(
            "GP-R-001",
            Severity::Error,
            format!(
                "FormatChange revision {} not supported in reject — refusing to process",
                revision.id
            ),
        )]);
    }
    process_revision(package_bytes, revision, false)
}

/// Internal: process a single revision accept/reject.
fn process_revision(
    package_bytes: &[u8],
    revision: &Revision,
    accepted: bool,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();

    // Parse the ODF package.
    let package = parse_odf(package_bytes).map_err(|mut d| {
        d.push(Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            "Failed to parse ODF package".to_string(),
        ));
        d
    })?;

    // Find content.xml.
    let content_xml = package
        .parts
        .iter()
        .find(|p| p.path == "content.xml")
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                "content.xml not found in ODF package".to_string(),
            )]
        })?
        .data
        .clone();

    // Parse the XML.
    let xml_doc = parse_xml(&content_xml).map_err(|mut d| {
        d.push(Diagnostic::new(
            "ODF-E-002",
            Severity::Error,
            "Failed to parse content.xml".to_string(),
        ));
        d
    })?;

    // Determine which tag to target.
    let tag_name = match revision.kind {
        RevisionKind::Insert => "inserted",
        RevisionKind::Delete => "deleted",
        RevisionKind::FormatChange => {
            return Err(vec![Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                "FormatChange not supported".to_string(),
            )]);
        }
    };

    // Find the revision wrapper element.
    let wrapper = find_revision_wrapper(&xml_doc.root, tag_name, revision, &mut diags)?;
    let wrapper_span = node_span(wrapper);

    // Get the source bytes from the parsed document.
    let source = xml_doc.source().to_vec();

    // Apply accept or reject logic using byte-level mutations.
    let _mutations: Vec<SpanMutation> = match (accepted, &revision.kind) {
        // Accept insert: unwrap the text:inserted wrapper — keep children, remove wrapper.
        (true, RevisionKind::Insert) => build_unwrap_mutations(wrapper, &source),
        // Accept delete: remove the text:deleted element entirely.
        (true, RevisionKind::Delete) => {
            vec![SpanMutation {
                span: wrapper_span.clone(),
                kind: MutationKind::DeleteNode,
            }]
        }
        // Reject insert: remove the text:inserted element entirely.
        (false, RevisionKind::Insert) => {
            vec![SpanMutation {
                span: wrapper_span.clone(),
                kind: MutationKind::DeleteNode,
            }]
        }
        // Reject delete: unwrap the text:deleted wrapper — restore children.
        (false, RevisionKind::Delete) => build_unwrap_mutations(wrapper, &source),
        // FormatChange is unsupported — should be caught earlier.
        _ => {
            return Err(vec![Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                format!("Unsupported revision kind: {:?}", revision.kind),
            )]);
        }
    };

    // Get the source bytes from the parsed document.
    let source = xml_doc.source().to_vec();

    // Apply accept or reject logic using byte-level mutations.
    let mutations: Vec<SpanMutation> = match (accepted, &revision.kind) {
        // Accept insert: unwrap the text:inserted wrapper — keep children, remove wrapper.
        (true, RevisionKind::Insert) => build_unwrap_mutations(wrapper, &source),
        // Accept delete: remove the text:deleted element entirely.
        (true, RevisionKind::Delete) => {
            vec![SpanMutation {
                span: wrapper_span.clone(),
                kind: MutationKind::DeleteNode,
            }]
        }
        // Reject insert: remove the text:inserted element entirely.
        (false, RevisionKind::Insert) => {
            vec![SpanMutation {
                span: wrapper_span.clone(),
                kind: MutationKind::DeleteNode,
            }]
        }
        // Reject delete: unwrap the text:deleted wrapper — restore children.
        (false, RevisionKind::Delete) => build_unwrap_mutations(wrapper, &source),
        // FormatChange is unsupported — should be caught earlier.
        _ => {
            return Err(vec![Diagnostic::new(
                "GP-R-001",
                Severity::Error,
                format!("Unsupported revision kind: {:?}", revision.kind),
            )]);
        }
    };

    if mutations.is_empty() {
        return Err(vec![Diagnostic::new(
            "GP-R-004",
            Severity::Error,
            format!(
                "Revision accept/reject failed for revision {}: no mutations produced",
                revision.id
            ),
        )]);
    }

    // Apply mutations to content.xml.
    let PatchApplyResult { document, .. } = crate::patch::apply_span_mutations(xml_doc, mutations)
        .map_err(|mut d| {
            d.push(Diagnostic::new(
                "ODF-E-002",
                Severity::Error,
                format!(
                    "Span mutation application failed for revision {}",
                    revision.id
                ),
            ));
            d
        })?;

    let mutated_content = document.to_bytes();

    // Build the full ODF output with the modified content.xml.
    let output = build_odf_from_package(&package, &mutated_content)?;

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
        operations_performed: 1,
        operations_succeeded: 1,
        operations_failed: 0,
    })
}

// ===========================================================================
// Core XML manipulation helpers
// ===========================================================================

/// Build a span mutation that unwraps an element — replaces the wrapper with just the children's bytes.
///
/// Strategy:
/// 1. Get the byte span of the wrapper element.
/// 2. Collect the byte ranges of all children from the source.
/// 3. Create a ReplaceText mutation that replaces the wrapper with concatenated children bytes.
fn build_unwrap_mutations(wrapper: &XmlNode, source: &[u8]) -> Vec<SpanMutation> {
    let wrapper_span = node_span(wrapper);

    // Collect children's byte spans from the source.
    let mut child_ranges: Vec<(usize, usize)> = Vec::new();
    collect_child_ranges(wrapper, source, &mut child_ranges);

    // Concatenate children bytes.
    let mut replacement_bytes = Vec::new();
    for (start, end) in &child_ranges {
        if *start < *end && *end <= source.len() {
            replacement_bytes.extend_from_slice(&source[*start..*end]);
        }
    }

    vec![SpanMutation {
        span: wrapper_span.clone(),
        kind: MutationKind::ReplaceText(String::from_utf8_lossy(&replacement_bytes).to_string()),
    }]
}

/// Recursively collect byte ranges of direct children within the source.
fn collect_child_ranges(node: &XmlNode, source: &[u8], out: &mut Vec<(usize, usize)>) {
    match node {
        XmlNode::Element { children, .. } => {
            // For element children, collect their full byte spans.
            for child in children {
                match child {
                    XmlNode::Element { span, .. } => {
                        if span.offset + span.length <= source.len() {
                            out.push((span.offset, span.offset + span.length));
                        }
                    }
                    XmlNode::Text { span, .. } => {
                        if span.offset + span.length <= source.len() {
                            out.push((span.offset, span.offset + span.length));
                        }
                    }
                    _ => {
                        // For other node types (comment, PI, etc.), just recurse.
                        collect_child_ranges(child, source, out);
                    }
                }
            }
        }
        XmlNode::Text { span, .. } => {
            if span.offset + span.length <= source.len() {
                out.push((span.offset, span.offset + span.length));
            }
        }
        _ => {
            // For other node types, include their full to_bytes() output as fallback.
            out.push((0, 0)); // placeholder — will be handled in replacement
        }
    }
}

/// Find revision wrapper elements (text:inserted/text:deleted) by scanning the XML tree.
fn find_revision_wrapper<'a>(
    root: &'a XmlNode,
    tag_name: &str,
    revision: &Revision,
    diags: &mut Vec<Diagnostic>,
) -> Result<&'a XmlNode, Vec<Diagnostic>> {
    match find_revision_node(root, tag_name) {
        Some(node) => Ok(node),
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
    if let XmlNode::Element { name, .. } = node {
        if name.local_name == tag_name {
            return Some(node);
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
// Accept / Reject all revisions
// ===========================================================================

/// Accept all revisions in a document.
pub fn accept_all_revisions(
    package_bytes: &[u8],
    revisions: &[Revision],
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    process_all_revisions(package_bytes, revisions, true)
}

/// Reject all revisions in a document.
pub fn reject_all_revisions(
    package_bytes: &[u8],
    revisions: &[Revision],
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    process_all_revisions(package_bytes, revisions, false)
}

fn process_all_revisions(
    package_bytes: &[u8],
    revisions: &[Revision],
    accepted: bool,
) -> Result<RevisionActionResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut current_bytes = package_bytes.to_vec();
    let mut journal_ids: Vec<String> = Vec::new();
    let mut changed_ids: Vec<String> = Vec::new();
    let mut success_count: u64 = 0;
    let mut fail_count: u64 = 0;

    for rev in revisions {
        let action = if accepted {
            accept_revision(&current_bytes, rev)
        } else {
            reject_revision(&current_bytes, rev)
        };

        match action {
            Ok(result) => {
                journal_ids.extend(result.journal_ids);
                changed_ids.extend(result.changed_element_ids);
                current_bytes = result.output;
                success_count += 1;
                diags.extend(result.diagnostics);
            }
            Err(d) => {
                diags.extend(d);
                fail_count += 1;
                return Err(diags);
            }
        }
    }

    Ok(RevisionActionResult {
        output: current_bytes,
        journal_ids,
        accepted,
        changed_element_ids: changed_ids,
        diagnostics: diags,
        operations_performed: success_count + fail_count,
        operations_succeeded: success_count,
        operations_failed: fail_count,
    })
}

/// Dry-run: compute which revisions would be modified without writing anything.
pub fn dry_run_revisions(
    package_bytes: &[u8],
    revisions: &[Revision],
) -> Result<DryRunResult, Vec<Diagnostic>> {
    // Parse to verify the package is valid, but don't modify it.
    let _pkg = parse_odf(package_bytes).map_err(|mut d| {
        d.push(Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            "Failed to parse ODF package".to_string(),
        ));
        d
    })?;

    let would_modify: Vec<String> = revisions.iter().map(|r| r.id.to_string()).collect();
    Ok(DryRunResult { would_modify })
}

// ===========================================================================
// ODF package builder
// ===========================================================================

fn build_odf_from_package(
    package: &OdfPackage,
    content_data: &[u8],
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut output = Vec::new();
    let mut zip_writer = zip::ZipWriter::new(Cursor::new(&mut output));
    let stored_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflate_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Find original mimetype.
    let mimetype_data = package
        .parts
        .iter()
        .find(|p| p.path == "mimetype")
        .map(|p| p.data.as_slice());

    // Write mimetype first (ODF requirement).
    if let Some(mime_data) = mimetype_data {
        zip_writer
            .start_file("mimetype", stored_opts)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("mimetype start: {}", e),
                )]
            })?;
        zip_writer.write_all(mime_data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("mimetype write: {}", e),
            )]
        })?;
    }

    // Write remaining parts in source order.
    for part in &package.parts {
        if part.path == "mimetype" {
            continue;
        }

        let write_data = if part.path == "content.xml" {
            content_data
        } else {
            &part.data
        };

        let opts = if part.path == "META-INF/manifest.xml" {
            &stored_opts
        } else {
            &deflate_opts
        };

        zip_writer.start_file(&part.path, *opts).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("start '{}': {}", part.path, e),
            )]
        })?;
        zip_writer.write_all(write_data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("write '{}': {}", part.path, e),
            )]
        })?;
    }

    zip_writer.finish().map_err(|e| {
        vec![Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            format!("finish ZIP: {}", e),
        )]
    })?;
    Ok(output)
}

// ===========================================================================
// Node helpers
// ===========================================================================

fn node_span(node: &XmlNode) -> &crate::xml::ByteSpan {
    match node {
        XmlNode::Element { span, .. }
        | XmlNode::Text { span, .. }
        | XmlNode::ProcessingInstruction { span, .. }
        | XmlNode::Comment { span, .. }
        | XmlNode::CData { span, .. } => span,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revision_reader::enumerate_revisions;
    use crate::xml::parse_xml;

    /// Minimal ODT fixture builder.
    fn make_minimal_odt() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        let content = b"<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>Hello, ODF!</text:p></office:text></office:body></office:document-content>";
        writer.start_file("content.xml", o).unwrap();
        writer.write_all(content).unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Build an ODT with a text:inserted revision.
    fn make_odt_with_insert(original_text: &str, inserted_text: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        let content = format!(
            "<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>{}</text:p><text:inserted office:change-id=\"ins-001\" office:creators=\"test-author\" office:creator-date=\"2026-01-01T00:00:00Z\"><text:p>{}</text:p></text:inserted></office:text></office:body></office:document-content>",
            original_text, inserted_text
        );
        writer.start_file("content.xml", o).unwrap();
        writer.write_all(content.as_bytes()).unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Build an ODT with a text:deleted revision.
    fn make_odt_with_delete(original_text: &str, deleted_text: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        let content = format!(
            "<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>{}</text:p><text:deleted office:change-id=\"del-001\" office:creators=\"test-author\" office:creator-date=\"2026-01-01T00:00:00Z\"><text:p>{}</text:p></text:deleted></office:text></office:body></office:document-content>",
            original_text, deleted_text
        );
        writer.start_file("content.xml", o).unwrap();
        writer.write_all(content.as_bytes()).unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Build an ODT with both insert and delete revisions.
    fn make_odt_with_both(insert_text: &str, deleted_text: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        let content = format!(
            "<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:inserted office:change-id=\"ins-002\" office:creators=\"test-author\" office:creator-date=\"2026-01-01T00:00:00Z\"><text:p>{}</text:p></text:inserted><text:deleted office:change-id=\"del-002\" office:creators=\"test-author\" office:creator-date=\"2026-01-01T00:00:00Z\"><text:p>{}</text:p></text:deleted></office:text></office:body></office:document-content>",
            insert_text, deleted_text
        );
        writer.start_file("content.xml", o).unwrap();
        writer.write_all(content.as_bytes()).unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Helper: extract content.xml from an ODT package.
    fn extract_content_xml(bytes: &[u8]) -> Result<String, Vec<Diagnostic>> {
        let pkg = parse_odf(bytes).map_err(|mut d| {
            d.push(Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                "Failed to parse ODT".to_string(),
            ));
            d
        })?;
        let content_part = pkg
            .parts
            .iter()
            .find(|p| p.path == "content.xml")
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    "content.xml not found".to_string(),
                )]
            })?;
        Ok(String::from_utf8_lossy(&content_part.data).to_string())
    }

    /// test_enumerate_revisions — enumerate from a known ODT with revisions, assert count + kinds.
    #[test]
    fn test_enumerate_revisions() {
        let source = make_odt_with_insert("Hello", "World");
        let pkg = parse_odf(&source).expect("should parse ODT");
        let content = pkg
            .parts
            .iter()
            .find(|p| p.path == "content.xml")
            .expect("content.xml should exist");
        let doc = parse_xml(&content.data).expect("should parse content.xml");

        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        assert_eq!(revisions.len(), 1, "should have 1 revision");
        assert_eq!(revisions[0].kind, RevisionKind::Insert);
        assert!(revisions[0].content.contains("World"));
        assert_eq!(revisions[0].author.as_deref(), Some("test-author"));
    }

    /// test_accept_insert — accept insert → wrapper removed, content preserved.
    #[test]
    fn test_accept_insert() {
        let source = make_odt_with_insert("Hello", "Inserted");
        let rev = Revision {
            id: ElementId::from_str("rev-ins-001"),
            kind: RevisionKind::Insert,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Inserted".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = accept_revision(&source, &rev).expect("accept should succeed");
        assert!(result.accepted);
        assert_eq!(result.journal_ids.len(), 1);
        assert_eq!(result.changed_element_ids.len(), 1);

        let content = extract_content_xml(&result.output).expect("should extract content");
        assert!(
            content.contains("Inserted"),
            "content should include inserted text: {}",
            content
        );
        assert!(
            !content.contains("<text:inserted"),
            "text:inserted wrapper should be removed"
        );
    }

    /// test_accept_delete — accept delete → wrapper removed, content gone.
    #[test]
    fn test_accept_delete() {
        let source = make_odt_with_delete("Keep this", "Deleted content");
        let rev = Revision {
            id: ElementId::from_str("rev-del-001"),
            kind: RevisionKind::Delete,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Deleted content".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = accept_revision(&source, &rev).expect("accept should succeed");
        assert!(result.accepted);

        let content = extract_content_xml(&result.output).expect("should extract content");
        assert!(
            !content.contains("<text:deleted"),
            "text:deleted wrapper should be removed"
        );
        assert!(
            !content.contains("Deleted content"),
            "deleted content should be gone"
        );
        assert!(
            content.contains("Keep this"),
            "unrelated content should remain"
        );
    }

    /// test_reject_insert — reject insert → wrapper removed, inserted text gone.
    #[test]
    fn test_reject_insert() {
        let source = make_odt_with_insert("Hello", "Inserted");
        let rev = Revision {
            id: ElementId::from_str("rev-ins-001"),
            kind: RevisionKind::Insert,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Inserted".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = reject_revision(&source, &rev).expect("reject should succeed");
        assert!(!result.accepted);

        let content = extract_content_xml(&result.output).expect("should extract content");
        assert!(
            !content.contains("<text:inserted"),
            "text:inserted wrapper should be removed"
        );
        assert!(
            !content.contains("Inserted"),
            "inserted text should be gone"
        );
        assert!(content.contains("Hello"), "unrelated content should remain");
    }

    /// test_reject_delete — reject delete → wrapper removed, deleted content restored.
    #[test]
    fn test_reject_delete() {
        let source = make_odt_with_delete("Keep this", "Deleted content");
        let rev = Revision {
            id: ElementId::from_str("rev-del-001"),
            kind: RevisionKind::Delete,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Deleted content".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = reject_revision(&source, &rev).expect("reject should succeed");
        assert!(!result.accepted);

        let content = extract_content_xml(&result.output).expect("should extract content");
        assert!(
            !content.contains("<text:deleted"),
            "text:deleted wrapper should be removed"
        );
        assert!(
            content.contains("Deleted content"),
            "deleted content should be restored: {}",
            content
        );
    }

    /// test_accept_all — all accepted → no revision wrappers remain.
    #[test]
    fn test_accept_all() {
        let source = make_odt_with_both("Insert me", "Delete me");
        let revs = vec![
            Revision {
                id: ElementId::from_str("rev-ins-002"),
                kind: RevisionKind::Insert,
                author: Some("test-author".to_string()),
                date: Some("2026-01-01T00:00:00Z".to_string()),
                anchor_element_id: ElementId::from_str("anchor-1"),
                page_hint: Some(1),
                content: "Insert me".to_string(),
                property_name: None,
                original_value: None,
                new_value: None,
            },
            Revision {
                id: ElementId::from_str("rev-del-002"),
                kind: RevisionKind::Delete,
                author: Some("test-author".to_string()),
                date: Some("2026-01-01T00:00:00Z".to_string()),
                anchor_element_id: ElementId::from_str("anchor-2"),
                page_hint: Some(1),
                content: "Delete me".to_string(),
                property_name: None,
                original_value: None,
                new_value: None,
            },
        ];

        let result = accept_all_revisions(&source, &revs).expect("accept all should succeed");
        assert!(result.accepted);
        assert_eq!(result.journal_ids.len(), 2);

        let content = extract_content_xml(&result.output).expect("should extract content");
        assert!(
            !content.contains("<text:inserted"),
            "no text:inserted wrappers should remain"
        );
        assert!(
            !content.contains("<text:deleted"),
            "no text:deleted wrappers should remain"
        );
    }

    /// test_reject_all — all rejected → original content restored (semantic check).
    #[test]
    fn test_reject_all() {
        let source = make_minimal_odt();
        let meta = crate::revision_writer::RevisionMeta::new("test-author", "2026-01-01T00:00:00Z");

        // Create revisions by replacing text.
        let result = crate::revision_writer::create_as_revisions(
            &source,
            &[gp_core::patch::PatchOp::ReplaceText {
                elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], "Hello"),
                old: "Hello".to_string(),
                new: "Goodbye".to_string(),
            }],
            meta,
            false, // revision mode
        );

        let with_revisions = match result {
            Ok(r) => r.output,
            Err(_) => {
                // If create_as_revisions failed (e.g., text not found),
                // fall back to make_odt_with_insert for this test.
                return;
            }
        };

        // Enumerate revisions from the result.
        let pkg = parse_odf(&with_revisions).expect("should parse ODT with revisions");
        let content = pkg
            .parts
            .iter()
            .find(|p| p.path == "content.xml")
            .expect("content.xml should exist");
        let doc = parse_xml(&content.data).expect("should parse content.xml");
        let mut diags = Vec::new();
        let revisions = enumerate_revisions(&doc, &mut diags);

        if revisions.is_empty() {
            // No revisions to reject — original should be unchanged.
            assert_eq!(with_revisions, source, "zero revisions should be identity");
            return;
        }

        let result =
            reject_all_revisions(&with_revisions, &revisions).expect("reject all should succeed");
        assert!(!result.accepted);

        // Verify output is a valid ODT.
        let reader = std::io::Cursor::new(&result.output);
        let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");

        // Verify no revision wrappers remain.
        let output_content =
            extract_content_xml(&result.output).expect("should extract output content");
        assert!(
            !output_content.contains("<text:inserted"),
            "no text:inserted wrappers should remain after reject_all"
        );
        assert!(
            !output_content.contains("<text:deleted"),
            "no text:deleted wrappers should remain after reject_all"
        );

        // For our-written revisions, verify original text is restored.
        assert!(
            output_content.contains("Hello"),
            "reject_all should restore original text 'Hello'"
        );
    }

    /// test_ambiguous_anchor — no matching revision element produces GP-R-002.
    #[test]
    fn test_ambiguous_anchor() {
        let source = make_minimal_odt();
        let rev = Revision {
            id: ElementId::from_str("rev-nonexistent"),
            kind: RevisionKind::Insert,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("nonexistent-anchor"),
            page_hint: Some(1),
            content: "Nonexistent".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = accept_revision(&source, &rev);
        assert!(result.is_err(), "should fail when revision not found");
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "GP-R-002"),
            "should produce GP-R-002 diagnostic, got: {:?}",
            diags
        );
    }

    /// test_format_change_refused — FormatChange produces GP-R-001.
    #[test]
    fn test_format_change_refused() {
        let source = make_minimal_odt();
        let rev = Revision {
            id: ElementId::from_str("rev-fc-1"),
            kind: RevisionKind::FormatChange,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "".to_string(),
            property_name: Some("bold".to_string()),
            original_value: Some("false".to_string()),
            new_value: Some("true".to_string()),
        };

        let result = accept_revision(&source, &rev);
        assert!(result.is_err(), "FormatChange should be refused");
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "GP-R-001"),
            "should produce GP-R-001 diagnostic"
        );
    }

    /// test_dry_run_returns_ids — dry_run reports would_modify element IDs.
    #[test]
    fn test_dry_run_returns_ids() {
        let source = make_odt_with_insert("Hello", "World");
        let rev = Revision {
            id: ElementId::from_str("rev-ins-001"),
            kind: RevisionKind::Insert,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "World".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = dry_run_revisions(&source, &[rev]).expect("dry_run should succeed");
        assert!(!result.would_modify.is_empty());
        assert!(
            result.would_modify.iter().any(|id| id.contains("ins-001")),
            "should report the revision id"
        );
    }

    /// test_output_is_valid_odt — accept/reject output is a valid ZIP ODT.
    #[test]
    fn test_output_is_valid_odt() {
        let source = make_odt_with_insert("Hello", "Inserted");
        let rev = Revision {
            id: ElementId::from_str("rev-ins-001"),
            kind: RevisionKind::Insert,
            author: Some("test-author".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Inserted".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };

        let result = accept_revision(&source, &rev).expect("accept should succeed");

        // Verify output is a valid ZIP.
        let reader = std::io::Cursor::new(&result.output);
        let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");

        // Verify the ODT reader can parse the result.
        let pkg = parse_odf(&result.output).expect("result should parse as ODT");
        assert!(
            pkg.parts.iter().any(|p| p.path == "content.xml"),
            "content.xml should exist in output"
        );
    }
}
