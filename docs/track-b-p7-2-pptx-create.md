# P7-2 — pptx create: content-driven pptx from IR

**Created:** 2026-09-24. **Task:** P7-2 from `docs/remaining-work.md` Track-B.

---

## 1. Current state assessment

### What exists

| Component | File | State |
|---|---|---|
| PPTX creation writer | `crates/adapters-ooxml/src/create_pptx.rs` | **Exists.** `create_minimal_pptx(&Presentation) -> Result<Vec<u8>, Vec<Diagnostic>>` works: writes OPC package with `[Content_Types].xml`, `_rels/.rels`, `ppt/presentation.xml`, `ppt/_rels/presentation.xml.rels`, `ppt/theme/theme1.xml`, `ppt/slides/slideN.xml`, `ppt/slideLayouts/slideLayout1.xml`, `ppt/slideMasters/slideMaster1.xml`, `docProps/core.xml`, `docProps/app.xml`. 10 unit tests pass. |
| PPTX creation dispatcher | `crates/adapters-ooxml/src/create.rs` | **Not present for pptx.** This file only dispatches `create_minimal_docx`. No `create_pptx` import or dispatch path. |
| PPTX reader (round-trip input) | `crates/adapters-ooxml/src/pptx.rs` | **Complete.** `read_pptx(&OpcPackage) -> ReadResult` parses OPC → Presentation IR. Parses slides, layouts, masters, text shapes, placeholders, tables, graphic frames, group shapes. Includes 7+ corpus test fixtures. |
| PPTX patch writer | `crates/adapters-ooxml/src/pptx_writer.rs` | **Exists.** `PptxPatchWriter` applies PatchOps via span mutations to existing pptx. Not creation — surgical edits only. |
| Presentation IR types | `crates/gp-core/src/ir.rs` | **Present but minimal.** `Presentation`, `Slide`, `SlideElement`, `Layout`, `Master`. `SlideElement.kind` is a `String` (e.g. "text", "shape", "image", "table", "drawing", "graphic") with no further fields. |
| CLI `create` verb | `crates/cli/src/main.rs` | **Rejects pptx.** `cmd_create` validates `target_format` to only `docx | odt | pdf`. Error message: `"invalid format '{}': must be one of: docx, odt, pdf"`. |
| Engine creation API | `crates/engines/src/creation.rs` | **Rejects pptx.** `VALID_TARGET_FORMATS = &["docx", "odt", "pdf"]`. `validate_args()` rejects `pptx`. |
| CreateTarget enum | `crates/gp-core/src/ir.rs` | **No pptx variant.** Only `Docx`, `Odt`, `Pdf`. |
| create_template | `crates/adapters-ooxml/src/create_template.rs` | **docx-only.** Template-based creation for docx only. No pptx template variant. |
| C1 Presentation IR contract | `docs/contracts/C1.md` | **Present (section 3.3).** Documents Presentation IR schema. |

### What does NOT exist

- `CreateTarget::Pptx` variant
- `pptx` in `VALID_TARGET_FORMATS` or CLI format validation
- Engine dispatch path for `pptx` in `create_document_engine`
- `adapters_ooxml::create_pptx` import in `create.rs`
- `engines::creation` routing to pptx writer
- `SlideElement` IR fields beyond `id` + `kind` string (no text content, no position/size, no image data)

### Key observation: `create_pptx.rs` is structurally complete but functionally shallow

The writer produces a **valid** OPC package (ZIP + correct XML namespaces), but the slide content generation is placeholder-driven:

- `build_slide_element_xml` (`create_pptx.rs:389`) assigns positions/size via hash of element index — not from IR data
- `build_text_element` (`create_pptx.rs:408`) renders text elements as placeholder graphicFrame with a fake blip relationship — does NOT emit `<p:txBody>` with actual text
- `build_shape_element` (`create_pptx.rs:449`) generates a blue rectangle with element name as text — ignores any shape-specific IR data
- `build_table_element` (`create_pptx.rs:534`) emits a skeleton `<p:tbl>` with `<p:rowRef>` placeholders — no actual cell data
- `build_image_element` (`create_pptx.rs:493`) emits a graphicFrame with a fake `r:embed` — no actual image data or media parts
- `build_layout_xml` (`create_pptx.rs:585`) generates layout XML without placeholder position/size (only `name`)
- `build_master_xml` (`create_pptx.rs:625`) generates master XML without content placeholders or theme overrides

