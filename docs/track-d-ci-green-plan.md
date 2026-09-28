# stichos — Track D: fix remaining errors/warnings + green GitHub CI (incl. nightly)

**Created:** 2026-09-28 (measured audit of working tree at `b7b8805` + uncommitted Track C/P7
changes, plus live GitHub Actions runs via `gh`). Plan-only — no fixes applied in this session.
Conventions same as `docs/remaining-work.md`: each **task** = one deliverable; subtasks sized
**S**/<50 LoC · **M**/2–4 files · **L**/crate-spanning. 👤 = owner-only (git push, contract
freeze decisions).

**Prerequisite:** the working tree carries ~2,700 LoC of uncommitted Track C (C-J/C-K pandoc AST,
probe/version/screenshot work) + P7 fmt fixes. Those changes must be committed (👤) before or
with Track D, or Track D results will not match CI. Local state at start: `main` = `origin/main`
= `b7b8805`; 30 modified + 3 untracked files pending.

---

## 0. State snapshot (measured 2026-09-28)

| Area | State |
|---|---|
| `cargo fmt --all --check` | **FAIL** — at HEAD: 9 files (cli/main.rs + 8 gp-core P7-5 files, fmt fixes already present uncommitted); working tree: 2 untracked files (`pandoc_ast.rs`, `pandoc_to_ir.rs`) |
| `cargo clippy --workspace --lib -- -D warnings` | **FAIL** — 2 errors in `adapters-ooxfuzz` (lib): `same item pushed into Vec` ×2 (lib.rs:1838, 1887 in `hostile_sig_byterange_*` helpers). Blocks phase4/phase5 hard clippy steps. `mcp`, `adapters-odf`, `gp-core`, `adapters-pdf`, `adapters-ooxml`, `engines`, `lint`, `stichos` lib-clippy otherwise clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL** — **46 errors** across 9 targets (phase3 gate hard-fails here first) |
| `cargo test --workspace --lib` (with local blobs) | **2240 pass / 13 known fails** (4 OPC zip-bomb in adapters-ooxml + 9 date-hardcoded gp-core) — matches AGENTS claim |
| `cargo test --workspace --lib` (clean tree, **no blobs**) | **2122 pass / 48 fails** — 29 adapters-ooxml + 10 adapters-odf + 9 gp-core. Blob-dependent tests panic `NotFound` on `corpus/blobs/*` (gitignored, never in git) |
| `cargo test -p adapters-ooxfuzz` (clean tree, no blobs) | **226 pass / 0 fail** locally — yet **fails on GitHub** (exit 101) in both phase4 and phase6 gates; cause unknown (gate discards test output) |
| `gen verify --no-blobs` | **FAIL 18 problems** — 12 PDF schema + 6 revision budget. Breaks nightly `platforms ×3` + nightly `harness` + pr.yml validate |
| `gen verify` (full, with blobs) | **FAIL 19 problems** — 12 PDF schema + 6 revision `feature recount … != manifest {}` + 1 `tracked-changes-docx-001: sha256 drift`. Breaks nightly `harness` |
| pr.yml taxonomy lint (step after verify) | would fail: 18 manifest categories missing from `corpus/PLAN.md` §2 |
| pr.yml validate → `metrics pytest` | 40 passed locally (OK once earlier steps fixed) |
| `.github/workflows/pr.yml` | **workflow file issue** — 0 jobs, every push since ~2026-09-15 (last success 2026-09-07). actionlint: 4× job-level `hashFiles('Cargo.toml')` not allowed (lines 146/164/177/195) |
| `.github/workflows/phase5-gate.yml` | **not parseable YAML** — `- name: Gate 0: intake hygiene` (colons at lines 22/24/26/28). Also breaks Dependabot `github_actions` (`dependency_file_not_parseable`) |
| `.github/workflows/nightly.yml` | failing since ≥2026-09-24: `harness` at `gen verify` (19 problems); `platforms ×3` at `gen verify --no-blobs` (18) |
| `phase6-gate.yml` (CI) | **86/90** — 4 FAIL: `ooxml --lib formula`, `ooxml --lib xlsx` (missing blobs), `ooxfuzz --lib` (unexplained), `cargo fmt --check` |
| `phase3-gate.yml` (CI) | FAIL at step 1c — `cargo clippy --workspace --all-targets -- -D warnings` |
| `phase4-gate.yml` (CI) | FAIL at 5a — `cargo test -p adapters-ooxfuzz` exit 101 (output discarded by `run_check`) |
| `docker-profile.yml` (CI) | FAIL — `docker build` context `docker/profile/..` = `docker/`, but Dockerfile `COPY crates/ ./crates/` needs **repo root** |
| Dependabot | pip OK; `github_actions` fails on phase5-gate.yml unparseable |
| `tools/check-deps.sh` | green (0 errors) |
| Node 20 deprecation warnings | `checkout@v4` / `cache@v4` / `upload-artifact@v4` in phase3/4/5/6, docker-profile, release (nightly/pr already on @v7) |

