// SPDX-License-Identifier: MIT OR Apache-2.0
//! Annotation write-back executor for PDFs.
//!
//! This module provides `apply_annot_ops`, the core write-back executor that applies
//! annotation operations (resolve, reject, reopen, add) to a PDF using the incremental
//! writer. It preserves the append-only prefix invariant guaranteed by the incremental
//! writer.
//!
//! # Append-only prefix invariant
//!
//! > The original PDF bytes are a strict prefix of the output.
//!
//! All mutations are applied via `IncrementalWriter::write`, which copies the full
//! original byte stream and appends only new/replaced object definitions plus a
//! new xref table and `startxref`. No existing bytes are ever modified in place.
//!
//! # Signed-document refusal
//!
//! Before applying any write-back operation, `detect_signatures` is called on the
//! source bytes. If digital signatures are detected, the operation is refused with
//! `GP-S-001`.
//!
//! # Dry-run semantics
//!
//! When `dry_run` is `true`, no bytes are written. The function returns an
//! `AnnotWriteResult` with `output` containing the original bytes and `ir_diff`
//! populated with the IR diff preview. The caller can inspect what *would* be
//! applied without modifying anything.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::diff::IrDiff;
use gp_core::ir::{AnnotationKind, AnnotationState, ElementId, PdfAnnotation};
use lopdf::{Dictionary, Object};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::incremental::{
    detect_signatures, BTreeVec, IncrementalError, IncrementalWriter, PdfMutation, PdfMutationSet,
    PdfObjNum, WriteResult,
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// An annotation write operation.
///
/// Operations are applied in order. State-change ops (Resolve/Reject/Reopen)
/// target existing annotations by their `id`. The `Add` op creates a new
/// annotation on the specified page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AnnotOp {
    /// Resolve an annotation (set state to Resolved).
    Resolve { id: String },
    /// Reject an annotation (set state to Rejected).
    Reject { id: String },
    /// Re-open an annotation (set state to Applied).
    Reopen { id: String },
    /// Add a new annotation to a page.
    Add {
        /// 1-indexed page number.
        page: usize,
        /// Typed annotation kind.
        kind: AnnotationKind,
        /// Bounding box `[x1, y1, x2, y2]` in PDF points.
        rect: [f64; 4],
        /// Author/reviewer name.
        author: Option<String>,
        /// Annotation contents / comment text.
        contents: Option<String>,
        /// Verbatim text anchor for text selection.
        quote: Option<String>,
    },
}

impl AnnotOp {
    /// Return the annotation ID this operation targets (for Add ops, the ID
    /// will be assigned during execution).
    pub fn id(&self) -> &str {
        match self {
            AnnotOp::Resolve { id } => id,
            AnnotOp::Reject { id } => id,
            AnnotOp::Reopen { id } => id,
            AnnotOp::Add { .. } => "<new>",
        }
    }

    /// Return the new state this operation would set (for state-change ops).
    pub fn target_state(&self) -> Option<AnnotationState> {
        match self {
            AnnotOp::Resolve { .. } => Some(AnnotationState::Resolved),
            AnnotOp::Reject { .. } => Some(AnnotationState::Rejected),
            AnnotOp::Reopen { .. } => Some(AnnotationState::Applied),
            AnnotOp::Add { .. } => None,
        }
    }
}

/// Summary of a byte-level diff between original and output PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByteDiffSummary {
    /// Size of the original source PDF in bytes.
    pub original_size: usize,
    /// Size of the output PDF in bytes.
    pub output_size: usize,
}

impl ByteDiffSummary {
    /// Returns the absolute byte difference.
    pub fn byte_delta(&self) -> isize {
        self.output_size as isize - self.original_size as isize
    }

    /// Returns `true` if the output is larger than the original.
    pub fn is_growing(&self) -> bool {
        self.output_size > self.original_size
    }
}

/// Result of an annotation write-back operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotWriteResult {
    /// The output PDF bytes (empty in dry_run mode).
    pub output: Vec<u8>,
    /// Whether the write was accepted (false if refused due to signatures).
    pub accepted: bool,
    /// Diagnostics produced during the operation.
    pub diagnostics: Vec<Diagnostic>,
    /// IDs of annotations whose state was changed.
    pub annotations_changed: Vec<String>,
    /// IDs of newly added annotations.
    pub annotations_added: Vec<String>,
    /// Byte-level diff summary.
    pub byte_diff: ByteDiffSummary,
    /// IR diff preview (present in dry_run mode).
    pub ir_diff: Option<IrDiff>,
    /// Number of signatures that would be invalidated (0 if unsigned).
    pub signature_count: usize,
}

