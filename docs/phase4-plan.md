# stichos — Phase 4 plan: agent-sized steps (M4)

**Status:** active plan · derived from [`phase4-concept.md`](phase4-concept.md) (scope, gate) and
`project_plan.md` §5 tracks (PAT, API, ODF, CORE, ENG, SEC, OPS, HUM) ·
**21 steps, 6 waves (0, R, O, S, H, X), 6 gates**.
**Canonical progress state:** [`phase4-status.json`](phase4-status.json) — this file defines the steps;
the JSON tracks them. Do not maintain status in two places.
**Last updated:** 2026-09-16

---

## 1. How this plan is executed (agent rules)

- **One agent, one step, one session.** Budget ~200k tokens. Load only: your step row + wave gate here,
  the named contracts in `docs/contracts/` + ADRs, the crate(s) you touch, the named corpus subset.
  No repo-wide archaeology: `CODEBASE.md` maps the tree; fetch the specific ECMA-376/OASIS clause you
  need (revision sections §17.13.5.15 `w:ins`/`w:del`; ODF `text:inserted`/`text:deleted`,
  `office:change-info`), not the spec.
- **Sizing rule:** every row is scoped to one 200k session (≈ ≤1 200 new LOC + tests). **2s** rows =
  two sequential sessions with a written handoff in the step's status entry. If a step grows past
  scope anyway, stop, record `blocked: split-needed` + evidence; do not sprawl.
- Write only inside the step's deliverable paths + its row in `phase4-status.json`.
- `project_concept.md` / `project_plan.md` / `phase4-concept.md` are **not edited by executing
  agents**; a contradiction stops the step and flags the owner. Measured vs assumed, always.
- Definition of done: `project_plan.md` §8 + the step's **Verify** column, run and recorded with
  command + result + date.
