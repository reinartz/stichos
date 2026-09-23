# stichos — Phase 6 concept: context budget + M6 intake (M6)

**Status:** planning artifact · derived from [`project_concept.md`](../project_concept.md) §14 + §16
(decisions D1–D27) + [`m5-acceptance.md`](m5-acceptance.md) (§4 Phase-6 intake, §9 post-acceptance
correction) + owner scope calls 2026-09-20 (§10: C6–C9) + owner scope calls 2026-09-22 (§11:
C10–C13, Wave I feature-completeness extension) · **proposes no new decision rows**
(Wave 0 preserves the §3.2 DAG; COM/formula/revisions/launch all execute existing decisions;
Wave I executes existing promises — D7, C11 exposure, the advertised 23-tool surface).
**Companion:** [`phase6-plan.md`](phase6-plan.md) (steps) · [`phase6-status.json`](phase6-status.json) (progress).
**Last updated:** 2026-09-22 (Wave I extension added as §11; §7 gains gate item 10)

---

## 1. What Phase 6 is

M5 closed the PDF-native loop and the trust layer: annotations round-trip through the incremental
writer, review sessions run over PDFs, signed documents refuse silent mutation, PAdES-B-B signs
afterwards. Two things remain before the tool can be called complete for agentic use:

1. **Responses that fit the reasoning turn.** Today a full `inspect_document` of a 50-page doc can
   exceed 50k tokens and every `render_document` ships inline base64. The context budget (contract
   C14, decision D27) flips the defaults — summary-first inspect, URL-first images, explicit caps,
   `dry_run`-true previews, `__budget` on every response — shipped as C7 additive **v1.6**.
2. **The deferred M6 intake, now unblocked.** The Windows + M365 host exists (owner call C7), so
   COM-2..6 implementation and Word-interop verification schedule into M6 instead of staying
   design-only. xlsx formula evaluation gets its ADR + minimal core subset (C3/C9). ODS/ODP revision
   write completes the C11 story odt started. Launch stays rehearsal-only (D26, C8).

And first, a hygiene item that gates everything else:

0. **Wave 0 — DAG-inversion closeout.** `docs/m5-acceptance.md` §9 records that
   `tools/check-deps.sh` fails at the `m5` tree: P5-09's text-layer route made `mcp` depend on
   `adapters-pdf`, violating `project_plan.md` §3.2 (`mcp`: `gp-core` only). No M6 work merges
   until the edge is removed by dependency inversion and `check-deps.sh` exits 0 at a committed
   tree (owner call C6: fix the code, not the plan).

### Starting state — measured 2026-09-20 (not assumed)

- M5 **shipped**: tag `m5` = `484496a` = `main` HEAD, present on origin (`git ls-remote`
  `refs/tags/m5`); 19/19 steps verified; `cargo test --workspace --lib` **1 807 passed, 0 failed**
  (211+150+332+128+239+360+294+93; adapters-pdf 2 ignored); clippy/fmt clean.
- **Surface truth is measured**: MCP tool registry = **23 tools**
  (`spec.rs` asserts `TOOLS.len() == 23`), CLI = **21 verbs** (`enum Command`, incl. `Schema`);
  C7 **v1.5.0** additive-only since v1.0.0.
- **The one red item**: `bash tools/check-deps.sh` exits 1 at the `m5` tree
  (`mcp -> adapters-pdf`); hence `phase5_gate.sh --ci` gate 8 does not reproduce green.
  Everything else in the M5 transcript re-verifies.
- Host facts (this box): pandoc 3.7.0.2 ✅, soffice 26.2.6.1 ✅, pdftoppm ✅, xmllint ✅,
  node v22 ✅; typst ✗, qpdf ✗, veraPDF ✗ (CI installs; honest-absent paths stay first-class).
  Corpus: 169 manifests (157 M0–M4 + 12 P5-02 PDF-annot/signed/PDF-A); `pr-subset.txt` 27 entries.
- **Windows host with M365 is now available** (C7) — COM-2..6 and Word-interop verification have a
  machine to run on. Nothing in Wave 0 / CTX work needs it; COM steps do.
