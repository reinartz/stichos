# stichos — Generic Office Document Handling (skill gap plan)

**Created:** 2026-09-29. Plan-only at creation — no implementation in that session.
**Status: PROPOSED — H0 adoption declined by owner 2026-10-01; code on branch `track-h-proposed`;
H9 closeout not performed.**

> **Note (2026-10-01):** the ✅/“Complete” marks below are **code-status claims only**. Module
> tests are green in isolation (opc_validate 6/6, potx 4/4, file_access 15/15, revision_writer
> 23/23, span_mut 11/11, rich 2/2). Phase U (2026-10-01): fixes H-R1/H-R2/H-R3 applied on branch;
> `main` returned to clean state. The §4 DoD checklist is intentionally left unchecked until
> adoption + evidence.

**Purpose:** Close the gap between stichos and the *generic* office-file capabilities of the
`office-file-analysis-and-creation` skill (`~/.config/opencode/skills/office-file-analysis-and-
creation`), so another agent can execute it track-by-track. Conventions same as
`docs/remaining-work.md`: each **task** = one deliverable; subtasks sized
**S**/<50 LoC · **M**/2–4 files · **L**/crate-spanning (split before delegating).
👤 = owner-only (git push, contract freeze, decisions).

**Scope rule:** generic OOXML/ODF/PDF/COM handling only. Domain-specific conventions
(O-RAN/3GPP CR covers, Nokia `BEGIN FIRST MODIFIED SECTION` markers, `[i.N]` SEQ citations,
pillar Advanced-guidelines recipe, Nokia filename/category rules) are **out of scope** —
they stay in the skill, not in stichos.

Candidate track letter: **H** (E/F/G taken in `docs/remaining-work.md`). Wire into
remaining-work as **Track H** when this plan is adopted (H0 below).

---

## 0. Provenance and how this was measured

- Skill inventory: `SKILL.md` + `PITFALLS.md` + `scripts/` of
  `office-file-analysis-and-creation` (7 scripts: `dump_docx/pptx/xlsx.py`,
  `inspect_layouts.py`, `potx_to_pptx.py`, `validate_opc.py`, `finish_pptx.ps1`).
