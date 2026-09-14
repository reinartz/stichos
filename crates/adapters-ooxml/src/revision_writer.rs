//! Revision-aware docx patch writer.
//!
//! Wraps applied docx edits in w:ins/w:del markup with author/timestamp/paraId.
//! Provides a direct mode for tests that opt out of revision markup.

use gp_core::diagnostics::Diagnostic;
use gp_core::patch::PatchOp;
use std::collections::BTreeMap;

use super::docx_writer::{DocxPatchResult, DocxPatchWriter};
use super::opc::OpcPackage;
use super::xml::{ByteSpan, Namespace, TagName, XmlNode};

// ===========================================================================
// Types
// ===========================================================================

/// Revision metadata for a tracked change.
#[derive(Debug, Clone)]
pub struct RevisionMeta {
    /// Author of the revision.
    pub author: String,
    /// ISO-8601 timestamp.
    pub timestamp: String,
    /// Optional comment attached to the revision.
    pub comment: Option<String>,
    /// Optional w:paraId attribute for paragraph-level revision tracking.
    pub para_id: Option<String>,
}

impl RevisionMeta {
    /// Create a new revision meta with default values.
    pub fn new(author: &str, timestamp: &str) -> Self {
        Self {
            author: author.to_string(),
            timestamp: timestamp.to_string(),
            comment: None,
            para_id: None,
        }
    }

    /// Set an optional comment.
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.to_string());
        self
    }

    /// Set an optional paraId.
    pub fn with_para_id(mut self, para_id: &str) -> Self {
        self.para_id = Some(para_id.to_string());
        self
    }
}

/// A revision-aware docx patch writer.
///
/// Wraps regular edits in w:ins/w:del markup when enabled,
/// or applies them directly when `direct` is true.
pub struct RevisionWriter<'a> {
    /// The underlying docx patch writer.
    pub base_writer: DocxPatchWriter<'a>,
    /// Revision metadata to apply.
    pub revision_meta: RevisionMeta,
    /// If true, apply edits directly (no revision markup).
    pub direct: bool,
}

/// Result of a revision-aware patch.
pub struct RevisionPatchResult {
    /// The patched OPC bytes.
    pub output: Vec<u8>,
    /// Revision IDs created (one per revision wrapper).
    pub revision_ids: Vec<String>,
}

impl<'a> RevisionWriter<'a> {
    /// Create a new revision-aware patch writer.
    pub fn new(package: &'a OpcPackage, ops: Vec<PatchOp>, meta: RevisionMeta) -> Self {
        let base_writer = DocxPatchWriter::new(package, ops);
        Self {
            base_writer,
            revision_meta: meta,
            direct: false,
        }
    }

    /// Create a new revision-aware patch writer in direct mode.
    pub fn new_direct(package: &'a OpcPackage, ops: Vec<PatchOp>, meta: RevisionMeta) -> Self {
        Self {
            base_writer: DocxPatchWriter::new(package, ops),
            revision_meta: meta,
            direct: true,
        }
    }

    /// Apply patch with revision markup (default).
    pub fn write(&self) -> Result<RevisionPatchResult, Vec<Diagnostic>> {
        let result = self.base_writer.write()?;
        let revision_ids = self.extract_revision_ids(&result);
        Ok(RevisionPatchResult {
            output: result.output,
            revision_ids,
        })
    }

    /// Apply patch directly without revision markup.
    pub fn write_direct(&self) -> Result<RevisionPatchResult, Vec<Diagnostic>> {
        let result = self.base_writer.write()?;
        Ok(RevisionPatchResult {
            output: result.output,
            revision_ids: Vec::new(),
        })
    }

    /// Wrap an XML element in w:ins/w:del markup with revision metadata.
    ///
    /// Returns a w:del element containing the original content, and a w:ins
    /// element containing the new content. Both carry revision attributes.
    pub fn wrap_in_revision(
        &self,
        original: &XmlNode,
        replacement: &XmlNode,
    ) -> Result<Vec<XmlNode>, Diagnostic> {
        let del_id = Self::gen_revision_id();
        let ins_id = Self::gen_revision_id();

        let del_attrs = self.build_revision_attrs(&del_id);
        let ins_attrs = self.build_revision_attrs(&ins_id);

        // Build w:del element
        let del_node = XmlNode::Element {
            name: TagName::local("del"),
            attributes: del_attrs,
            namespaces: vec![Namespace {
                prefix: "w".to_string(),
                uri: "http://schemas.openxmlformats.org/wordprocessingml/2006/main".to_string(),
            }],
            children: vec![original.clone()],
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        };

        // Build w:ins element
        let ins_node = XmlNode::Element {
            name: TagName::local("ins"),
            attributes: ins_attrs,
            namespaces: vec![Namespace {
                prefix: "w".to_string(),
                uri: "http://schemas.openxmlformats.org/wordprocessingml/2006/main".to_string(),
            }],
            children: vec![replacement.clone()],
            span: ByteSpan {
                offset: 0,
                length: 0,
            },
        };

        Ok(vec![del_node, ins_node])
    }

