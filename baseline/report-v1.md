# Baseline report v1 (HAR-06)

**Date:** 2026-09-06 · **Corpus:** 151 fixtures · **Data:** `baseline/baseline-raw.csv` · **Machine-readable:** `baseline/report-v1.json`

## Methodology

- Environment: pinned image `stichos-harness:26.2.5` (id `sha256:ded77b6fce12…`), LibreOffice **26.2.5.2**, poppler 24.02.0-1ubuntu9.9, `fontset_hash` `sha256:6f26ae4f9bdea927cffa0984d48adcb3e4b60b3fbe263c6a44440a5628927509`; host `worker` (7.1.5-201.fc44.x86_64, python 3.14.6), **pandoc 3.7.0.2**.
- **soffice round-trip** (container, blobs mounted read-only, all work in `/tmp`, outputs via `docker cp`): fixture → own format (`docx:MS Word 2007 XML` / `xlsx:Calc MS Excel 2007 XML` / `pptx:Impress MS PowerPoint 2007 XML` / `odt:writer8` / `ods:calc8` / `odp:impress8`), then round-trip → PDF for page counts. `metrics.feature_counts` before/after → per-feature retention map + PASS (C9 §6: PASS = no tracked key changed, ref-impl rule 11).
- **F1(soffice)** = `metrics.text_f1(pdftotext -enc UTF-8 <golden pdf> -, corpus/reference/<id>.txt)` — golden PDFs reused from `goldens/staging/` (HAR-05 stability run, LO 26.2.5.2); no re-render here. C9 §7 tokenization incl. CJK per-char rule.
- **F1(pandoc)** = host pandoc `-t plain --wrap=none` vs the same reference. **Measured coverage (pandoc 3.7.0.2): readers exist only for docx and odt (61/151 fixtures).** pptx was attempted per brief and fails `rc=21 Unknown input format pptx` (the pptx reader was removed in pandoc 3.0; absent from `pandoc --list-input-formats`); odp verified unsupported (`pandoc -f odp` → "Unknown input format odp"); xlsx/ods have never had readers → recorded `no-pandoc-reader`.
- **pdftoppm**: timed `pdftoppm -r 150 -gray -png` on the golden PDF inside the pinned container (pages counted from output PNGs).
- Reproduce: `python3 harness/baseline/baseline.py run --jobs 4` (resumable; ids already in `baseline-raw.csv` are skipped), then `python3 harness/baseline/baseline.py report`.

## Headline aggregates

| measure | value |
|---|---|
| fixtures measured | 151 |
| round-trip ok / attempted | 151 / 151 |
| retention pass / measured | 123 / 151 (0.815) |
| mean text F1 — soffice (golden PDF text layer) | **0.7017** (min 0.0000, <0.9: 80; excl. odp artifact: 0.8082) |
| mean text F1 — pandoc (supported readers only) | **0.9088** (min 0.3789, <0.9: 19) |
| page-delta Δ=0 | 151/151 — hist {0: 151} |
| round-trip fails | none |
| pdftoppm 150dpi gray (s) | median 0.072, p95 1.249, max 20.377 |
| soffice round-trip (s) | median 0.812, p95 1.014, max 2.142 |

## Rollup by category

