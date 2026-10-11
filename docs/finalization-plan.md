# Finalization plan — tree unblock + Tracks E / I / G / F + owner closeout

Plan-only document. Created 2026-10-01 at HEAD `76a98cb` (dirty Track-H working tree; Phase U **complete** —
code moved to branch `track-h-proposed`, `main` clean).
Task IDs E/F/G/H follow `docs/remaining-work.md` rev 6 (source of truth for status); this doc is the
**executable ordering for a later agent**, plus the **owner-only 👤 steps** (§8). Track I is new here.
No implementation performed here. Do not add unverified claims to source-of-truth docs while executing.

## 0. Baseline (measured, 2026-10-01)

| Surface | Clean `76a98cb` (2026-09-29) | Working tree (2026-10-01, 60 porcelain entries) |
|---|---|---|
| fmt / check-deps | green | green |
| clippy `--all-targets -D warnings` | green | **FAIL** — 2 × E0425 `crates/engines/src/ms_office.rs:479,481` |
| `cargo test --workspace --lib` | 2244 pass / 9 fail | **does not compile** (excl. engines: 31 × `opc.rs` panics + 9 gp-core + 1 mcp) |
| `cargo test -p mcp --lib` | 93/93 | 92/1 (`http::tests::test_review_version_hash_invalidated`) |
| phase3 / phase4 `--ci` | 26/26, 26/26 | 22/26, 24/26 |
| phase5 / phase6 `--ci` | 50/52 (E1), 92/93 (E4) | 48/57, 88/93 |
| `p606_ctx06_compliance` | 6/6 | 5/6 (schema-freeze expects 23 tools, tree has 24) |
| verbs / MCP tools | 22 / 23 | 24 / 24 (+`validate`, +`finish`, +`validate_package`) |

Three tree regressions from the **proposed, unadopted** Track H (owner declined H0 2026-10-01):
- **H-R1** `crates/engines/src/ms_office.rs:477` — test binds `_engine`, body uses `engine` → E0425 ×2.
- **H-R2** `crates/adapters-ooxml/src/opc.rs:301` and `:319` — `search_start = actual_pos + pos + 1;`
  double-counts `pos` (already folded into `actual_pos`) → cursor runs out of bounds → 31 test panics.
- **H-R3** `crates/mcp/src/http.rs:306-318` — hash check is `if let Ok(bytes) = read_document_bytes(..)`;
  `read_document_bytes` rejects non-PK `.docx` (`crates/gp-core/src/file_access.rs:263-277`, no raw-mode
  escape hatch), so a changed corrupt docx **silently skips** invalidation → 200 instead of 410.

Plus pre-existing clean-HEAD debt: 9 gp-core formula fails (Track I, §5), stale gate expectations
(Track E, §4), P7-3 tranches C/D (Track F, §7), KI-05/KI-02 (Track G, §6).

## 1. D0 — owner decision, ADOPT vs SHELVE (👤, blocks everything)

Gates cannot be green while a 24-tool tree is measured by 23-tool expectations, and H0 adoption was
declined. One ruling unblocks (owner may re-take D0 at any time; the 2026-10-01 ruling was "keep proposed"):

| | **Path A — adopt Track H** | **Path B — shelve Track H (default, respects the declination)** |
|---|---|---|
| Meaning | Track H code lands on `main`; counts become 24/24 formally | Track H code preserved on branch `track-h-proposed`; `main` returns to 22/23 |
| Gate counts | updated to 24 (+ C7 additive-bump chain, §7.4) | stay 23 (only E1 fixes needed) |
| Then | E3 re-measure → E4 re-anchor → F/G proceed | E1 → E3 → E4 → F/G/I proceed on clean main |

## 2. Phase U — tree unblock (agent, after D0)

**Path B (recommended):** ✅ **COMPLETE** (2026-10-01)
- U1 👤/agent: committed `.github/workflows/nightly.yml` `python`→`python3` fix on `main` (= E2).
- U2 agent: committed doc sync (`AGENTS.md`, `CODEBASE.md`, `README.md`, `CHANGELOG.md`,
  `project_plan.md`, `project_concept.md`, `docs/remaining-work.md`, `docs/KNOWN_ISSUES.md`,
  `docs/track-h-generic-office-handling.md`, this plan).
