# C11 — Revision Model Contract

| | |
|---|---|
| Owner | CORE |
| Consumers | all adapters (ooxml, odf), engines, lint, mcp, cli |
| Freeze | **frozen at M4** |
| Version | 1.0.0 |
| Status | frozen at M4 |
| Artifacts | `crates/gp-core/src/ir.rs` (Revision, RevisionKind, RevisionOp, Document.revisions), `crates/gp-core/src/diagnostics.rs` (GP-R001..GP-R004) |
| Conformance | serde roundtrip + C1 additive + refuse-list + two-tier losslessness |

## 1. Scope

This contract defines the revision model in the stichos intermediate representation (IR).
Revisions capture insert, delete, and format-change operations on document content.
They replace the older `TrackedChange` type and provide a richer, more structured model
for the human-loop (Phase 4) surface.

## 2. Types

### 2.1 RevisionKind

```rust
pub enum RevisionKind {
    Insert,
    Delete,
    FormatChange,
}
```

Serialized as snake_case JSON strings: `"insert"`, `"delete"`, `"format_change"`.

### 2.2 Revision

```rust
pub struct Revision {
    pub id: ElementId,
    pub kind: RevisionKind,
    pub author: Option<String>,
    pub date: Option<String>,
    pub anchor_element_id: ElementId,
    pub page_hint: Option<usize>,
    pub content: String,
    pub property_name: Option<String>,
    pub original_value: Option<String>,
    pub new_value: Option<String>,
}
```

- **id**: Stable unique identifier (C2 format: `id-<hex>`).
- **kind**: The type of revision operation.
- **author**: Human-readable author name (optional; warning GP-R-003 if missing).
- **date**: ISO-8601 timestamp of the revision (optional).
- **anchor_element_id**: The C2 element ID this revision is anchored to. See §4.
- **page_hint**: Page number hint (derivable, optional).
- **content**: For `Insert` — the new content. For `Delete` — the deleted content.
  For `FormatChange` — the property value description.
- **property_name**: For `FormatChange` only — the name of the changed property
  (e.g. `"bold"`, `"font_size"`).
- **original_value**: For `FormatChange` only — the value before the change.
- **new_value**: For `FormatChange` only — the value after the change.

### 2.3 Document.revisions

The `Document` struct gains a field:

```rust
#[serde(skip_serializing_if = "Vec::is_empty", default)]
pub revisions: Vec<Revision>,
```

The `skip_serializing_if` ensures backward compatibility: documents with no revisions
do not emit the field in JSON. See C1 additive rule (§8).

### 2.4 RevisionOp

```rust
#[serde(tag = "op")]
pub enum RevisionOp {
    Accept { revision_id: ElementId },
    Reject { revision_id: ElementId },
    AcceptAll,
    RejectAll,
}
```

Serialized with a discriminator `"op"` field (snake_case):
- `{"op":"accept","revision_id":"id-..."}`
- `{"op":"reject","revision_id":"id-..."}`
- `{"op":"accept_all"}`
- `{"op":"reject_all"}`

## 3. Accept/Reject Semantics

### 3.1 Accept

- **Insert**: unwrap the inserted content — the content becomes part of the document
  at the anchor point, the revision entry is removed.
- **Delete**: remove the deleted content from the document — the content that was
  deleted is no longer present, the revision entry is removed.

### 3.2 Reject

- **Insert**: remove the inserted content from the document — reverses the insert,
  the revision entry is removed.
- **Delete**: restore the deleted content — puts back the content captured in the
  revision's `content` field at the anchor point, the revision entry is removed.

### 3.3 Bulk Operations

- **AcceptAll**: apply accept to all revisions atomically.
- **RejectAll**: apply reject to all revisions atomically, restoring byte-identical
  original content (Tier 1, see §7).

## 4. Refuse List

The following revision kinds and patterns are **refused** (rejected with diagnostic):

