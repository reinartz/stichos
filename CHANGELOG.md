# CHANGELOG

All notable changes to stichos. The format follows
[keep-a-changelog](https://keepachangelog.com/); the workspace uses one unified version for all
crates and the `stichos` binary. Tool-surface (C7) versioning is additive-only within a minor;
breaking changes require a minor bump per `docs/contracts/C7.md`.

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
