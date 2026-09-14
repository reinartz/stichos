# stichos — Codebase Structure

> **Note:** This document reflects the current state of the Rust codebase. It is maintained by
> contributors; `project_plan.md` §3 is the authoritative plan reference.

## Workspace layout

```
Cargo.toml                    # workspace root: 9 crates, resolver=2, unified version 0.1.0
rust-toolchain.toml           # MSRV 1.98 (pinned)
deny.toml                     # cargo-deny config (license allow-list, ban rules)

crates/
  gp-core/                    # core types, IR, diagnostics, traits, store, testkit
  adapters-ooxml/             # OOXML (docx, xlsx, pptx), OPC, XML parsing
  adapters-odf/               # ODF (odt, ods, odp), manifest, XML parsing
  adapters-pdf/               # PDF parsing (parser, text, form, annot, outline, metadata, qpdf)
  engines/                    # render engine abstraction (pandoc, libreoffice, probe, cache, mock)
  lint/                       # fidelity linting (budget, ssim, contrast, font, diff, cropper)
  mcp/                        # MCP protocol (jsonrpc, dispatch, server, tools, transport, http, spec)
  cli/                        # the `stichos` binary (inspect, render, diff, doctor, serve)
  adapters-ooxfuzz/           # fuzzing harness for OOXML adapters
```

## Crate dependency graph

```
gp-core                              (traits + types; no workspace deps)
  |
  +-- adapters-ooxml                  (depends on gp-core)
  +-- adapters-odf                    (depends on gp-core)
  +-- adapters-pdf                    (depends on gp-core)
  +-- engines                         (depends on gp-core)
  +-- lint                            (depends on gp-core, adapters-pdf)
  +-- mcp                             (depends on gp-core, adapters-ooxml, adapters-odf, adapters-pdf)
  |
  +-- cli -- depends on all above + mcp
  |
  +-- adapters-ooxfuzz -- depends on gp-core, adapters-ooxml, adapters-pdf
```

Dependency enforcement:
- `gp-core` may depend on external allow-list only
- `adapters-*` may depend on `gp-core` only
- `engines` may depend on `gp-core` only
- `lint` may depend on `gp-core`, `adapters-pdf`
- `mcp` may depend on everything
- `cli` may depend on `mcp` (transitively all)

## Crate details

### gp-core (foundation)

Core types, IR, diagnostics, traits, and content-addressed store. No workspace crate dependencies.

| Module | Purpose | Key types |
|--------|---------|-----------|
| `ir` | Intermediate representation types | `Document`, `Spreadsheet`, `Presentation`, `PdfDocument`, `IRDocument`, `ElementId` |
| `fidelity` | Fidelity budget types + merge algebra | `FidelityBudget`, `RendererInfo` |
| `diagnostics` | Diagnostic code registry | `DiagnosticRegistry`, `DiagnosticCode`, `ReadError`, `Severity` |
| `store` | Content-addressed blob store | `Store` |
| `traits` | Core trait definitions | `Codec`, `PatchExecutor`, `Engine`, `Probe`, `Linter`, `EngineInfo` |
| `macro_check` | Macro expansion checks | — |
| `testkit` | Shared test utilities | — |

Constants: `IR_VERSION = "1.0.0"`, `ELEMENT_ID_PATTERN = "^id-[0-9a-f]{12,}$"`

### adapters-ooxml

Open XML (OOXML) format adapters. Parses docx, xlsx, pptx, and OPC packages.

| Module | Purpose |
|--------|---------|
| `lib` | Module re-exports |
| `opc` | OPC (Open Packaging Conventions) package reader |
| `xml` | XML parsing utilities |
| `docx` | WordprocessingML adapter |
| `xlsx` | SpreadsheetML adapter |
| `pptx` | PresentationML adapter |
| `census` | Document census (feature counting) |
| `validate` | Package validation |
| `main` | Stub (prints error, exits) |

### adapters-odf

OpenDocument Format (ODF) adapters. Parses odt, ods, odp.

| Module | Purpose |
|--------|---------|
| `lib` | Module re-exports |
| `odf` | ODF package reader (deprecated stub) |
| `xml` | XML parsing utilities |
| `manifest` | ODF manifest parsing |
| `odt` | Word processing adapter |
| `ods` | Spreadsheet adapter |
| `odp` | Presentation adapter |
| `main` | Stub (prints error, exits) |

### adapters-pdf

PDF parsing and analysis. No rendering — purely structural analysis.

