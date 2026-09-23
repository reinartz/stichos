# stichos — Phase 6 plan: agent-sized steps (M6)

**Status:** active plan · derived from [`phase6-concept.md`](phase6-concept.md) (scope, gate;
incl. §11 Wave I extension) and `project_plan.md` §5 tracks (CTX, PAT, API, ENG/COM, OOX/ODF,
SEC, OPS, GOV) · **35 steps, 8 waves (0, C, F, R, W, G, I, X), 8 gates**.
Wave I (P6-20..P6-34, feature-completeness gap-fill, owner calls C10–C13) executes after Wave G
and **before Wave X**; the `m6` tag waits on the Wave I gate (concept §7 item 10).
**Canonical progress state:** [`phase6-status.json`](phase6-status.json) — this file defines the steps;
the JSON tracks them. Do not maintain status in two places.
**Last updated:** 2026-09-22 (Wave I added; Wave X deps extended)

---

## 1. How this plan is executed (agent rules)

- **One agent, one step, one session.** Budget ~200k tokens. Load only: your step row + wave gate
  here, the named contracts in `docs/contracts/` + ADRs, the crate(s) you touch, the named corpus
  subset. For formula work, fetch only the SpreadsheetML clause you need (`<c>`, `<f>`, `<v>`,
  shared formulas); for COM, only the interface you automate — never the whole spec.
- **Sizing rule:** every row scoped to one 200k session (≈ ≤1 200 new LOC + tests). **2s** rows =
  two sequential sessions with a written handoff in the step's status entry. If a step grows past
  scope, stop, record `blocked: split-needed` + evidence; do not sprawl.
