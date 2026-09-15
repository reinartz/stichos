# stichos — Project Plan

**Status:** Phase 0 (M0) executing · derived from [`project_concept.md`](project_concept.md) (decisions D1–D20, phases §14)
**Last updated:** 2026-09-08 — runbook stages SA-1..SA-10 done (`docs/phase0-execution.md`): 151-fixture corpus +
goldens, baseline report-v1 published, reproducible `fontset_hash`. Repo live: `github.com/reinartz/stichos`
(private, D18; pushed 2026-09-07). **Nightly ×3 green (run 34221533557** after two determinism fixes, runbook
CI bug-fix log**).** SA-6 golden approval ✅ (owner 2026-09-08) · **G1 ✅ decided: lopdf + pdf-writer
(D19, ADR-0001 accepted)** · OQ5 ✅ fully resolved (skip `stichos.com`/no homepage — D20; no TM filing — D21).
M0 exit gate now blocked only on: the **GOV-02 registration sweep**.
**Rule:** this file is a plan, not a decision record. If a plan choice contradicts the concept,
resolve it in `project_concept.md` §16 (decision log) first, then reflect it here.
Traceability: every task cites the concept section (§) or decision (D) it implements.

---

## 1. Reading guide

- **Milestones** M0–M4 map 1:1 to concept §14 Phases 0–4, each with a hard exit gate.
- **Tracks** are the fine-grained work split: 16 tracks, 114 tasks (+6 COM subtasks). Task IDs are stable
  (`HAR-03`); the `Depends` column is the internal work graph.
- **Components** (§3) are the crates + repo-level dirs; their dependency DAG (§3.2) and the
  contract registry (§4) are the "well-maintained dependencies" mechanism — both CI-enforced.
- Effort figures are planning estimates for a senior-Rust team of 4–5, **not measured claims**.

---

## 2. Milestones and exit gates

| M | Concept phase | Ships | Exit gate (all must pass) | Est. |
|---|---|---|---|---|
| **M0** | Phase 0 — harness | no user artifact; fidelity harness + corpus live in CI | harness runs ≥150 fixtures × engines, baseline report published (HAR-06); reproducible `fontset_hash`; CI nightly green on 3 platforms; name-hygiene sweep done (GOV-01..03) | 4–6 w |
| **M1** | Phase 1 — read | read-only binary: `inspect`/`render`/`doctor` + MCP server, 6 targets | installs clean on all 6 targets; every tool response carries provenance (D6); read-path fuzz-clean (SEC-01/06); `doctor` capability matrix = real conversions (D8); edit tools **absent** | +8–12 w |
| **M2** | Phase 2 — surgical edit | closed-loop editing: `apply`/`diff`, dry-run, tracked changes, per-edit lint | INV-1 surgical suite green (§3.4); edit→render→look ≤ 2 s P95 in CI perf gate (OPS-02); fidelity budget on 100 % of mutations (D6); tracked changes default (D7) | +10–14 w |
| **M3** | Phase 3 — creation | create from IR/Markdown/HTML/Typst; Rhai escape hatch | created docs pass lint + schema validation; pandoc/Chromium/Typst paths measured against corpus | +6 w |
| **M4** | Phase 4 — human loop | pdf.js overlay, accept/reject, Collabora/add-in handoff | tracked-change accept/reject round-trip lossless on corpus subset | +6–8 w |

Cross-cutting tracks (SEC, DIS, OPS, GOV) run continuously and are gated per milestone.

---

## 3. Components

### 3.1 Repository layout

```
stichos/
├── Cargo.toml                  # workspace, resolver=2, unified version
├── crates/
│   ├── core/                   # gp-core: IR, element IDs, store, journal, patch model,
│   │                           #   fidelity budget, policy types, trait definitions, diagnostics
│   ├── adapters-ooxml/         # OPC + WordprocessingML / SpreadsheetML / PresentationML
│   ├── adapters-odf/           # ODF package + odt/ods/odp
│   ├── adapters-pdf/           # PDF object layer, forms, annotations, incremental update
│   ├── engines/                # probe framework; LO/pandoc/poppler/gs/mutool/Chromium runners;
│   │                           #   render ladder + master cache; COM worker (windows, feature-gated)
│   ├── lint/                   # layout linter, font checks, schema gates, perceptual diff
│   ├── mcp/                    # tool registry, MCP stdio + streamable HTTP, payload budget
│   └── cli/                    # the `stichos` binary (the only fn main)
├── harness/                    # Python fidelity harness (concept §6: Python kept for eval only)
├── corpus/                     # fixture manifests + generator; blobs in external object store
├── docker/                     # pinned harness/OCI images (LO version + fixed font set)
├── packaging/                  # deb, rpm, macOS pkg, WiX MSI, brew cask, winget manifests
├── docs/                       # ADRs, contract specs (docs/contracts/), threat model
└── assets/                     # brand (existing; unchanged)
```

### 3.2 Internal dependency DAG (CI-enforced)