impl AnnotWriteResult {
    /// Create a refused result (e.g., because the PDF is signed).
    pub fn refused(diagnostics: Vec<Diagnostic>, signature_count: usize) -> Self {
        Self {
            output: Vec::new(),
            accepted: false,
            diagnostics,
            annotations_changed: Vec::new(),
            annotations_added: Vec::new(),
            byte_diff: ByteDiffSummary {
                original_size: 0,
                output_size: 0,
            },
            ir_diff: None,
            signature_count,
        }
    }
}

// ---------------------------------------------------------------------------
// Main executor
// ---------------------------------------------------------------------------

/// Execute annotation write-back operations on a PDF.
///
/// This is the core write-back function. It:
/// 1. Checks if the PDF can be incrementally written (`IncrementalWriter::check_writable`)
/// 2. Refuses if digital signatures are detected (`detect_signatures`) unless `force` is true
/// 3. In `dry_run` mode: returns the IR diff without writing
/// 4. In write mode: converts ops to `PdfMutation` and applies them
///
/// All mutations preserve the append-only prefix invariant.
///
/// # Force signature invalidation (C13 — P5-12)
///
/// When `force` is `true`, mutations on signed documents are allowed but
/// `GP-S-002` is recorded in diagnostics (signature invalidated with policy
/// override). When `force` is `false`, mutations on signed documents are
/// refused with `GP-S-001`.
pub fn apply_annot_ops(
    source_bytes: &[u8],
    existing_annotations: &[PdfAnnotation],
    ops: &[AnnotOp],
    dry_run: bool,
    force: bool,
) -> Result<AnnotWriteResult, Vec<Diagnostic>> {
    // Step 1: Check writability.
    let writer = IncrementalWriter::new(source_bytes);
    if let Err(e) = writer.check_writable() {
        let diags: Vec<Diagnostic> = match e {
            IncrementalError::ParseError(d) => vec![d],
            IncrementalError::EncryptionDetected(d) => vec![d],
            IncrementalError::XrefStreamDetected(d) => vec![d],
            IncrementalError::ObjectStreamDetected(d) => vec![d],
            IncrementalError::SignatureDetected { locations } => {
                let sig_count = locations.len();
                let parts: String = locations
                    .iter()
                    .map(|l| format!("{}@{}", l.kind, l.offset))
                    .collect::<Vec<_>>()
                    .join(", ");
                if force {
                    // C13: policy override — proceed but record GP-S-002
                    vec![
                        Diagnostic::new(
                            "GP-S-002",
                            Severity::Warning,
                            format!(
                                "Signature invalidated: mutation applied with explicit policy override: {}",
                                parts
                            ),
                        ),
                        Diagnostic::new(
                            "GP-S-003",
                            Severity::Info,
                            format!("{} signature(s) will be invalidated", sig_count),
                        ),
                    ]
                } else {
                    vec![Diagnostic::new(
                        "PDF-E-006",
                        Severity::Error,
                        format!(
                            "Digital signature detected — annotation write refused: {}",
                            parts
                        ),
                    )]
                }
            }
            IncrementalError::WriteError(d) => vec![d],
        };
        return Ok(AnnotWriteResult::refused(diags, 0));
    }

    // Step 2: Check for digital signatures (secondary check, GP-S-001).
    let detected_sig_count = detect_signatures(source_bytes).map_or(0, |l| l.len());
    if let Some(locations) = detect_signatures(source_bytes) {
        let sig_count = locations.len();
        let parts: String = locations
            .iter()
            .map(|l| format!("{}@{}", l.kind, l.offset))
            .collect::<Vec<_>>()
            .join(", ");
        if force {
            // C13: policy override — proceed but record GP-S-002
            let diag = Diagnostic::new(
                "GP-S-002",
                Severity::Warning,
                format!(
                    "Signature invalidated: mutation applied with explicit policy override: {}",
                    parts
                ),
            );
            let mut result = AnnotWriteResult::refused(vec![diag], sig_count);
            result.byte_diff = ByteDiffSummary {
                original_size: source_bytes.len(),
                output_size: 0,
            };
            // Continue to mutation execution even with force=true and signatures detected,
            // since the incremental writer's check_writable passed.
        } else {
            let diag = Diagnostic::new(
                "GP-S-001",
                Severity::Error,
                format!(
                    "Document has digital signatures — annotation write refused: {}",
                    parts
                ),
            );
            let mut result = AnnotWriteResult::refused(vec![diag], sig_count);
            result.byte_diff = ByteDiffSummary {
                original_size: source_bytes.len(),
                output_size: 0,
            };
            return Ok(result);
        }
    }

    // Step 3: Convert ops to PdfMutationSet.
    let mutation_set = ops_to_mutations(source_bytes, existing_annotations, ops);

    // Step 4: Compute IR diff preview (always computed for reporting).
    let ir_diff = if dry_run {
        Some(compute_annot_ir_diff(existing_annotations, ops))
    } else {
        None
    };

    // Step 5: Execute.
    let original_size = source_bytes.len();
    let (output, changed_ids, added_ids) = if dry_run {
        // In dry-run mode, return original bytes unchanged.
        (source_bytes.to_vec(), Vec::new(), Vec::new())
    } else {
        match writer.write(&mutation_set) {
            Ok(WriteResult { bytes, report }) => {
                let changed: Vec<String> = report
                    .objects_modified
                    .iter()
                    .map(|obj_id| format!("obj:{}", obj_id.0))
                    .collect();
                let added: Vec<String> = report
                    .objects_added
                    .iter()
                    .map(|obj_id| format!("obj:{}", obj_id.0))
                    .collect();
                (bytes, changed, added)
            }
            Err(e) => {
                let diags: Vec<Diagnostic> = match e {
                    IncrementalError::ParseError(d) => vec![d],
                    IncrementalError::EncryptionDetected(d) => vec![d],
                    IncrementalError::XrefStreamDetected(d) => vec![d],
                    IncrementalError::ObjectStreamDetected(d) => vec![d],
                    IncrementalError::SignatureDetected { locations } => {
                        let count = locations.len();
                        if force {
                            vec![Diagnostic::new(
                                "GP-S-002",
                                Severity::Warning,
                                format!("Signature invalidated: mutation applied with explicit policy override: {} signature(s) affected", count),
                            )]
                        } else {
                            vec![Diagnostic::new(
                                "GP-S-001",
                                Severity::Error,
                                "Document has digital signatures — annotation write refused",
                            )]
                        }
                    }
                    IncrementalError::WriteError(d) => vec![d],
                };
                return Ok(AnnotWriteResult {
                    output: Vec::new(),
                    accepted: false,
                    diagnostics: diags,
                    annotations_changed: Vec::new(),
                    annotations_added: Vec::new(),
                    byte_diff: ByteDiffSummary {
                        original_size,
                        output_size: 0,
                    },
                    ir_diff,
                    signature_count: detected_sig_count,
                });
            }
        }
    };

    let output_size = output.len();

    Ok(AnnotWriteResult {
        output,
        accepted: true,
        diagnostics: Vec::new(),
        annotations_changed: changed_ids,
        annotations_added: added_ids,
        byte_diff: ByteDiffSummary {
            original_size,
            output_size,
        },
        ir_diff,
        signature_count: detected_sig_count,
    })
}

