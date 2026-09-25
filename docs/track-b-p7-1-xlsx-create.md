# P7-1 — xlsx create: content-driven xlsx from IR

**Created:** 2026-09-24. **Task:** P7-1 from `docs/remaining-work.md` §Track-B.
**Status:** spec draft — awaiting implementation.

---

## 1. Current state assessment

### What exists

| Component | File | State |
|---|---|---|
| xlsx creation writer | `crates/adapters-ooxml/src/create_xlsx.rs` | **Exists.** `create_minimal_xlsx(&Spreadsheet) -> Result<Vec<u8>, Vec<Diagnostic>>` works: writes OPC package with `[Content_Types].xml`, `_rels/.rels`, `xl/workbook.xml`, `xl/_rels/workbook.xml.rels`, `xl/sharedStrings.xml`, `xl/styles.xml`, `xl/worksheets/sheetN.xml`, `docProps/core.xml`, `docProps/app.xml`. 11 unit tests pass. |
| xlsx creation dispatcher | `crates/adapters-ooxml/src/create.rs` | **Not present for xlsx.** This file only dispatches `create_minimal_docx`. No `create_xlsx` import or dispatch path. |
| xlsx reader (round-trip input) | `crates/adapters-ooxml/src/xlsx.rs` | **Complete.** `read_xlsx(&OpcPackage) -> ReadResult` parses OPC → Spreadsheet IR. Includes shared strings, styles, formulas with C15 evaluation, merged ranges, cell types (n/s/b/e/str). |
| xlsx patch writer | `crates/adapters-ooxml/src/xlsx_writer.rs` | **Exists.** `XlsxPatchWriter` applies PatchOps via span mutations to existing xlsx. Not creation — surgical edits only. |
| Spreadsheet IR types | `crates/gp-core/src/ir.rs` | **Complete.** `Spreadsheet`, `Sheet`, `SheetRow`, `SheetCell`, `MergedRange` with full serde + Visit impls. |
| CLI `create` verb | `crates/cli/src/main.rs` | **Rejects xlsx.** `cmd_create` validates `target_format` to only `docx | odt | pdf`. Error message: `"invalid format '{}': must be one of: docx, odt, pdf"`. |
| Engine creation API | `crates/engines/src/creation.rs` | **Rejects xlsx.** `VALID_TARGET_FORMATS = &["docx", "odt", "pdf"]`. `validate_args()` rejects `xlsx`. |
| CreateTarget enum | `crates/gp-core/src/ir.rs` | **No xlsx variant.** Only `Docx`, `Odt`, `Pdf`. |
| C1 Spreadsheet IR contract | `docs/contracts/` | **Not present as standalone doc.** Spreadsheet types live in `ir.rs`. No `C1-spreadsheet-ir.md` contract file exists (unlike C7, C11, C12, C13, C14, C15). |

### What does NOT exist

- `CreateTarget::Xlsx` variant
- `xlsx` in `VALID_TARGET_FORMATS` or CLI format validation
- Engine dispatch path for `xlsx` in `create_document_engine`
- `adapters_ooxml::create_xlsx` import in `create.rs`
- `engines::creation` routing to xlsx writer
- C1 Spreadsheet IR contract doc in `docs/contracts/`

---

## 2. C1 Spreadsheet IR — fields and types

All types are in `crates/gp-core/src/ir.rs` (lines 798-850).

```rust
pub struct Spreadsheet {
    pub ir_version: String,                      // always "1.0.0"
    pub ir_version_migration: Option<String>,
    pub sheets: Vec<Sheet>,                      // one or more named sheets
}

pub struct Sheet {
    pub id: ElementId,                           // content-anchored
    pub name: String,                            // e.g. "Data"
    pub rows: Vec<SheetRow>,                     // sparse: only rows with cells
    pub merged_ranges: Vec<MergedRange>,         // optional
    pub header_row: Option<usize>,               // optional hint
    pub footer_row: Option<usize>,               // optional hint
}

pub struct SheetRow {
    pub id: ElementId,
    pub cells: Vec<SheetCell>,                   // sparse: only cells with values
}

pub struct SheetCell {
    pub id: ElementId,
    pub row: usize,                              // 0-based
    pub col: usize,                              // 0-based
    pub value: Option<String>,                   // raw string (resolved for shared strings)
    pub formula: Option<String>,                 // e.g. "=SUM(A1:B2)"
    pub cached_value: Option<String>,            // computed value for formula cells
    pub style_name: Option<String>,              // name reference into styles.xml
}

pub struct MergedRange {
    pub id: ElementId,
    pub start_row: usize, pub start_col: usize,
    pub end_row: usize,   pub end_col: usize,
}
```