| category | n | F1 soffice (mean) | F1 pandoc (mean) | retention pass | rt fails | Δpages=0 | pdftoppm med (s) |
|---|---:|---:|---:|---:|---:|---:|---:|
| cjk-text | 10 | 0.9130 | 1.0000 | 8/10 | 0 | 10/10 | 0.069 |
| comments-annotations | 10 | 0.3858 | 0.6898 | 8/10 | 0 | 10/10 | 0.064 |
| conditional-formatting | 4 | 0.8841 | — | 4/4 | 0 | 4/4 | 0.060 |
| custom-styles | 10 | 0.7070 | 1.0000 | 8/10 | 0 | 10/10 | 0.064 |
| embedded-ole | 10 | 0.7285 | 1.0000 | 4/10 | 0 | 10/10 | 0.070 |
| footnotes-endnotes | 4 | 0.9134 | 0.9134 | 4/4 | 0 | 4/4 | 0.085 |
| formulas-cached | 6 | 0.6516 | — | 6/6 | 0 | 6/6 | 0.059 |
| headers-fields | 4 | 0.9960 | 1.0000 | 4/4 | 0 | 4/4 | 1.153 |
| huge-cell-text | 6 | 0.5882 | 0.9998 | 6/6 | 0 | 6/6 | 2.992 |
| image-textwrap | 7 | 0.7128 | 0.9939 | 5/7 | 0 | 7/7 | 0.087 |
| long-deck-300 | 4 | 0.2000 | — | 2/4 | 0 | 4/4 | 18.283 |
| long-tables | 6 | 0.9988 | 0.8576 | 6/6 | 0 | 6/6 | 0.472 |
| merged-cells | 10 | 0.9284 | 0.5359 | 10/10 | 0 | 10/10 | 0.067 |
| numbering-restarts | 8 | 0.5777 | 0.8688 | 6/8 | 0 | 8/8 | 0.079 |
| protected-readonly | 8 | 0.7245 | 1.0000 | 6/8 | 0 | 8/8 | 0.066 |
| rtl-sheets | 4 | 0.1500 | — | 4/4 | 0 | 4/4 | 0.088 |
| rtl-text | 8 | 0.3328 | 0.9847 | 6/8 | 0 | 8/8 | 0.069 |
| scanned-ocr | 6 | 0.6667 | 0.9161 | 4/6 | 0 | 6/6 | 0.166 |
| smartart-drawing | 5 | 0.8955 | 0.8000 | 5/5 | 0 | 5/5 | 0.075 |
| toc-fields | 4 | 0.9414 | 0.8914 | 2/4 | 0 | 4/4 | 0.081 |
| tracked-changes | 7 | 0.8492 | 0.8679 | 7/7 | 0 | 7/7 | 0.062 |
| unicode-edge | 10 | 0.7073 | 1.0000 | 8/10 | 0 | 10/10 | 0.074 |

## Rollup by format

| format | n | F1 soffice (mean) | F1 pandoc (mean) | retention pass | rt fails | Δpages=0 | pdftoppm med (s) |
|---|---:|---:|---:|---:|---:|---:|---:|
| docx | 35 | 0.9175 | 0.8956 | 31/35 | 0 | 35/35 | 0.071 |
| odp | 22 | 0.0773 | — | 0/22 | 0 | 22/22 | 0.060 |
| ods | 21 | 0.5593 | — | 19/21 | 0 | 21/21 | 0.067 |
| odt | 26 | 0.8854 | 0.9265 | 26/26 | 0 | 26/26 | 0.073 |
| pptx | 26 | 0.8127 | — | 26/26 | 0 | 26/26 | 0.086 |
| xlsx | 21 | 0.7738 | — | 21/21 | 0 | 21/21 | 0.062 |

## Per-fixture table

*soffice→pdf pages = golden page count; F1(pandoc) `no-pandoc-reader` = format not readable by pandoc (see methodology).*

