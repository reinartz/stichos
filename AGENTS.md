# stichos

Phases 0–6 complete and gated (tags `m1-closeout`, `m2`, `m3`, `m4`, `m5`, `m6`; M6 recorded
2026-09-23: 35/35 steps, `phase6_gate.sh --ci` 8 gates, `check-deps.sh` green — Wave 0 removed the
`mcp -> adapters-pdf` edge by dependency inversion). **M6 closeout is pending**: see
`docs/m6-acceptance.md` §11 corrections (2026-09-23 audit — C7 v1.6 spec-test ceilings red after the
Gate-C defaults flip, `__budget` envelope not yet stamped on 100 % of responses, gate checks
1b/1d/1e/1f pass in both branches) and the task breakdown in `docs/remaining-work.md`.
`main` (8 commits) and tag `m6` are **local only — not pushed**.
Rust workspace lives in `crates/`; `harness/`, `corpus/`, `docker/`, `baseline/`, `.github/` hold
the M0 Python fidelity harness and supporting material.

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
Phase 6 (M6) shipped 2026-09-23: 35/35 steps, `phase6_gate.sh --ci` green pre-tag (86/88
post-tag: the `m6`-tag-exists check is an expected artifact, see m6-acceptance "Gate post-tag
note"); tag `m6` on `23db42b`. Records: `docs/phase6-concept.md`, `docs/phase6-plan.md`,
`docs/phase6-status.json`, `docs/m6-acceptance.md` (§11 has the 2026-09-23 audit corrections —
the Gate-C defaults flip is **uncommitted** in the working tree, it left 3 `mcp` spec tests red
(1.5.0 ceilings), the `__budget` envelope is only stamped in `cli/src/image_output.rs` (not on
inspect/apply/diff/journal or the MCP surface), and gate checks 1b/1d/1e/1f pass in both
branches). Follow-ups tracked in `docs/remaining-work.md`. **`main` and `m6` are not pushed.**
Phase-4 records: `docs/phase4-plan.md`, `docs/phase4-status.json`, `docs/m4-acceptance.md`
(Git-State section has the tag-re-anchoring correction — milestone tags move only to committed
gate-green trees).
Phase-3 records: `docs/phase3-plan.md`,
`docs/phase3-status.json`, `docs/m3-acceptance.md` (§7 has post-acceptance corrections).

## Phase-5 tech

9 Rust crates in the workspace (`crates/`):

| Crate | Purpose |
|-------|---------|
| `gp-core` | Core types, IR, diagnostics, traits, content-addressed store, PDF annotation model (C12), signature integrity (C13), context-budget envelope (C14: `envelope.rs`, `tempfiles.rs`), formula evaluator (C15: `formula/`), `ir_convert` |
| `adapters-ooxml` | OOXML (docx, xlsx, pptx), OPC, XML parsing, revision read/write (C11) |
| `adapters-odf` | ODF (odt, ods, odp), manifest, XML parsing, revision read/write |
| `adapters-pdf` | PDF parsing (parser, text, form, annot, outline, metadata, qpdf), incremental annot write-back |
| `engines` | Render engine abstraction (libreoffice, pandoc, chromium, typst live; ms-office COM on Windows host via logon-agent model) |
| `lint` | Fidelity linting (budget, ssim, contrast, font, diff, cropper) |
| `mcp` | MCP protocol (jsonrpc, dispatch, server, session/review incl. text-layer routes, tools, transport, http, spec) |
| `cli` | `stichos` binary — 21 verbs (inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, annots, review, open, sign) |
| `adapters-ooxfuzz` | Fuzzing harness for OOXML adapters (dev-only) |

Engines are spawned subprocesses, never linked; zero hard external deps.
