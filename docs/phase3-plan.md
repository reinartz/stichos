# stichos — Phase 3 plan: agent-sized steps (M3)

**Status:** active plan · derived from [`phase3-concept.md`](phase3-concept.md) (scope, gate) and
`project_plan.md` §5 tracks (AUT, ODF, PDF, OOX, ENG, API, LIN, REN, SEC, DIS, OPS) ·
**33 steps, 8 waves (0, A–G), 8 gates**.
**Canonical progress state:** [`phase3-status.json`](phase3-status.json) — this file defines the steps;
the JSON tracks them. Do not maintain status in two places.
**Last updated:** 2026-09-15

---

## 1. How this plan is executed (agent rules)

- **One agent, one step, one session.** Budget ~200k tokens. Load only: your step row + wave gate here,
  the named contracts in `docs/contracts/` + ADRs, the crate(s) you touch, the named corpus subset.
  No repo-wide archaeology: `CODEBASE.md` maps the tree; fetch the specific ECMA-376/OASIS/PDF clause
  you need, not the spec.
- **Sizing rule:** every row is scoped to one 200k session (≈ ≤1 200 new LOC + tests). **2s** rows =
  two sequential sessions with a written handoff in the step's status entry (`part1 done: …; part2: …`).
  If a step grows past scope anyway, stop, record `blocked: split-needed` + evidence; do not sprawl.
- Write only inside the step's deliverable paths + its row in `phase3-status.json`.
- `project_concept.md` / `project_plan.md` / this file are **not edited by executing agents**;
  a contradiction stops the step and flags the owner. Measured vs assumed, always.
- Definition of done: `project_plan.md` §8 + the step's **Verify** column, run and recorded with
  command + result + date.
- Status: `pending → active → verified | blocked(reason)`.
- **Host facts (measured 2026-09-15):** pandoc 3.7.0.2, chromium-browser, xmllint, soffice 26.2.6.1,
  pdftoppm present; **typst, qpdf, verapdf absent** — code must probe honestly; CI installs them.
  Corpus ODF = 69 fixtures (odt 26 / ods 21 / odp 22).

## 2. Waves and gates

| Wave | Theme | Gate (command + pass condition) |
|---|---|---|
| **0** | Phase-2 debt closeout | `phase2_gate.sh` exit 0 on **committed** tree; `git rev-parse m2 == HEAD`; branch pushed; clippy/fmt clean |
| **A** | authoring engines (chromium, typst) | both probed + registered (C5); doctor lists them honestly; `cargo deny` bans `typst` crate; engine trait suites green |
| **B** | ODF write path | **ODF zero-patch byte-identical ×69**; odt/ods/odp edit chains re-read; census+budget; RelaxNG gate; ODF refusals retired (PDF refusal still present) |
| **C** | PDF incremental writer | **append-only prefix invariant** on measured PDFs; annot/form e2e; signed-input refusal; PDF refusals retired |
| **D** | native creation (IR → OOXML) | created docx/xlsx/pptx pass lint + schema gate + **IR round-trip equality**; template style/theme parts byte-identical |
| **E** | creation surface (C7 v1.2 tools) | e2e CLI+MCP on all 4 paths; create-as-revisions read-back; dry_run writes nothing |
| **F** | sandbox + a11y + annotator | Rhai escape suite green (zero side effects); veraPDF runner honest-absent + real in CI; annotator legend tests |
| **G** | gates, fuzz, measurement, distribution, ship | `tools/phase3_gate.sh` green host + CI (== M3 exit, concept §7); fuzz ≥24 h clean; corpus measurement table published; release CI ×6 + selfupdate fixture proof; tag `m3` |

Waves ordered; inside a wave run in Deps order; parallel only where deps allow and files don't collide.
Wave B ∥ C ∥ D ∥ (P3-25/26/29) are independent of each other — parallel lanes.

## 3. Steps

`Implements` cites `project_plan.md` §5; `Verify` = evidence the step must produce.
**Size:** 1 = one 200k session, 2s = two with handoff.