```
                          gp-core        (traits + types; no workspace deps)
             ┌──────────────┼──────────────┬───────────────┐
      adapters-ooxml  adapters-odf   adapters-pdf        engines
             │              │            │   ╲              │
             ╲              │            ╲    ╲─────────────┤
              ╲             │             ╲                 │
                       lint (consumes core + adapters-pdf + render artifacts;
                         ╲                  never renders, never touches packages)
                          ╲                ╱
                            mcp ──────────╯   (protocol + tools + sole wiring point)
                             │
                            cli
```

| Crate | May depend on | Must NOT depend on | Enforced by |
|---|---|---|---|
| `gp-core` | external allow-list only | any workspace crate | `cargo tree` check + `cargo deny` |
| `adapters-*` | `gp-core` only | each other, `engines`, `lint` | `tools/check-deps.sh` in CI |
| `engines` | `gp-core` only | adapters, lint (no document semantics here) | same |
| `lint` | `gp-core`, `adapters-pdf` | `engines` (consumes render artifacts) | same |
| `mcp` | everything | — (sole crate allowed to know the MCP SDK) | deny SDK outside `mcp` |
| `cli` | `mcp` (transitively all) | direct adapter/engine calls | graph check + review rule |

**Dependency-inversion rule:** `gp-core` defines the `Codec`, `PatchExecutor`, `Engine`,
`Probe`, `Linter` traits; adapters/engines/lint only implement them; `mcp` is the only place
that wires implementations together (nothing else dispatches on format or engine).
Formats are cargo features (`ooxml`, `odf`, `pdf`) on `mcp`/`cli`; default = all.

### 3.3 External dependency policy (concept §8, D3/D4/D8)

| Class | Items | Rule |
|---|---|---|
| Baked in (Rust crates) | zip, quick-xml, serde, sha2, thiserror, lopdf, pdf-writer, typst, rmcp-or-own-jsonrpc, clap, image | `cargo deny` license allow-list MIT/Apache-2.0/BSD/ISC/Unicode; advisory gate; SBOM per release |
| **Banned pending gate** | any C-linked renderer (pdfium, MuPDF, LOKit), any AGPL crate | blocked until PDF-01/OQ1 ADR; `cargo deny` bans + CI check that no `-sys` crate for these enters the tree (protects D2 cross-compile) |
| Soft, spawned at runtime | LibreOffice, MS 365 (COM), pandoc, poppler, Ghostscript, mutool, Chromium | never `Depends:`/`Requires:`, `Recommends:` only (D8); capability = fixture conversion at probe time |
| Bundled data | Carlito, Caladea, Liberation, Noto subset (D14) | license audit (DIS-07) before bundling |

### 3.4 Global invariants (test suites, not prose)

- **INV-1 Surgical (P1/D5):** an unrelated edit leaves every untouched OPC part byte-identical
  (incl. zip metadata where the format allows); PDF edits are append-only increments.
- **INV-2 Provenance (D6):** no tool response lacks `rendered_by` or an explicit
  `visual_feedback:"unavailable"` (§9). Asserted by response-schema on every tool.
- **INV-3 Never in place (P6):** originals immutable in the store; every mutation journaled;
  revert returns a byte-identical original.
- **INV-4 Determinism (D15):** render-cache key = `sha256(doc)+engine+version+fontset_hash+dpi`;
  golden renders bit-reproducible inside the pinned container.
- **INV-5 Hostile input (§11):** fuzz targets (OPC reader, XML, PDF) clean ≥24 h per release;
  zip-bomb/entity limits proven on a malicious-package corpus.

---

## 4. Contract registry — versioned interfaces between tracks

Each contract = a spec in `docs/contracts/`, a Rust type in `gp-core` (or owner crate), and a
**contract test suite** run by both producer and consumers. Freezes gate the work graph.

| # | Contract | Owner | Consumers | Freeze |
|---|---|---|---|---|
| C1 | **IR schema**: node types, property maps, schema version + migration policy | CORE | adapters, lint, API | M1 v1; minor bumps additive-only after |
| C2 | **Element ID scheme** (P4): content-anchored, edit-stable, serialized ID map | CORE | adapters, PAT, API | M2 |
| C3 | **Diagnostic codes**: stable IDs, severity, machine-readable JSON | LIN | every track, doctor, harness | before M1 tools ship |
| C4 | **Fidelity budget** (P2): `dropped_features[]`, warnings, `rendered_by`, confidence, merge algebra | CORE | adapters, PAT, API | M1 read-shape; extended M2 |
| C5 | **RenderResult + capability matrix** (D6, §9): engine id/version/fontset_hash | ENG | REN, LIN, API | M1 |
| C6 | **Journal format** (P6): versions, pre/post hashes, revert | CORE | PAT, API | M2 |
| C7 | **Tool schema** (MCP + CLI): names, args, `dry_run`, idempotency, deprecation cycle | API | clients | v1 at M1; additive only |
| C8 | **Payload rules** (D13): ≤20 image blocks, pre-resize table, token estimator (§10) | API | REN, LIN | M1 |
| C9 | **Corpus manifest + metric definitions** incl. SSIM parity harness↔binary | HAR | LIN, CI | M0 |
| C10 | **Policy model** (§11): edit allow-list, taint marks | SEC | PAT, API | M2 |

