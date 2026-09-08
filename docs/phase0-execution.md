# Phase 0 execution runbook (M0)

**Scope:** strict M0 exit gate only (plan §2). Agreed 2026-09-03: no Rust scaffold, no IR work.
**Gate (all must pass):** harness runs ≥150 fixtures × engines · baseline report published (HAR-06) ·
reproducible `fontset_hash` · CI nightly green on 3 platforms · GOV-01..03 name-hygiene sweep done.

**Gate status (2026-09-08):** ≥150 fixtures ✅ (151/151, report-v1) · baseline report ✅ · `fontset_hash` ✅ ·
nightly ×3 ✅ measured green on GitHub, run 34221533557 (2026-09-08): `harness` + `platforms` ubuntu/macos/windows
all success (pinned image + pin gates, corpus rebuild byte-repro vs committed manifests, convert 151/151,
stability sample, verify+pytest ×3) — fixes detailed in the CI bug-fix log below ·
GOV-01/03 research ✅, GOV-01 legal opinion + GOV-02 registrations ⏳ human-bound.

**CI bug-fix log (2026-09-08):** two root causes, both measured.
(1) `harness` byte-repro step: `normalize_zip` wrote ZIP_DEFLATED, so blob sha256 depended on the zlib build
(host `1.3.1.zlib-ng` vs ubuntu-runner stock zlib) → container now ZIP_STORED with `create_system=3` pinned
(stored bytes are compressor-independent — INV-4 across environments). Five manifest `notes` fields embedded
pre-normalize tmp sizes and one wall-clock value (also environment-dependent) → builders now measure the FINAL
normalized blob; clock value removed from prose (assert kept). Pinned `lxml`/`et_xmlfile` (they serialize the
XML bytes; were unpinned). Re-ran on host: `gen build` 151/151, `gen verify` 0 problems (incl. rebuild
determinism), dedup OK, `convert_check.sh` 151/151 ok (LO accepts STORED containers), goldens re-rendered
151/151 + stability ×2 151/151 ok. **Store is re-keyed on the new fixture_sha — owner golden approval (SA-6)
must be done against the new store.** Baseline numbers stand (uncompressed content identical; report-v1 embeds
no fixture sha).
(2) `windows-latest` UnicodeDecodeError: locale-default text I/O (cp1252) on utf-8 manifests (CJK fixtures) →
explicit `encoding="utf-8"` on every text read/write in generator, baseline, golden, nightly inline steps.
Bonus: `tools/convert_check.sh` reads the image tag from `docker/harness/pin.txt` (was hardcoded; the stale
local tag `galleyproof-harness` masked this).
**Round 2 (same day, run 34214500425):** windows ✅; `harness` still red — only the 13 image-bearing manifests
(image-textwrap + scanned-ocr) drifted. Isolated in a python:3.12-slim container (CI-equivalent): pixels and
IHDR identical, only the PIL-written PNG **IDAT** differed — Pillow 12.2.0's cp314 vs cp312 wheels bundle
different zlib/libspng builds. Fix: `core.png_rgb_bytes()` — PIL draws, container hand-encoded (fixed IHDR,
filter-0 scanlines, zlib **level 0** stored blocks = pure RFC-1951, no encoder choices). All 8 PIL→PNG sites
switched (docx/odf/pptx builders incl. the mirror paths). Proof: full 151-fixture rebuild in
python:3.12-slim+pinned-deps vs committed manifests = **0 drift** (and 0 on host); `gen verify` 0 problems;
convert-check 151/151; goldens re-rendered + stability 151/151 ok.

**Human-bound items (agent delivers research/checklists only):** trademark legal opinion (GOV-01),
registrations & payments (GOV-02), GitHub private-repo creation under the personal account (`reinartz/stichos`, D18)
for CI push, G1 license decision (final call).

**Environment (measured 2026-09-03):** Fedora 44 host; docker 29.6.2, podman 5.8.4, LibreOffice
26.2.5.2, pandoc 3.7.0.2, pdftoppm + gs 10.06, python 3.14 (PIL/numpy/skimage), rust 1.98, git,
network OK. Missing: mutool, qpdf, apt/brew/choco (non-Fedora distro checks go via web).