### Wave 0 — Phase-2 debt closeout (S1; fix the ground before building)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-01 | Fix the 5 clippy errors without behavior change: unused `super::*` (`cli`), 2× `map_or` simplifiable + 2× if-then-else-bool at `crates/cli/src/diff_tool.rs:259,268`; re-check all targets incl. test/bins; then `cargo fmt --all` (10 diffs / 6 files) | M2 fix | — | 1 | `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --all --check` clean; `cargo test --workspace` ≥1280 pass, 0 fail; no test expectation weakened |
| P3-02 | Commit Waves C–F (uncommitted since `3f1cb2a`) as logical commits (C: executor/journal/dry-run/QA/sig; D: policy/taint/C10; E: surface tools; F: schema-gate/fuzz/perf/gate/docs); delete misplaced `m2` tag, recreate at the gate-green tip; push all to `origin/main` | M2 ship | 01 | 1 | `git status` clean; `bash tools/phase2_gate.sh` exit 0; `git rev-parse m2 == git rev-parse HEAD`; `git status -sb` shows in sync with origin |
| P3-03 | Honesty trail: append closeout addendum to `docs/m2-acceptance.md` (the 2026-09-15 "clippy/fmt clean" claim did not reproduce; what was fixed, when); refresh `phase2-status.json` gate-F evidence with re-run date; confirm governance sync landed (OQ4/G4 in `project_concept.md` §18-4, S2/S3 in `project_plan.md` §6.3/§6.4 — written at plan creation 2026-09-15) | M2 fix | 02 | 1 | addendum exists; §18-4 marked RESOLVED; gate still green |

### Wave A — authoring engines: Chromium + Typst (shell-out)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-04 | ADR-0004: Typst engine = CLI shell-out (corrects ENG-11 "pure-Rust lib" wording; engines-never-linked preserved; D2 intact); register engine ids `chromium`, `typst` in capability matrix (C5); add `typst` crate to `deny.toml` bans (protects the invariant) | ENG-11, D3 | 03 | 1 | ADR-0004 accepted; `cargo deny check bans` green with the ban present; ids appear in capability matrix tests |
| P3-05 | Chromium runner (`engines/chromium.rs`): discovery order chromium-browser→chromium→google-chrome→google-chrome-stable; version parse; `--headless=new --print-to-pdf` + `--screenshot` + paper/margins/background flags; isolated `--user-data-dir` per call; hard timeout + kill; Engine trait + probe (capability = real fixture conversion) | ENG-10 | 04 | 2s | trait + e2e html→pdf on host (`/usr/bin/chromium-browser` measured present); probe reports version; timeout-kill test; `rendered_by` naming test |
| P3-06 | Typst runner (`engines/typst.rs`): discovery `typst`; `typst compile in.typ out.pdf`; version parse; timeout; probe with **honest absent path** (doctor: unavailable + install hint, not an error) | ENG-11 | 04 | 1 | trait tests w/ fake binary; absent-path golden; CI (installs pinned typst) runs the real-conversion test |
| P3-07 | Negotiation + doctor: authoring orders (html→pdf: chromium→LO; md→docx: pandoc→native-later); `renderer.preference` respected; `doctor --json` + human output extended with chromium/typst fixture-probe rows | ENG-04, ENG-05 | 05, 06 | 1 | doctor golden (host: chromium ok, typst absent-honest); negotiation unit matrix |