- **M5 lessons baked into this phase's rules:** (a) a milestone tag moves only to a **committed**
  gate-green tree — commit → gate → tag → push, in that order; (b) every verify command produces
  its verdict from **exit codes**, headless (no `/dev/tty`); (c) counts in docs must be measured
  from code, never carried forward; (d) new — **the gate transcript in the acceptance report must
  be re-runnable at the tagged tree** (§9 exists because it wasn't; P6-00/P6-X assert it).

## 2. The ship

- **Wave 0: `mcp` is protocol-only again.** The `/review/<token>/text/<N>.json` route keeps its
  behavior byte-for-byte; the text-span extraction moves behind a `gp-core` trait
  (`TextExtractor`/`PdfTextSource`), the `adapters-pdf` impl lives in `adapters-pdf`, concrete
  wiring moves to `cli` (the de-facto wiring point). `crates/mcp/Cargo.toml` drops the
  `adapters-pdf` edge; `tools/check-deps.sh` exits 0. No schema change, no behavior change —
  proven by the unchanged P5-09/P5-10 e2e suites.
- **Context budget at C7 v1.6 (track CTX, `project_plan.md` §5):** `inspect_document`
  `detail=summary|elements|all` (default **summary**); `output=urls|base64|both` (default **urls**)
  + `max_pages` on `render_document` / `inspect_region` / `annotate_pages`; `diff_documents`
  `max_ops` default **20** + `summary` counters; `journal` `max_entries` default **10**;
  `dry_run` default **true** on `apply_patch` / `edit_*` with a ≤20-op preview;
  `__budget: {tokens_est, elements_count, image_count}` on **every** response; temp-file image
  store (`$XDG_RUNTIME_DIR/stichos/renders/`, 1 h TTL + startup sweep, `expiresAt`).
  CLI mirrors every flag (`--detail`, `--output`, `--max-pages`, `--max-ops`, `--dry-run/--apply`).
- **`stichos` on the Windows host with Office first.** COM-2 probing (ProgIDs, `CurVer`,
  ClickToRun, `WINWORD.EXE`; probe-never-assume), COM-3 Word worker, COM-4 Excel + PowerPoint
  workers, COM-5 `CompareDocuments` authoritative diff, COM-6 logon-agent packaging — all behind
  the existing `ms-office` feature flag, all with LO fallback honest (D9).
- **Word-interop verification of accept/reject:** the M4 OOXML revision round-trips re-verified
  through real Word on the Windows host (open → accept → save → our reader: lossless).
- **xlsx formula evaluation, minimal core subset:** ADR (engine choice: own evaluator vs
  library; cached-values-always semantics) + arithmetic/aggregate/logic subset with
  cached-vs-computed parity lint; everything else → fidelity budget, never silent wrong values.
- **ODS/ODP revision write:** the odt-first C11 pattern extended to spreadsheet/presentation
  revisions (accept/reject round-trip lossless on a corpus subset).
- **Launch rehearsal only (D26, C8):** hygiene stays green, publish dry-runs re-recorded, no
  publish, no flip. The flip remains an explicit owner action.

## 3. Product surface (C7 v1.6 additive; no new tools)

No new tool or verb ships in M6 — the surface change is **parameters + defaults + envelope**:

| Tool | New / changed params | Default flip (versioned at v1.6) |
|---|---|---|
| `inspect_document` | `detail`, `max_elements`, cursor | `detail="summary"`, `max_elements=100` |
| `render_document` | `output`, `max_pages` | `output="urls"`, `max_pages=3` |
| `inspect_region` | `output` | `output="urls"` |
| `diff_documents` | `summary`, `hasMore`, `totalOperations` | `max_ops=20` (was unlimited) |
| `apply_patch` | preview shape, `includeFullDiff` | `dry_run=true` (was false) |
| `edit_text` / `edit_cell` / `edit_structure` | preview shape | `dry_run=true` (was false) |
| `annotate_pages` | `output`, `max_pages` | `output="urls"`, `max_pages=5` |
| `journal` | `max_entries` | `max_entries=10` |
| **every tool** | `__budget` envelope | present unconditionally |

Baseline is the **measured** 23 tools / 21 verbs; M6 target: same counts, C7 **v1.6.0**.
The flips are **behavioral but schema-additive** (all old params still accepted; explicit
`output="base64"`, `dry_run=false`, `max_ops=0`/large restore old behavior) — CHANGELOG carries
an explicit **default-flip** section and migration notes (C14 §8).

## 4. Capability matrix (M6)

| Path | Input → output | Engine/deps | Explicitly not |
|---|---|---|---|
| DAG inversion (Wave 0) | same text-layer JSON via trait | own (`gp-core` trait, `cli` wiring) | any new `mcp` edge; behavior change |
| Summary inspect | 50-page docx → ~150-token summary + IDs | own | full fidelity in default mode (opt-in `all`) |
| URL-first render | pages → temp PNGs + `expiresAt` | own + raster ladder | CDN/hosted URLs; inline base64 by default |
| Capped diff/journal | doc pair → ≤20 ops + summary | own | unbounded default output |
| Preview-first mutation | patch → ≤20-op preview + token estimate | own | applying without explicit opt-out of preview |
| COM Word worker | docx → Word render/accept path | MS 365 on Windows host | service/non-interactive contexts (D10) |
| COM Excel/PPT workers | xlsx/pptx → Office render path | MS 365 on Windows host | same constraint |
| `CompareDocuments` diff | doc pair → authoritative track-changes | Word COM | non-Windows authoritative diff (G3 open) |
| Formula eval subset | xlsx → computed values + parity lint | own evaluator (ADR) | full function coverage; silent evaluation gaps |
| ODS/ODP revisions | ods/odp → accept/reject round-trip | own | change-tracking UI (still `review`) |
| Word-interop verify | M4 revision fixtures → Word → back | Windows host Word | LO-vs-Word pagination sameness (provenance, D6) |
| Launch rehearsal | repo state → dry-runs recorded | cargo/winget/brew tooling | the publish itself (D26) |

## 5. Behavior promises (each test-bound)

- **Wave-0 inversion gate**: `tools/check-deps.sh` exits 0 at a committed tree **before any CTX
  step merges**; the P5-09/P5-10 text-layer e2e suites pass unchanged (behavior byte-identical);
  `mcp/Cargo.toml` contains no `adapters-*`/`engines` edge.
- **One-turn defaults**: `inspect_document` default response < 500 tokens on the corpus
  50-page fixture; `render_document`/`inspect_region`/`annotate_pages` default responses contain
  **zero base64** (gate greps the JSON); `diff_documents` default returns ≤ 20 ops with
  `hasMore`/`totalOperations`; `journal` default returns ≤ 10 entries.
- **`__budget` ubiquity**: 100 % of tool responses carry `__budget` + `diagnostics` +
  `rendered_by` (INV-2 schema gate extended; a single negative test per tool).
- **Preview-first safety**: omitting `dry_run` on `apply_patch`/`edit_*` returns a preview and
  writes nothing (fs-assert zero writes); explicit `dry_run=false` restores apply.
- **Additive-only schema**: automated C7 v1.5→v1.6 diff shows additions only; legacy
  explicit-parameter tests prove old paths (`output="base64"`, `dry_run=false`, large `max_ops`).
- **COM honesty**: on the Windows host, `doctor` capability rows come from real fixture
  conversions (D8); without Office present the worker reports honest-absent and LO order applies
  (D9). One document per instance, hard timeout, kill on wedge (D10 constraints in code).
- **Formula parity**: cached-vs-computed lint on every evaluated cell; unsupported functions →
  budget entry + cached value surfaced, never an invented value; GP-F0xx refusals for the
  refuse-list (array/matrix black-box, external refs, macros).
- **Revision lossless tiers** (C11, unchanged): ODS/ODP accept/reject round-trip lossless on the
  corpus subset; Word-interop: accept in real Word → our reader sees the accepted state exactly.
- **Inherited unchanged**: INV-1/2/3/4, C2 anchoring, C8 payload rules, honest absence,
  signature refusal-by-default (C13), text-layer XSS containment (P5-10).

## 6. Explicitly out of scope (where it lands)

| Deferred | Lands |
|---|---|
| Actual public launch (D18 flip, registry publishes) | owner-timed, post-M6 (D26, C8); M6 ships rehearsal only |
| PAdES-T/-LT/-LTV, trusted certs, OCSP/CRL | post-M6 once B-B is proven in the field |
| PDF redaction, content-stream surgery | post-M6 (needs its own ADR) |
| Full formula-function coverage | post-M6, driven by the parity-lint gap list M6 produces |
| OQ2 Word tier for non-Windows users (G3) | stays open; M6 proves the Windows path, which informs it |
| G5 pdfium | stays gated (no new evidence forces it) |
| Multi-user / hosted review | not planned (local single-reviewer by design) |
| New tools/verbs beyond the 23/21 surface | not in M6 (C7 v1.6 is params + defaults + envelope) |

## 7. M6 exit gate (measurable; asserted by `tools/phase6_gate.sh`, step P6-X)

1. **Wave-0 closeout**: `tools/check-deps.sh` exits 0; `mcp` depends on `gp-core` only;
   text-layer e2e suites green with zero behavior delta.
2. **One-turn defaults**: summary < 500 tokens; zero base64 in URL-mode responses;
   `max_ops`/`max_entries`/`max_pages` caps asserted; `__budget`+`diagnostics`+`rendered_by`
   on 100 % of responses (schema-gate test per tool).
3. **Preview-first safety**: default-omit `dry_run` writes nothing; explicit opt-outs restore
   legacy behavior (legacy-parameter regression tests).
4. **C7 v1.6 frozen additive** (23 tools / 21 verbs; automated schema diff; CHANGELOG
   default-flip section + migration notes).
5. **COM on the Windows host**: probe matrix from real conversions; Word/Excel/PPT workers run
   one-doc-per-instance with timeouts; `CompareDocuments` diff e2e; logon-agent packaging
   rehearsed (MSI component + task XML + docs).
6. **Formula + revisions + interop**: ADR accepted; subset evaluates with parity lint;
   ODS/ODP accept/reject lossless on corpus subset; Word-interop accept round-trip lossless.
7. **Fuzz round 5** (CTX parsers/caps, formula evaluator, COM boundary strings) clean + SEC-06
   Phase-6 review + threat-model update.
8. **Launch rehearsal without launch**: hygiene pack green; publish dry-runs exit 0 and recorded;
   absence of any publish asserted (no version tags pushed, registries untouched).
9. **Hygiene + gate**: `phase6_gate.sh` green **headless** + CI; **tag `m6` moves only to a
   committed, gate-green tree**; acceptance report committed before the tag; the gate transcript
   is re-runnable at the tagged tree (M5 §9 lesson — P6-X asserts it by re-running the gate
   after the report commit, before the tag).
10. **Feature completeness (Wave I — §11, added 2026-09-22)**: six-format `inspect_document`/
    `diff_documents` (docx/odt + xlsx/ods + pptx/odp, PDF where defined); `apply`/`edit --apply`
    write real bytes with journal + revert and refuse everything else explicitly (never silent
    success); `revisions` list/accept/reject exposed for odt/ods/odp; `sign_document` dispatched
    (spec↔dispatch parity test); `inspect_region` and the review page route serve real pixels
    with honest `rendered_by`; ODS `of:=` formulas evaluated under C15 parity; the adapters-pdf
    `parser.rs:498` panic is fixed (render/annotate binary e2e unblocked); P6-33 Wave W host
    evidence re-recorded. Asserted by P6-18; the `m6` tag waits on this item (owner call C10).

## 8. Risks

| Risk | Mitigation |
|---|---|
| C14 default flips break existing automation silently (previews instead of applies, URLs instead of base64) | flips ship only at C7 **v1.6** with explicit opt-outs; CHANGELOG default-flip section; legacy explicit-parameter regression tests; gate asserts `__budget` presence so clients can detect the new shape |
| Wave-0 inversion changes text-layer behavior by accident | behavior-parity tests run the old and new paths against the same fixtures and byte-compare the JSON; P5-09/P5-10 suites unchanged and green |
| COM worker scope bleed (Word automation constraints, Store editions, ARM64 quirks) | probe-never-assume (COM-2) first; LO fallback always honest (D9); logon-agent model isolated from core (D10); host-matrix recorded in `doctor` |
| Formula evaluator invents values for unsupported functions | cached-value-always semantics: unsupported → budget entry + cached value, never computed; parity lint on every evaluated cell; GP-F0xx refuse-list |
| Temp-file URLs leak across sessions or fill disks | 1 h TTL + startup sweep + per-response `expiresAt`; cleanup tests with aged files; no temp path ever trusted as input without validation |
| Token-estimator drift (estimator says 150, reality says 900) | C14 §7.4 contract tests bound summary size in bytes as well as tokens; estimator reuses REN-02, calibrated once |
| Scope bleed from post-M6 items (launch flip "while we're at it") | §6 is hard scope; the flip is a D26 owner action with its own checklist, not a gate item |

## 9. Decisions/contracts this phase stands on

**Unchanged:** D1–D27 — no new decision rows are proposed. Wave 0 *executes* the §3.2 DAG
(owner call C6 picks the code-fix branch, which needs no D-log edit); COM-2..6 execute D9/D10;
formula executes C3; launch restraint is D26/D18 as-is. If any step finds a contradiction, it
stops and flags the owner per the standing rule — executing agents still do not edit
`project_concept.md`/`project_plan.md`/this concept.

**New ADRs:** **ADR-0011** (xlsx formula evaluation: evaluator choice, supported subset,
cached-value semantics, GP-F0xx refuse-list, parity-lint design). Number reserved and ADR written
by P6-07 (`docs/adr/README.md` updated there).
**Contracts:** **C14 draft → freeze at M6** (context budget; ships as C7 additive **v1.6**) ·
**C15 new** (formula-evaluation model: supported function set + versioning, cached-vs-computed
parity rules, GP-F0xx refusals; draft → freeze at M6) · C11 extended (ODS/ODP revision write;
lossless tiers unchanged) · C7 additive **v1.6** · C9 extended (CTX fixtures: 50-page inspect
corpus doc, multi-page render set, large-diff pair, formula fixtures) · C6/C10/C12/C13 consumed frozen.
ADR-0007 consumed (sessions unchanged), ADR-0001/D19 consumed (no new PDF write path).

## 10. Owner scope calls 2026-09-20 (this session)

- **C6** — **M5 DAG deviation is fixed in code, not in the plan**: Phase 6 Wave 0 removes the
  `mcp -> adapters-pdf` edge by dependency inversion (`gp-core` trait + `cli` wiring); no M6
  work merges before `check-deps.sh` is green; `docs/m5-acceptance.md` §9 records the correction.
- **C7** — **Windows + M365 host is available now**: COM-2..6 implementation plus Word-interop
  verification of accept/reject schedule into M6 (the M4 design ADR + compile-only skeleton are
  the starting point, not a re-design).
- **C8** — **Launch stays prep-only and owner-timed** (D26 unchanged): M6 ships a launch
  rehearsal (hygiene green + dry-runs recorded + absence asserted), never the flip.
- **C9** — **xlsx formula evaluation is ADR + minimal core subset**: arithmetic/aggregate/logic
  with cached-vs-computed parity lint; unsupported functions surface cached values via the
  fidelity budget; fuller coverage is post-M6, driven by the gap list M6 produces.

## 11. Scope extension 2026-09-22 (session 2): feature completeness (Wave I)

**Trigger (owner):** "are there functions not yet, or not yet fully, implemented?" A surface census
was run against the **code, not the docs**; every finding below is measured (file:line at the
2026-09-22 working tree) and classified: **defect** (an advertised behavior does not happen),
**unwired capability** (an adapter does the work, no verb/tool routes it), **documented stub**
(C13/D26 honesty — stays), or **doc-rot** (stale claim).

### 11.1 Measured findings

| # | Gap | Evidence | Class |
|---|---|---|---|
| 1 | `inspect_document` rejects xlsx/pptx/ods/odp — readers exist (`XlsxReader`→`Spreadsheet`, `PptxReader`→`Presentation`, ODS/ODP readers in `adapters-odf`) but no converter into the tagged-tree IR; `Spreadsheet`/`Presentation` have zero references in `cli`/`mcp` | `cli/src/inspect.rs:53-83` | unwired capability |
| 2 | `diff_documents` rejects the same four formats | `cli/src/diff_tool.rs:222-231` | unwired capability |
| 3 | `apply` / `edit text\|cell\|structure --apply` **write nothing**: the base executor returns a stub (`GP-I-002`) for every writable format and the apply tool sniffs that message to skip the write; the real executor paths (`OoxmlExecutor::execute_docx/xlsx/pptx`, ODF patch writer) are orphaned | `gp-core/src/executor.rs:720-738`, `cli/src/apply_tool.rs:87-95`, `cli/src/sugar_edits.rs` (all three edits delegate to `apply_patch`) | defect (and D7 — "docx edits land as tracked revisions" — never materializes through the patch pipeline) |
| 4 | `PatchOp::Insert`/`TrackInsert`/`TrackDelete` are "not yet implemented" — `edit_structure insert` and tracked edit ops are no-ops | `gp-core/src/patch.rs:97-129` | defect |
| 5 | `revisions` list/accept/reject is docx-only; **list returns a silent empty success** on other formats — the P6-09 ODS/ODP revision reader/writer exists in `adapters-odf` but no verb/tool routes it | `cli/src/revisions_tool.rs:216-220,246,268` | unwired capability + silent-empty defect |
| 6 | MCP `sign_document` advertised since v1.5.0 (1 of the 23; spec-asserted) but never dispatched — runtime `unknown tool: sign_document` | `mcp/src/spec.rs:897` vs the `cli` dispatch fallthrough at `cli/src/main.rs:2032` | defect |
| 7 | `region`/`inspect_region` crops a **synthetic gray page** (`read_master_page` P1 stub) while stamping `rendered_by: "libreoffice + poppler"` — fake pixels, false provenance | `cli/src/region.rs:126-135,110` | defect |
| 8 | review page-image route returns HTTP 501 ("engine cache not wired"); `MasterCache` in `engines` is dead code | `mcp/src/http.rs:680-686`, `engines/src/cache.rs` | unwired capability |
| 9 | `annotate_pages` `marked` artifact is a byte-mirror of `clean` (the legend is honest; the artifact is not the promised overlay) | `cli/src/annots.rs:341-362` | partial |
| 10 | ODS formulas are parsed (`of:=` stored raw) but never evaluated — C15 parity reaches xlsx only | `adapters-odf/src/ods.rs:247,311-332` | unwired capability |
| 11 | adapters-pdf `parser.rs:498` subtract-overflow panics on LO-produced PDFs — the recorded blocker of **binary** render/annotate e2e since P6-03 | `phase6-status.json` notes (reproduced on 3 corpus fixtures, present at the `m5` tree) | defect |
| 12 | Wave W evidence gap: P6-11..15 cite Linux stub-mode `cargo test --lib ms_office` (37 tests) with no host run recorded, while the status legend and plan §5 require host-cited Wave W evidence (P6-10 has it: "Windows + Word 16.0") | `phase6-status.json` evidence fields | evidence gap |
| 13 | doc-rot: `CODEBASE.md`/`CHANGELOG.md`/`AGENTS.md`/`support-matrix.md` still call the ms-office tier "compile-only skeleton/deferred"; `engines/Cargo.toml` comment says "deferred to Phase 5"; `formula/mod.rs` carries a stale Part-2 handoff checklist; `cmd_update` hard-codes a workspace fixture path | `engines/Cargo.toml:14-17`, `formula/mod.rs:19-26`, `cli/src/main.rs:2193-2195` | doc-rot + small defect |

### 11.2 Wave I ships (owner call C10: inside Phase 6, before Wave X)

- **Six-format read**: `inspect_document` for xlsx/pptx/ods/odp (summary/elements/all, caps and
  cursors unchanged; cell records carry `formula` + value + parity flag — the C15 work finally
  becomes visible through the surface) and `diff_documents` gets cell-level and slide-level diff
  inside the CTX-04 caps.
- **Mutation honesty**: real write-back for docx/xlsx/pptx/odt through the existing adapter
  executors, journal + `revert` round-trip on the apply path; ods/odp/pdf refuse with explicit
  codes (never silent success); `Insert`/`TrackInsert`/`TrackDelete` implemented so D7's tracked
  docx edits finally land via the patch pipeline.
- **Revisions exposure**: the `revisions` verb/tool covers odt/ods/odp (the P6-09 capability,
  routed); silent-empty lists become explicit refusals; review snapshot mode labels the real format.
- **Real pixels**: a master-page provider (engines cache / render pipeline) behind
  `inspect_region` and the review page route; `rendered_by` names the engine that actually ran;
  the `marked` annotate artifact gets a real numbered-box overlay **or** is honestly dropped.
- **Dispatch honesty**: `sign_document` wired to the existing signing path; a spec↔dispatch parity
  test proves all 23 tools resolve.
- **Formula parity to ODS**: `of:=` translation into the C15 subset evaluator with the
  cached-vs-computed lint; refuse semantics unchanged (cached surfaced, never invented).
- **Parser fix**: the `:498` overflow fixed; the render/annotate binary e2e deferred by P6-03/P6-06
  lands with it.
- **Wave W evidence re-recorded (C13)** on the Windows + M365 host before the tag.
- **Hardening + doc corrections**: `update` verb path fix, stale-doc sweep, ADR-0011/C15 freeze
  checklist for P6-19, Phase-7 intake drafted.

### 11.3 Surface and versioning (owner call C11)

Still **23 tools / 21 verbs** — Wave I adds no tool, no verb, and no new parameters (formats and
behavior only). C7 **v1.6.0 is amended, not bumped**: nothing has ever shipped at v1.6.0 (M6
untagged, publishing rehearsal-only per D26), so amending the pre-tag `C7.md` matrix + CHANGELOG
is the honest record; a v1.7.0 row for unshipped content would imply a release that never happened.
Legacy explicit-parameter behavior is untouched; every Wave I response stays envelope-compliant
(`__budget` + `diagnostics` + `rendered_by`).

### 11.4 Explicitly out of Wave I (lands Phase 7)

- Real PKCS#7/CMS signing + verification (owner call C12): the `signature.rs` stub stays as
  documented (C13 honesty); the **ADR-0010 number is reserved** for the crypto-dependency
  decision (D2/D25 — owner-gated new dependency surface).
- `create` to xlsx/pptx/ods/odp (content-driven creation; `create_minimal_xlsx/pptx` stay internal).
- Full formula-function coverage — the M6 parity-lint gap list drives Phase 7, unchanged.
- `lint` encode/ssim refresh (the "deferred to Phase 2" notes are re-triaged to Phase 7, not kept silently).
- Review-action write-back (stays audit-log), OQ2 non-Windows Word tier (G3), G5 pdfium,
  multi-user review, PAdES-T/-LT/-LTV, redaction — all unchanged from §6.

### 11.5 Behavior promises (each test-bound)

- **Read parity**: `inspect_document` default summary < 500 tokens on corpus xlsx + pptx + ods +
  odp fixtures; docx/odt/pdf outputs byte-locked unchanged; formula cells expose `formula` +
  `value` (+ parity flag); unknown extensions stay explicit errors.
- **Mutation truth**: `--apply` writes a byte-valid file re-readable by its adapter and renderable
  by LO; a journal entry + `revert` restores the original bytes; refused formats write nothing
  (fs-assert zero writes); after P6-26, a `direct=false` docx edit is visible through
  `revisions list` as a tracked change (D7).
- **Exposure honesty**: `revisions list` on an unsupported format returns a refusal diagnostic,
  never an empty success; `sign_document` dispatches (spec==dispatch parity test in CI);
  `region`/review pages are real pixels matching the rendered page with truthful `rendered_by`;
  a `marked` annotate artifact is either a real overlay or absent-with-diagnostic — never a mirror.
- **Formula parity (extended)**: `of:=` subset golden-evaluates against cached values; unsupported
  → cached value + budget entry, never invented; GP-F0xx refusals unchanged.
- **No panics**: the `:498` regression corpus parses clean; render/annotate binary e2e green
  (closes the P6-03/P6-06 caveat).
- **Host-cited Wave W**: every Wave W evidence string cites host + build before the `m6` tag (C13).

### 11.6 Contracts/decisions touched

C7 **v1.6.0 amended** (support-matrix rows + dispatch note; still additive over v1.5) ·
C11 **consumed** (ODS/ODP revision capability routed to verbs; lossless tiers unchanged) ·
C15 scope extended (ODS `of:=` under the same subset + parity rules; freeze at M6) ·
C9 fixture extension (xlsx/pptx/ods/odp inspect fixtures, diff pairs, `of:=` fixtures, the
parser-panic corpus, master-page crops) · C3 new GP-I-xxx refusal codes (explicit unsupported-write
paths) · **no new ADRs** (ADR-0010 reserved, unallocated, for the Phase-7 CMS decision) ·
D1–D27 unchanged — P6-26 finally *executes* D7.

### 11.7 Owner scope calls 2026-09-22 (session 2)

- **C10** — **Wave I runs inside Phase 6**: scheduled after Wave G, before Wave X; the `m6` tag
  waits on the Wave I gate (concept §7 item 10).
- **C11** — **C7 v1.6.0 is amended, not bumped**: nothing shipped at v1.6.0; the CHANGELOG records
  Wave I under v1.6.0 before the tag.
- **C12** — **real CMS signing defers to Phase 7** (ADR-0010 reserved). Wave I only dispatches the
  already-advertised `sign_document` onto the existing (stub-backed, documented) signing path —
  C13 honesty intact, no new dependency.
- **C13** — **Wave W host re-verification is tag-blocking** (P6-33): stub-mode Linux evidence is
  not host proof; concept §5 COM honesty and the plan §5 evidence rule apply.
