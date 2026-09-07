# HAR-01 — Corpus plan (Phase 0)

Contract: C9 (`docs/contracts/C9.md`), schema `corpus/schema/fixture.schema.json`. Freezes at M0.
Concept §14 Phase 0 nasty-document list; plan §5 Track HAR.

## 1. Targets

| Constraint | Value |
|---|---|
| Total fixtures | ≥150 |
| Generated (HAR-02) | ≥120 — plan allocates **151** (§3, v1.1; was 127 at v1.0) |
| Curated real-world (HAR-01 sourcing) | ≤30 — plan allocates **24** (§5) — ingest outstanding (owner follow-up) |
| Planned total | **151 generated** (v1.1 M0-gate additions, §3) + 24 curated when ingested |
| PDF inputs | **None in Phase 0.** PDF enters at baseline via rendering (HAR-06); `pdf-external` category exists for curated XFA/scanned items only, flagged out-of-scope-for-generation |

## 2. Category taxonomy

ID = kebab-case; fixture ID = `<cat>-<format>-<nnn>` (e.g. `merged-cells-docx-001`).
Formats: `docx xlsx pptx odt ods odp` (+ `pdf` for curated `pdf-external` only).
"gen?" = generator feasibility in HAR-02.

| Category id | Nasty axis (concept §14) | Formats | gen? | Notes |
|---|---|---|---|---|
| `merged-cells` | merged cells | docx xlsx pptx odt ods | yes | horizontal/vertical/diagonal-merge combos |
| `smartart-drawing` | SmartArt / drawings | docx xlsx pptx | yes | OOXML `w:drawing`/`wps:` shapes; odp gets plain `draw:` frames as ODF analogue |
| `embedded-ole` | embedded OLE objects | all 6 | yes | embedded binary parts (PNG-in-container + legacy `*.bin` OLE payload) |
| `tracked-changes` | tracked changes | docx xlsx odt ods | yes | insert/delete/format revisions; `w:ins`/`w:del`, `text:change` |
| `pdf-external` | XFA forms (PDF-only) | pdf | **no** | out-of-scope-for-generation; curated only; §5 |
| `scanned-ocr` | scanned-OCR input | docx odt pptx odp | yes | image-only full-page body (no text layer); reference text = authored OCR ground truth |
| `cjk-text` | CJK text | all 6 | yes | zh/ja/ko; line-break & justification edge cases; Noto fonts (D14) |
| `rtl-text` | RTL text (writer/deck) | docx pptx odt odp | yes | ar/he bidi runs |
| `rtl-sheets` | right-to-left sheets | xlsx ods | yes | sheet-level RTL, reversed column order |
| `long-deck-300` | 300-slide decks | pptx odp | yes | 300+ slides, masters + placeholders; CI-subset-excluded (heavy) |
| `long-tables` | tables spanning pages | docx xlsx odt ods | yes | repeat-header rows, row split across page boundary |
| `footnotes-endnotes` | footnotes/endnotes | docx odt | yes | continuation across pages; id renumbering |
| `headers-fields` | headers/footers with fields | docx odt | yes | PAGE/NUMPAGES/date fields |
| `toc-fields` | TOC fields | docx odt | yes | `w:fldSimple`/`w:instrText` TOC; ODF `text:table-of-content` |
| `image-textwrap` | images with text-wrap | docx odt pptx odp | yes | square/tight/through wrap anchors |
| `formulas-cached` | formulas + cached values | xlsx ods | yes | formula **and** stale cached value; concept §14 Phase 1 |
| `conditional-formatting` | conditional formatting | xlsx ods | yes | color scales, data bars, cell rules |
| `custom-styles` | custom styles | all 6 | yes | style inheritance chains; latent styles |
| `numbering-restarts` | numbering restarts | docx pptx odt odp | yes | restart-at / continue-from-previous |
| `comments-annotations` | comments/annotations | all 6 | yes | `w:commentReference` / `office:annotation` anchored |
| `protected-readonly` | protected/readonly | all 6 | yes | doc protection, sheet lock, final readonly flag |
| `unicode-edge` | unicode edge cases | all 6 | yes | ligature chars, ZWJ/ZWSP/soft-hyphen, surrogates-as-emoji, combining stacks |
| `huge-cell-text` | huge single-cell text | docx xlsx odt ods | yes | ≥64 KB in one cell; overflow + pagination stress |
| `corrupt-zip-bomb` | hostile packages | reserved | no | **SEC-01 corpus (later).** Id reserved, 0 fixtures now |
| `corrupt-entity-bomb` | hostile packages | reserved | no | **SEC-01 corpus (later).** Id reserved, 0 fixtures now |