### Wave B — ODF write path (ODF-01w/04/05; retire the ODF refusal)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-08 | ODF surgical package writer (`adapters-odf`): reuse OOX-02 raw stream-copy core; ODF rules: `mimetype` first entry stored/uncompressed, manifest.xml entries + hashes updated on part add/remove; encryption detection (manifest/`xml:encrypt`) → refuse C3 | ODF-01 | 03 | 2s | **zero-patch round-trip byte-identical ×69 ODF fixtures** (sha256 == input); per-quirk synthetic fixtures (mimetype order/compression, manifest); encryption refusal asserted |
| P3-09 | content.xml/styles.xml offset-based patch primitives on the adapters-odf DOM-lite (Phase-1 spans): replace-text, insert-node, delete-node, set-attribute; right-to-left; span re-verify before write | ODF-04 | 08 | 2s | property tests over **all ODF corpus XML parts**: only the span's bytes change; stale-span rejects with C3, writes nothing |
| P3-10 | IR patch → odt/ods/odp routing (paras/runs/table cells/text frames; ods cell values; odp shape text) + executor wiring: **retire ODF refusals** from P2-18, refusal matrix updated (PDF still refuses until Wave C) | ODF-04, PAT-01 | 09 | 2s | odt/ods/odp edit chains re-read via Phase-1 reader show exactly the intended change; INV-1 unrelated-edit sample on ODF; refusal-matrix tests updated both directions |
| P3-11 | ODF census (text:p/table:table/draw:frame/text:list + revision counts) → fidelity budget; RelaxNG validation gate via `xmllint --relaxng` with bundled ODF 1.2 rng (license note in file header) | ODF-05, LIN-04 ext | 10 | 1 | census deltas == manifest expectations on ODF subset; rng gate pass fixtures + a corrupt-write block fixture |

### Wave C — PDF incremental writer (PDF-03; retire the PDF refusal)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-12 | Incremental writer core (`adapters-pdf::incremental`, lopdf + pdf-writer per D19): new/updated objects + classic-xref append + `startxref`; **append-only invariant: original bytes are a strict prefix of the output**; refuse encryption and complex xref-stream inputs with explicit C3 codes (concept §8) | PDF-03 | 03 | 2s | append-only property test on PDFs rendered from corpus fixtures (LO path); `qpdf --check` green (CI installs qpdf; host absent 2026-09-15 — honest skip + CI-required marker) |
| P3-13 | Mutation surface as increments: annotation add/update, form field value (AcroForm), metadata (XMP/DocInfo); **signed PDF → refuse write** (never silently invalidate; PAT-05 warn path stays for Office) | PDF-03, PDF-05/06 | 12 | 1 | annot/form e2e re-read + prefix invariant holds; synthetic-signed fixture refusal asserted |
| P3-14 | Executor wiring: **retire PDF refusals**; journal + revert for PDF edits; increment segment = byte-diff report; dry-run preview via raster ladder on proposed-state temp file | PAT-01, PAT-02 ext | 13 | 1 | e2e apply→render→revert; refusal matrix = all three families writable (exceptions: signed/encrypted) |

### Wave D — native creation (IR → OOXML, OOX-11)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-15 | Creation writer skeleton (`adapters-ooxml::create`): brand-new OPC package write (content types, rels, part tree) + valid WordprocessingML skeleton (document/styles/settings/fontTable/theme) from IR doc meta | OOX-11 | 03 | 2s | minimal created docx: schema gate pass + LO renders (golden) + Phase-1 reader re-read → IR round-trip equal |
| P3-16 | docx body coverage from IR: paragraphs/runs/run-props, numbering, tables, headers/footers, sections, media images (rels + content types), hyperlinks | OOX-11 | 15 | 2s | IR fixtures derived from 10 corpus docs → create → round-trip equality + LO render + lint no overflow; media byte-preserved in package |
| P3-17 | xlsx + pptx creation: sheets/cells (inline strings)/styles/number-formats; presentation/slide/layout placeholder binding | OOX-11 | 16 | 2s | same triple (schema gate + round-trip + LO render) on ≥5 IR fixtures each |
| P3-18 | Template-based native creation: reference docx as base package — style/theme/master parts copied **byte-for-byte**, content parts replaced; `template` metadata on IR (C1 minor additive bump + spec note) | AUT-04, C1 | 16 | 1 | template create → style parts hash-identical to template; body from IR; budget reports any template drop |