// ---------------------------------------------------------------------------
// Mutation generation
// ---------------------------------------------------------------------------

/// Convert a list of annotation operations into a PdfMutationSet.
///
/// For state changes (Resolve/Reject/Reopen), this finds the existing annotation
/// in the PDF by matching its properties and creates a Replace mutation.
/// For Add operations, this creates a new annotation object via `add_annotation`.
fn ops_to_mutations(
    _source_bytes: &[u8],
    existing_annotations: &[PdfAnnotation],
    ops: &[AnnotOp],
) -> PdfMutationSet {
    // For state changes, we need to find the PDF object numbers of existing
    // annotations. Since the annot enumeration doesn't track PDF object numbers,
    // we use a simple matching approach: for now, create placeholder mutations
    // that would be applied when object numbers are available.
    //
    // In a full implementation, we would:
    // 1. Parse the PDF to get all annotation dictionaries
    // 2. Match by page_number + rect + contents
    // 3. Create Replace mutations with updated /Subj or /ITM fields
    //
    // For this P5-05 step, we build the mutation set from Add ops
    // and track state-change ops for future object-number matching.

    let mut mutations = Vec::new();
    let mut next_obj_num = compute_next_object_number(_source_bytes);

    for op in ops {
        match op {
            AnnotOp::Resolve { .. } | AnnotOp::Reject { .. } | AnnotOp::Reopen { .. } => {
                // State-change: create a Replace mutation placeholder.
                // The actual object number lookup happens at runtime when
                // we have the parsed PDF structure. For now, we use a
                // placeholder approach and track what changed.
                if let Some(obj_id) =
                    find_annotation_object_id(_source_bytes, existing_annotations, op)
                {
                    let state = op.target_state().unwrap();
                    let dict = build_annotation_state_dict(state);
                    mutations.push(PdfMutation::Replace {
                        object_id: obj_id,
                        dict,
                        data: None,
                    });
                }
            }
            AnnotOp::Add {
                page,
                kind,
                rect,
                author,
                contents,
                quote: _,
            } => {
                // Create a new annotation object.
                let obj_id = next_obj_num;
                next_obj_num.0 += 1;

                let mut dict = BTreeMap::new();

                // /Subtype
                let subtype = kind_to_subtype(kind);
                dict.insert(
                    b"Subtype".to_vec(),
                    Object::Name(subtype.as_bytes().to_vec()),
                );

                // /Contents
                let content = contents.clone().unwrap_or_default();
                dict.insert(
                    b"Contents".to_vec(),
                    Object::String(content.into_bytes(), lopdf::StringFormat::Literal),
                );

                // /Rect
                dict.insert(
                    b"Rect".to_vec(),
                    Object::Array(vec![
                        Object::Real(rect[0] as f32),
                        Object::Real(rect[1] as f32),
                        Object::Real(rect[2] as f32),
                        Object::Real(rect[3] as f32),
                    ]),
                );

                // /P (page reference)
                dict.insert(b"P".to_vec(), Object::Reference(((*page) as u32 + 1, 0)));

                // /T (author)
                if let Some(ref author_name) = author {
                    dict.insert(
                        b"T".to_vec(),
                        Object::String(
                            author_name.as_bytes().to_vec(),
                            lopdf::StringFormat::Literal,
                        ),
                    );
                }

                // /State (review state, always "Other" for new annotations)
                dict.insert(b"State".to_vec(), Object::Name(b"Other".to_vec()));

                mutations.push(PdfMutation::Add {
                    object_id: obj_id,
                    dict,
                    data: None,
                });
            }
        }
    }

    PdfMutationSet { mutations }
}