- U3 agent: `git switch -c track-h-proposed`; applied H-R1/H-R2/H-R3 fixes (§7.1–7.3);
  verified `cargo fmt`, clippy, `cargo test --workspace --lib` (expect the 9 known gp-core fails only),
  `cargo test -p mcp --lib` 93/93; committed all Track-H code + `Cargo.lock`; pushed the branch.
- U4 agent: `git switch main` — tree returned to clean state; deleted stray `test_output*.txt`
  (UTF-16 PowerShell logs, regenerable).
- U5 agent: re-measure all gates + tests on main; header docs (AGENTS/CODEBASE/concept/plan) then
  describe **clean main** — removed "dirty tree red" block added 2026-10-01 and noted Track H code
  preserved on branch `track-h-proposed`.

**Path A (adoption):**
- A1 agent: fix H-R1/R2/R3 on main (§7.1–7.3), U1+U2 commits included.
- A2 agent: raise the three count-sensitive expectations to 24: `tools/phase6_gate.sh:595-600` (3e),
  `crates/cli/tests/p606_ctx06_compliance.rs:244-248`, `tools/p5_01_verify.sh:23-24` (see E1d).
- A3 agent: C7 additive-tool chain per `docs/contracts/C7.md` §Versioning (L173): bump contract
  1.6.0 → 1.7.0; extend accepted versions in `crates/mcp/src/spec.rs:880-896`; update phase6 3c
  expectation; keep Wave-C per-tool versions frozen at 1.6.0; `validate_package` schema_version set
  accordingly; p606 freeze test still asserts Wave-C at 1.6.0 but TOOLS 24.
- A4 agent: full re-measure (fmt, clippy, workspace lib, mcp, check-deps, 4 gates, p606) — all green
  except E4 tag lag; 👤 commits; headers synced to adopted state (Track H becomes adopted — needs
  registration line in `project_plan.md` header + `project_concept.md` §19, decision-log entry if a
  formal track letter is assigned).

## 3. Track E — gate/tag consistency (agent; needs clean main, i.e. after Phase U)

- **E1a** `tools/phase5_gate.sh:582-588`: stop demanding zero `m5*` tags. Adopt the phase6-7b pattern
  (`tools/phase6_gate.sh:855-863`): pass when `m5` absent **or** an ancestor of HEAD.
- **E1b** `tools/phase5_gate.sh:647,657-660`: C7 expectation `1.5.0` → `1.6.0` (or accept ≥1.5).
- **E1d (new, found 2026-10-01)** `tools/p5_01_verify.sh:23-24` demands `TOOLS.len()==20` and `:46`
  demands `CODEBASE.md` "20 specs" — stale at both HEAD (23) and tree (24); measured red standalone
  (12 PASS / 2 FAIL). Set to 23 (Path B) / 24 (Path A) or derive dynamically; align `:44-48`
  doc-string checks with current docs. ⚠️ This means the "phase5 50/52 with exactly 2 fails" record is
  suspect (check 8e runs this script) — re-measure and correct `docs/remaining-work.md` rev 6 §0 if needed.
- **E1e (optional hardening)** `tools/phase5_gate.sh:687-695` (7e) greps `TOOLS\.len()\|23` and passes
  unconditionally; `tools/phase6_gate.sh` 9f comment says "23 tools" but only greps `sign_document`.
  Make 7e assert the real count; leave 9f.
- **E3** re-run all gates `--ci`; record counts (target: phase3 26/26, phase4 26/26, phase5 52/52,
  phase6 92/93 pre-E4, p606 6/6, workspace tests see Track I).

## 4. Track I — gp-core formula correctness (agent) — NEW, fixes the 9 known fails

The "9 date-hardcoded fails" label is a **misnomer**: tests are deterministic; the evaluator is wrong
(measured 2026-10-01). All in `crates/gp-core/src/formula/evaluator.rs`; `chrono = "0.4"` is already a
dependency (`crates/gp-core/Cargo.toml:22`).