Versioning discipline: one workspace version for all crates and the binary; contracts
semver independently; `tools/check-deps.sh` + `cargo deny` guard the DAG; `cargo-machete`
prunes unused deps; per-crate changelogs; ADR required for any new external crate.

---

## 5. Work breakdown

`Deps` = task IDs that must land first. Effort in person-weeks (p-w), planning estimates.

### Track HAR — fidelity harness & corpus (Phase 0; §14, §5.3)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| HAR-01 | Corpus plan: categories (merged cells, SmartArt, embedded OLE, tracked changes, XFA, scanned OCR, CJK/RTL, 300-slide decks), sourcing, **redistribution license review** | — | plan + license table | 1 |
| HAR-02 | Synthetic fixture generator (Python) + `corpus/` manifests with feature-count expectations (`w:tbl`/`w:drawing`/`w:fldSimple`/comment counts) | HAR-01 | generator + ≥120 fixtures | 2 |
| HAR-03 | Pinned OCI harness image: Ubuntu LTS + exact LibreOffice build + poppler + fixed font set; emits `fontset_hash` (D14/D15, INV-4) | — | `docker/harness/` | 1 |
| HAR-04 | Metrics lib: per-page SSIM, page-count delta, feature-retention assertions, text-extraction F1 (C9) | HAR-01 | `harness/metrics/` | 2 |
| HAR-05 | Golden render pipeline: fixture × engine × DPI → approved-render store (content-addressed) | HAR-03, HAR-04 | goldens + store | 2 |
| HAR-06 | Baseline report: soffice + pandoc + pdftoppm over full corpus — the table that gates every library decision (§5 preamble, §19-1) | HAR-05 | baseline report v1 | 1 |
| HAR-07 | CI integration: PR = ~20-fixture subset, nightly = full; results trended | HAR-06 | workflows | 1 |
| HAR-08 | Engine-upgrade gate: new LO release → full corpus run + human review before pin bump (D15) | HAR-07 | upgrade procedure + job | 0.5 |

### Track CORE — workspace & kernel (`crates/core`)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| CORE-01 | Workspace scaffold: 8 crates, rust-toolchain (6 targets), workspace lints, unified version, `check-deps.sh`, `cargo deny`, `cargo-machete` | — | green CI skeleton | 1 |
| CORE-02 | Error model + diagnostic-code registry (C3) | CORE-01 | `gp-core::diag` | 1 |
| CORE-03 | IR schema v0 (C1): doc/section/para/run/table/list/header/footer/footnote/drawing + sheet/cell/formula + slide/layout/master/placeholder + PDF page/annot/form-field; serde; versioned | CORE-01 | `gp-core::ir` + spec | 3 |
| CORE-04 | Element IDs (C2, P4): content-anchored, edit-stable; ID-map serialization; property tests | CORE-03 | `gp-core::id` + `docs/contracts/C2.md` | 3 |
| CORE-05 | Content-addressed store + session dir (`~/.stichos/`): blob layout, doc versions, LRU | CORE-03 | `gp-core::store` | 2 |
| CORE-06 | Undo journal + single revert primitive (C6, P6) | CORE-05 | `gp-core::journal` | 2 |
| CORE-07 | Patch model: abstract ops (insert/replace/delete/style/tracked-change), IR application, IR unified diff (P3) | CORE-03 | `gp-core::patch` | 3 |
| CORE-08 | Fidelity-budget types + merge algebra (C4, P2) | CORE-02 | `gp-core::fidelity` | 1 |
| CORE-09 | Policy + taint-marking types (C10 skeleton, §11) | CORE-03 | `gp-core::policy` | 1 |
| CORE-10 | Trait definitions: `Codec`, `PatchExecutor`, `Engine`, `Probe`, `Linter` | CORE-03 | `gp-core::traits` | 1 |
| CORE-11 | Shared test kit: property tests, corpus-driven round-trip helpers for all crates | CORE-03 | `gp-core::testkit` | 1 |