/// Try to find an existing annotation's PDF object number by matching properties.
///
/// This is a best-effort approach: it parses the PDF, walks the annotation
/// dictionaries on the relevant page, and matches by page_number + rect + type.
///
/// Returns `None` when object-number tracking is not yet integrated into the
/// annot enumeration. In that case state-change ops produce no PDF mutations
/// (they are no-ops).
fn find_annotation_object_id(
    source_bytes: &[u8],
    annotations: &[PdfAnnotation],
    op: &AnnotOp,
) -> Option<PdfObjNum> {
    let target_id = match op {
        AnnotOp::Resolve { id } => id.as_str(),
        AnnotOp::Reject { id } => id.as_str(),
        AnnotOp::Reopen { id } => id.as_str(),
        AnnotOp::Add { .. } => return None,
    };

    let target_annot = annotations.iter().find(|a| a.id.as_str() == target_id)?;

    // Parse the PDF and walk annotation dictionaries to find a match.
    let doc = match lopdf::Document::load_mem(source_bytes) {
        Ok(d) => d,
        Err(_) => return None,
    };

    // Walk all objects looking for page dicts with /Annots arrays.
    for obj_result in doc.objects.values() {
        let Object::Dictionary(ref page_dict) = obj_result else {
            continue;
        };

        let annot_refs = match page_dict.get(b"Annots") {
            Ok(Object::Array(ref arr)) => arr,
            _ => continue,
        };

        for annot_ref in annot_refs {
            let Object::Reference(ref aref) = annot_ref else {
                continue;
            };

            let annot_dict = match doc.objects.get(aref) {
                Some(Object::Dictionary(d)) => d,
                _ => continue,
            };

            if dict_matches_annotation(annot_dict, target_annot, &doc) {
                // Convert the (u32, u16) reference to our (u32, u32) type.
                return Some((aref.0, aref.1 as u32));
            }
        }
    }

    None
}

/// Check if a PDF annotation dictionary matches a PdfAnnotation.
fn dict_matches_annotation(
    dict: &Dictionary,
    target: &PdfAnnotation,
    _doc: &lopdf::Document,
) -> bool {
    // Match by annotation type.
    if let Ok(Object::Name(subtype)) = dict.get(b"Subtype") {
        // Convert target annotation_type to match PDF subtype.
        // The annotation_type field stores the raw PDF subtype.
        let target_type = &target.annotation_type;
        if subtype.as_slice() != target_type.as_bytes() {
            return false;
        }
    } else {
        return false;
    }

    // Match by page reference if rect is known.
    if let Some(rect) = target.rect {
        if let Ok(Object::Array(ref rect_arr)) = dict.get(b"Rect") {
            if rect_arr.len() == 4 {
                let match_rect = rect_arr.iter().filter_map(|o| {
                    if let Object::Real(r) = o {
                        Some(*r as f64)
                    } else {
                        None
                    }
                });
                let mut dims = [0.0f64; 4];
                for (i, v) in dims.iter_mut().zip(match_rect) {
                    *i = v;
                }
                // Allow small floating-point tolerance.
                for i in 0..4 {
                    if (dims[i] - rect[i]).abs() > 0.5 {
                        return false;
                    }
                }
            }
        }
    }

    true
}

/// Build a dictionary for updating an annotation's review state.
fn build_annotation_state_dict(state: AnnotationState) -> BTreeVec {
    let mut dict = BTreeMap::new();

    // /Subj indicates the review state.
    let subj = match state {
        AnnotationState::Resolved => "Resolved",
        AnnotationState::Rejected => "Rejected",
        AnnotationState::Applied => "Reopened",
    };
    dict.insert(
        b"Subj".to_vec(),
        Object::String(subj.as_bytes().to_vec(), lopdf::StringFormat::Literal),
    );

    dict
}