## 3. Count budget — generated (HAR-02)

Table is **v1.1** = v1.0 (Σ=127) + **M0-gate additions (+24)**, marked ✚ in cells.
Rationale (honest): the M0 exit gate requires the harness to run **≥150 fixtures**; v1.0 shipped
127 generated + 24 curated planned = 151, but the curated-24 ingest (HAR-01 sourcing, §5) remains
an **owner follow-up** (license per-file checks pending, blobs never in git). To reach the gate on
generated fixtures alone, 24 second instances were added in the **under-represented formats** —
pptx 15→26, odp 11→22, ods 19→21 (the three formats with the thinnest per-category coverage).
Every added fixture is a real second instance with **content that differs from `-001`**
(different strings/layout/counts, asserted in the builders; byte-dedup-checked across same-category
fixtures), not a re-hash.

| Category | docx | xlsx | pptx | odt | ods | odp | Σ |
|---|---|---|---|---|---|---|---|
| merged-cells | 2 | 3 | 2 ✚ | 1 | 2 | 0 | 10 |
| smartart-drawing | 3 | 0 | 2 | 0 | 0 | 0 | 5 |
| embedded-ole | 2 | 1 | 2 ✚ | 1 | 2 ✚ | 2 ✚ | 10 |
| tracked-changes | 3 | 1 | 0 | 2 | 1 | 0 | 7 |
| scanned-ocr | 1 | 0 | 2 ✚ | 1 | 0 | 2 ✚ | 6 |
| cjk-text | 2 | 1 | 2 ✚ | 2 | 1 | 2 ✚ | 10 |
| rtl-text | 2 | 0 | 2 ✚ | 2 | 0 | 2 ✚ | 8 |
| rtl-sheets | 0 | 2 | 0 | 0 | 2 | 0 | 4 |
| long-deck-300 | 0 | 0 | 2 | 0 | 0 | 2 ✚ | 4 |
| long-tables | 2 | 2 | 0 | 1 | 1 | 0 | 6 |
| footnotes-endnotes | 2 | 0 | 0 | 2 | 0 | 0 | 4 |
| headers-fields | 2 | 0 | 0 | 2 | 0 | 0 | 4 |
| toc-fields | 2 | 0 | 0 | 2 | 0 | 0 | 4 |
| image-textwrap | 2 | 0 | 2 ✚ | 1 | 0 | 2 ✚ | 7 |
| formulas-cached | 0 | 3 | 0 | 0 | 3 | 0 | 6 |
| conditional-formatting | 0 | 2 | 0 | 0 | 2 | 0 | 4 |
| custom-styles | 2 | 1 | 2 ✚ | 2 | 1 | 2 ✚ | 10 |
| numbering-restarts | 2 | 0 | 2 ✚ | 2 | 0 | 2 ✚ | 8 |
| comments-annotations | 2 | 1 | 2 ✚ | 1 | 2 ✚ | 2 ✚ | 10 |
| protected-readonly | 1 | 1 | 2 ✚ | 1 | 1 | 2 ✚ | 8 |
| unicode-edge | 2 | 1 | 2 ✚ | 2 | 1 | 2 ✚ | 10 |
| huge-cell-text | 1 | 2 | 0 | 1 | 2 | 0 | 6 |
| **Σ** | **35** | **21** | **26** | **26** | **21** | **22** | **151** |

**v1.1 M0-gate additions (+24):** pptx merged-cells/embedded-ole/scanned-ocr/cjk-text/rtl-text/
image-textwrap/custom-styles/numbering-restarts/comments-annotations/protected-readonly/unicode-edge
(11×1→2); odp embedded-ole/scanned-ocr/cjk-text/rtl-text/image-textwrap/custom-styles/
numbering-restarts/comments-annotations/protected-readonly/unicode-edge/long-deck-300 (11×1→2);
ods embedded-ole/comments-annotations (2×1→2). Generated total 151 ≥ 150 gate ✓ without waiting on
curated-24 ingest (which stays planned, §5 — owner follow-up; if ingested later, the total grows
beyond 151). 151 ≥ 120 ✓. `corrupt-*` rows omitted (reserved, count 0). PR-subset (HAR-07) excludes
`long-deck-300` and `huge-cell-text` and is unchanged (all 001 ids).

