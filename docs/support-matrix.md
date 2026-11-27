# stichos — Support Matrix

> **Note:** This document tracks engine support, distro compatibility, deprecation notices,
> and the evolving tool surface. It is a living reference — update it when tools/engines
> are added or retired.

## Engines (N/N-1 support)

| Engine | Supported versions | Notes |
|--------|-------------------|-------|
| LibreOffice | 24.2+ (current 26.2.6) | Headless mode; 26.2 confirmed on this host |
| Pandoc | 2.0+ (current 3.7.0) | Text extraction + creation; 3.7 confirmed |
| Chromium | Any (headless) | `--headless=new --print-to-pdf`; version probed |
| Typst | — | Not installed on this host; honest-absent path shipped |

## Distro Grid

| Distribution | Rust 1.85+ | LibreOffice | Pandoc | Chromium |
|-------------|-----------|-------------|--------|----------|
| Ubuntu 22.04 | ✓ | ✓ (PPA) | ✓ (APT) | ✓ (APT) |
| Ubuntu 24.04 | ✓ | ✓ (APT) | ✓ (APT) | ✓ (APT) |
| Fedora 39+ | ✓ | ✓ (DNF) | ✓ (DNF) | ✓ (DNF) |
| Debian 12+ | ✓ | ✓ (APT) | ✓ (APT) | ✓ (APT) |
| Alpine 3.19+ | ✓ | ✓ (APKG) | limited | limited |

## Deprecation Notices

- **M1 tool surface (v1.0.0):** Frozen. No removals planned; additive changes only.
- **Typst engine:** Marked as honest-absent (not installed); future implementation may depend on D15.
- **MS-Office COM engine:** Implementation (COM-2..6) landed on the Windows host in Phase 6;
  `ms-office` feature flag gates the crate for non-Windows compile. The COM worker runs
  interactively on Windows (D10 logon-agent model) with LibreOffice as the honest-absent
  fallback on other platforms (D9).
- **pdf.js:** Vendored 3.11.174 under `assets/vendor/pdfjs/` (text layer only, opt-in per session,
  D24); raster overlay remains the default medium.

## Phase-5 Surface (current)

- **23 MCP tools** (measured 2026-09-23: `mcp/spec.rs` asserts `TOOLS.len() == 23`): Added
  `list_annotations`, `resolve_annotations`, `sign_document`
- **21 CLI verbs** (measured from `enum Command` in `cli/src/main.rs`): Added `annots`, `sign`
  (21 total: inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert,
  journal, perf, create, script, update, revisions, annots, review, open, sign)
- **Schema version:** 1.5.0 (additive-only since v1.0.0)
- **Contracts:** C1 (minor additive), C7 (v1.5.0), C12 (frozen at M5), C13 (frozen at M5);
  C14 draft (freezes at M6 as C7 v1.6)

## Phase-6 Surface (shipped 2026-09-23)

- **23 MCP tools:** 10 tools updated to `schema_version: "1.6.0"` (additive defaults):
  `inspect_document`, `render_document`, `inspect_region`, `diff_documents`, `apply_patch`,
  `edit_text`, `edit_cell`, `edit_structure`, `annotate_pages`, `journal`
- **Default changes (CTO-01..06):**
  - `inspect_document`: `detail="summary"` (was full detail)
  - `render_document`: `output="urls"` (was base64 inline)
  - `inspect_region`: `output="urls"` (was base64 inline)
  - `diff_documents`: `max_ops=20` (was unlimited)
  - `apply_patch`: `dry_run=true` (was false — applied immediately)
  - `edit_*`: `dry_run=true` (was false — applied immediately)
  - `annotate_pages`: `output="urls"`, `max_pages=5`
  - `journal`: `max_entries=10`
- **C14 envelope:** Every response carries `__budget` (tokens_est, elements_count, image_count),
  `diagnostics`, and `rendered_by`
- **Legacy opt-outs:** `inspect --detail=all`, `render --output=base64`, `diff --max-ops=0`,
  mutation without `--apply` restores legacy behavior
- **Contracts:** C7 frozen at v1.6.0; C14 frozen at v1.0.0

## Phase-7 Intake (post-M6)

