# C12 — PDF Annotations Contract

| | |
|---|---|
| Owner | PDF |
| Consumers | all adapters (read), cli/mcp (surface) |
| Freeze | **frozen at P5-03** |
| Version | 1.0.0-draft |
| Status | draft at P5-03 |
| Artifacts | `crates/gp-core/src/ir.rs` (AnnotationKind, AnnotationState, PdfAnnotation + helpers), `crates/gp-core/src/diagnostics.rs` (GP-A-001..GP-A-005) |
| Conformance | serde roundtrip + C1 additive + refuse-list + state machine |

## 1. Scope

This contract defines the PDF annotation-review model in the stichos intermediate representation (IR).
Annotations capture highlights, text notes, free-text annotations, stamps, and other PDF annotation types.
The model provides a typed `kind` field alongside the raw `annotation_type` for backward-compatible
parsing, an optional review `state` machine, and strict refuse-list semantics for unknown subtypes.

## 2. Types

### 2.1 AnnotationKind

```rust
#[serde(rename_all = "lowercase")]
pub enum AnnotationKind {
    Highlight, Text, FreeText, Stamp, Line, Square, Circle,
    Underline, StrikeOut, Squiggly, Caret, Ink, Popup,
    FileAttachment, Sound, Movie, Widget, Screen,
    PrinterMark, TrapNet, Watermark,
}
```

Serialized as snake_case JSON strings: `"highlight"`, `"text"`, `"free_text"`, etc.

### 2.2 AnnotationState

```rust
#[serde(rename_all = "lowercase")]
pub enum AnnotationState {
    Applied,
    Resolved,
    Rejected,
}
```

Serialized as lowercase JSON strings: `"applied"`, `"resolved"`, `"rejected"`.
Default state is `Applied`.

### 2.3 PdfAnnotation

```rust
pub struct PdfAnnotation {
    pub id: ElementId,
    pub annotation_type: String,
    pub page_number: usize,
    pub rect: Option<[f64; 4]>,
    pub contents: Option<String>,
    pub kind: Option<AnnotationKind>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub quote: Option<String>,
    pub state: Option<AnnotationState>,
    pub resolved: Option<bool>,
}
```

- **id**: Stable unique identifier (C2 format: `id-<hex>`).
- **annotation_type**: Raw PDF /Subtype value (e.g. `"Highlight"`, `"Text"`). Always present for backward compat.
- **page_number**: The page this annotation appears on (1-indexed).
- **rect**: Optional bounding box `[x1, y1, x2, y2]` in PDF points.
- **contents**: Raw annotation content / comment text.
- **kind**: Typed annotation kind (optional; `None` for raw/unknown subtypes).
- **author**: Author/reviewer name (optional).
- **date**: ISO-8601 timestamp (optional).
- **quote**: Verbatim text anchor for text selection (optional).
- **state**: Review state (optional; `None` defaults to `Applied`).
- **resolved**: Legacy compatibility flag (optional).

### 2.4 PdfDocument.annotations

Annotations are stored in the `PdfDocument.annotations` field as `Vec<PdfAnnotation>`.

```rust
pub struct PdfDocument {
    pub id: ElementId,
    pub pages: Vec<PdfPage>,
    pub metadata: Option<PdfMetadata>,
    pub annotations: Vec<PdfAnnotation>,
    // ... other fields
}
```

The `skip_serializing_if = "Vec::is_empty", default` ensures backward compatibility.

### 2.5 PdfAnnotation helpers

```rust
impl PdfAnnotation {
    pub fn is_supported_kind(&self) -> bool
    pub fn transition_state(&mut self, new_state: AnnotationState)
}
```

- `is_supported_kind`: Returns `true` if the annotation's kind is `None` or one of the supported types
  (Highlight, Text, FreeText, Stamp). `None` is supported because raw subtypes are preserved in
  `annotation_type`.
- `transition_state`: Transitions the annotation to a new state.

## 3. State Machine

### 3.1 States