### Track OOX — OOXML adapter (`crates/adapters-ooxml`)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| OOX-01 | OPC reader: zip, content types, parts, relationships; resource limits (bomb defense, entity caps) with SEC-01 | CORE-01 | reader + limit tests | 2 |
| OOX-02 | OPC writer, surgical strategy (P1): unchanged entries stream-copied byte-for-byte incl. extra fields; unknown parts pass-through + recorded | OOX-01 | writer + INV-1 tests | 3 |
| OOX-03 | XML layer: namespace- and order-preserving DOM-lite with byte spans (patch anchors) | OOX-01 | `ooxml::xml` | 2 |
| OOX-04 | WordprocessingML → IR: styles, numbering, sections, headers/footers, fields, comments, tracked-changes read | OOX-03, CORE-03, CORE-10 | Word reader | 4 |
| OOX-05 | SpreadsheetML → IR: cells, formulas **and cached values** (§14 P1), shared strings, styles, merged ranges, tables; charts/pivots detected → budget | OOX-03 | Excel reader | 3 |
| OOX-06 | PresentationML → IR: slides, layouts, masters, placeholders, notes | OOX-03 | PPT reader | 3 |
| OOX-07 | Feature census + drop report → fidelity budget (C4); before/after counts per part (§5.3) | OOX-04..06, CORE-08 | census module | 2 |
| OOX-08 | ECMA-376 XSD validation (embedded schema set; strict/loose modes; §5.3) | OOX-03 | validator + diagnostics | 2 |
| OOX-09 | IR patch → byte-surgical XML edits via spans; OPC commit path | OOX-02, OOX-03, CORE-07 | `ooxml::patch` | 4 |
| OOX-10 | Tracked-changes **write**: revisions, anchored comments, `paraId` anchoring (D7; §5.4 SecurityRonin lesson) | OOX-09 | revisions writer | 3 |
| OOX-11 | IR → OOXML creation writer (Phase 3) | OOX-09, CORE-03 | creator | 3 |

### Track ODF — ODF adapter (`crates/adapters-odf`)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| ODF-01 | ODF package reader/writer (manifest.xml, encryption detection), reusing the generic surgical-zip core from OOX-02 | OOX-02 | opc-odf module | 2 |
| ODF-02 | content.xml/styles.xml/meta.xml model + ODT → IR | ODF-01, CORE-03 | ODT reader | 3 |
| ODF-03 | ODS → IR, ODP → IR | ODF-02 | Calc/Impress readers | 3 |
| ODF-04 | Surgical patching of text-centric content.xml (offset-based; different shape from OPC parts) | ODF-02, CORE-07 | patch executor | 3 |
| ODF-05 | Feature census + drop report; OASIS RelaxNG validation | ODF-02 | census + validator | 2 |

### Track PDF — PDF adapter (`crates/adapters-pdf`)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| PDF-01 | **Decision gate OQ1** (ADR): pure-Rust (lopdf/pdf-writer) vs shelled-out qpdf/mutool vs AGPL in-process; raster-path choice incl. the pdfium question vs D2/D3 | — | ADR + `cargo deny` update | 1 |
| PDF-02 | Object-model reader: page tree, xref/xref streams, object streams; encryption policy (read-only best-effort) | CORE-01, PDF-01 | reader | 3 |
| PDF-03 | Incremental-update writer (P1 for PDF): append-only, signing-safe | PDF-02 | writer + INV-1 tests | 3 |
| PDF-04 | Text + **word-level bbox** extraction (§4 feedback layer): own content-stream extractor (basic) → poppler `-bbox` JSON when probed present | PDF-02 | bbox module | 4 |
| PDF-05 | Forms: AcroForm read/write; XFA detected → flagged unsupported (§14) | PDF-02 | forms module | 3 |
| PDF-06 | Annotations read/write; metadata, outline, tagged-structure extraction (a11y feed) | PDF-02 | annot/meta module | 3 |
| PDF-07 | qpdf `--check` / veraPDF runner glue (consumed by LIN-04/05) | PDF-02 | runner glue | 0.5 |

### Track ENG — engine pool & negotiation (`crates/engines`)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| ENG-01 | Engine API: trait impls + capability matrix (C5); provenance naming (`rendered_by` + version, D6) | CORE-10 | `engines::api` | 1 |
| ENG-02 | Probe framework (D8): discovery, fixture-conversion probes (capability = real conversion), cache, N/N-1 policy (D15) | ENG-01 | probe module | 3 |
| ENG-03 | LibreOffice runner: isolated `-env:UserInstallation` profiles, `--convert-to` + filter options (verified §17), hard timeouts + wedge kill, instance pool, version parse | ENG-01 | LO engine | 3 |
| ENG-04 | Renderer negotiation (§9): per-platform order, `renderer.preference`, per-session cache, honest `visual_feedback:"unavailable"` | ENG-02, ENG-03 | negotiation | 2 |
| ENG-05 | `doctor`: engine×fixture matrix (three bundled fixtures per §8), font probe (`fc-match` matrix per §8), LO-version notes; human + `--json` output | ENG-02 | doctor command | 2 |
| ENG-06 | pandoc runner: md/html → docx with `--reference-doc`; JSON AST passthrough (§5.2) | ENG-01 | pandoc engine | 1 |
| ENG-07 | Raster ladder (D11): poppler → gs → mutool → `soffice --convert-to png` behind one `rasterize()` | ENG-01 | raster module | 2 |
| ENG-08 | Master cache + orchestration (§10 loop-1): 150-DPI PNG masters, cache key per INV-4, LRU | ENG-07, CORE-05 | cache module | 2 |
| ENG-09 | **Windows COM worker** (D9/D10) — sub-split below; starts after M1, lands mid-M2 | ENG-01 | COM worker | 8 |
| ENG-10 | Chromium runner: headless screenshot + print-to-pdf (authoring, Phase 3); probe registered earlier | ENG-01 | chromium engine | 2 |
| ENG-11 | Typst integration (pure-Rust lib, §5.2; allowed under D2): typst → PDF path for authoring | ENG-01 | typst engine | 2 |
| ENG-12 | LOKit optional-plugin: design doc only; build only if D3 revisit trigger fires (throughput > 2 s/cycle) | — | ADR draft | 0.5 |