---

## Track D — fix order

### D0 — Commit the pending working tree *(👤)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| D0.1 | 👤 git | Commit the 30 modified + 3 untracked files (Track C C-J/C-K + P7 fmt fixes + docs) on `main`. Without this, every Track D fix below would be built on an uncommitted base and CI cannot see it. Run `cargo fmt --all` first so the new `pandoc_*.rs` files are formatted. | S |

---

### D1 — `cargo fmt` (phase6 gate FAIL 4/4, phase3/4/5 fmt steps)

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| D1.1 | whole tree | `cargo fmt --all` (covers HEAD’s 9 unformatted P7 files + untracked `pandoc_ast.rs`/`pandoc_to_ir.rs`). `cargo fmt --all --check` exit 0. | S |

---

### D2 — clippy `-D warnings` (phase3 all-targets + phase4/5/6 lib gates)

46 errors under `--all-targets`, 2 under `--lib`. Fix by lint; no allow-attributes except where
noted.

#### D2.1 — `adapters-ooxfuzz` lib (2) — **blocks phase4/phase5 lib clippy** · M

- `lib.rs:1838` / `1887` — `same item pushed into Vec` in `hostile_sig_byterange_tamper` /
  `_second_tamper` (80× `b'A'` / `b'B'` padding loops). Replace with `vec![b'A'; 80]`-style
  construction or `data.extend(std::iter::repeat_n(b'A', 80))`.
- Acceptance: `cargo clippy -p adapters-ooxfuzz --lib -- -D warnings` exit 0.

#### D2.2 — `adapters-ooxfuzz` tests (19) · M

- `lib.rs:4463/4477/4491/4511` redundant pattern matching (`matches!(result, Ok(_))` → `result.is_ok()`).
- `formula_fuzz.rs:245` single-pattern `match` → `if let`; `:491` unused `formula`.
- `parser_regression.rs:261` single-pattern `match` → `if let`; `:369` items after test module
  (move `mod tests` to end of file or move items above it).
- `wave_i_fuzz.rs:298-299,323,325` unused `parent_id` / `current_children` (dead scaffolding —
  remove); `:481,:930` unneeded `mut`; `:956` unused `name` (use `_name` or drop);
  `:1057` needless borrow.
- Acceptance: `cargo clippy -p adapters-ooxfuzz --all-targets -- -D warnings` exit 0.

#### D2.3 — `adapters-ooxml` (9 + 1) · S

- `formula/fixtures.rs:670,679,688,697,706,715` — `assert!(reader.len() > 0)` → `assert!(!reader.is_empty())` (6×).
- `revision_reader.rs:465,488,509` — snake_case test names: `test_enumerate_revisions_paraId` →
  `..._para_id` (or `#[allow(clippy::non_snake_case)]` if the name documents the OOXML attribute —
  prefer rename + comment).
- `tests/lossless_suite.rs:271` — useless `vec!` → array literal.
- Acceptance: `cargo clippy -p adapters-ooxml --all-targets -- -D warnings` exit 0.

#### D2.4 — `adapters-pdf` tests (4) · S