The `SlideElement` IR type only carries `id` + `kind: String`, so there is **no data to use** even if the writer wanted to. The IR needs enrichment before content-driven creation can be meaningful.

---

## 2. C1 Presentation IR — fields and types

All types are in `crates/gp-core/src/ir.rs` (lines 929-971).

```rust
pub struct Presentation {
    pub ir_version: String,                      // always "1.0.0"
    pub ir_version_migration: Option<String>,
    pub slides: Vec<Slide>,                      // one or more slides
    pub layouts: Vec<Layout>,                    // slide layout definitions
    pub masters: Vec<Master>,                    // slide master definitions
}

pub struct Slide {
    pub id: ElementId,                           // content-anchored
    pub elements: Vec<SlideElement>,             // content elements on the slide
}

pub struct SlideElement {
    pub id: ElementId,
    pub kind: String,                            // "text" | "shape" | "image" | "table" | "drawing" | "graphic"
}

pub struct Layout {
    pub id: ElementId,
    pub name: String,                            // e.g. "Title Slide"
}

pub struct Master {
    pub id: ElementId,
    pub name: String,                            // e.g. "Office Theme"
}
```

**What's missing from the IR for creation:**

1. **Text elements need content + formatting.** Currently `kind` only says "text" — the actual text content, font, size, bold/italic, alignment are all absent. The writer has to synthesize placeholder names from element indices.

2. **Position and size are not in the IR.** The writer computes `(x, y, w, h)` from element index hash (`create_pptx.rs:391`). Real creation needs `x_pt`, `y_pt`, `width_pt`, `height_pt` or a `bbox_pt: [f64; 4]` field on `SlideElement`.

3. **Image elements need data.** Currently `kind == "image"` produces a graphicFrame with a fake `r:embed` relationship and no media part. Real creation needs `data: Vec<u8>` (base64-encoded image bytes) + `mime_type: String` on the element, plus the creation writer must emit `ppt/media/` parts.

4. **Table elements need cell data.** Currently `kind == "table"` emits a skeleton `<p:tbl>` with 3 row refs and 2 col refs. Real creation needs row count, column count, and cell content (strings/text) per cell — similar to `SectionItem::Table` in the docx IR.

5. **Shape elements need geometry.** Currently `kind == "shape"` produces a blue rectangle with a name. Real creation needs `preset: String` (rect, ellipse, diamond, triangle, etc.), fill color, line color/width.

6. **Layouts need placeholder definitions.** Currently `Layout` only has `name`. Real creation needs placeholder positions/sizes for title, content, etc. — mapped to `<p:ph>` elements in the layout XML.

7. **Masters need content placeholders and color/font schemes.** Currently `Master` only has `name`. The writer generates a hardcoded "Office Theme" master with a static color map.

8. **No `IRDocument` wrapper for presentations.** The `IRDocument` type (`ir.rs:1209`) is a tagged tree for PDF, not a cross-format wrapper. There is no `{ presentation: Presentation }` or `{ document: Document }` envelope.

---

## 3. Gap analysis

### Gap A: IR enrichment (highest priority, blocking)

