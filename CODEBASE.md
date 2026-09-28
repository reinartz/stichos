# stichos — Codebase Structure

> **Note:** This document reflects the current state of the Rust codebase (synced 2026-09-27,
> HEAD `3b4a396`, working tree clean). It is maintained by contributors; `project_plan.md` §3 is the
> authoritative plan reference. Where reality and the plan DAG differ, §"Known DAG deviations"
> below records it. Track A′ (M6 closeout round 2) is **complete** — gate 90/90, `p606` 6/6, tag
> `m6` re-anchored + pushed; Phase-7 Track B status: P7-5 committed (`3b4a396`); remaining: P7-3
> tranche C/D deferred, P7-9 launch owner-only — tracked in `docs/remaining-work.md`; audit trail
> in `docs/m6-acceptance.md` §11–§13.

## Workspace layout

```
Cargo.toml                    # workspace root: 9 crates, resolver=2, unified version 0.1.0
rust-toolchain.toml           # MSRV 1.98 (pinned)
deny.toml                     # cargo-deny config (license allow-list, ban rules incl. typst crate)

crates/
  gp-core/                    # core types, IR, diagnostics, traits, store, journal, patch,
                              #   executor, policy, Rhai sandbox, selfupdate, testkit;
                              #   M6: envelope.rs (C14 __budget), tempfiles.rs (URL/TTL),
                              #   formula/ (C15 evaluator+parser+parity+refuse), ir_convert
  adapters-ooxml/             # OOXML (docx, xlsx, pptx): OPC, readers, surgical writers,
                              #   creation writers, revision writer, census, validation
  adapters-odf/               # ODF (odt, ods, odp): manifest, readers, surgical writer,
                              #   patch primitives, census, RelaxNG gate; M6: ODS/ODP revisions
  adapters-pdf/               # PDF: parser, text/bboxes, forms, annotations, outline,
                              #   metadata, qpdf glue, incremental (append-only) writer
  engines/                    # render engines as subprocesses: libreoffice, pandoc,
                              #   chromium, typst; probe, negotiate, doctor, cache, raster;
                              #   M6: ms_office COM workers (feature `ms-office`), msi.rs,
                              #   scheduler.rs (logon-agent model)
  lint/                       # fidelity linting: layout, budget, ssim, contrast, font,
                              #   diff, annotator, a11y (veraPDF), perf, coords, cropper
  mcp/                        # MCP protocol: spec, tool registry, dispatch, jsonrpc,
                              #   server, transport, http (protocol layer only; Wave 0
                              #   removed the adapters-pdf edge via gp-core::traits)
  cli/                        # the `stichos` binary — 21 verbs, wires everything together
  adapters-ooxfuzz/           # fuzzing harness (dev-only): read + write-path fuzz corpora
```

## Crate dependency graph (measured from Cargo.toml, 2026-09-16)

```
gp-core                              (traits + types; no workspace deps)
  |
  +-- adapters-ooxml                 (gp-core, lint)
  +-- adapters-odf                   (gp-core)
  +-- adapters-pdf                   (gp-core)
  +-- engines                        (gp-core, adapters-ooxml)
  +-- lint                           (gp-core)
  +-- mcp                            (gp-core — protocol only; no adapter/engine knowledge)
  |
  +-- cli -- depends on all of the above (the de-facto wiring point)
  +-- adapters-ooxfuzz -- (gp-core, adapters-ooxml, adapters-pdf, lint)
```

All edges are ratified in `project_plan.md` §3.2 as the canonical DAG.
`tools/check-deps.sh` enforces cross-crate edge constraints (P4-02).

## Known DAG deviations (measured 2026-09-23)

- **None open.** The former `mcp -> adapters-pdf` edge (`crates/mcp/src/http.rs` text-layer route)
  was removed in Phase 6 Wave 0 by dependency inversion: text extraction moved behind
  `gp-core::traits::PdfTextSource` with the impl in `adapters-pdf` and wiring in `cli`.
  `tools/check-deps.sh` now exits 0 (the `m5`-tree failure recorded in `docs/m5-acceptance.md` §9
  is resolved).

## Crate details

### gp-core (foundation)

Core types, IR, diagnostics, traits, store, journal, patch, executor, policy, sandbox,
selfupdate. No workspace crate dependencies.