**COM sub-split (ENG-09):**
COM-1 design ADR — per-user logon agent, scheduled-task activation, never a Windows service (§9a, D10) →
COM-2 probing: ProgIDs (`Word/Excel/PowerPoint.Application`), `HKCR\...\CurVer`, ClickToRun
`ProductReleaseIds`, `WINWORD.EXE` under `root\Office16`; probe-never-assume (§9) →
COM-3 Word worker via `windows` crate: `DisplayAlerts=None`, `AutomationSecurity=High`, add-ins off,
one-doc-per-instance, hard timeout, activation/first-run dialog handling →
COM-4 Excel + PowerPoint workers →
COM-5 `CompareDocuments` authoritative track-changes diff tool (§5.3) →
COM-6 logon-agent packaging: MSI component + task-scheduler XML + docs (§9 unattended-batch story).

### Track REN — render→LLM payload pipeline (concept §10)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| REN-01 | Model-copy encoder: pre-resize to known sizes, JPEG q≈85/WebP, byte-exact accounting of what is sent (§10-6) | ENG-08 | encoder | 2 |
| REN-02 | Token estimator (verified `⌈w/28⌉×⌈h/28⌉` model, §10) + payload budgeter (C8; D13 ≤20 blocks, ≤2000 px/side when >20) | REN-01 | estimator | 1 |
| REN-03 | Changed-page detection + contact-sheet builder (§10-2; never re-send unchanged pages) | REN-01, CORE-05 | changed-pages module | 2 |
| REN-04 | Coordinate bookkeeping (§10-5): `bbox_pt` ↔ resized-pixel conversion, scale recorded per payload; property tests | PDF-04 | coord module | 1 |
| REN-05 | `inspect_region` cropper: from 300-DPI master, bbox+zoom → ~600-token crops (§10-3, D12) | ENG-08, REN-04 | cropper | 1 |
| REN-06 | Annotation renderer: numbered linter boxes drawn on page; ship marked + clean pair (§10-4) | REN-01, LIN-01 | annotator | 2 |

### Track LIN — lint & QA (concept §5.3; deterministic-before-VLM §19-4)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| LIN-01 | Layout linter core: frame overflow, off-page/negative coords, overlaps, autofit shrink — inputs = word bboxes + IR + page geometry; no VLM | PDF-04, CORE-03 | linter v1 | 4 |
| LIN-02 | Font substitution + missing-glyph detection: requested-vs-used fonts, tofu via raster heuristic (§5.3, D14) | LIN-01 | font checks | 2 |
| LIN-03 | Page-count delta + contrast checks | LIN-01 | extra checks | 1 |
| LIN-04 | Schema gate: OOX-08 + ODF-05 + `qpdf --check` (§5.3) | OOX-08, ODF-05, PDF-07 | validation gate | 1 |
| LIN-05 | veraPDF runner (PDF/A, PDF/UA) + Tagged-PDF structure checks (§15 a11y row) | PDF-06, PDF-07 | a11y gate | 2 |
| LIN-06 | Perceptual diff in Rust (SSIM/pixelmatch-class) with parity tests against harness metrics (C9) | HAR-04 | diff module | 2 |
| LIN-07 | Diagnostic-registry v1 freeze (C3) + lint manifest JSON schema | LIN-01 | frozen contract | 0.5 |
| LIN-08 | IR structural diff + XML part diff renderers (feeds `diff_documents`) | CORE-07 | diff renderers | 2 |

### Track PAT — patch kernel & closed loop (Phase 2; §4 KERNEL)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| PAT-01 | Patch executor: route IR patch → OOX-09 / ODF-04 / PDF-03; verify untouched-part hashes; emit byte-diff report (P1/P3) | OOX-09, ODF-04, PDF-03 | executor + INV-1 suite | 3 |
| PAT-02 | Dry-run flow: IR unified diff + preview render of proposed state in temp file (P3) | PAT-01, ENG-04 | dry-run | 2 |
| PAT-03 | Post-edit QA loop on every mutation: render + lint + fidelity assembly (§4; §10 2-s budget) | PAT-01, LIN-01, ENG-04 | closed loop | 2 |
| PAT-04 | Journal + revert + ID continuity across edits (P4/P6, C6) | PAT-01, CORE-06, CORE-04 | journaled edits | 2 |
| PAT-05 | Signature-invalidation detection + sign-later stub (§11) | PAT-01 | sig warning path | 1 |
| PAT-06 | Tracked-change edit path end-to-end; default on high-stakes formats (D7) | PAT-03, OOX-10 | revisions loop | 2 |

