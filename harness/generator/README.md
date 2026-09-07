# harness/generator — synthetic fixture generator (HAR-02)

Builds the generated corpus (budget in `corpus/PLAN.md §3`, Σ=127): fixture blobs →
`corpus/blobs/` (gitignored), manifests → `corpus/manifests/` (C9), reference text →
`corpus/reference/`.

## Commands (run from `harness/generator/`)

```
python3 -m gen status                      # budget vs builders vs built
python3 -m gen build [--format F] [--category C]
python3 -m gen verify                      # schema + sha + feature recount + rebuild determinism
```

## Rules for builders (contract)

- Register with `@builder(category, fmt)`; signature `fn(ctx: Ctx, nnn: int) -> BuildResult`.
  Put docx/xlsx/pptx builders in the matching module, ODF trio in `odf.py`.
- **Deterministic bytes:** no wall-clock (pin props via `pin_docx_props/pin_xlsx_props/
  pin_pptx_props` and `core.FIXED_DT`; ODF `meta.xml` dates = `2026-01-01T00:00:00`),
  randomness only from `ctx.rng` (seeded by fixture id), no iteration over unordered
  sets/dicts when writing content. `gen verify` rebuilds and fails on sha drift.
- `reference_text` = the exact strings inserted (C9 §7 + §10.6 CJK per-char F1 rule applies
  downstream); `None` only when the fixture has no text truth (e.g. scanned image page).
- `features{}` is filled by the framework from `metrics.feature_counts` (normative, C9 §6) —
  builders never set it.
- `notes` must state honestly what is real feature use vs synthetic emulation
  (e.g. "synthetic diagramData part, not MS-generated SmartArt").
- Hostile-safe intent: fixture sizes stay small (≤ ~15 MB) except where the category is the
  size itself (`huge-cell-text`, `long-deck-300`).

## Deps (harness-only, never shipped)

python-docx, openpyxl, python-pptx, jsonschema, Pillow, numpy, scikit-image, pytest —
all MIT/Apache/BSD/ISC class; ODF is hand-written zip+XML (odfpy is GPLv2 — deliberately
unused).

## Container validation

`gen build` does not render. Post-generation conversion check per fixture
(`soffice --convert-to pdf` + `pdftoppm`) runs via `tools/convert_check.sh` in the pinned
image `stichos-harness:26.2.5` (see `docs/phase0-execution.md`, SA-5v).
