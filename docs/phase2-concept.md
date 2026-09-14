# stichos — Phase 2 concept: surgical edit + closed loop (M2)

**Status:** planning artifact · derived from [`project_concept.md`](../project_concept.md) §14 Phase 2 +
[`project_plan.md`](../project_plan.md) §2 M2 + [`phase1-concept.md`](phase1-concept.md) §6 (the deferred
columns are this phase's intake) · introduces **no new D-rows**; the three owner scope calls (§10) are
plan-level and flagged for ratification in `project_plan.md`.
**Companion:** [`phase2-plan.md`](phase2-plan.md) (steps) · [`phase2-status.json`](phase2-status.json) (progress).
**Last updated:** 2026-09-13

---

## 1. What Phase 2 is

Turn the read-only binary into the **closed-loop editor**: an agent sends an IR-level patch, stichos
applies it **surgically at the OPC/XML layer** (P1/D5 — untouched parts byte-identical), renders the
result through a real engine, lints it, and returns *images + IR diff + byte-diff report + fidelity
budget* in one round trip (P2/P3/D6). Every mutation is journaled; one primitive reverts it (P6).
On `.docx` the agent's edits **land as tracked revisions** (D7, insert-scoped — §5).

Concept §14: *"OPC-level patching with an ID map, byte-diff report, LO render + layout lint on every
mutation, dry-run diffs."*

### Starting state — measured 2026-09-13 (not assumed)

`phase1-status.json` claims 61/61 + 6/6 gates, but re-running the gate commands on the current tree:
944 tests pass / **4 fail** (2 known header/footer + 2 opc limit tests, unacknowledged);
`clippy -D warnings` and `fmt --check` **fail** (KI-01 fallout in `adapters-pdf/parser.rs`);
cross-check is **5/6** (`rust-toolchain.toml` never lists `aarch64-pc-windows-msvc`);
`tools/phase1_gate.sh` reports **2/6** (broken checks: INV-2 filter matches 0 tests, `apply` grep
false-positives, hardcoded echoes); the entire Phase 1 tree is **uncommitted** (HEAD = Phase 0).
Functionally the read path works (`doctor` = real conversions on LO 26.2.6.1, `inspect` on fixtures;
`deny` and `check-deps` green). **Phase 2 therefore opens with a closeout wave (Wave 0): nothing is
built on evidence that does not reproduce.**

## 2. The ship

- `apply` verb enters the CLI + MCP edit tools (**C7 v1.1, additive**): `apply_patch` + sugar edits,
  `dry_run` on all, `revert` + journal.
- **OOXML-first** (owner call §10): docx/xlsx/pptx editable; **ODF + PDF edits detect-and-refuse** with
  an explicit C3 code — honest, never silent.
- The **closed loop**: mutation → LO render → layout lint → fidelity assembly → payload
  (changed pages + marked/clean pair), P95 ≤ 2 s on the reference machine (OPS-02).
- Windows COM worker, ODF/PDF writers, formula evaluation: **out** (§6).

## 3. Product surface (edit tool set, C7 v1.1 additive)

| Tool | Returns | Notes |
|---|---|---|
| `apply_patch` | IR unified diff + byte-diff report (per part) + fidelity budget + rendered images (changed pages) + `rendered_by` + journal id | `dry_run:true` = diff + preview render only, zero writes (P3) |
| `edit_text` / `edit_cell` / `edit_structure` | same envelope | sugar over the same executor; address by **element ID** (P4); idempotent |
| `revert_document` | byte-identical original restored + journal entry | INV-3 |
| `journal` (list) | version chain with pre/post hashes | C6 |
| existing read tools | unchanged | C7 additive-only policy |

Every mutation response carries: provenance (INV-2), fidelity budget (**100 % of mutations, D6**),
journal id, and the §10 payload rules (changed pages ≤20 blocks, marked+clean pair).

## 4. Edit capability matrix (M2)

| Format | M2 edit surface | Explicit non-surface |
|---|---|---|
| .docx | paragraph/run text replace + insert/delete paragraph; run properties (bold/italic/size); table cell text; **all edits as `w:ins`/`w:del` + `paraId` (D7-insert)** | accept/reject UI (P4 phase), tracked-change *resolution*, fields re-evaluation |
| .xlsx | cell value / inline string set; number-format set; row append | **no formula evaluation** — edits that stale cached results are flagged `stale_cached_values[]` in the budget, never silently rewritten; charts/pivots → refuse-part |
| .pptx | shape/placeholder text replace | master/layout surgery |
| .odt/.ods/.odp | **refuse** (`write unsupported in M2`, C3 code) | ODF-04 → Phase 2b |
| PDF | **refuse** (read stays full) | PDF-03 incremental writer → Phase 2b |
| all | render + lint + payload after every applied mutation | creation paths (Phase 3) |

**Surgical promises (INV-1, the phase's spine):** zero-patch round-trip through the new OPC writer is
**byte-identical** on all 151 corpus fixtures (entry order, compression method, extra fields preserved);
an unrelated edit changes only the touched part(s) + rels/content-types when a part is added/removed;
every untouched part hash-verified post-commit; `revert` returns a byte-identical original (INV-3).

## 5. Behavior promises (new + inherited, each test-bound)

- **INV-1 surgical** → INV-1 suite (Wave B/C gates): round-trip + unrelated-edit byte tests, corpus-wide.
- **INV-3 never in place** → originals immutable; journal per mutation; revert = byte-identical (§4).
- **P3 dry_run everywhere** → preview render of the proposed state in a temp file before any write.
- **P4 ID continuity** → element IDs survive an edit chain (unchanged nodes keep IDs); **C2 freezes at M2**.
- **D7 tracked revisions (insert-scoped)** → agent edits on `.docx` land as `w:ins`/`w:del` with
  author/timestamp/`paraId`; proof = the Phase-1 reader reads them back **and** LO renders revisions.
- **Fidelity budget on 100 % of mutations (M2 gate)** → census before/after per part; "what did this
  edit cost" is the product answer, not an afterthought.
- **2 s loop (OPS-02)** → edit→render→look P95 ≤ 2 s CI perf gate; changed-page payload (REN-03-M2).
- Inherited unchanged: INV-2 provenance, INV-4 determinism, INV-5 hostile input — **the write path is
  new attack surface and gets its own fuzz targets** (malformed patch JSON + hostile package + patch).

## 6. Explicitly out of scope (where it lands)

| Deferred | Lands |
|---|---|
| Windows COM worker ENG-09/COM-1..6, G3 Word tier | Phase 2b (needs Windows CI; unverifiable on this host) |
| ODF surgical patch ODF-04/05 | Phase 2b |
| PDF incremental writer PDF-03 | Phase 2b |
| Formula evaluation engine (xlsx) | open question — never in M2; `stale_cached_values[]` warning instead |
| Tracked-change accept/reject + human UI HUM-01, anchored comments | Phase 4 (D7's human half) |
| Linter-box annotator scope beyond numbered boxes, veraPDF LIN-05 | Phase 3 |
| Creation AUT-*, Rhai hatch API-09, Chromium/Typst ENG-10/11 | Phase 3 |

## 7. M2 exit gate (measurable; asserted by `tools/phase2_gate.sh`, step P2-35)

1. **INV-1 surgical suite green** corpus-wide (zero-patch byte-identical ×151; unrelated-edit tests; revert byte-identical).
2. **Edit→render→look P95 ≤ 2 s** on the reference machine (OPS-02 CI job, recorded).
3. **Fidelity budget on 100 % of mutations** (response-schema matrix on every edit tool).
4. **Tracked revisions default on .docx**: applied edits read back as `w:ins`/`w:del`; LO renders them.
5. **ODF/PDF write refusals asserted** (no silent partial writes anywhere).
6. Write-path fuzz clean (patcher + hostile-package × patch, nightly ≥24 h before ship) + M2 security review (SEC-06).
7. Wave 0 done: `phase1_gate.sh` honest + green on host + CI; Phase 1 tree committed; toolchain ×6 real.

## 8. Phase-2-specific risks

| Risk | Mitigation |
|---|---|
| **Span drift**: byte spans invalid after multi-op patch sets | ops applied right-to-left within a part; span re-verification before every write; reject-and-report on mismatch |
| Zip metadata fidelity breaks "byte-identical" (order/extra fields/timestamps) | writer stream-copies raw local+data records for untouched entries (P2-12 gate catches each class; synthetic fixture per class) |
| Element-ID churn under edits breaks P4 promise | C2 freeze gated on edit-stability property tests (P2-09); ADR-0002 anchor-healing fallback pre-agreed |
| Generated revisions not accepted by Word/LO | proof via own reader **and** LO round-trip render per revision fixture (P2-18); corpus tracked-change fixtures reused |
| 2 s loop dies when LO cold-starts per mutation | instance pool (P1-26) + master cache (P1-29) + render-once-per-mutation budget; OPS-02 measures, does not hope |
| xlsx stale caches silently lie | never evaluate; flag `stale_cached_values[]`; refuse cell ranges feeding charts |
| Write-path hostile input (new) | patch JSON schema-validated; hostile corpus × patcher fuzz before ship (P2-34) |
| Wave-0 temptation to paper over red tests | closeout fixes semantics, not expectations; gate rewrite must catch a *planted* violation (P2-04 verify) |

## 9. Decisions/contracts this phase stands on

**Unchanged:** D2–D10, D11–D15 (loop + engines), D18–D21. **Contracts:** C1 (IR), C3 (frozen v1),
C4 (write extension: per-mutation budget + `stale_cached_values[]`), C5, C8 — consumed;
**C2 + C6 freeze in this phase; C10 drafted + frozen; C7 additive v1.1.**
No new D-rows. The three scope calls below are plan-level; ratifying them edits `project_plan.md`,
not the decision log.

## 10. Owner scope calls 2026-09-13 + deviations from `project_plan.md` §6.3 (flagged)

Owner calls (this session, recorded):
- **S1** — M1 closeout = **Wave 0 of Phase 2** (not a separate remediation project).
- **S2** — **OOXML-first M2**: ODF-04/05, PDF-03, ENG-09 (COM-1..6) move to Phase 2b; M2 refusals are
  explicit. W3's "Windows Office tier live" gate item slips with them.
- **S3** — **D7 insert-scoped for M2**: revision-insert write path only; accept/reject stays Phase 4
  (already planned there).

Sequencing notes: 1) writers/journal/policy (CORE-06/07/09, OOX-02/09) enter Wave A/B exactly where
`phase1-concept.md` §6 promised. 2) REN-06 (annotator) joins Phase 2 (the loop needs marked pages at
gates, §10-4). 3) LIN-04 schema gate wires post-edit; LIN-05 veraPDF does not (Phase 3 a11y work).