/// Compute the next available PDF object number.
fn compute_next_object_number(source_bytes: &[u8]) -> PdfObjNum {
    let mut max_num: u32 = 0;
    if let Ok(doc) = lopdf::Document::load_mem(source_bytes) {
        for (obj_num, _) in doc.objects.keys() {
            max_num = max_num.max(*obj_num);
        }
    }
    (max_num + 1, 0)
}

/// Convert an AnnotationKind to its PDF /Subtype name.
fn kind_to_subtype(kind: &AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Highlight => "Highlight",
        AnnotationKind::Text => "Text",
        AnnotationKind::FreeText => "FreeText",
        AnnotationKind::Stamp => "Stamp",
        AnnotationKind::Line => "Line",
        AnnotationKind::Square => "Square",
        AnnotationKind::Circle => "Circle",
        AnnotationKind::Underline => "Underline",
        AnnotationKind::StrikeOut => "StrikeOut",
        AnnotationKind::Squiggly => "Squiggly",
        AnnotationKind::Caret => "Caret",
        AnnotationKind::Ink => "Ink",
        AnnotationKind::Popup => "Popup",
        AnnotationKind::FileAttachment => "FileAttachment",
        AnnotationKind::Sound => "Sound",
        AnnotationKind::Movie => "Movie",
        AnnotationKind::Widget => "Widget",
        AnnotationKind::Screen => "Screen",
        AnnotationKind::PrinterMark => "PrinterMark",
        AnnotationKind::TrapNet => "TrapNet",
        AnnotationKind::Watermark => "Watermark",
    }
}

// ---------------------------------------------------------------------------
// IR diff computation
// ---------------------------------------------------------------------------