| Module | Purpose | Key types |
|--------|---------|-----------|
| `ir` | Intermediate representation (C1) | `Document`, `Spreadsheet`, `Presentation`, `PdfDocument`, `IRDocument`, `ElementId`, template metadata, `PdfAnnotation` (C12), `SignatureInfo` (C13), `Document.annotations` |
| `patch` | Abstract patch ops + IR application + unified diff (P3) | `PatchOp` |
| `diff` | IR structural diff | — |
| `executor` | Format routing, refusal matrix, QA loop, byte-diff report, signature detection + enforcement | `FormatType`, `ExecuteResult`, `PartByteDiff`, `QALoopResult`, `RenderPage`, `SignatureInfo`, `PolicyDenial`, `ExecutorConfig.force_signature_invalidation` |
| `journal` | Undo journal + revert (C6, P6), signature-invalidation events | `SignatureInvalidated`, `SignedDocument` |
| `store` | Content-addressed blob store | `Store` |
| `policy` | Edit allow-list + taint marks (C10) | — |
| `fidelity` | Fidelity budget types + merge algebra (C4, incl. signature-invalidation entry) | `FidelityBudget`, `RendererInfo` |
| `diagnostics` | Diagnostic-code registry (C3): GP-A0xx (C12 refusals), GP-S001..016 (C13) | `DiagnosticRegistry`, `DiagnosticCode`, `ReadError`, `Severity` |
| `script` | Sandboxed Rhai escape hatch (D17, ADR-0005) | `ScriptConfig`, `ScriptResult`, `run_script`, `has_sandbox_violations` |
| `selfupdate` | Verify-before-swap update primitive (ADR-0006) | `UpdateChannel`, `UpdateManifest`, `UpdateResult` |
| `traits` | Core trait definitions | `Codec`, `PatchExecutor`, `Engine`, `Probe`, `Linter`, `EngineInfo` |
| `signature` | PAdES-B-B sign-later (D25, pure-Rust crypto): keygen, real manual-DER PKCS#7/CMS builder (P7-7 committed `7d56af5`/`fe266cd`), byte-range, `/Sig` detection | `sign_pdf()`, `detect_signatures()`, `detect_signatures_raw()` |
| `timestamp_client` / `ocsp_client` / `crl_client` / `ltv` / `pkcs12` / `cert_chain` | PAdES-T/LTV (P7-5, feature `pades-ltv`): RFC 3161 TSA, OCSP/CRL fetch, LTV embed, PKCS#12 + chain (GP-S-020..025) | `SigningIdentity`, `cert_chain` validation |
| `edit_stability` | Element-ID stability property tests (C2) | — |
| `macro_check` | Macro expansion checks | — |
| `testkit` | Shared test utilities | — |

Constants: `IR_VERSION = "1.0.0"`, `ELEMENT_ID_PATTERN = "^id-[0-9a-f]{12,}$"`

### adapters-ooxml

Open XML (OOXML) adapters: read, surgical write, create, revisions.

| Module | Purpose |
|--------|---------|
| `opc` | OPC package reader + raw stream-copy writer core |
| `writer` / `commit` | Surgical package writer (P1): unchanged entries byte-copied |
| `xml` / `span_mut` | Namespace/order-preserving DOM-lite + byte-span patch primitives |
| `docx` / `xlsx` / `pptx` | Readers → IR (structure, IDs, coordinates, budget) |
| `docx_writer` / `xlsx_writer` / `pptx_writer` | Surgical part writers feeding `commit` |
| `executor` | Patch execution + refusal matrix for OOXML (incl. signed-doc GP-S0xx refusal) |
| `create` / `create_xlsx` / `create_pptx` | Native creation from IR (M3) |
| `create_template` | Template-based creation; style/theme parts byte-identical |
| `md_ir` | Markdown → IR converter |
| `revision_writer` | Tracked-changes write (`w:ins`/`w:del`, `paraId` anchoring) + `create_as_revisions` |
| `revision_reader` / `revision_executor` / `revision_ops` / `comment_writer` | Revision + comment read/write (C11, M4) |
| `census` | Feature census → fidelity budget |
| `validate` | Schema/structural validation gate |

### adapters-odf

OpenDocument Format (ODF) adapters. Full read + surgical write since M3.

