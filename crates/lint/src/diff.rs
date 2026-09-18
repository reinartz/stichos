//! IR structural diff and XML part diff for stichos.
//!
//! Anchored to element IDs (P4/C2) for stable references across edits.

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::Document;
use gp_core::ir::IRDocument;

/// A diff operation on an IR element.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DiffOp {
    /// Element exists in original but not in new version.
    Removed { id: String, kind: String },
    /// Element exists in new version but not in original.
    Added { id: String, kind: String },
    /// Element exists in both but content changed.
    Modified { id: String, kind: String },
}

impl DiffOp {
    pub fn op_type(&self) -> &str {
        match self {
            DiffOp::Removed { .. } => "removed",
            DiffOp::Added { .. } => "added",
            DiffOp::Modified { .. } => "modified",
        }
    }
}

/// A structural diff between two IR documents.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IrDiff {
    /// Diff operations, ordered by element ID.
    pub ops: Vec<DiffOp>,
    /// Total operations.
    pub total_ops: usize,
    /// Number of removed elements.
    pub removed: usize,
    /// Number of added elements.
    pub added: usize,
    /// Number of modified elements.
    pub modified: usize,
    /// Whether the documents are structurally identical.
    pub identical: bool,
}

impl IrDiff {
    pub fn empty() -> Self {
        Self {
            ops: Vec::new(),
            total_ops: 0,
            removed: 0,
            added: 0,
            modified: 0,
            identical: true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Collect all element IDs and kinds from a Document (flattened).
fn collect_ids_doc(doc: &Document) -> Vec<(String, String)> {
    let mut ids = Vec::new();
    collect_ids_from_document(doc, &mut ids);
    ids.sort_by(|a, b| a.0.cmp(&b.0));
    ids
}

fn collect_ids_from_document(doc: &Document, ids: &mut Vec<(String, String)>) {
    ids.push(("document".to_string(), "Document".to_string()));
    for section in &doc.sections {
        let kind_str = match &section.kind {
            gp_core::ir::SectionKind::Body => "Section:Body".to_string(),
            gp_core::ir::SectionKind::Header => "Section:Header".to_string(),
            gp_core::ir::SectionKind::Footer => "Section:Footer".to_string(),
            gp_core::ir::SectionKind::Footnotes => "Section:Footnotes".to_string(),
            gp_core::ir::SectionKind::Endnotes => "Section:Endnotes".to_string(),
        };
        ids.push((section.id.as_str().to_string(), kind_str));
        for item in &section.items {
            collect_ids_from_item(item, ids);
        }
    }
}

fn collect_ids_from_item(item: &gp_core::ir::SectionItem, ids: &mut Vec<(String, String)>) {
    match item {
        gp_core::ir::SectionItem::Paragraph(p) => {
            ids.push((p.id.as_str().to_string(), "Paragraph".to_string()));
            for run in &p.runs {
                ids.push((run.id.as_str().to_string(), "Run".to_string()));
            }
        }
        gp_core::ir::SectionItem::Table(t) => {
            ids.push((t.id.as_str().to_string(), "Table".to_string()));
            for row in &t.rows {
                for (ci, cell) in row.cells.iter().enumerate() {
                    ids.push((cell.id.as_str().to_string(), "Cell".to_string()));
                    for content in &cell.contents {
                        collect_ids_from_item(content, ids);
                    }
                    let _ = ci;
                }
            }
        }
        gp_core::ir::SectionItem::Drawing(d) => {
            ids.push((d.id.as_str().to_string(), "Drawing".to_string()));
        }
        gp_core::ir::SectionItem::List(l) => {
            ids.push((l.id.as_str().to_string(), "List".to_string()));
            for item in &l.items {
                ids.push((item.id.as_str().to_string(), "ListItem".to_string()));
                for content in &item.content {
                    collect_ids_from_item(content, ids);
                }
            }
        }
    }
}

/// Compare two IR documents and produce a structural diff.
///
/// Compares by element ID: elements that share an ID are compared for content changes.
/// Elements with different IDs are treated as added/removed (stable even on moves).
pub fn diff_documents(a: &Document, b: &Document) -> IrDiff {
    let ids_a = collect_ids_doc(a);
    let ids_b = collect_ids_doc(b);

    let mut ops = Vec::new();
    let a_map: std::collections::HashMap<_, _> = ids_a
        .iter()
        .map(|(id, kind)| (id.clone(), kind.clone()))
        .collect();
    let b_map: std::collections::HashMap<_, _> = ids_b
        .iter()
        .map(|(id, kind)| (id.clone(), kind.clone()))
        .collect();

    // Find removed and modified
    for (id, kind) in &ids_a {
        if let Some(b_kind) = b_map.get(id) {
            if kind != b_kind {
                ops.push(DiffOp::Modified {
                    id: id.clone(),
                    kind: format!("{}→{}", kind, b_kind),
                });
            }
        } else {
            ops.push(DiffOp::Removed {
                id: id.clone(),
                kind: kind.clone(),
            });
        }
    }

    // Find added
    for (id, kind) in &ids_b {
        if !a_map.contains_key(id) {
            ops.push(DiffOp::Added {
                id: id.clone(),
                kind: kind.clone(),
            });
        }
    }

    let removed = ops
        .iter()
        .filter(|op| matches!(op, DiffOp::Removed { .. }))
        .count();
    let added = ops
        .iter()
        .filter(|op| matches!(op, DiffOp::Added { .. }))
        .count();
    let modified = ops
        .iter()
        .filter(|op| matches!(op, DiffOp::Modified { .. }))
        .count();
    let identical = ops.is_empty();

    IrDiff {
        ops,
        total_ops: removed + added + modified,
        removed,
        added,
        modified,
        identical,
    }
}

/// Compare two IR documents and produce a structural diff.
pub fn diff_ir(a: &IRDocument, b: &IRDocument) -> IrDiff {
    let json_a = serde_json::to_string(a).unwrap_or_default();
    let json_b = serde_json::to_string(b).unwrap_or_default();

    if json_a == json_b {
        return IrDiff::empty();
    }

    IrDiff {
        ops: vec![
            DiffOp::Removed {
                id: "ir".to_string(),
                kind: "IRDocument".to_string(),
            },
            DiffOp::Added {
                id: "ir".to_string(),
                kind: "IRDocument".to_string(),
            },
        ],
        total_ops: 2,
        removed: 1,
        added: 1,
        modified: 0,
        identical: false,
    }
}

/// An XML diff entry: a change at a specific byte span.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct XmlDiffEntry {
    /// The path/selector for this element.
    pub path: String,
    /// Type of change.
    pub diff_type: String,
    /// Old value (for removed/modified).
    pub old_value: Option<String>,
    /// New value (for added/modified).
    pub new_value: Option<String>,
    /// Byte span in the source document.
    pub span: Option<gp_core::diagnostics::Span>,
}

/// Compare two XML part byte slices and produce a diff.
///
/// For P1: byte-level diff using line-by-line comparison.
/// Full XML-aware diff is deferred to Phase 2.
pub fn diff_xml_parts(original: &[u8], modified: &[u8]) -> Vec<XmlDiffEntry> {
    let orig_lines = std::str::from_utf8(original)
        .map(|s| s.lines().collect::<Vec<_>>())
        .unwrap_or_default();
    let mod_lines = std::str::from_utf8(modified)
        .map(|s| s.lines().collect::<Vec<_>>())
        .unwrap_or_default();

    let mut entries = Vec::new();
    let max_len = orig_lines.len().max(mod_lines.len());

    for i in 0..max_len {
        let line_num = i + 1;
        match (orig_lines.get(i), mod_lines.get(i)) {
            (None, Some(_)) => {
                entries.push(XmlDiffEntry {
                    path: format!("line:{}", line_num),
                    diff_type: "added".to_string(),
                    old_value: None,
                    new_value: Some(mod_lines[i].to_string()),
                    span: Some(gp_core::diagnostics::Span {
                        offset: 0,
                        length: mod_lines[i].len(),
                    }),
                });
            }
            (Some(_), None) => {
                entries.push(XmlDiffEntry {
                    path: format!("line:{}", line_num),
                    diff_type: "removed".to_string(),
                    old_value: Some(orig_lines[i].to_string()),
                    new_value: None,
                    span: Some(gp_core::diagnostics::Span {
                        offset: 0,
                        length: orig_lines[i].len(),
                    }),
                });
            }
            (Some(a), Some(b)) if a != b => {
                entries.push(XmlDiffEntry {
                    path: format!("line:{}", line_num),
                    diff_type: "modified".to_string(),
                    old_value: Some(a.to_string()),
                    new_value: Some(b.to_string()),
                    span: Some(gp_core::diagnostics::Span {
                        offset: 0,
                        length: b.len(),
                    }),
                });
            }
            _ => {} // No change
        }
    }

    entries
}

/// Compare two XML parts and return a summary diagnostic.
pub fn xml_parts_diagnostic(original: &[u8], modified: &[u8]) -> Option<Diagnostic> {
    let entries = diff_xml_parts(original, modified);
    if entries.is_empty() {
        return None;
    }

    Some(Diagnostic::new(
        "LIN-I-002",
        gp_core::diagnostics::Severity::Info,
        format!("XML part differs: {} line(s) changed", entries.len()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::diagnostics::Severity;
    use gp_core::ir::*;

    fn make_doc(sections: Vec<Section>) -> Document {
        Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections,
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
            revisions: vec![],
        }
    }

    #[allow(dead_code)]
    fn make_section(id: &str, kind: SectionKind, items: Vec<SectionItem>) -> Section {
        Section {
            id: ElementId::content_anchored("section", &[id], id),
            kind,
            items,
            headers: vec![],
            footers: vec![],
            footnotes: vec![],
            endnotes: vec![],
        }
    }

    fn make_section_id(id: ElementId, kind: SectionKind, items: Vec<SectionItem>) -> Section {
        Section {
            id,
            kind,
            items,
            headers: vec![],
            footers: vec![],
            footnotes: vec![],
            endnotes: vec![],
        }
    }

    fn make_para(id: &str, runs: Vec<Run>) -> SectionItem {
        SectionItem::Paragraph(Paragraph {
            id: ElementId::content_anchored("paragraph", &[id], id),
            runs,
            page_number: None,
            style_name: None,
            numbering_info: None,
        })
    }

    fn make_run(id: &str, text: &str) -> Run {
        Run {
            id: ElementId::content_anchored("run", &[id], text),
            text: text.to_string(),
            bold: None,
            italic: None,
            font_name: None,
            font_size_pt: None,
            underline: None,
            strikethrough: None,
            style_name: None,
        }
    }

    /// test_diff_identical_documents: same IDs -> no ops
    #[test]
    fn test_diff_identical_documents() {
        let section = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![make_para("p0", vec![make_run("r0", "hello")])],
        );
        let doc_a = make_doc(vec![section.clone()]);
        let doc_b = make_doc(vec![section]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(diff.identical, "identical docs should produce empty diff");
        assert_eq!(diff.total_ops, 0);
    }

    /// test_diff_removed_element: element in A not B -> removed
    #[test]
    fn test_diff_removed_element() {
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![
                make_para("p0", vec![make_run("r0", "hello")]),
                make_para("p1", vec![make_run("r1", "world")]),
            ],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![make_para("p0", vec![make_run("r0", "hello")])],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert_eq!(diff.removed, 2); // p1 + r1
        assert_eq!(diff.added, 0);
    }

    /// test_diff_added_element: element in B not A -> added
    #[test]
    fn test_diff_added_element() {
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![make_para("p0", vec![make_run("r0", "hello")])],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![
                make_para("p0", vec![make_run("r0", "hello")]),
                make_para("p1", vec![make_run("r1", "world")]),
            ],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert_eq!(diff.added, 2); // p1 + r1
        assert_eq!(diff.removed, 0);
    }

    /// test_diff_modified_kind: element kind changed -> modified
    #[test]
    fn test_diff_modified_kind() {
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![make_para("p0", vec![make_run("r0", "hello")])],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Header,
            vec![make_para("p0", vec![make_run("r0", "hello")])],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert_eq!(diff.modified, 1);
        assert!(
            matches!(&diff.ops[0], DiffOp::Modified { kind, .. } if kind.contains("Body") && kind.contains("Header"))
        );
    }

    /// test_diff_multiple_ops: mixed added/removed/modified
    #[test]
    fn test_diff_multiple_ops() {
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![make_para("p0", vec![make_run("r0", "a")])],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Header,
            vec![
                // Same text as doc_a r0 so the ID stays the same (content-anchored)
                make_para("p0", vec![make_run("r0", "a")]),
                make_para("p1", vec![make_run("r1", "new")]),
            ],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert_eq!(diff.modified, 1); // section kind change
        assert_eq!(diff.added, 2); // p1 + r1
        assert_eq!(diff.ops.len(), diff.total_ops);
    }

    /// test_diff_summary_counts: removed/added/modified counts correct
    #[test]
    fn test_diff_summary_counts() {
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![make_para("p0", vec![make_run("r0", "hello")])],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert_eq!(diff.removed, 2); // p0 + r0
        assert_eq!(diff.added, 0);
        assert_eq!(diff.modified, 0);
        assert_eq!(diff.total_ops, 2);
    }

    /// test_diff_xml_parts_identical: no changes -> empty
    #[test]
    fn test_diff_xml_parts_identical() {
        let xml = b"<root><child>text</child></root>";
        let entries = diff_xml_parts(xml, xml);
        assert!(entries.is_empty());
    }

    /// test_diff_xml_parts_added_line: line in B not A
    #[test]
    fn test_diff_xml_parts_added_line() {
        let a = b"<root></root>";
        let b = b"<root>\n<child>text</child>\n</root>";
        let entries = diff_xml_parts(a, b);
        assert!(!entries.is_empty());
        let added = entries.iter().filter(|e| e.diff_type == "added").count();
        assert!(added > 0, "expected added lines");
    }

    /// test_diff_xml_parts_removed_line: line in A not B
    #[test]
    fn test_diff_xml_parts_removed_line() {
        let a = b"<root>\n<child>text</child>\n</root>";
        let b = b"<root></root>";
        let entries = diff_xml_parts(a, b);
        assert!(!entries.is_empty());
        let removed = entries.iter().filter(|e| e.diff_type == "removed").count();
        assert!(removed > 0, "expected removed lines");
    }

    /// test_diff_xml_parts_modified_line: line content changed
    #[test]
    fn test_diff_xml_parts_modified_line() {
        let a = b"<root><old>text</old></root>";
        let b = b"<root><new>text</new></root>";
        let entries = diff_xml_parts(a, b);
        assert!(!entries.is_empty());
        let modified = entries.iter().filter(|e| e.diff_type == "modified").count();
        assert!(modified > 0, "expected modified lines");
    }

    /// test_xml_parts_diagnostic_no_change: identical -> None
    #[test]
    fn test_xml_parts_diagnostic_no_change() {
        let xml = b"<root>text</root>";
        let result = xml_parts_diagnostic(xml, xml);
        assert!(result.is_none());
    }

    /// test_xml_parts_diagnostic_with_change: different -> Some diagnostic
    #[test]
    fn test_xml_parts_diagnostic_with_change() {
        let a = b"<root>old</root>";
        let b = b"<root>new</root>";
        let result = xml_parts_diagnostic(a, b);
        assert!(result.is_some());
        let diag = result.unwrap();
        assert_eq!(diag.code, "LIN-I-002");
        assert_eq!(diag.severity, Severity::Info);
    }

    /// test_diff_stable_on_move: same IDs despite position change -> no diff
    #[test]
    fn test_diff_stable_on_move() {
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![
                make_para("p0", vec![make_run("r0", "first")]),
                make_para("p1", vec![make_run("r1", "second")]),
            ],
        );
        // Same paragraphs but in reversed order
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![
                make_para("p1", vec![make_run("r1", "second")]),
                make_para("p0", vec![make_run("r0", "first")]),
            ],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(
            diff.identical,
            "same IDs should produce identical diff regardless of position: {:?}",
            diff.ops
        );
    }

    /// test_serde_diff_op: roundtrip
    #[test]
    fn test_serde_diff_op() {
        let op = DiffOp::Added {
            id: "test-id".to_string(),
            kind: "Paragraph".to_string(),
        };
        let json = serde_json::to_string(&op).expect("serialize");
        let back: DiffOp = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(op, back);
    }

    /// test_serde_ir_diff: roundtrip
    #[test]
    fn test_serde_ir_diff() {
        let diff = IrDiff {
            ops: vec![
                DiffOp::Added {
                    id: "id-abc123".to_string(),
                    kind: "Paragraph".to_string(),
                },
                DiffOp::Removed {
                    id: "id-def456".to_string(),
                    kind: "Run".to_string(),
                },
            ],
            total_ops: 2,
            removed: 1,
            added: 1,
            modified: 0,
            identical: false,
        };
        let json = serde_json::to_string(&diff).expect("serialize");
        let back: IrDiff = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(diff, back);
    }

    /// test_diff_tables: table elements are collected with cells
    #[test]
    fn test_diff_tables() {
        let cell = gp_core::ir::Cell {
            id: ElementId::content_anchored("cell", &["cell:0"], "c0"),
            contents: vec![],
            colspan: None,
            rowspan: None,
        };
        let row = gp_core::ir::Row { cells: vec![cell] };
        let table = gp_core::ir::Table {
            id: ElementId::content_anchored("table", &["table:0"], "t0"),
            rows: vec![row],
        };
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![SectionItem::Table(table.clone())],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert!(diff.removed > 0, "table elements should be removed");
    }

    /// test_diff_drawing: drawing elements are collected
    #[test]
    fn test_diff_drawing() {
        let drawing = gp_core::ir::Drawing {
            id: ElementId::content_anchored("drawing", &["drawing:0"], "d0"),
            mime_type: "image/png".to_string(),
            data: "base64data".to_string(),
            bbox_pt: None,
        };
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![SectionItem::Drawing(drawing.clone())],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert!(diff.removed > 0, "drawing should be removed");
    }

    /// test_diff_list: list elements with items are collected
    #[test]
    fn test_diff_list() {
        let list = gp_core::ir::List {
            id: ElementId::content_anchored("list", &["list:0"], "l0"),
            kind: ListKind::Bullet,
            items: vec![
                gp_core::ir::ListItem {
                    id: ElementId::content_anchored("listitem", &["item:0"], "i0"),
                    content: vec![],
                    level: 0,
                },
                gp_core::ir::ListItem {
                    id: ElementId::content_anchored("listitem", &["item:1"], "i1"),
                    content: vec![],
                    level: 0,
                },
            ],
        };
        let section_a = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![SectionItem::List(list.clone())],
        );
        let section_b = make_section_id(
            ElementId::content_anchored("section", &["s0"], "s0"),
            SectionKind::Body,
            vec![],
        );
        let doc_a = make_doc(vec![section_a]);
        let doc_b = make_doc(vec![section_b]);
        let diff = diff_documents(&doc_a, &doc_b);
        assert!(!diff.identical);
        assert!(diff.removed > 0, "list and list items should be removed");
    }

    /// test_diff_ir_documents_different_types: different IRDocument content -> ops
    #[test]
    fn test_diff_ir_documents_different_types() {
        let ir_a = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::content_anchored("tag", &["root"], "root"),
                tag: "root".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        };
        let ir_b = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::content_anchored("tag", &["root"], "other"),
                tag: "root".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        };
        let diff = diff_ir(&ir_a, &ir_b);
        assert!(!diff.identical);
        assert!(diff.total_ops > 0);
    }