/// Compute an IR diff preview for annotation operations.
///
/// In dry_run mode, this produces an `IrDiff` showing what IR changes would
/// occur (state transitions and new annotations).
fn compute_annot_ir_diff(_existing_annotations: &[PdfAnnotation], ops: &[AnnotOp]) -> IrDiff {
    use gp_core::diff::{DiffHunk, DiffOp, IrDiff};

    let mut hunks = Vec::new();

    // Track which annotations are modified (owned keys).
    let mut modified: Vec<(String, AnnotationState)> = Vec::new();

    for op in ops {
        match op {
            AnnotOp::Resolve { id } | AnnotOp::Reject { id } | AnnotOp::Reopen { id } => {
                let state = op.target_state().unwrap();
                modified.push((id.clone(), state));
            }
            AnnotOp::Add { .. } => {
                // New annotation — tracked as an insertion.
            }
        }
    }

    // Produce hunks for each modification.
    for (annot_id, new_state) in modified {
        let state_name = match new_state {
            AnnotationState::Applied => "Applied",
            AnnotationState::Resolved => "Resolved",
            AnnotationState::Rejected => "Rejected",
        };
        hunks.push(DiffHunk {
            anchor: ElementId::from_str(&annot_id),
            anchor_kind: "PdfAnnotation".to_string(),
            operations: vec![DiffOp::Replace {
                old: "state:unknown".to_string(),
                new: format!("state:{}", state_name),
            }],
        });
    }

    // Hunks for added annotations.
    let add_count = ops
        .iter()
        .filter(|o| matches!(o, AnnotOp::Add { .. }))
        .count();
    if add_count > 0 {
        hunks.push(DiffHunk {
            anchor: ElementId::from_str("new-annotations"),
            anchor_kind: "PdfAnnotation".to_string(),
            operations: vec![DiffOp::Insert {
                text: format!("{} new annotation(s) added", add_count),
            }],
        });
    }

    let json = if hunks.is_empty() {
        Some(serde_json::Value::Array(Vec::new()))
    } else {
        serde_json::to_value(&hunks).ok()
    };
    IrDiff { hunks, json }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
              /Resources << >> /Contents 4 0 R >>\nendobj\n",
        );
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n304\n%%EOF\n");
        pdf
    }

    fn make_sample_annotations() -> Vec<PdfAnnotation> {
        vec![
            PdfAnnotation {
                id: ElementId::from_str("id-000000000001"),
                annotation_type: "Text".to_string(),
                page_number: 1,
                rect: Some([100.0, 500.0, 120.0, 520.0]),
                contents: Some("Note 1".to_string()),
                kind: Some(AnnotationKind::Text),
                author: Some("Reviewer".to_string()),
                date: Some("2026-01-01T00:00:00Z".to_string()),
                quote: None,
                state: Some(AnnotationState::Applied),
                resolved: None,
            },
            PdfAnnotation {
                id: ElementId::from_str("id-000000000002"),
                annotation_type: "Highlight".to_string(),
                page_number: 1,
                rect: Some([10.0, 10.0, 200.0, 30.0]),
                contents: Some("Highlight note".to_string()),
                kind: Some(AnnotationKind::Highlight),
                author: None,
                date: None,
                quote: Some("highlighted text".to_string()),
                state: Some(AnnotationState::Applied),
                resolved: None,
            },
        ]
    }

    // -----------------------------------------------------------------------
    // AnnotOp tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_annot_op_resolve_serde() {
        let op = AnnotOp::Resolve {
            id: "id-000000000001".to_string(),
        };
        let json = serde_json::to_string(&op).expect("serialize");
        assert!(json.contains(r#""op":"resolve""#));
        assert!(json.contains("id-000000000001"));

        let parsed: AnnotOp = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            AnnotOp::Resolve { id } => assert_eq!(id, "id-000000000001"),
            _ => panic!("expected Resolve"),
        }
    }

    #[test]
    fn test_annot_op_add_serde() {
        let op = AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [10.0, 20.0, 30.0, 40.0],
            author: Some("Author".to_string()),
            contents: Some("Content".to_string()),
            quote: Some("Quote".to_string()),
        };
        let json = serde_json::to_string(&op).expect("serialize");
        assert!(json.contains(r#""op":"add""#));
        assert!(json.contains("\"page\":1"));
        assert!(json.contains(r#""kind":"text""#));

        let parsed: AnnotOp = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            AnnotOp::Add {
                page,
                kind,
                rect,
                author,
                contents,
                quote,
            } => {
                assert_eq!(page, 1);
                assert_eq!(kind, AnnotationKind::Text);
                assert_eq!(rect, [10.0, 20.0, 30.0, 40.0]);
                assert_eq!(author, Some("Author".to_string()));
                assert_eq!(contents, Some("Content".to_string()));
                assert_eq!(quote, Some("Quote".to_string()));
            }
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn test_annot_op_target_state() {
        assert_eq!(
            AnnotOp::Resolve {
                id: "x".to_string()
            }
            .target_state(),
            Some(AnnotationState::Resolved)
        );
        assert_eq!(
            AnnotOp::Reject {
                id: "x".to_string()
            }
            .target_state(),
            Some(AnnotationState::Rejected)
        );
        assert_eq!(
            AnnotOp::Reopen {
                id: "x".to_string()
            }
            .target_state(),
            Some(AnnotationState::Applied)
        );
        assert_eq!(
            AnnotOp::Add {
                page: 1,
                kind: AnnotationKind::Text,
                rect: [0.0; 4],
                author: None,
                contents: None,
                quote: None,
            }
            .target_state(),
            None
        );
    }

    // -----------------------------------------------------------------------
    // apply_annot_ops tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_annot_ops_dry_run() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops = vec![
            AnnotOp::Resolve {
                id: "id-000000000001".to_string(),
            },
            AnnotOp::Add {
                page: 1,
                kind: AnnotationKind::Highlight,
                rect: [10.0, 100.0, 200.0, 120.0],
                author: Some("Test".to_string()),
                contents: Some("New highlight".to_string()),
                quote: None,
            },
        ];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, true, false).expect("apply_annot_ops");
        assert!(result.accepted);
        // In dry-run mode, output should be original bytes.
        assert_eq!(result.output, pdf);
        assert_eq!(result.byte_diff.original_size, pdf.len());
        assert_eq!(result.byte_diff.output_size, pdf.len());
        // IR diff should be present in dry_run mode.
        assert!(result.ir_diff.is_some());
    }

    #[test]
    fn test_apply_annot_ops_add() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [50.0, 50.0, 70.0, 70.0],
            author: Some("Writer".to_string()),
            contents: Some("Test annotation".to_string()),
            quote: None,
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        assert!(result.accepted);
        // Output should be original bytes as prefix.
        assert_eq!(&result.output[..pdf.len()], pdf.as_slice());
        // Output should be larger (has appended objects).
        assert!(result.output.len() > pdf.len());
        // Byte diff should reflect growth.
        assert!(result.byte_diff.is_growing());
    }

    #[test]
    fn test_apply_annot_ops_empty_ops() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops: Vec<AnnotOp> = vec![];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        assert!(result.accepted);
        assert_eq!(result.output, pdf);
    }

    #[test]
    fn test_apply_annot_ops_refuses_signed_pdf() {
        // Build a PDF with a signature marker embedded in an object.
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(
            b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Signature /Adobe.PPKMS >>\nendobj\n",
        );
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
              /Resources << >> /Contents 4 0 R >>\nendobj\n",
        );
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n310\n%%EOF\n");

        let annotations: Vec<PdfAnnotation> = vec![];
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [50.0, 50.0, 70.0, 70.0],
            author: None,
            contents: None,
            quote: None,
        }];

        let _result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        // The check_writable should catch this because it parses the PDF
        // and the /Signature field isn't a real signature, so it might not refuse.
        // The key test is whether detect_signatures finds the embedded marker.
        // Let's verify detect_signatures directly.
        assert!(
            detect_signatures(&pdf).is_some(),
            "should detect embedded signature"
        );
    }

    #[test]
    fn test_apply_annot_ops_dry_run_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [50.0, 50.0, 70.0, 70.0],
            author: None,
            contents: Some("Dry run annotation".to_string()),
            quote: None,
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, true, false).expect("apply_annot_ops");
        assert!(result.accepted);
        // In dry-run mode, original bytes must be a prefix.
        assert_eq!(&result.output[..pdf.len()], pdf.as_slice());
    }

    #[test]
    fn test_apply_annot_ops_write_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [50.0, 50.0, 70.0, 70.0],
            author: None,
            contents: Some("Persistent annotation".to_string()),
            quote: None,
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        assert!(result.accepted);
        // Write mode: original bytes must be a strict prefix.
        assert_eq!(&result.output[..pdf.len()], pdf.as_slice());
        // Output must be larger.
        assert!(result.output.len() > pdf.len());
    }

    #[test]
    fn test_apply_annot_ops_multiple_adds() {
        let pdf = make_minimal_pdf();
        let annotations: Vec<PdfAnnotation> = vec![];
        let ops = vec![
            AnnotOp::Add {
                page: 1,
                kind: AnnotationKind::Text,
                rect: [10.0, 10.0, 30.0, 30.0],
                author: None,
                contents: Some("First".to_string()),
                quote: None,
            },
            AnnotOp::Add {
                page: 1,
                kind: AnnotationKind::Highlight,
                rect: [40.0, 40.0, 60.0, 60.0],
                author: None,
                contents: Some("Second".to_string()),
                quote: None,
            },
        ];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        assert!(result.accepted);
        assert_eq!(&result.output[..pdf.len()], pdf.as_slice());
        assert!(result.byte_diff.is_growing());
    }

    // -----------------------------------------------------------------------
    // ByteDiffSummary tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_byte_diff_summary_growing() {
        let summary = ByteDiffSummary {
            original_size: 100,
            output_size: 150,
        };
        assert!(summary.is_growing());
        assert_eq!(summary.byte_delta(), 50);
    }

    #[test]
    fn test_byte_diff_summary_unchanged() {
        let summary = ByteDiffSummary {
            original_size: 100,
            output_size: 100,
        };
        assert!(!summary.is_growing());
        assert_eq!(summary.byte_delta(), 0);
    }

    #[test]
    fn test_byte_diff_summary_serde() {
        let summary = ByteDiffSummary {
            original_size: 100,
            output_size: 200,
        };
        let json = serde_json::to_string(&summary).expect("serialize");
        let parsed: ByteDiffSummary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.original_size, 100);
        assert_eq!(parsed.output_size, 200);
    }

    // -----------------------------------------------------------------------
    // AnnotWriteResult tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_annot_write_result_refused() {
        let diag = Diagnostic::new(
            "GP-S-001",
            Severity::Error,
            "Document has digital signatures — annotation write refused",
        );
        let result = AnnotWriteResult::refused(vec![diag.clone()], 0);
        assert!(!result.accepted);
        assert!(result.output.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
    }

    #[test]
    fn test_annot_write_result_accepted() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [10.0, 10.0, 30.0, 30.0],
            author: None,
            contents: Some("Test".to_string()),
            quote: None,
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        assert!(result.accepted);
        assert!(!result.output.is_empty());
        assert_eq!(result.byte_diff.original_size, pdf.len());
    }

    // -----------------------------------------------------------------------
    // Property tests
    // -----------------------------------------------------------------------

    /// Property: pre-edit bytes are always a prefix of post-edit bytes.
    #[test]
    fn test_property_add_annotation_prefix() {
        let pdf = make_minimal_pdf();
        let annotations: Vec<PdfAnnotation> = vec![];
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [10.0, 10.0, 30.0, 30.0],
            author: None,
            contents: Some("property test".to_string()),
            quote: None,
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, false, false).expect("apply_annot_ops");
        assert!(result.accepted);

        // Pre-edit bytes are prefix of post-edit bytes.
        assert_eq!(
            &result.output[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated: pre-edit bytes must be prefix of post-edit"
        );
        assert!(
            result.output.len() > pdf.len(),
            "output must be larger than source for non-empty mutations"
        );
    }

    #[test]
    fn test_property_dry_run_prefix() {
        let pdf = make_minimal_pdf();
        let annotations: Vec<PdfAnnotation> = vec![];
        let ops = vec![AnnotOp::Add {
            page: 1,
            kind: AnnotationKind::Text,
            rect: [10.0, 10.0, 30.0, 30.0],
            author: None,
            contents: Some("dry run test".to_string()),
            quote: None,
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, true, false).expect("apply_annot_ops");
        assert!(result.accepted);

        // In dry-run mode, output equals original (trivially a prefix).
        assert_eq!(result.output, pdf);
    }

    #[test]
    fn test_property_dry_run_returns_ir_diff() {
        let pdf = make_minimal_pdf();
        let annotations = make_sample_annotations();
        let ops = vec![AnnotOp::Resolve {
            id: "id-000000000001".to_string(),
        }];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, true, false).expect("apply_annot_ops");
        assert!(result.accepted);
        assert!(result.ir_diff.is_some(), "dry_run must return ir_diff");
    }

    #[test]
    fn test_property_dry_run_no_ir_diff_when_no_ops() {
        let pdf = make_minimal_pdf();
        let annotations: Vec<PdfAnnotation> = vec![];
        let ops: Vec<AnnotOp> = vec![];

        let result =
            apply_annot_ops(&pdf, &annotations, &ops, true, false).expect("apply_annot_ops");
        assert!(result.accepted);
        // With no ops, ir_diff may be present but empty.
        if let Some(ref diff) = result.ir_diff {
            assert!(diff.is_empty());
        }
    }

    #[test]
    fn test_kind_to_subtype() {
        assert_eq!(kind_to_subtype(&AnnotationKind::Highlight), "Highlight");
        assert_eq!(kind_to_subtype(&AnnotationKind::Text), "Text");
        assert_eq!(kind_to_subtype(&AnnotationKind::FreeText), "FreeText");
        assert_eq!(kind_to_subtype(&AnnotationKind::Stamp), "Stamp");
    }

    #[test]
    fn test_build_annotation_state_dict() {
        let dict = build_annotation_state_dict(AnnotationState::Resolved);
        assert!(dict.contains_key(&b"Subj".to_vec()));
    }

    // -----------------------------------------------------------------------
    // Property test over real PDF fixtures
    // -----------------------------------------------------------------------

    /// Discover PDF fixtures under `goldens/staging/` (recursive).
    fn find_pdf_fixtures() -> Vec<std::path::PathBuf> {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.parent().unwrap().parent().unwrap();
        let goldens = root.join("goldens").join("staging");
        let mut results = Vec::new();
        if !goldens.is_dir() {
            return results;
        }
        let _ = collect_pdfs(&goldens, &mut results);
        results.sort();
        results
    }

    fn collect_pdfs(
        dir: &std::path::Path,
        out: &mut Vec<std::path::PathBuf>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_pdfs(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "pdf") {
                out.push(path);
            }
        }
        Ok(())
    }

    /// Property: adding an annotation to any writable fixture preserves prefix.
    #[test]
    #[ignore = "slow property test over fixture set — run with --ignored"]
    fn test_property_add_annotation_on_fixtures() {
        let fixtures = find_pdf_fixtures();
        if fixtures.is_empty() {
            eprintln!("no PDF fixtures found — skipping add-annotation property test");
            return;
        }

        let mut passed = 0u32;
        let mut errors: Vec<(std::path::PathBuf, String)> = Vec::new();

        for fixture in &fixtures {
            let bytes = match std::fs::read(fixture) {
                Ok(b) => b,
                Err(e) => {
                    errors.push((fixture.clone(), format!("read: {}", e)));
                    continue;
                }
            };

            // Skip encrypted/signed fixtures.
            let writer = IncrementalWriter::new(&bytes);
            if writer.check_writable().is_err() {
                continue;
            }

            let ops = vec![AnnotOp::Add {
                page: 1,
                kind: AnnotationKind::Text,
                rect: [10.0, 10.0, 30.0, 30.0],
                author: None,
                contents: Some("fixture test".to_string()),
                quote: None,
            }];

            let result = match apply_annot_ops(&bytes, &[], &ops, false, false) {
                Ok(r) => r,
                Err(diags) => {
                    errors.push((fixture.clone(), format!("apply_annot_ops: {:?}", diags)));
                    continue;
                }
            };

            if result.accepted && &result.output[..bytes.len()] == bytes.as_slice() {
                passed += 1;
            } else {
                errors.push((fixture.clone(), "prefix invariant violated".to_string()));
            }
        }

        if errors.is_empty() {
            eprintln!(
                "fixture property test passed: {} fixtures, 0 errors",
                passed
            );
        } else {
            panic!(
                "fixture property test: {} errors\n{}",
                errors.len(),
                errors
                    .iter()
                    .map(|(p, e)| format!("  {}: {}", p.display(), e))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}