| Refused Pattern | Diagnostic Code | Behavior |
|-----------------|-----------------|----------|
| Move-revs (move-from/move-to) | GP-R-001 | "Unsupported revision kind: {} — refusing to process" |
| Unknown revision kinds | GP-R-001 | Same error with unknown kind name |
| Format-change revisions | GP-R-001 | Refused: scope is insert/delete only for M4 |

Format-change revisions are defined in the model but not yet implemented.
Support is reserved for future waves (P4-xx beyond M4).

## 5. Anchoring

### 5.1 C11 Anchoring

Revisions are anchored by ElementId (C2 contract). The `anchor_element_id` field
references a stable C2 element ID in the document. The adapter is responsible for
resolving this anchor to the correct position in the source document.

### 5.2 Format-Change Anchoring

Format-change revisions use the format-run element as the anchor. This ensures
that formatting changes target the correct text run within a paragraph.

### 5.3 Nested Revisions

Nested revisions (e.g., `ins` inside `del`, or `del` inside `ins`) are handled
by depth-first traversal. The outer revision is processed first, then inner
revisions in document order.

## 6. Two-Tier Losslessness

### 6.1 Tier 1: Own Revisions

For revisions produced by stichos itself:

- **Reject-All**: `reject_all` ⇒ byte-identical to the original document (before
  any revisions were applied). This is a hard guarantee.
- **Accept-All**: `accept_all` ⇒ IR-equal to what would result from direct editing
  (applying the changes without the revision layer). Byte identity is not promised
  for the accept path (formatting normalization may differ).

### 6.2 Tier 2: Foreign Revisions

For revisions imported from other systems (e.g., MS Word `.docx` revision marks):

- **Accept/Reject**: asserted IR equality + visible-text equality. That is, the
  resulting IR, when rendered, produces the same visible text as if the user had
  manually accepted/rejected the revisions in the source application.
- Byte identity is **not** promised (foreign formatting may differ from stichos's
  canonical output).

## 7. C1 Additive Compatibility

Old IR documents (without the `revisions` field) deserialize into a new `Document`
with `revisions` defaulting to an empty `Vec`. This is enforced by the
`skip_serializing_if = "Vec::is_empty"` serde attribute and the `default`
deserialization behavior.

```rust
// Old IR (no revisions field)
let json = r#"{"ir_version":"1.0.0","sections":[...]}"#;
let doc: Document = serde_json::from_str(json).unwrap();
assert!(doc.revisions.is_empty()); // C1 additive
```

## 8. Diagnostic Codes

| Code | Severity | Category | Description |
|------|----------|----------|-------------|
| GP-R-001 | Error | parsing | Unsupported revision kind: {} — refusing to process |
| GP-R-002 | Error | validation | Ambiguous revision anchor: {} has no matching element |
| GP-R-003 | Warning | parsing | Revision {} has no author and none provided |
| GP-R-004 | Error | execution | Revision accept/reject failed: {} |

## 9. Migration from TrackedChange

The `TrackedChange` type remains in the IR for backward compatibility. It maps to
`Revision` as follows:

| TrackedChange field | Revision field |
|---------------------|----------------|
| `id` | `id` |
| `kind: Inserted` | `kind: Insert`, `content` |
| `kind: Deleted` | `kind: Delete`, `content` |
| `author` | `author` |
| `date` | `date` |
| _(none)_ | `anchor_element_id` _(new)_ |
| _(none)_ | `page_hint` _(new)_ |
| _(none)_ | `property_name` _(new)_ |
| _(none)_ | `original_value` _(new)_ |
| _(none)_ | `new_value` _(new)_ |

Migration from `TrackedChange` → `Revision` adds anchoring information (C2) which
is derived by the adapter during read. The reverse (Revision → TrackedChange) is
lossy for format-change revisions.

## 10. Revision history

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0 | — | Initial draft |
| 1.0.0 | 2026-09-16 | Frozen at M4: Revision, RevisionKind, RevisionOp, Document.revisions, GP-R codes, two-tier losslessness, refuse list |