| id | soffice→pdf pages | rt ok | rt retention pass | F1(soffice) | F1(pandoc) | pdftoppm s |
|---|---:|---|---|---:|---:|---:|
| cjk-text-docx-001 | 1 | ok | pass | 1.0000 | 1.0000 | 0.070 |
| cjk-text-docx-002 | 1 | ok | pass | 1.0000 | 1.0000 | 0.067 |
| cjk-text-odp-001 | 1 | ok | **FAIL** | 0.6667 | *no-pandoc-reader* | 0.068 |
| cjk-text-odp-002 | 1 | ok | **FAIL** | 0.6552 | *no-pandoc-reader* | 0.071 |
| cjk-text-ods-001 | 1 | ok | pass | 0.9114 | *no-pandoc-reader* | 0.067 |
| cjk-text-odt-001 | 1 | ok | pass | 1.0000 | 1.0000 | 0.072 |
| cjk-text-odt-002 | 1 | ok | pass | 1.0000 | 1.0000 | 0.061 |
| cjk-text-pptx-001 | 1 | ok | pass | 0.9444 | *fail-rc21* | 0.083 |
| cjk-text-pptx-002 | 1 | ok | pass | 0.9524 | *fail-rc21* | 0.085 |
| cjk-text-xlsx-001 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.064 |
| comments-annotations-docx-001 | 1 | ok | pass | 0.6957 | 0.6957 | 0.066 |
| comments-annotations-docx-002 | 1 | ok | pass | 0.6847 | 0.6847 | 0.074 |
| comments-annotations-odp-001 | 1 | ok | **FAIL** | 0.0488 | *no-pandoc-reader* | 0.059 |
| comments-annotations-odp-002 | 1 | ok | **FAIL** | 0.0377 | *no-pandoc-reader* | 0.060 |
| comments-annotations-ods-001 | 1 | ok | pass | 0.2222 | *no-pandoc-reader* | 0.059 |
| comments-annotations-ods-002 | 1 | ok | pass | 0.2051 | *no-pandoc-reader* | 0.062 |
| comments-annotations-odt-001 | 1 | ok | pass | 0.6889 | 0.6889 | 0.071 |
| comments-annotations-pptx-001 | 1 | ok | pass | 0.4615 | *fail-rc21* | 0.075 |
| comments-annotations-pptx-002 | 1 | ok | pass | 0.4800 | *fail-rc21* | 0.079 |
| comments-annotations-xlsx-001 | 1 | ok | pass | 0.3333 | *no-pandoc-reader* | 0.057 |
| conditional-formatting-ods-001 | 1 | ok | pass | 0.8696 | *no-pandoc-reader* | 0.058 |
| conditional-formatting-ods-002 | 1 | ok | pass | 0.6667 | *no-pandoc-reader* | 0.060 |
| conditional-formatting-xlsx-001 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.063 |
| conditional-formatting-xlsx-002 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.061 |
| custom-styles-docx-001 | 1 | ok | pass | 1.0000 | 1.0000 | 0.064 |
| custom-styles-docx-002 | 1 | ok | pass | 1.0000 | 1.0000 | 0.064 |
| custom-styles-odp-001 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.061 |
| custom-styles-odp-002 | 1 | ok | **FAIL** | 0.0370 | *no-pandoc-reader* | 0.059 |
| custom-styles-ods-001 | 1 | ok | pass | 0.5333 | *no-pandoc-reader* | 0.062 |
| custom-styles-odt-001 | 1 | ok | pass | 1.0000 | 1.0000 | 0.070 |
| custom-styles-odt-002 | 1 | ok | pass | 1.0000 | 1.0000 | 0.070 |
| custom-styles-pptx-001 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.083 |
| custom-styles-pptx-002 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.089 |
| custom-styles-xlsx-001 | 1 | ok | pass | 0.5000 | *no-pandoc-reader* | 0.062 |
| embedded-ole-docx-001 | 1 | ok | **FAIL** | 1.0000 | 1.0000 | 0.066 |
| embedded-ole-docx-002 | 1 | ok | **FAIL** | 1.0000 | 1.0000 | 0.064 |
| embedded-ole-odp-001 | 2 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.102 |
| embedded-ole-odp-002 | 2 | ok | **FAIL** | 0.0488 | *no-pandoc-reader* | 0.104 |
| embedded-ole-ods-001 | 1 | ok | **FAIL** | 0.6452 | *no-pandoc-reader* | 0.064 |
| embedded-ole-ods-002 | 1 | ok | **FAIL** | 0.8182 | *no-pandoc-reader* | 0.067 |
| embedded-ole-odt-001 | 1 | ok | pass | 0.9730 | 1.0000 | 0.072 |
| embedded-ole-pptx-001 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.082 |
| embedded-ole-pptx-002 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.081 |
| embedded-ole-xlsx-001 | 1 | ok | pass | 0.8000 | *no-pandoc-reader* | 0.058 |
| footnotes-endnotes-docx-001 | 1 | ok | pass | 0.9045 | 0.9045 | 0.091 |
| footnotes-endnotes-docx-002 | 1 | ok | pass | 0.9091 | 0.9091 | 0.071 |
| footnotes-endnotes-odt-001 | 1 | ok | pass | 0.9189 | 0.9189 | 0.080 |
| footnotes-endnotes-odt-002 | 2 | ok | pass | 0.9211 | 0.9211 | 0.113 |
| formulas-cached-ods-001 | 1 | ok | pass | 0.5882 | *no-pandoc-reader* | 0.061 |
| formulas-cached-ods-002 | 2 | ok | pass | 0.2857 | *no-pandoc-reader* | 0.106 |
| formulas-cached-ods-003 | 1 | ok | pass | 0.2857 | *no-pandoc-reader* | 0.058 |
| formulas-cached-xlsx-001 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.058 |
| formulas-cached-xlsx-002 | 2 | ok | pass | 0.8750 | *no-pandoc-reader* | 0.104 |
| formulas-cached-xlsx-003 | 1 | ok | pass | 0.8750 | *no-pandoc-reader* | 0.058 |
| headers-fields-docx-001 | 6 | ok | pass | 0.9940 | 1.0000 | 1.235 |
| headers-fields-docx-002 | 6 | ok | pass | 0.9910 | 1.0000 | 1.249 |
| headers-fields-odt-001 | 4 | ok | pass | 0.9990 | 1.0000 | 1.062 |
| headers-fields-odt-002 | 4 | ok | pass | 1.0000 | 1.0000 | 1.070 |
| huge-cell-text-docx-001 | 23 | ok | pass | 1.0000 | 0.9998 | 4.449 |
| huge-cell-text-ods-001 | 18 | ok | pass | 0.0047 | *no-pandoc-reader* | 0.854 |
| huge-cell-text-ods-002 | 16 | ok | pass | 0.8178 | *no-pandoc-reader* | 2.819 |
| huge-cell-text-odt-001 | 17 | ok | pass | 1.0000 | 0.9998 | 3.764 |
| huge-cell-text-xlsx-001 | 1 | ok | pass | 0.0007 | *no-pandoc-reader* | 0.058 |
| huge-cell-text-xlsx-002 | 22 | ok | pass | 0.7059 | *no-pandoc-reader* | 3.164 |
| image-textwrap-docx-001 | 1 | ok | pass | 1.0000 | 0.9945 | 0.102 |
| image-textwrap-docx-002 | 1 | ok | pass | 1.0000 | 0.9972 | 0.143 |
| image-textwrap-odp-001 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.063 |
| image-textwrap-odp-002 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.060 |
| image-textwrap-odt-001 | 1 | ok | pass | 0.9899 | 0.9899 | 0.120 |
| image-textwrap-pptx-001 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.077 |
| image-textwrap-pptx-002 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.087 |
| long-deck-300-odp-001 | 300 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 16.417 |
| long-deck-300-odp-002 | 300 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 14.525 |
| long-deck-300-pptx-001 | 300 | ok | pass | 0.4000 | *fail-rc21* | 20.149 |
| long-deck-300-pptx-002 | 300 | ok | pass | 0.4000 | *fail-rc21* | 20.377 |
| long-tables-docx-001 | 2 | ok | pass | 1.0000 | 0.9901 | 0.403 |
| long-tables-docx-002 | 2 | ok | pass | 1.0000 | 0.5907 | 0.409 |
| long-tables-ods-001 | 3 | ok | pass | 0.9926 | *no-pandoc-reader* | 0.529 |
| long-tables-odt-001 | 3 | ok | pass | 1.0000 | 0.9920 | 0.415 |
| long-tables-xlsx-001 | 5 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.837 |
| long-tables-xlsx-002 | 5 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.848 |
| merged-cells-docx-001 | 1 | ok | pass | 1.0000 | 0.4595 | 0.064 |
| merged-cells-docx-002 | 1 | ok | pass | 1.0000 | 0.3789 | 0.069 |
| merged-cells-ods-001 | 1 | ok | pass | 0.8235 | *no-pandoc-reader* | 0.065 |
| merged-cells-ods-002 | 2 | ok | pass | 0.5714 | *no-pandoc-reader* | 0.117 |
| merged-cells-odt-001 | 1 | ok | pass | 0.8889 | 0.7692 | 0.059 |
| merged-cells-pptx-001 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.086 |
| merged-cells-pptx-002 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.072 |
| merged-cells-xlsx-001 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.061 |
| merged-cells-xlsx-002 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.064 |
| merged-cells-xlsx-003 | 2 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.109 |
| numbering-restarts-docx-001 | 1 | ok | pass | 0.8684 | 0.8684 | 0.080 |
| numbering-restarts-docx-002 | 1 | ok | pass | 0.8675 | 0.8675 | 0.078 |
| numbering-restarts-odp-001 | 1 | ok | **FAIL** | 0.0385 | *no-pandoc-reader* | 0.065 |
| numbering-restarts-odp-002 | 1 | ok | **FAIL** | 0.0385 | *no-pandoc-reader* | 0.057 |
| numbering-restarts-odt-001 | 1 | ok | pass | 0.6970 | 0.8684 | 0.079 |
| numbering-restarts-odt-002 | 1 | ok | pass | 0.7037 | 0.8710 | 0.072 |
| numbering-restarts-pptx-001 | 1 | ok | pass | 0.7000 | *fail-rc21* | 0.087 |
| numbering-restarts-pptx-002 | 1 | ok | pass | 0.7083 | *fail-rc21* | 0.089 |
| protected-readonly-docx-001 | 1 | ok | pass | 1.0000 | 1.0000 | 0.065 |
| protected-readonly-odp-001 | 1 | ok | **FAIL** | 0.1290 | *no-pandoc-reader* | 0.064 |
| protected-readonly-odp-002 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.056 |
| protected-readonly-ods-001 | 1 | ok | pass | 0.6667 | *no-pandoc-reader* | 0.067 |
| protected-readonly-odt-001 | 1 | ok | pass | 1.0000 | 1.0000 | 0.068 |
| protected-readonly-pptx-001 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.090 |
| protected-readonly-pptx-002 | 2 | ok | pass | 1.0000 | *fail-rc21* | 0.133 |
| protected-readonly-xlsx-001 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.059 |
| rtl-sheets-ods-001 | 1 | ok | pass | 0.0000 | *no-pandoc-reader* | 0.067 |
| rtl-sheets-ods-002 | 2 | ok | pass | 0.2667 | *no-pandoc-reader* | 0.118 |
| rtl-sheets-xlsx-001 | 1 | ok | pass | 0.0000 | *no-pandoc-reader* | 0.064 |
| rtl-sheets-xlsx-002 | 2 | ok | pass | 0.3333 | *no-pandoc-reader* | 0.109 |
| rtl-text-docx-001 | 1 | ok | pass | 0.2093 | 1.0000 | 0.064 |
| rtl-text-docx-002 | 1 | ok | pass | 0.4167 | 1.0000 | 0.070 |
| rtl-text-odp-001 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.057 |
| rtl-text-odp-002 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.057 |
| rtl-text-odt-001 | 1 | ok | pass | 0.6250 | 1.0000 | 0.073 |
| rtl-text-odt-002 | 1 | ok | pass | 0.6122 | 0.9388 | 0.068 |
| rtl-text-pptx-001 | 1 | ok | pass | 0.4146 | *fail-rc21* | 0.090 |
| rtl-text-pptx-002 | 1 | ok | pass | 0.3846 | *fail-rc21* | 0.082 |
| scanned-ocr-docx-001 | 1 | ok | pass | 1.0000 | 0.9231 | 0.719 |
| scanned-ocr-odp-001 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.054 |
| scanned-ocr-odp-002 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.054 |
| scanned-ocr-odt-001 | 1 | ok | pass | 1.0000 | 0.9091 | 0.745 |
| scanned-ocr-pptx-001 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.175 |
| scanned-ocr-pptx-002 | 1 | ok | pass | 1.0000 | *fail-rc21* | 0.158 |
| smartart-drawing-docx-001 | 1 | ok | pass | 1.0000 | 0.7500 | 0.066 |
| smartart-drawing-docx-002 | 1 | ok | pass | 1.0000 | 0.8000 | 0.068 |
| smartart-drawing-docx-003 | 1 | ok | pass | 1.0000 | 0.8500 | 0.100 |
| smartart-drawing-pptx-001 | 1 | ok | pass | 0.7500 | *fail-rc21* | 0.076 |
| smartart-drawing-pptx-002 | 1 | ok | pass | 0.7273 | *fail-rc21* | 0.075 |
| toc-fields-docx-001 | 1 | ok | **FAIL** | 1.0000 | 0.9000 | 0.078 |
| toc-fields-docx-002 | 1 | ok | **FAIL** | 1.0000 | 0.9000 | 0.079 |
| toc-fields-odt-001 | 1 | ok | pass | 0.8710 | 0.8710 | 0.082 |
| toc-fields-odt-002 | 1 | ok | pass | 0.8947 | 0.8947 | 0.087 |
| tracked-changes-docx-001 | 1 | ok | pass | 0.9167 | 1.0000 | 0.062 |
| tracked-changes-docx-002 | 1 | ok | pass | 0.9375 | 1.0000 | 0.065 |
| tracked-changes-docx-003 | 1 | ok | pass | 0.9375 | 0.8824 | 0.062 |
| tracked-changes-ods-001 | 1 | ok | pass | 0.6957 | *no-pandoc-reader* | 0.062 |
| tracked-changes-odt-001 | 1 | ok | pass | 0.6923 | 0.6923 | 0.062 |
| tracked-changes-odt-002 | 1 | ok | pass | 0.7647 | 0.7647 | 0.061 |
| tracked-changes-xlsx-001 | 1 | ok | pass | 1.0000 | *no-pandoc-reader* | 0.056 |
| unicode-edge-docx-001 | 1 | ok | pass | 0.9589 | 1.0000 | 0.073 |
| unicode-edge-docx-002 | 1 | ok | pass | 0.8219 | 1.0000 | 0.074 |
| unicode-edge-odp-001 | 1 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.052 |
| unicode-edge-odp-002 | 2 | ok | **FAIL** | 0.0000 | *no-pandoc-reader* | 0.087 |
| unicode-edge-ods-001 | 1 | ok | pass | 0.8759 | *no-pandoc-reader* | 0.090 |
| unicode-edge-odt-001 | 1 | ok | pass | 0.9589 | 1.0000 | 0.074 |
| unicode-edge-odt-002 | 1 | ok | pass | 0.8219 | 1.0000 | 0.073 |
| unicode-edge-pptx-001 | 1 | ok | pass | 0.9863 | *fail-rc21* | 0.095 |
| unicode-edge-pptx-002 | 1 | ok | pass | 0.8219 | *fail-rc21* | 0.096 |
| unicode-edge-xlsx-001 | 1 | ok | pass | 0.8276 | *no-pandoc-reader* | 0.061 |

