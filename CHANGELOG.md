# CHANGELOG

All notable changes to stichos. The format follows
[keep-a-changelog](https://keepachangelog.com/); the workspace uses one unified version for all
crates and the `stichos` binary. Tool-surface (C7) versioning is additive-only within a minor;
breaking changes require a minor bump per `docs/contracts/C7.md`.

## [Unreleased] — M4 + M5 (tags `m4`, `m5`)

### Added
- **Human loop (M4):** `revisions` verb + `list_revisions`, `accept_revisions`,
  `reject_revisions`, `add_comment` tools (C7 v1.3.0); ODF revision read/write (C11);
  `review` session + raster-over-masters overlay (D22); `open` platform handoff;
  COM worker design + compile-only skeleton (`ms-office` feature flag).
- **PDF-native loop (M5):** `annots` verb + `list_annotations`, `resolve_annotations` tools
  (C7 v1.4.0); C12 annotation-review model (Highlight/Text/FreeText/Stamp, page+rect+quote
  anchors, resolve states, GP-A0xx refusals); incremental annotation write-back (append-only,
  journaled, `dry_run`); `review` sessions over PDF (`mode: "pdf-annot"`); opt-in vendored
  pdf.js 3.11.174 text layer (`--text-layer`, D24, ADR-0009) with selection→quote anchoring.
- **Trust (M5):** `sign` verb + `sign_document` tool (C7 v1.5.0); C13 signature-integrity model
  (GP-S001..016); refuse-by-default mutation of signed documents with explicit policy pass;
  PAdES-B-B sign-later over the incremental writer (pure-Rust crypto, D25, ADR-0010);
  PDF/A revalidation wired into mutation QA + `doctor` row (honest-absent preserved).
- **Hygiene (M5):** `SECURITY.md`, `CODE_OF_CONDUCT.md`, SPDX headers, dual-license audit (D26:
  launch stays owner-timed, prep only).

### Fixed
- Post-acceptance correction (`docs/m5-acceptance.md` §9, 2026-09-20): `tools/check-deps.sh`
  fails at the `m5` tree on the `mcp -> adapters-pdf` edge (P5-09 text-layer route); scheduled
  for dependency-inversion repair in Phase 6 Wave 0. No schema change.

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