- Status: `pending → active → verified | blocked(reason)`.
- **Headless discipline (new, from the M3 audit):** every verify command must produce its verdict
  from **exit codes**, not from grepping piped output, and must work without `/dev/tty` (this
  project's agent shells have none). Gate scripts inherit this rule (P4-01).
- **Host facts (measured 2026-09-16):** pandoc 3.7.0.2, chromium-browser, soffice 26.2.6.1, xmllint,
  pdftoppm present; **typst, qpdf, verapdf absent** (CI installs; honest-absent paths must stay).
  No Windows host; no `/dev/tty`/`script` binary. Corpus: 151 fixtures (69 ODF); revision fixtures
  are created by P4-07 (none exist yet).
- Parallel lanes: R ∥ O ∥ (P4-15, P4-16) after Wave 0; S depends on R (needs C11 + accept/reject).
  `gp-core/executor.rs` and `docs/contracts/C7.md` are collision hotspots — sequential edits only.

## 2. Waves and gates

| Wave | Theme | Gate (command + pass condition) |
|---|---|---|
| **0** | ship-state hygiene | gates run headless: `bash tools/phase3_gate.sh` exit 0 **over ssh/non-tty** (P4-01); DAG check enforces the ratified policy and fails a planted edge (P4-02); clippy/fmt/deny/test clean |
| **R** | OOXML revisions kernel + surface | `stichos revisions list/accept/reject` e2e on fixture; lossless suite green (byte tier for own revisions); C7 v1.3 matrix green |
| **O** | ODF revisions | odt revision write read-back (own reader + LO golden); odt accept/reject lossless subset green |
| **S** | review session + overlay | headless HTTP e2e: create → list → accept via token → doc updated; security suite (traversal/token/CSP/XSS) green; comment write-back read-back + golden |
| **H** | handoff + COM design + add-in spike | `open` host-verified + honest-absent golden; ADR-0008 **accepted**; windows-msvc compile job green in CI; spike report published |
| **X** | security, ops, ship | `tools/phase4_gate.sh` green **headless** + CI (== M4 exit, concept §7); fuzz round-3 regressions locked; C7 v1.3 additive scan; tag `m4` |

Waves ordered; inside a wave run in Deps order; parallel only where deps allow and files don't collide.

## 3. Steps

`Implements` cites `project_plan.md` §5; `Verify` = evidence the step must produce.
**Size:** 1 = one 200k session, 2s = two with handoff.

### Wave 0 — ship-state hygiene (audit findings from 2026-09-16)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P4-01 | Rework `tools/phase2_gate.sh` + `tools/phase3_gate.sh` to be headless-safe: capture command output to temp logs, branch on **exit codes** (not `tee /dev/tty` + grep); keep planted-violation and `--ci` modes; keep output identical on a tty host | hygiene | — | 1 | both gates exit 0 run from a no-tty shell (`bash -c ... < /dev/null`); planted-violation still exits 1; verdicts match the 2026-09-16 re-verification (`m3-acceptance.md` §7) |
| P4-02 | DAG ratification: present the two §3.2 violations (`adapters-ooxml → lint`, `engines → adapters-ooxml`) + `cli`-as-wiring reality to the owner; execute the ratified choice — either refactor the edges out, or amend `project_plan.md` §3.2 — **and** extend `tools/check-deps.sh` with real cross-crate edge enforcement (allow-list matrix parsed from Cargo.toml, planted bad edge fails) | hygiene, plan §3.2 | — | 1 | `check-deps.sh` exit 0 on ratified tree; planted disallowed edge → exit 1; `cargo test --workspace` still 0 failures if refactor chosen |

### Wave R — OOXML revisions kernel + surface

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P4-03 | Revision model in `gp-core` (**C11**): `Revision { id, kind(insert/delete/format-change), author, date, anchored ElementId, page_hint }`, `Document.revisions` (C1 additive minor), `RevisionOp { accept, reject }` semantics table; new diagnostic codes (GP-R0xx refusals); write `docs/contracts/C11-revisions.md` incl. the **two-tier losslessness definition** and the refuse-list (move-revs, unknown kinds) | CORE (C11, C1) | 0 (wave 0) | 1 | contract doc exists; model round-trips serde; C1 additive check passes (old IR deserializes without `revisions`); codes registered in C3 registry |
| P4-04 | OOXML enumeration → C11: `w:ins`/`w:del`/`rPrChange`/`pPrChange` → `Revision`s anchored via C2 ids; nested (ins-in-del) handling; `paraId`/`textId` cross-links; **refuse** move-from/to + unknown kinds with GP-R codes (never silent-drop) | OOX, C11 | 03 | 2s | enumeration parity vs census counters on every docx fixture w/ revisions (synthetic + M2/M3 revision outputs); nested fixture handled; refused kinds asserted |
| P4-05 | OOXML accept/reject executor: accept-ins (unwrap), reject-ins (remove), accept-del (remove), reject-del (unwrap), format-change keep/discard; **span-surgical** (reuse `span_mut`, right-to-left), journaled via C6, `dry_run` = IR unified diff + zero writes; unrelated-parts byte-identical property test | PAT, OOX-09 | 04 | 2s | property tests: only touched spans change; stale-span rejects; journal ids returned; dry_run fs-assert zero writes; refusals never write |
| P4-06 | Bulk + filtered ops: `accept_all/reject_all`, by-author/by-date filters; **all-or-nothing atomicity** per call (stage in temp, commit once); C10 policy gate (deny `all` destructive unless explicitly allowed); budget assembly per C4 | PAT, SEC-04, C10 | 05 | 1 | atomicity test (mid-batch failure ⇒ original untouched); filter matrix unit tests; policy denial golden |
| P4-07 | Lossless proof suite + revision fixture generation: extend `harness/generator` with tracked-change builders (LO-produced + python-docx + our `create_as_revisions` output); asserts byte-tier (reject-all == original sha256) and IR-tier (accept-all == `--direct` result) on pr-subset-derived + generated set; LO render goldens for both states | HAR-02 ext, M4 gate | 05 | 2s | suite green in `cargo test`; ≥10 revision fixtures registered in `corpus/manifests`; goldens stored content-addressed |
| P4-08 | Surface: CLI `revisions` verb (list/accept/reject; `--id/--author/--all/--dry-run/--json`) + MCP `list_revisions`, `accept_revisions`, `reject_revisions`; bump per-tool schema_version to **1.3.0** for the three new tools; update `docs/contracts/C7.md` matrix + CHANGELOG | API-07 ext, C7 v1.3 | 06 | 1 | e2e CLI+MCP on fixtures; schema matrix green; additive-only scan passes (no existing tool modified) |

### Wave O — ODF revisions (owner call S1)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P4-09 | ODF revision **write**: edits on odt wrap into `text:inserted`/`text:deleted` with `office:change-info` (author/date); change-id allocation; `stichos apply` on odt gains `--revision-mode` (direct stays default per current behavior; document the difference) | ODF-04 ext | 0 (wave 0) | 2s | written revisions read back by our reader **and** render as tracked in LO (golden pair); zero-patch invariant intact ×69 |
| P4-10 | ODF revisions read → C11 + accept/reject: `text:inserted/deleted` → `Revision` (change-id ↔ element-id mapping strategy per C11 §anchoring); accept/reject via `adapters-odf::patch` primitives; lossless subset suite (odt fixtures from P4-07 + generated odt revisions) | ODF, C11, PAT | 03, 09 | 2s | enumeration parity on odt fixtures; accept/reject chains re-read via P1 reader; byte-tier reject on our-written odt revisions; ambiguous anchoring → GP-R refusal |

### Wave S — review session + overlay (D22 raster medium)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P4-11 | ADR-0007 (review-session model + security) + session core: `ReviewSession { token, store doc-id, version hash, revision snapshot, comment drafts, audit trail }`; lifecycle create/get/list/close; **version-hash invalidation** on doc change; `stichos review` verb (prints URL + token, `--no-browser`, refuses dirty doc) | HUM-01, ADR-0007 | 08 | 1 | session lifecycle tests; invalidation test (edit doc mid-session ⇒ 410); dirty-doc refusal; token entropy asserted |
| P4-12 | HTTP review surface on `mcp/http.rs`: `GET /review/<token>/` (viewer), `/pages/<n>.png` (raster masters via engines cache), `/data.json` (revisions + bboxes + taint labels, HTML-escaped strings), mutations through existing `POST /` with session arg; **localhost bind default**; token required on every route; traversal + MIME + CSP hardening | HUM-01, API-03 ext, SEC | 11 | 2s | headless e2e: session → GET data → POST accept-by-id → doc updated + journaled; negative suite: bad token 401, traversal 404, wrong MIME refused, GET without session 404 unchanged |
| P4-13 | Viewer UI (static, vendored/hand-rolled ≤1k LOC JS, no build step, pinned hashes in-repo): page image + revision boxes (bbox via existing coords infra), per-box accept/reject buttons, comment pin + draft flow, polling `data.json`, keyboard navigation; **document text rendered as inert escaped text only** | HUM-01, D22 | 12 | 2s | JS unit tests (node) or DOM-less logic tests; XSS corpus fixtures render inert (asserted server-side escaping + CSP header); golden `data.json` schema test |
| P4-14 | Comment write-back: `add_comment` tool + `edit --kind comment` — docx `comments.xml` + `commentRangeStart/End` anchored to C2 ids; odt `office:annotation`; surgical, journaled; wired from session POST (`add_comment_draft` → commit) | HUM-01, OOX/ODF, API | 12 (∥ 13) | 2s | comment read-back by our reader (both formats); LO render golden shows comment; dry_run zero writes; C7 v1.3 matrix updated |

### Wave H — handoff + COM design + add-in spike

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P4-15 | `stichos open` (HUM-02): resolve platform handler (xdg-open / open / cmd /start), open the **current store version** (or `--version <journal-id>` materialized copy); `--app libreoffice|default`; probe + `doctor` handoff row; never blocks on the launched app | HUM-02, ENG-04 | 0 (wave 0) | 1 | host golden (LO present ⇒ launches; assert spawn + args via fake handler binary); honest-absent golden (no handler ⇒ refusal + hints) |
| P4-16 | COM design ADR-0008 (logon-agent process model per concept §9a/D10, ProgID probe design, `windows`-crate feature-gated `engines::ms_office` **compile-only skeleton**: trait impls returning `unimplemented` + probe types) + CI job compiling `x86_64-pc-windows-msvc` with the feature on | COM-1 (design), ENG-09 skeleton | — | 2s | ADR-0008 **accepted** by owner; `cargo check --target x86_64-pc-windows-msvc --features ms-office` green in CI; zero behavior change on Linux (matrix tests unchanged) |
| P4-17 | Add-in spike report (HUM-03, decision-gated): Office add-in manifest + taskpane architecture calling the review HTTP API; cost/risk table; explicit recommend; no implementation | HUM-03 | 12 | 1 | report `docs/addin-spike.md` published; decision row added to owner board |

### Wave X — security, ops, ship

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P4-18 | Fuzz round 3 + SEC-06: hostile revision trees (nested, orphan/dup ids, author/date bombs, huge counts), session HTTP (traversal, token confusion, header injection), viewer XSS corpus via document text; `adapters-ooxfuzz` additions locked as regression tests; threat-model.md + SEC-06 Phase-4 checklist | SEC-01, SEC-06 | 10, 12, 13 | 2s | new fuzz corpora in `adapters-ooxfuzz`; regressions green; ≥24 h clean nightly log linked (CI); review checklist green |
| P4-19 | OPS-03 support-matrix doc (engines N/N-1, distro grid, deprecation notices, D15) + docs polish: README/CODEBASE updated for the Phase-4 surface | OPS-03, ship | 14, 15 | 1 | `docs/support-matrix.md` exists; CODEBASE/README reflect 19 tools / 18 verbs |
| P4-20 | `tools/phase4_gate.sh`: asserts concept §7 items 1–8 **headless** (lossless suites, ODF revisions, session security suite, comment round-trip, fuzz status, C7 v1.3 additive scan, COM skeleton CI status marker, hygiene steps); planted-violation proof; CI job `phase4-gate.yml` | ship | 18, 19 | 1 | gate exit 0 headless + CI green; planted violation (silenced lossless test) → exit 1 |
| P4-21 | M4 acceptance report `docs/m4-acceptance.md`: run all gate checks, record measured vs assumed (incl. what stayed CI-only), tag `m4`, Phase-5 intake (COM impl when host exists, pdf.js wave, PDF review loop, PAdES, formula eval, public-launch decision) | ship | 20 | 1 | report exists; `m4` tag == gate-green commit; pushed with owner approval |

## 4. M4 exit-gate ↔ steps map

| Gate item (concept §7) | Steps | Asserted by |
|---|---|---|
| lossless accept/reject (OOXML) | P4-03..08 | gate runs the lossless suite; goldens present |
| ODF revisions | P4-09..10 | gate runs odt write/read-back + lossless subset |
| review session e2e + security | P4-11..13 | gate runs headless e2e + security suite |
| comment write-back | P4-14 | read-back + golden in gate |
| fuzz round 3 + SEC-06 | P4-18 | nightly status + review doc |
| handoff + COM design | P4-15..17 | host golden + ADR status + CI marker |
| C7 v1.3 frozen additive | P4-08, P4-14, P4-20 | schema matrix + changelog scan |
| hygiene + headless gate | P4-01..02, P4-20 | gate itself runs headless; DAG check green |

## 5. Sizing & context notes (the 200k discipline)

- **2s steps** (P4-04, 05, 07, 09, 10, 12, 13, 14, 16, 18): split = build-core first, wire/test
  second; handoff names files landed + the exact next assertion. Quote the spec clause (ECMA-376
  §17.13.5.15; ODF `text:inserted`, `office:change-info`) into the module doc in part 1.
- Corpus never loaded into context: fixture matrices run **in code**; steps cite counts.
- Collision rules: `gp-core/executor.rs` touched by P4-05 then P4-06 only, sequentially;
  `docs/contracts/C7.md` by P4-08 then P4-14; `mcp/http.rs` by P4-12 then P4-13's server-side bits.
- Wave R ∥ Wave O ∥ (P4-15, P4-16) run in parallel after Wave 0 (different crates). Wave S needs
  Wave R complete (C11 + accept/reject). Wave X last.
- Honest absence stays a first-class path (raster engines, platform handlers); a step is not
  verified by skipping where CI can run the real thing.
- New external crates: none planned (viewer is vendored JS/hand-rolled; if a JS dep becomes
  unavoidable, ADR required + pinned hash — concept §8).