### Wave E — creation surface (AUT paths + C7 v1.2 tools)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-19 | pandoc creation path: md/html → docx/odt via `--reference-doc` (branded template); output ingested to store + QA loop envelope; KI-04 note respected (pandoc probe is write-direction only) | AUT-01 | 03 | 1 | md fixture set → docx + odt → lint + schema + IR round-trip; `rendered_by: "pandoc 3.7.x"` |
| P3-20 | Chromium creation path: html → pdf via P3-05 runner (paper/margins/background); page-count + lint on raster + payload | AUT-02 | 05 | 1 | html fixtures → pdf e2e on host chromium; lint manifest present |
| P3-21 | Typst creation path: `.typ` + offline template → pdf via P3-06 runner; envelope as above; honest failure message when binary absent | AUT-03 | 06 | 1 | typst fixture set in CI (binary pinned); host path = graceful refusal with hint |
| P3-22 | Create-as-revisions (D7 suggestion mode): target docx + md source → IR diff (P2-07) → patch set → OOX-10 revision writer: new text `w:ins`, removals `w:del`, author param | AUT-05 | 16, (P2-07/17) | 2s | md→existing fixture: own reader reads back revisions; LO renders revision markup (golden pair); `--direct` opt-out |
| P3-23 | Creation surface wiring: MCP `create_document` + CLI `create` verb (format/source/template/dry_run flags, full envelope); route to the 4 paths; schema validation on args (C3 codes on bad input) | API-10 | 19, 20, 21, 22 | 1 | e2e CLI+MCP on all 4 paths over `pr-subset`-derived set; dry_run zero writes (fs assert); bad args → C3, no write |

### Wave F — sandbox + a11y + annotator

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-24 | Rhai escape hatch: ADR-0005 (dependency + sandbox contract); `script` tool — host fns scoped to store/IR/patch primitives only; deny-by-default (no fs/net/env, no `register_package`); limits: ops, script size, call depth, wall-clock; any patch applied is journaled like `apply_patch` | API-09, D17 | 23 | 2s | escape-attempt suite (fs read, net, env, loop bomb, deep recursion → C3 diagnostics + zero side effects); golden bulk-edit scripts on fixture; ADR accepted |
| P3-25 | veraPDF runner (`adapters-pdf` glue + lint section): probe java+verapdf honest-absent; PDF/A + tagged-structure checks → lint manifest `a11y` section; wire into post-creation QA for pdf outputs | LIN-05 | 03 | 1 | absent-path golden; CI (verapdf installed) real run on created PDFs |
| P3-26 | Annotator beyond numbered boxes (REN-06 completion): severity-styled boxes, label collision avoidance, legend JSON manifest alongside marked+clean pair | REN-06 | (P2-29) | 1 | placement tests on synthetic lint fixtures; legend schema test; token cost still ≤ budget |