**What's needed for creation from IR:**
- **Already sufficient:** `SheetCell.value` covers strings, numbers (as string), booleans, errors. The writer's `build_cell_xml` does type detection via string heuristics (`is_numeric`, boolean check, error prefix).
- **Already sufficient:** `SheetCell.formula` + `cached_value` → `<f>` + `<v>` element.
- **Already sufficient:** `MergedRange` → `<mergeCells>`.
- **Already sufficient:** `SheetCell.style_name` → style index mapping (via `collect_style_indices`).
- **Already sufficient:** `Sheet.name` + `Spreadsheet.sheets` → `<sheets>` + per-sheet files.
- **Already sufficient:** shared string deduplication via `collect_shared_strings`.

**Gap — no style customization in IR:** The writer emits a static default `styles.xml` (Calibri 11pt, no custom fonts/fills/borders beyond defaults). Named style references in `style_name` are collected but never written to `styles.xml`. This means `style_name` on `SheetCell` is effectively a no-op in the creation path.

**Gap — no number formats in IR:** Excel custom number formats (currency, percentage, date) are not represented in the IR. `SheetCell.style_name` is a string, not a number-format-id mapping.

---

## 3. Gap analysis

### Gap A: Engine/CLI integration (highest priority)

| Gap | File(s) | Description |
|---|---|---|
| A1 | `crates/gp-core/src/ir.rs` | Add `Xlsx` to `CreateTarget` enum + `pandoc_format()` arm returning `"xlsx"`. |
| A2 | `crates/engines/src/creation.rs` | Add `"xlsx"` to `VALID_TARGET_FORMATS`; add routing in `create_document_engine` for `CreateTarget::Xlsx`. |
| A3 | `crates/cli/src/main.rs` | Add `"xlsx"` to CLI format validation in `cmd_create`. |
| A4 | `crates/adapters-ooxml/src/create.rs` | Import `create_xlsx` module and dispatch `create_minimal_xlsx` from a `CreateXlsx` builder. |

### Gap B: IR completeness (medium priority)

| Gap | File(s) | Description |
|---|---|---|
| B1 | `crates/gp-core/src/ir.rs` | Consider adding `SheetCell.number_format: Option<String>` for Excel number-format IDs. Currently style_name is a string name, not a format id. |
| B2 | `crates/adapters-ooxml/src/create_xlsx.rs` | Wire `style_name` → `styles.xml` cellXfs entries with proper numFmtId/fontId/fillId/borderId mapping. Currently the style index is collected but the styles.xml is static. |
| B3 | `crates/adapters-ooxml/src/create_xlsx.rs` | Add `Sheet.print_settings: Option<PrintSettings>` for page setup, margins, orientation. |
| B4 | `crates/adapters-ooxml/src/create_xlsx.rs` | Add `SheetColDef` type for column widths and row heights (sparse per-sheet col/row definitions). |
| B5 | `crates/adapters-ooxml/src/create_xlsx.rs` | Add `Spreadsheet.protect: Option<SheetProtection>` for worksheet protection. |

### Gap C: C1 contract doc (low priority, documentation)

| Gap | File | Description |
|---|---|---|
| C1 | `docs/contracts/C1-spreadsheet-ir.md` | Create standalone contract doc (pattern from C7, C11, C12, etc.) documenting the Spreadsheet IR schema, constraints, and versioning rules. |

### Gap D: Integration wiring (medium priority)

| Gap | File(s) | Description |
|---|---|---|
| D1 | `crates/engines/src/creation.rs` | Create `create_via_ooxml_writer()` engine function that takes `CreateSource::Spreadsheet { ir_bytes }` → `CreateTarget::Xlsx` → calls `adapters_ooxml::create_xlsx::create_minimal_xlsx`. |
| D2 | `crates/engines/src/creation.rs` | Handle `CreateSource::Spreadsheet { ir_bytes }` as a new CreateSource variant (or reuse `CreateSource::Markdown` with a different interpretation). |

---

## 4. Implementation plan

### Phase 1: Core wiring (prerequisites, S–M)

1. **1a.** Add `Xlsx` variant to `CreateTarget` in `gp-core/src/ir.rs`.
   - Add `pandoc_format()` arm → `"xlsx"` (or `"excel"` — pandoc uses `"xlsx"`).
   - Size: **S**

2. **1b.** Update CLI `cmd_create` in `crates/cli/src/main.rs` to accept `"xlsx"`.
   - Change validation match arm: `"docx" | "odt" | "pdf" | "xlsx" => {}`.
   - Size: **S**