| Stage | Task IDs | Deliverable | Status |
|---|---|---|---|
| SA-1 | GOV-01(research), GOV-03, GOV-02(prep) | `docs/gov/clearance-memo.md`, `docs/gov/distro-collisions.md`, `docs/gov/registration-checklist.md` | done |
| SA-2 | HAR-01, C9(draft) | `corpus/PLAN.md` + license table + manifest schema | done (127 generated + 24 curated budget; C9 draft) |
| SA-3 | HAR-03 | `docker/harness/` pinned image + `tools/fontset_hash.sh` | done (LO 26.2.5 pinned; font set fixed +cjk/extra; `fontset_hash=sha256:6f26ae4f9bdea927cffa0984d48adcb3e4b60b3fbe263c6a44440a5628927509`) |
| SA-4 | HAR-04 | `harness/metrics/` (SSIM, page-count delta, feature retention, text F1) | done (40 tests; C9 amended to 0.2.0-draft §10) |
| SA-5 | HAR-02 | `harness/generator/` + `corpus/manifests/` ≥120 generated fixtures | done (127/127 built, verify 0 problems, LO convert 127/127 ok; `corpus/GENERATION_REPORT.md`) |
| SA-6 | HAR-05 | golden pipeline + content-addressed approved store | done (151 candidate records, stability ok; **approval = human gate, pending owner**) |
| SA-7 | HAR-06 | `baseline/report-v1.md` — soffice + pandoc + pdftoppm over full corpus | done (report-v1 over 151: rt 151/151, retention 123/151, F1 soffice 0.70 / pandoc 0.91) |
| SA-8 | HAR-07, DIS-01, SEC-02 | `.github/workflows/` PR subset + nightly ×3 platforms + cargo-deny | done (M0 CI interpretation: `pr.yml` = platform-agnostic manifest-only gate — `gen verify --no-blobs` + metrics pytest + PLAN §2 taxonomy + `corpus/pr-subset.txt` (20 ids, one/category; `long-deck-300`+`huge-cell-text` deferred per PLAN §3) + yaml/json syntax gates; `nightly.yml` = linux fidelity job (pinned image built + gated vs new machine-readable `docker/harness/pin.txt` — LO build & `fontset_hash` — corpus rebuilt from generator with byte-reproducible-manifests check, full convert via `tools/convert_check.sh`, 25-id `golden.py stability` sample, trend row appended to `baseline/nightly-trend.csv` as **artifact + step summary only, nothing committed back** — owner option later: separate `contents:write` PR job) + platform-agnostic matrix ubuntu/macos/windows running verify+pytest = the "green ×3 platforms" gate (fidelity image is linux-only by design, D15; LO-on-Windows probes = Phase 1/ENG-05); SEC-02: `deny.toml` ready-but-skipped (CI step self-activates on `Cargo.toml` at CORE-01) + `dependabot.yml` (pip+actions weekly); harness deps pinned in `harness/requirements-dev.txt` (dev-only, concept §6). Offline-verified: YAML parse ×3, inline steps dry-run, fontset hash re-measured, generator rebuild byte-identical; not run here: docker buildx/gha-cache, docker convert/stability, cargo-deny) |
| SA-9 | HAR-08, PDF-01(support) | upgrade-gate procedure + G1 ADR decision-support draft | done (`docs/harness/upgrade-gate.md`: LO+poppler+fontset pin gate — trigger = new TDF stable (download-tree listing check sketch + release-notes page; measured 2026-09-05: 26.2.6 and 26.8.0 ALREADY on the tree → first gate application imminent), ARG bump → full-corpus re-harvest keyed per engine-key so old goldens survive (INV-4) → full stability → diff vs old pin (page Δ / `metrics.ssim_pages` distribution where counts match / retention vs report-v1 111/127 / F1 with odp+RTL review-only caveats) → human checklist → pin.txt commit + D-log note on N-1 window shift; rollback free (immutable old tag + per-pin goldens); LO upgrade = ONLY admissible cause of mass golden change, drift at unchanged pin = bug; nightly = enforcement; owners/SLA = sketch pending owner); `docs/adr/ADR-0001-pdf-stack-g1.md` **status proposed** — decision owner = project owner, decide-by W0; RECOMMENDATION: lopdf+pdf-writer pure-Rust core + qpdf shell-out, mutool/gs optional D11 rungs, pdfium deferred to G5 (M0 corpus produced NO PDF-input evidence → G5 open; deciding experiment = PDF-04 word-bbox sub-corpus vs poppler `-bbox` feeding LIN-01). License facts fetched 2026-09-05: qpdf Apache-2.0 ✓, mutool AGPL-3.0 ✓, pdf-writer MIT OR Apache-2.0 ✓, **lopdf = MIT-only — corrects the "MIT/Apache" assumption** (still allow-list-safe); pdfium LICENSE = BSD-3-style text w/ appended Apache-2.0 text; gs dual AGPL/commercial researched-not-refetched; all legal-interpretation rows unverified → counsel) |
| SA-10 | HAR-02(v1.1) + HAR-06 extension — M0 ≥150 gate | corpus 127→**151** generated (v1.1 M0-gate additions, PLAN.md §3 ✚-marked): second instances in under-represented formats (pptx 15→26, odp 11→22, ods 19→21; +24), each builder 002 producing content that DIFFERS from 001 (strings/layout/counts/payloads/authors; asserts include ABSENCE of 001 content); curated-24 ingest remains owner follow-up (gate met on generated alone) | done (`gen build` 151/151; `gen status` 151/151 no gaps; `gen verify` 0 problems; `tools/dedup_check.py` (new): all same-category blobs distinct; 127 pre-existing blobs byte-identical post-rebuild (goldens/baseline rows stay valid); LO convert 24/24 ok first pass in pinned image → `convert-check.csv` 151/151 ok (ODP annotations kept in `draw:notes` per measured lesson); goldens: 24 rendered + `stability` ×2 re-renders → 24/24 stable, store 151 records, stability ok=151 fail=0; baseline `run` (resumable, 24 new ids only) + `report` → `report-v1.md/json` over **151**: round-trip 151/151, retention 123/151 (0.815; was 111/127=0.874 — delta fully explained by the 11 new odp fixtures failing the known LO frame re-wrap + 1 known ods-OLE-frame-drop), mean F1(soffice) 0.7017 / excl-odp 0.8082, F1(pandoc) 0.9088 (docx+odt, 61/151), Δpages=0 in 151/151, no new failure mechanism — new rows mirror 001 patterns; static counts in baseline.py narrative made dynamic) |

Rules of execution:
- One subagent at a time; each writes only inside its deliverable paths.
- `project_concept.md` and `project_plan.md` are not edited by subagents; decision changes go through the §16 D-log by the owner.
- Name always lowercase `stichos`; assets/ untouched.
- Findings marked measured (command + result) vs researched (URL) vs unverified — no invented claims.

**Flags for owner (from SA-1):** `stichos.org` registered 2026-07-06 with a live same-space
"production proofing" tool (also a paid SaaS at stichos.levships.com) — counsel must weigh before
TM filing; possible D1 revisit trigger. All registries still free; `stichos.com` taken (exp 2027).
TM offices all unverified (JS-only) — manual search steps recorded. winget-pkgs check needs auth.
