# Corpus generation report (HAR-02 / SA-5v)

**Date:** 2026-09-05, updated 2026-09-06 for the v1.1 M0-gate additions ·
**Generator:** `harness/generator` v0.1.0 · **Manifest schema:** C9 0.2.0-draft

| Check | Result |
|---|---|
| Fixtures generated | **151/151** (budget `corpus/PLAN.md §3` v1.1): docx 35 · xlsx 21 · pptx 26 · odt 26 · ods 21 · odp 22 |
| `gen verify` (schema + sha + feature recount + rebuild determinism) | **0 problems** |
| Same-category byte-dedup (`python3 tools/dedup_check.py`) | **OK — all same-category blobs distinct** |
| LO conversion in pinned image `stichos-harness:26.2.5` (LO 26.2.5.2) | **151/151 ok**, median 1 s/fixture, max 4 s |
| Detail | `baseline/convert-check.csv` (id, format, status, pages, seconds) |

Reproduce: `cd harness/generator && python3 -m gen build` then `python3 tools/dedup_check.py`
and `bash tools/convert_check.sh`. Blobs live in `corpus/blobs/` (gitignored); manifests +
reference texts are in git.

## v1.1 M0-gate additions (+24, 2026-09-06)

The M0 exit gate requires the harness to run **≥150 fixtures**. v1.0 shipped 127 generated with
**24 curated fixtures planned** (§5 sourcing) — but **curated-24 ingest is still outstanding**
(owner follow-up: per-file license/provenance checks, blobs never in git). To meet the gate on
generated fixtures alone, 24 second instances (nnn=002) were added in the under-represented
formats (pptx 15→26, odp 11→22, ods 19→21), marked ✚ in PLAN.md §3.

- Every 002 builder emits content that **differs from 001** (different strings, geometry,
  counts, payloads, authors), not a re-hash: block-merge tables, mirrored overlaps, swapped
  bullet/autonum roles, second unicode family, three-script CJK, multi-author comment layers,
  a phase-shifted 300-slide deck, settings flipped false→true; builders assert both the new
  structure and the ABSENCE of 001 content where relevant.
- All 24 converted **ok** in the pinned image on the first pass (ODP annotation lesson honored:
  odp comments stay in `draw:notes`, never in slide frames).
- The 127 pre-existing blobs re-hashed **byte-identical** after the rebuild — goldens,
  convert-check rows and baseline rows for 001 fixtures remain valid (INV-4).
- Baseline extended to 151 (resumable run, new ids only): all 24 new round-trips ok; the odp
  retention failures and odp/pptx F1 patterns of the new instances match the known 001
  mechanisms (LO frame re-wrap on odp, OLE frame drop on ods round-trip, odp golden-text
  extraction artifact) — findings, not generator bugs. See `baseline/report-v1.md`.
- If curated-24 is ingested later, the corpus total grows beyond 151; the §3 budget and this
  report track generated fixtures only.

## Findings baked into fixtures / notes

- **ODP has no slide-comment model** — inline `office:annotation` in a slide text box makes LO
  refuse the package; annotations placed in `draw:notes` (measured 2026-09-05).
- Harness image font set extended (`fonts-noto-cjk` + `fonts-noto-extra`) so cjk/rtl fixtures
  render for real; `fontset_hash` re-pinned (README docker/harness).
- openpyxl overwrites `dcterms:modified` with wall-clock at save → determinism requires the
  `_freeze_modified` pin; ECMA-376 caps a cell string at 32 767 chars (huge-cell-text/xlsx-001).
- Synthetic emulations (honest `notes` in manifests): diagram parts are not MS SmartArt; OLE
  embeds are CFB stubs; xlsx tracked changes are injected `worksheetRevisions` records;
  pptx "protection" is legacy `modifyVerifier`; odp/odt settings.xml readonly are heuristics.
- Unicode-edge fixtures intentionally stress LO glyph coverage — tofu in renders is a **finding**
  for the baseline report, not a generator bug.
- Conversion harness caveat (fixed here): mounts must be container-writable or LO reports a
  false input error (`Io Access Code:7` on uid-mismatched bind mounts).