### Wave G — gates, fuzz, measurement, distribution & ship

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P3-27 | Write-path fuzz round 2: fuzzer targets for ODF writer (hostile content.xml + patch), PDF increment (hostile PDF + mutation), creation IR input, Rhai script corpus (bombs); nightly CI extension; SEC-06 Phase-3 review (pandoc args, chromium flag injection, typst input, sandbox) + `docs/threat-model.md` update | SEC-01, SEC-06 | 24, 10, 13 | 2s | fuzz regressions locked as tests; ≥24 h clean nightly log linked; review checklist green |
| P3-28 | Creation-path corpus measurement (C9): matrix {pandoc, chromium, typst, native} × {IR/md fixtures derived from 20 corpus docs} → page-count delta, SSIM vs LO golden, latency, drop counts → published table in `docs/`; edit-loop P95 (OPS-02) still green (no regression) | C9, OPS-02 | 23, 25 | 1 | measurement report doc + CI job; numbers recorded in status evidence |
| P3-29 | `selfupdate` primitive: ADR-0006 (channel = GitHub Releases per G4/S4; downloader = shell-out curl/wget; verify = minisign sig + sha256 before swap); `stichos update`: fetch manifest → verify → backup → swap → smoke test → rollback on fail; opt-in, explicit command only | DIS-05 | 03 | 2s | verify/rollback e2e against **local fixture channel** (file:// + fixture sig); tampered/unsigned manifest → refuse, no-op; smoke-fail → auto-rollback |
| P3-30 | OCI profile image (`docker/profile/`): stichos + LO 26.2 + poppler + pinned font set + qpdf + typst + pandoc + veraPDF, reusing HAR-03 pinning/fontset_hash; push GHCR | DIS-06 | 06, 25 | 1 | image builds in CI; `doctor --json` inside image = all-green golden |
| P3-31 | Release automation + playbook: version-bump script (unified workspace version), changelog gen, tag → CI builds ×6 targets + packages (reuse DIS-01..04) + GitHub Release draft + winget/Homebrew manifest drafts | DIS-08 | 29 | 1 | dry-run release (0.x) on tagged tree: workflow green, artifact list == ×6 targets; playbook in `docs/` |
| P3-32 | `tools/phase3_gate.sh`: asserts concept §7 (creation matrix + round-trip, ODF ×69, PDF prefix invariant, create-as-revisions, sandbox suite, fuzz status, C7 v1.2 additive check + full 15-tool schema matrix, OQ4 recorded, ×6 release status); reads `phase3-status.json` only; planted violations caught | ship | 27, 28, 31, 26 | 1 | gate green host + CI job added; planted-violation proof (silenced round-trip test, unfrozen C7) |
| P3-33 | M3 acceptance runbook + evidence: run all gate checks, publish report `docs/m3-acceptance.md`, tag `m3`, write Phase 4 intake (HUM-01 accept/reject, COM still deferred, OQ2/G3 + G5 open) | ship | 32 | 1 | report exists; tag `m3` == gate-green commit; CI green on pushed tree |

## 4. M3 exit-gate ↔ steps map

| Gate item (concept §7) | Steps | Asserted by |
|---|---|---|
| creation matrix measured + lint/schema/round-trip | P3-15..23, P3-28 | `phase3_gate.sh` runs the creation matrix; measurement doc exists |
| INV-1 across OOXML/ODF/PDF | P3-08..10, P3-12..14 (OOXML suites inherited) | gate runs zero-patch ×69, ODF unrelated-edit, PDF prefix invariant |
| create-as-revisions read-back | P3-22 | read-back + LO golden fixtures in gate |
| sandbox proof | P3-24 | escape suite in gate test list |
| fuzz round 2 + SEC-06 | P3-27 | nightly status + review doc |
| distribution (G4) | P3-29..31 | release CI status + selfupdate fixture proof |
| C7 v1.2 frozen, additive-only | P3-24, P3-32 | schema matrix + changelog check |
| Wave 0 (honest M2) | P3-01..03 | `phase2_gate.sh` green + `m2 == HEAD` at Wave A start |

## 5. Sizing & context notes (the 200k discipline)

- **2s steps** (P3-05, 08, 09, 10, 12, 15, 16, 17, 22, 24, 27, 29): split = build-core first,
  wire/test-second; handoff names files landed + the exact next assertion. Quote the spec clause
  (ODF mimetype rule, PDF 1.7 §7.5.6 incremental update, ECMA-376 content-types) into the module doc
  during part 1 so part 2 doesn't re-fetch.
- Corpus is never loaded into context: ×69 zero-patch and measurement matrices run **in code**;
  steps cite counts, not contents.
- Wave B ∥ C ∥ D are separate lanes touching `adapters-odf` / `adapters-pdf` / `adapters-ooxml`
  respectively — no file collisions; executor edits (P3-10, P3-14) touch `gp-core/executor.rs`
  sequentially, never in the same session.
- Engines absent on host (typst/qpdf/verapdf): steps must land the honest-absent path **and** the CI
  job that installs the tool; a step is not verified by skipping.
- New crates allowed only via their ADR steps (P3-04/24/29); `cargo deny` is re-run by every wave gate.
