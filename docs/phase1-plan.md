# stichos — Phase 1 plan: agent-sized steps (M1)

**Status:** active plan · derived from [`phase1-concept.md`](phase1-concept.md) (scope, gate) and
`project_plan.md` §5 task tracks · **61 steps, 6 waves, 6 gates**.
**Progress (update at session close):** `61/61 verified · 6/6 gates · all waves verified`.

> **Session complete (2026-09-10):** All 6 waves executed and verified. Gates A-F all green. P1-61 gate script + runbook complete. M1 ship-ready.
**Canonical progress state:** [`phase1-status.json`](phase1-status.json) — this file defines the steps;
the JSON tracks them. Do not maintain status in two places.
**Last updated:** 2026-09-10

---

## 1. How this plan is executed (agent rules)

- **One agent, one step, one session.** Budget ~200k tokens. Load only: this plan (your step row + your
  wave gate), the named contracts in `docs/contracts/`, the crate(s) you touch, and the named corpus
  subset. Repo-wide scans are not needed; if you need ECMA/OASIS spec text, fetch the specific clause.
- **Write only inside the step's deliverable paths** + your step's row in `phase1-status.json`
  (phase-0 rule of execution, unchanged).
- `project_concept.md` / `project_plan.md` / this file are **not edited by executing agents**. A step
  that contradicts them stops and flags the owner; findings marked measured vs assumed.
- **Split rule:** if a step grows past ~1500 new LOC or needs a second crate family, stop, record
  `blocked: split-needed` with evidence, and let the owner split it. Do not silently sprawl.
- Definition of done: `project_plan.md` §8 (tests, check-deps, deny, clippy/fmt, C3 codes, docs)
  plus the step's **Verify** column, run and recorded as measured evidence.
- Status values: `pending → active → verified | blocked(reason)`. `verified` requires evidence:
  command + result + date (concept §17 discipline — measured, not assumed).

## 2. Progress tracking