### Track API — tool surface: MCP + CLI (concept §4, §10)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| API-01 | Tool-registry design (C7): ~40 coarse, semantic, idempotent tools (§4); names/args/`dry_run`/idempotency semantics; versioning + deprecation policy | CORE-07 | tool spec v1 | 2 |
| API-02 | ADR `rmcp` vs own JSON-RPC; MCP stdio server; result shape = `content` + `structuredContent` + `isError` (§10, spec-verified) | API-01 | server v1 | 3 |
| API-03 | Streamable HTTP mode (`serve --http :8731`) + session handling (§7) | API-02 | http mode | 2 |
| API-04 | CLI skeleton: clap; verbs `inspect render diff apply doctor`; `--json`; exit-code table; **gate OQ3** (`gp` alias) | API-01 | binary | 2 |
| API-05 | Read tools: `inspect_document` (structure + coordinates), `inspect_region`, `render_document` (changed pages + contact sheet + manifest) | API-02, REN-* | read tools | 3 |
| API-06 | `diff_documents` + `doctor` tools | API-02, LIN-08, ENG-05 | tools | 1.5 |
| API-07 | Edit tool family: `apply_patch`, text/table/structure edits; `dry_run` on all (P3); tracked changes default (D7/P5) | API-02, PAT-* | edit tools | 4 |
| API-08 | Payload-budget enforcer (D13/C8) wired into every image-returning tool | REN-02, API-05 | enforcer | 1 |
| API-09 | Rhai sandbox escape hatch (D17): host fns scoped to store only (no fs/net); `script` tool | PAT-03 | scripting | 3 |
| API-10 | Creation tools + template handling (Phase 3, via AUT) | AUT-* | create tools | 2 |

### Track SEC — security (concept §11)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| SEC-01 | Hostile-input hardening: zip bombs, entity caps (XXE off; openpyxl precedent), oversized parts, path traversal/symlinked OPC entries; `cargo-fuzz` targets (OPC, XML, PDF) | OOX-01, PDF-02 | limits + fuzz CI jobs | 3 |
| SEC-02 | Supply chain: `cargo deny` (advisories + license bans per §3.3), Dependabot, SBOM (cargo-cyclonedx) | CORE-01 | policy in CI | 1 |
| SEC-03 | Taint marking + instruction-channel quarantine (C10): document text taints through IR → tool responses | CORE-09, OOX-04 | taint path | 2 |
| SEC-04 | Edit allow-list engine enforced in PAT (deny-by-default destructive ops; no exfiltration paths) | CORE-09, PAT-01 | policy gate | 2 |
| SEC-05 | Macro-format policy: xlsm/docm detected, disabled + flagged, never silently enabled | OOX-01 | macro policy | 0.5 |
| SEC-06 | Threat-model doc + security review at each ship gate (M1, M2) | SEC-01..05 | threat model | 1.5 |

### Track DIS — distribution (concept §7, D4/D8; gated per milestone)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| DIS-01 | CI cross-compile matrix: 6 targets, old-glibc container + musl fallback; CI check "no C in tree" (D2/D4) | CORE-01 | release CI | 2 |
| DIS-02 | Linux: deb + rpm wrapping the same binary, `Recommends:` only (D8); tarball + `install.sh`; optional AppImage | DIS-01 | Linux packages | 2 |
| DIS-03 | macOS: universal `.pkg` via `lipo`, codesign + notarization, Homebrew cask | DIS-01 | pkg + cask | 2 |
| DIS-04 | Windows: MSI (WiX) + portable ZIP + winget/choco manifests; Authenticode + timestamp | DIS-01 | Win packages | 2 |
| DIS-05 | `selfupdate` primitive: signed update channel, verification, rollback | DIS-02..04 | selfupdate | 2 |
| DIS-06 | Optional OCI profile image (LO + poppler + pinned fonts) — reuses HAR-03 (§8) | HAR-03 | profile image | 1 |
| DIS-07 | Font bundling: license audit of Carlito/Caladea/Liberation/Noto subset; first-run fontconfig strategy (D14) | — | audit + bundle | 1 |
| DIS-08 | Release process: version-bump script, changelog, GitHub Releases; **gate OQ4** (own apt/rpm repos vs Releases + winget/Homebrew only) | DIS-02..05 | release playbook | 1.5 |

### Track GOV — name hygiene & governance (concept §12, §19-6; do first)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| GOV-01 | Trademark search classes 9 + 42 (EUIPO/USPTO/WIPO Madrid) — gate before any public push (§12) — **DONE 2026-09-08: memo delivered (SA-1); owner decision: NO filing for the OSS project (D21), counsel-gate retired** | — | clearance memo | 1.5 |
| GOV-02 | Same-day registration sweep: GitHub **private repo under owner's personal account** (`reinartz/stichos`, no org — D18), crates.io/npm/PyPI (defensive), `stichos.dev`, MCP registry + skill-marketplace ids (`io.github.reinartz/stichos`) | GOV-01 | registrations | 0.5 |
| GOV-03 | Distro collision check: `apt-cache`/`dnf`/`brew`/`choco` on the five targets (§12) | — | results table | 0.5 |
| GOV-04 | Open-question board: OQ2 (Word tier: Aspose vs hosted worker — decide before COM-5 scope locks), OQ5; owner + due date per OQ | — | tracked decisions | 0.5 |
| GOV-05 | Repo hygiene: license (MIT/Apache-2.0 dual pending OQ1), headers, DCO, CONTRIBUTING, ADR template, D-log discipline (§16) | GOV-02 | repo policies | 0.5 |

