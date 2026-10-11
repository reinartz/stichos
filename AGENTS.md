# stichos

Phases 0–6 complete and gated (tags `m1-closeout`..`m6`). Phase-7 Tracks A′/B/C/D complete
(A′ closeout, B except P7-3 C/D + P7-9, C engine completion, D CI green). **Measured at clean HEAD:**
`cargo test --workspace --lib` **2244 pass / 9 known fails**
(9 date-hardcoded gp-core formula), `cargo test -p mcp --lib` **93/93**, fmt + clippy
(all-targets, `-D warnings`) + `check-deps.sh` green. p606_ctx06_compliance **6/6**.
Remaining work = **Tracks E/F/G + owner items** in `docs/remaining-work.md`. P7-7 CMS signing
committed (`7d56af5`, `fe266cd`), OQ-C12 confirmed (`6c1ba86`), P7-5 committed (`3b4a396`);
ADR-0012/0013 accepted. **Track H** (generic Office handling) **proposed but not adopted** (owner
declined H0 2026-10-01); proposed code preserved on branch `track-h-proposed`.
Rust workspace lives in `crates/`; `harness/`, `corpus/`, `docker/`, `baseline/`, `.github/` hold
the M0 Python fidelity harness and supporting material.

## Source of truth
- `project_concept.md` — decisions D1–D27, architecture, verification log. Claims there are marked
  measured/verified; do not add unverified claims. If a decision changes, update the decision log (§16) too.
- `brand.md` — identity rules; `assets/` holds the mark, wordmarks, lockups, generated rasters.
- Name is **always lowercase** `stichos`. Never `Stichos`, never bold the wordmark (use ink vs paper).
- Voice: "proof", "render", "fidelity budget", "renderer provenance"; CLI verbs `inspect render diff apply doctor`
   (24 verbs: 22 M5 verbs + `verify` + `validate` + `finish`):
   `inspect region render diff doctor serve schema apply edit annotate revert journal perf create
   script update revisions annots review open sign verify validate finish`.

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
Phase 6 (M6) shipped 2026-09-23: 35/35 steps; closeout round 1 (Gate-C flip + T-A freeze
consistency + T-B envelope on all CLI verbs/MCP serve boundary + T-C gate hardening) committed in
`ff80f8e`; tag `m6` re-anchored to `ff80f8e`; **`main` + `m6` pushed** 2026-09-24. Records:
`docs/phase6-concept.md`, `docs/phase6-plan.md`, `docs/phase6-status.json`, `docs/m6-acceptance.md`
(§11 = 2026-09-23 audit corrections, §12 = 2026-09-24 closeout-round-1 status, §13 =
closeout-round-2). **Closeout round 2 (Track A′) complete** — `phase6_gate.sh --ci` **90/90**
at tag time, `p606_ctx06_compliance` **6/6**. Phase-7 Track B mostly complete
(P7-1/2/4/5/6/8 and B0 done; P7-3 tranche C/D deferred; P7-9 launch owner-only). Tracks C
(engine completion) and D (CI green) complete; see the header block for 2026-09-29 re-measured
gate/test counts. **Track H** (generic Office handling, `docs/track-h-generic-office-handling.md`)
proposed 2026-09-29; owner declined H0 adoption 2026-10-01 — code uncommitted in the working tree,
closeout H9 not performed; at-tree regressions logged in `docs/remaining-work.md` rev 6.
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
| `cli` | `stichos` binary — 24 verbs (inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, annots, review, open, sign, verify, validate, finish) |
| `adapters-ooxfuzz` | Fuzzing harness for OOXML adapters (dev-only) |

Engines are spawned subprocesses, never linked; zero hard external deps.
