# CHANGELOG

All notable changes to stichos. The format follows
[keep-a-changelog](https://keepachangelog.com/); the workspace uses one unified version for all
crates and the `stichos` binary. Tool-surface (C7) versioning is additive-only within a minor;
breaking changes require a minor bump per `docs/contracts/C7.md`.

## [Unreleased] — M4 + M5 + M6 (tags `m4`, `m5`, `m6`)

### Added
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
  `tempfiles.rs`). 10 Wave-C tools bumped to `schema_version` 1.6.0. **Closeout open:** the
  `__budget`+`rendered_by` stamping currently lands only in `cli/src/image_output.rs` — full
  100 %-response wiring and the C7 v1.6 spec-test/contract-doc freeze are tracked in
  `docs/remaining-work.md` (T-A/T-B), see `docs/m6-acceptance.md` §11.
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

### Fixed
- **Dead dependency cleanup (B0.6):** Removed unused `pkcs7 = "0.4"` from `gp-core/Cargo.toml`
  (deprecated crate, never imported). Zero new dependencies vs D25/ADR-0010.
- Post-acceptance correction (`docs/m5-acceptance.md` §9, 2026-09-20): `tools/check-deps.sh`
  fails at the `m5` tree on the `mcp -> adapters-pdf` edge (P5-09 text-layer route); repaired by
  dependency inversion in Phase 6 Wave 0 (`gp-core::traits::PdfTextSource`) — `check-deps.sh`
  exits 0 at the `m6` tree. No schema change.
- `adapters-pdf` `parser.rs:498` subtract-overflow panic on LibreOffice-produced PDFs (P6-32):
  `saturating_sub` fix, empty/short-data guards, regression fixtures, fuzz seed.
- Doc-rot sweep (P6-34): stale ms-office / "deferred to Phase 5" / "compile-only skeleton"
  references removed from `CODEBASE.md`, `CHANGELOG.md`, `AGENTS.md`, `support-matrix.md`,
  `engines/Cargo.toml`, `formula/mod.rs`.

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
creation + ODF/PDF write completion. See `docs/phase*-plan.md` / `docs/*-acceptance.md` for the
full step-level record. (The transient `0.1.1`/`0.1.0` bump entries previously listed here were
artifacts of a `tools/version_bump.sh` dry-run test, not releases.)