| Module | Purpose |
|--------|---------|
| `manifest` | ODF manifest parsing (+ hashes) |
| `odt` / `ods` / `odp` | Readers → IR |
| `xml` | XML parsing utilities (DOM-lite with spans) |
| `writer` | Surgical package writer: mimetype-first/stored, manifest updates, encryption refusal |
| `patch` | Offset-based content.xml patch primitives (right-to-left, span re-verify) |
| `patch_executor` | IR patch → odt/ods/odp routing (`PatchExecutor` impl) |
| `revision_reader` / `revision_writer` / `revision_executor` / `comment_writer` | ODF revision + comment read/write (C11, M4; ODS/ODP revision write landed in M6) |
| `lossless_suite` | Corpus lossless round-trip suite |
| `census` | ODF feature census → budget |
| `rng_validate` | RelaxNG validation gate (bundled ODF 1.2 rng, xmllint) |

### adapters-pdf

PDF parsing and incremental mutation. No rendering — structural only. D19 stack (lopdf +
pdf-writer, qpdf shell-out glue).

| Module | Purpose |
|--------|---------|
| `parser` | PDF object layer parser (page tree, xref, object streams) |
| `text` | Text extraction with bounding boxes (also serves the review text-layer route) |
| `form` | AcroForm field read/write (XFA flagged unsupported) |
| `annot` | Annotation read (Highlight/Text/FreeText/Stamp → C12) + enumeration parity |
| `annot_write` | Annotation write-back via the incremental writer (append-only, journaled, `dry_run`) |
| `outline` / `metadata` | TOC + metadata/XMP extraction and updates |
| `qpdf` | `qpdf --check` runner glue (honest-absent) |
| `incremental` | Append-only incremental writer (prefix invariant; refuses encrypted/xref-stream/obj-stm; signed-input refusal) + `/Sig` detection |
| `patch_executor` | IR patch → PDF increments (`PatchExecutor` impl) |

### engines

Render engines as spawned subprocesses, never linked (D3).

| Module | Purpose |
|--------|---------|
| `libreoffice` | LibreOffice headless runner (isolated profiles, timeouts, wedge kill) |
| `pandoc` | Pandoc runner: text extraction + creation (`--reference-doc`) |
| `chromium` | Chromium runner: discovery chain, `--headless=new --print-to-pdf`, isolated user-data-dir (print-to-pdf only; `--screenshot` = Track C gap) |
| `typst` | Typst runner: `typst compile` shell-out, honest-absent path (ADR-0004) |
| `creation` | Unified creation dispatcher (pandoc/chromium/typst/native/revisions) |
| `probe` | Engine discovery + fixture-conversion capability probes (D8) |
| `negotiate` | Renderer negotiation incl. `authoring_order()` per conversion |
| `doctor` | Doctor probe matrix (engines × fixtures + PDF/A revalidation row); SystemInfo carries chromium/pandoc/typst version fields with report rows in `engines::doctor`, but CLI doctor registers only libreoffice+pandoc engines and omits chromium/typst version fields in JSON (Track C gap C1.3) |
| `raster` | Rasterization ladder (poppler → gs → mutool → soffice) |
| `cache` | Render master cache (key per INV-4) |
| `capability` | Capability matrix (C5) |
| `mock` | Mock engine for testing |
| `ms_office` | COM worker implementation (COM-2..6) on the Windows host; cfg stubs on non-Windows |
| `open_handler` | Platform handoff resolver for `stichos open` (M4) |

Engine identifiers: `libreoffice`, `pandoc`, `chromium`, `typst` (live); `ms-office` (Windows host; COM-2..6 implemented, D10 logon-agent model).

### lint

Fidelity linting. Consumes render artifacts (PDFs, extracted text) — never touches source packages.

| Module | Purpose |
|--------|---------|
| `linter` | Main linter orchestration (overflow, overlaps, coords, autofit) |
| `budget` | Fidelity budget enforcement (C4) |
| `ssim` | Per-page SSIM comparison (C9 parity) |
| `contrast` | Color contrast checks |
| `font` | Font substitution / missing-glyph detection |
| `diff` | IR structural diff renderers |
| `sheet` | Sheet handling |
| `encode` | Encoding fidelity tests |
| `cropper` / `coords` | Region cropping + `bbox_pt` ↔ pixel bookkeeping |
| `page_delta` | Page-count delta checks |
| `annotator` | Numbered/severity lint boxes on pages + legend JSON manifest (REN-06) |
| `a11y` | Accessibility section orchestration (veraPDF-backed) |
| `verapdf` | veraPDF runner (honest-absent probe; CI-real) |
| `validate` | Schema gate wiring (OOX-08 + ODF RelaxNG + qpdf) |
| `perf` | Edit→render→look latency measurement (OPS-02) |
| `c3` | C3 diagnostic-code checks |