| Gap | File(s) | Description |
|---|---|---|
| A1 | `crates/gp-core/src/ir.rs` | Enrich `SlideElement` with content-bearing fields: `text: Option<String>`, `bold: Option<bool>`, `italic: Option<bool>`, `font_name: Option<String>`, `font_size_pt: Option<f64>`, `x_pt: Option<f64>`, `y_pt: Option<f64>`, `width_pt: Option<f64>`, `height_pt: Option<f64>`, `data: Option<Vec<u8>>`, `mime_type: Option<String>`, `preset: Option<String>`, `fill_color: Option<String>`, `line_color: Option<String>`. |
| A2 | `crates/gp-core/src/ir.rs` | Add `Table` to Presentation IR: `rows: Vec<TableRow>` where `TableRow { cells: Vec<TableCell> }` and `TableCell { contents: Vec<TextRun> }`. |
| A3 | `crates/gp-core/src/ir.rs` | Enrich `Layout` with placeholder definitions: `placeholders: Vec<Placeholder>` where `Placeholder { name: String, type_: String, x: f64, y: f64, w: f64, h: f64 }`. |
| A4 | `crates/adapters-ooxml/src/pptx.rs` | Update pptx→IR reader to populate the enriched fields (text content, positions, image data, table cell contents). |

### Gap B: Writer content depth (high priority)

| Gap | File(s) | Description |
|---|---|---|
| B1 | `crates/adapters-ooxml/src/create_pptx.rs` | Rewrite `build_text_element` to emit actual `<p:txBody>` with `<a:p>`, `<a:rPr>`, `<a:t>` using `SlideElement.text`, formatting fields. |
| B2 | `crates/adapters-ooxml/src/create_pptx.rs` | Rewrite `build_image_element` to emit `<p:pic>` with `<a:blip r:embed="rIdN"/>` and generate `ppt/media/imageN.<ext>` parts from `SlideElement.data`. |
| B3 | `crates/adapters-ooxml/src/create_pptx.rs` | Rewrite `build_table_element` to emit actual `<p:tbl>` with `<p:tr>` and `<p:tc>` elements containing text content. |
| B4 | `crates/adapters-ooxml/src/create_pptx.rs` | Rewrite `build_shape_element` to support preset geometry types (`<a:prstGeom prst="...">`) and configurable fills/lines. |
| B5 | `crates/adapters-ooxml/src/create_pptx.rs` | Rewrite `build_slide_element_xml` to use actual position/size from IR instead of hash-based synthetic values. |
| B6 | `crates/adapters-ooxml/src/create_pptx.rs` | Add `build_media_parts()` helper to extract image data from slides and generate `ppt/media/` entries with correct content types. |
| B7 | `crates/adapters-ooxml/src/create_pptx.rs` | Wire media parts into OPC packaging (current `pack_opc` only takes `(uri, data)` tuples from slide/build parts). |

### Gap C: Engine/CLI integration (medium priority)

| Gap | File(s) | Description |
|---|---|---|
| C1 | `crates/gp-core/src/ir.rs` | Add `Pptx` to `CreateTarget` enum + `pandoc_format()` arm returning `"pptx"`. |
| C2 | `crates/engines/src/creation.rs` | Add `"pptx"` to `VALID_TARGET_FORMATS`; add routing in `create_document_engine` for `CreateTarget::Pptx`. |
| C3 | `crates/cli/src/main.rs` | Add `"pptx"` to CLI format validation in `cmd_create`. |
| C4 | `crates/adapters-ooxml/src/create.rs` | Import `create_pptx` module and dispatch `create_minimal_pptx` from a `CreatePptx` builder. |
| C5 | `crates/engines/src/creation.rs` | Create `create_via_pptx_writer()` engine function: takes `Presentation` IR → `create_minimal_pptx` → returns `CreationResponse`. |
| C6 | `crates/gp-core/src/ir.rs` | Add `CreateSource::Presentation { ir_bytes: Vec<u8> }` variant to accept pre-built IR JSON for pptx output. |

### Gap D: Presentation IR contract doc (low priority, documentation)

| Gap | File | Description |
|---|---|---|
| D1 | `docs/contracts/C1-presentation-ir.md` | Create standalone contract doc (pattern from C1, C7, C11) documenting the Presentation IR schema, constraints, and versioning rules. |

### Gap E: Template-based pptx creation (future)