- Write only inside the step's deliverable paths + its row in `phase6-status.json`.
- `project_concept.md` / `project_plan.md` / `phase6-concept.md` are **not edited by executing
  agents** (no step writes them; Wave 0 executes the DAG, it doesn't amend it).
  A contradiction stops the step and flags the owner. Measured vs assumed, always.
- Definition of done: `project_plan.md` §8 + the step's **Verify** column, run and recorded with
  command + result + date.
- Status: `pending → active → verified | blocked(reason)`.
- **Headless discipline (M4/M5-proven):** verdicts from **exit codes**, never from grepping piped
  output; everything must work with no `/dev/tty`. COM steps run on the Windows host; their
  verify commands are PowerShell/`cargo test` with exit-code verdicts, recorded the same way.
- **Ship-state rule (M4 lesson) + re-run rule (M5 §9 lesson):** a milestone tag moves only to a
  **committed** gate-green tree — commit → gate → tag → push, in that order; acceptance report
  committed before the tag; **and the gate is re-run after the report commit, before the tag**,
  so the transcript in the report reproduces at the tagged tree (P6-19 encodes the order).
- **Wave-0 merge gate:** no step outside Wave 0 merges until P6-00 is `verified`
  (`tools/check-deps.sh` exits 0 at a committed tree). Steps may *execute* in parallel against
  the pre-merge tree, but merge in wave order.
- **Host facts (measured 2026-09-20):** Linux box: pandoc 3.7.0.2, soffice 26.2.6.1, pdftoppm,
  xmllint, node v22 ✅; typst/qpdf/veraPDF ✗ (CI installs; honest-absent paths stay first-class).
  **Windows + M365 host available** (C7) — Wave W and P6-10 run there. Corpus: 169 manifests,
  `pr-subset` 27. Baseline surface: **23 tools / 21 verbs, C7 v1.5.0** (measured, not carried).
- Collision hotspots — **sequential edits only**: `mcp/spec.rs` + `docs/contracts/C7.md`
  (P6-02 → P6-03 → P6-04 → P6-05 → P6-06, enforced by the Deps chain; Wave I amendments continue
  the C7.md chain P6-21 → P6-22 → P6-27 → P6-30); `gp-core/src/ir.rs`
  (P6-01 envelope hooks, then P6-08 formula fields — P6-08 lists P6-01 in Deps);
  `cli/src/main.rs` dispatch (each CTX step appends its own flags; rebase in Deps order;
  Wave I touches it only in P6-27, P6-30, P6-34 — take them in that order);
  `cli/src/inspect.rs` (P6-21 → P6-22), `cli/src/diff_tool.rs` (P6-23 → P6-24),
  `gp-core/src/executor.rs`+`patch.rs` (P6-25 → P6-26), `cli/src/region.rs`+`mcp/src/http.rs`
  (P6-28 → P6-29).

## 2. Waves and gates

| Wave | Theme | Gate (command + pass condition) |
|---|---|---|
| **0** | DAG-inversion closeout | `tools/check-deps.sh` exits 0; `mcp` → `gp-core` only; P5-09/P5-10 text-layer e2e green with zero behavior delta |
| **C** | context budget (CTX-01..06) | summary < 500 tokens; zero base64 in URL-mode JSON; caps asserted; `__budget` on 100 % of responses; legacy opt-outs restore old behavior |
| **F** | formula eval (ADR-0011 + C15 + subset) | ADR accepted; subset evaluates with cached-vs-computed parity lint; unsupported → budget + cached value, never invented; GP-F0xx refusals exact |
| **R** | ODS/ODP revisions + Word interop | ODS/ODP accept/reject lossless on corpus subset; Word-interop accept round-trip lossless (OOXML + ODS/ODP) |
| **W** | COM workers on Windows host | probe matrix from real conversions; Word/Excel/PPT one-doc-per-instance with timeouts; `CompareDocuments` e2e; logon-agent packaging rehearsed |
| **G** | launch rehearsal (no launch) | hygiene pack green; publish dry-runs exit 0 and recorded; absence of any publish asserted |
| **I** | feature completeness (gap-fill; concept §11, calls C10–C13) | six-format inspect/diff; `--apply`/`edit` write real bytes + journal + explicit refusals; `revisions` exposed for odt/ods/odp; `sign_document` dispatched (spec↔dispatch parity test); region/review serve real pixels with honest `rendered_by`; ODS `of:=` parity; `parser.rs:498` panic fixed; P6-33 host evidence recorded |
| **X** | security + ship | fuzz round 5 + SEC-06; `phase6_gate.sh` green headless + CI (asserts §7 items 1–**10**); C7 v1.6 additive scan incl. Wave I matrix rows; gate re-run after report commit; tag `m6` on committed tree |

Waves ordered 0 → C/F/R/W/G/I (F, R, W, G, I parallel after Wave 0, Deps permitting) → X last.
Inside Wave C run strictly in Deps order (shared-file chain). Inside Wave I the read-side chain
P6-20 → {P6-21, P6-22, P6-23, P6-24} and the mutation chain P6-25 → P6-26 are strict; the rest
are independent (§5 notes).

## 3. Steps

`Implements` cites `project_plan.md` §5; `Verify` = evidence the step must produce.
**Size:** 1 = one 200k session, 2s = two with handoff.

### Wave 0 — DAG-inversion closeout

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-00 | Remove the `mcp -> adapters-pdf` edge by dependency inversion: new `gp-core` trait for PDF text-span extraction (e.g. `PdfTextSource`, with span/rect/quote types); move the `adapters-pdf` impl into `adapters-pdf`; move concrete wiring into `cli` (de-facto wiring point); `mcp/http.rs` text-layer route calls only the trait; drop the edge from `crates/mcp/Cargo.toml`; no other `mcp` edge changes | hygiene, CORE | — | 1 | `bash tools/check-deps.sh` exits 0; `cargo test -p mcp --lib review` + `http::tests` green; old-vs-new route JSON byte-identical on P5-02 fixtures (parity test committed); `grep adapters-pdf crates/mcp/Cargo.toml` → 1 (exit 1) |

### Wave C — context budget (CTX-01..06; strict Deps chain)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-01 | CTX-01 envelope in `gp-core`: `RenderOutput`/`InspectDetail` enums, `ResponseBudget` (`__budget: tokens_est, elements_count, image_count`), shared preview shape; reuse REN-02 token estimator; extend the INV-2 schema gate to require `__budget`+`diagnostics`+`rendered_by` on every tool response | CTX-01, CORE | P6-00 | 1 | envelope round-trips serde; estimator unit tests; schema-gate test fails on a deliberately unenveloped fixture response (negative test) |
| P6-02 | CTX-02 `inspect_document` detail levels: `detail=summary\|elements\|all` (default **summary** ~150 tokens), `max_elements` (100), `hasMore` + cursor pagination (MCP pagination spec); CLI `--detail/--max-elements`; spec params + surgically-scoped C7.md rows | CTX-02, API | P6-00, P6-01 | 1 | summary of the 50-page corpus fixture < 500 tokens (bytes + tokens asserted); `elements` caps at 100 with `hasMore`; `all` == legacy output byte-identical |
| P6-03 | CTX-03 temp-file image path: `TempFileManager` (`$XDG_RUNTIME_DIR/stichos/renders/`, `{prefix}_{hash}_{page}.png`, 1 h TTL + startup sweep, `expiresAt`); `output=urls\|base64\|both` (default **urls**) + `max_pages` on `render_document` (3) / `inspect_region` (urls only) / `annotate_pages` (5); CLI `--output/--max-pages`; spec params + C7.md rows | CTX-03, API | P6-02 | 2s | URL-mode JSON contains zero base64 (gate-greppable); TTL + startup-sweep tests with aged files; `both` returns URLs and blocks for the same pages |
| P6-04 | CTX-04 caps + summaries: `diff_documents` `max_ops` default **20** + `summary` counters + `hasMore`/`totalOperations`; `journal` `max_entries` default **10**; CLI `--max-ops/--max-entries`; spec params + C7.md rows | CTX-04, API | P6-03 | 1 | large-diff pair returns exactly ≤20 ops with `hasMore` + totals; long-journal fixture returns ≤10 entries; explicit large `max_ops` restores full output (legacy test) |
| P6-05 | CTX-05 mutation-tool safety: `dry_run` default **true** for `apply_patch` + `edit_text/cell/structure`; ≤20-op preview with `estimatedTokenChange` + `diffSummary`; `includeFullDiff` opt-out; CLI `--dry-run/--apply`; spec descriptions + C7.md rows | CTX-05, API | P6-04 | 1 | omitted `dry_run` writes nothing (fs-assert zero writes) and returns preview; `dry_run=false` applies (legacy test); preview caps at 20 ops |
| P6-06 | CTX-06 surface freeze: C7 **v1.6.0** matrix + per-tool `schema_version` bumps, automated v1.5→v1.6 additive-only diff, CHANGELOG **default-flip** section + migration notes (C14 §8), C14 §7.4 contract-compliance tests (summary budget, URL purity, caps, dry_run default, cleanup) | CTX-06, API | P6-05 | 1 | additive-only scan exits 0; all §7.4 tests green; CHANGELOG names every flipped default with its opt-out flag |

### Wave F — formula evaluation

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-07 | **ADR-0011** (evaluator choice, supported subset, cached-value semantics, GP-F0xx refuse-list, parity-lint design; reserves number in `docs/adr/README.md`) + **C15 draft** (`docs/contracts/C15-formula-evaluation.md`: supported function set + versioning, cached-vs-computed parity rules, GP-F0xx codes registered in C3) | CORE (C15), SEC | P6-00 | 1 | ADR accepted by owner; contract doc exists; codes in C3 registry; refuse-list machine-readable (test enumerates it) |
| P6-08 | Minimal evaluator subset (arithmetic/aggregate/logic per ADR-0011) in `adapters-ooxml` (new `formula` module): evaluate → IR values; **cached-vs-computed parity lint** on every evaluated cell; unsupported → budget entry + cached `<v>` surfaced, never invented; formula fixtures (builders + manifests + goldens, C9 extended); read-path census parity | PAT, OOX-ext | P6-07, P6-01 | 2s | parity lint catches an injected stale-cache fixture (golden); hostile formulas (deep nesting, external refs, macro calls) refuse with exact GP-F0xx; subset functions golden-evaluated vs cached values |

### Wave R — ODS/ODP revisions + Word interop

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-09 | ODS/ODP revision write (C11 extended): accept/reject on Calc/Impress change tracking reusing the odt-first pattern (`revision_reader`/`revision_writer`/`revision_executor` in `adapters-odf`); lossless round-trip suite on a corpus subset; refusal matrix for unsupported change kinds | ODF-ext, C11 | P6-00 | 2s | accept→bytes→read shows accepted state exactly (per-format goldens); reject restores byte-identical original on the subset; unknown change kinds refuse, never silent-drop |
| P6-10 | Word-interop verification harness on the Windows host: M4 OOXML revision fixtures + P6-09 ODS/ODP fixtures through real Word (open → accept → save → our reader); lossless assertions; host + build matrix recorded in `docs/support-matrix.md` | HUM-ext, OPS | P6-09 | 1 | interop suite green on the host (exit codes recorded); any Word-vs-ours divergence filed as a corpus golden, not a silent pass |

### Wave W — COM workers (Windows host; `ms-office` feature flag)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-11 | COM-2 probing for real: ProgIDs (`Word/Excel/PowerPoint.Application`), `HKCR\...\CurVer`, ClickToRun `ProductReleaseIds`, `WINWORD.EXE` under `root\Office16`; probe-never-assume incl. Store-edition paths; results feed `doctor` capability rows from fixture conversions (D8) | ENG (COM-2) | P6-00 | 1 | on the host: `doctor --json` probe rows cite real conversions; with Office absent: honest-absent rows (negative test); Store-vs-ClickToRun divergence handled, never assumed |
| P6-12 | COM-3 Word worker via `windows` crate: `DisplayAlerts=None`, `AutomationSecurity=High`, add-ins off, one-doc-per-instance, hard timeout + wedge kill, activation/first-run dialog handling; render + accept/reject path through Word | ENG (COM-3) | P6-11 | 2s | render a corpus docx through Word with `rendered_by: "Word 16.0 (…)"`; wedge-kill test (hung instance reaped, timeout recorded); activation-dialog host documented |
| P6-13 | COM-4 Excel + PowerPoint workers on the COM-3 harness (per-app quirks isolated per worker); render paths for xlsx/pptx with provenance | ENG (COM-4) | P6-12 | 2s | xlsx + pptx corpus renders through Office with versioned provenance; per-app quirk table in module docs; LO fallback order unchanged when Office absent |
| P6-14 | COM-5 `CompareDocuments` authoritative track-changes diff tool (internal + `diff` backing when Office is the negotiated renderer) | ENG (COM-5) | P6-12 | 1 | doc-pair diff through Word matches our IR diff on the M4 fixture subset (or divergences documented as goldens with provenance blamed correctly) |
| P6-15 | COM-6 logon-agent packaging: MSI component + task-scheduler XML + docs (per-user logon agent, never a service — D10); unattended-batch story documented | ENG (COM-6), DIS | P6-13 | 1 | package rehearsal installs the agent on the host VM; scheduled-task activation demonstrated; service-install explicitly refused/documented |

### Wave G — launch rehearsal (no launch)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-16 | GOV/DIS rehearsal: hygiene pack re-verified (headers, licenses, no-local-paths scan); `cargo publish --dry-run`, winget/brew manifest validation, release-asset rehearsal — all recorded; **absence asserted** (no tags pushed, no registry contact); D26 citation | GOV-ext, DIS-ext | P6-00 | 1 | every recorded command exit 0 with `--dry-run` pinned in command lines; absence checks green (tag list, registry untouched) |

### Wave I — feature completeness (gap-fill; concept §11; owner calls C10–C13)

Discovered by the 2026-09-22 measured surface census (concept §11.1 — every row cites file:line).
One agent, one step, same 200k sizing rule. Read-side chain (P6-20 → 21/22/23/24) and mutation
chain (P6-25 → 26) are strict; everything else in the wave is independent of the others.

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-20 | `Spreadsheet`/`Presentation` → `IRDocument` tagged-tree converters in `gp-core` (new `ir_convert.rs`): document→sheet/`slide`→row→cell and →shape/text nodes; reuse existing `ElementId`s (C2 stability); cell alt-text = display value; `unknown`/hostile workbooks degrade to empty subtrees, never panic; no `cli`/`mcp` changes yet | CORE | P6-00 | 1 | converter unit tests; corpus xlsx+pptx round-trip: tree walk yields stable IDs across two parses; empty/malformed inputs no panic; `cargo test -p gp-core --lib` green, clippy/fmt clean |
| P6-21 | `inspect_document` for **xlsx/pptx**: extend `parse_document` dispatch (`cli/src/inspect.rs:53-83`) via P6-20 converters; `document_type` = `spreadsheet`/`presentation`; summary object gains `sheetCount`/`slideCount`; pptx `pages[]` = slides; formula cells expose `formula` + `value` (+ parity flag from the read-path evaluator); C7.md **v1.6.0 support-matrix amendment** + CHANGELOG (unreleased, call C11); CLI+MCP parity (no new params); C9 fixtures (xlsx×2, pptx×2 manifests + goldens) | CTX, API, PAT, C9 | P6-20 | 1 | default summary < 500 tokens on the corpus xlsx + pptx fixtures; `detail=elements` on the formula fixture shows `formula`+cached value; docx/odt/pdf outputs byte-locked (existing tests green unchanged); envelope-compliant on every response |
| P6-22 | `inspect_document` for **ods/odp** (adapters-odf readers + P6-20 converters): same summary shape; C9 fixtures ods/odp; C7 matrix row | CTX, API, ODF-ext, C9 | P6-20, P6-21 (C7.md chain) | 1 | same gates as P6-21 on ods/odp fixtures; unknown extensions still explicit errors; envelope-compliant |
| P6-23 | `diff_documents` for **xlsx/ods**: cell-level diff ops (value/formula change, sheet add/remove) through the existing `max_ops`/`summary`/`hasMore` caps (CTX-04 unchanged) | PAT, API | P6-20 | 1 | cell-pair diff goldens; caps honored (`≤20` default, `hasMore`/`totalOperations`); docx/odt/pdf diff byte-locked unchanged |
| P6-24 | `diff_documents` for **pptx/odp**: slide add/remove + shape/text diff; same caps | PAT, API | P6-20, P6-23 (diff_tool.rs chain) | 1 | slide-pair diff goldens; caps honored; existing diff tests unchanged |
| P6-25 | Real write-back for `apply`/`edit text\|cell\|structure`: `gp-core` executor delegates to the existing adapter executors (docx `docx_writer`, xlsx/pptx `OoxmlExecutor::execute_*`, odt ODF patch writer) — the orphaned paths become the live path; journal entry + `revert` round-trip on the apply path; remove the stub-message sniff (`apply_tool.rs:87-95`); ods/odp/pdf refuse with explicit GP-I-xxx codes (C3 registry; never silent success); C13 signature-refusal precedence kept | CORE, API, HUM | P6-00 | 2s | `--apply` writes a byte-valid file re-readable by its adapter and renderable by LO (exit-code evidence); `edit_cell` on the xlsx golden lands the new value; refusals write zero bytes (fs-assert); journal entry recorded + `revert` restores original bytes; C7.md note |
| P6-26 | Implement `PatchOp::Insert`/`TrackInsert`/`TrackDelete` (`gp-core/src/patch.rs:97-129`) + docx tracked-change landing through the revision-writer path — **D7 becomes true via the patch pipeline**: `edit_structure insert` creates a real element; `direct=false` produces `w:ins`/`w:del` visible to our own revisions reader | CORE, PAT, D7 | P6-25 | 2s | insert/delete/track goldens; the tracked ops appear in `revisions list`; reject restores the original state; dry-run preview counts match applied ops |
| P6-27 | `revisions` verb + MCP tools exposed for **odt/ods/odp**: list/accept/reject via the `adapters-odf` revision reader/executor (the P6-09 capability, routed); replace the silent-empty list (`revisions_tool.rs:218-220`) and `"only docx supported"` with explicit per-format refusal codes; review snapshot mode label honesty (`main.rs:2323-2331`) + per-format snapshot | HUM, API, ODF-ext, C11 | P6-09 | 2s | accept/reject round-trip through the verb on odt/ods/odp fixtures matches the P6-09 library goldens; `revisions list` on pdf/xlsx returns a refusal diagnostic, never empty-success; review snapshot mode matches the file's real format |
| P6-28 | Real master-page provider: wire the engines cache (`MasterCache`, currently dead code) / render pipeline behind `read_master_page` (`cli/src/region.rs:126-135`); real pixels; `rendered_by` names the engine that actually ran (drop the unconditional `"libreoffice + poppler"` claim); correct page selection (KI-02 adjacency) | REN, ENG | P6-00 | 2s | region crop on corpus pdf + docx yields non-uniform pixels matching the rendered page (byte-level golden crop); provenance matches the engine actually invoked (mock-engine negative test); second call served from cache (no re-render, asserted via call count) |
| P6-29 | Review page-image route real: `http.rs:680-686` 501 → real pages via the P6-28 provider (update the 501-locked test); KI-05: `render_document` response reports the output dir/path; `annotate_pages` `marked` artifact decision: real numbered-box overlay **or** honest single-artifact rename + diagnostic — never a silent mirror (`annots.rs:341-362`) | API, REN | P6-28 | 1 | review page endpoint serves real PNGs (token-gated) with 200s, 501 path gone; render JSON carries the path; `marked` is either distinct real overlay bytes or absent-with-diagnostic |
| P6-30 | Wire the advertised MCP `sign_document` tool (`spec.rs:897`, never dispatched → `unknown tool`) onto the existing signing path; spec↔dispatch parity test (every `TOOLS` entry has a dispatch arm; 23 tools / 21 verbs unchanged); e2e sign via MCP stdio + HTTP | API, TRUST | P6-00 | 1 | `sign_document` returns the same artifact as `stichos sign` on the fixture; parity test enumerates 23/23 in CI; envelope-compliant response |
| P6-31 | ODS formula evaluation: `of:=` translation into the C15 subset evaluator (reuse the `adapters-ooxml` formula core; shared subset definition), cached-vs-computed parity lint, GP-F0xx refusal semantics unchanged; C9 fixtures | ODF-ext, C15, PAT | P6-08 | 1 | `of:=` goldens vs cached values; unsupported → budget entry + cached value, never invented; refuse-list tests port to ODS; `cargo test -p adapters-odf --lib` green |
| P6-32 | Fix the adapters-pdf `parser.rs:498` subtract-overflow panic on LO-produced PDFs (recorded blocker since P6-03, present at the `m5` tree): fix + regression corpus (the 3 reproducing fixtures); **unblock the render/annotate binary e2e** — the envelope/zero-base64 e2e deferred by P6-03/P6-06 lands here | PDF, SEC, C14 | P6-00 | 1 | previously-panicking fixtures parse exit 0; `stichos render` + `annotate` binary e2e green with `__budget` + zero-base64 URL-mode JSON; fuzz seed added to `adapters-ooxfuzz` |
| P6-33 | **Wave W evidence re-record on the Windows + M365 host (call C13, tag-blocking)**: re-run the P6-11 probe matrix, P6-12/13 Word/Excel/PPT worker renders, P6-14 `CompareDocuments`, P6-15 packaging rehearsal; every evidence string cites "ran on host X, build Y" (plan §5 rule); record the Word PNG-export-is-PDF-fallback quirk in `support-matrix.md`; divergences → corpus goldens with blamed provenance, never silence | ENG, OPS | P6-11 | 1 | host-cited evidence in the status JSON for P6-11..15 + gate W evidence refreshed with the host run; honest-absent rows asserted with an app hidden; PowerShell + `cargo test` exit codes recorded |
| P6-34 | Hardening + doc-corrections pack: `cmd_update` hard-coded workspace fixture path (`main.rs:2193-2195`) → flag/env with honest error when absent; stale-doc sweep (`CODEBASE.md`, `CHANGELOG.md` ms-office entries, `AGENTS.md` phase text, `support-matrix.md` "compile-only skeleton" row, `engines/Cargo.toml` "deferred to Phase 5" comment, `formula/mod.rs` stale handoff checklist); ADR-0011 accept + C15 freeze checklist for P6-19; Phase-7 intake section drafted (real CMS/ADR-0010 reserved, xlsx/pptx create, full formula coverage, lint encode/ssim refresh) | GOV, OPS | P6-00 | 1 | greps for the stale phrases exit 1 (in target files); `update` verb configurable via `--fixture-path` or `STICHOS_FIXTURE_PATH`; intake section committed in §12 of phase6-concept; hygiene pack still green |

### Wave X — security + ship

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P6-17 | Fuzz round 5 + SEC-06: hostile CTX inputs (giant `max_elements`/`max_ops`, cursor forgery, temp-path traversal), hostile formulas (deep nesting, huge `/ByteRange`-class ints, external-ref storms), COM boundary strings, **Wave I surfaces (sheet/presentation inspect inputs incl. `of:=`, apply-op fuzz against the real writers, `parser.rs` regression corpus)**; regressions locked in `adapters-ooxfuzz`; threat-model Phase-6 section + SEC-06 checklist | SEC-01, SEC-06 | P6-05, P6-08, P6-13, P6-21, P6-25, P6-31, P6-32 | 2s | new fuzz tests green as regressions; clippy clean on fuzz crate; SEC-06 checklist committed; ≥24 h nightly-clean path recorded (CI) |
| P6-18 | `tools/phase6_gate.sh`: asserts concept §7 items 1–**10** **headless** (inversion, one-turn defaults, preview safety, C7 v1.6 scan incl. Wave I matrix rows, COM goldens with host-absent skips, formula/revision/interop suites, fuzz status, rehearsal-absence check, hygiene, **Wave I completeness: six-format inspect/diff, real apply writes + explicit refusals, revisions odt/ods/odp, sign dispatch parity, region real pixels, parser-e2e unblocked**); planted-violation proof; CI job `phase6-gate.yml`; **post-report re-run hook** (gate runs green, then runs again after the acceptance commit — P6-19) | ship | P6-16, P6-17, P6-20..P6-34 (all Wave I) | 1 | gate exit 0 headless + CI green; planted violation (silenced cap test / restored `mcp` edge / re-stubbed apply / un-dispatched `sign_document`) → exit 1 |
| P6-19 | M6 acceptance report `docs/m6-acceptance.md`: all gate checks with measured vs assumed (what stayed CI/host-only); **Wave I gap table closed item-by-item with before/after evidence**; **commit → gate → report-commit → gate-re-run → tag `m6` → push** in that order (ship-state + re-run rules); Phase-7 intake (launch flip decision, formula coverage gaps, **real PKCS#7/CMS signing — ADR-0010 reserved**, xlsx/pptx `create`, lint encode/ssim refresh, PAdES-T/LTV, redaction) | ship | P6-18 | 1 | report committed **before** tag; gate re-run after the report commit exits 0; `test "$(git rev-parse 'm6^{commit}')" = "$(git rev-parse HEAD)"` exit 0; push with owner approval |

## 4. M6 exit-gate ↔ steps map

| Gate item (concept §7) | Steps | Asserted by |
|---|---|---|
| Wave-0 closeout | P6-00 | `check-deps.sh` exit 0 + text-layer parity suite |
| One-turn defaults | P6-01..06 | §7.4 contract tests + gate JSON greps (no base64, caps, `__budget`) |
| Preview-first safety | P6-05, P6-06 | zero-write fs asserts + legacy opt-out tests |
| C7 v1.6 frozen additive | P6-06, P6-21/22/27/30 (matrix amendments), P6-18 | automated schema diff + changelog scan (incl. Wave I support-matrix rows) |
| COM on Windows host | P6-11..15 + **P6-33 host re-verification (C13, tag-blocking)** | host conversion goldens + doctor rows + packaging rehearsal, all host-cited |
| Formula + revisions + interop | P6-07..10 | ADR + parity lint + lossless suites + interop harness |
| Fuzz round 5 + SEC-06 | P6-17 (+ Wave I surfaces P6-21/25/31/32) | nightly status + review doc |
| Launch rehearsal w/o launch | P6-16 | hygiene files + recorded dry-runs; absence asserted |
| **Feature completeness (Wave I — §7 item 10)** | **P6-20..34** | six-format inspect/diff fixture suites; apply write/journal/revert suite + zero-write refusal asserts; revisions odt/ods/odp suites; spec↔dispatch parity test; region/review pixel goldens; `of:=` parity goldens; parser regression + render/annotate binary e2e; P6-33 host evidence in status JSON |
| Hygiene + headless gate + tag rule | P6-00, P6-18, P6-19 | gate runs headless itself; post-report re-run; tag==committed-tree check |

## 5. Sizing & context notes (the 200k discipline)

- **2s steps** (P6-03, 08, 09, 12, 13, 17, **25, 26, 27, 28**): split = build-core first, wire/test
  second; handoff names files landed + the exact next assertion. P6-08 part 1 = fixtures +
  evaluator core; P6-09 part 1 = ODS, part 2 = ODP; P6-12 part 1 = worker harness + render,
  part 2 = accept path; **P6-25 part 1 = executor delegation + docx/xlsx write + journal, part 2 =
  pptx/odt write + refusal codes + C7 note; P6-26 part 1 = Insert/Track* in patch.rs, part 2 =
  docx tracked-change landing + goldens; P6-27 part 1 = list+refusals, part 2 = accept/reject +
  review snapshot; P6-28 part 1 = provider + region, part 2 = cache + provenance honesty.**
- Wave C is a **strict chain** (P6-01 → … → P6-06): the shared files (`mcp/spec.rs`,
  `docs/contracts/C7.md`, `cli/src/main.rs`) are edited by exactly one agent at a time. Waves
  F/R/W/G run **parallel** to Wave C and to each other once P6-00 merges (disjoint crates:
  `adapters-ooxml` formula vs `adapters-odf` revisions vs `engines` COM vs docs rehearsal).
- **Wave I runs parallel after Wave 0** except its own chains: read-side P6-20 → {21, 22, 23, 24},
  mutation-side P6-25 → P6-26, image-side P6-28 → P6-29; P6-27/30/31/32/33/34 independent
  (C7.md and `cli/src/main.rs` touchpoints sequenced by the §1 hotspot chain). Wave X starts only
  when the Wave I gate passes (C10): P6-17/18/19 re-gated on top of Wave I.
- Corpus never loaded into context: fixture matrices run **in code**; steps cite counts.
- **ADR-0011's crate set is the only new-dependency surface** — anything outside it needs an ADR
  (concept §8 rule unchanged); pure-Rust constraint (D2/D25) applies to the evaluator too;
  **Wave I adds zero new dependencies** (converters/executors/providers are all in-workspace;
  the only dependency discussion — real CMS — is deferred with ADR-0010 reserved, call C12).
- Honest absence stays first-class (Office/qpdf/veraPDF/TSA); a step is **not** verified by
  skipping where CI or the Windows host can run the real thing — record which leg ran where.
- COM steps record PowerShell + `cargo test` exit codes in the status JSON evidence field, same
  shape as headless steps; "ran on host X, build Y" is part of every Wave W evidence string —
  **P6-33 re-records P6-11..15 to this standard before the tag (C13)**.
- M5 carry-over watch-items: counts measured per step (`spec.rs` assert + `enum Command` count);
  tag moves only after commit **and** post-report gate re-run — P6-19 encodes the order.