- **24 MCP tools** (Track-H adopted: added `validate_package`): 12 tools at `schema_version: "1.6.0"` (additive defaults), 11 tools at `schema_version: "1.0.0"` (read-only), 1 tool at `schema_version: "1.7.0"` (`validate_package`)
- xlsx/pptx create (content-driven from IR)
- Full formula coverage (C15 extension)
- Lint encode/ssim refresh
- PAdES-T/LTV progression
- PDF redaction
- Real CMS signing (PKCS#7/CMS)
- pdfium raster rung

## Phase-8 Surface (Wave 6 — Surfacing & contracts, 2026-11-22)

- **24 MCP tools** (measured: `mcp/spec.rs` asserts `TOOLS.len() == 24`)
- **CLI:** 24 verbs; `stichos schema --json` now emits capability matrix as structured JSON
  (nested object keyed by format, not raw tuple array)
- **MCP `inspect_document`** parses `detail` and `features` params; `detail="feature"` enables
  all feature blocks (styles, media, annots, ids, coords, bboxes) via the same budget-enveloped
  pipeline as the CLI `inspect` command
- **Feature parity** (honest state from `CAPABILITIES` constant, measured 2026-11-22):

| Format | Feature | State | Notes |
|--------|---------|-------|-------|
| DOCX | complex_fields | 📖 read | Only read-capable DOCX feature |
| DOCX | styles | ❌ none | |
| DOCX | hyperlinks | ❌ none | |
| DOCX | drawings_media | ❌ none | |
| DOCX | images | ❌ none | |
| XLSX | cell_styles | ❌ none | |
| XLSX | freeze_panes | ❌ none | |
| XLSX | autofilter | ❌ none | |
| XLSX | defined_names | ❌ none | |
| XLSX | conditional_formatting | ❌ none | |
| XLSX | data_validation | ❌ none | |
| XLSX | tables | ❌ none | |
| XLSX | protection | ❌ none | |
| XLSX | page_setup | ❌ none | |
| XLSX | hyperlinks | ❌ none | |
| XLSX | number_formats | ❌ none | |
| PPTX | shape_tree | 📖 read | re-landed W5; xform/rot/prstGeom/fill/ln |
| PPTX | placeholders | 📖 read | |
| PPTX | table_cells | 📖 read | |
| PPTX | theme | 📖 read | |
| PPTX | notes | 📖 read | |
| PPTX | slide_size | 📖 read | |
| PPTX | charts | 📖 read | |
| Cross | capability_matrix | 📖 read | `stichos schema` + `--json` |
| Cross | inspect_flags | 📖 read | `--styles --media --annots --ids --coords --bboxes` |
| Cross | csv_export | ❌ none | |

Legend: 📖 = read-only implemented, ❌ = not yet surfaced in capability matrix

## Test Counts (measured 2026-11-27)

| Suite | Command | Count |
|-------|---------|-------|
| Workspace lib | `cargo test --workspace --lib` | 838 pass / 1 fail (pre-existing fixture count mismatch) |
| MCP protocol | `cargo test -p stichos-mcp --lib` | 93 pass / 0 fail |
| OOXML fuzz | `cargo test -p adapters-ooxfuzz` | 226 pass / 0 fail |
| Corpus apply | `cargo test -p adapters-ooxml --test real_corpus_apply` | 37 pass / 0 fail |
| Property tests | `cargo test -p adapters-ooxml --test property_tests` | 23 pass / 0 fail |
| Pyoracle | `python3 -m pytest harness/pyoracle -q` | 61 pass / 0 fail |

## Phase-4 Surface (historical)

- **20 MCP tools** (measured 2026-09-18: `mcp/spec.rs:824` asserts `TOOLS.len() == 20`): Added `list_revisions`, `accept_revisions`, `reject_revisions`, `add_comment`
- **19 CLI verbs** (measured from `enum Command` in `cli/src/main.rs`): Added `revisions`, `review`, `open`, `schema` (19 total: inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, review, open)
- **Schema version:** 1.3.0 (additive-only since v1.0.0)
- **Contracts:** C1 (minor additive), C7 (v1.3.0), C11 (frozen at M4)