3. **1c.** Update `engines/src/creation.rs`:
   - Add `"xlsx"` to `VALID_TARGET_FORMATS`.
   - Add `("spreadsheet", "xlsx")` to `VALID_COMBINATIONS`.
   - Add routing in `create_document_engine` for `CreateTarget::Xlsx`.
   - Size: **S**

### Phase 2: Creation writer enhancement (M–L)

4. **2a.** Wire `adapters-ooxml/src/create.rs` to dispatch to xlsx.
   - Add `mod create_xlsx;` to `lib.rs` (already present).
   - In `create.rs`, add `CreateXlsx` builder struct parallel to `CreateDocx`.
   - Add `create_minimal_xlsx()` public wrapper.
   - Size: **S**

5. **2b.** Enhance `create_xlsx.rs` styles writing.
   - Build `styles.xml` dynamically from collected `style_name` → index map.
   - Map known style names to font/border/fill/numFmt definitions.
   - Size: **M**

6. **2c.** Add column width / row height support.
   - Add optional `col_widths: Vec<f64>` and `row_heights: Vec<f64>` to `Sheet`.
   - Emit `<cols>` and `<row @ht="...">` in worksheet XML.
   - Size: **M**

7. **2d.** Add number format support.
   - Add `number_formats: BTreeMap<String, String>` to `Sheet` mapping style names to x:numFmt.
   - Emit `<numFmts>` section in `styles.xml`.
   - Size: **M**

### Phase 3: Round-trip verification (M)

8. **3a.** Create round-trip corpus fixtures.
   - Reuse `test_fixtures` in `xlsx.rs` to generate xlsx → IR → xlsx → compare.
   - Add test: create xlsx from IR → parse back → verify IR equality.
   - Size: **M**

9. **3b.** Add integration test in `engines/src/creation.rs`.
   - Test `create_document` with `xlsx` target, verify PK header.
   - Size: **S**

### Phase 4: C1 contract doc (S)

10. **4a.** Create `docs/contracts/C1-spreadsheet-ir.md`.
    - Document all Spreadsheet IR types, constraints, and versioning.
    - Follow pattern from C7, C11, C12.
    - Size: **S**

---

## 5. Verification strategy

### Unit tests (in `create_xlsx.rs`)

| Test | What it verifies | Status |
|---|---|---|
| `test_minimal_xlsx_creation` | Empty Spreadsheet → valid OPC | Exists, passes |
| `test_xlsx_with_data` | Strings in sharedStrings, numerics inline | Exists, passes |
| `test_xlsx_shared_strings` | Dedup across sheets | Exists, passes |
| `test_xlsx_with_formulas` | `<f>` + `<v>` elements | Exists, passes |
| `test_xlsx_merged_ranges` | `<mergeCells>` | Exists, passes |
| `test_xlsx_roundtrip` | All OPC parts present with correct structure | Exists, passes |
| `test_xlsx_with_styles` | Style name → cell type references | Exists, passes (but styles.xml not dynamic) |
| `test_cell_ref`, `test_col_to_letter`, `test_is_numeric`, `test_escape_xml`, `test_merged_range_ref` | Helper functions | Exist, pass |

### New tests to add

| Test | What it verifies |
|---|---|
| `test_xlsx_roundtrip_identity` | IR → xlsx → parse → IR equality (byte-identical or semantically equal) |
| `test_xlsx_with_styles_dynamic` | Named styles produce actual style entries in styles.xml |
| `test_xlsx_with_number_formats` | Currency/percentage formats in styles.xml numFmts |
| `test_xlsx_with_column_widths` | `<cols>` section with width attributes |
| `test_xlsx_invalid_input` | Empty sheets vec still produces valid OPC |

### Round-trip test pattern (from `xlsx.rs` test_fixtures)

```rust
#[test]
fn test_xlsx_create_roundtrip() {
    let spreadsheet = make_spreadsheet_with_data();
    // 1. Create xlsx from IR
    let bytes = create_minimal_xlsx(&spreadsheet).expect("create");
    // 2. Parse xlsx back to IR
    let pkg = parse_opc(&bytes).expect("opc parse");
    let read_result = read_xlsx(&pkg).expect("read");
    let spreadsheet_back: Spreadsheet = serde_json::from_slice(&read_result.ir).expect("deserialize");
    // 3. Verify key fields match
    assert_eq!(spreadsheet.sheets.len(), spreadsheet_back.sheets.len());
    // ... cell-level assertions
}
```

---

## 6. Size estimate

| Phase | Subtask | Size |
|---|---|---|
| 1a | `CreateTarget::Xlsx` | S |
| 1b | CLI format validation | S |
| 1c | Engine routing | S |
| 2a | `create.rs` dispatch | S |
| 2b | Dynamic styles.xml | M |
| 2c | Column/row dimensions | M |
| 2d | Number format support | M |
| 3a | Round-trip corpus | M |
| 3b | Integration test | S |
| 4a | C1 contract doc | S |
| **Total** | | **~3–5 days** (2 M + 6 S) |

