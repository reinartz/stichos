// SPDX-License-Identifier: MIT OR Apache-2.0
//! ODF patch primitives — byte-level mutations on content.xml / styles.xml.
//!
//! This module provides span-based mutation functions for ODF XML documents
//! parsed by `xml.rs`. All mutations work on byte offsets within the original
//! source bytes, enabling byte-exact re-serialization after patching.
//!
//! ODF namespaces handled:
//! - text: `urn:oasis:names:tc:opendocument:xmlns:text:1.0`
//! - table: `urn:oasis:names:tc:opendocument:xmlns:table:1.0`
//! - draw: `urn:oasis:names:tc:opendocument:xmlns:drawing:1.0`
//! - office: `urn:oasis:names:tc:opendocument:xmlns:office:1.0`
//! - style: `urn:oasis:names:tc:opendocument:xmlns:style:1.0`
//! - manifest: `urn:oasis:names:tc:opendocument:xmlns:manifest:1.0`

use crate::xml::{parse_xml, ByteSpan, TagName, XmlDocument, XmlNode};
use gp_core::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A span mutation to apply to an XML document.
#[derive(Debug, Clone)]
pub struct SpanMutation {
    /// The byte span in the original source that identifies what to mutate.
    pub span: ByteSpan,
    /// The kind of mutation to apply.
    pub kind: MutationKind,
}

/// The kind of mutation to apply at a given byte span.
#[derive(Debug, Clone)]
pub enum MutationKind {
    /// Replace the text content within the span.
    ReplaceText(String),
    /// Insert a new node at the span's start offset.
    InsertNode(XmlNode),
    /// Delete the node spanning the given range.
    DeleteNode,
    /// Set an attribute on the element at the span.
    SetAttribute { name: TagName, value: String },
}

/// Result of applying span mutations to an XML document.
#[derive(Debug, Clone)]
pub struct ApplyResult {
    /// The mutated document, re-serialised from the patched source bytes.
    pub document: XmlDocument,
    /// The list of changed spans that were successfully applied.
    pub changed_spans: Vec<ByteSpan>,
}

// ---------------------------------------------------------------------------
// apply_span_mutations
// ---------------------------------------------------------------------------

/// Apply span mutations to an XmlDocument.
///
/// Mutations are applied in **reverse** order of offset so that earlier
/// offsets remain valid while later ones are modified.
///
/// **Span re-verify**: before applying each mutation, the span is verified
/// to map to valid bytes in the *current* (already-patched) source. If a
/// span is stale (offset beyond source length), the entire write is rejected
/// with an `ODF-E-002` diagnostic.
///
/// **Right-to-left / multi-byte UTF-8**: all offsets are byte offsets into
/// the UTF-8 source. Slicing uses byte ranges so CJK and RTL text is
/// preserved correctly.
///
/// **Byte-exact re-serialization**: after all mutations, the document is
/// re-parsed from the patched source bytes to guarantee well-formed XML.
pub fn apply_span_mutations(
    doc: XmlDocument,
    mutations: Vec<SpanMutation>,
) -> Result<ApplyResult, Vec<Diagnostic>> {
    let mut source = doc.source().to_vec();
    let mut changed_spans: Vec<ByteSpan> = Vec::new();

    // Sort mutations by offset descending so earlier positions stay valid.
    let mut sorted_mutations: Vec<SpanMutation> = mutations;
    sorted_mutations.sort_by(|a, b| {
        b.span
            .offset
            .cmp(&a.span.offset)
            .then_with(|| b.span.end().cmp(&a.span.end()))
    });

    for mutation in &sorted_mutations {
        // Span re-verify: check the span is valid in the current source.
        let start = mutation.span.offset;
        let end = mutation.span.end();

        if start > source.len() {
            return Err(vec![Diagnostic::new(
                "ODF-E-002",
                gp_core::diagnostics::Severity::Error,
                format!(
                    "Stale span: offset {} is beyond source length {}",
                    start,
                    source.len()
                ),
            )]);
        }

        if end > source.len() {
            return Err(vec![Diagnostic::new(
                "ODF-E-002",
                gp_core::diagnostics::Severity::Error,
                format!(
                    "Stale span: end {} exceeds source length {}",
                    end,
                    source.len()
                ),
            )]);
        }

        // Apply the mutation.
        source = apply_single_mutation(source, mutation)?;
        changed_spans.push(mutation.span.clone());
    }

    // Re-parse to verify byte-exact round-trip and produce a clean XmlDocument.
    let document = parse_xml(&source)?;

    Ok(ApplyResult {
        document,
        changed_spans,
    })
}

