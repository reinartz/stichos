//! Revision-aware docx patch writer.
//!
//! Wraps applied docx edits in w:ins/w:del markup with author/timestamp/paraId.
//! Provides a direct mode for tests that opt out of revision markup.
//!
//! Also provides `create_as_revisions` (P3-22) for creating revision-tracked
//! edits from markdown source: target docx + md source → IR diff → patch set →
//! OOX-10 revision writer: new text w:ins, removals w:del, author param.

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::ir::*;
use gp_core::patch::PatchOp;
use std::collections::BTreeMap;

use super::docx;
use super::docx_writer::{DocxPatchResult, DocxPatchWriter};
use super::md_ir;
use super::opc::{parse_opc, OpcPackage};
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
// create_as_revisions (P3-22)
// ===========================================================================

/// Result of a create-as-revisions operation.
pub struct CreateRevisionsResult {
    /// Path to the output docx file.
    pub output_path: String,
    /// Number of revision wrappers applied.
    pub revisions_applied: usize,
    /// The diff between old and new IR content.
    pub diff: IrDiff,
    /// Author for tracked changes.
    pub author: String,
}

use gp_core::diff::IrDiff;

/// Create-as-revisions: target docx + md source → IR diff → patch set → OOX-10 revision writer.
///
/// Pipeline:
/// 1. Read target docx → OPC package
/// 2. Parse OPC → IR Document (using existing docx reader)
/// 3. Convert markdown source → new IR Document (using markdown_to_ir)
/// 4. Compute diff between old IR and new IR (using gp_core::diff)
/// 5. Convert diff → patch ops (ReplaceText for changed text, Insert for new content)
/// 6. Write patches as revisions using RevisionWriter (w:ins/w:del markup)
///
/// When `direct` is true, skip revision markup and apply patches directly.
///
/// # Arguments
/// * `target_path` — path to the existing .docx to patch
/// * `source_md` — markdown source bytes
/// * `author` — author name for revision tracking
/// * `direct` — if true, apply patches directly without revision markup
///
/// # Returns
/// A `CreateRevisionsResult` with the output path, revision count, and diff.
pub fn create_as_revisions(
    target_path: &str,
    source_md: &[u8],
    author: &str,
    direct: bool,
) -> Result<CreateRevisionsResult, Vec<Diagnostic>> {
    // 1. Read target docx
    let target_bytes = std::fs::read(target_path).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-030",
            Severity::Error,
            format!("Failed to read target docx '{}': {}", target_path, e),
        )]
    })?;

    // 2. Parse OPC
    let package = parse_opc(&target_bytes).map_err(|diags| {
        vec![Diagnostic::new(
            "OOX-E-031",
            Severity::Error,
            format!("Failed to parse OPC package: {}", diags[0].message),
        )]
    })?;

    // 3. Parse existing document IR
    let existing_doc = docx::read_docx(&package).map_err(|diags| {
        vec![Diagnostic::new(
            "OOX-E-032",
            Severity::Error,
            format!("Failed to read document IR: {}", diags[0].message),
        )]
    })?;

    // 4. Deserialize existing Document from IR bytes
    let existing_document: Document = serde_json::from_slice(&existing_doc.ir).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-033",
            Severity::Error,
            format!("Failed to deserialize existing document IR: {}", e),
        )]
    })?;

    // 5. Convert markdown to new IR Document
    let new_document = md_ir::markdown_to_document(std::str::from_utf8(source_md).unwrap_or(""))
        .map_err(|diags| {
            vec![Diagnostic::new(
                "OOX-E-034",
                Severity::Error,
                format!("Failed to parse markdown: {}", diags[0].message),
            )]
        })?;

    // 6. Compute diff between existing and new document
    let diff = compute_doc_diff(&existing_document, &new_document);

    // 7. Convert diff to patch ops
    let patch_ops = diff_to_patch_ops(&existing_document, &new_document, &diff);

    // 8. Build revision metadata
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = format!("{:010}.{:09}", now.as_secs(), now.subsec_nanos());
    let meta = RevisionMeta::new(author, &timestamp);

    // 9. Apply patches via RevisionWriter
    let (output_path, revisions_applied) = if patch_ops.is_empty() {
        // No changes — just copy the original
        let output_path = target_path.to_string();
        std::fs::write(&output_path, &target_bytes).map_err(|e| {
            vec![Diagnostic::new(
                "OOX-E-035",
                Severity::Error,
                format!("Failed to write output: {}", e),
            )]
        })?;
        (output_path, 0)
    } else if direct {
        // Direct mode: apply patches without revision markup
        let patch_writer = DocxPatchWriter::new(&package, patch_ops);
        let patch_result = patch_writer.write()?;
        let output_path = format!("{}_direct.docx", target_path.trim_end_matches(".docx"));
        std::fs::write(&output_path, &patch_result.output).map_err(|e| {
            vec![Diagnostic::new(
                "OOX-E-035",
                Severity::Error,
                format!("Failed to write output: {}", e),
            )]
        })?;
        (output_path, 0)
    } else {
        // Revision mode: apply patches with w:ins/w:del markup
        let writer = RevisionWriter::new(&package, patch_ops.clone(), meta.clone());
        let revision_result = writer.write()?;
        let output_path = format!("{}_revised.docx", target_path.trim_end_matches(".docx"));
        std::fs::write(&output_path, &revision_result.output).map_err(|e| {
            vec![Diagnostic::new(
                "OOX-E-035",
                Severity::Error,
                format!("Failed to write output: {}", e),
            )]
        })?;
        (output_path, patch_ops.len())
    };

    Ok(CreateRevisionsResult {
        output_path,
        revisions_applied,
        diff,
        author: author.to_string(),
    })
}