---

## 7. Dependencies (D2 gate check)

| Dep | Crate | Needs new external dep? | Notes |
|---|---|---|---|
| `zip` | `adapters-ooxml` | **No** — already in use by `create_xlsx.rs` | OPC packaging |
| `gp_core::ir` | `gp-core` | **No** — Spreadsheet types exist | IR types |
| `gp_core::diagnostics` | `gp-core` | **No** — already used | Diagnostics |
| `gp_core::traits` | `gp-core` | **No** — already used | Codec trait |
| `serde` | `gp-core` | **No** — already in use | Serialization |

**No new external dependencies required.** The `adapters-ooxml` crate already has `zip` as a dependency.

**D2 gate:** This task does NOT introduce a new dependency. The only dependency change is:
- `engines` crate → `adapters_ooxml::create_xlsx` (new cross-crate dependency within workspace)
- This is within the existing DAG: `engines` can depend on `adapters-ooxml` (already does for `revision_writer`)

---

## 8. Related work / context

- **docx create:** `create.rs` in `adapters-ooxml` — full-featured creation with sections, paragraphs, runs, tables, drawings, lists, headers/footers, numbering, themes. P3-16.
- **pptx create:** `create_pptx.rs` — minimal creation with slides/layouts/masters/themes. P7-2.
- **xlsx read:** `xlsx.rs` — complete OPC → IR parser with formula evaluation (C15), shared strings, styles, merged ranges.
- **xlsx patch:** `xlsx_writer.rs` — surgical patch writer for cell edits, row inserts, deletions.
- **C15 formula evaluation:** `formula/` module in `gp-core` and `adapters-ooxml` — 12-function subset with refusal checks and parity.
- **C7 v1.6:** All tool schemas at version 1.6.0. New CreateTarget variant needs C7 matrix update.
- **Track B, remaining-work.md:** P7-1 is the first item in the Phase-7 backlog, gated on C1.

---

## 9. Open questions

1. **Should `CreateSource` gain a `Spreadsheet { ir_bytes: Vec<u8> }` variant?** This would let the `create` verb accept pre-built IR JSON for xlsx output. Alternatively, the `--source-content` could contain IR JSON when `--format=xlsx`.

2. **Style system design:** Should `style_name` on `SheetCell` map to named styles (like docx "Heading 1") or to raw Excel style properties (bold, italic, font, fill)? The current IR only has a string name — a richer `CellStyle` type would be needed for full fidelity.

3. **Number format IDs:** Excel uses integer format IDs (0=default, 1=general, 2=decimal, etc.) referenced via `numFmtId` on cellXfs. Should the IR store format IDs directly, or style names that resolve to format IDs?

4. **Spreadsheet vs. table:** The IR `Sheet` is spreadsheet-specific. Should the `SectionItem::Table` (used in docx/pptx) be unified with `Sheet` for cross-format table creation?

---

## 10. Acceptance criteria

- [ ] `create --format xlsx --source-type spreadsheet --source-content '<spreadsheet JSON>'` produces a valid xlsx file (PK header, opens in Excel/LibreOffice).
- [ ] Round-trip: xlsx → IR → xlsx preserves all cells, formulas, shared strings, merged ranges.
- [ ] `CreateTarget::Xlsx` is added and documented in C1 contract.
- [ ] `VALID_TARGET_FORMATS` includes `"xlsx"` and engine routing works end-to-end.
- [ ] Unit tests cover: empty spreadsheet, single-sheet, multi-sheet, formulas, merged ranges, shared strings, number formats, custom styles.
- [ ] No new external dependencies.
- [ ] `cargo test -p adapters-ooxml --lib` passes.
- [ ] `cargo test -p engines --lib` passes.

---

## 11. Files to be modified

| File | Change |
|---|---|
| `crates/gp-core/src/ir.rs` | Add `Xlsx` to `CreateTarget` enum |
| `crates/cli/src/main.rs` | Add `"xlsx"` to format validation |
| `crates/engines/src/creation.rs` | Add `"xlsx"` to formats + routing |
| `crates/adapters-ooxml/src/lib.rs` | Ensure `create_xlsx` is public (already is) |
| `crates/adapters-ooxml/src/create.rs` | Add xlsx dispatch path |
| `crates/adapters-ooxml/src/create_xlsx.rs` | Enhance styles/number formats (if in scope) |
| `docs/contracts/C1-spreadsheet-ir.md` | **New file** — C1 contract doc |