### mcp

MCP (Model Context Protocol) server. Protocol layer only — wiring lives in `cli`.

| Module | Purpose |
|--------|---------|
| `spec` | Tool specification types (C7 compliance; per-tool `schema_version`) |
| `tools` | Tool registry (23 specs) + validation |
| `dispatch` | Tool handler trait + closure adapter |
| `jsonrpc` | JSON-RPC protocol |
| `server` | MCP server implementation |
| `session` | Review sessions incl. PDF-annot mode + text-layer opt-in (M5) |
| `transport` | Transport modes (stdio) |
| `http` | HTTP transport (session store, `POST /` routing, `/viewer/pdf/*` + `/review/<token>/text/<N>.json` routes) |

MCP tool surface (C7 v1.6.0, 23 specs): `doctor`, `serve` (v1.0.0); `revert_document` (v1.1.0);
`create_document`, `script_tool`, `check_update` (v1.2.0); `list_revisions`, `accept_revisions`,
`reject_revisions`, `add_comment` (v1.3.0); `list_annotations`, `resolve_annotations` (v1.4.0);
`sign_document` (v1.5.0); `inspect_document`, `render_document`, `inspect_region`,
`diff_documents`, `apply_patch`, `edit_text`, `edit_cell`, `edit_structure`, `annotate_pages`,
`journal` (v1.6.0).

### cli

The `stichos` binary. Entry point, clap parsing, subcommand dispatch, engine/adapter wiring.

| Module | Purpose |
|--------|---------|
| `main` | CLI entry, 21-verb dispatch |
| `inspect` / `region` / `render` / `diff_tool` / `doctor_tool` | Read-side verbs (doctor incl. PDF/A revalidation row) |
| `apply_tool` / `sugar_edits` | `apply`, `edit` — surgical edits + sugar (tracked revisions default) |
| `journal_tool` | `journal` / `revert` |
| `annots` | `annots list` / `annots resolve` — PDF annotation enumeration and write-back |
| `sign` | `sign` — PAdES-B-B sign-later (M5) |
| `revisions_tool` | `revisions` — accept/reject on tracked changes (OOXML + ODF, C11) |
| `review_tool` | `review` — review sessions incl. PDF-annot mode + `--text-layer` opt-in |
| `open_tool` | `open` — platform handoff resolver |
| `perf_tool` | `perf` — OPS-02 latency gate |
| `script_tool` | `script` — sandboxed Rhai runner |
| `budget_enforcer` | INV-2 / C8 payload budget enforcement |
| (verbs in `main.rs`) | `serve`, `schema`, `create`, `update` |

Subcommands (21): `inspect`, `region`, `render`, `diff`, `doctor`, `serve`, `schema`, `apply`,
`edit`, `annotate`, `revert`, `journal`, `perf`, `create`, `script`, `update`, `revisions`,
`annots`, `review`, `open`, `sign`.

### adapters-ooxfuzz

Fuzzing harness for OOXML + write paths. Dev-only, not shipped.

| Module | Purpose |
|--------|---------|
| `lib` | Read-path fuzz cases |
| `odf_writer_fuzz` | ODF writer hostile-content cases |
| `pdf_incremental_fuzz` | PDF incremental-writer cases |
| `creation_fuzz` | Creation-IR input cases |
| `rhai_corpus` | Rhai script-bomb corpus (sandbox regression lock) |

## Harness (Python, dev-only)

```
harness/
  baseline/
    baseline.py           # baseline generation utility
  generator/
    gen/
      core.py             # core generator logic
      builders/           # per-format builders (docx, odf, pptx, xlsx)
  metrics/
    metrics/
      __main__.py         # metrics CLI entry point
      features.py         # document feature counting
      pages.py            # page count metrics
      ssim.py             # SSIM image similarity
      text.py             # text fidelity (F1 score)
    tests/                # 43 pytest functions
  render/
    golden.py             # golden comparison tool
```

## Corpus

```
corpus/
  PLAN.md                 # fixture generation plan
  GENERATION_REPORT.md    # generation results
  # manifests, blobs, reference text; 169 fixtures (157 M0–M4 + 12 PDF-annot/signed/PDF-A from P5-02), pr-subset.txt (27)
```