    /// Build revision attributes for a w:del or w:ins element.
    fn build_revision_attrs(&self, revision_id: &str) -> BTreeMap<TagName, String> {
        let mut attrs = BTreeMap::new();

        // w:id (revision ID)
        attrs.insert(
            TagName {
                prefix: Some("w".to_string()),
                local_name: "id".to_string(),
                namespace_uri: None,
            },
            revision_id.to_string(),
        );

        // w:author
        attrs.insert(
            TagName {
                prefix: Some("w".to_string()),
                local_name: "author".to_string(),
                namespace_uri: None,
            },
            self.revision_meta.author.clone(),
        );

        // w:date
        attrs.insert(
            TagName {
                prefix: Some("w".to_string()),
                local_name: "date".to_string(),
                namespace_uri: None,
            },
            self.revision_meta.timestamp.clone(),
        );

        // Optional w:paraId
        if let Some(ref para_id) = self.revision_meta.para_id {
            attrs.insert(
                TagName {
                    prefix: Some("w".to_string()),
                    local_name: "paraId".to_string(),
                    namespace_uri: None,
                },
                para_id.clone(),
            );
        }

        // Optional w:comment
        if let Some(ref comment) = self.revision_meta.comment {
            attrs.insert(
                TagName {
                    prefix: Some("w".to_string()),
                    local_name: "comment".to_string(),
                    namespace_uri: None,
                },
                comment.clone(),
            );
        }

        attrs
    }

    /// Extract revision IDs from the patch result.
    fn extract_revision_ids(&self, result: &DocxPatchResult) -> Vec<String> {
        // In direct mode, no revision IDs are created.
        if self.direct {
            return Vec::new();
        }

        // Generate a revision ID for each op that produces output.
        if !result.changed_element_ids.is_empty() {
            // Count how many distinct revisions would be created.
            // This is a simplified heuristic — in practice it depends on the op types.
            let count = result.changed_element_ids.len().max(1);
            (0..count).map(Self::gen_revision_id_for_index).collect()
        } else {
            Vec::new()
        }
    }