- **I1** replace broken serial↔calendar converters `serial_to_ymd` (L701-720, corrupted Hinnant variant,
  wrong epoch 719527) and `ymd_to_serial` (L723-731, off by 25569) with chrono-based math on the Excel
  epoch (serial 1 = 1900-01-01; keep the serial-60 1900-leap handling consistent). Fixes YEAR/MONTH/DAY/
  DATEDIF(d,m,y)/EOMonth conversions (measured: `YEAR(45672)`→2043, `MONTH(45672)`→612).
- **I2** swap the RANK conditions in `eval_rank_fn` (L1041-1050): `order==0` must count `v > target`
  (largest → rank 1), else `v < target`. Measured inverted: `RANK(3,1..10,0)`→3, want 8.
- **I3** fix genuinely wrong test constants in `test_eomonth` (L1956-1963): end-of-Jan-2025 is serial
  **45688** not 45701; end-of-Feb-2025 is **45716** not 45729 (verify every constant against an
  independent reference, e.g. LibreOffice or Python). Fix mislabelled `test_datedif_*` comments
  (44927/45292 are 2023-01-01/2024-01-01; expected values are correct).
- **I4** acceptance: `cargo test -p gp-core --lib formula` fully green; `cargo test --workspace --lib`
  0 fails on main; update `docs/KNOWN_ISSUES.md` footer + KI-10 wording (drop "9 date-hardcoded",
  workspace known-fails → 0) and the test tables in `AGENTS.md`/`CODEBASE.md`.

## 5. Track G — known issues (agent; independent of E)

- **G1** (KI-05, M): `crates/cli/src/render.rs` — `render_document` must report `output_dir` (and/or
  per-page paths) **in the JSON response**, not only stderr; MCP route parity; acceptance: JSON carries
  `output_dir`; C7 patch-level note if wire-visible; KI-05 → resolved.
- **G2** (KI-02, S): `crates/engines/src/raster.rs` — either pass a real page/page-range (pdfium rung is
  available since G5/pdfium adoption) or post-check output page count; else downgrade KI-02 to
  documented wontfix with mitigation text; update `docs/KNOWN_ISSUES.md` either way.
- **G4 (new)**: refresh stale Prerequisites/Status sections of `docs/track-b-p7-9-launch.md` (L52-64,
  L96-132 predate Tracks A′/C/D) so the owner checklist is executable.

## 6. Track F — P7-3 formula coverage tranches C+D (agent; largest, ~5 person-weeks)

Spec: `docs/track-b-p7-3-formula-coverage.md` §4 tranches C/D + §5 verification; task list F1–F8 in
`docs/remaining-work.md` §1 (F1 financial 7 fn ∥ F2 advanced date/time ∥ F3 TEXT/TEXTJOIN/SEARCH/FIND ∥
F4 XOR/IFS/SWITCH → F5 named ranges → F6 3-D refs → F7 structured table refs → F8 parity+contract sync).
Notes for the executing agent:
- **Sequence Track I first** — F1/F2 touch the same `evaluator.rs`.
- Lift `GP-F-007` (financial), `GP-F-008` partial (text), `GP-F-009` partial (logical) in `refuse.rs`;
  keep honest-refusal codes for what stays out (F6 may refuse 3-D refs with a dedicated GP-F code).
- No wall-clock anywhere: use `analysis_date()` deterministic stub per A4 design note / ADR-0011.
- Acceptance: known-value fixtures in `corpus/fixtures/formula-parity/`, `tools/parity_formula.sh --ci`
  inventory asserts 100 % implemented-or-refused (never parse-error), `supported_functions()` exhaustive,
  C15 §1 addendum note, CHANGELOG entry, `phase*_gate.sh --ci` non-regression.

## 7. Track H fix details (apply on branch in Path B, on main in Path A)

- **7.1 H-R1** `crates/engines/src/ms_office.rs:477`: rename `_engine` → `engine`; if a Windows CI build
  ever warns (body is `cfg(not(windows))`), add `#[allow(unused_variables)]` on the test fn.
