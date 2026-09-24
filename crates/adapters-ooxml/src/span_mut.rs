// SPDX-License-Identifier: MIT OR Apache-2.0
//! Span-based mutation primitives for XML documents.
//!
//! Provides right-to-left span mutations with pre-verification: every mutation
//! re-verifies that the span's content still matches the source bytes before
//! applying. Stale-span input is rejected with the `OOX-E-002` diagnostic code.

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use std::collections::BTreeMap;

use crate::xml::ByteSpan;
use crate::xml::{TagName, XmlDocument, XmlNode};

// ---------------------------------------------------------------------------
// Mutation types
// ---------------------------------------------------------------------------

/// A span-based mutation with pre-verification.
#[derive(Debug, Clone)]
pub struct SpanMutation {
    pub span: ByteSpan,
    pub kind: MutationKind,
}

/// The kind of mutation to apply at a given span.
#[derive(Debug, Clone)]
pub enum MutationKind {
    ReplaceText(String),
    InsertNode(XmlNode),
    DeleteNode,
    SetAttribute { name: TagName, value: String },
    WrapDelete,
}

/// Result of applying mutations.
#[derive(Debug, Clone)]
pub struct SpanMutationResult {
    pub document: XmlDocument,
    pub changed_spans: Vec<ByteSpan>,
    pub byte_diff: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_span(node: &XmlNode) -> &ByteSpan {
    match node {
        XmlNode::Element { span, .. }
        | XmlNode::Text { span, .. }
        | XmlNode::ProcessingInstruction { span, .. }
        | XmlNode::Comment { span, .. }
        | XmlNode::CData { span, .. } => span,
    }
}

fn find_node_by_span<'a>(node: &'a XmlNode, span: &ByteSpan) -> Option<&'a XmlNode> {
    let ns = node_span(node);
    if ns.offset == span.offset && ns.length == span.length {
        return Some(node);
    }
    if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let Some(found) = find_node_by_span(child, span) {
                return Some(found);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

/// Find the XML node at a given byte span.
pub fn find_node_at_span<'a>(
    document: &'a XmlDocument,
    span: &'a ByteSpan,
) -> Result<&'a XmlNode, Diagnostic> {
    find_node_by_span(&document.root, span).ok_or_else(|| {
        Diagnostic::new(
            "OOX-E-002",
            Severity::Error,
            format!(
                "No node found at span offset={} length={}",
                span.offset, span.length
            ),
        )
    })
}

/// Verify that a span's content matches the source bytes.
pub fn verify_span(document: &XmlDocument, span: &ByteSpan) -> Result<(), Diagnostic> {
    let node = find_node_by_span(&document.root, span).ok_or_else(|| {
        Diagnostic::new(
            "OOX-E-002",
            Severity::Error,
            format!(
                "No node found at span offset={} length={}",
                span.offset, span.length
            ),
        )
    })?;

    let node_bytes = node.to_bytes();
    let source = document.source();
    let end = span.offset + span.length;
    if end > source.len() {
        return Err(Diagnostic::new(
            "OOX-E-002",
            Severity::Error,
            format!(
                "Span offset={}+length={} exceeds source length={}",
                span.offset,
                span.length,
                source.len()
            ),
        ));
    }
    let actual_bytes = &source[span.offset..end];
    if node_bytes != actual_bytes {
        return Err(Diagnostic::new(
            "OOX-E-002",
            Severity::Error,
            format!(
                "Span at offset={} is stale: node content does not match source bytes",
                span.offset
            ),
        ));
    }
    Ok(())
}

/// Apply a single span mutation, verifying the span first.
pub fn apply_span_mutation(
    document: &XmlDocument,
    mutation: &SpanMutation,
) -> Result<SpanMutationResult, Diagnostic> {
    verify_span(document, &mutation.span)?;

    let mut doc = document.clone();
    let _original_len = doc.source().len();

    let changed_span = apply_mut_to_node(&mut doc.root, &mutation.span, &mutation.kind)?;

    let mutated_bytes = doc.root.to_bytes();
    doc.set_source(mutated_bytes);
    reindex_spans(&mut doc.root, 0);

    let new_len = doc.source().len();

    Ok(SpanMutationResult {
        document: doc,
        changed_spans: vec![changed_span],
        byte_diff: new_len as i64 - _original_len as i64,
    })
}