## 4. Ground truth strategy

Every generated fixture ships, from the generator, in one commit:

| Artifact | Path | Truth source |
|---|---|---|
| expected-features manifest | `corpus/manifests/<id>.json` | generator writes counts it emitted (`features{}`), exact by construction |
| reference text | `corpus/reference/<id>.txt` | authored text pre-serialization; basis for F1 (C9 §5) |
| expected page-count range | `expected.page_count_min/max` in manifest | `null` at generation; filled as **goldens** when HAR-05 approves renders on the pinned image (never guessed) |

Golden approval (human, HAR-05) records `(fixture sha256, engine id+version, dpi, fontset_hash)` — see C9 §2.

## 5. Curated sourcing & license review

Rule: **blobs never in git** (`.gitignore: corpus/blobs/`); manifests + hashes only; blobs live in a
content-addressed external object store. Status: measured = command/fetch + result · researched = URL cited,
not re-verified this pass · unverified = claim to check before ingest. Verdicts: *hash-only* (reference by
hash/URL, no copy) · *external-store* (mirrorable to our store with notices) · *rejected* (do not ingest).

| Source | Cite | License | Status | Verdict |
|---|---|---|---|---|
| LibreOffice core test suite (`sw/qa`, `sc/qa`, `sd/qa` `qa/` data dirs) | libreoffice.org/about-us/licenses/ ; git.libreoffice.org/core | software MPL-2.0 (measured 2026-09-03); test-data files repo-tri-license MPL-2.0+/LGPL-3.0+/GPL-3.0+ | license measured; **per-file status of binary fixtures unverified** | external-store, after per-file provenance check; attribution in manifest |
| OOXML sample files inside ECMA-376 archive | ecma-international.org publications page (measured 2026-09-03: free download exists) | Ecma copyright; free download ≠ redistribution right | redistribution terms unverified | hash-only (record URL + sha256); blobs not mirrored |
| ISO/IEC 29500 (same content, ISO channel) | iso.org | paid, no redistribution | researched | rejected |
| Apache OpenOffice legacy test corpora | openoffice.org (ASF) | Apache-2.0 for repo incl. test docs | researched; per-file unverified | external-store + NOTICE/attribution |
| Open XML SDK sample documents (`dotnet/Open-XML-SDK`) | github.com/dotnet/Open-XML-SDK | MIT | researched | external-store + attribution |
| US federal publications (GPO/GAO reports in docx/odt) | govinfo.gov | public domain, 17 U.S.C. §105 | researched | external-store (PD note) |
| CC0 office files (Wikimedia Commons) | commons.wikimedia.org `Category:CC0` office files | CC0 / CC-BY / CC-BY-SA mixed — per-file check required | researched | external-store; CC-BY-SA copyleft noted per-file |
| Microsoft Office sample templates (microsoft.com) | microsoft.com support/download pages | Microsoft copyright | researched | rejected |
| Bug-report attachments (LO Bugzilla, OOXML trackers) | bugzilla.documentfoundation.org | unspecified license by default | researched | rejected unless explicit grant from author |

Curated budget (≤30): `pdf-external` (XFA) 3 · `smartart-drawing` 4 · `tracked-changes` 3 ·
`merged-cells` 3 · `cjk-text` 3 · `rtl-text` 2 · `long-deck-300` 2 · `unicode-edge` 2 ·
`long-tables` 2 · `formulas-cached` 2 · `scanned-ocr` 2 → **24**, headroom 6.

Every curated manifest must carry `origin.source`, `origin.license`, `origin.url`, and the review
verdict in `notes`; ingest is blocked on missing license fields (schema enforces structure, CI enforces table).

## 6. Storage layout

```
corpus/
├── PLAN.md                      # this file
├── schema/fixture.schema.json   # C9 manifest schema (draft 2020-12)
├── manifests/<id>.json          # one manifest per fixture — IN GIT
├── reference/<id>.txt           # reference text for F1 — IN GIT
└── blobs/                       # GITIGNORED — fixture bytes
```

`corpus/blobs/` is a local cache; canonical addressing is **sha256**: the external object store keys
objects by hash, the manifest's `sha256` field resolves `<id> → object`; a cached file is valid iff its
sha256 matches the manifest (`sha256` stays `null` in git until the blob exists and is hashed).
