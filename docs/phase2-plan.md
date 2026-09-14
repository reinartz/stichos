# stichos — Phase 2 plan: agent-sized steps (M2)

**Status:** active plan · derived from [`phase2-concept.md`](phase2-concept.md) (scope, gate) and
`project_plan.md` §5 tracks (CORE, OOX, PAT, API, SEC, REN, LIN, OPS) · **35 steps, 7 waves (0, A–F), 7 gates**.
**Progress (update at session close):** `0/35 verified · 0/7 gates · wave 0 pending`.
**Canonical progress state:** [`phase2-status.json`](phase2-status.json) — this file defines the steps;
the JSON tracks them. Do not maintain status in two places.
**Last updated:** 2026-09-13

---

## 1. How this plan is executed (agent rules)

- **One agent, one step, one session.** Budget ~200k tokens. Load only: your step row + wave gate here,
  the named contracts in `docs/contracts/`, the crate(s) you touch, the named corpus subset. No repo-wide
  archaeology: `CODEBASE.md` maps the tree; fetch the specific ECMA-376 clause you need, not the spec.
- **Sizing rule:** every row below is scoped to one 200k session (≈ ≤1 200 new LOC + tests). Steps
  marked **2s** are two sequential sessions with a written handoff in the step's status entry
  (`part1 done: …; part2: …`). If a step grows past its scope anyway, stop, record
  `blocked: split-needed` + evidence; do not sprawl.
- Write only inside the step's deliverable paths + its row in `phase2-status.json`.
- `project_concept.md` / `project_plan.md` / this file are **not edited by executing agents**; a
  contradiction stops the step and flags the owner. Measured vs assumed, always.
- Definition of done: `project_plan.md` §8 + the step's **Verify** column, run and recorded with
  command + result + date.
- Status: `pending → active → verified | blocked(reason)`.

## 2. Waves and gates

| Wave | Theme | Gate (command + pass condition) |
|---|---|---|
| **0** | M1 closeout | rewritten `phase1_gate.sh` green on host + CI **and** catches a planted violation; `cargo test/clippy -D warnings/fmt` green; cross ×6 real; Phase 1 tree committed (tag `m1-closeout`) |
| **A** | kernel: patch/journal/IDs/policy | `gp-core::patch` + journal suites green; ID edit-stability property tests pass; **C2 frozen**, C6/C10 drafted |
| **B** | OOXML surgical writer | **zero-patch round-trip byte-identical on all 151 fixtures**; span-mutation untouched-sibling bytes test; revision fixtures read back via P1-12 reader |
| **C** | executor + closed loop | **INV-1 suite green corpus-wide**; dry-run writes nothing; loop latency recorded ≤ 2 s target (perf gate formal in F) |
| **D** | policy + taint | allow-list denies every unplanted destructive op in the gate matrix; taint flags present on tool output from tainted fixtures; **C10 frozen** |
| **E** | surface | e2e: apply→render shows change; dry_run writes nothing; revert → byte-identical; docx edits read back as `w:ins`/`w:del`; ODF/PDF refuse asserted |
| **F** | gates & ship | `tools/phase2_gate.sh` green host + CI (== M2 exit, concept §7); write-path fuzz 24 h; OPS-02 P95 ≤ 2 s recorded |

Waves ordered; inside a wave run in Deps order; parallel only where deps allow and files don't collide.

## 3. Steps

`Implements` cites `project_plan.md` §5; `Verify` = evidence the step must produce. **Size:** 1 = one
200k session, 2s = two with handoff.