- `redact.rs:866` unused `make_pdf_with_text_stream` (dead helper in `mod tests` — delete or use);
  `:894` unused `xref_start`; `:932` value assigned to `off` never read (remove the +=
  bookkeeping or use the value).
- `annot_write.rs:1448` `b"Subj".to_vec()` → `b"Subj".as_slice()` / adjust key type.
- Acceptance: `cargo clippy -p adapters-pdf --all-targets -- -D warnings` exit 0.

#### D2.5 — `gp-core` tests (2) · S

- `formula/evaluator.rs:2168` — `clippy::approx_constant` fires on `ROUND(3.14159,2)`. The test
  asserts rounding of π-ish input; change the literal to a non-PI value (e.g. `3.1415`, expect
  `3.14`) so intent is preserved without the lint.
- `formula/evaluator.rs:2359` — `with_text` in `LookupTestContext` never used (dead code). Either
  delete it or add the lookup test that uses it (prefer a real test if coverage is missing).
- Acceptance: `cargo clippy -p gp-core --lib --tests -- -D warnings` exit 0.

#### D2.6 — `lint` tests (7) · S

- `encode.rs:516,517,536,567,568` needless `&r` → `r` (5×).
- `ssim.rs:502,516` manual range → `(0.00..=0.50).contains(&score)` (2×).
- (Auto-fixable: `cargo clippy --fix --lib -p lint --tests`.)
- Acceptance: `cargo clippy -p lint --all-targets -- -D warnings` exit 0.

#### D2.7 — `engines` tests (3) · S

- `libreoffice.rs:997` unused `use std::os::unix::io::FromRawFd` (already in scope) — delete import.
- `ms_office.rs:321` `capabilities.len() > 0` → `!capabilities.is_empty()`.
- `pandoc_ast.rs:395` `test_parse_orderedList` → `test_parse_ordered_list`.
- Acceptance: `cargo clippy -p engines --all-targets -- -D warnings` exit 0.

#### D2.8 — `stichos` bin (1) · S

- `cli/src/main.rs:812` `and_then(|v| if v == 0 { Some(usize::MAX) } else { Some(v) })` →
  `map(|v| if v == 0 { usize::MAX } else { v })`.
- Acceptance: `cargo clippy -p stichos --all-targets -- -D warnings` exit 0 (note: package name
  for the binary is `stichos`, not `cli`).

#### D2.9 — gate verification · S

- `cargo clippy --workspace --all-targets -- -D warnings` exit 0 → unblocks phase3 gate step 1c.
- `cargo clippy --workspace --lib -- -D warnings` exit 0 → unblocks phase4 gate 8a, phase5 gate 8a,
  phase5-gate.yml clippy step.
- Phase6 gate’s “pre-existing issues accepted” tolerance for ooxfuzz clippy becomes moot (exit 0).

---

### D3 — Corpus / `gen verify` (nightly ×4 jobs + pr.yml validate)

Root causes measured:

1. **Schema** (`corpus/schema/fixture.schema.json` `allOf[2]`): `format: pdf → source must be
   "curated"`, but Phase-5 added **12 generated PDFs** (`source: generated`, with `generator`).
2. **Budget** (`harness/generator/gen/core.py` `BUDGET`): missing 15 `(cat, fmt)` keys —
   6 `revision-*`/`docx` + 9 pdf id-prefix/`pdf`. `select()` iterates BUDGET → `gen build`
   **skips revision fixtures** → nightly full verify reports `blob missing`.
3. **Taxonomy** (`corpus/PLAN.md` §2): 18 manifest categories absent → pr.yml taxonomy lint fails.
4. **PDF id↔category mismatch**: all 12 PDF manifests violate the generated-fixture rule
   (`categories == [id-prefix]`): e.g. `comment-annot-pdf-001` parses id-cat `comment-annot` but
   carries `categories: ["pdf-annot-comment"]`. Schema validation `continue`s before these checks
   today; they will surface the moment the schema is fixed.
5. **Full-verify extras**: 6 revision manifests have `features: {}` vs non-empty recount;
   `tracked-changes-docx-001: sha256 drift` (1).