| Gap | File | Description |
|---|---|---|
| E1 | `crates/adapters-ooxml/src/create_pptx_template.rs` | **New file.** Template-based pptx creation: preserve theme/master/styles from `.pptx` template, replace slide content from IR. Pattern from `create_template.rs`. |

---

## 4. Implementation plan

### Phase 1: IR enrichment (blocking, M)

1. **1a.** Enrich `SlideElement` in `gp-core/src/ir.rs`.
   - Add fields: `text: Option<String>`, `x_pt: Option<f64>`, `y_pt: Option<f64>`, `width_pt: Option<f64>`, `height_pt: Option<f64>`, `data: Option<Vec<u8>>`, `mime_type: Option<String>`, `preset: Option<String>`, `fill_color: Option<String>`, `line_color: Option<String>`, `bold: Option<bool>`, `italic: Option<bool>`, `font_name: Option<String>`, `font_size_pt: Option<f64>`, `alignment: Option<String>`.
   - Add `TableCell` type: `id: ElementId, contents: Vec<TextRun>` where `TextRun { text: String, bold: Option<bool>, ... }`.
   - Add `TableRow` type: `id: ElementId, cells: Vec<TableCell>`.
   - Add `Table` type to Presentation family (separate from docx `SectionItem::Table`): `id: ElementId, rows: Vec<TableRow>, col_widths: Option<Vec<f64>>`.
   - Add `Placeholder` type: `id: ElementId, name: String, ph_type: String, x: f64, y: f64, w: f64, h: f64`.
   - Enrich `Layout`: add `placeholders: Vec<Placeholder>`, `layout_type: String`.
   - Enrich `Master`: add `background_color: Option<String>`, `color_scheme: Option<BTreeMap<String, String>>`.
   - Size: **M**

2. **1b.** Add `Visit` impl for enriched Presentation types.
   - Populate IdMap from slides → elements → placeholders.
   - Size: **S**

3. **1c.** Update `pptx.rs` reader to populate enriched fields.
   - Extract text from `<a:t>` elements → `SlideElement.text`.
   - Extract position from `<a:off x="..." y="..."/>` → `SlideElement.x_pt`, `y_pt`.
   - Extract dimensions from `<a:ext cx="..." cy="..."/>` → `SlideElement.width_pt`, `height_pt`.
   - Extract shape preset from `<a:prstGeom prst="..."/>` → `SlideElement.preset`.
   - Extract table cell content → `TableCell.contents`.
   - Extract placeholder info from `<p:ph type="..."/>` → `Placeholder`.
   - Size: **M**

### Phase 2: Writer content depth (L)

4. **2a.** Rewrite `build_text_element` in `create_pptx.rs`.
   - Emit `<p:txBody>` with `<a:bodyPr>`, `<a:p>`, `<a:rPr>`, `<a:t>` elements.
   - Map `SlideElement.text` → `<a:t>`.
   - Map formatting fields → `<a:rPr>` attributes.
   - Size: **S**

5. **2b.** Add `build_media_parts` and wire image data into OPC.
   - Collect image data from `SlideElement` entries where `mime_type` is set.
   - Emit `ppt/media/imageN.ext` parts in the ZIP.
   - Update relationship XML (`ppt/_rels/slideN.xml.rels`) with `<Relationship Id="rIdN" Type="..." Target="media/imageN.ext"/>`.
   - Update `<a:blip r:embed="rIdN"/>` in slide XML.
   - Size: **M**

6. **2c.** Rewrite `build_table_element` in `create_pptx.rs`.
   - Emit actual `<p:tbl>` with `<p:tblPr>`, `<p:tblGrid>`, `<p:tr>`, `<p:tc>`, `<p:txBody>`.
   - Fill cell content from `TableCell.contents`.
   - Size: **S**

7. **2d.** Rewrite `build_shape_element` in `create_pptx.rs`.
   - Support preset geometries: `rect`, `ellipse`, `diamond`, `triangle`, `rectangle`, `star`, etc.
   - Emit configurable `<a:solidFill>`, `<a:ln>` for fill/line colors.
   - Size: **S**