fn apply_mut_to_node(
    node: &mut XmlNode,
    span: &ByteSpan,
    kind: &MutationKind,
) -> Result<ByteSpan, Diagnostic> {
    let ns = node_span(node);
    if ns.offset != span.offset || ns.length != span.length {
        if let XmlNode::Element { children, .. } = node {
            for child in children {
                if let Some(result) = apply_mut_recursive(child, span, kind) {
                    return Ok(result);
                }
            }
        }
        return Err(Diagnostic::new(
            "OOX-E-002",
            Severity::Error,
            format!("No matching node at span offset={}", span.offset),
        ));
    }

    let old_span = node_span(node).clone();

    match kind {
        MutationKind::ReplaceText(new_text) => match node {
            XmlNode::Text { text, span: s } => {
                *text = new_text.clone();
                *s = ByteSpan {
                    offset: s.offset,
                    length: new_text.len(),
                };
                Ok(ByteSpan {
                    offset: s.offset,
                    length: new_text.len(),
                })
            }
            _ => Err(Diagnostic::new(
                "OOX-E-002",
                Severity::Error,
                format!(
                    "Cannot ReplaceText on non-Text node at offset={}",
                    span.offset
                ),
            )),
        },
        MutationKind::InsertNode(new_node) => match node {
            XmlNode::Element { children, .. } => {
                let inserted = set_span_offset(&mut new_node.clone(), span.offset);
                children.push(inserted);
                Ok(old_span)
            }
            _ => Err(Diagnostic::new(
                "OOX-E-002",
                Severity::Error,
                format!("Cannot InsertNode at non-Element offset={}", span.offset),
            )),
        },
        MutationKind::DeleteNode => match node {
            XmlNode::Element { children, .. } => {
                let pos = children.iter().position(|c| {
                    let cs = node_span(c);
                    cs.offset == span.offset && cs.length == span.length
                });
                if let Some(idx) = pos {
                    children.remove(idx);
                    Ok(old_span)
                } else {
                    Err(Diagnostic::new(
                        "OOX-E-002",
                        Severity::Error,
                        format!("DeleteNode: no matching child at offset={}", span.offset),
                    ))
                }
            }
            _ => Err(Diagnostic::new(
                "OOX-E-002",
                Severity::Error,
                format!("Cannot DeleteNode on non-Element offset={}", span.offset),
            )),
        },
        MutationKind::SetAttribute { name, value } => match node {
            XmlNode::Element { attributes, .. } => {
                attributes.insert(name.clone(), value.clone());
                Ok(old_span)
            }
            _ => Err(Diagnostic::new(
                "OOX-E-002",
                Severity::Error,
                format!("Cannot SetAttribute on non-Element offset={}", span.offset),
            )),
        },
        MutationKind::WrapDelete => {
            // Wrap this node in a w:del element.
            // Replace this node's children with the w:del wrapper containing the old content.
            match node {
                XmlNode::Element {
                    name,
                    attributes,
                    namespaces,
                    children,
                    span: s,
                } => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default();
                    let del_id = format!("{:016x}", now.as_nanos());
                    let timestamp = format!("{:010}.{:09}", now.as_secs(), now.subsec_nanos());
                    let mut del_attrs = BTreeMap::new();
                    del_attrs.insert(
                        TagName {
                            prefix: Some("w".into()),
                            local_name: "id".into(),
                            namespace_uri: None,
                        },
                        del_id.clone(),
                    );
                    del_attrs.insert(
                        TagName {
                            prefix: Some("w".into()),
                            local_name: "author".into(),
                            namespace_uri: None,
                        },
                        "stichos".into(),
                    );
                    del_attrs.insert(
                        TagName {
                            prefix: Some("w".into()),
                            local_name: "date".into(),
                            namespace_uri: None,
                        },
                        timestamp,
                    );
                    let del_node = XmlNode::Element {
                        name: TagName::local("w:del"),
                        attributes: del_attrs,
                        namespaces: vec![super::xml::Namespace {
                            prefix: "w".into(),
                            uri: "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                                .into(),
                        }],
                        children: vec![XmlNode::Element {
                            name: name.clone(),
                            attributes: attributes.clone(),
                            namespaces: namespaces.clone(),
                            children: children.clone(),
                            span: s.clone(),
                        }],
                        span: s.clone(),
                    };
                    // Replace current node with the del wrapper
                    *node = del_node;
                    Ok(old_span)
                }
                _ => Err(Diagnostic::new(
                    "OOX-E-002",
                    Severity::Error,
                    format!("Cannot WrapDelete on non-Element offset={}", span.offset),
                )),
            }
        }
    }
}