## Docs

```
docs/
  contracts/              # C1–C15 interface specs (C7 v1.6.0 in spec.rs AND C7.md matrix — T-A2
                          #   landed ff80f8e; C12/C13 frozen at M5; C14 frozen at M6 (C7 v1.6.0);
                          #   C15 frozen at M6)
  adr/                    # ADR-0001..0013 (0009 pdf.js vendoring, 0010 crypto stack ACCEPTED,
                          #   0011 formula evaluation, 0012 PAdES-T/LTV ACCEPTED, 0013 PDF redaction
                          #   ACCEPTED) + legacy pre-convention files (see README index)
  gov/                    # name hygiene, registration
  harness/                # harness-specific docs
  implementation-guide-context-budget.md  # C14 implementation detail (M6 input)
  phase0-execution.md     # M0 runbook
  phase1-concept/plan/status · phase2-… · phase3-… · phase4-… · phase5-…   # per-phase artifacts
  phase6-concept/plan/status · m6-acceptance.md  # Phase 6 (M6, shipped; closeout complete)
  track-b-p7-{1,2,3,4,7,8,9}-*.md  # Phase-7 intake specs (tracked; P7-1 xlsx create, P7-2 pptx
                          #   create, P7-3 formula coverage, P7-4 lint encode/ssim, P7-7 CMS
                          #   signing, P7-8 pdfium spike, P7-9 launch)
  m2-acceptance.md · m3-acceptance.md · m4-acceptance.md · m5-acceptance.md  # milestone acceptance reports (m3 §7, m4 Git-State, m5 §9 = post-acceptance corrections)
  creation-measurement.md/.csv                       # M3 creation-path matrix (C9)
  threat-model.md         # threat model incl. SEC-06 reviews (M1–M6)
  KNOWN_ISSUES.md         # KI-01..09
  release-playbook.md     # DIS-08 release procedure
  remaining-work.md       # post-M6 task map (Track A′ closeout round 2 — complete; Track B Phase-7 backlog)
```

## Reviewer + vendored assets

```
assets/
  reviewer/index.html     # review overlay UI (raster-over-masters default + opt-in text layer, M5)
  vendor/pdfjs/           # vendored pdf.js 3.11.174 (text layer only) + HASHES.toml (ADR-0009, D24)
```

## Test coverage summary (measured 2026-09-27, HEAD `3b4a396`, `cargo test --workspace --lib --no-fail-fast`)

| Crate | Passed | Failed | Ignored | Notes |
|-------|--------|--------|---------|-------|
| gp-core | 567 | 9 | 0 | incl. script sandbox + selfupdate + C12/C13 suites; 9 fails = pre-existing date-hardcoded `formula::evaluator` tests (time-dependent); with `--features pades-ltv`: 567 → 592 pass / 9 fail |
| adapters-ooxml | 352 | 4 | 0 | read + surgical write + creation + revisions (C11); 4 fails = known OPC zip-bomb (accepted) |
| adapters-odf | 226 | 0 | 0 | zero-patch in-suite + revision write |
| adapters-pdf | 149 | 0 | 2 | prefix invariant in-suite + annot enumeration/write-back |
| engines | 244 | 0 | 0 | chromium/typst honest-absent paths + COM skeleton |
| lint | 300 | 0 | 0 | |
| mcp | 93 | 0 | 0 | incl. session/review + text-layer route tests |
| adapters-ooxfuzz | 226 | 0 | 0 | fuzz regressions locked as tests (rounds 1–4) |
| **Total (lib)** | **2 157** | **13** | **2** | 13 = 4 known OPC zip-bomb + 9 date-hardcoded gp-core (all pre-existing) |

(CLI bin + integration + doctest suites from the m3-era table are exercised via the phase gates;
see `docs/m5-acceptance.md` §3 for the gate transcript.)

## Naming conventions

- Crate names: `kebab-case` (`gp-core`, `adapters-ooxml`)
- Package names: match crate names except `cli` → `stichos`
- Module names: `snake_case`
- Public types: `PascalCase`
- Public functions: `snake_case`
- Constants: `UPPER_SNAKE_CASE`
- Diagnostic codes: `PREFIX-CATnnn` (e.g., `GP-E001`, `OOX-E001`)