8. **2e.** Rewrite position/size assignment.
   - Use `SlideElement.x_pt`, `y_pt`, `width_pt`, `height_pt` from IR.
   - Convert from points to EMU (1 pt = 914400 EMU) for PowerPoint XML.
   - Fall back to default grid if not provided (maintain backward compat).
   - Size: **S**

9. **2f.** Enrich layout and master generation.
   - Add `<p:ph>` placeholder definitions to layout XML from `Layout.placeholders`.
   - Add background and color scheme to master XML from `Master` fields.
   - Size: **S**

10. **2g.** Add `CreatePptx` builder to `create.rs`.
    - Add `mod create_pptx;` to `lib.rs` (already present).
    - Add `CreatePptx` builder struct parallel to `CreateDocx`.
    - Add `create_minimal_pptx()` public wrapper.
    - Size: **S**

### Phase 3: Engine/CLI integration (M)

11. **3a.** Add `Pptx` to `CreateTarget` in `gp-core/src/ir.rs`.
    - Add arm: `CreateTarget::Pptx => "pptx"`.
    - Size: **S**

12. **3b.** Add `CreateSource::Presentation { ir_bytes: Vec<u8> }`.
    - Add `format_name()` arm: `"presentation"`.
    - Size: **S**

13. **3c.** Update CLI format validation in `cmd_create`.
    - Add `"pptx"` to match arm.
    - Size: **S**

14. **3d.** Update `engines/src/creation.rs`:
    - Add `"pptx"` to `VALID_TARGET_FORMATS`.
    - Add routing for `CreateSource::Presentation` → `CreateTarget::Pptx`.
    - Create `create_via_pptx_writer()` engine function.
    - Size: **M**

### Phase 4: Template creation and C1 contract (M)

15. **4a.** Create `create_pptx_template.rs`.
    - Parse `.pptx` template as OPC.
    - Preserve theme, master, layout parts byte-for-byte.
    - Replace slide content from IR.
    - Size: **L** (complex — pptx template parts are more varied than docx)

16. **4b.** Create `docs/contracts/C1-presentation-ir.md`.
    - Document all Presentation IR types, constraints, versioning.
    - Size: **S**

### Phase 5: Round-trip verification (M)

17. **5a.** Create round-trip corpus tests.
    - Reuse existing `cjk-text-pptx-001.pptx` and other 20+ corpus fixtures.
    - Test: IR → pptx → parse → IR equality (semantic, not byte-identical).
    - Size: **M**

18. **5b.** Add integration test in `engines/src/creation.rs`.
    - Test `create_document` with `pptx` target, verify PK header.
    - Size: **S**

19. **5c.** Add corpus-based regression tests.
    - Generate pptx from corpus-IR → render → compare text extraction.
    - Size: **M**

---

## 5. Verification strategy

### Unit tests (in `create_pptx.rs`)

**Existing tests (10, all pass):**

| Test | What it verifies |
|---|---|
| `test_minimal_pptx_creation` | Empty Presentation → valid OPC |
| `test_pptx_with_slides` | Slides generate `ppt/slides/slideN.xml` |
| `test_pptx_text_element` | Text elements → graphicFrame (placeholder — needs rewrite) |
| `test_pptx_layouts_masters` | Layouts/masters → correct part names |
| `test_pptx_roundtrip` | OPC parts present with correct namespaces |
| `test_pptx_with_shapes` | Shape elements → `<p:sp>` |
| `test_pptx_with_images` | Image elements → `<p:pic>` (placeholder — needs rewrite) |
| `test_pptx_with_table` | Table elements → `<p:tbl>` (placeholder — needs rewrite) |
| `test_escape_xml` | XML special char escaping |
| `test_content_types`, `test_theme` | OPC metadata correctness |

**New tests to add (after IR enrichment + writer rewrite):**

