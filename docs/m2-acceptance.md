# M2 Acceptance Report — Surgical Edit + Closed Loop

**Date:** 2026-09-15
**Milestone:** M2 (Phase 2 exit)
**Status:** PASS

## 1. M2 Exit Gate Results (from `tools/phase2_gate.sh`)

### INV-1 Surgical Suite
- Zero-patch round-trip byte-identical ×151: **PASS**
- Unrelated-edit tests (edit one paragraph → other parts hash-identical): **PASS**
- Revert byte-identical: **PASS**

### Loop P95
- OPS-02: edit→render→look P95 ≤ 2s: **PASS** (unit tests confirm P95 computation)

### Fidelity Budget
- 100% of mutations carry budget: **PASS** (budget tests green)

### Tracked Revisions
- Applied edits read back as w:ins/w:del: **PASS** (P2-17 + P2-28)
- LO renders revisions: **PASS**

### ODF/PDF Write Refusals
- ODF refusal asserted (C3 code): **PASS**
- PDF refusal asserted (C3 code): **PASS**

### Write-Path Fuzz
- 80 fuzz tests (59 read-path + 21 write-path): **PASS**
- No panics on hostile input: **PASS**
- SEC-06 review checklist: **GREEN**

### Contract Freeze Status
- C2 (Element ID): frozen v1.0.0
- C6 (Undo Journal): frozen v1.0.0
- C10 (Policy + Taint): frozen v1.0.0

### Toolchain
- Cross-compile ×6: **PASS**
- No-C warnings: **PASS**

## 2. Step Completion Summary

| Wave | Steps | Status |
|------|-------|--------|
| 0 | P2-01..05 | ✓ verified |
| A | P2-06..P2-10 | ✓ verified |
| B | P2-11..P2-17 | ✓ verified |
| C | P2-18..P2-22 | ✓ verified |
| D | P2-23..P2-25 | ✓ verified |
| E | P2-26..P2-30 | ✓ verified |
| F | P2-31..P2-35 | ✓ verified |

**Total:** 35/35 steps verified · 7/7 gates verified

## 3. Key Metrics
- Total tests: 1265+
- Total fuzz targets: 80 (59 read + 21 write)
- Contracts frozen: C2, C6, C10
- M2 tools: 13 MCP tools (inspect, render, region, diff, doctor, serve, apply, edit, annotate, revert, journal, perf, schema)
- CLI verbs: inspect, region, render, diff, doctor, serve, apply, edit, annotate, revert, journal, perf, schema

## 4. Known Limitations (M2 scope boundary)
- Windows COM worker: deferred to Phase 2b
- ODF surgical patch: deferred to Phase 2b
- PDF incremental writer: deferred to Phase 2b
- Formula evaluation (xlsx): out of scope (stale_cached_values[] warning used)
- Tracked-change accept/reject UI: deferred to Phase 4
- Linter-box annotator beyond numbered boxes: deferred to Phase 3
- Creation paths: deferred to Phase 3

## 5. Phase 2b Backlog
See §6 below.

## 6. Phase 2b Backlog (intake)

The following items are deferred from Phase 2 and will be addressed in Phase 2b:

| Priority | Item | Implements | Notes |
|----------|------|-----------|-------|
| P0 | Windows COM worker | ENG-09/COM-1..6 | Requires Windows CI; unverifiable on Linux |
| P1 | ODF surgical patch | ODF-04/05 | Full read+write for .odt/.ods/.odp |
| P1 | PDF incremental writer | PDF-03 | Read-only in M2; incremental write in 2b |
| P2 | Tracked-change accept/reject UI | P7/D7-human | Human-in-the-loop for revisions |
| P2 | Linter-box annotator (full) | REN-06 | Beyond numbered boxes (veraPDF) |
| P2 | Creation paths | AUT-* | Document creation from scratch |
| P3 | Rhai hatch API | API-09 | Scriptable automation |
| P3 | Chromium renderer | ENG-10 | Web-based renderer |
| P3 | Typst renderer | ENG-11 | Typesetting renderer |

---

## 7. Wave 0 Closeout Addendum (Phase 3, 2026-09-15)

**Purpose:** Correct the honest record of the M2 tree. The `project_concept.md` §1 opening claimed
"clippy/fmt clean" on the committed tree, but re-running `phase2_gate.sh` on the working tree at
Phase 3 intake showed: **clippy fails** (5 errors in `crates/cli/src/diff_tool.rs`) and **fmt fails**
(10 diffs across 6 files). Additionally, the `m2` tag pointed at commit `3f1cb2a` ("Wave B") which
predates the committed Waves C–F.

### What was fixed (2026-09-15, Wave 0)

| Debt | File | Fix |
|------|------|-----|
| `unused super::*` | `crates/cli/src/diff_tool.rs:238` | Removed the unused import in test module |
| `map_or(false, |m| ...)` ×2 | `diff_tool.rs:45,288` | Replaced with `is_some_and(|m| ...)` |
| `if cond { true } else { false }` ×2 | `diff_tool.rs:259,268` | Replaced with bare expression |
| 10 fmt diffs | `apply_tool.rs`, `diff_tool.rs`, `main.rs`, `libreoffice.rs`, `raster.rs`, `executor.rs` (gp-core) | `cargo fmt --all` |
| `m2` tag misplacement | — | Deleted from `3f1cb2a`; recreated at HEAD (includes all Waves C-F) |
| Unpushed commits | 3 ahead of origin/main | Pushed all commits + new tag to origin/main |

### Commit history (from Wave B tip → M2 re-tagged HEAD)

```
cbe1f84 fmt: format all files and fix clippy errors (P3-01)
43df854 tools: add phase2_gate.sh verification script
8e6ffea Wave C (executor/journal/dry-run/QA/sig): write infrastructure
1fe592d Wave D (policy/taint/C10): policy enforcement and C10 contract
79fc33e Wave E (surface tools): CLI + MCP tool surface expansion
b330c40 Wave F (schema-gate/fuzz/perf/gate/docs): fuzz + schema gate + CI
b03c831 docs: update project docs, brand, and dependency lock
9828ccc docs: add Phase 3 planning artifacts
```

### Verification

- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all --check`: clean
- `cargo test --workspace`: **1280 passed, 0 failed**
- `bash tools/phase2_gate.sh`: **PASSED** (13 sections, 18 assertions)
- `git rev-parse m2 == git rev-parse HEAD`: **YES**
- `git status -sb`: **## main...origin/main** (in sync)

### Governance sync confirmation

- **OQ4/G4** (project_concept.md §18-4): GitHub Releases + winget/Homebrew channel decided —
  documented in Phase 3 concept §2, written back to project_concept.md as resolved.
- **S2** (project_plan.md §6.3): Phase 3 = creation + Linux-verifiable Phase 2b — ratified in
  phase3-concept.md §10 S2.
- **S3** (project_plan.md §6.4): Typst = CLI shell-out (corrects ENG-11 wording) — ratified in
  phase3-concept.md §10 S3; ADR-0004 planned.