### Wave 0 — M1 closeout (fix the ground before building; S1)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-01 | Fix the 4 red tests **semantics-first**: `opc::test_entry_size_exceeded` / `test_expansion_ratio_exceeded` (compressible-zeros fixture trips the ratio check before the entry-size check — fix fixture entropy and/or check ordering so per-entry violation reports OOX-E-001); `docx::test_parse_header_reference` / `test_parse_footer_reference` (real reader bug in hdr/ftr resolution vs fixture limitation — fix reader, else fix fixture + manifest + `KNOWN_ISSUES.md` entry) | M1 fix | — | 1 | `cargo test --workspace` green (0 failed); no test expectations weakened without a KNOWN_ISSUES entry |
| P2-02 | fmt + clippy cleanup (KI-01 fallout): `adapters-pdf/parser.rs` (unused `idx`, dead `parse_object_at`, let-return), `engines/libreoffice.rs` format-borrow, remainder under `--all-targets` | M1 fix | — | 1 | `cargo fmt --all --check` + `cargo clippy --workspace --all-targets -- -D warnings` clean |
| P2-03 | Toolchain + no-C triage: add `aarch64-pc-windows-msvc` to `rust-toolchain.toml` targets; fix `CODEBASE.md` MSRV note (1.98 pin); triage `cross_check.sh` libc/linux-raw-sys warnings → whitelist rule with rationale (host-std bindings, not linked by us) | DIS-01 fix | — | 1 | `bash tools/cross_check.sh` 6/6 OK, exit 0; no untriaged C warnings |
| P2-04 | Honest gate rewrite `tools/phase1_gate.sh`: reads `phase1-status.json` (no hardcoded echoes); INV-2 check targets real test names (`test_inv2_compliance_*`) + schema matrix; edit-absent via `stichos --help` verb list + MCP `tools/list` probe (no fragile grep); pipefail-safe pipelines; includes build/test/clippy/fmt/deny/check-deps/cross | M1 fix | 01,02,03 | 1 | gate green on host; **planted violations caught** (add `apply` verb → gate fails; break INV-2 test → gate fails); no `echo "verified"` literals left |
| P2-05 | M1 acceptance addendum + commit: rerun every gate check, rewrite gate-F evidence in `phase1-status.json` with 2026-09-13 measurements + this closeout trail; commit the whole Phase 1 tree (`crates/`, `Cargo.*`, `docs/`, `tools/`, `packaging/`, `CODEBASE.md`); tag `m1-closeout` | M1 ship | 04 | 1 | clean `git status`; tag exists; CI (pr.yml cargo job) green on the pushed tree |

### Wave A — kernel: patch model, journal, ID stability, policy types

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-06 | `gp-core::patch` (CORE-07a): op enum — `Insert{anchor,kind}`, `ReplaceText{elem,old,new}`, `Delete{elem}`, `SetRunProps{elem,props}`, `SetCellValue{cell,value}`, `TrackInsert`/`TrackDelete` (revision variants); apply-to-IR engine; op serialization (JSON) | CORE-07 | 00 (C1) | 1 | unit + proptests: apply on IR fixtures; JSON round-trip; unknown-op → C3 code |
| P2-07 | IR unified diff (CORE-07b): patch ↔ unified-text + machine-JSON diff, anchored to element IDs (feeds dry_run + `apply_patch` response) | CORE-07, LIN-08 ext | 06 | 1 | golden diff pairs (text + JSON); empty patch ⇒ empty diff; ID anchors present every hunk |
| P2-08 | Undo journal (CORE-06, **C6**): version chain in the store, pre/post sha256 per mutation, session id, single `revert(n)` primitive; `docs/contracts/C6.md` | CORE-06 (C6) | 07 (P1-07) | 1 | journal tests: chain integrity; revert restores byte-identical bytes from store; corrupt-journal diagnostics (C3) |
| P2-09 | **C2 freeze prep**: ID edit-stability — apply patch sets to IR; unchanged nodes keep IDs (proptest); touched nodes keep ID with content-hash roll per ADR-0002 policy; anchor-healing fallback implemented; freeze `docs/contracts/C2.md` v1 | CORE-04 (C2) | 06, ADR-0002 | 2s | proptest: random patch sets never renumber untouched elements; corpus replay of 20 fixture edit chains; C2 marked frozen |
| P2-10 | Policy + taint types (CORE-09, **C10 draft**): `EditPolicy` (part-class allow-list, destructive-op default-deny), `TaintMark`, taint propagation field on IR response envelope; `docs/contracts/C10.md` draft | CORE-09 (C10) | 06 | 1 | type + serde tests; deny-by-default semantics proven in unit tests (empty policy ⇒ nothing allowed) |