fn apply_mut_recursive(
    node: &mut XmlNode,
    span: &ByteSpan,
    kind: &MutationKind,
) -> Option<ByteSpan> {
    let ns = node_span(node);
    if ns.offset == span.offset && ns.length == span.length {
        apply_mut_to_node(node, span, kind).ok()
    } else if let XmlNode::Element { children, .. } = node {
        for child in children {
            if let Some(result) = apply_mut_recursive(child, span, kind) {
                return Some(result);
            }
        }
        None
    } else {
        None
    }
}

fn set_span_offset(node: &mut XmlNode, offset: usize) -> XmlNode {
    let cloned = node.clone();
    let bytes_len = cloned.to_bytes().len();

    match node {
        XmlNode::Text { text, span, .. } => {
            *span = ByteSpan {
                offset,
                length: text.len(),
            };
        }
        XmlNode::CData { text, span, .. } => {
            *span = ByteSpan {
                offset,
                length: text.len(),
            };
        }
        XmlNode::Comment { text, span, .. } => {
            let blen = text.len() + 8;
            *span = ByteSpan {
                offset,
                length: blen,
            };
        }
        XmlNode::ProcessingInstruction { target, data, span } => {
            let blen = 4 + target.len() + 1 + data.len() + 2;
            *span = ByteSpan {
                offset,
                length: blen,
            };
        }
        XmlNode::Element { children, span, .. } => {
            *span = ByteSpan {
                offset,
                length: bytes_len,
            };
            let mut cur = offset;
            for child in children {
                set_span_offset(child, cur);
                cur = node_span(child).end();
            }
        }
    }

    cloned
}

fn reindex_spans(node: &mut XmlNode, current_offset: usize) -> usize {
    let start = current_offset;
    let cloned = node.clone();
    let bytes_len = cloned.to_bytes().len();

    match node {
        XmlNode::Text { text, span, .. } => {
            *span = ByteSpan {
                offset: start,
                length: text.len(),
            };
            start + text.len()
        }
        XmlNode::CData { text, span, .. } => {
            *span = ByteSpan {
                offset: start,
                length: text.len(),
            };
            start + text.len()
        }
        XmlNode::Comment { text, span, .. } => {
            let blen = text.len() + 8;
            *span = ByteSpan {
                offset: start,
                length: blen,
            };
            start + blen
        }
        XmlNode::ProcessingInstruction { target, data, span } => {
            let blen = 4 + target.len() + 1 + data.len() + 2;
            *span = ByteSpan {
                offset: start,
                length: blen,
            };
            start + blen
        }
        XmlNode::Element { children, span, .. } => {
            *span = ByteSpan {
                offset: start,
                length: bytes_len,
            };
            let mut offset = start;
            for child in children {
                offset = reindex_spans(child, offset);
            }
            offset
        }
    }
}

