# CHANGELOG

## [0.2.2] — 2026-10-18

- Add per-crate `README.md` for all 8 published crates (what it does, install, usage example,
  when to use, limitations, related crates); each crate now ships its own README on crates.io
  instead of the workspace root README.
- Documentation only — no code or API changes.


## [0.2.1] — 2026-10-17

- Version bump: 0.2.0 → 0.2.1
- Generated: 2026-10-17T21:08:11+0200



All notable changes to stichos. The format follows
[keep-a-changelog](https://keepachangelog.com/); the workspace uses one unified version for all
crates and the `stichos` binary. Tool-surface (C7) versioning is additive-only within a minor;
breaking changes require a minor bump per `docs/contracts/C7.md`.

## [0.2.0] — 2026-10-17 (first public release; M4 + M5 + M6, tags `m4`, `m5`, `m6`)

First public release (P7-9 launch): publishes to crates.io, GitHub Release, and package-manager
manifests per `docs/release-playbook.md`. This section covers everything shipped since the
(unpublished) `0.1.0` milestone packaging of M1–M3.

**2026-10-01 completion audit:** measured dirty-tree state recorded in the local-only
remaining-work doc (rev 6) — working tree carried uncommitted proposed Track-H code (60 porcelain
entries); the 2026-09-29 clean-HEAD `76a98cb` figures above remain the baseline.
**Track H (generic Office handling):** proposed
2026-09-29; H0 adoption declined by owner 2026-10-01 — code moved to branch `track-h-proposed`
by Phase U (2026-10-01), closeout H9 not performed; `main` returned to clean state.
**Doc sweep (Track G G3) complete:** `AGENTS.md`, `CODEBASE.md`, `README.md`,
`project_concept.md` headers synced to the Phase U clean-HEAD state (verb/tool counts, Track H
status).

### Added
- **Track D (CI green):** CI gates green — `phase6_gate.sh --ci` 93/93 at `bd29637` (92/93 at 2026-09-29 HEAD: `m6` tag lag, E4), `phase3/4_gate.sh --ci` 26/26, `phase5_gate.sh --ci` 50/52 (2 stale-gate fails, E1). Includes: corpus PDF renames (D3.2), `gen build` PDF integration (D3.3), `gen build` blob bootstrap in CI (D3.4), workflow YAML fixes (D4), gate known-failures tolerance (D5), clippy/fmt fixes across all crates (D1/D2).
- **Creation store handback (C7 v1.2.1):** `create_document` returns `store_root` in response
  (path to content store containing the blob) so CLI/MCP consumers can fetch the document blob
  by ID after creation. null for dry_run. (Track C / C-F.2)
- **Create dry-run (C7 v1.2.1):** `create_document` honours `dry_run` — no content store write,
  returns base64-encoded preview bytes and `"dry_run": true`. CLI `--dry-run` now outputs preview
  to stdout. (Track C / C-F.1)
- **Human loop (M4):** `revisions` verb + `list_revisions`, `accept_revisions`,
  `reject_revisions`, `add_comment` tools (C7 v1.3.0); ODF revision read/write (C11);
  `review` session + raster-over-masters overlay (D22); `open` platform handoff;
  COM worker design + compile-only skeleton (`ms-office` feature flag).
- **COM implementation (M6):** COM-2..6 engine implementation on the Windows host
  (Word/Excel/PPT workers, `CompareDocuments`, logon-agent design); MS-Office COM
  engine compiles cross-platform behind the `ms-office` feature gate with cfg stubs.
  MSI packaging (`engines/src/msi.rs`) + session scheduler (`engines/src/scheduler.rs`).
- **Context budget (M6, C14/D27 — C7 additive v1.6):** summary-first `inspect_document`
  (`detail=summary` default), URL-first images (`output=urls` default; temp files, 1 h TTL +
  startup sweep, `expiresAt`), caps `max_elements=100` / `max_pages=3–5` / `max_ops=20` /
  `max_entries=10`, `dry_run=true` default on `apply_patch`/`edit_*` (CLI `--apply` opts out;
  `--dry-run` kept as deprecated no-op); `__budget` envelope types in `gp-core` (`envelope.rs`,
  `tempfiles.rs`). 10 Wave-C tools bumped to `schema_version` 1.6.0. **Closeout complete:**
  `__budget`+`rendered_by` stamped on 100 % of CLI verb + MCP serve-boundary responses (T-B),
  C7 v1.6 spec-test freeze + C7.md/C14.md frozen (T-A), gate hardened (T-C) — Track A′ complete.
- **Formula evaluation (M6, C15/ADR-0011):** own evaluator for the xlsx subset (12 functions:
  SUM, AVERAGE/AVG, COUNT, MIN, MAX, AND, OR, NOT, IF, CONCATENATE, IFERROR), cached-value
  semantics, GP-F-001..004/010/020 refuse codes, parity lint (LIN-B-010) for stale caches;
  ODS `of:=` formulas translated into the same evaluator.
- **ODS/ODP revisions (M6):** accept/reject lossless on corpus subset (draft-clause format,
  `text:delete` reader).
- **Word-interop verification (M6):** 12 integration tests against Word 16.0 (M365) on the
  Windows host (`cargo test --test word_interop`); divergence-as-golden pattern.
- **Feature-completeness gap-fill (M6 Wave I, owner calls C10–C13):** `inspect`/`diff` accept
  xlsx/pptx/ods/odp; `apply`/`edit` executor wired (no silent no-op); `sign_document` tool
  dispatched; `region` renders real master pixels via `MasterCache` (honest provenance); review
  page route serves real PNGs (no 501); `annotate_pages marked` renamed honestly to
  `marked_mirror` with a diagnostic; `update --fixture-path` / `STICHOS_FIXTURE_PATH`.
- **PDF-native loop (M5):** `annots` verb + `list_annotations`, `resolve_annotations` tools
  (C7 v1.4.0); C12 annotation-review model (Highlight/Text/FreeText/Stamp, page+rect+quote
  anchors, resolve states, GP-A0xx refusals); incremental annotation write-back (append-only,
  journaled, `dry_run`); `review` sessions over PDF (`mode: "pdf-annot"`); opt-in vendored
  pdf.js 3.11.174 text layer (`--text-layer`, D24, ADR-0009) with selection→quote anchoring.
- **Trust (M5):** `sign` verb + `sign_document` tool (C7 v1.5.0); C13 signature-integrity model
  (GP-S001..016); refuse-by-default mutation of signed documents with explicit policy pass;
  PAdES-B-B sign-later over the incremental writer (pure-Rust crypto, D25, ADR-0010);
  PDF/A revalidation wired into mutation QA + `doctor` row (honest-absent preserved).
- **CMS verification (B0.4):** `verify_signature()` — full PKCS#7 SignedData parser,
  signedAttrs digest verification, byte-range coverage check, X.509 cert self-signed
  validation; `stichos verify <path>` CLI verb; `stichos sign --verify` post-write
  verification; ECDSA `ETSI.CAdES.detached` / RSA `adbe.pkcs7.detached` subfilter mapping.
- **OpenSSL CI tests (B0.5):** `openssl cms`-based verification tests (feature-gated
  `openssl-ci`); tamper-detection fuzz fixtures in `adapters-ooxfuzz` (byterange tamper,
  hex content tamper); 19/19 gp-core signature tests green.
- **Sign wire:** `signature_value` + `signature_length` JSON fields replace `byte_range`
  (additive wire change per C7); no schema break.
- **Hygiene (M5):** `SECURITY.md`, `CODE_OF_CONDUCT.md`, SPDX headers, dual-license audit (D26:
  launch stays owner-timed, prep only).

### Changed
- **Create dry-run (C7 v1.2.1):** `dry_run=true` now returns `"dry_run": true` in the response
  and includes base64-encoded preview bytes under `"preview_bytes"`. Previously the field was
  always `"dry_run": false`.

### Fixed
- **Track D clippy/fmt (D1/D2):** `cargo clippy --workspace --all-targets -- -D warnings` now exits 0. Fixed 46 all-targets errors across `adapters-ooxfuzz` (19), `adapters-ooxml` (9+1), `adapters-pdf` (4), `gp-core` (2), `lint` (7), `engines` (3), `stichos` bin (1). `cargo fmt --all --check` clean.
- **Track D corpus (D3.2):** 12 PDF fixtures renamed (`comment-annot-pdf-*` → `pdf-annot-*` etc.) to satisfy schema id↔category rule. Schema `fixture.schema.json` relaxed to allow `source: generated` PDFs. BUDGET +15 keys synced. 6 revision manifests regenerated with correct features.
- **Track D CI blob bootstrap (D3.4):** `gen build` added to phase3/4/5/6 gate `--ci` paths — clean checkout CI shows only 13 known fails (down from 48).
- **Track D workflow YAML (D4):** `phase5-gate.yml` YAML parse error fixed (quoted colon-containing step names). `pr.yml` job-level `hashFiles()` expressions removed (actionlint clean). Toolchain bumps: checkout@v4→@v7, cache@v4→@v6. Docker-profile build context + base image aligned.
- **Track D gate hardening (D5):** Phase3 gate uses `--lib` + known-failures tolerance (13 pre-existing). Phase5 gate uses `--lib` + known-failures tolerance for gp-core date-hardcoded tests + Marvin advisory accepted in deny. PDF-annot fixture names updated. `gen build` bootstrap in all gates.
- **Dead dependency cleanup (B0.6):** Removed unused `pkcs7 = "0.4"` from `gp-core/Cargo.toml`
  (deprecated crate, never imported). Zero new dependencies vs D25/ADR-0010.
- Post-acceptance correction (2026-09-20): `tools/check-deps.sh`
  fails at the `m5` tree on the `mcp -> adapters-pdf` edge (P5-09 text-layer route); repaired by
  dependency inversion in Phase 6 Wave 0 (`gp-core::traits::PdfTextSource`) — `check-deps.sh`
  exits 0 at the `m6` tree. No schema change.
- `adapters-pdf` `parser.rs:498` subtract-overflow panic on LibreOffice-produced PDFs (P6-32):
  `saturating_sub` fix, empty/short-data guards, regression fixtures, fuzz seed.
- Doc-rot sweep (P6-34): stale ms-office / "deferred to Phase 5" / "compile-only skeleton"
  references removed from `CODEBASE.md`, `CHANGELOG.md`, `AGENTS.md`, `support-matrix.md`,
  `engines/Cargo.toml`, `formula/mod.rs`.
- Pre-launch fix sweep (2026-10-17): isolated MCP review-test temp files so parallel runs do not
  collide; phase gates no longer abort on a failed check under `set -e`; stray `page-1.png` render
  artifacts removed; stale status claims refreshed and links to excluded docs dropped (KI-05);
  publish/doctor docs section added.

### Dependencies
- Workspace version bumped 0.1.0 → 0.2.0 for the first public release; workspace-internal
  dependency pins (path + version) bumped in lockstep and `Cargo.lock` regenerated.

## [0.1.0] — 2026-09-16

First packaged release (milestones M1–M3 shipped; tags `m1-closeout`, `m2`, `m3`).

### Added
- **Read (M1):** `inspect`, `region`, `render`, `diff`, `doctor`, `serve`, `schema` CLI verbs and
  MCP tools `inspect_document`, `render_document`, `inspect_region`, `diff_documents`, `doctor`,
  `serve` — structure + coordinates + fidelity budget + rendered pages with provenance (INV-2).
- **Surgical edit (M2):** `apply`, `edit`, `annotate`, `revert`, `journal`, `perf` verbs and
  `apply_patch`, `edit_text`, `edit_cell`, `edit_structure`, `annotate_pages`, `revert_document`,
  `journal`, `perf` tools; byte-surgical OOXML patching (INV-1), undo journal (INV-3), dry-run
  diffs, tracked revisions by default (D7).
- **Creation (M3):** `create` verb + `create_document` tool across four paths — pandoc
  (`md/html → docx/odt`), Chromium (`html → pdf`), Typst (`typ → pdf`), and the native OOXML
  writer (IR → `docx`/`xlsx`/`pptx`, template-preserving); create-as-revisions (`w:ins`/`w:del`).
- **Write-path completion (M3):** ODF surgical writer + patch primitives + census/RelaxNG gate;
  PDF incremental append-only writer with annot/form/metadata mutations (signed inputs refused).
- **Sandbox (M3):** `script` verb + tool — sandboxed Rhai escape hatch (ADR-0005, D17).
- **Distribution (M3):** `update` verb + `check_update` tool with verify-before-swap and rollback
  (ADR-0006); release automation (`tools/version_bump.sh`, `.github/workflows/release.yml` ×6
  targets, winget/Homebrew manifest drafts); OCI profile image (`docker/profile/`).

## 0.0.x — pre-release development (2026-09-07 … 2026-09-16)

Phase 0 fidelity harness + 151-fixture corpus, Phase 1 read ship, Phase 2 surgical edit, Phase 3
creation + ODF/PDF write completion. (The transient `0.1.1`/`0.1.0` bump entries previously
listed here were artifacts of a `tools/version_bump.sh` dry-run test, not releases.)
