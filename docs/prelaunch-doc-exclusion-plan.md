# Plan: Pre-launch documentation exclusion (Option A)

**For:** executing agent (single, sequential run)
**Mode:** Option A — `git rm --cached` + local-only `.git/info/exclude`
**Exception:** `AGENTS.md` and `CODEBASE.md` stay tracked and public, reworked to current-state-only
**Do not push until Step 8 (owner-gated).**

---

## Goal

Remove prior-development-process documentation from the public GitHub repository while keeping
every file on the local disk and out of future commits. No history rewrite.

## Scope

### Excluded from the index (stay on disk, local-only thereafter)

```
docs/phase0-execution.md
docs/phase1-concept.md
docs/phase1-plan.md
docs/phase1-runbook.md
docs/phase1-status.json
docs/phase2-concept.md
docs/phase2-plan.md
docs/phase2-status.json
docs/phase3-concept.md
docs/phase3-plan.md
docs/phase3-status.json
docs/phase4-concept.md
docs/phase4-plan.md
docs/phase4-status.json
docs/phase5-concept.md
docs/phase5-plan.md
docs/phase5-status.json
docs/phase6-concept.md
docs/phase6-plan.md
docs/phase6-status.json
docs/m2-acceptance.md
docs/m3-acceptance.md
docs/m4-acceptance.md
docs/m5-acceptance.md
docs/m6-acceptance.md
docs/track-b-p7-1-xlsx-create.md
docs/track-b-p7-2-pptx-create.md
docs/track-b-p7-3-formula-coverage.md
docs/track-b-p7-4-lint-encode-ssim.md
docs/track-b-p7-7-cms-signing.md
docs/track-b-p7-8-pdfium-spike.md
docs/track-b-p7-9-launch.md
docs/track-d-ci-green-plan.md
docs/track-h-generic-office-handling.md
docs/remaining-work.md
docs/finalization-plan.md
docs/harness/upgrade-gate.md
project_plan.md
corpus/PLAN.md
corpus/GENERATION_REPORT.md
docs/prelaunch-doc-exclusion-plan.md        (this plan file)
```

### Kept tracked and public (do not touch, except the two reworks)

- `AGENTS.md`, `CODEBASE.md` — **rework** in Steps 2–3, then keep
- `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`,
  `LICENSE-MIT`, `LICENSE-APACHE`, `brand.md`, `project_concept.md`
- `docs/threat-model.md`, `docs/support-matrix.md`, `docs/release-playbook.md`,
  `docs/KNOWN_ISSUES.md`, `docs/creation-measurement.md` (+ `.csv`), `docs/font-bundle-audit.md`,
  `docs/addin-spike.md`, `docs/implementation-guide-context-budget.md`
- `docs/adr/**`, `docs/contracts/**`, `docs/gov/**`, `docs/harness/**` except `upgrade-gate.md`
- Everything else not in the exclusion list

### Constraints

1. Never delete files from disk (`git rm --cached` only, never plain `git rm`/`rm`).
2. No history rewrite: no `filter-repo`, no `rebase`, no amend of prior commits.
3. Excluded docs remain reachable in **existing history** on GitHub — accepted; tip-only exclusion.
4. `.git/info/exclude` is local and unversioned — never copy its contents into `.gitignore`.
5. Run steps strictly in order; one commit does not begin before the previous step verifies green.

---

## Step 0 — Preflight

```bash
cd /localdata/workspace/stichos
git status --porcelain          # expect the known dirty tree (16 modified + 2 untracked)
git branch --show-verify main   # must be on main
```

Record the porcelain output. If unexpected entries appear beyond the known set, stop and report.

Known dirty set (16 modified):
`AGENTS.md CODEBASE.md crates/adapters-ooxfuzz/src/formula_fuzz.rs crates/cli/src/render.rs
crates/engines/src/raster.rs crates/gp-core/src/formula/evaluator.rs
crates/gp-core/src/formula/parser.rs crates/gp-core/src/formula/refuse.rs
crates/lint/src/formula_parity.rs docs/KNOWN_ISSUES.md docs/finalization-plan.md
docs/release-playbook.md docs/remaining-work.md docs/track-b-p7-9-launch.md
project_concept.md tools/phase5_gate.sh`
Known untracked (needed, not clutter):
`corpus/fixtures/formula-parity/ tools/parity_formula.sh`

## Step 1 — Preserve all WIP in a commit first

The exclusion must not swallow uncommitted work. Stage and commit everything as-is:

```bash
git add -A
git commit -m "prelaunch: land finalization WIP (Track F/I/G, E1a, doc syncs, formula-parity fixtures)"
```

Verify: `git status --porcelain` is empty. If commit hooks fail, fix and retry — do not skip.

## Step 2 — Rework `AGENTS.md` (current state only, no history)

Rewrite `AGENTS.md` to this structure. **Present tense only.** No dates, no commit hashes, no
milestone tags, no "shipped/closeout/re-anchored/proposed/declined" narrative, no references to
any file in the exclusion list.