| Test | What it verifies |
|---|---|
| `test_pptx_text_element_with_content` | `SlideElement { text: "Hello" }` → actual `<a:t>Hello</a:t>` in slide XML |
| `test_pptx_image_element_with_data` | `SlideElement { data: [...], mime_type: "image/png" }` → `ppt/media/image1.png` part with correct relationship |
| `test_pptx_table_with_cells` | `SlideElement { kind: "table", rows: [...] }` → actual `<p:tr>/<p:tc>` with cell text |
| `test_pptx_shape_with_preset` | `SlideElement { preset: Some("ellipse"), fill_color: Some("#E25840") }` → correct `<a:prstGeom>` and `<a:solidFill>` |
| `test_pptx_with_positions` | `SlideElement { x_pt: 100.0, y_pt: 200.0, ... }` → correct EMU conversion in `<a:off>` |
| `test_pptx_multiple_media_parts` | Multiple images → multiple `ppt/media/` entries with unique relationship IDs |
| `test_pptx_roundtrip_identity` | IR → pptx → parse → IR equality (semantic) |
| `test_pptx_layout_placeholders` | `Layout { placeholders: [...] }` → `<p:ph>` elements in layout XML |
| `test_pptx_master_scheme` | `Master { color_scheme: {...} }` → `<a:clrScheme>` in master XML |
| `test_pptx_empty_presentation` | Empty slides vec → valid minimal OPC (no slide parts, default layout/master) |

### Round-trip test pattern

```rust
#[test]
fn test_pptx_create_roundtrip() {
    let presentation = make_presentation_with_text_and_images();
    // 1. Create pptx from IR
    let bytes = create_minimal_pptx(&presentation).expect("create");
    // 2. Parse pptx back to IR
    let pkg = parse_opc(&bytes).expect("opc parse");
    let read_result = read_pptx(&pkg).expect("read");
    let presentation_back: Presentation = serde_json::from_slice(&read_result.ir).expect("deserialize");
    // 3. Verify slide count
    assert_eq!(presentation.slides.len(), presentation_back.slides.len());
    // 4. Verify key text content
    let slide1_text = extract_slide_text(&presentation_back.slides[0]);
    assert!(slide1_text.contains("Title text"));
}
```

### Corpus-based verification

- Use existing 20+ pptx corpus fixtures in `corpus/blobs/`.
- For each fixture: `pptx → IR → create_minimal_pptx → pptx → text extraction → compare`.
- Focus on: text content fidelity, table structure, image presence, slide count.

### Corpus fixtures available

20+ pptx files in `corpus/blobs/`:
- `cjk-text-pptx-001.pptx`, `cjk-text-pptx-002.pptx` — CJK text content
- `comments-annotations-pptx-001/002.pptx` — comments + annotations
- `custom-styles-pptx-001/002.pptx` — custom styles
- `embedded-ole-pptx-001/002.pptx` — embedded OLE objects
- `image-textwrap-pptx-001/002.pptx` — images with text wrap
- `long-deck-300-pptx-001/002.pptx` — 300-slide presentations
- `merged-cells-pptx-001/002.pptx` — table merged cells
- `numbering-restarts-pptx-001/002.pptx` — numbered content
- `protected-readonly-pptx-001/002.pptx` — protected presentations
- `rtl-text-pptx-001/002.pptx` — right-to-left text

---

## 6. Size estimate

| Phase | Subtask | Size |
|---|---|---|
| 1a | Enrich `SlideElement` + add `Table`/`Placeholder` types to IR | **M** |
| 1b | Add `Visit` impls for enriched types | **S** |
| 1c | Update pptx reader to populate enriched fields | **M** |
| 2a | Rewrite `build_text_element` with actual content | **S** |
| 2b | Add image/media parts support | **M** |
| 2c | Rewrite `build_table_element` with cell content | **S** |
| 2d | Rewrite `build_shape_element` with geometry support | **S** |
| 2e | Use actual positions from IR | **S** |
| 2f | Enrich layout/master generation | **S** |
| 2g | Add `CreatePptx` builder to `create.rs` | **S** |
| 3a | Add `CreateTarget::Pptx` variant | **S** |
| 3b | Add `CreateSource::Presentation` variant | **S** |
| 3c | Update CLI format validation | **S** |
| 3d | Engine routing for pptx | **M** |
| 4a | Template-based pptx creation | **L** |
| 4b | C1-presentation-ir.md contract doc | **S** |
| 5a | Round-trip corpus tests | **M** |
| 5b | Integration test in engines | **S** |
| 5c | Corpus-based regression tests | **M** |
| **Total** | | **~10–15 days** (3 L + 5 M + 11 S) |