- **7.2 H-R2** `crates/adapters-ooxml/src/opc.rs:301,319`: `search_start = actual_pos + 1;` in both the
  `<Override` and `<Default` find-cursor loops (or rewrite with `match_indices`). Acceptance: the 31
  panicking adapters-ooxml tests + `opc_validate` 6/6 green.
- **7.3 H-R3** `crates/mcp/src/http.rs:306`: replace `if let Ok(bytes) = read_document_bytes(..)` with a
  `match` where `Err` **also returns the 410 "document changed since session created"** response
  (unreadable/changed doc counts as invalidated); keep the success/copy-aside path untouched. Do not add
  a raw-bypass flag to `file_access.rs` unless a second call site needs it.
- **7.4 count sync (Path A only)** — see A2/A3; also re-run `docs/track-h-generic-office-handling.md`
  §4 DoD checklist (H9) and record evidence there before any adoption commit.

## 8. Owner steps 👤 (complete list)

1. **D0 ruling** — adopt (Path A) or shelve (Path B) the Track-H tree (§1). Blocks Phase U.
2. **Approve Phase U commits/push** — especially branch `track-h-proposed` push and creation
   (Path B), or the adoption commit (Path A); and deletion of stray `test_output*.txt`.
3. **E2** — commit (or explicitly authorize the agent to commit) the `nightly.yml` `python3` fix.
4. **E4** — after E1–E3 land on a gate-green commit (post-Track I too, ideally):
   `git tag -f m6 <gate-green-HEAD> && git push --force-with-lease origin m6`; acceptance: `phase6_gate.sh
   --ci` 93/93 and `git rev-parse m6` == gate-green HEAD. Owner-approved precedent 2026-09-24.
5. **OQ2 ruling** — Word tier for non-Windows users (Aspose/Syncfusion vs hosted worker vs none);
   record in `project_concept.md` §16/§18-2.
6. **P7-9 launch** — after G4 refreshes the checklist: run `docs/track-b-p7-9-launch.md` (pre-flight,
   `tools/version_bump.sh`, `tools/changelog_gen.sh`, annotated `v*` tag, CI 6-target release, verify
   draft artifacts, publish release, Winget + Homebrew submissions, rollback plan per
   `docs/release-playbook.md`). Not delegable.
7. Optional: authorize pushing agent commits as they land (repo convention: only commit when asked).

## 9. Sequencing and final acceptance

```
D0(👤) → U (unblock) → E1a/E1b/E1d → I (formula fixes) → E3 re-measure → E4(👤 re-anchor m6)
              ├→ G1, G2, G4 (independent, anytime after U)
              ├→ F1..F4 (parallel-capable) → F5 → F6 → F7 → F8
              └→ (Path A only) H9 closeout before E4
then: header/docs final sync → P7-9 launch prep (agent) → launch (👤)
```

Final acceptance (all on `main`, tags aligned):
- `cargo fmt --all --check`, clippy `--workspace --all-targets -D warnings`, `tools/check-deps.sh` green.
- `cargo test --workspace --lib` **0 failures**; `cargo test -p mcp --lib` all green (93 or 94).
- `phase3/4/5/6_gate.sh --ci` **26/26, 26/26, 52/52, 93/93**; `p606_ctx06_compliance` **6/6**.
- `m6` tag == gate-green HEAD; `main` pushed; verb/tool counts consistent across
  AGENTS/CODEBASE/concept/plan/spec asserts/gates (22/23 Path B · 24/24 Path A).
- KI-02, KI-05 closed or documented-wontfix; 9-formula misnomer retired in `docs/KNOWN_ISSUES.md`.

## 10. Verification appendix

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test -p mcp --lib
tools/check-deps.sh
bash tools/phase3_gate.sh --ci && bash tools/phase4_gate.sh --ci
bash tools/phase5_gate.sh --ci && bash tools/phase6_gate.sh --ci
cargo test -p stichos --test p606_ctx06_compliance
bash tools/p5_01_verify.sh
```