```
# stichos

<1 short paragraph: what the product is; current state — feature-complete, version 0.1.0,
pre-first-release; workspace layout location (`crates/`); supporting dirs harness/corpus/docker/
baseline/.github. No phase history.>

## Source of truth
(keep existing bullets; drop any reference to remaining-work.md, phase*/track* docs, or project_plan.md)

## Voice and CLI verbs
(keep the existing voice rules and the 24-verb list — they are current facts)

## Asset pipeline
(keep as-is)

## Brand rules agents break
(keep as-is)

## Workspace
(the 9-crate table; rename heading from "Phase-5 tech" to "Workspace"; keep table content;
 drop any "C11/M4/M6/P7-5" historical ticket annotations if you trim, or keep contract letters
 as current references only — no dates)

## Development
(current commands only: cargo test --workspace --lib, cargo fmt, cargo clippy --all-targets
 -- -D warnings, tools/check-deps.sh, tools/phase*_gate.sh as available gates; engines spawned
 as subprocesses note)

## Current status
(present facts: test/gate counts **re-measured by you in this step** via
 `cargo test --workspace --lib` and `cargo test -p mcp --lib`; launch not yet cut (no v* tag,
 version 0.1.0); repo private until owner launch. No dates, no tag list, no phase narrative.)
```

Hard rules for the rework:
- Remove the entire `## Phase status` section (all of it).
- Remove the Track H paragraph (declined/adopted/branch story). If verbs/modules need describing,
  describe only what is in the tree now (24 verbs incl. `validate`, `finish`).
- Do not mention `m1-closeout`..`m6`, `track-h-proposed`, or any tag.
- Keep the file concise: target ≤ 70 lines.

## Step 3 — Rework `CODEBASE.md` (current state only, no history)

Keep the document's structure (workspace layout, dependency graph, crate details, harness, corpus,
docs, assets, test table, naming) but scrub history:

1. **Header (lines 3–13):** replace entirely with one line, e.g.
   `> Reflects the current tree. Maintained by contributors.`
   Delete all sync dates, commit hashes, Track H dual-state narration, remaining-work rev notes,
   and acceptance-doc references.
2. **Track H annotations:** delete every `*(branch track-h-proposed, Track H — proposed; not
   adopted)*` aside (`file_access`, `opc_validate`, `potx`). Describe those modules in plain
   present tense if they exist in the tree (they do — verify with `ls`), else drop the row.
3. **Verb/tool counts:** state only current reality. Verify before writing:
   - `grep -c '"' crates/mcp/src/spec.rs` tool specs / check `TOOLS.len()` assertion — write the
     single current number (expected 24), never "22 at HEAD / 24 in proposed".
   - CLI subcommands: list the current set only (expected 24 incl. `validate`, `finish`).
4. **Contract versions:** read `docs/contracts/C7.md` and `crates/mcp/src/spec.rs`; write only the
   current version(s). No "T-A2 landed ff80f8e"-style annotations.
5. **Dependency graph heading:** drop `(measured 2026-09-16)`; drop the sentence referencing
   `project_plan.md` §3.2 (file will be excluded) — replace with "enforced by `tools/check-deps.sh`".
6. **Known DAG deviations:** delete the section (pure history). Optionally state the current
   invariant under Development: `tools/check-deps.sh` exits 0.
7. **Test coverage table:** re-run `cargo test --workspace --lib` (and `-p mcp --lib`) **after
   Step 1**, then write fresh numbers. Strip the heading date, "after finalization", "Track I
   fixed", "previously 9 …" and similar historical Notes — Notes column becomes current facts only
   (or empty).
8. **Docs section:** list only files that remain tracked (drop every excluded path from the tree
   diagram). Same for any mention of `remaining-work.md`, `phase*`, `track-*`, `m*-acceptance`,
   `project_plan.md`, `corpus/PLAN.md`, `corpus/GENERATION_REPORT.md`.
9. **Corpus section:** drop the two excluded markdown filenames; keep factual fixture counts if
   still accurate.
10. Remove any remaining dates (`20xx-xx-xx`), tag names, commit hashes — final grep must be clean.

## Step 4 — Stage the two reworked files (they are part of the exclusion commit)

```bash
git add AGENTS.md CODEBASE.md
```

## Step 5 — `git rm --cached` the exclusion list

```bash
git rm --cached -- \
  docs/phase0-execution.md \
  docs/phase1-concept.md docs/phase1-plan.md docs/phase1-runbook.md docs/phase1-status.json \
  docs/phase2-concept.md docs/phase2-plan.md docs/phase2-status.json \
  docs/phase3-concept.md docs/phase3-plan.md docs/phase3-status.json \
  docs/phase4-concept.md docs/phase4-plan.md docs/phase4-status.json \
  docs/phase5-concept.md docs/phase5-plan.md docs/phase5-status.json \
  docs/phase6-concept.md docs/phase6-plan.md docs/phase6-status.json \
  docs/m2-acceptance.md docs/m3-acceptance.md docs/m4-acceptance.md \
  docs/m5-acceptance.md docs/m6-acceptance.md \
  docs/track-b-p7-1-xlsx-create.md docs/track-b-p7-2-pptx-create.md \
  docs/track-b-p7-3-formula-coverage.md docs/track-b-p7-4-lint-encode-ssim.md \
  docs/track-b-p7-7-cms-signing.md docs/track-b-p7-8-pdfium-spike.md \
  docs/track-b-p7-9-launch.md \
  docs/track-d-ci-green-plan.md docs/track-h-generic-office-handling.md \
  docs/remaining-work.md docs/finalization-plan.md docs/harness/upgrade-gate.md \
  project_plan.md \
  corpus/PLAN.md corpus/GENERATION_REPORT.md \
  docs/prelaunch-doc-exclusion-plan.md
```