    /// test_diff_ir_documents_same_type: same IRDocument -> no ops
    #[test]
    fn test_diff_ir_documents_same_type() {
        let root = TaggedNode {
            id: ElementId::content_anchored("tag", &["root"], "root"),
            tag: "root".to_string(),
            alt_text: None,
            bbox: None,
            children: vec![],
        };
        let ir_a = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: root.clone(),
        };
        let ir_b = IRDocument {
            ir_version: IR_VERSION.to_string(),
            root,
        };
        let diff = diff_ir(&ir_a, &ir_b);
        assert!(diff.identical);
        assert_eq!(diff.total_ops, 0);
    }

    /// test_ir_diff_empty: empty diff methods
    #[test]
    fn test_ir_diff_empty() {
        let diff = IrDiff::empty();
        assert!(diff.is_empty());
        assert!(diff.identical);
        assert_eq!(diff.total_ops, 0);
        assert_eq!(diff.removed, 0);
        assert_eq!(diff.added, 0);
        assert_eq!(diff.modified, 0);
    }

    /// test_diff_op_type: op_type returns correct string
    #[test]
    fn test_diff_op_type() {
        let removed = DiffOp::Removed {
            id: "x".into(),
            kind: "y".into(),
        };
        assert_eq!(removed.op_type(), "removed");

        let added = DiffOp::Added {
            id: "x".into(),
            kind: "y".into(),
        };
        assert_eq!(added.op_type(), "added");

        let modified = DiffOp::Modified {
            id: "x".into(),
            kind: "y".into(),
        };
        assert_eq!(modified.op_type(), "modified");
    }

    /// test_xml_diff_entry_serialization: XmlDiffEntry roundtrip
    #[test]
    fn test_xml_diff_entry_serialization() {
        let entry = XmlDiffEntry {
            path: "line:5".to_string(),
            diff_type: "modified".to_string(),
            old_value: Some("<old/>".to_string()),
            new_value: Some("<new/>".to_string()),
            span: Some(gp_core::diagnostics::Span {
                offset: 100,
                length: 10,
            }),
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: XmlDiffEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, back);
    }
}
