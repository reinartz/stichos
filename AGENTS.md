# stichos

Stichos is a document fidelity tool: inspect, render, diff, and surgically edit Office and PDF
documents.  Feature-complete at version 0.1.0, pre-first-release.  The Rust workspace lives in
`crates/`; `harness/`, `corpus/`, `docker/`, `baseline/`, `.github/` hold the Python fidelity
harness and supporting material.

## Source of truth

- `project_concept.md` — decisions D1–D27, architecture, verification log.  Claims there are
  marked measured/verified; do not add unverified claims.  If a decision changes, update the
  decision log (16) too.
- `brand.md` — identity rules; `assets/` holds the mark, wordmarks, lockups, generated rasters.
- Name is **always lowercase** `stichos`.  Never `Stichos`, never bold the wordmark (use ink vs
  paper).
- Voice: "proof", "render", "fidelity budget", "renderer provenance"; CLI verbs
  `inspect render diff apply doctor` (24 verbs incl. `verify`, `validate`, `finish`):
  `inspect region render diff doctor serve schema apply edit annotate revert journal perf create
  script update revisions annots review open sign verify validate finish`.

## Asset pipeline (run from `assets/`)

- `python3 wordmark.py` regenerates `wordmark*.svg` and `lockup*.svg` (needs PIL + fontconfig;
  picks Inter via fc-match). `*-outline.svg` files are generated: re-run the inkscape text-to-path
  loop in `wordmark.py`'s docstring after editing any wordmark/lockup SVG.
- `bash build-icons.sh` regenerates everything under `dist/icons/` (needs ImageMagick `magick`).
  Never hand-edit `dist/icons/`. `out/` is design-review scratch output — regenerable, not shipped.
- `.icns` needs `iconutil` on macOS; not available on this Linux box.

## Brand rules agents break

- Exactly **one** vermilion (`#E2483A`) element per composition; vermilion on ink/paper only;
  never fill/gradient it.
- Never reintroduce Office blue or LibreOffice green.
- Palette: ink `#101A2E`, paper `#FBF7EE`, slate `#93A0B8`, surface `#F4F1EA`.
- Three icon tiers are measured, not decorative: Full ≥64px, Tight 16–48px, Glyph ≥20px
  (outline only).  Don't merge tiers or "simplify".
- Lockups must embed the Full-tier mark (Tight reads as a checkbox at lockup sizes).

## Workspace

9 Rust crates in the workspace (`crates/`):

| Crate | Purpose |
|-------|---------|
| `gp-core` | Core types, IR, diagnostics, traits, content-addressed store, PDF annotation model (C12), signature integrity (C13), context-budget envelope (C14), formula evaluator (C15), `ir_convert` |
| `adapters-ooxml` | OOXML (docx, xlsx, pptx), OPC, XML parsing, revision read/write (C11) |
| `adapters-odf` | ODF (odt, ods, odp), manifest, XML parsing, revision read/write |
| `adapters-pdf` | PDF parsing (parser, text, form, annot, outline, metadata, qpdf), incremental annot write-back |
| `engines` | Render engine abstraction (libreoffice, pandoc, chromium, typst live; ms-office COM on Windows host via logon-agent model) |
| `lint` | Fidelity linting (budget, ssim, contrast, font, diff, cropper) |
| `mcp` | MCP protocol (jsonrpc, dispatch, server, session/review incl. text-layer routes, tools, transport, http, spec) |
| `cli` | `stichos` binary — 24 verbs (inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, annots, review, open, sign, verify, validate, finish) |
| `adapters-ooxfuzz` | Fuzzing harness for OOXML adapters (dev-only) |

Engines are spawned subprocesses, never linked; zero hard external deps.

## Development

- `cargo test --workspace --lib` — all lib tests (2 350 pass / 0 fails, 2 ignored)
- `cargo test -p mcp --lib` — MCP protocol tests (93 pass / 0 fails)
- `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` — lint
- `tools/check-deps.sh` — dependency DAG enforcement (exits 0)
- `tools/phase*_gate.sh` — phase gates where available
- Engines spawned as subprocesses; no hard external dependencies.

## Current status

- Test counts: **2 350 pass / 0 fails** (workspace lib), **93 / 0** (mcp)
- Version 0.1.0; no v* tag yet; repo private until owner launch.