**Phased approach recommendation:**
- **P7-2a (MVP, 4–6 days):** IR enrichment (1a) + text/image/table writer rewrites (2a, 2b, 2c) + engine/CLI wiring (3a, 3b, 3c, 3d) + basic round-trip tests (5a). Produces a working `stichos create --format pptx` that can generate presentations with text, images, and tables.
- **P7-2b (polish, 3–5 days):** Shape geometry (2d), positions (2e), layout/master enrichment (2f), template creation (4a), corpus regression tests (5c).
- **P7-2c (docs, 1 day):** C1 contract doc (4b), Visit impls (1b).

---

## 7. Dependencies (D2 gate check)

| Dep | Crate | Needs new external dep? | Notes |
|---|---|---|---|
| `zip` | `adapters-ooxml` | **No** — already in use by `create_pptx.rs` | OPC packaging, media part support |
| `gp_core::ir` | `gp-core` | **No** — Presentation types exist (need enrichment) | IR types |
| `gp_core::diagnostics` | `gp-core` | **No** — already used | Diagnostics |
| `gp_core::traits` | `gp-core` | **No** — already used | Codec trait |
| `serde` | `gp-core` | **No** — already in use | Serialization |

**No new external dependencies required.** The `adapters-ooxml` crate already has `zip` as a dependency.

**D2 gate:** This task does NOT introduce a new external dependency. The only dependency changes are:
- `engines` crate → `adapters_ooxml::create_pptx` (new cross-crate dependency within workspace, already done for `create_xlsx`)
- `cli` → `engines::creation` (already connected for docx/odt/pdf)
- This is within the existing DAG: `engines` can depend on `adapters-ooxml` (already does for `revision_writer`)

---

## 8. Related work / context

- **docx create:** `create.rs` in `adapters-ooxml` — full-featured creation with sections, paragraphs, runs, tables, drawings, lists, headers/footers, numbering, themes. P3-16.
- **docx template create:** `create_template.rs` — reference docx as base, preserve styles/theme/master, replace content.
- **xlsx create:** `create_xlsx.rs` — minimal creation with sheets, cells, formulas, shared strings, merged ranges. P7-1.
- **xlsx read:** `xlsx.rs` — complete OPC → IR parser with formula evaluation (C15).
- **xlsx patch:** `xlsx_writer.rs` — surgical patch writer for cell edits.
- **pptx read:** `pptx.rs` — complete OPC → IR parser for slides, layouts, masters. 20+ corpus fixtures.
- **pptx patch:** `pptx_writer.rs` — surgical patch writer for shape/placeholder text.
- **pptx creation:** `create_pptx.rs` — structurally valid but content-placeholder-driven. **This task.**
- **OOXML fuzzing:** `adapters-ooxfuzz/src/creation_fuzz.rs` — fuzzes `create_minimal_pptx`.
- **Commit/executor:** `commit.rs`, `executor.rs` — route PatchOps to format-specific writers. Pptx already wired.
- **C1 Presentation IR:** `docs/contracts/C1.md` §3.3. No standalone contract doc.
- **Track B, remaining-work.md:** P7-1 (xlsx create) and P7-2 (pptx create) are consecutive Phase-7 backlog items.

---

## 9. Open questions

