# stichos — Codebase Structure

> **Note:** This document reflects the current state of the Rust codebase (synced 2026-09-16,
> tag `m3`). It is maintained by contributors; `project_plan.md` §3 is the authoritative plan
> reference. Where reality and the plan DAG differ, §"Known DAG deviations" below records it.

## Workspace layout

```
Cargo.toml                    # workspace root: 9 crates, resolver=2, unified version 0.1.0
rust-toolchain.toml           # MSRV 1.98 (pinned)
deny.toml                     # cargo-deny config (license allow-list, ban rules incl. typst crate)

crates/
  gp-core/                    # core types, IR, diagnostics, traits, store, journal, patch,
                              #   executor, policy, Rhai sandbox, selfupdate, testkit
  adapters-ooxml/             # OOXML (docx, xlsx, pptx): OPC, readers, surgical writers,
                              #   creation writers, revision writer, census, validation
  adapters-odf/               # ODF (odt, ods, odp): manifest, readers, surgical writer,
                              #   patch primitives, census, RelaxNG gate
  adapters-pdf/               # PDF: parser, text/bboxes, forms, annotations, outline,
                              #   metadata, qpdf glue, incremental (append-only) writer
  engines/                    # render engines as subprocesses: libreoffice, pandoc,
                              #   chromium, typst; probe, negotiate, doctor, cache, raster
  lint/                       # fidelity linting: layout, budget, ssim, contrast, font,
                              #   diff, annotator, a11y (veraPDF), perf, coords, cropper
  mcp/                        # MCP protocol: spec, tool registry, dispatch, jsonrpc,
                              #   server, transport, http (protocol layer only)
  cli/                        # the `stichos` binary — 18 verbs, wires everything together
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

## Crate details

### gp-core (foundation)

Core types, IR, diagnostics, traits, store, journal, patch, executor, policy, sandbox,
selfupdate. No workspace crate dependencies.

| Module | Purpose | Key types |
|--------|---------|-----------|
| `ir` | Intermediate representation (C1) | `Document`, `Spreadsheet`, `Presentation`, `PdfDocument`, `IRDocument`, `ElementId`, template metadata |
| `patch` | Abstract patch ops + IR application + unified diff (P3) | `PatchOp` |
| `diff` | IR structural diff | — |
| `executor` | Format routing, refusal matrix, QA loop, byte-diff report | `FormatType`, `ExecuteResult`, `PartByteDiff`, `QALoopResult`, `RenderPage`, `SignatureInfo`, `PolicyDenial` |
| `journal` | Undo journal + revert (C6, P6) | — |
| `store` | Content-addressed blob store | `Store` |
| `policy` | Edit allow-list + taint marks (C10) | — |
| `fidelity` | Fidelity budget types + merge algebra (C4) | `FidelityBudget`, `RendererInfo` |
| `diagnostics` | Diagnostic-code registry (C3) | `DiagnosticRegistry`, `DiagnosticCode`, `ReadError`, `Severity` |
| `script` | Sandboxed Rhai escape hatch (D17, ADR-0005) | `ScriptConfig`, `ScriptResult`, `run_script`, `has_sandbox_violations` |
| `selfupdate` | Verify-before-swap update primitive (ADR-0006) | `UpdateChannel`, `UpdateManifest`, `UpdateResult` |
| `traits` | Core trait definitions | `Codec`, `PatchExecutor`, `Engine`, `Probe`, `Linter`, `EngineInfo` |
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
| `executor` | Patch execution + refusal matrix for OOXML |
| `create` / `create_xlsx` / `create_pptx` | Native creation from IR (M3) |
| `create_template` | Template-based creation; style/theme parts byte-identical |
| `md_ir` | Markdown → IR converter |
| `revision_writer` | Tracked-changes write (`w:ins`/`w:del`, `paraId` anchoring) + `create_as_revisions` |
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
| `census` | ODF feature census → budget |
| `rng_validate` | RelaxNG validation gate (bundled ODF 1.2 rng, xmllint) |

### adapters-pdf

PDF parsing and incremental mutation. No rendering — structural only. D19 stack (lopdf +
pdf-writer, qpdf shell-out glue).

| Module | Purpose |
|--------|---------|
| `parser` | PDF object layer parser (page tree, xref, object streams) |
| `text` | Text extraction with bounding boxes |
| `form` | AcroForm field read/write (XFA flagged unsupported) |
| `annot` | Annotation read/write |
| `outline` / `metadata` | TOC + metadata/XMP extraction and updates |
| `qpdf` | `qpdf --check` runner glue (honest-absent) |
| `incremental` | Append-only incremental writer (prefix invariant; refuses encrypted/xref-stream/obj-stm; signed-input refusal) |
| `patch_executor` | IR patch → PDF increments (`PatchExecutor` impl) |

### engines

Render engines as spawned subprocesses, never linked (D3).

| Module | Purpose |
|--------|---------|
| `libreoffice` | LibreOffice headless runner (isolated profiles, timeouts, wedge kill) |
| `pandoc` | Pandoc runner: text extraction + creation (`--reference-doc`) |
| `chromium` | Chromium runner: discovery chain, `--headless=new --print-to-pdf`/`--screenshot`, isolated user-data-dir |
| `typst` | Typst runner: `typst compile` shell-out, honest-absent path (ADR-0004) |
| `creation` | Unified creation dispatcher (pandoc/chromium/typst/native/revisions) |
| `probe` | Engine discovery + fixture-conversion capability probes (D8) |
| `negotiate` | Renderer negotiation incl. `authoring_order()` per conversion |
| `doctor` | Doctor probe matrix (engines × fixtures, incl. chromium/pandoc/typst rows) |
| `raster` | Rasterization ladder (poppler → gs → mutool → soffice) |
| `cache` | Render master cache (key per INV-4) |
| `capability` | Capability matrix (C5) |
| `mock` | Mock engine for testing |

Engine identifiers: `libreoffice`, `pandoc`, `chromium`, `typst` (live); `ms-office` (defined,
not implemented — COM tier deferred).

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
| `tools` | Tool registry (19 specs) + validation |
| `dispatch` | Tool handler trait + closure adapter |
| `jsonrpc` | JSON-RPC protocol |
| `server` | MCP server implementation |
| `transport` | Transport modes (stdio) |
| `http` | HTTP transport (session store, `POST /` routing) |

MCP tool surface (C7 v1.3.0, 19 specs): `inspect_document`, `render_document`,
`inspect_region`, `diff_documents`, `doctor`, `serve` (v1.0.0); `apply_patch`, `edit_text`,
`edit_cell`, `edit_structure`, `annotate_pages`, `revert_document`, `journal` (v1.1.0);
`create_document`, `script_tool`, `check_update` (v1.2.0); `list_revisions`, `accept_revisions`,
`reject_revisions` (v1.3.0).

### cli

The `stichos` binary. Entry point, clap parsing, subcommand dispatch, engine/adapter wiring.

| Module | Purpose |
|--------|---------|
| `main` | CLI entry, 18-verb dispatch |
| `inspect` / `region` / `render` / `diff_tool` / `doctor_tool` | Read-side verbs |
| `apply_tool` / `sugar_edits` | `apply`, `edit` — surgical edits + sugar (tracked revisions default) |
| `journal_tool` | `journal` / `revert` |
| `perf_tool` | `perf` — OPS-02 latency gate |
| `script_tool` | `script` — sandboxed Rhai runner |
| `budget_enforcer` | INV-2 / C8 payload budget enforcement |
| (verbs in `main.rs`) | `serve`, `schema`, `create`, `update` |

Subcommands (18): `inspect`, `region`, `render`, `diff`, `doctor`, `serve`, `schema`, `apply`,
`edit`, `annotate`, `revert`, `journal`, `perf`, `create`, `script`, `update`, `revisions`,
`review`.

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
  # manifests, blobs, reference text; 151 fixtures (69 ODF), pr-subset.txt (27)
```

