# stichos

Phases 0–5 complete and gated (tags `m1-closeout`, `m2`, `m3`, `m4`, `m5`; M5 recorded
2026-09-19: 19/19 steps, `phase5_gate.sh --ci` 8 gates, 1 807 lib tests green, clippy/fmt clean —
see `docs/m5-acceptance.md` §9 for the post-acceptance correction: `tools/check-deps.sh` fails at
the `m5` tree, `mcp -> adapters-pdf` edge; Wave 0 of Phase 6 fixes it by dependency inversion).
Phase 6 (context budget C14 + M6 intake: COM-2..6, formula eval, ODS/ODP revisions, Word interop)
is active — concept `docs/phase6-concept.md`, plan `docs/phase6-plan.md`, live status
`docs/phase6-status.json`. Rust workspace lives in `crates/`; `harness/`, `corpus/`, `docker/`,
`baseline/`, `.github/` hold the M0 Python fidelity harness and supporting material.

## Source of truth
- `project_concept.md` — decisions D1–D27, architecture, verification log. Claims there are marked
  measured/verified; do not add unverified claims. If a decision changes, update the decision log (§16) too.
- `brand.md` — identity rules; `assets/` holds the mark, wordmarks, lockups, generated rasters.
- Name is **always lowercase** `stichos`. Never `Stichos`, never bold the wordmark (use ink vs paper).
- Voice: "proof", "render", "fidelity budget", "renderer provenance"; CLI verbs `inspect render diff apply doctor`
  (21 verbs total at M5: `inspect region render diff doctor serve schema apply edit annotate revert
  journal perf create script update revisions annots review open sign`).

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
Phase 5 (M5) shipped 2026-09-19: tag `m5` on the gate-green commit, `main` + `m5` pushed.
Records: `docs/phase5-plan.md`, `docs/phase5-status.json`, `docs/m5-acceptance.md` (§9 has the
post-acceptance correction — `tools/check-deps.sh` fails at the `m5` tree on the `mcp ->
adapters-pdf` edge; Phase 6 Wave 0 fixes it by dependency inversion, milestone tags still move
only to committed gate-green trees).
Phase 6 (M6) is active: `docs/phase6-concept.md`, `docs/phase6-plan.md`,
live status `docs/phase6-status.json`. M6 intake (COM-2..6 impl on the now-available Windows host,
xlsx formula eval ADR + minimal subset, ODS/ODP revisions, Word-interop verification, launch
rehearsal only per D26) is listed in phase6-concept §6. Phase-4 records: `docs/phase4-plan.md`,
`docs/phase4-status.json`, `docs/m4-acceptance.md` (Git-State section
has the tag-re-anchoring correction — milestone tags move only to committed gate-green trees).
Phase-3 records: `docs/phase3-plan.md`,
`docs/phase3-status.json`, `docs/m3-acceptance.md` (§7 has post-acceptance corrections).

## Phase-5 tech

9 Rust crates in the workspace (`crates/`):

| Crate | Purpose |
|-------|---------|
| `gp-core` | Core types, IR, diagnostics, traits, content-addressed store, PDF annotation model (C12), signature integrity (C13) |
| `adapters-ooxml` | OOXML (docx, xlsx, pptx), OPC, XML parsing, revision read/write (C11) |
| `adapters-odf` | ODF (odt, ods, odp), manifest, XML parsing, revision read/write |
| `adapters-pdf` | PDF parsing (parser, text, form, annot, outline, metadata, qpdf), incremental annot write-back |
| `engines` | Render engine abstraction (libreoffice, pandoc, chromium, typst live; ms-office COM design + compile-only skeleton) |
| `lint` | Fidelity linting (budget, ssim, contrast, font, diff, cropper) |
| `mcp` | MCP protocol (jsonrpc, dispatch, server, session/review incl. text-layer routes, tools, transport, http, spec) |
| `cli` | `stichos` binary — 21 verbs (inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, annots, review, open, sign) |
| `adapters-ooxfuzz` | Fuzzing harness for OOXML adapters (dev-only) |

Engines are spawned subprocesses, never linked; zero hard external deps.