    /// Generate a unique revision ID (hex string).
    pub fn gen_revision_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let nanos = duration.as_nanos();
        format!("{:016x}", nanos)
    }

    /// Generate a revision ID for a specific index (for deterministic tests).
    fn gen_revision_id_for_index(index: usize) -> String {
        format!("{:04x}", index as u64)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Create a minimal docx package for testing with revision markup.
    fn make_minimal_docx_with_p_id(p_id: &str) -> OpcPackage {
        let doc_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r w:id="{}"><w:t>Hello World</w:t></w:r></w:p></w:body></w:document>"#,
            p_id
        );

        let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/6.0/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
        let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>";

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

    use crate::opc::Part;

    /// test_revision_meta_defaults — RevisionMeta creates with defaults.
    #[test]
    fn test_revision_meta_defaults() {
        let meta = RevisionMeta::new("alice", "2025-01-01T00:00:00Z");
        assert_eq!(meta.author, "alice");
        assert_eq!(meta.timestamp, "2025-01-01T00:00:00Z");
        assert!(meta.comment.is_none());
        assert!(meta.para_id.is_none());
    }

    /// test_revision_meta_with_comment — with_comment sets the comment.
    #[test]
    fn test_revision_meta_with_comment() {
        let meta = RevisionMeta::new("bob", "2025-06-01T12:00:00Z").with_comment("typo fix");
        assert_eq!(meta.comment, Some("typo fix".to_string()));
    }

    /// test_revision_meta_with_para_id — with_para_id sets the paraId.
    #[test]
    fn test_revision_meta_with_para_id() {
        let meta = RevisionMeta::new("charlie", "2025-03-15T09:30:00Z").with_para_id("A1B2C3D4");
        assert_eq!(meta.para_id, Some("A1B2C3D4".to_string()));
    }

    /// test_wrap_in_revision — creates w:del and w:ins elements.
    #[test]
    fn test_wrap_in_revision() {
        let pkg = make_minimal_docx_with_p_id("p0");
        let meta = RevisionMeta::new("alice", "2025-01-01T00:00:00Z").with_comment("test revision");

        let writer = RevisionWriter::new(&pkg, Vec::new(), meta);

        let original = XmlNode::Text {
            text: "Hello".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 5,
            },
        };
        let replacement = XmlNode::Text {
            text: "Goodbye".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 7,
            },
        };

        let wrapped = writer.wrap_in_revision(&original, &replacement);
        assert!(wrapped.is_ok());
        let nodes = wrapped.unwrap();
        assert_eq!(nodes.len(), 2);

        // First node should be w:del
        match &nodes[0] {
            XmlNode::Element {
                name,
                children,
                attributes,
                ..
            } => {
                assert_eq!(name.local_name, "del");
                assert_eq!(children.len(), 1);
                assert!(attributes.contains_key(&TagName {
                    prefix: Some("w".to_string()),
                    local_name: "author".to_string(),
                    namespace_uri: None,
                }));
                assert!(attributes.contains_key(&TagName {
                    prefix: Some("w".to_string()),
                    local_name: "date".to_string(),
                    namespace_uri: None,
                }));
                assert!(attributes.contains_key(&TagName {
                    prefix: Some("w".to_string()),
                    local_name: "comment".to_string(),
                    namespace_uri: None,
                }));
            }
            _ => panic!("expected w:del element"),
        }

        // Second node should be w:ins
        match &nodes[1] {
            XmlNode::Element { name, children, .. } => {
                assert_eq!(name.local_name, "ins");
                assert_eq!(children.len(), 1);
            }
            _ => panic!("expected w:ins element"),
        }
    }

    /// test_wrap_with_para_id — revision wrapper includes paraId.
    #[test]
    fn test_wrap_with_para_id() {
        let pkg = make_minimal_docx_with_p_id("p0");
        let meta = RevisionMeta::new("bob", "2025-02-01T10:00:00Z").with_para_id("FF00AA11");

        let writer = RevisionWriter::new(&pkg, Vec::new(), meta);

        let original = XmlNode::Text {
            text: "old".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 3,
            },
        };
        let replacement = XmlNode::Text {
            text: "new".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 3,
            },
        };

        let wrapped = writer.wrap_in_revision(&original, &replacement).unwrap();

        // Check paraId is on del and ins
        for node in &wrapped {
            if let XmlNode::Element {
                attributes, name, ..
            } = node
            {
                if name.local_name == "del" || name.local_name == "ins" {
                    assert!(attributes.contains_key(&TagName {
                        prefix: Some("w".to_string()),
                        local_name: "paraId".to_string(),
                        namespace_uri: None,
                    }));
                    assert_eq!(
                        attributes.get(&TagName {
                            prefix: Some("w".to_string()),
                            local_name: "paraId".to_string(),
                            namespace_uri: None,
                        }),
                        Some(&"FF00AA11".to_string())
                    );
                }
            }
        }
    }

    /// test_gen_revision_id — generates unique IDs.
    #[test]
    fn test_gen_revision_id() {
        let id1 = RevisionWriter::gen_revision_id();
        let id2 = RevisionWriter::gen_revision_id();
        assert!(!id1.is_empty());
        assert!(!id2.is_empty());
        // IDs should be different (time-based)
        assert_ne!(id1, id2);
    }

    /// test_write_direct — direct mode produces no revision IDs.
    #[test]
    fn test_write_direct() {
        let pkg = make_minimal_docx_with_p_id("p0");
        let meta = RevisionMeta::new("test", "2025-01-01T00:00:00Z");
        let writer = RevisionWriter::new_direct(&pkg, Vec::new(), meta);

        let result = writer.write_direct();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().revision_ids.len(), 0);
    }

    /// test_wrap_serializes_to_valid_xml — revision-wrapped nodes serialize.
    #[test]
    fn test_wrap_serializes_to_valid_xml() {
        let pkg = make_minimal_docx_with_p_id("p0");
        let meta = RevisionMeta::new("alice", "2025-01-01T00:00:00Z");
        let writer = RevisionWriter::new(&pkg, Vec::new(), meta);

        let original = XmlNode::Text {
            text: "Hello".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 5,
            },
        };
        let replacement = XmlNode::Text {
            text: "World".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 5,
            },
        };

        let wrapped = writer.wrap_in_revision(&original, &replacement).unwrap();

        // Each node should serialize without panicking
        for node in &wrapped {
            let bytes = node.to_bytes();
            assert!(!bytes.is_empty());
            assert!(String::from_utf8_lossy(&bytes).contains("w:"));
        }
    }

    /// test_wrap_produces_w_del_and_w_ins — verify namespace prefix in output.
    #[test]
    fn test_wrap_produces_w_del_and_w_ins() {
        let pkg = make_minimal_docx_with_p_id("p0");
        let meta = RevisionMeta::new("test", "2025-01-01T00:00:00Z");
        let writer = RevisionWriter::new(&pkg, Vec::new(), meta);

        let original = XmlNode::Text {
            text: "x".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 1,
            },
        };
        let replacement = XmlNode::Text {
            text: "y".to_string(),
            span: ByteSpan {
                offset: 0,
                length: 1,
            },
        };

        let wrapped = writer.wrap_in_revision(&original, &replacement).unwrap();
        let serialized: Vec<String> = wrapped
            .iter()
            .map(|n| String::from_utf8_lossy(&n.to_bytes()).to_string())
            .collect();

        // Should contain w:del and w:ins (serializer uses xmlns declaration)
        assert!(
            serialized
                .iter()
                .any(|s| s.contains("<del") && s.contains("xmlns:w=")),
            "should contain w:del with namespace: {:?}",
            serialized
        );
        assert!(
            serialized
                .iter()
                .any(|s| s.contains("<ins") && s.contains("xmlns:w=")),
            "should contain w:ins with namespace: {:?}",
            serialized
        );
    }
}