#### D3.1 — Owner decision: schema relaxation vs manifest change *(👤)* · S

Options:
- **(A, recommended)** Relax `allOf[2]` — drop the `format:pdf → source:curated` clause (or scope
  it to `pdf-external` category only). Backward-compatible (all currently-valid manifests stay
  valid) → `schema_version` stays 1, C9 **minor** bump + note in `docs/contracts/C9.md` (freeze
  rule: minor bumps additive-only post-M0; a relaxation is compatible). Update the `format`
  property description (“Phase-0 generated corpus has no pdf inputs” → now stale).
- **(B)** Mark the 12 PDFs `source: curated` — wrong (they are generated; curated requires
  `origin` license review per PLAN §5). Not recommended.
- Owner confirms (A); D3.2 executes.

#### D3.2 — Schema + PLAN + BUDGET sync · M

- Apply D3.1(A) to `corpus/schema/fixture.schema.json`; bump C9 contract version + CHANGELOG row.
- **PDF ids**: rename 12 manifests+references+blobs so id-prefix == single category —
  `comment-annot-pdf-001` → `pdf-annot-comment-pdf-001`, … `pdfa-basic-pdf-001` →
  `pdf-a-pdf-001` (or keep id and change category — decide once; renaming ids keeps the
  `pdf-*` namespace and satisfies `_no_blob_problems`). Update:
  - `harness/generator/gen/pdf_fixtures.py` (fixture ids + `MANIFESTS.glob` patterns),
  - `tools/p5_02_verify.sh` (hard-coded ids `highlight-annot-pdf-001` etc. + globs),
  - any `adapters-pdf`/`mcp` tests referencing old ids (grep shows only
    `mcp/src/http.rs:2307` sample path — verify).
- **BUDGET** (`gen/core.py`): add 6 revision keys `(revision-*, docx): 1` and 9 pdf keys
  (id-prefix after rename, `pdf`: 1 or 2 for multipage). `budget_total` 151 → 167.
- **PLAN.md §2**: add rows for 6 `revision-*` + 9 `pdf-*` categories (taxonomy lint source).
- **PLAN.md §3**: add matching budget rows; update Σ and the v1.1 narrative honestly (Phase-5
  added generated PDFs + Phase-4 revision fixtures after the M0 freeze).
- **Revision manifests**: after BUDGET includes them, run `gen build --category revision-…`
  (builders already registered) to rewrite manifests with correct `features`; investigate +
  fix `tracked-changes-docx-001` sha256 drift (rebuild should rewrite both blob and manifest —
  if drift persists, diff `feature_counts` vs stored sha inputs).
- Acceptance: `python3 -m gen verify --no-blobs` **0 problems**; `python3 -m gen verify`
  (after `gen build`) **0 problems**; `python3 -m pytest harness/metrics/tests -q` still 40 pass;
  pr.yml taxonomy + PR-subset steps pass.

#### D3.3 — Wire PDF generation into `gen build` · M

- `gen build` currently has no PDF builders (`BUILDERS` has 0 pdf keys; `--format` choices omit
  `pdf`); PDFs come only from running `gen/pdf_fixtures.py` manually. Register the
  `pdf_fixtures` fixture classes as `@builder(...)` entries (or call `generate_all()` from
  `build_all`), add `pdf` to `--format` choices.
- Nightly `harness` job then needs no extra step: `python3 -m gen build` produces all 169 blobs.
- Acceptance: fresh clone → `python3 -m gen build` → `ls corpus/blobs | wc -l` = 169 →
  `gen verify` 0 problems → `git diff --exit-code -- corpus/manifests corpus/reference` clean.

#### D3.4 — CI: generate blobs before Rust tests · M

- Blob-dependent tests (29 ooxml + 10 odf without blobs) panic in any clean checkout. Add to
  phase3/4/5/6 workflows (or at gate-script start, `--ci` mode):
  1. `pip install -r harness/requirements-dev.txt`
  2. `python3 -m gen build` (`working-directory: harness/generator`) — measured **~6 s** local.