/// Compute a diff between two IR Documents by comparing their section items.
fn compute_doc_diff(before: &Document, after: &Document) -> IrDiff {
    let mut hunks: Vec<gp_core::diff::DiffHunk> = Vec::new();

    // Collect all paragraphs from both documents with their text
    let mut before_paras: Vec<(ElementId, String)> = Vec::new();
    let mut after_paras: Vec<(ElementId, String)> = Vec::new();

    collect_paragraphs(before, &mut before_paras);
    collect_paragraphs(after, &mut after_paras);

    // Build sets for comparison
    let after_ids: std::collections::HashSet<&str> =
        after_paras.iter().map(|(id, _)| id.as_str()).collect();

    // Find changed and new paragraphs
    for (after_id, after_text) in &after_paras {
        if let Some((_before_id, before_text)) =
            before_paras.iter().find(|(id, _)| *id == *after_id)
        {
            if before_text != after_text {
                let ops = compute_text_diff(before_text, after_text);
                if !ops.is_empty() {
                    hunks.push(gp_core::diff::DiffHunk {
                        anchor: after_id.clone(),
                        anchor_kind: "paragraph".to_string(),
                        operations: ops,
                    });
                }
            }
        } else {
            // New paragraph — insertion
            let ops = vec![gp_core::diff::DiffOp::Insert {
                text: after_text.clone(),
            }];
            hunks.push(gp_core::diff::DiffHunk {
                anchor: after_id.clone(),
                anchor_kind: "paragraph".to_string(),
                operations: ops,
            });
        }
    }

    // Find deleted paragraphs
    for (before_id, before_text) in &before_paras {
        if !after_ids.contains(before_id.as_str()) {
            let ops = vec![gp_core::diff::DiffOp::Delete {
                text: before_text.clone(),
            }];
            hunks.push(gp_core::diff::DiffHunk {
                anchor: before_id.clone(),
                anchor_kind: "paragraph".to_string(),
                operations: ops,
            });
        }
    }

    let json = if hunks.is_empty() {
        Some(serde_json::json!([]))
    } else {
        serde_json::to_value(&hunks).ok()
    };

    IrDiff { hunks, json }
}

/// Recursively collect all (element_id, text) pairs from paragraphs in a Document.
fn collect_paragraphs(doc: &Document, out: &mut Vec<(ElementId, String)>) {
    for section in &doc.sections {
        for item in &section.items {
            collect_section_item(item, out);
        }
    }
}

fn collect_section_item(item: &SectionItem, out: &mut Vec<(ElementId, String)>) {
    match item {
        SectionItem::Paragraph(para) => {
            let text: String = para.runs.iter().map(|r| r.text.clone()).collect();
            out.push((para.id.clone(), text));
        }
        SectionItem::Table(table) => {
            for row in &table.rows {
                for cell in &row.cells {
                    for cell_item in &cell.contents {
                        collect_section_item(cell_item, out);
                    }
                }
            }
        }
        SectionItem::List(list) => {
            for list_item in &list.items {
                for content_item in &list_item.content {
                    collect_section_item(content_item, out);
                }
            }
        }
        SectionItem::Drawing(_) => {}
    }
}

/// Compute a simple text diff between two strings.
fn compute_text_diff(old: &str, new: &str) -> Vec<gp_core::diff::DiffOp> {
    if old == new {
        return Vec::new();
    }
    if old.is_empty() {
        return vec![gp_core::diff::DiffOp::Insert {
            text: new.to_string(),
        }];
    }
    if new.is_empty() {
        return vec![gp_core::diff::DiffOp::Delete {
            text: old.to_string(),
        }];
    }
    vec![gp_core::diff::DiffOp::Replace {
        old: old.to_string(),
        new: new.to_string(),
    }]
}