/// Apply a single mutation to the source bytes.
fn apply_single_mutation(
    source: Vec<u8>,
    mutation: &SpanMutation,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let start = mutation.span.offset;
    let end = mutation.span.end();
    let prefix = source[..start].to_vec();
    let suffix = source[end..].to_vec();

    match &mutation.kind {
        MutationKind::ReplaceText(text) => {
            // Replace the bytes in [start, end) with the new text.
            let mut out = prefix;
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(&suffix);
            Ok(out)
        }
        MutationKind::InsertNode(node) => {
            // Insert the new XML at offset `start` (before whatever was there).
            let node_bytes = node.to_bytes();
            let mut out = prefix;
            out.extend_from_slice(&node_bytes);
            out.extend_from_slice(&suffix);
            Ok(out)
        }
        MutationKind::DeleteNode => {
            // Remove the bytes in [start, end).
            let mut out = prefix;
            out.extend_from_slice(&suffix);
            Ok(out)
        }
        MutationKind::SetAttribute { name, value } => {
            // Find and update/insert the attribute within the element's
            // opening tag that spans the mutation span.
            let mut out = prefix;

            // Scan forward from `start` to find the closing `>` of the tag.
            let tag_end = find_tag_end(&source[start..]).ok_or_else(|| {
                vec![Diagnostic::new(
                    "ODF-E-002",
                    gp_core::diagnostics::Severity::Error,
                    format!("Cannot find end of opening tag at offset {}", start),
                )]
            })?;

            let tag_bytes = &source[start..start + tag_end];
            let tag_str = String::from_utf8_lossy(tag_bytes);

            let (new_tag, _) = set_attribute_in_tag(&tag_str, name, value)?;

            out.extend_from_slice(new_tag.as_bytes());
            out.extend_from_slice(&suffix);

            Ok(out)
        }
    }
}

/// Find the end of the opening tag starting at the given byte slice.
/// Returns the number of bytes from the start (exclusive) that form the
/// complete opening tag (including `>` or `/>`).
fn find_tag_end(tag: &[u8]) -> Option<usize> {
    let mut i = 0;
    let len = tag.len();

    // Skip the '<'
    if i >= len || tag[i] != b'<' {
        return None;
    }
    i += 1;

    // Skip tag name
    while i < len && !matches!(tag[i], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
        i += 1;
    }

    // Skip attributes (quoted strings and unquoted)
    while i < len && tag[i] != b'>' {
        if tag[i] == b'"' || tag[i] == b'\'' {
            let quote = tag[i];
            i += 1;
            while i < len && tag[i] != quote {
                i += 1;
            }
            if i < len {
                i += 1; // skip closing quote
            }
        } else if tag[i] == b'/' {
            i += 1;
            if i < len && tag[i] == b'>' {
                i += 1;
                return Some(i);
            }
        } else {
            i += 1;
        }
    }

    if i < len && tag[i] == b'>' {
        Some(i + 1)
    } else {
        None
    }
}

/// Set or replace an attribute in an XML opening tag string.
/// Returns the modified tag string and whether the attribute was replaced.
fn set_attribute_in_tag(
    tag: &str,
    name: &TagName,
    value: &str,
) -> Result<(String, bool), Vec<Diagnostic>> {
    let qname = name.qname();
    let attr_pattern = format!("{}=\"", qname);

    // Try to find the existing attribute.
    if let Some(pos) = tag.find(&attr_pattern) {
        let after_eq = &tag[pos + attr_pattern.len()..];
        if let Some(end_quote) = after_eq.find('"') {
            // Replace the attribute value.
            // attr_pattern includes '=' and opening '"', so we extract the attr name
            // by stripping the '="' suffix from the pattern.
            let attr_name = &qname[..];
            let before_attr = &tag[..pos];
            let after_attr = &after_eq[end_quote + 1..];
            let new_tag = format!("{}{}=\"{}\"{}", before_attr, attr_name, value, after_attr);
            return Ok((new_tag, true));
        }
    }

    // Attribute not found — insert it before the closing `>` or `/>`.
    let new_attr = format!(" {}=\"{}\"", qname, value);

    // Check if it's a self-closing tag.
    let tag_trimmed = tag.trim_end();
    if tag_trimmed.ends_with("/>") {
        let new_tag = tag.replace("/>", &format!("{} />", new_attr));
        return Ok((new_tag, true));
    }

    // Regular closing `>`.
    let new_tag = format!("{}{}", tag, new_attr);
    Ok((new_tag, true))
}