### Track AUT — creation from IR (Phase 3)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| AUT-01 | Markdown/HTML → docx/odt via pandoc `--reference-doc` (branded templates) | ENG-06, ODF-02 | md path | 2 |
| AUT-02 | HTML → PDF via Chromium with the user's own engine (§5.1) | ENG-10 | html path | 1.5 |
| AUT-03 | Typst authoring path (pure Rust; deterministic, D2-compatible) | ENG-11 | typst path | 2 |
| AUT-04 | IR → OOXML/ODF native creation (uses OOX-11) + template metadata in IR | OOX-11, ODF-04 | native creation | 3 |
| AUT-05 | Creation output as tracked changes / comments when editing existing docs (D7) | AUT-04, OOX-10 | suggestion output | 1.5 |

### Track HUM — human loop (Phase 4)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| HUM-01 | pdf.js annotation overlay served by `serve` (static assets); accept/reject on tracked changes (§14 P4) | PAT-06, API-03 | overlay UI | 4 |
| HUM-02 | Handoff integration: open in Collabora/ONLYOFFICE/Word (`stichos open` resolves platform handler) | ENG-04 | handoff | 1.5 |
| HUM-03 | Office add-in spike (optional, decision-gated by demand) | COM-3 | spike report | 2 |

### Track OPS — observability & performance (cross-cutting)

| ID | Task | Deps | Deliverable | p-w |
|---|---|---|---|---|
| OPS-01 | Opt-in telemetry design + events (§11): engine-detection failures, fixture failures, fallback rates, latency histograms; privacy review | ENG-02 | telemetry v1 | 2 |
| OPS-02 | Perf-gate CI: edit→render→look P95 ≤ 2 s on reference machine (baseline measured §10); master-cache hit-rate tracking | PAT-03 | perf job | 1 |
| OPS-03 | Support matrix doc: engine N/N-1 table, distro support grid, deprecation notices (D15) | DIS-08 | support doc | 0.5 |

---

## 6. Sequencing, staffing, critical path

### 6.1 Critical path

`HAR-03 → CORE-01 → CORE-03 → OOX-01 → OOX-03 → OOX-04 → ENG-03 → ENG-08 → API-02/05` (M1)
`→ OOX-09 → PAT-01 → PAT-03 → API-07` (M2).
Longest serial pole ≈ 40–45 p-w of the ≈ 226 total; everything else parallelizes around it. The two riskiest
path items are **CORE-04 (stable IDs)** and **OOX-09 (surgical XML patching)** — both have
design ADRs before implementation and are staffed by the strongest kernel person.

### 6.2 Staffing model (4–5 senior Rust engineers)

| Role | Owns | Notes |
|---|---|---|
| E1 kernel | CORE-*, PAT-* | ID scheme + patch algebra; reviews all contract changes |
| E2 formats | OOX-*, ODF-* | ECMA-376/OASIS spec work; pairs with E1 on OOX-09 |
| E3 render/QA | PDF-*, REN-*, LIN-* | bbox extraction, linter, payload pipeline |
| E4 engines/ops | ENG-*, OPS-*, HAR infra | LO lifecycle, probes, later COM-1..6 |
| E5 surface | API-*, DIS-* (+ GOV with legal) | MCP/CLI, packaging, release engineering |
| shared | HAR-*, SEC-* | harness and security tasks rotate onto everyone; SEC-06 review is external-capable |

### 6.3 Wave plan (parallel tracks per wave; a wave ends when its tasks + gate pass)

| Wave | Contents (≈ weeks) | Gate |
|---|---|---|
| **W0** (w1–2) | HAR-01/03/04 · CORE-01/02/10 · GOV-01..03 · PDF-01 · DIS-01 | CI skeleton green; corpus plan + pinned image exist |
| **W1** (w3–6) | CORE-03..11 · OOX-01/02/03 · HAR-02/05/06 · SEC-01/02 · ENG-01/02/03/07 | M0: baseline report published (HAR-06); IR v0 frozen (C1 draft) |
| **W2** (w7–12) | OOX-04..08 · ODF-01..03 · PDF-02..07 · ENG-04/05/06/08 · REN-01..04 · LIN-01..03/06..08 · API-01..06 · DIS-02..04 · HAR-07 | **M1 ship**: read-only binary + MCP across 6 targets |
| **W3** (w13–22) | OOX-09/10 · ODF-04/05 · PDF-03 · PAT-01..06 · API-07/08 · SEC-03/04 · LIN-04/05 · REN-05/06 · ENG-09(COM-1..6) · OPS-01/02 · SEC-06 | **M2 ship**: closed-loop editing; Windows Office tier live |
| **W4** (w23–28) | ENG-10/11 · AUT-01..05 · API-09/10 · DIS-05..08 (+ Phase-2b intake per S2: ODF-04/05, PDF-03; COM stays deferred; Typst=CLI shell-out per S3; see `docs/phase3-plan.md`) | **M3 ship**: creation |
| **W5** (w29–35) | HUM-01..03 · OPS-03 · polish + corpus-driven hardening | **M4 ship**: human loop |