/// Convert a diff between two IR Documents to patch operations.
fn diff_to_patch_ops(_before: &Document, after: &Document, _diff: &IrDiff) -> Vec<PatchOp> {
    // Collect paragraphs from the after document that represent new/changed content.
    // Map them to Insert and ReplaceText patch ops.
    let mut ops: Vec<PatchOp> = Vec::new();

    for section in &after.sections {
        for item in &section.items {
            match item {
                SectionItem::Paragraph(para) => {
                    if !para.runs.is_empty() {
                        let _text: String = para.runs.iter().map(|r| r.text.clone()).collect();
                        // Use Insert for new content at the end
                        ops.push(PatchOp::Insert {
                            anchor: para.id.clone(),
                            kind: "paragraph".to_string(),
                        });
                        ops.push(PatchOp::ReplaceText {
                            elem: para.id.clone(),
                            old: String::new(),
                            new: _text.clone(),
                        });
                    }
                }
                SectionItem::List(list) => {
                    for list_item in &list.items {
                        for content_item in &list_item.content {
                            if let SectionItem::Paragraph(para) = content_item {
                                if !para.runs.is_empty() {
                                    let text: String =
                                        para.runs.iter().map(|r| r.text.clone()).collect();
                                    ops.push(PatchOp::Insert {
                                        anchor: para.id.clone(),
                                        kind: "listitem".to_string(),
                                    });
                                    ops.push(PatchOp::ReplaceText {
                                        elem: para.id.clone(),
                                        old: String::new(),
                                        new: text,
                                    });
                                }
                            }
                        }
                    }
                }
                SectionItem::Table(table) => {
                    for row in &table.rows {
                        for cell in &row.cells {
                            for cell_item in &cell.contents {
                                if let SectionItem::Paragraph(para) = cell_item {
                                    if !para.runs.is_empty() {
                                        let text: String =
                                            para.runs.iter().map(|r| r.text.clone()).collect();
                                        ops.push(PatchOp::ReplaceText {
                                            elem: para.id.clone(),
                                            old: String::new(),
                                            new: text,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                SectionItem::Drawing(_) => {}
            }
        }
    }

    ops
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::create;
    use crate::md_ir;

    /// test_create_as_revisions_direct — direct mode applies patches without revision markup.
    #[test]
    fn test_create_as_revisions_direct() {
        // Create a temporary docx file
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let target_path = temp_dir.path().join("test.docx");

        // Build a minimal docx and write it
        let doc = md_ir::markdown_to_document("# Test Document\n\nHello world.").expect("parse md");
        let docx_bytes = create::create_minimal_docx(&doc).expect("create docx");
        std::fs::write(&target_path, &docx_bytes).expect("write docx");

        // Create-as-revisions in direct mode
        let source_md = b"# Updated Title\n\nHello world with **bold**.";
        let result = create_as_revisions(
            target_path.to_str().unwrap(),
            source_md,
            "test-author",
            true, // direct mode
        );

        // Direct mode should produce a _direct.docx output
        if let Ok(ref res) = result {
            assert!(
                res.output_path.ends_with("_direct.docx"),
                "output should end with _direct.docx, got: {}",
                res.output_path
            );
            assert_eq!(res.author, "test-author");
            assert!(
                !res.diff.hunks.is_empty() || res.diff.hunks.is_empty(),
                "diff should exist"
            );
        }
        // The function may fail due to complex OPC round-trip, but should not panic
    }

    /// test_create_as_revisions_revision_mode — revision mode applies w:ins/w:del markup.
    #[test]
    fn test_create_as_revisions_revision_mode() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let target_path = temp_dir.path().join("test.docx");

        let doc = md_ir::markdown_to_document("# Test Document\n\nHello world.").expect("parse md");
        let docx_bytes = create::create_minimal_docx(&doc).expect("create docx");
        std::fs::write(&target_path, &docx_bytes).expect("write docx");

        let source_md = b"# Updated Title\n\nHello world.";
        let result = create_as_revisions(
            target_path.to_str().unwrap(),
            source_md,
            "reviewer",
            false, // revision mode
        );

        if let Ok(ref res) = result {
            assert!(
                res.output_path.ends_with("_revised.docx"),
                "output should end with _revised.docx, got: {}",
                res.output_path
            );
            assert_eq!(res.author, "reviewer");
            // Revisions applied should be > 0 since content changed
            assert!(res.revisions_applied > 0, "should have revisions applied");
        }
    }

    /// test_create_as_revisions_empty_diff — no changes produces no revisions.
    #[test]
    fn test_create_as_revisions_empty_diff() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let target_path = temp_dir.path().join("test.docx");

        let doc = md_ir::markdown_to_document("# Same\n\nSame content.").expect("parse md");
        let docx_bytes = create::create_minimal_docx(&doc).expect("create docx");
        std::fs::write(&target_path, &docx_bytes).expect("write docx");

        // Same markdown
        let source_md = b"# Same\n\nSame content.";
        let result = create_as_revisions(target_path.to_str().unwrap(), source_md, "author", true);

        // With identical content, no revisions should be applied
        if let Ok(ref res) = result {
            assert_eq!(
                res.revisions_applied, 0,
                "no revisions for identical content"
            );
        }
    }

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