## Interpretation for the library-decision table

- **LO round-trip is the bar to beat, not the gold standard.** 151/151 round-trips completed; tracked feature counts changed in 28/151 fixtures (retention pass rate 0.815). C9 §6 is exact-match, so every changed key is a report entry — and each mechanism below is a place stichos's own codec must be provably better than LO:
  - *frame re-wrap* ×22 (all odp): LO adds a `draw:frame` per round-trip (typically 2→3; `long-deck-300-odp-001` 600→900) — representation churn, not content loss.
  - *field representation rewrite* ×2 (`toc-fields-docx-001/002`): `w:fldSimple` 1→0 while `w:instrText` 0→1 — the same field becomes a complex field; semantically preserved, C9 §6-exact-match broken. Candidate for a §10 amendment discussion.
  - *true feature loss*: `comments-annotations-odp-001` — `office:annotation` 3→0 (LO **drops slide annotations** on odp round-trip).
  - *true feature loss*: `comments-annotations-odp-002` — `office:annotation` 4→0 (LO **drops slide annotations** on odp round-trip).
  - *true feature loss*: `embedded-ole-ods-001` — `draw:frame` 1→0 (OLE-object frame **disappears** on ods round-trip).
  - *true feature loss*: `embedded-ole-ods-002` — `draw:frame` 1→0 (OLE-object frame **disappears** on ods round-trip).
  Priority for our codec conformance tests: ODF annotations, OLE object frames, field representation stability.