- **`phase1-status.json`** (canonical): one entry per step — `id`, `wave`, `status`, `evidence`,
  `depends`. Gate entries flip `pending → verified` only when all steps in the wave are verified **and**
  the wave-gate command below passes (its output goes in the gate's `evidence`).
- **Session close ritual:** update JSON (step status + evidence), then update the **Progress** header
  line at the top of this file: `n/61 verified · g/6 gates · wave X active`.
- **CI:** `pr.yml`'s dormant cargo steps self-activate the moment `Cargo.toml` appears (measured, SA-8) —
  after P1-01 every PR runs `build + test + deny + check-deps`; nightly extends the fidelity job with the
  Rust render-stability sample. CI green is part of `verified` evidence for any step with a CI surface.
- **Gate script:** `tools/phase1_gate.sh` (built in P1-61) asserts the M1 exit gate and prints the
  rollup; it is the ship-time source of truth, the JSON the running one.

## 3. Waves and gates

| Wave | Theme | Gate (command + pass condition) |
|---|---|---|
| **A** | foundations | `cargo build/test/deny/check-deps` green; contract skeletons C1/C3/C4 compile with tests |
| **B** | readers | **read-all test**: every 151 corpus fixture parses to IR; census matches manifest feature expectations (C9) |
| **C** | engines/render | `doctor --json` matrix = real conversions on host + pinned container; pr-subset renders SSIM ≥ threshold vs goldens; loop latency recorded |
| **D** | QA+payload | linter fixture matrix green; C3 v1 frozen; budgeter tests green |
| **E** | surface | end-to-end tool matrix green; INV-2 schema green on **every** tool; `apply` absent |
| **F** | harden+ship | `tools/phase1_gate.sh` green on host + CI (== M1 exit) |

Waves are ordered; steps inside a wave run in dependency order (Deps column), parallelizable only where
deps allow and files don't collide.

## 4. Steps

`Implements` cites `project_plan.md` §5 task IDs; `Verify` = the tests that validate the step.

### Wave A — foundations

| ID | Build → deliverable | Implements | Deps | Verify |
|---|---|---|---|---|
| P1-01 | Workspace scaffold: 8 crates (`gp-core`, `adapters-ooxml/-odf/-pdf`, `engines`, `lint`, `mcp`, `cli`), rust-toolchain (6 targets), workspace lints + SPDX check, `tools/check-deps.sh` (DAG §3.2), activate `deny.toml` + machete in CI | CORE-01, SEC-02 | — | `cargo build --workspace` · `cargo deny check` · `check-deps.sh` green · clippy+fmt · pr.yml cargo job activates green |
| P1-02 | Diagnostic registry + error model: stable prefixed codes, severity, JSON shape; draft `docs/contracts/C3.md` | CORE-02 (C3) | 01 | code-uniqueness + serde-snapshot tests; lint gate: no ad-hoc error strings |
| P1-03 | IR v1: doc/section/para/run/table/list/header/footer/footnote/drawing · sheet/cell/formula+cached-value · slide/layout/master/placeholder · pdf page/annot/form-field; versioned serde; `docs/contracts/C1.md` | CORE-03 (C1) | 01 | serde round-trip property tests; version/migration stub; C1 contract-test skeleton |
| P1-04 | Element IDs (P4, read-stability): content-anchored ID derivation + ID-map serialization; **ADR first** (risk register); edit-stability deferred to C2 freeze (Phase 2) | CORE-04 (C2 draft) | 03 | ADR written (owner flips accepted); property tests: same input ⇒ same ID, survives reparse; collision sweep over corpus |
| P1-05 | Traits (`Codec`,`Engine`,`Probe`,`Linter`; `PatchExecutor` as type-only) + mock testkit loading `corpus/manifests` | CORE-10, CORE-11 | 03 | mocks compile against traits; testkit loads all 151 manifests + `pr-subset.txt` |
| P1-06 | Fidelity-budget read shape: `dropped_features[]`, warnings, `rendered_by`, confidence, merge algebra; `docs/contracts/C4.md` (read part) | CORE-08 (C4) | 02 | algebra unit tests (merge, idempotence, ordering) |
| P1-07 | Content-addressed store: blob layout, doc versions, LRU, `~/.stichos/` layout | CORE-05 | 03 | sha256-keying + LRU + never-overwrite property tests |

### Wave B — readers

| ID | Build → deliverable | Implements | Deps | Verify |
|---|---|---|---|---|
| P1-08 | OPC reader: zip, content types, relationships; limits (entry count, expansion ratio, no entities, path-traversal/symlink reject) | OOX-01, SEC-01(part) | 01 | opens every docx/xlsx/pptx corpus fixture; synthetic zip-bomb + traversal fixtures rejected with C3 codes |
| P1-09 | XML DOM-lite: namespace- and order-preserving, byte spans per node; re-serialize == original bytes | OOX-03 | 08 | byte-exact re-serialize over **all** XML parts of all fixtures; span→slice property tests |
| P1-10 | Word→IR (a): body, runs, tables → IR + IDs | OOX-04a | 09,04 | manifest assertions: `w:p`/`w:tbl` counts match expectations (C9) |
| P1-11 | Word→IR (b): styles, numbering, sections, headers/footers | OOX-04b | 10 | manifest assertions: styles/sectPr/header-ref counts |
| P1-12 | Word→IR (c): fields, comments, tracked changes (read-only representation) | OOX-04c | 11 | counts of `w:fldSimple`/`w:instrText`, comments, `w:ins`/`w:del` vs manifests |
| P1-13 | Excel→IR: cells, formulas **and cached values**, shared strings, styles, merged ranges; charts/pivots detected → budget entries | OOX-05 | 09,04 | formula cells carry formula+cached in fixtures; merged-range counts vs manifests |
| P1-14 | PPT→IR: slides, layouts, masters, placeholders, notes | OOX-06 | 09,04 | slide/layout/master/placeholder counts vs manifests |
| P1-15 | Feature census → read fidelity budget: per-part counts, unknown-part inventory | OOX-07 | 12,13,14,06 | census matches manifest expectations across corpus; unknown-part list non-empty on OLE fixtures |
| P1-16 | XSD validation (embedded ECMA schema set, strict/loose); generate broken-fixture set in-step | OOX-08 | 09 | all corpus fixtures pass loose mode; broken set fails with expected C3 codes |
| P1-17 | ODF package reader: `manifest.xml`, encryption detection (flag only) | ODF-01(read) | 08 | opens all odt/ods/odp fixtures; synthetic encrypted flagged |
| P1-18 | ODT→IR: content/styles/meta | ODF-02 | 17,09,04 | paragraph/table/style counts vs manifests |
| P1-19 | ODS + ODP→IR: cells/formulas+cached; slides/layouts/notes | ODF-03 | 18 | per-format manifest counts |
| P1-20 | PDF object reader: xref + xref/object streams, page tree, encryption read-only best-effort policy | PDF-02 | 01 | opens every corpus-derived PDF; malformed inputs → C3 codes |
| P1-21 | PDF text + word bboxes: own content-stream extractor + poppler `-bbox` rung when probed; **G5 evidence report** (ADR-0001 follow-up) | PDF-04 (D19) | 20 | parity vs poppler `-bbox` on the PDF sub-corpus; report quantifies gaps and feeds gate G5 |
| P1-22 | AcroForm read: field tree + values; XFA detected → unsupported flag | PDF-05(read) | 20 | field counts vs form fixtures; XFA fixture flagged |
| P1-23 | Annotations, metadata, outline, tagged-structure extraction (a11y feed) | PDF-06(read) | 20 | annot counts vs fixtures; outline serialization test |
| P1-24 | `qpdf --check` runner: report parsed → C3 diagnostics; graceful skip when binary absent | PDF-07 | 20,02 | runs over PDF sub-corpus; planted broken PDF → expected codes |

### Wave C — engines & rendering

| ID | Build → deliverable | Implements | Deps | Verify |
|---|---|---|---|---|
| P1-25 | Engine API + capability matrix: engine id/version/`fontset_hash`, `rendered_by` builder (D6); `docs/contracts/C5.md` | ENG-01 (C5) | 05 | mock-engine tests; matrix serde; provenance format tests |
| P1-26 | LibreOffice runner: isolated `-env:UserInstallation`, `--convert-to` + filter options, hard timeout + wedge kill, instance pool, version parse | ENG-03 | 25 | converts `pr-subset` in pinned container; timeout-kill test; version == `docker/harness/pin.txt` |
| P1-27 | Probe framework: discovery + fixture-conversion probes (**capability = real conversion**), cache, N/N-1 (D15) | ENG-02 | 26 | probe returns host truth; synthetic broken-LO case → capability false despite binary present |
| P1-28 | Raster ladder (D11): poppler → gs → mutool → `soffice --convert-to png` behind one `rasterize()` | ENG-07 | 25 | same page through available rungs; dimensions/DPI as specced |
| P1-29 | Master cache: 150-DPI masters + on-demand 300-DPI for crops; key `sha256(doc)+engine+ver+fontset_hash+dpi` (INV-4); LRU | ENG-08 | 28,07 | hit/miss matrix; key rotates on pin or `fontset_hash` change; eviction tests |
| P1-30 | Renderer negotiation (§9): per-platform order, `renderer.preference`, session cache, honest `visual_feedback:"unavailable"` | ENG-04 | 27 | ordering/override unit tests per platform cfg; unavailable path returns honest field |
| P1-31 | `doctor`: engine × bundled-fixture matrix (docx/xlsx/pptx from corpus, §8), `fc-match` font table, LO-version notes; human + `--json` | ENG-05 | 27 | `doctor --json` schema test on host + container; matrix reflects real conversions |
| P1-32 | pandoc runner (optional, small): office→markdown text extraction only (md→docx deferred to Phase 3) | ENG-06(read) | 25 | text-extraction F1 parity vs harness metrics on `pr-subset` (C9) |

### Wave D — QA & payload

| ID | Build → deliverable | Implements | Deps | Verify |
|---|---|---|---|---|
| P1-33 | Layout linter core: frame overflow, off-page/negative coords, overlaps, autofit shrink — inputs word bboxes + page geometry; no VLM | LIN-01 | 21,29,03 | synthetic bad-layout PDFs raise each code; clean goldens raise none |
| P1-34 | Font substitution + missing-glyph checks: requested-vs-used, tofu via raster heuristic | LIN-02 | 33 | Calibri→Carlito fixture reports substitution; tofu fixture detected |
| P1-35 | Page-count delta + contrast checks | LIN-03 | 33 | unit + golden comparisons |
| P1-36 | Perceptual diff (SSIM) in Rust with **parity vs harness metrics** | LIN-06 | 29 | parity within C9 tolerance on golden pairs |
| P1-37 | Model-copy encoder: pre-resize to known sizes, JPEG q≈85/WebP, byte-exact accounting | REN-01 | 29 | byte-accounting tests; resize-table conformance |
| P1-38 | Token estimator + payload budgeter: `⌈w/28⌉×⌈h/28⌉`, ≤20 blocks, ≤2000 px/side rule; `docs/contracts/C8.md` | REN-02 (C8) | 37 | table-driven estimator tests; budgeter reject tests |
| P1-39 | Coordinate bookkeeping: `bbox_pt` ↔ resized-pixel, scale recorded per payload | REN-04 | 21,37 | property: pt→px→pt round-trip ≤1 px |
| P1-40 | `inspect_region` cropper: bbox+zoom from 300-DPI master, ~600-token budget | REN-05 | 29,39 | crop-position correctness; token-budget assert |
| P1-41 | Contact-sheet builder (M1 scope: full-doc overview; changed-page logic lands with M2 edits) | REN-03(M1) | 37 | layout tests; sheet cost ≤ budget |
| P1-42 | IR structural diff + XML part diff renderers (feeds `diff_documents`; anchored to element IDs) | LIN-08 | 03,04,09 | golden diffs on fixture pairs; ID stability under move/rename cases |
| P1-43 | **C3 v1 freeze**: every crate emits registry diagnostics only; JSON schema frozen | LIN-07 (C3) | 33–42 | grep-gate: no ad-hoc strings in user-visible output; contract suite green; C3 marked frozen in `docs/contracts/` |

### Wave E — surface (MCP + CLI)

| ID | Build → deliverable | Implements | Deps | Verify |
|---|---|---|---|---|
| P1-44 | Tool spec v1 read set (C7): the 5 tools of concept §3 + `serve`; schemas, idempotency, versioning/deprecation policy; `docs/contracts/C7.md` | API-01 (C7) | 31,42,43 | spec + schema fixtures; consistency test spec↔types |
| P1-45 | MCP transport ADR (`rmcp` vs own JSON-RPC) + stdio server; result shape `content`+`structuredContent`+`isError` (§10) | API-02 | 44 | ADR; handshake + list_tools/call_tool tests |
| P1-46 | Streamable HTTP mode: `serve --http :8731` + session handling | API-03 | 45 | session lifecycle tests |
| P1-47 | CLI skeleton: clap; verbs `inspect render diff doctor serve` (**no `apply`**); `--json`; exit-code table; record gate G2 (`gp` alias) | API-04 | 44 | exit-code + help snapshot tests; G2 outcome recorded in status JSON |
| P1-48 | `inspect_document` e2e: structure + IDs + coordinates manifest + fidelity budget | API-05a | 45,47 | e2e on `pr-subset`: JSON schema, IDs present, budget present |
| P1-49 | `render_document` e2e: ≤20 image blocks + contact sheet + manifest | API-05b | 45,47,29,37,38,41 | payload budget enforced; SSIM vs goldens; `rendered_by` correct |
| P1-50 | `inspect_region` e2e: crop + coordinate conversion in payload | API-05c | 45,47,40 | crop e2e correctness; scale recorded |
| P1-51 | `doctor` + `diff_documents` tools | API-06 | 45,47,31,42 | doctor JSON parity CLI↔MCP; diff goldens |
| P1-52 | Payload-budget enforcer wired into every image-returning tool + INV-2 response-schema test over **all** tools | API-08 | 48–51,38 | schema matrix green; >20-image requests rejected |

### Wave F — harden & ship

| ID | Build → deliverable | Implements | Deps | Verify |
|---|---|---|---|---|
| P1-53 | Fuzz OPC/XML/PDF ≥24 h + malicious-package corpus (zip bombs, XXE, traversal, oversized parts) | SEC-01 | 08,09,20 | cargo-fuzz CI jobs clean 24 h; corpus regressions locked as tests |
| P1-54 | Macro-format policy: xlsm/docm detected → flagged, disabled (no execution path exists) | SEC-05 | 08 | macro fixtures flagged; assert-by-construction |
| P1-55 | Threat model + M1 security review | SEC-06 | 53,54 | `docs/threat-model.md`; review checklist green |
| P1-56 | Cross-compile matrix: 6 targets, old-glibc + musl fallback; CI "no C in tree" check (D2) | DIS-01 | 01 | matrix green; gate catches a planted `-sys` dep |
| P1-57 | Linux: deb + rpm wrapping the same binary, `Recommends:` only; tarball + `install.sh` | DIS-02 | 47,56 | container install + `doctor --json` runs; metadata lint proves no `Depends:` |
| P1-58 | macOS: universal `.pkg` via `lipo` + codesign; Homebrew cask draft | DIS-03 | 47,56 | builds on CI macOS runner; `brew audit`; **notarization = owner-bound, flagged** |
| P1-59 | Windows: MSI (WiX) + portable ZIP + winget manifest | DIS-04 | 47,56 | MSI builds on CI; winget manifest validates; **Authenticode = owner-bound, flagged** |
| P1-60 | Font bundling: license audit Carlito/Caladea/Liberation/Noto subset; first-run fontconfig strategy | DIS-07 | 31 | license table; fontless-container render uses bundle (fc-match evidence) |
| P1-61 | M1 acceptance: runbook + `tools/phase1_gate.sh` asserting the exit gate (installs, INV-2, fuzz status, doctor matrix, edit-absent); freeze report | ship | all | gate script green host + CI; acceptance report published |

## 5. M1 exit-gate ↔ steps map

| Gate item (plan §2 / concept §7) | Steps | Asserted by |
|---|---|---|
| installs clean × 6 targets | P1-56..59 | `phase1_gate.sh` + acceptance runbook |
| provenance on every tool response | P1-25, P1-52 | INV-2 schema matrix (CI) |
| read-path fuzz-clean | P1-53 | nightly fuzz status read by gate |
| doctor = real conversions | P1-27, P1-31 | gate re-runs `doctor --json`, checks matrix |
| edit tools absent | P1-47, P1-44 | verb/tool-list assert in gate |
| DoD per task + 6 wave gates | all | `phase1-status.json` full `verified` |

## 6. Effort

Sum of cited task estimates from `project_plan.md` §5/§9 restricted to this phase: **≈ 115.5 p-w**
(planning estimate, senior engineers — not a measured claim). Agent execution is serial, so wall-clock
is not the plan's W2 calendar; **progress is the status JSON + gates, not dates.**

## 7. Owner-bound items surfaced by this plan

- Gate **G2** (`gp` alias) must be decided before P1-47 freezes the CLI.
- Gate **G5** evidence arrives from P1-21 (bbox parity report) — owner flips D-log if pdfium enters.
- macOS notarization + Windows Authenticode signing (P1-58/59) need certificates/accounts.
- GOV-02 registration sweep still blocks **public** M1 announcement only (M0 tail).
- P1-04 ADR (element IDs) needs owner acceptance before Wave B readers ship IDs in outputs.