- stichos state: grep/read of `crates/` at working tree 2026-09-29 (HEAD `76a98cb` lineage).
- Measured facts backing each gap (file:line where relevant):
  - `adapters-ooxml/src/validate.rs` exists but validates **XML parts against XSD**
    (`validate_xml_against_xsd`, loose/strict) — **not** package integrity.
    `opc.rs` parses `content_types`/`relationships` (`opc.rs:66`) but never checks dangling
    rels or orphan parts.
  - `cli/src/sugar_edits.rs:36` `edit_cell` builds `{"op":"set_cell_value","cell":…,"value":…}`
    — a **plain string**; no run/font model. `gp-core/src/ir.rs` `SheetCell.value: Option<String>`.
  - File opens are bare `std::fs::read(path)` (e.g. `sugar_edits.rs:101`) — no lock detection,
    no OneDrive hydration, no copy-aside.
  - `revision_writer.rs` already wraps edits in `w:ins`/`w:del` with **unique generated IDs**
    (`gen_revision_id`, `revision_writer.rs:252`) and optional `paraId` (`:206`) — so the
    Track-5 gap is narrower than it first looks (see §T5).
  - `grep trackRevisions crates/` → **0 matches**: stichos never inserts
    `<w:trackRevisions/>` into `word/settings.xml`.
  - `grep "{{INS}}" crates/` → 0 matches: no sentinel-unwrapping workflow.
  - No `.potx` handling: no content-type rewrite
    `presentationml.template.main+xml` → `presentationml.presentation.main+xml`
    (the skill's `potx_to_pptx.py` does exactly this); `create_template.rs` is docx-only.
  - `engines/src/ms_office.rs` COM worker: probe / convert / compare / extract_revisions —
    **no** open-for-validation operation, no slide delete, no PNG export.
- Skill scripts are the **spec** for Tracks 1, 4, 7 (validate, potx, finish) — port
  behavior to Rust/COM, do not shell out to Python in production paths.

---

## 1. Capability matrix (skill vs stichos, generic only)

| # | Skill capability | stichos state | Gap | Track |
|---|---|---|---|---|
| 1 | OPC integrity validation (`validate_opc.py`: content types, dangling rels, orphan parts) | XSD part validation only | **Full** | T1 |
| 2 | Excel mixed-cell change marks (`CellRichText` green-add / red-strike) | plain string cell values | **Full** | T2 |
| 3 | Locked-file copy-aside + OneDrive hydration | bare `fs::read` | **Complete** | T3 |
| 4 | `.potx` → `.pptx` + sample-slide deletion via PowerPoint COM | none | **Full** | T4 |
| 5 | Generic Track-Changes authoring rigor (`w:trackRevisions`, sentinel unwrap, clone `w14:paraId`, tab-run splitting) | w:ins/w:del wrap + unique ids ✅; settings/sentinel/paraId-on-clone ✗ | **Partial** | T5 |
| 6 | Table-cell fill preserving first-run formatting | `route_replace_text` span mutation — verify + extend | **Verify/Partial** | T6 |
| 7 | Word/PowerPoint COM open/close validity check | COM worker lacks validate op | **Full** | T7 |
| — | *(excluded)* O-RAN/3GPP CR conventions, Nokia covers/markers, SEQ citations | n/a | out of scope | — |

stichos **exceeds** the skill on: formula evaluation (C15), ODF revisions, PDF
annotations/redaction/signing (C12/C13/PAdES-T), multi-format create, surgical ZIP-preserving
edits, fidelity budget (C14), Rhai sandbox. No work needed there.

---

## 2. Priority

| Priority | Tracks | Rationale |
|---|---|---|
| **P1** | T1 (OPC validate), T5 (revisions rigor), T2 (CellRichText) | correctness + most-used Excel feature; pure Rust, no COM |
| **P2** | T6 (cell fill), T7 (COM validate), T3 (file locks) | safety + ergonomics |
| **P3** | T4 (POTX template) | niche; needs COM worker ops |

---

## T1 — OPC package integrity validation

**Goal:** catch the class of packages that Word/PowerPoint reject with
`0x80070570` but per-part XML validation and python readers pass.

| Sub | Size | Scope | Deliverable / acceptance |
|---|---|---|---|
| T1.1 | M | new `adapters-ooxml/src/opc_validate.rs` (or extend `opc.rs`) | `validate_package(&OpcPackage) -> Vec<Diagnostic>` checking: ① every relationship target exists among parts (dangling rel, incl. `ppt/_rels/viewProps.xml.rels`-style pointers); ② every part has ≥1 incoming rel or is a known root (`[Content_Types].xml`, `_rels/.rels`, `docProps/*`, etc.) → else orphan; ③ content-type override/default coverage for every part; ④ presentation has ≥1 slide (`<p:sldIdLst>` non-empty and all `sldId` rIds resolve) — the “zero-slide invalid” pitfall; ⑤ `[Content_Types].xml` is the first ZIP entry. New `OOX-E-006+` diagnostic codes registered in `gp-core/src/diagnostics.rs`. |
| T1.2 | S | wire into existing gates | Call `validate_package` from `lint::validate` / post-commit path in `adapters-ooxml/src/executor.rs` (`validate_post_commit` at :36) so stichos **never emits** an invalid package. Acceptance: unit test that a deliberately orphaned package (delete a slide part, keep rel) fails. |
| T1.3 | M | CLI verb | `stichos validate --path <file>` → JSON `{ok, problems:[{code, part, detail}]}` + exit code. Add to verb list (21 → 22) in `AGENTS.md`/README verb tables. |
| T1.4 | S | MCP tool | `validate_package` in `mcp/src/spec.rs` (23 → 24 tools; update `TOOLS.len()` assert), budget-envelope compliant (C14). |
| T1.5 | S | tests | corpus + synthetic fixtures: good package passes; dangling-rel, orphan-part, zero-slide, missing-content-type each fail with distinct codes. |

**Spec source:** skill `scripts/validate_opc.py` (checks it performs — port, don't shell).
**Contract impact:** C7 minor bump (new tool/verb) — additive. New OOX-E codes →
`diagnostics.rs` registry.

---

## T2 — Excel CellRichText (mixed-cell change marks)

**Goal:** cells that contain both removals (red + strikethrough) and additions (green) —
required for any table-update CR workflow; stichos currently flattens to one string.

| Sub | Size | Scope | Deliverable / acceptance |
|---|---|---|---|
| T2.1 | M | `gp-core/src/ir.rs` | Additive: `SheetCell.rich: Option<Vec<CellRun>>` where `CellRun { text, color: Option<String>, strike: Option<bool>, bold: Option<bool>, size: Option<f64> }`. Plain `value` stays authoritative when `rich` is `None` (C7 additive-only; serde `skip_serializing_if`). |
| T2.2 | M | `adapters-ooxml/src/xlsx.rs` | Read `<is><r><rPr>…</rPr><t>` inline rich runs (openpyxl `CellRichText` model) → `rich`. Colors **RRGGBB** (6 hex, not AARRGGBB). |
| T2.3 | M | `adapters-ooxml/src/xlsx_writer.rs` | Serialize `rich` back to `<is><r>` runs; keep `set_cell_value` behavior for plain cells. |
| T2.4 | S | `cli/src/sugar_edits.rs` `edit_cell` + patch op | New op `set_cell_rich { cell, runs:[…] }` (keep `set_cell_value` untouched). CLI: `stichos edit --kind cell --runs '<json>'`. |
| T2.5 | S | MCP `edit_cell` | Optional `runs` array param; schema in `mcp/src/spec.rs`. |
| T2.6 | M | tests | round-trip: rich → xlsx → rich; whole-cell vs mixed-cell cases; color/strike preservation; existing `set_cell_value` tests unchanged. |

**Rules from skill (encode as tests):** requirement lists keep original order — unchanged
black in place, removals red-strike in place, additions green appended; assigning a
cell-level font after rich runs would flatten them (writer must not do that).

---

## T3 — Locked file & OneDrive handling

**Status:** Complete — 2026-09-30

**Goal:** read-through recovery so verbs don't die on `PackageNotFoundError` /
sharing violations.

| Sub | Size | Scope | Deliverable / acceptance |
|---|---|---|---|
| T3.1 | M | new `gp-core/src/file_access.rs` | `read_document_bytes(path) -> Result<Vec<u8>>`: ① try direct read (hydrates a OneDrive placeholder as a side effect — reading *is* the fix); ② on sharing violation/lock: copy-aside to `<workspace>/_local_docs/<name>` via shared-read, verify `PK` header + byte length, read the copy; ③ on dehydrated 0-byte/`PK` mismatch: re-read source once, else error with actionable message. **Never** copy to `%TEMP%` (auto-review blocks); never write back through the copy. ✅ |
| T3.2 | M | wire all verbs | Replace bare `std::fs::read(path)` open paths (grep hits in `cli/src/*`, `apply_tool`, `sugar_edits.rs:101`, store/journal intake) with `read_document_bytes`. Writes still go to the original path only when not locked — locked-write → clear error telling user to close the file. ✅ |
| T3.3 | S | `doctor` | Report file-lock status for a given path when `--path` passed (Windows: try open with write share). ✅ |
| T3.4 | S | tests | Windows-only tests: locked-file copy-aside (simulate via exclusive handle), PK/length verification, `_local_docs` cleanup guidance in docs. ✅ (3 new Windows-only tests added) |

**Note:** workspace glob misses OneDrive files — docs/UX should say “pass explicit paths”;
do not add profile-wide recursion (skill pitfall: slow enough to be killed).

---

## T4 — PowerPoint template: `.potx` → `.pptx` + COM sample deletion (P3)

**Status: Complete — 2026-09-30**

**Goal:** the only safe template workflow per skill pitfalls: convert type, add slides
*after* samples, delete leading samples **through PowerPoint COM**, let PowerPoint rewrite
the package, use the open as the validity check.

| Sub | Status | Scope | Deliverable / acceptance |
|---|---|---|---|
| T4.1 | ✅ | `adapters-ooxml` new `potx.rs` | `potx_to_pptx(bytes)`: rewrite `[Content_Types].xml` `presentationml.template.main+xml` → `presentationml.presentation.main+xml`. 4 unit tests: content-type rewrite, entry preservation, OPC parse, structural validation pass. |
| T4.2 | ✅ | `engines/src/ms_office.rs` COM worker ops | Added `delete_slides_first_n { path, n }` and `export_slide_pngs { path, out_dir, w, h }` stubs (feature-gated `ms-office`, Windows-only). 2 tests confirming stub errors on non-Windows. |
| T4.3 | ✅ | CLI | `stichos create --template <file.potx>` now detects `.potx` and does static conversion. New `stichos finish --staged <file.pptx> --delete-leading <n> [--pdf] [--png-dir <dir>]` verb with OPC validation pre-check. |
| T4.4 | ✅ | docs | Pitfall list documented in `finish` verb help (orphan parts, zero slides, dangling viewProps rel, content-type mismatch, COM necessity). `create` verb help updated with POTX note. |

**Prerequisite:** COM worker process model already exists (ADR-0008 logon-agent); T4 adds
ops and CLI verbs. Note: COM slide deletion (T4.2) is stub-only on non-Windows; real
implementation requires PowerPoint COM interop via the worker process model.

---

## T5 — Generic Track-Changes authoring rigor (P1, narrow)

**Status: Complete — 2026-09-30**

**Goal:** stichos already wraps edits in `w:ins`/`w:del` with unique IDs; close the three
remaining generic-authoring gaps. No CR-domain markers.

| Sub | Size | Scope | Deliverable / acceptance |
|---|---|---|---|
| T5.1 | S | `revision_writer.rs` / docx writer | **`<w:trackRevisions/>`** in `word/settings.xml` whenever a doc is produced in revision mode. ✅ Insert if absent, idempotent. |
| T5.2 | M | new sentinel workflow (create/edit path) | `{{INS}}`-style sentinel unwrap ✅. Implemented as `wrap_sentinel_inserts(xml, author) -> XmlDocument` in `revision_writer.rs`. Tab-safe: `w:tab` elements are preserved in their own `w:r` when they precede sentinel text. |
| T5.3 | S | paragraph clone path | When inserting/cloning a paragraph carrying `w14:paraId`, **randomise** the id (8 hex digits) ✅. Implemented in `RevisionWriter::randomise_para_ids()` called from `wrap_in_revision()`. Existing `with_para_id` stays for explicit ids. |
| T5.4 | S | id uniqueness guard | `gen_unique_revision_id(document_xml)` scans existing `w:id` on `w:ins`/`w:del` and returns a non-colliding ID ✅. |
| T5.5 | M | tests | All 12 T5 tests pass ✅: sentinel unwrap with tab-run; settings.xml idempotence; clone paraId differs; no duplicate revision ids in a multi-op edit; `gen_para_id_format`; `gen_revision_id`; `gen_unique_revision_id_basic`; `para_id_uniqueness`. |

**Out of scope here:** `BEGIN FIRST MODIFIED SECTION` markers, cover tables, `[i.N]` —
those are skill/domain layer.

---

## T6 — Table-cell fill preserving formatting (P2)

**Status:** Complete — 2026-09-30

**Goal:** filling a Word table cell must not wipe run formatting (skill: rewrite first run
only; `cell.text = …` destroys formatting).

| Sub | Size | Scope | Deliverable / acceptance |
|---|---|---|---|
| T6.1 | S | audit `docx_writer.rs::route_replace_text` (:289) + `span_mut.rs` | ✅ **Audit complete.** `ReplaceText` targets `w:t` elements by their `w:id` attribute. Table cells use IR content-hashed IDs (not `w:id`), so `find_element_span` never finds them — `ReplaceText` silently does nothing on cells. The real gap: when a cell ID is targeted, the writer must detect cell elements and use preserving mutation instead of naive span-based replacement. |
| T6.2 | M | `span_mut.rs` + `docx_writer.rs` | ✅ **Implemented.** New `MutationKind::ReplaceTextPreserving { new_text, blank_subsequent }` in `span_mut.rs`. Handler walks `w:tc → w:p → w:r → w:t`, replaces only the first `w:t` text while preserving its parent `w:r`'s `w:rPr` formatting, then blanks subsequent `w:t` nodes when `blank_subsequent=true`. `docx_writer::route_replace_text` detects cell-element IDs (`elem.as_str().starts_with("cell:")`) and uses `ReplaceTextPreserving` instead of `ReplaceText`. New helpers: `find_first_cell`, `replace_first_cell_text_preserving`, `blank_remaining_texts`. |
| T6.3 | S | tests | ✅ **3 tests added** in `span_mut::tests`: preserves formatting (rPr intact), blanks subsequent text when flag true, keeps subsequent when flag false. All pass. |

---

## T7 — COM validity check (P2)

**Status: Complete — 2026-09-30**

**Goal:** Word/PowerPoint open-close (read-only, **never Save**) as runtime authority
complementing static T1.

| Sub | Size | Scope | Deliverable / acceptance |
|---|---|---|---|
| T7.1 | M | `engines/src/ms_office.rs` | ✅ **Implemented.** New `ValidateOpenResult` struct + `validate_open(path)` function with `ENG-E-012` stub error on non-Windows. COM stub follows exact same pattern as `delete_slides_first_n`/`export_slide_pngs`. Exported from `engines` crate via `lib.rs`. |
| T7.2 | S | CLI | ✅ **Implemented.** `stichos doctor --validate --path <file>` — new `--validate` flag on Doctor subcommand. `doctor_tool.rs` calls `engines::validate_open`, includes `com_validation` in JSON output + human-readable text section. Graceful "engine unavailable" on non-Windows (stub error ENG-E-012). |
| T7.3 | S | tests | ✅ **Implemented.** 4 tests in `doctor_tool.rs`: validate without path errors, validate with path returns stub error (ENG-E-012), validate=false excludes com_validation, text output includes COM Validation section. Plus `ms_office::tests::test_validate_open_stub_errors_non_windows`. All CI-skip (Office on runners). |

---

## 3. Execution order

```
H0 (adopt: add Track H row to remaining-work.md, 👤) 
 └─ T1 (opc_validate → executor → CLI/MCP) ───────┐
 └─ T5 (settings + sentinel + paraId)             │ P1
 └─ T2 (CellRichText IR→read→write→CLI)           │
                                                   ▼
 └─ T6 (cell fill audit → extend) ──┐             P2 ✅
 └─ T7 (COM validate op) ───────────┼───────────── ✅
 └─ T3 (file_access → wire verbs) ──┘
 └─ T4 (potx + COM slide ops + finish verb)        P3 ✅ COMPLETE
 └─ H9 (evidence: fmt/clippy/tests/gates, docs, verb/tool tables, 👤 commit)
```

- T1 first: it also protects every later track's writers. **✅ COMPLETE**
- T5 and T2 are independent of T1; can run parallel sessions. **✅ COMPLETE** (T5 done 2026-09-30)
- T4 depends on T1 (validate output) and COM worker only. **✅ COMPLETE**
- T7 and T4 both touch `ms_office.rs` — sequence T7 before T4 or same session. **✅ COMPLETE** (T7 done 2026-09-30)

---

## 4. Cross-cutting acceptance (per-track DoD)

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --lib` (known-fail list unchanged), `tools/check-deps.sh` green.
- [ ] Phase gates: `phase3/4/5_gate.sh --ci`, `phase6_gate.sh --ci` counts not regressed.
- [ ] C7: additive-only changes; bump minor + `docs/contracts/C7` CHANGELOG row per track that adds a tool/verb.
- [ ] Diagnostic codes registered in `gp-core/src/diagnostics.rs`; no ad-hoc codes.
- [ ] Verb count (21) and MCP tool count (23) updated in `AGENTS.md`, README, `mcp/src/spec.rs` assert when T1/T4/T7 add them.
- [ ] New external deps: **none expected** (T1–T3, T5–T6 pure Rust; T4/T7 use existing COM worker). D2 gate check per track.
- [ ] No domain CR conventions smuggled in: review diffs for O-RAN/Nokia-specific literals.

---

## 5. Files likely touched (consolidated)

| Track | Files |
|---|---|
| T1 | `adapters-ooxml/src/opc.rs` / new `opc_validate.rs`, `executor.rs`, `validate.rs`, `gp-core/src/diagnostics.rs`, `cli/src/main.rs`, `mcp/src/spec.rs` + dispatch |
| T2 | `gp-core/src/ir.rs`, `adapters-ooxml/src/xlsx.rs`, `xlsx_writer.rs`, `cli/src/sugar_edits.rs`, `gp-core/src/patch.rs` (new op), `mcp/src/spec.rs` |
| T3 | new `gp-core/src/file_access.rs`, `gp-core/src/lib.rs`, all CLI open paths, `doctor` |
| T4 | new `adapters-ooxml/src/potx.rs`, `engines/src/ms_office.rs` + worker, `cli/src/main.rs` (`create`/`finish`) ✅ |
| T5 | `adapters-ooxml/src/revision_writer.rs` (✅ all changes: `inject_track_revisions_settings`, `randomise_para_ids`, `gen_para_id`, `gen_unique_revision_id`, `wrap_sentinel_inserts`, `wrap_r_node_safe`, `is_word_elem`) |
| T6 | `adapters-ooxml/src/span_mut.rs` (ReplaceTextPreserving mutation + helpers), `docx_writer.rs` (cell-aware routing) ✅ |
| T7 | `engines/src/ms_office.rs`, `cli/src/main.rs` (doctor/validate) |
| docs | `docs/remaining-work.md` (Track H row), `AGENTS.md` (verb/tool counts), `docs/KNOWN_ISSUES.md` if any KI closes, C7 contract |

---

## 6. Out of scope (stay in the skill)

- O-RAN/3GPP CR cover tables, Nokia `NOK-YYYY.MM.DD-…` filenames, category B/F rules.
- `********** BEGIN FIRST MODIFIED SECTION **********` marker conventions.
- `[i.N]` informative-reference SEQ numbering.
- Pillar Advanced-guidelines (clause 6.2.x) insertion recipe.
- O-RAN `EX`/`EW`/`NO` excerpt styles (tab layout is generic enough for T5.2 tests, but
  the *style names* and their semantics are domain — don't hardcode).
- Skill's Python dump scripts (`dump_*.py`) — stichos `inspect`/`region` are the product
  equivalent; no parity work needed.