### Wave B — OOXML surgical write path (the spine)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-11 | OPC surgical writer (OOX-02a): stream-copy untouched entries raw (order, compression method, extra fields, local headers); zero-patch mode = byte-identical output | OOX-02 | 00 | 2s | **zero-patch round-trip byte-identical on all 151 fixtures** (sha256 == input); per-class zip-metadata synthetic fixtures (order/extra-field/stored-vs-deflated) each proven byte-identical |
| P2-12 | Writer mutations (OOX-02b): replace-part bytes + add part + rels/[Content_Types] update; unknown-part pass-through recorded in budget | OOX-02 | 11 | 1 | patched-part tests: only touched parts + rels differ; synthetic new-part add keeps all other bytes; unknown parts inventoried |
| P2-13 | XML span-mutation primitives (OOX-09a) on the P1-09 DOM-lite: replace-text, insert-node, delete-node, set-attribute — all right-to-left within a part; every mutation re-verifies spans against source bytes before applying | OOX-09 | 09 (P1-09), 06 | 2s | property test over **all corpus XML parts**: mutation changes only the span's bytes; stale-span input rejects with C3 code (never writes) |
| P2-14 | IR patch → docx routing (OOX-09b): `ReplaceText`/`TrackInsert`/`TrackDelete`/`Delete`/`SetRunProps` on paragraphs/runs/table cells → span edits in `document.xml` (+ header/footer parts) | OOX-09 | 11,12,13 | 2s | corpus docx edit chains: apply → re-read via P1 reader shows exactly the intended change; census delta == expected (C9-style manifest) |
| P2-15 | IR patch → xlsx/pptx routing (OOX-09c): `SetCellValue` (+ `stale_cached_values[]` budget entry when the cell feeds a formula — detection by formula-text scan, **no evaluation**), number-format set, row append; pptx `ReplaceText` on shape text | OOX-09 | 14 | 1 | corpus xlsx/pptx edit chains re-read + census delta; stale-cache flag asserted on formula-dependent fixture; chart-referenced range → refuse-part |
| P2-16 | Commit path + byte-diff report (OOX-09d): patch-set → writer; post-commit verify all untouched part hashes; per-part byte-diff report (JSON + human) in result envelope | OOX-09, PAT-01(part) | 14,15 | 1 | INV-1 unrelated-edit suite: edit one paragraph ⇒ all other parts hash-identical, corpus sample 30+; report schema test |
| P2-17 | Revision-insert write (OOX-10, scoped S3/D7): wrap applied docx edits in `w:ins`/`w:del` with author/timestamp/`paraId`; `--direct` opt-out for tests | OOX-10 | 14 | 2s | own reader (P1-12 path) reads back revisions on 10 fixtures incl. existing tracked-change corpus set; **LO round-trip**: converted PDF shows revision rendering (golden pair) |

### Wave C — patch executor & closed loop

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-18 | Patch executor (PAT-01): `PatchExecutor` impl routing IR patch → docx/xlsx/pptx paths; ODF/PDF → `write unsupported` C3 code, zero side effects | PAT-01 | 16, 08 | 1 | executor suite: format routing table; refusal asserted for every ODF/PDF fixture (nothing written); INV-1 re-run through executor |
| P2-19 | Journaled edits + ID continuity (PAT-04): every applied patch writes a journal entry; `revert(n)` through executor; element IDs stable across a 5-edit chain (P4 realized) | PAT-04 | 18, 08, 09 | 1 | e2e: edit chain → IDs match pre-chain for untouched elements → revert(0) byte-identical original |
| P2-20 | Dry-run flow (PAT-02, P3): `dry_run:true` ⇒ IR unified diff + preview render of proposed state in temp file; **zero writes to store or input** | PAT-02 | 18, 07, (P1-26) | 1 | dry-run leaves store/input byte-identical (fs-level assert); preview image present; diff == later apply diff |
| P2-21 | Post-edit QA loop (PAT-03 + REN-03-M2): render + lint + fidelity assembly on every mutation; changed-page detection → payload images; loop latency recorded per fixture class | PAT-03, REN-03 | 18, (LIN P1-33..35, P1-29) | 2s | loop e2e on `pr-subset`: every response carries budget + lint manifest + changed-page list; latency table recorded (target ≤2 s, gate formal in P2-33) |
| P2-22 | Signature invalidation (PAT-05): detect existing OPC XML-DSig/signature parts → warn diagnostic + budget entry; sign-later stub documented | PAT-05 | 18 | 1 | signed-fixture (synthetic) edit → warning + `signatures_invalidated[]`; no fake "signed" state anywhere |

### Wave D — policy & taint (SEC)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-23 | Taint marking (SEC-03): document text taints through IR into tool responses; response envelope carries `tainted: true` + source part; instruction-channel quarantine documented in `docs/threat-model.md` §agent-facing | SEC-03 (C10) | 10 | 1 | taint propagation tests (docx/pdf fixture text → response flag); no taint on clean fixtures |
| P2-24 | Edit allow-list enforced in executor (SEC-04): `EditPolicy` consulted pre-apply; deny-by-default destructive ops (delete-part, macro-part writes, external-rel adds → hard deny); per-class gate matrix | SEC-04 | 18, 10 | 1 | gate matrix test: every destructive op class denied under default policy; allowed subset works under explicit policy |
| P2-25 | C10 freeze + contract test suite (producer=gp-core, consumers=executor/tools) | C10 | 23, 24 | 1 | contract suite green both sides; C10 marked frozen in `docs/contracts/` |

