# stichos — Remaining Work (post-M6)

**Created:** 2026-09-23 (completion audit). **Supersedes nothing** — the Phase-7 intake list in
`docs/phase6-concept.md` §12 is carried here unchanged and expanded into subagent-sized tasks.

Conventions: each **task** is one deliverable; each **subtask** is scoped so a single subagent
session (~200 k-token context) can finish it end-to-end (read → change → test → report). Subtask
sizes: **S** ≈ single file / <50 LoC, **M** ≈ 2–4 files, **L** ≈ crate-spanning (split before
delegating). Verification commands assume the workspace root and `./target/debug/stichos` built.

## 0. State snapshot (measured 2026-09-23)

| Area | State |
|---|---|
| Milestones M0–M6 | all tagged; `m1-closeout..m5` pushed; **`main` (8 commits ahead) + `m6` tag local-only** |
| `phase6_gate.sh --ci` (working tree) | 86/88 — expected `m6`-tag artifact + fmt (fixed during audit) |
| `cargo test -p mcp --lib` | **90 pass / 3 fail** — 1.5.0 ceilings vs `spec.rs` 1.6.0 flip |
| C14 `__budget`+`rendered_by` on 100 % of responses | **not met** — stamped only in `cli/src/image_output.rs` |
| Gate-C defaults flip | in working tree, **uncommitted** (`--apply` opt-out, `dry_run=true`, URLs/summary defaults, spec 1.6.0) |
| Gate checks 1b/1d/1e/1f | **toothless** — `pass` on both branches; 1d/1c wrong shape/flags |
| `p606_ctx06_compliance.rs` | weakened during flip (envelope/cap asserts replaced by presence checks) |
| Contract docs | `C7.md` 1.5.0 (matrix pending); `C14` status `draft`; C15 + ADR-0011 frozen |
| `check-deps.sh` | green (Wave 0 edge removed) |

Full audit detail: `docs/m6-acceptance.md` §11.

---

## Track A — M6 closeout (gate integrity; blocks pushing `main` + `m6`)

### T-A — C7 v1.6 freeze consistency  *(do first; unblocks the red tests)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| T-A1 | `crates/mcp/src/spec.rs` tests only | Update `test_all_tools_schema_version` expected map (10 tools → `1.6.0`), `test_apply_patch_is_non_idempotent` (1.6.0), `test_verify_c7_compliance` version ceiling to admit `1.6.0`. Green: `cargo test -p mcp --lib` 93/93. | S |
| T-A2 | `docs/contracts/C7.md` | Bump header Version → 1.6.0; matrix rows for the 10 flipped tools → 1.6.0; add "Tools (v1.6.0 additive — C14 context budget)" section listing the 9 default flips + opt-outs (mirror CHANGELOG). Cross-check against `spec.rs` (source of truth). | M |
| T-A3 | `docs/contracts/C14-context-budget.md` | Status `draft` → `frozen — M6 (C7 v1.6)`; tick the freeze checklist items that are actually landed; leave the envelope-on-all-responses item **unchecked** with a pointer to T-B. | S |

### T-B — C14 envelope wiring: `__budget` + `diagnostics` + `rendered_by` on 100 % of responses *(the headline gap; D27)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| T-B1 | `gp-core/src/envelope.rs` (helper), `crates/cli/src/main.rs` + per-verb output points | Single CLI-side stamping helper (reuse `ResponseBudget`/envelope fns; `envelope.rs:382` already validates shape) applied to the JSON output of **every verb** (inspect, region, render, diff, apply, edit, annotate, revert, journal, create, script, update, revisions, annots, review, open, sign; doctor/perf/schema: decide + document). Acceptance: smoke script runs each verb `--json` on a corpus fixture and asserts the three envelope keys. | L — delegate per verb-group (read verbs / mutation verbs / misc) after the helper lands |
| T-B2 | `crates/cli/src/main.rs` `cmd_serve` handler arms (MCP responses are assembled there) | Stamp the envelope on all **23 tool responses** at the serve boundary (do not add an `adapters-*` dep to `mcp` — keep the Wave-0 DAG). Acceptance: `tools/mcp_envelope_smoke.sh` (new, stdlib JSON-RPC over stdio) asserts envelope keys on all 23 tools. | L — may split 12+11 |
| T-B3 | `crates/cli/tests/p606_ctx06_compliance.rs` | Restore strict asserts: tokens from `__budget.tokens_est` (estimator, not byte heuristic) `< 500` on `long-tables-docx-001`; URL purity needs CLI `region --output urls --json` — add the flags to the `region` verb (small `main.rs` change, mirror `image_output` routing) then assert zero base64 substring + `image.url`/`expiresAt` + file exists; diff: `ir_diff.ops ≤ 20` + `hasMore` + `summary.totalOperations`; journal cap + `totalEntries`; preview snake_case wire shape + source byte-identity. All 6 green. | M |
| T-B4 | gate + docs | `phase6_gate.sh` new check: corpus `inspect` summary `__budget.tokens_est < 500` **fails** when violated (see T-C1). Note measured tokens on the 50-page fixture in `docs/phase6-status.json` gate-C evidence. | S |