| Module | Purpose |
|--------|---------|
| `lib` | Module re-exports |
| `parser` | PDF object layer parser |
| `text` | Text extraction with bounding boxes |
| `form` | Form field parsing |
| `annot` | Annotation extraction |
| `outline` | Document outline / TOC parsing |
| `metadata` | PDF metadata extraction |
| `qpdf` | QPDF integration for checks |
| `main` | Stub (prints error, exits) |

### engines

Render engine abstraction. Engines are spawned subprocesses, never linked.

| Module | Purpose |
|--------|---------|
| `lib` | Module re-exports, engine trait re-exports |
| `libreoffice` | LibreOffice headless subprocess runner |
| `pandoc` | Pandoc subprocess runner |
| `probe` | Engine discovery and capability probing |
| `negotiate` | Engine selection / fallback negotiation |
| `doctor` | Doctor probe logic |
| `cache` | Render cache management |
| `raster` | Rasterization helpers |
| `capability` | Capability matrix aggregation |
| `mock` | Mock engine for testing |
| `main` | Stub (prints error, exits) |

Engine identifiers: `libreoffice`, `pandoc`, `ms-office`.

### lint

Fidelity linting. Consumes render artifacts (PDFs, extracted text) — never touches source packages.

| Module | Purpose |
|--------|---------|
| `lib` | Module re-exports |
| `linter` | Main linter orchestration |
| `budget` | Fidelity budget enforcement |
| `ssim` | Per-page SSIM comparison |
| `contrast` | Color contrast checks |
| `font` | Font substitution / missing glyph detection |
| `diff` | IR structural diff |
| `sheet` | Sheet handling |
| `encode` | Encoding fidelity tests |
| `cropper` | Region cropping |
| `coords` | Coordinate transformation |
| `page_delta` | Page count delta |
| `c3` | C3 metric checks |
| `main` | Stub (prints error, exits) |

### mcp

MCP (Model Context Protocol) server implementation. The only crate allowed to know the MCP SDK.

| Module | Purpose |
|--------|---------|
| `lib` | Module re-exports |
| `spec` | Tool specification types (C7 compliance) |
| `tools` | Tool registry + validation |
| `dispatch` | Tool handler trait + closure adapter |
| `jsonrpc` | JSON-RPC protocol |
| `server` | MCP server implementation |
| `transport` | Transport modes (stdio, HTTP) |
| `http` | HTTP transport server |
| `main` | Stub (prints error, exits) |

Phase 1 read-only tools: `inspect_document`, `render_document`, `inspect_region`,
`diff_documents`, `doctor`, `serve`.

### cli

The `stichos` binary. Entry point with CLI subcommands.

| Module | Purpose |
|--------|---------|
| `main` | CLI entry point, clap argument parsing, subcommand dispatch |
| `inspect` | `inspect` subcommand handler |
| `render` | `render` subcommand handler |
| `diff_tool` | `diff` subcommand handler |
| `doctor_tool` | `doctor` subcommand handler |
| `region` | `region` subcommand handler |
| `budget_enforcer` | INV-2 / C8 payload budget enforcement |

Subcommands: `inspect`, `region`, `render`, `diff`, `doctor`, `serve`, `schema`.

Note: `apply` is intentionally absent in Phase 1 (read-only ship).

### adapters-ooxfuzz

Fuzzing harness for OOXML adapters. Not a shipped component — dev-only fuzzing.

| Module | Purpose |
|--------|---------|
| `lib` | Fuzz test cases (59 tests) |

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
  # manifests, blobs, reference text
```

## Docs

```
docs/
  contracts/              # C1–C9 interface specs
  adr/                    # architecture decision records
  gov/                    # name hygiene, registration
  harness/                # harness-specific docs
  contracts/              # contract specs (C1–C9)
  phase*-executions       # milestone execution records
```

## Test coverage summary

| Area | Files | `#[test]` / `def test` | Notes |
|------|-------|----------------------|-------|
| Rust (all crates) | 69 `.rs` | ~650 `#[test]` | ~465 across 45 files with `#[cfg(test)]`; 21 files with 0 tests (mostly trivial stubs) |
| Python harness | 5 `.py` | 43 `def test_` | Well-covered for dev-only harness |
| Integration tests | 1 (`cli_tests.rs`) | 10 `#[test]` | End-to-end CLI binary tests |

## Naming conventions

- Crate names: `kebab-case` (`gp-core`, `adapters-ooxml`)
- Package names: match crate names except `cli` → `stichos`
- Module names: `snake_case`
- Public types: `PascalCase`
- Public functions: `snake_case`
- Constants: `UPPER_SNAKE_CASE`
- Diagnostic codes: `PREFIX-CATnnn` (e.g., `GP-E001`, `OOX-E001`)