## Docs

```
docs/
  contracts/              # C1–C10 interface specs (C7 at v1.3.0)
  adr/                    # ADR-0001..0006 + legacy pre-convention files (see README index)
  gov/                    # name hygiene, registration
  harness/                # harness-specific docs
  phase0-execution.md     # M0 runbook
  phase1-concept/plan/status · phase2-… · phase3-…   # per-phase artifacts
  m2-acceptance.md · m3-acceptance.md                # milestone acceptance reports (m3 §7 = corrections)
  creation-measurement.md/.csv                       # M3 creation-path matrix (C9)
  threat-model.md         # M1–M3 threat model incl. SEC-06 reviews
  KNOWN_ISSUES.md         # KI-01..05
  release-playbook.md     # DIS-08 release procedure
```

## Test coverage summary (measured 2026-09-16, tag m3+, `cargo test --workspace`)

| Crate | Passed | Failed | Ignored | Notes |
|-------|--------|--------|---------|-------|
| gp-core | 335 | 0 | 0 | incl. script sandbox + selfupdate suites |
| adapters-ooxml | 282 | 0 | 0 | read + surgical write + creation + revisions |
| adapters-odf | 146 | 0 | 0 | zero-patch ×69 in-suite |
| adapters-pdf | 100 | 0 | 1 | prefix invariant in-suite |
| engines | 219 | 0 | 0 | chromium/typst honest-absent paths |
| lint | 294 | 0 | 0 | |
| mcp | 62 | 0 | 0 | |
| cli (bin) | 86 | 0 | 0 | |
| cli integration (`tests/cli_tests.rs`) | 10 | 0 | 0 | end-to-end binary tests |
| adapters-ooxfuzz | 107 | 0 | 0 | fuzz regressions locked as tests |
| doctests | 3 | 0 | 0 | |
| **Total** | **1733+** | **0** | **1** | |

## Naming conventions

- Crate names: `kebab-case` (`gp-core`, `adapters-ooxml`)
- Package names: match crate names except `cli` → `stichos`
- Module names: `snake_case`
- Public types: `PascalCase`
- Public functions: `snake_case`
- Constants: `UPPER_SNAKE_CASE`
- Diagnostic codes: `PREFIX-CATnnn` (e.g., `GP-E001`, `OOX-E001`)