### T-C — Gate hardening (make `phase6_gate.sh` actually measure)

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| T-C1 | `tools/phase6_gate.sh` §1b/1d/1e/1f | Replace `else pass` with `else fail` (or an explicit, greppable `SKIP:` state reserved for host-missing cases only); fix 1d to read `ir_diff.ops`/`ir_diff.hasMore`; 1c to use the flags from T-B3; keep 1f asserting all three keys present. Acceptance: `--planted-violation` mode demonstrates each fixed check flips to FAIL. | M |
| T-C2 | `tools/phase*_gate.sh` (0–5) | Audit for the same both-branches-pass pattern; fix or file as KNOWN_ISSUES entries. Acceptance: diff review + gate re-runs unchanged-green. | M |
| T-C3 | `tools/phase6_gate.sh` Gate-3 filter | Broaden `cargo test -p mcp --lib ctx06` → full `cargo test -p mcp --lib` so ceiling tests can't hide. | S |

### T-D — Commit, re-tag, push *(human-gated steps marked 👤)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| T-D1 | whole tree | Re-verify: `cargo build --workspace`, `cargo test --workspace --lib` (known OPC zip-bomb failures accepted), `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`, `bash tools/check-deps.sh`, `bash tools/phase6_gate.sh --ci`. Update `docs/phase6-status.json` (gate-C evidence incl. real measured tokens) + append §12 note to `docs/m6-acceptance.md` referencing this doc. | M |
| T-D2 | git 👤 | Commit the Gate-C flip + T-A/T-B/T-C fixes; **re-anchor tag `m6`** to the committed gate-green tree (repo rule: tags move only to committed gate-green trees; precedent `docs/m4-acceptance.md` Git-State). | S |
| T-D3 | git 👤 (owner approval, D26) | `git push origin main` + `git push origin m6` + remaining tags; confirm via `git ls-remote`. | S |

### T-E — Housekeeping

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| T-E1 | 8 open dependabot PRs (refs/pull/1..8) | Triage: check CI, merge or close with reason; `actions/checkout-7` (branch `tmp-7`) first. | M |
| T-E2 | `docs/KNOWN_ISSUES.md`, `docs/support-matrix.md` | Add post-M6 rows: toothless-gate history (fixed via T-C), envelope closeout status; verify support matrix matches shipped behavior after T-B. | S |

**Track-A order:** T-A1 → T-C3 (test the tests) → T-B1 → T-B3/T-B2 (parallelizable) → T-A2/T-A3 → T-C1/C2 → T-B4 → T-E → T-D1 → T-D2 👤 → T-D3 👤.

---

## Track B — Phase-7 backlog (from `docs/phase6-concept.md` §12; not gated on Track A)

Each item opens with an ADR/scope step if it touches a new dependency (D2) or owner-gated surface.

| Task | Description | First subtask (context starter) | Size | Gate |
|---|---|---|---|---|
| P7-1 xlsx create | `create` verb → content-driven xlsx from IR (docx create exists since M3) | Audit `adapters-ooxml` creation writer + C1 Spreadsheet IR; draft spec section | L — split: sheet/cell writers / styles / round-trip corpus | C1 |
| P7-2 pptx create | same for pptx | same pattern | L — split: slide/text/table writers / layouts | C1 |
| P7-3 full formula coverage | Extend C15 beyond the 12-function subset (date/time, financial, statistical, text) driven by the parity-lint gap list | Run parity lint over corpus; rank gaps by frequency; propose next tranche ADR-0011 amendment | M per tranche | C15 amendment |
| P7-4 lint encode/ssim refresh | Re-triage "deferred to Phase 2" notes: `lint::encode` fidelity tests, `lint::ssim` with updated baselines | Inventory deferred notes in `crates/lint`; refresh baselines against goldens | M | C9, REN-02 |
| P7-5 PAdES-T/LTV | Timestamps + LTV via OCSP/CRL + trusted certs beyond B-B | Draft ADR (crypto deps — owner-gated D2; ADR-0010 reserved) **before code** | L, ADR first | owner + C13 |
| P7-6 redaction | PDF redaction / content-stream surgery | Draft ADR (D2/D19) before any parser work | L, ADR first | owner |
| P7-7 real CMS signing | Replace `signature.rs` stub with real PKCS#7/CMS | ADR-0010 crypto-dependency decision 👤 (owner call C12) — task blocked until decided | L | owner |
| P7-8 pdfium (gate G5) | Bundled pdfium raster rung experiment; G5 closes if adopted or documented-rejected | Spike: `pdfium-render` behind feature flag; measure word-bbox quality vs current ladder | M | G5 / D19 |
| P7-9 launch | D18/D26 flip: registry publishes, visibility | Owner action with its own checklist (`docs/release-playbook.md`) — not delegable | — | owner 👤 |

---

## Notes for subagents

- Read first: `AGENTS.md` (rules), `docs/m6-acceptance.md` §11 (audit), this file, the task's scope files.
- Never hand-edit `dist/icons/`; brand rules in `AGENTS.md` apply to any UI/doc artifact.
- Keep the DAG: `mcp` depends on `gp-core` only; new adapter/engine knowledge goes through
  `gp-core::traits` and is wired in `cli` (Wave-0 precedent, `PdfTextSource`).
- Contract changes are additive-only (C7); any default flip needs a version bump + CHANGELOG + opt-out.
- Do not commit/push — hand back the patch; commits are T-D2/T-D3 👤 steps.
- Report back: files touched, test commands + tails, measured numbers (token counts, op counts), anything that contradicts this doc.