- This restores real corpus coverage in CI (48 fails → 13 known fails, matching local/AGENTS).
- Alternative (not recommended): skip-if-absent in each test — vacuous in CI; project’s
  honest-absent pattern does not fit “corpus regression” tests.
- Acceptance: clean-clone CI `cargo test --workspace --lib` failure count == 13 (known list).

---

### D4 — Workflow YAML / expression fixes (make GitHub accept the files)

#### D4.1 — `phase5-gate.yml` parse error *(blocks Dependabot too)* · S

Quote the four step names containing colons (lines 22/24/26/28):
`- name: "Gate 0: intake hygiene"` etc. Acceptance: `python3 -c "import yaml; yaml.safe_load(...)"`
and `actionlint` clean; next Dependabot `github_actions` run no longer reports
`dependency_file_not_parseable`.

#### D4.2 — `pr.yml` job-level `hashFiles` · S

Remove the 4 job-level `if: ${{ hashFiles('Cargo.toml') != '' }}` (lines 146/164/177/195) —
the “self-activating at CORE-01” placeholder is obsolete (workspace exists since M0) and
actionlint/GitHub reject `hashFiles` at job level → whole workflow “workflow file issue”, 0 jobs.
Also remove the now-pointless step-level conditions (128/140) + stale header comments.
Acceptance: `actionlint .github/workflows/pr.yml` 0 errors; push → pr.yml run creates jobs.

#### D4.3 — `docker-profile` build context · S

`docker/profile/build.sh`: `docker build --file "${SCRIPT_DIR}/Dockerfile" --tag "${IMAGE}"
"${SCRIPT_DIR}/../.."` (context = repo root). Also `COPY Cargo.lock` + `rust-toolchain.toml`
in the Dockerfile build stage (reproducible deps/toolchain), and align base image with
`rust-toolchain.toml` (`1.98`) — current `rust:1.85-slim` only matches `rust-version` MSRV.
Acceptance: `bash docker/profile/build.sh` exits 0 (doctor all green).

#### D4.4 — Action/toolchain consistency · S

- Bump `actions/checkout@v4` → `@v7` and `actions/cache@v4` → `@v6` (or current) in
  phase3/4/5/6, docker-profile, release (removes Node-20 deprecation warnings; nightly/pr
  already current).
- Pin `dtolnay/rust-toolchain@stable` → `dtolnay/rust-toolchain@master` + `toolchain: 1.98`
  in phase3-gate.yml / phase4-gate.yml (match phase5/6 and `rust-toolchain.toml`).
- `release.yml:60` actionlint expression warning (`steps.version.outputs.version` used in the
  same step that defines it) — restructure to compute then echo, or ignore with a comment if
  GitHub accepts it at runtime (verify on next release rehearsal).

#### D4.5 — Validate all workflows · S

`actionlint` over `.github/workflows/` → 0 errors (or documented exceptions). Note:
`nightly.yml` already quotes its colon-containing step names — phase5 was the outlier.

---

### D5 — Gate scripts: surface failures + blob bootstrap

#### D5.1 — Print test output on failure · M

`tools/phase3_gate.sh`, `phase4_gate.sh`, `phase5_gate.sh`, `phase6_gate.sh`: when a
`run_check`/redirected command fails, print `tail -40 "$LOG"` before `fail`. Today the
`cargo test -p adapters-ooxfuzz` CI failure (phase4 + phase6) is invisible — this is the
prerequisite for diagnosing D6.

#### D5.2 — Blob bootstrap inside gates · M

If D3.4 puts `gen build` in the workflows, also add it to gate `--ci` paths (gate is run
locally too, and local clean clones fail the same way). Order: pip deps → `gen build` →
`cargo build -p stichos` (already present, R-D1) → checks.

#### D5.3 — phase6 cache hygiene · S

`phase6-gate.yml` caches `target/` keyed only on `OS + Cargo.lock` — stale/corrupt caches can
survive interrupted saves and cause mystery test failures. Either drop `target/` from the cache
(keep registry/git only, like phase5) or add rustc version + source hash to the key.
Acceptance: fresh CI run after cache clear is green.

---

### D6 — `adapters-ooxfuzz` CI-only test failure (diagnose → fix)

