# stichos

Phases 0–4 complete and gated (tags `m1-closeout`, `m2`, `m3`, `m4`; M4 verified 2026-09-18: 1 851
tests green, clippy/fmt clean, `phase4_gate.sh` 26/26 headless). Phase 5 (PDF-native loop + trust)
is drafted, not started — concept `docs/phase5-concept.md`, plan `docs/phase5-plan.md`. Rust
workspace lives in `crates/`; `harness/`, `corpus/`, `docker/`, `baseline/`, `.github/` hold the M0
Python fidelity harness and supporting material.

## Source of truth
- `project_concept.md` — decisions D1–D22, architecture, verification log. Claims there are marked
  measured/verified; do not add unverified claims. If a decision changes, update the decision log (§16) too.
- `brand.md` — identity rules; `assets/` holds the mark, wordmarks, lockups, generated rasters.
- Name is **always lowercase** `stichos`. Never `Stichos`, never bold the wordmark (use ink vs paper).
- Voice: "proof", "render", "fidelity budget", "renderer provenance"; CLI verbs `inspect render diff apply doctor`.

## Asset pipeline (run from `assets/`)
- `python3 wordmark.py` regenerates `wordmark*.svg` and `lockup*.svg` (needs PIL + fontconfig; picks Inter via fc-match).
- `*-outline.svg` files are generated: re-run the inkscape text-to-path loop in `wordmark.py`'s docstring after editing any wordmark/lockup SVG.
- `bash build-icons.sh` regenerates everything under `dist/icons/` (needs ImageMagick `magick`). Never hand-edit `dist/icons/`.
- `out/` is design-review scratch output — regenerable, not shipped; ignore it.
- `.icns` needs `iconutil` on macOS; not available on this Linux box.

## Brand rules agents break
- Exactly **one** vermilion (`#E2483A`) element per composition; vermilion on ink/paper only; never fill/gradient it.
- Never reintroduce Office blue or LibreOffice green.
- Palette: ink `#101A2E`, paper `#FBF7EE`, slate `#93A0B8`, surface `#F4F1EA`.
- Three icon tiers are measured, not decorative: Full ≥64px, Tight 16–48px, Glyph ≥20px (outline only). Don't merge tiers or "simplify".
- Lockups must embed the Full-tier mark (Tight reads as a checkbox at lockup sizes).

## Phase status

Phase 4 (M4) shipped 2026-09-18: tag `m4` on the gate-green commit, `main` + `m4` pushed.
Records: `docs/phase4-plan.md`, `docs/phase4-status.json`, `docs/m4-acceptance.md` (Git-State section
has the tag-re-anchoring correction — milestone tags move only to committed gate-green trees).
Phase 5 (M5) drafted 2026-09-18, **not started**: `docs/phase5-concept.md`, `docs/phase5-plan.md`,
live status `docs/phase5-status.json`. M6 intake (COM impl, formula eval, actual launch, ODS/ODP
revisions) is listed in phase5-concept §6. Phase-3 records: `docs/phase3-plan.md`,
`docs/phase3-status.json`, `docs/m3-acceptance.md` (§7 has post-acceptance corrections).

## Phase-3 tech

9 Rust crates in the workspace (`crates/`):

| Crate | Purpose |
|-------|---------|
| `gp-core` | Core types, IR, diagnostics, traits, content-addressed store |
| `adapters-ooxml` | OOXML (docx, xlsx, pptx), OPC, XML parsing |
| `adapters-odf` | ODF (odt, ods, odp), manifest, XML parsing |
| `adapters-pdf` | PDF parsing (parser, text, form, annot, outline, metadata, qpdf) |
| `engines` | Render engine abstraction (libreoffice, pandoc, chromium, typst live; ms-office COM deferred) |
| `lint` | Fidelity linting (budget, ssim, contrast, font, diff, cropper) |
| `mcp` | MCP protocol (jsonrpc, dispatch, server, tools, transport, http, spec) |
| `cli` | `stichos` binary — 19 verbs (inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, review, open) |
| `adapters-ooxfuzz` | Fuzzing harness for OOXML adapters (dev-only) |

Engines are spawned subprocesses, never linked; zero hard external deps.