### 6.4 Decision gates (blocking; each has a decision-by wave and an owner)

| Gate | Question | Decide by | Blocks | Concept ref |
|---|---|---|---|---|
| G1 | OQ1 license split: pure-Rust only vs shell-out vs AGPL-in-process — **DECIDED 2026-09-08 (owner): lopdf + pdf-writer, qpdf shell-out (ADR-0001 accepted, D19)** | W0 ✅ | PDF-02 raster path, `cargo deny` allow-list, dual-license choice | §18-1, D2 |
| G2 | OQ3 `gp` alias yes/no | W2 (before API-04 CLI freeze) | C7 tool/CLI naming | §18-3 |
| G3 | OQ2 Word tier for non-Windows users (Aspose/Syncfusion vs hosted worker vs none) | W3 start | COM-5 scope, M2 marketing claims | §18-2 |
| G4 | OQ4 own apt/rpm repos vs GitHub Releases + winget/Homebrew — **DECIDED 2026-09-15 (owner): GitHub Releases + winget/Homebrew only, no own repos (phase3-concept §10 S4)** | W4 ✅ | DIS-08 | §18-4 |
| G5 | pdfium bundling (only if G1 shows pure-Rust extraction insufficient for LIN-01) | after HAR-06 data | D2 "C stays out" invariant — needs D-log entry if reversed | §5.3, D2 |

---

## 7. Risk register (top items, with the plan's mitigation)

| Risk | Impact | Mitigation in this plan |
|---|---|---|
| Stable-ID scheme (CORE-04) proves unsound under real edits | P4 promise breaks; multi-turn editing degrades | ADR + property tests + corpus replay before freeze (C2, W3 gate); fallback to journal-assigned IDs with anchor healing |
| Byte-surgical zip/XML fidelity (OOX-02/09) | INV-1 fails → "surgical" claim dead | invariant suite per fixture; stream-copy strategy; never use library save paths |
| Word automation constraints (§9) on Windows ARM64/Store editions | Office tier slips past M2 | probe-never-assume (COM-2); LO fallback always honest (D9); COM-6 logon-agent model isolated from core |
| Distro LibreOffice age/variability | layout truth varies; doctor blamed | fixture-based capability probes (D8), N/N-1 pinning (D15), version-in-report (D6) |
| pdfium vs "C stays out" tension | raster/extract quality ceiling | G1/G5 gates with HAR-06 evidence; shell-out ladder (D11) as floor |
| 20-image limit / vision pricing changes (D13) | payload design churn | estimator isolated in REN-02 behind C8; single config point |
| Corpus licensing (HAR-01) | cannot ship harness publicly | generated-fixture fallback (HAR-02); license review as first-class task |
| ECMA-376 breadth underestimated | W2 slips | read-only M1 first (concept §14: "read ships safely"); feature census in OOX-07 turns gaps into budget, not bugs |
| One-binary × 6 targets matrix breakage | release slips | DIS-01 from W0; install acceptance in every ship gate |

---

## 8. Definition of done (per task, unless task text says otherwise)

1. Unit + contract tests green; contract owners run producer+consumer suites (§4).
2. `check-deps.sh` + `cargo deny` + clippy + fmt clean; no new external crate without ADR.
3. Diagnostics emitted via C3 codes (no ad-hoc strings).
4. Docs: rustdoc + entry in `docs/contracts/` or ADR index if the task touched an interface.
5. Anything touching rendering/edits: corpus-subset run in CI (HAR-07) with no regression.

---

## 9. Effort summary (planning estimates, senior team)

| Track | p-w | | Track | p-w |
|---|---|---|---|---|
| HAR | 10.5 | | PAT | 12 |
| CORE | 19 | | API | 23.5 |
| OOX | 31 | | SEC | 10 |
| ODF | 13 | | DIS | 13.5 |
| PDF | 17.5 | | GOV | 3.5 |
| ENG (incl. COM) | 28.5 | | AUT | 10 |
| REN | 9 | | HUM | 7.5 |
| LIN | 14.5 | | OPS | 3.5 |
| **Total** | | | | **≈ 226.5 p-w** |

≈ 114 tasks ≈ 45 engineer-weeks of elapsed time with 5 people; the W0–W5 calendar above
(≈ 35 w) assumes 5–6 engineers with the harness/security rotation. Concept's own phase shape
(§14) is preserved; nothing in this plan contradicts D1–D17 — the only items that could force
a D-log edit are gates G1/G5 (pdfium/AGPL).