| State | Description |
|-------|-------------|
| `Applied` | Annotation is active / under review (default) |
| `Resolved` | Annotation has been resolved |
| `Rejected` | Annotation has been rejected |

### 3.2 Transitions

| From → To | Allowed |
|-----------|---------|
| Applied → Resolved | Yes |
| Applied → Rejected | Yes |
| Rejected → Applied | Yes (re-open) |
| Resolved → Applied | No (final; requires re-creation) |
| Resolved → Rejected | No (final; requires re-creation) |

### 3.3 Usage

The `transition_state` method transitions the annotation to a new state.
Adapters and surfaces should enforce the transition rules above.

## 4. Anchoring Rules

Annotations are anchored to a page number (`page_number`) plus an optional rectangle (`rect`).

- The `page_number` is the 1-indexed page where the annotation appears.
- The `rect` defines the bounding box `[x1, y1, x2, y2]` in PDF points (bottom-left origin).
- If `rect` is `None`, the annotation is still valid but triggers a warning (GP-A-002).
- If `kind` is `None`, the `annotation_type` field contains the raw PDF subtype string.

### 4.1 Anchoring Summary

Annotations are anchored to page number + rect. Unknown subtypes → refuse with GP-A-001.
XFA/XDP forms → refuse with GP-A-005. No-rect annotations → warn with GP-A-002.

## 5. Refuse List

| Pattern | Diagnostic Code | Behavior |
|---------|-----------------|----------|
| Unknown annotation subtype (not in AnnotationKind) | GP-A-001 | Warning: refuse to process |
| XFA/XDP form in annotations | GP-A-005 | Warning: refuse to process |
| No rectangle on annotation | GP-A-002 | Warning: cannot render |
| Duplicated annotation ID | GP-A-003 | Warning: keep first occurrence |
| Annotation references unknown page | GP-A-004 | Warning: out-of-range page |

## 6. C1 Additive Compatibility

New `PdfAnnotation` fields are all `Option<T>` with `skip_serializing_if = "Option::is_none"`.
Old IR documents (without the new fields) deserialize with all new fields defaulting to `None`.

```rust
// Old IR annotation (no kind, author, date, quote, state, resolved)
let old_json = r#"{
    "id": "id-000000000000",
    "annotation_type": "Highlight",
    "page_number": 1,
    "rect": [0, 0, 100, 20],
    "contents": "old annotation"
}"#;
let annot: PdfAnnotation = serde_json::from_str(old_json).unwrap();
assert!(annot.kind.is_none());       // defaults to None
assert!(annot.author.is_none());     // defaults to None
assert!(annot.state.is_none());      // defaults to None
```

## 7. Diagnostic Codes

| Code | Severity | Category | Description |
|------|----------|----------|-------------|
| GP-A-001 | Warning | unsupported | Unsupported annotation subtype '{}' — refusing to process |
| GP-A-002 | Warning | validation | Annotation has no rectangle — cannot render |
| GP-A-003 | Warning | validation | Duplicated annotation ID '{}' — keeping first occurrence |
| GP-A-004 | Warning | validation | Annotation references unknown page {} (max: {}) |
| GP-A-005 | Warning | unsupported | XFA/XDP form detected in annotations — unsupported |

## 8. Annotation Kind Reference

| Kind | PDF Subtype | Supported | Description |
|------|-------------|-----------|-------------|
| Highlight | Highlight | Yes | Highlights text |
| Text | Text | Yes | Text note / comment |
| FreeText | FreeText | Yes | Free text annotation |
| Stamp | Stamp | Yes | Stamp annotation |
| Line | Line | No | Line annotation |
| Square | Square | No | Square annotation |
| Circle | Circle | No | Circle annotation |
| Underline | Underline | No | Underline annotation |
| StrikeOut | StrikeOut | No | Strikeout annotation |
| Squiggly | Squiggly | No | Squiggly underline |
| Caret | Caret | No | Caret annotation |
| Ink | Ink | No | Ink annotation |
| Popup | Popup | No | Popup annotation |
| FileAttachment | FileAttachment | No | File attachment annotation |
| Sound | Sound | No | Sound annotation |
| Movie | Movie | No | Movie annotation |
| Widget | Widget | No | Widget annotation |
| Screen | Screen | No | Screen annotation |
| PrinterMark | PrinterMark | No | Printer mark annotation |
| TrapNet | TrapNet | No | Trap net annotation |
| Watermark | — | No | Watermark (not a standard PDF subtype) |