**Symptom:** `cargo test -p adapters-ooxfuzz` exit 101 on GitHub (phase4 run 06:56:53→06:58:21;
phase6 `--lib` also failed) while **226/0 pass locally in a clean worktree without blobs**
(verified twice, plus `--doc` 0 tests, `--test-threads=4` OK). No blob/env/date dependence found
in ooxfuzz sources.

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| D6.1 | gate (D5.1) | Re-run failing CI with log-tail printing; capture the actual panic/test name. | S |
| D6.2 | hypotheses to check against the log | ① OOM/stack overflow on 2-core/7 GiB runner (`Vec::with_capacity(100_000)` + parallel debug tests → try `--test-threads=1/2` or `RUST_TEST_THREADS`); ② phase6 restored corrupt `target/` cache (D5.3); ③ phase4 `rust-toolchain@stable` skew (D4.4). | S |
| D6.3 | fix per diagnosis | e.g. `cargo test -p adapters-ooxfuzz -- --test-threads=2` in gates, or fix the offending test. Acceptance: phase4 + phase6 CI green on this check. | S–M |

---

### D7 — Evidence + green CI *(👤 for push)*

| Sub | Scope | Deliverable / acceptance |
|---|---|---|
| D7.1 | local gate | `cargo fmt --check` 0 · `cargo clippy --workspace --all-targets -- -D warnings` 0 · `cargo clippy --workspace --lib -- -D warnings` 0 · `cargo test --workspace --lib` 13 known fails only · `gen verify` + `--no-blobs` 0 · `check-deps.sh` 0 · `phase6_gate.sh --ci` **90/90** · `phase3_gate.sh --ci` / `phase4_gate.sh --ci` / `phase5_gate.sh --ci` exit 0. |
| D7.2 | actionlint | all 8 workflow files clean. |
| D7.3 | 👤 commit + push | Single commit (or stacked: code fixes → corpus/CI) on `main`; push; watch `pr.yml`, `phase3/4/5/6-gate`, `docker-profile` go green. |
| D7.4 | nightly 👤 | `workflow_dispatch` the nightly: `harness` + `platforms ×3` green; `gen verify` 0 problems; trend row artifact uploads. |
| D7.5 | Dependabot | confirm `github_actions` update job succeeds after D4.1. |
| D7.6 | docs | Update `docs/remaining-work.md` (Track D rows), `docs/KNOWN_ISSUES.md` (blob-absent CI → fixed; pr.yml/phase5-gate workflow-file issues → fixed), `AGENTS.md` state snapshot, CHANGELOG. |

---

## Execution order (dependencies)

```
D0 (commit base, 👤)
 └─ D1 fmt ─ D2 clippy (D2.1 first — unblocks phase4/5) ─ D7.1 local gate
 └─ D3 corpus (D3.1 👤 → D3.2 → D3.3 → D3.4) ──┐
 └─ D4 workflows (D4.1, D4.2 independent; D4.3; D4.4) ──┤
 └─ D5 gate improvements (D5.1 → D5.2/D5.3) ─────────────┤
                                                          ▼
                                          D6 ooxfuzz CI diagnosis (needs D5.1 + push)
                                                          ▼
                                          D7 evidence + push (👤) → watch CI → nightly (👤)
```

- D4.1 is the smallest high-value fix (also unblocks Dependabot) — can ship first.
- D4.2 restores pr.yml runs; its `validate` job will then hit D3 failures — land D3 before or
  with D4.2 if a green pr.yml run is the goal in one shot.
- D5.1 before D6; D6 before declaring phase4/phase6 fully green.

## Out of scope (unchanged, tracked elsewhere)

- 13 known test fails (4 OPC zip-bomb + 9 date-hardcoded) — accepted per AGENTS.
- P7-3 tranches C/D, P7-6 redaction detail, P7-8 pdfium closeout, P7-9 launch (Track B).
- `cargo deny` `rsa` Marvin advisory (RUSTSEC-2025-0009) — pre-existing (B0.6).
- KI-02/KI-05 — active but unrelated to CI green.