### Wave E — surface (edit tools on MCP + CLI)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-26 | `apply_patch` tool + CLI `apply` verb (API-07a): JSON patch input (schema-validated), `dry_run` flag, full response envelope (diff + byte-report + budget + provenance + images + journal id) | API-07 | 20, 21, (P1-44..47) | 2s | e2e CLI + MCP on `pr-subset`: apply→render shows change; INV-2 matrix extended to edit tools; bad patch JSON → C3 codes, no write |
| P2-27 | Sugar edits (API-07b): `edit_text`, `edit_cell`, `edit_structure` (insert/delete paragraph); idempotency per C7 (same args twice = same doc, one journal entry) | API-07 | 26 | 1 | idempotency tests per tool; element-ID addressing e2e |
| P2-28 | Tracked-changes default (API-07c): docx edits land as revisions unless policy opts out (D7/S3); `--direct` flag; MCP tool schema documents it | API-07, D7 | 26, 17 | 1 | default path produces read-back revisions (reuse P2-17 proof); direct flag proven by census |
| P2-29 | Annotator (REN-06): numbered lint boxes drawn on page; marked + clean image pair in payload (§10-4) | REN-06 | 21, (P1-33) | 1 | box placement correctness on synthetic lint fixtures; token cost ≤ budget; pair always shipped |
| P2-30 | `revert` + `journal` tools (CLI + MCP); C7 v1.1 additive freeze + response-schema matrix over the **full** tool set | API-07, C7 | 19, 26 | 1 | revert e2e byte-identical; journal list matches chain; schema matrix green on every tool; C7 changelog entry (additive-only check) |

### Wave F — gates & ship (M2 exit)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P2-31 | Schema gate wired post-edit (LIN-04): XSD validate mutated parts + `qpdf --check` (PDF outputs, if any); broken mutation ⇒ block + report (never ship a corrupt write) | LIN-04 | 21, (P1-16, P1-24) | 1 | mutation fixtures that break schema → blocked with codes; clean mutations pass |
| P2-32 | Write-path fuzz: patch-JSON fuzzer + hostile-package × patcher combination targets; nightly CI job; SEC-06 M2 security review + threat-model update | SEC-01(write), SEC-06 | 26, 24 | 2s | fuzz corpus regressions locked as tests; review checklist green; ≥24 h clean run before ship tag |
| P2-33 | OPS-02 perf gate: edit→render→look P95 ≤ 2 s CI job on reference profile + cache hit-rate tracking | OPS-02 | 21 | 1 | perf job green in CI; latency recorded in status evidence; regression > 10 % fails the job |
| P2-34 | `tools/phase2_gate.sh`: asserts concept §7 (INV-1 corpus suite, budget-on-100 %, revisions read-back, ODF/PDF refusals, loop P95, fuzz status, C2/C6/C10 frozen markers); reads status JSON only | ship | all | 1 | gate green host + CI; planted violations caught (silenced budget, weakened INV-1 test) |
| P2-35 | M2 acceptance runbook + ship evidence: run all gate checks, publish report, tag `m2`; note Phase-2b intake (COM, ODF-04, PDF-03) | ship | 34 | 1 | report in `docs/`; tag `m2`; Phase-2b backlog written |

## 4. M2 exit-gate ↔ steps map

| Gate item (concept §7) | Steps | Asserted by |
|---|---|---|
| INV-1 surgical suite green | P2-11, P2-16, P2-18, P2-19 | `phase2_gate.sh` runs INV-1 suites (zero-patch ×151 + unrelated-edit + revert) |
| loop ≤ 2 s P95 | P2-21, P2-33 | OPS-02 CI job; gate reads its status |
| fidelity budget on 100 % of mutations | P2-21, P2-26..28 | response-schema matrix (every edit tool) |
| revisions default on docx | P2-17, P2-28 | read-back + LO render fixtures |
| ODF/PDF refusals asserted | P2-18, P2-30 | refusal matrix in gate |
| write-path fuzz + security review | P2-32 | nightly status + review doc |
| Wave 0 (honest M1) | P2-01..05 | rewritten `phase1_gate.sh` green + `m1-closeout` tag |

## 5. Sizing & context notes (the 200k discipline)

- **2s steps** (P2-09, 11, 13, 14, 17, 21, 26, 32): the split is parse/derive-part first, wire/test-part
  second; the handoff note must name files landed and the exact next assertion. Largest context risk is
  ECMA-376 spec fetch — quote the clause into the step's module doc comment during part 1 so part 2
  doesn't re-fetch.
- Corpus loading is never whole-repo: steps name their subset (`pr-subset`, one format family, or
  "all 151" only where the gate demands it — P2-11's round-trip runs in code, not in context).
- Contract docs (C2/C6/C10) are written by the step that freezes them; consumers read the frozen file,
  not chat history.
- Wave 0 is the only wave allowed to touch Phase-1 files outside the edit paths; everything after is
  additive-or-scoped per the row's deliverable column.