1. **Should `SlideElement` be an enum rather than a struct with `kind: String`?** This would be more type-safe and make the IR self-documenting. The `kind` string is currently ad-hoc ("text", "shape", "image", "table", "drawing", "graphic"). An enum would prevent invalid values at compile time.
   - **Recommendation:** Use an enum `SlideElementKind` with serde tag-based serialization. Map from reader `kind` strings during round-trip.

2. **Should table content in presentations share types with docx `SectionItem::Table`?** Currently the docx IR has `SectionItem::Table { rows: Vec<Row> }` where `Row { cells: Vec<Cell> }` and `Cell { contents: Vec<SectionItem> }`. Should the presentation `Table` reuse these types?
   - **Recommendation:** Create a shared `Table` type in `gp-core` that both `Document` and `Presentation` can reference. This reduces duplication and enables cross-format table operations.

3. **How should image data be represented in the IR?** Currently the writer would need `data: Vec<u8>` on `SlideElement`, which is large in serialized IR. Should images be stored externally in a content-addressed store and referenced by hash?
   - **Recommendation:** Keep `data: Option<Vec<u8>>` in `SlideElement` for simplicity (consistent with `Drawing.data` in docx). For large presentations, the IR can be materialized from a store at read time.

4. **Should the creation writer support `--template` for pptx like docx does?** Template-based pptx creation would be more complex because pptx themes/masters/layouts have more variance than docx styles.
   - **Recommendation:** Defer template creation to P7-2b. MVP focuses on content-driven creation from scratch.

5. **Should `SlideElement` support groups?** PowerPoint supports nested `<p:grpSp>` (group shapes). The reader already flattens groups into individual elements. Should the writer support emitting groups?
   - **Recommendation:** Defer group support. The MVP writer emits all elements at the top level of `<p:spTree>`. Groups can be added as an enhancement.

---

## 10. Acceptance criteria

- [ ] `CreateTarget::Pptx` variant exists and is documented.
- [ ] `stichos create --format pptx --source-type presentation --source-content '<presentation JSON>'` produces a valid pptx file (PK header, opens in PowerPoint/LibreOffice).
- [ ] Slide content is content-driven (not placeholder-driven): text, images, tables, shapes use actual IR data.
- [ ] Round-trip: pptx → IR → create_minimal_pptx → pptx → parse → IR preserves text content, slide count, table structure, image presence.
- [ ] Engine routing works: `engines::creation::create_document` with `pptx` target produces valid output.
- [ ] CLI accepts `pptx` as a valid `--format` value.
- [ ] Image elements emit `ppt/media/` parts with correct content-type overrides in `[Content_Types].xml`.
- [ ] Table elements emit actual cell content (not just `<p:rowRef>` placeholders).
- [ ] Text elements emit actual `<a:t>` content with formatting.
- [ ] No new external dependencies.
- [ ] `cargo test -p adapters-ooxml --lib` passes.
- [ ] `cargo test -p engines --lib` passes.
- [ ] Round-trip corpus tests pass on ≥5 pptx fixtures from `corpus/blobs/`.

---

## 11. Files to be modified

| File | Change |
|---|---|
| `crates/gp-core/src/ir.rs` | Enrich `SlideElement`, `Layout`, `Master`; add `Table`/`Placeholder`/`TableRow`/`TableCell` types; add `CreateTarget::Pptx` and `CreateSource::Presentation` |
| `crates/cli/src/main.rs` | Add `"pptx"` to format validation; add `"presentation"` to source_type validation |
| `crates/engines/src/creation.rs` | Add `"pptx"` to formats + routing; add `CreateSource::Presentation` handling |
| `crates/adapters-ooxml/src/create.rs` | Import `create_pptx` module; add `CreatePptx` builder dispatch |
| `crates/adapters-ooxml/src/create_pptx.rs` | Rewrite text/image/table/shape element builders with content-driven XML; add media part support |
| `crates/adapters-ooxml/src/pptx.rs` | Update reader to populate enriched IR fields |
| `docs/contracts/C1-presentation-ir.md` | **New file** — standalone Presentation IR contract |