## 9. Write-Back Path (P5-05)

### 9.1 Overview

The annotation write-back path allows applying state-change operations (resolve, reject, reopen)
and adding new annotations to a PDF while preserving the **append-only prefix invariant**:

> The original PDF bytes are a strict prefix of the output.

The executor lives in `crates/adapters-pdf/src/annot_write.rs` as `apply_annot_ops`.

### 9.2 AnnotOp Enum

| Variant | Description | PDF Effect |
|---------|-------------|------------|
| `Resolve { id }` | Set annotation state to Resolved | Replace annotation dict with `/Subj "Resolved"` |
| `Reject { id }` | Set annotation state to Rejected | Replace annotation dict with `/Subj "Rejected"` |
| `Reopen { id }` | Set annotation state to Applied | Replace annotation dict with `/Subj "Reopened"` |
| `Add { .. }` | Create a new annotation | Add new PDF object with `/Subtype`, `/Rect`, `/Contents`, etc. |

### 9.3 Signed-Document Refusal

Before applying any write-back operation, `detect_signatures` scans the source bytes.
If digital signatures are detected, the operation is refused with code `GP-S-001`:

```
GP-S-001: "Document has digital signatures — annotation write refused"
```

This check runs in addition to `IncrementalWriter::check_writable`, which also
refuses on signatures (PDF-E-006). The GP-S-001 code provides a higher-level
semantic for the annotation subsystem.

### 9.4 Append-Only Prefix Invariant

The incremental writer guarantees that `&source_bytes == &output[..source_bytes.len()]`.

The incremental update consists of:
1. All original bytes (exact copy — strict prefix)
2. New/updated annotation object definitions appended after the original
3. A new classic xref table referencing ALL objects
4. A new `startxref` pointing to the new xref table
5. `%%EOF` marker

No existing PDF bytes are ever modified in place. This invariant is tested
by property tests that verify pre-edit bytes are always a prefix of post-edit bytes.

### 9.5 Dry-Run Semantics

When `dry_run: true`, `apply_annot_ops`:
- Does **not** modify any files
- Returns `output` containing the original source bytes (unchanged)
- Populates `ir_diff` with an `IrDiff` preview of what would change
- Sets `accepted: true` (as long as the document is writable)

### 9.6 State Change Implementation

State-change operations (Resolve/Reject/Reopen) locate the target annotation by
matching properties (page_number, rect, annotation_type) against the parsed PDF
annotation dictionaries, then create a `PdfMutation::Replace` that updates the
`/Subj` field with the review state string.

### 9.7 Add Operation Implementation

The `Add` operation creates a new PDF annotation object with:
- `/Subtype` — mapped from `AnnotationKind` (e.g., "Highlight", "Text")
- `/Contents` — annotation comment text
- `/Rect` — bounding box
- `/P` — reference to the target page
- `/T` — author name (optional)
- `/State` — set to "Other" for new annotations

### 9.8 Result Type

`AnnotWriteResult` carries:
- `output`: The resulting PDF bytes (original in dry_run)
- `accepted`: Whether the write was accepted
- `diagnostics`: Any warnings or errors
- `annotations_changed`: IDs of annotations whose state was modified
- `annotations_added`: IDs of newly added annotations
- `byte_diff`: Summary of original vs output size
- `ir_diff`: IR diff preview (present in dry_run mode)

## 10. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0-draft | 2026-09-18 | Initial: AnnotationKind, AnnotationState, expanded PdfAnnotation, helpers, GP-A codes |
| 1.1.0-draft | 2026-09-18 | Write-back path: AnnotOp, apply_annot_ops, append-only prefix invariant, dry_run semantics, GP-S-001 |