- **odp text-F1 is a golden-pipeline artifact, not a content gap** (measured): all 22 odp golden PDFs carry a one-char-per-line text layer (`pdftotext` of `custom-styles-odp-001` yields `B\no\nd\ny…`), collapsing F1 to 0.077 mean while CJK fixtures score higher (per-char tokenization happens to match). Excluding odp, mean F1(soffice) is **0.8082**. Decision: `pdftotext` F1 is not a usable golden-text metric for odp at LO 26.2.5 — use adapter extraction (our own) or `-layout`-less alternatives there; SSIM is unaffected.
- **pandoc is not a viable baseline reader across the corpus** (pandoc 3.7.0.2 measured): no-pandoc-reader ×64, pptx ×rc21 (reader removed in pandoc 3.0) ×26 — readers exist only for docx+odt, so text extraction cannot be delegated to pandoc for 90/151 fixtures (xlsx/ods/odp/pptx).
- Lowest F1(pandoc) among supported formats: `merged-cells-docx-002` 0.3789, `merged-cells-docx-001` 0.4595, `long-tables-docx-002` 0.5907, `comments-annotations-docx-002` 0.6847, `comments-annotations-odt-001` 0.6889 — table-merge semantics (merged-cells-docx mean 0.5359 vs soffice 0.9284) and comment text (comments-annotations mean 0.6898): extraction gaps our adapters must close; where pandoc F1 is high (cjk/custom-styles/unicode-edge ≈1.0), LO's PDF text layer is the weaker side instead.
- **RTL is the largest genuine render-side text gap**: rtl-text+rtl-sheets mean F1(soffice) 0.272 vs F1(pandoc) 0.985 — LO's PDF text layer mangles Arabic/Hebrew extraction (`rtl-sheets-xlsx-001`, `rtl-sheets-ods-001` reach 0.0000) while pandoc round-trips the same bytes at ≈1.0. Our text metric must not trust pdftotext for RTL.
- **Scanned/huge fixtures define honest floors**: scanned-ocr mean F1(soffice) 0.667 (raster-only fixtures by design, per GENERATION_REPORT) and huge-cell-text (xlsx/ods `0.0007`/`0.0047`, 32 767-char cells clipped by the page) — golden-text expectations must be per-category, not corpus-wide.
- **Pagination is stable under LO round-trip**: Δpages=0 in 151/151 measured fixtures; non-zero deltas: none — C9 §5 page-delta gating is usable as-is.
- **Raster budget**: pdftoppm @150dpi gray median 0.072 s (p95 1.249, max 20.377 — the 300-slide long-deck fixtures); LO round-trip median 0.812 s (max 2.142) — rasterization is ~10× cheaper than conversion for normal decks, so per-PR SSIM runs fit the budget; long decks need the nightly tier.