// ---------------------------------------------------------------------------
// Corpus fixture tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Namespace;

    /// Read an ODF corpus fixture and extract a specific XML part.
    fn corpus_odt_xml(part_name: &str) -> Option<Vec<u8>> {
        let patterns = [
            "cjk-text-odt-001.odt",
            "cjk-text-odt-002.odt",
            "comments-annotations-odt-001.odt",
            "custom-styles-odt-001.odt",
            "embedded-objects-odt-001.odt",
            "forms-odt-001.odt",
            "headers-footers-odt-001.odt",
            "images-odt-001.odt",
            "large-odt-001.odt",
            "math-odt-001.odt",
            "numbering-odt-001.odt",
            "page-breaks-odt-001.odt",
            "rtl-text-odt-001.odt",
            "tables-odt-001.odt",
            "text-alignments-odt-001.odt",
        ];

        for pattern in &patterns {
            let path = format!("{}/corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), pattern);
            if let Ok(data) = std::fs::read(&path) {
                let reader = std::io::Cursor::new(&data);
                if let Ok(mut archive) = zip::ZipArchive::new(reader) {
                    if let Ok(mut entry) = archive.by_name(part_name) {
                        let mut buf = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                        return Some(buf);
                    }
                }
            }
        }
        None
    }

    /// Read an ODS corpus fixture and extract a specific XML part.
    fn corpus_ods_xml(part_name: &str) -> Option<Vec<u8>> {
        let patterns = [
            "cjk-text-ods-001.ods",
            "comments-annotations-ods-001.ods",
            "conditional-formatting-ods-001.ods",
            "custom-styles-ods-001.ods",
            "large-ods-001.ods",
            "math-formula-ods-001.ods",
            "rtl-text-ods-001.ods",
            "tables-ods-001.ods",
        ];

        for pattern in &patterns {
            let path = format!("{}/corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), pattern);
            if let Ok(data) = std::fs::read(&path) {
                let reader = std::io::Cursor::new(&data);
                if let Ok(mut archive) = zip::ZipArchive::new(reader) {
                    if let Ok(mut entry) = archive.by_name(part_name) {
                        let mut buf = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                        return Some(buf);
                    }
                }
            }
        }
        None
    }

    /// Read an ODP corpus fixture and extract a specific XML part.
    fn corpus_odp_xml(part_name: &str) -> Option<Vec<u8>> {
        let patterns = [
            "cjk-text-odp-001.odp",
            "comments-annotations-odp-001.odp",
            "custom-styles-odp-001.odp",
            "large-odp-001.odp",
            "rtl-text-odp-001.odp",
        ];

        for pattern in &patterns {
            let path = format!("{}/corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), pattern);
            if let Ok(data) = std::fs::read(&path) {
                let reader = std::io::Cursor::new(&data);
                if let Ok(mut archive) = zip::ZipArchive::new(reader) {
                    if let Ok(mut entry) = archive.by_name(part_name) {
                        let mut buf = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                        return Some(buf);
                    }
                }
            }
        }
        None
    }

    /// Helper: convert document bytes to String safely.
    fn doc_bytes_to_str(result: &super::ApplyResult) -> String {
        let bytes = result.document.to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Helper: find all positions of a byte pattern in source.
    fn find_all_positions_raw(source: &[u8], pattern: &[u8]) -> Vec<usize> {
        let mut positions = Vec::new();
        if pattern.is_empty() {
            return positions;
        }
        for i in 0..=source.len().saturating_sub(pattern.len()) {
            if source[i..i + pattern.len()] == pattern[..] {
                positions.push(i);
            }
        }
        positions
    }

    // ------------------------------------------------------------------
    // Synthetic fixture tests
    // ------------------------------------------------------------------

    /// test_replace_text_simple — replace "hello" with "world" in a simple text node.
    #[test]
    fn test_replace_text_simple() {
        let xml = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>Hello world</text:p></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let text_to_replace = "Hello";
        let text_start = xml
            .windows(text_to_replace.len())
            .position(|w| w == text_to_replace.as_bytes())
            .expect("find text");
        let text_span = ByteSpan {
            offset: text_start,
            length: text_to_replace.len(),
        };

        let mutations = vec![SpanMutation {
            span: text_span,
            kind: MutationKind::ReplaceText("World".to_string()),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = result.document.to_bytes();
        let expected = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>World world</text:p></office:document-content>";
        assert_eq!(out, expected, "text should be replaced");
        assert_eq!(result.changed_spans.len(), 1);
    }

    /// test_insert_node_at_offset — insert a paragraph before existing content.
    #[test]
    fn test_insert_node_at_offset() {
        let xml = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>Existing</text:p></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let insert_before = find_all_positions_raw(xml, b"<text:p>")[0];

        let new_paragraph = XmlNode::Element {
            name: TagName::local("p"),
            attributes: Default::default(),
            namespaces: vec![],
            children: vec![XmlNode::Text {
                text: "Inserted".to_string(),
                span: ByteSpan {
                    offset: 0,
                    length: 8,
                },
            }],
            span: ByteSpan {
                offset: insert_before,
                length: 0,
            },
        };

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: insert_before,
                length: 0,
            },
            kind: MutationKind::InsertNode(new_paragraph),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(
            out.contains("Inserted"),
            "inserted paragraph should be present"
        );
        assert!(
            out.contains("Existing"),
            "existing content should be preserved"
        );
        assert!(result.changed_spans.len() == 1);
    }

    /// test_delete_node — delete a paragraph element by span.
    #[test]
    fn test_delete_node() {
        let xml = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>Keep</text:p><text:p>Delete</text:p></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let delete_tag_start = xml
            .windows(8)
            .position(|w| w == b"<text:p>")
            .expect("find first <text:p>");

        let second_p_start = xml[delete_tag_start + 8..]
            .windows(8)
            .position(|w| w == b"<text:p>")
            .map(|p| p + delete_tag_start + 8);

        let second_p_start = second_p_start.expect("find second <text:p>");
        let delete_end = second_p_start + 8 + 6 + 8;

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: second_p_start,
                length: delete_end - second_p_start,
            },
            kind: MutationKind::DeleteNode,
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(!out.contains("Delete"), "deleted paragraph should be gone");
        assert!(
            out.contains("Keep"),
            "remaining content should be preserved"
        );
    }

    /// test_set_attribute — add/set an attribute on an element.
    #[test]
    fn test_set_attribute() {
        let xml = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>Hello</text:p></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let p_start = find_all_positions_raw(xml, b"<text:p>")[0];
        let tag_content = &xml[p_start..];
        let tag_end = find_tag_end(tag_content).expect("find tag end");

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: p_start,
                length: tag_end,
            },
            kind: MutationKind::SetAttribute {
                name: TagName::local("style-name"),
                value: "MyStyle".to_string(),
            },
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(
            out.contains("style-name=\"MyStyle\""),
            "attribute should be set"
        );
        assert!(out.contains("Hello"), "text should be preserved");
    }

    /// test_stale_span_rejected — verify that out-of-range span produces ODF-E-002.
    #[test]
    fn test_stale_span_rejected() {
        let xml = b"<root><child>text</child></root>";
        let doc = parse_xml(xml).expect("parse");

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: 99999,
                length: 10,
            },
            kind: MutationKind::ReplaceText("x".to_string()),
        }];

        let result = apply_span_mutations(doc, mutations);
        assert!(result.is_err(), "stale span should be rejected");
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "ODF-E-002"),
            "should be ODF-E-002"
        );
    }

    /// test_multibyte_utf8_span — verify span works with multi-byte UTF-8 (CJK).
    #[test]
    fn test_multibyte_utf8_span() {
        let cjk_text = "你好世界";
        let cjk_bytes = cjk_text.as_bytes();
        assert_eq!(cjk_bytes.len(), 12, "expect 3 bytes per CJK char");

        let xml = format!("<root>{}</root>", cjk_text).into_bytes();
        // xml = "<root>你好世界</root>" = 6 (tag) + 12 (text) + 7 (closing) = 25 bytes
        // "你" is at byte offset 6 (after <root>), length 3
        // "好世界" is at byte offset 9 (6 + 3), length 9
        let doc = parse_xml(&xml).expect("parse");

        let start = 6 + 3; // skip <root> (6) + first char "你" (3 bytes)
        let length = 9; // "好世界" = 9 bytes

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: start,
                length,
            },
            kind: MutationKind::ReplaceText(" replaced ".to_string()),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(
            out.contains("你 replaced "),
            "first char + replacement should remain"
        );
        assert!(!out.contains("好世界"), "replaced text should be gone");
    }

    /// test_right_to_left_text — verify span offsets for RTL text content.
    #[test]
    fn test_right_to_left_text() {
        // Hebrew text: שלום (shalom) = 4 characters, 8 bytes in UTF-8 (2 bytes per Hebrew char).
        let hebrew = "שלום";
        assert_eq!(hebrew.len(), 8, "Hebrew chars are 2 bytes each in UTF-8");

        let xml = format!("<root>{}</root>", hebrew).into_bytes();
        let doc = parse_xml(&xml).expect("parse");

        // Replace the entire Hebrew text with its transliteration.
        let start = 6; // offset after "<root>"
        let length = 8; // 8 bytes for "שלום"

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: start,
                length,
            },
            kind: MutationKind::ReplaceText("shalom".to_string()),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert_eq!(out, "<root>shalom</root>", "RTL text replaced correctly");
    }

    /// test_multiple_mutations_ordered — apply multiple mutations, verify correct result.
    #[test]
    fn test_multiple_mutations_ordered() {
        let xml = b"<root><a>alpha</a><b>beta</b><c>gamma</c></root>";
        let doc = parse_xml(xml).expect("parse");

        let alpha_pos = find_all_positions_raw(xml, b"alpha")[0];
        let beta_pos = find_all_positions_raw(xml, b"beta")[0];
        let gamma_pos = find_all_positions_raw(xml, b"gamma")[0];

        let mutations = vec![
            SpanMutation {
                span: ByteSpan {
                    offset: alpha_pos,
                    length: 5,
                },
                kind: MutationKind::ReplaceText("ALPHA".to_string()),
            },
            SpanMutation {
                span: ByteSpan {
                    offset: beta_pos,
                    length: 4,
                },
                kind: MutationKind::ReplaceText("BETA".to_string()),
            },
            SpanMutation {
                span: ByteSpan {
                    offset: gamma_pos,
                    length: 5,
                },
                kind: MutationKind::ReplaceText("GAMMA".to_string()),
            },
        ];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(out.contains("ALPHA"), "alpha replaced");
        assert!(out.contains("BETA"), "beta replaced");
        assert!(out.contains("GAMMA"), "gamma replaced");
        assert_eq!(result.changed_spans.len(), 3);
    }

    /// test_odf_namespaces_preserved — after mutation, ODF namespace declarations preserved.
    #[test]
    fn test_odf_namespaces_preserved() {
        let xml = b"<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><text:p>Hello</text:p></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let h_pos = xml
            .windows(5)
            .position(|w| w == b"Hello")
            .expect("find Hello");

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: h_pos,
                length: 5,
            },
            kind: MutationKind::ReplaceText("World".to_string()),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(
            out.contains("xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\""),
            "office namespace preserved"
        );
        assert!(
            out.contains("xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\""),
            "text namespace preserved"
        );
        assert!(out.contains("World"), "text replaced");
    }

    // ------------------------------------------------------------------
    // Corpus fixture tests
    // ------------------------------------------------------------------

    /// corpus_content_xml_replace_text — apply a text replacement to a real corpus content.xml.
    #[test]
    fn corpus_content_xml_replace_text() {
        let xml = corpus_odt_xml("content.xml");
        let xml = match xml {
            Some(x) => x,
            None => {
                eprintln!("SKIP: no corpus ODT content.xml found");
                return;
            }
        };

        let doc = parse_xml(&xml).expect("parse corpus content.xml");
        let source = doc.source();

        let find_text = if source.windows(5).any(|w| w == b"Hello") {
            "Hello"
        } else if source.windows(4).any(|w| w == b"test") {
            "test"
        } else if source.windows(4).any(|w| w == b"Test") {
            "Test"
        } else {
            "OK"
        };

        if let Some(pos) = source
            .windows(find_text.len())
            .position(|w| w == find_text.as_bytes())
        {
            let mutations = vec![SpanMutation {
                span: ByteSpan {
                    offset: pos,
                    length: find_text.len(),
                },
                kind: MutationKind::ReplaceText("XXX".to_string()),
            }];

            let result = apply_span_mutations(doc, mutations).expect("apply");
            let out = doc_bytes_to_str(&result);
            assert!(out.contains("XXX"), "replacement should be present");
            assert!(
                out.starts_with("<?xml"),
                "XML declaration should be preserved"
            );
        }
    }

    /// corpus_styles_xml_replace_text — apply a text replacement to a real styles.xml.
    #[test]
    fn corpus_styles_xml_replace_text() {
        let xml = corpus_odt_xml("styles.xml");
        let xml = match xml {
            Some(x) => x,
            None => return,
        };

        let doc = parse_xml(&xml).expect("parse corpus styles.xml");
        let source = doc.source();
        let find_text = if source.windows(7).any(|w| w == b"default") {
            "default"
        } else if source.windows(5).any(|w| w == b"Styles") {
            "Styles"
        } else {
            return;
        };

        if let Some(pos) = source
            .windows(find_text.len())
            .position(|w| w == find_text.as_bytes())
        {
            let mutations = vec![SpanMutation {
                span: ByteSpan {
                    offset: pos,
                    length: find_text.len(),
                },
                kind: MutationKind::ReplaceText("MODIFIED".to_string()),
            }];

            let _ = apply_span_mutations(doc, mutations).expect("apply");
        }
    }

    /// corpus_ods_content_xml_replace_text — apply a text replacement to ODS content.xml.
    #[test]
    fn corpus_ods_content_xml_replace_text() {
        let xml = corpus_ods_xml("content.xml");
        let xml = match xml {
            Some(x) => x,
            None => {
                eprintln!("SKIP: no corpus ODS content.xml found");
                return;
            }
        };

        let doc = parse_xml(&xml).expect("parse corpus ODS content.xml");
        let source = doc.source();

        let find_text = if source.windows(5).any(|w| w == b"Hello") {
            "Hello"
        } else if source.windows(4).any(|w| w == b"test") {
            "test"
        } else {
            "OK"
        };

        if let Some(pos) = source
            .windows(find_text.len())
            .position(|w| w == find_text.as_bytes())
        {
            let mutations = vec![SpanMutation {
                span: ByteSpan {
                    offset: pos,
                    length: find_text.len(),
                },
                kind: MutationKind::ReplaceText("YYY".to_string()),
            }];

            let result = apply_span_mutations(doc, mutations).expect("apply");
            let out = doc_bytes_to_str(&result);
            assert!(out.contains("YYY"), "replacement should be present");
        }
    }

    /// corpus_odp_content_xml_replace_text — apply a text replacement to ODP content.xml.
    #[test]
    fn corpus_odp_content_xml_replace_text() {
        let xml = corpus_odp_xml("content.xml");
        let xml = match xml {
            Some(x) => x,
            None => {
                eprintln!("SKIP: no corpus ODP content.xml found");
                return;
            }
        };

        let doc = parse_xml(&xml).expect("parse corpus ODP content.xml");
        let source = doc.source();

        let find_text = if source.windows(5).any(|w| w == b"Hello") {
            "Hello"
        } else {
            "OK"
        };

        if let Some(pos) = source
            .windows(find_text.len())
            .position(|w| w == find_text.as_bytes())
        {
            let mutations = vec![SpanMutation {
                span: ByteSpan {
                    offset: pos,
                    length: find_text.len(),
                },
                kind: MutationKind::ReplaceText("ZZZ".to_string()),
            }];

            let _ = apply_span_mutations(doc, mutations).expect("apply");
        }
    }

    /// corpus_stale_span_rejected — stale span on corpus document should be rejected.
    #[test]
    fn corpus_stale_span_rejected() {
        let xml = corpus_odt_xml("content.xml");
        let xml = match xml {
            Some(x) => x,
            None => return,
        };

        let doc = parse_xml(&xml).expect("parse");
        let source_len = doc.source().len();

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: source_len + 100,
                length: 10,
            },
            kind: MutationKind::ReplaceText("x".to_string()),
        }];

        let result = apply_span_mutations(doc, mutations);
        assert!(result.is_err(), "stale span on corpus should be rejected");
        let diags = result.unwrap_err();
        assert!(diags.iter().any(|d| d.code == "ODF-E-002"));
    }

    /// corpus_roundtrip — parse, apply no-op mutation, verify parseable output.
    #[test]
    fn corpus_roundtrip() {
        let xml = corpus_odt_xml("content.xml");
        let xml = match xml {
            Some(x) => x,
            None => return,
        };

        let doc = parse_xml(&xml).expect("parse");

        let mutations: Vec<SpanMutation> = vec![];
        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = result.document.to_bytes();
        assert_eq!(out, xml, "no-op should be byte-identical");
    }

    /// test_set_attribute_modify_value — modify existing attribute value.
    #[test]
    fn test_set_attribute_modify_value() {
        let xml = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p style-name=\"Standard\">Hello</text:p></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let p_start = find_all_positions_raw(xml, b"<text:p")[0];
        let tag_content = &xml[p_start..];
        let tag_end = find_tag_end(tag_content).expect("find tag end");

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: p_start,
                length: tag_end,
            },
            kind: MutationKind::SetAttribute {
                name: TagName::local("style-name"),
                value: "ModifiedStyle".to_string(),
            },
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(
            out.contains("style-name=\"ModifiedStyle\""),
            "style-name should be updated"
        );
        assert!(!out.contains("Standard"), "old value should be gone");
    }

    /// test_insert_node_with_namespace — insert a namespaced node.
    #[test]
    fn test_insert_node_with_namespace() {
        let xml = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:body/></office:document-content>";
        let doc = parse_xml(xml).expect("parse");

        let body_start = find_all_positions_raw(xml, b"<office:body/>")[0];

        let ns_uri = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
        let new_p = XmlNode::Element {
            name: TagName {
                prefix: Some("text".to_string()),
                local_name: "p".to_string(),
                namespace_uri: Some(ns_uri.to_string()),
            },
            attributes: Default::default(),
            namespaces: vec![Namespace {
                prefix: "text".to_string(),
                uri: ns_uri.to_string(),
            }],
            children: vec![XmlNode::Text {
                text: "Inserted with ns".to_string(),
                span: ByteSpan {
                    offset: 0,
                    length: 17,
                },
            }],
            span: ByteSpan {
                offset: body_start,
                length: 0,
            },
        };

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: body_start,
                length: 0,
            },
            kind: MutationKind::InsertNode(new_p),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert!(
            out.contains("Inserted with ns"),
            "namespaced node should be inserted"
        );
        assert!(out.contains("office:body"), "original body should remain");
    }

    /// test_empty_mutations — empty mutation list returns original document.
    #[test]
    fn test_empty_mutations() {
        let xml = b"<root>Hello</root>";
        let doc = parse_xml(xml).expect("parse");

        let result = apply_span_mutations(doc, vec![]).expect("apply");
        let out = result.document.to_bytes();
        assert_eq!(out, xml, "empty mutations should be identity");
        assert!(result.changed_spans.is_empty(), "no changed spans");
    }

    /// test_delete_node_single_byte_span — delete a single byte.
    #[test]
    fn test_delete_node_single_byte_span() {
        let xml = b"<root>hello</root>";
        let doc = parse_xml(xml).expect("parse");

        // Delete the 'h' at offset 6 in "<root>hello</root>"
        let h_pos = 6;

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: h_pos,
                length: 1,
            },
            kind: MutationKind::DeleteNode,
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert_eq!(out, "<root>ello</root>", "single byte deleted");
    }

    /// test_replace_text_empty — replace text with empty string.
    #[test]
    fn test_replace_text_empty() {
        let xml = b"<root>Hello World</root>";
        let doc = parse_xml(xml).expect("parse");

        let space_pos = xml.windows(1).position(|w| w == b" ").expect("find space");

        let mutations = vec![SpanMutation {
            span: ByteSpan {
                offset: space_pos,
                length: 1,
            },
            kind: MutationKind::ReplaceText(String::new()),
        }];

        let result = apply_span_mutations(doc, mutations).expect("apply");
        let out = doc_bytes_to_str(&result);
        assert_eq!(out, "<root>HelloWorld</root>", "space removed");
    }

    // ---------------------------------------------------------------------------
    // Helper for corpus text search
    // ---------------------------------------------------------------------------

    /// find_text_in_source — find all occurrences of ASCII text in source bytes.
    fn find_all_positions(source: &[u8], text: &str) -> Vec<usize> {
        find_all_positions_raw(source, text.as_bytes())
    }

    /// test_find_all_positions — verify text position finding.
    #[test]
    fn test_find_all_positions() {
        let src = b"Hello world Hello";
        let positions = find_all_positions(src, "Hello");
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0], 0);
        assert_eq!(positions[1], 12);
    }
}