Every path must succeed (all are tracked). Any `error: ... did not match` means the list is stale —
stop and report; do not improvise extra removals.

## Step 6 — Write local-only ignore rules

Append this block to `.git/info/exclude` (preserve the existing default comment lines):

```
# Pre-launch documentation exclusion (Option A) — prior-development records, local-only.
# Collaborators must replicate this block in their own .git/info/exclude.
/docs/phase*.md
/docs/phase*-status.json
/docs/m*-acceptance.md
/docs/track-b-p7-*.md
/docs/track-d-ci-green-plan.md
/docs/track-h-generic-office-handling.md
/docs/remaining-work.md
/docs/finalization-plan.md
/docs/harness/upgrade-gate.md
/docs/prelaunch-doc-exclusion-plan.md
/project_plan.md
/corpus/PLAN.md
/corpus/GENERATION_REPORT.md
```

Do **not** touch `.gitignore`.

## Step 7 — Commit the exclusion

```bash
git add -u   # staged deletions + AGENTS.md + CODEBASE.md; must NOT add excluded files back
git commit -m "chore: exclude prior-development docs from the public tree (Option A); rework AGENTS.md/CODEBASE.md to current state"
git status --porcelain    # must be empty
```

If `git status` shows excluded files as untracked-but-not-ignored, the `.git/info/exclude` block
did not apply — fix before committing.

## Step 8 — Verification (all must pass)

```bash
# 1. Files still exist on disk
for f in docs/remaining-work.md docs/finalization-plan.md project_plan.md \
         docs/phase6-plan.md docs/m6-acceptance.md docs/prelaunch-doc-exclusion-plan.md; do
  test -f "$f" || echo "MISSING $f"
done

# 2. No longer tracked
git ls-files -- docs/ project_plan.md corpus/PLAN.md corpus/GENERATION_REPORT.md \
  | grep -E 'phase[0-9]|m[0-9]-acceptance|track-b-p7|track-d-ci|track-h-|remaining-work|finalization|upgrade-gate|PLAN.md|GENERATION_REPORT|prelaunch-doc' \
  && echo "FAIL: still tracked" || echo "OK: untracked"

# 3. Ignored locally via .git/info/exclude (not .gitignore)
git check-ignore -v docs/remaining-work.md project_plan.md docs/prelaunch-doc-exclusion-plan.md

# 4. AGENTS.md + CODEBASE.md still tracked
git ls-files --error-unmatch AGENTS.md CODEBASE.md

# 5. No history leaked into the two reworked files
grep -nE '20[0-9]{2}-|shipped|closeout|re-anchored|remaining-work|track-h-proposed|m[0-9]-acceptance|phase[0-9]-(plan|concept|status)|\bm[0-6]\b|ff80f8e|af40e44|--ci.*9[0-9]' \
  AGENTS.md CODEBASE.md && echo "FAIL: history present" || echo "OK: no history"

# 6. Clean tree
git status --porcelain   # empty

# 7. Repo health (docs-only change; should match Step 2/3 re-measure)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

Record pass/fail per check. Any FAIL: fix and re-run; do not proceed to Step 9.

## Step 9 — Push (OWNER-GATED — do not run unprompted)

Pushing publishes everything currently on local `main` (the 3 previously unpushed commits, the
Step 1 WIP commit, and the exclusion commit) to `origin/main`. **Ask the owner for explicit
approval first**, and remind them:

- the Track-H-state contradiction becomes public on `origin/main` (local `main` contains
  Track-H code while governance docs elsewhere said declined);
- excluded files remain visible in older commits on GitHub (tip-only exclusion).

```bash
git push origin main
```

If the owner declines: stop here; the exclusion is complete locally.

---

## Out of scope (not part of this plan)

- History rewrite / purge of excluded docs from past commits.
- Removing the two tracked scratch PNGs (`page-1.png`, `crates/engines/page-1.png`) — previously
  flagged; owner may schedule separately via `git rm`.
- `.gitignore` hardening (`.pytest_cache/`, `*.tmp`, `.DS_Store`, …).
- Deleting on-disk caches (`__pycache__`, `target/`, `goldens/`, `out/`).
- The P7-9 launch itself (owner-only).
- Borderline docs (`project_concept.md`, `docs/gov/`, spike/measurement docs) — **kept** by
  default; owner may add them to the list later with the same Step 5–7 procedure.