/// Apply multiple span mutations in right-to-left order (highest offset first).
pub fn apply_span_mutations(
    document: &XmlDocument,
    mut mutations: Vec<SpanMutation>,
) -> Result<SpanMutationResult, Diagnostic> {
    mutations.sort_by_key(|m| std::cmp::Reverse(m.span.offset));

    let mut doc = document.clone();
    let _original_len = doc.source().len();
    let mut total_byte_diff: i64 = 0;
    let mut all_changed_spans: Vec<ByteSpan> = Vec::new();

    for mutation in &mutations {
        verify_span(&doc, &mutation.span)?;
        let result = apply_span_mutation(&doc, mutation)?;
        doc = result.document;
        total_byte_diff += result.byte_diff;
        all_changed_spans.extend(result.changed_spans);
    }

    Ok(SpanMutationResult {
        document: doc,
        changed_spans: all_changed_spans,
        byte_diff: total_byte_diff,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_doc(xml: &str) -> XmlDocument {
        crate::xml::parse_xml(xml.as_bytes()).expect("valid XML")
    }

    #[test]
    fn test_verify_span_valid() {
        let doc = make_doc("<root><child>Hello</child></root>");
        // Walk the tree to find the Text node and get its span
        let text_span = find_text_span(&doc.root);
        assert!(text_span.is_some(), "should find text node");
        let span = text_span.unwrap();
        assert!(verify_span(&doc, &span).is_ok());
    }

    fn find_text_span(node: &XmlNode) -> Option<ByteSpan> {
        match node {
            XmlNode::Text { span, text } => {
                if !text.is_empty() {
                    Some(span.clone())
                } else {
                    None
                }
            }
            XmlNode::Element { children, .. } => {
                for child in children {
                    if let Some(s) = find_text_span(child) {
                        return Some(s);
                    }
                }
                None
            }
            _ => None,
        }
    }

    #[test]
    fn test_verify_span_no_node_found() {
        let doc = make_doc("<root/>");
        let span = ByteSpan {
            offset: 999,
            length: 1,
        };
        assert!(verify_span(&doc, &span).is_err());
        assert_eq!(verify_span(&doc, &span).unwrap_err().code, "OOX-E-002");
    }

    #[test]
    fn test_find_node_at_span() {
        let doc = make_doc("<root><child>Hello</child></root>");
        let text_span = find_text_span(&doc.root).expect("should find text node");
        let node = find_node_at_span(&doc, &text_span);
        assert!(node.is_ok());
        assert!(matches!(node.unwrap(), XmlNode::Text { .. }));
    }

    #[test]
    fn test_replace_text_mutation() {
        let doc = make_doc("<root>Hello</root>");
        // Find the text node span
        let text_span = find_text_span(&doc.root).expect("should find text node");
        let mutation = SpanMutation {
            span: text_span.clone(),
            kind: MutationKind::ReplaceText("World".to_string()),
        };
        let result = apply_span_mutation(&doc, &mutation);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.byte_diff, 0);
        let source = String::from_utf8_lossy(r.document.source());
        assert!(source.contains("World"));
        assert!(!source.contains("Hello"));
    }

    #[test]
    fn test_set_attribute_mutation() {
        let doc = make_doc("<root id=\"1\"/>");
        // Find root element span
        let root_span = find_element_span(&doc.root).expect("should find root element");
        let mutation = SpanMutation {
            span: root_span.clone(),
            kind: MutationKind::SetAttribute {
                name: TagName::local("new-attr"),
                value: "new-val".to_string(),
            },
        };
        assert!(apply_span_mutation(&doc, &mutation).is_ok());
    }

    fn find_element_span(node: &XmlNode) -> Option<ByteSpan> {
        if let XmlNode::Element { span, .. } = node {
            Some(span.clone())
        } else {
            None
        }
    }

    #[test]
    fn test_replace_text_roundtrip() {
        let doc = make_doc("<root>Hello</root>");
        let text_span = find_text_span(&doc.root).expect("should find text node");
        let mutation = SpanMutation {
            span: text_span.clone(),
            kind: MutationKind::ReplaceText("World".to_string()),
        };
        let result = apply_span_mutation(&doc, &mutation).unwrap();
        assert!(
            crate::xml::parse_xml(result.document.source()).is_ok(),
            "mutated doc should be parseable"
        );
    }

    #[test]
    fn test_span_verification_valid() {
        let doc = make_doc("<root><child>Hello World</child></root>");
        let text_span = find_text_span(&doc.root).expect("should find text node");
        assert!(verify_span(&doc, &text_span).is_ok());
    }

    #[test]
    fn test_span_verification_stale() {
        let doc = make_doc("<root>Hello</root>");
        let span = ByteSpan {
            offset: 6,
            length: 5,
        };
        assert!(verify_span(&doc, &span).is_ok());

        let stale = ByteSpan {
            offset: 100,
            length: 1,
        };
        assert!(verify_span(&doc, &stale).is_err());
    }
}
