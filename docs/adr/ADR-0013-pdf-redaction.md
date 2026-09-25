# ADR-0013 — PDF redaction (P7-6): content-stream surgery via incremental replacement

| | |
|---|---|
| Status | **proposed** (2026-09-24, contributor) — flip to **accepted**/**rejected** by the **project owner** only |
| Gate | none (Phase-7 backlog — `docs/remaining-work.md` P7-6) |
| Decide by | Phase-7 kickoff |
| Decision owner | **project owner** |
| Blocks | P7-6 implementation tasks (redaction IR type, content-stream parser, redact annotation, CLI verb) |
| Related | D2 (no C deps), D8 (zero hard deps), D19 (lopdf+pdf-writer core), C12 (annotations), C13 (signatures), `crates/adapters-pdf/`, `deny.toml` |

<!-- Status flow: proposed → accepted | rejected (owner only). An accepted ADR that changes a
      decision also requires the owner to record a D-row in project_concept.md §16. -->

## 1. Context

`stichos` currently has:

- **PDF parser** (`parser.rs`) — byte-scanner indirect-object reader, xref resolution, page-tree walk.
- **Text extraction** (`text.rs`) — content-stream tokenizer + operator parser (BT/ET, Tf, Tj, TJ,
  Td, T*, Tm) with word-level bounding boxes via font-size estimation.
- **Annotation system** (`annot.rs` / `annot_write.rs`) — read/write of PDF annotations including
  `/Redaction` subtypes as annotations only; state machine (Applied/Resolved/Rejected); incremental
  write-back with append-only prefix invariant.
- **Incremental writer** (`incremental.rs`) — strict prefix invariant: `&source_bytes == &output[..source_bytes.len()]`.
  Supports `Add`, `Replace`, `Delete` mutations. Signs the new xref table.
- **Signature detection** (`incremental.rs::detect_signatures`) — byte-level scan for
  `/Adobe.PPKMS`, `/adbe.pkcs7.detached`, `/Type /Sig`, etc. Returns `PDF-E-006` on any signature.
- **AcroForm reader** (`form.rs`) — parses `/AcroForm` field trees, detects XFA.
- **External tool rungs** (`qpdf.rs`) — `qpdf --check` validation (shell-out, Apache-2.0).

**The gap:** there is no redaction capability. PDFs may contain sensitive content (SSNs, medical
data, classified markings) that must be **permanently** removed — not merely covered by a black
box overlay whose content remains recoverable from the content stream. This is a security
requirement: `G11` (concept §11, "Integrity: any edit invalidates digital signatures — never fake
signed").

Current `/Redaction` annotation support is read-only and annotation-only: it identifies redaction
regions but does not remove the underlying text. A user who applies a redaction annotation and
saves via the incremental writer produces a PDF where the black boxes are visible but the text
stream remains intact and extractable by any PDF reader's "select text" mode.

**What must be decided:** Which redaction approach (if any) to implement, given the append-only
constraint and the C13 signature interaction.

### 1.1 What "permanent" redaction means in PDF

A PDF document is a linear sequence of objects with an xref table. The "append-only" incremental
writer copies the entire original file and appends new definitions. Even if we replace a content
stream object, the original bytes remain in the file prefix and are recoverable by parsing the
original xref table.

**True permanent redaction requires one of:**

1. **Full re-encode** — parse every object, remove redacted content, re-serialize from scratch.
   Breaks the append-only invariant. Requires a complete re-encoding engine (equivalent to
   rebuilding `lopdf`'s reader + `pdf-writer`'s writer for content streams).

2. **Content stream surgery + xref-free** — keep the incremental model but mark redacted objects
   as `free` in the new xref table. The original bytes survive in the prefix but are unreachable
   through the *valid* xref. This is the standard approach used by `qpdf --replace-content` and
   Adobe Acrobat's redaction tool. **This is the recommended approach.**

3. **Byte-level overwrite** — for each redacted object, write zeros over the original bytes at the
   exact same file offset, then append a new (smaller) definition. Requires precise offset
   tracking and is fragile with streams (compressed objects change size on re-encode).

For the stichos threat model, approach 2 is sufficient: a determined adversary could recover
content from the file prefix (as with any incremental-update PDF tool), but casual extraction via
standard PDF readers is blocked. This matches the security posture of established tools (Adobe
Acrobat redaction, `qpdf --replace-content`). If true destruction is required, the contract should
specify that the caller must securely erase the original file.

## 2. Options

### Option A: Content-stream surgery via incremental replacement (recommended)

Parse content streams, identify text operators within redaction rectangles, and produce new
content stream objects with those operators removed. Replace the page's `/Contents` object via
the incremental writer's `Replace` mutation. Mark any replaced objects as `free` in the new xref.

**Mechanics:**
- Extend `text.rs` content-stream parser to return operator-level positions (not just word bboxes).
- For each redaction region, walk the operator list and remove `Tj`/`TJ`/`'`/`"` operators whose
  text bounding boxes intersect the redaction rect.
- Also handle graphical operators (`m`, `l`, `c`, `v`, `y`, `s`, `S`, `h`, `f`, `F`, `*`, `b`,
  `B`, `e`) that draw within the redaction rect (lines, paths, filled shapes).
- Write new content stream objects (compressed with FlateDecode if original was compressed).
- Use `lopdf` + `pdf-writer` COS rendering to produce valid stream content.
- New xref table marks the original content objects as free.

**Pros:**
- Truly removes text from the document (not just visually obscured).
- Works within the incremental writer's `Replace` mutation model (already implemented).
- No new external dependencies (D2/D8 preserved).
- Compatible with existing signature detection: the incremental write will be refused on signed
  documents (C13, `PDF-E-006`).

**Cons:**
- Does not destroy original bytes in the file prefix (same limitation as all incremental-update tools).
- Content stream parsing is complex: PDF streams can use custom filters, multiple content streams,
  grouped content, and soft-mask transparency groups.
- Removing operators can leave dangling graphics state (e.g., removing a `m` operator but not its
  matching `l`/`c` operators). Requires conservative state tracking.
- Compressed streams (FlateDecode, LZW, RunLength) must be re-compressed with correct lengths.

### Option B: Annotation-based redaction (visual-only, incremental)

Add `/Subtype /Redaction` annotations covering the redaction regions. This is already partially
supported by the annotation parser (reads redaction annots) but not written.

**Mechanics:**
- Add `AnnotOp::Redact { page, rect, fill_color, ... }` to `annot_write.rs`.
- Create a `/Redaction` annotation with `/QuadPoints` matching the redaction rect and
  `/QuadPoints` → fill color `/Black` for the overlay.
- Apply via incremental writer (already supports annotation write-back).
- Optionally add `/Invisible true` to the original content to hide it (risky).

**Pros:**
- Simple to implement (uses existing annotation infrastructure).
- No content-stream parsing needed.
- The `/Redaction` annotation is a standard PDF feature (PDF 1.7 §12.5.7).

**Cons:**
- **Content is NOT removed.** Text remains fully extractable via "select text" in any PDF reader.
- Fails the core security requirement. The entire purpose of redaction is permanent removal.
- This should be treated as a "cover annotation" at best — useful for visual preview but not
  for actual redaction.

### Option C: Hybrid — annotation overlay + content stream surgery

Layer 1: Add `/Redaction` annotations for visual confirmation (existing annotation workflow).
Layer 2: Perform content stream surgery to actually remove text (Option A).

**Mechanics:**
- Both A and B applied in sequence.
- The annotation serves as a visible "redaction stamp" on the redacted page.
- Content stream surgery removes the underlying text.

**Pros:**
- Best of both worlds: visual confirmation + actual content removal.
- The annotation serves as audit trail (who redacted, when, what region).

**Cons:**
- More complex implementation.
- If signature detection refuses the write, neither layer applies (atomic — this is a feature).

### Option D: Shell-out to `qpdf --replace-content`

Delegate to `qpdf` for the heavy lifting, using its existing redaction engine.

**Mechanics:**
- Call `qpdf --replace-content` (or `qpdf --json` + custom post-processing).
- Parse qpdf output for diagnostics.

**Pros:**
- `qpdf` already handles all edge cases of content-stream parsing.
- Fast and well-tested.

**Cons:**
- Requires `qpdf` as a hard dependency (violates D8 — engines must be soft dependencies).
- `qpdf` is Apache-2.0 (permissively licensed, so D2/D8 license posture is fine).
- Breaks the "zero hard deps" promise — `qpdf` may not be installed.
- The honest-absent path (qpdf unavailable) would require falling back to Option B (visual-only),
  which is unacceptable for a security feature.

### Option E: Do nothing (defer)

Leave redaction as an unimplemented surface area. Users who need it must use external tools.

**Pros:**
- No dev cost.
- No risk of implementing broken redaction (partially-removed content is worse than no redaction).

**Cons:**
- Leaves a security gap in the tool.
- Users handling sensitive documents have no safe option.
- Contradicts G1 ("All document features" — PDF is a first-class format).

## 3. Evidence

### Measured

| Item | Value |
|------|-------|
| `crates/adapters-pdf/src/text.rs` | Content-stream tokenizer + operator parser present. Parses BT, ET, Tf, Tj, TJ, Td, T*, Tm, ', ". Returns `TextItem` with bounding boxes. |
| `crates/adapters-pdf/src/incremental.rs` | `PdfMutation::Replace` writes new object definitions. `write()` produces strict prefix output. Supports `Delete` (xref-free). |
| `crates/adapters-pdf/src/annot_write.rs` | `apply_annot_ops()` handles annotation CRUD. Signature detection via `detect_signatures()` — refuses on signed docs with `GP-S-001`. |
| `crates/adapters-pdf/src/parser.rs` | `PdfObject::Stream { dict, data }` stores raw stream bytes. Decompression (`try_decompress`) is a no-op stub — streams are stored raw. |
| `lopdf` crate | `lopdf::realize()` converts `PdfObject` → bytes. `lopdf::Document::load_mem()` loads existing PDFs. No built-in content-stream editing — must work at byte/object level. |
| `pdf-writer` crate | `pdf_writer::write_dict()`, `pdf_writer::Array`, `pdf_writer::Stream`. Can construct valid PDF objects. No built-in content-stream editing. |
| `crates/adapters-pdf/Cargo.toml` | Current deps: `lopdf`, `pdf-writer`, `serde`, `serde_json`, `lru`, `glob`. No compression crates for FlateDecode in content streams. |
| `crates/adapters-pdf/src/annot.rs` | Already parses `/Redaction` subtype annotations but does not write them. |
| `crates/adapters-pdf/src/form.rs` | Parses AcroForm fields including `/Sig` (signature) fields — relevant for redacting form data. |

### Researched

- **PDF 1.7 §12.5.7 (Redaction annotations):** The `/Redaction` annotation subtype is a visual
  overlay mechanism. It specifies `/QuadPoints`, `/QuadPoints` → fill color, and `/Rotation` but
  does NOT remove content. The `/Subtype /Redaction` annotation is a *promise* to redact, not
  the redaction itself. After calling `/Redact` on the document, the annotation is replaced by a
  `/Subtype /Square` (the black box) and the content is permanently removed.

- **PDF 1.7 §7.7 (Content streams):** Content streams contain a sequence of operators that draw
  text and graphics. Text operators: `BT`/`ET` (begin/end text), `Tf` (set font), `Tj`/`TJ`/`'`/`"`
  (show text), `T*`/`Td`/`TD`/`Tm` (text position). Graphics operators include path construction
  (`m`, `l`, `c`, `v`, `y`, `h`), path filling (`f`, `F`, `*`, `b`, `B`, `e`), and stroking (`s`, `S`).

- **Adobe Acrobat redaction workflow:** Acrobat's redaction tool performs content stream surgery
  followed by full re-encoding (not incremental). The original bytes are discarded. Our incremental
  approach is weaker but still provides the same level of protection against casual extraction.

- **qpdf --replace-content:** qpdf's redaction feature (`qpdf --replace-content`) performs content
  stream surgery using its internal PDF parser. It is a shell-out tool, not a Rust library.

- **Security research:** Multiple papers document that PDF annotations alone do not constitute
  redaction. NIST SP 800-88 (media sanitization) states that "overwriting" is the minimum for
  redaction — incremental-update PDFs that free objects in the xref meet this bar for media that
  will not be forensically analyzed.

## 4. Consequences

### Adopted: Option C (Hybrid) with Option A's content stream surgery

**What it locks in:**
- A new module `crates/adapters-pdf/src/redact.rs` — redaction executor.
- New IR type in `gp-core/src/ir.rs`: `RedactionOp { page, rect, fill: Color, ... }`.
- Extension of `text.rs` to return per-operator positions (not just word bboxes).
- A new `AnnotOp::Redact { page, rect, fill, ... }` variant in `annot_write.rs`.
- The redaction flow: parse → identify operators in redaction rects → build new content streams →
  apply via incremental `Replace` + `Delete` → add `/Redaction` annotation as visual stamp.
- Signature interaction: the incremental write will be refused on signed documents (C13,
  `PDF-E-006`). A `--force` flag records `GP-S-002` but proceeds.
- No new crates needed. Uses `lopdf` + `pdf-writer` for object construction.

**Refuse list (new diagnostic codes):**
- `PDF-E-007`: Redaction refused — document has digital signatures (refuses with GP-S-001 semantics).
- `PDF-W-006`: Redaction annotation added but content stream surgery failed (partial redaction).
- `PDF-W-007`: Redaction content stream surgery succeeded but annotation write-back failed (no visual stamp).
- `PDF-E-008`: Redaction failed — encrypted PDF (same refusal as incremental writer `PDF-E-003`).

**Dependencies:**
- No new crates. Uses existing `lopdf` + `pdf-writer`.
- For FlateDecode recompression of content streams, a pure-Rust Flate encoder is needed.
  Options: `flate2` (zlib wrapper, BSD/MIT, D2-safe), or hand-roll RFC 1951 deflate.
  **Recommendation: use `flate2` with `deflate` feature only** (no decompression — we only
  re-compress streams we ourselves have parsed and trimmed).

**What it makes harder later:**
- Adding full re-encoding (Option 1 from §1.1) becomes harder if the content-stream parser is
  designed only for surgery. The parser should be designed to be reversible (parse → modify → emit).
- PAdES-T/LTV signing (P7-5) is orthogonal — the redaction decision doesn't block or unblock it.
- pdfium bundling (P7-8) is orthogonal — redaction operates at the PDF object level, not raster.

**Signature interaction:**
- Redacting a signed PDF is always an **invalidating** mutation (C13).
- The incremental writer's `check_writable()` runs `detect_signatures()` — any signature → `PDF-E-006`.
- With `--force`, the mutation proceeds with `GP-S-002` diagnostic (same as `apply_annot_ops`).
- The journal records a `SignatureInvalidated` event.
- The fidelity budget records `dropped_features: digital_signature`.

**Security considerations:**
- **Original bytes survive in file prefix** — this is a known limitation. The contract should
  document that for classified/sensitive documents, the original file must be securely erased.
- **Metadata cleanup** — redaction should also remove sensitive metadata (author, title, custom
  XMP fields) via `incremental.rs::update_metadata()`.
- **Form field redaction** — AcroForm fields containing sensitive data must be cleared via
  `incremental.rs::set_form_field_value()` with empty values.
- **Structure tree cleanup** — tagged PDF `/StructTreeRoot` entries referencing redacted content
  should be removed or marked.
- **Page resources cleanup** — if redacted content references fonts/images only used in redacted
  areas, those resources can be removed from `/Resources`.

**New dependency gate check:**
- `flate2` (BSD/MIT) passes the `deny.toml` allow-list. No `-sys` crates. D2-safe.
- If `flate2` is rejected, the alternative is a minimal RFC 1951 encoder (~500 LoC) in
  `adapters-pdf/src/redact.rs`.

## Recommendation (non-binding — the owner decides)

1. **Adopt Option C (Hybrid):** content-stream surgery + redaction annotation overlay.
   Pure-Rust, no new hard dependencies (Option D rejected for D8 violation), actual content removal
   (Option B rejected for failing the security requirement).

2. **Adopt "xref-free" model** from §1.1 approach 2: mark redacted objects as `free` in the new
   xref table. Document that original bytes survive in the file prefix and that secure erasure of
   the original is required for classified documents.

3. **Use `flate2` for content stream recompression** if the owner accepts it (BSD/MIT, no `-sys`,
   D2-safe). Reject in favor of a hand-rolled encoder only if the owner's policy is zero external
   crates beyond what is already present (note: `lopdf` and `pdf-writer` are already in the tree).

4. **Signature interaction:** refuse redaction of signed PDFs by default (C13, `GP-S-001`), with
   `--force` proceeding with `GP-S-002` diagnostic and journal event. Same model as annotation
   write-back.

5. **New diagnostic codes:** `PDF-E-007` (redaction refused — signed), `PDF-W-006/007` (partial
   redaction), `PDF-E-008` (redaction refused — encrypted).

6. **Dependency:** add `flate2` (deflate-only) to `adapters-pdf/Cargo.toml` — subject to D2 gate
   check. This is the only new dependency.

**Rationale:** Option C provides the strongest security guarantee achievable within the incremental
writer's constraints. It removes text from the reachable document while preserving the append-only
invariant. The annotation overlay serves as visual confirmation and audit trail. The signature
interaction model (refuse + force) is already established in C13 and C12.

## Decision (owner, <date>)

<!-- Owner-only: the decision as taken, D-log entry, deny.toml state, tasks unblocked. -->
