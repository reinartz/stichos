# stichos — Phase 4 concept: the human loop (M4)

**Status:** planning artifact · derived from [`project_concept.md`](../project_concept.md) §14 Phase 4
+ [`project_plan.md`](../project_plan.md) §2 M4 + [`m3-acceptance.md`](m3-acceptance.md) §4 (Phase-4
intake) + [`phase2-concept.md`](phase2-concept.md) §6 (2b backlog: "tracked-change accept/reject UI") ·
introduces **one new decision row (D22, review-overlay medium)**; five owner scope calls (§10,
2026-09-16) are plan-level and written back where needed.
**Companion:** [`phase4-plan.md`](phase4-plan.md) (steps) · [`phase4-status.json`](phase4-status.json) (progress).
**Last updated:** 2026-09-16

---

## 1. What Phase 4 is

The agent half of the loop has been shipped since M2: edits land as tracked revisions (D7). What is
missing is the **human half** — a person must be able to *see* the proposals, decide, and have the
decision applied losslessly. M4 ships exactly that, plus the pieces the review loop needs:

1. **Revisions kernel (new).** A revision model (C11) in `gp-core`, enumeration from OOXML
   (`w:ins`/`w:del`/format changes) and ODF (`text:inserted`/`text:deleted`), and **accept/reject**
   executors that are surgical (INV-1), journaled (INV-3), policy-gated (C10), dry-run capable (P3).
   Today the kernel only *writes* revisions — there is no revision model and no accept/reject path.
2. **ODF revisions (owner call S1).** ODF gets revision **write** (the M3 "open" item) and
   accept/reject, so the M4 lossless gate extends to `.odt`.
3. **Review overlay (HUM-01, medium per D22).** A **raster overlay over the 150-DPI masters**
   served by the existing HTTP transport: page image + revision boxes + per-box accept/reject +
   comment pins. Not pdf.js (D22 — see §8 for what that buys and costs). Session-scoped,
   token-authenticated, localhost by default (ADR-0007).
4. **Comment write-back.** Human comments from the overlay land in the document as first-class
   OOXML comments / ODF `office:annotation` — surgical, journaled, readable by our own reader,
   visible in LibreOffice.
5. **Handoff (HUM-02) + COM design (S3).** `stichos open` resolves the platform handler for the
   current store version. The Windows COM worker gets a **design ADR + compile-only skeleton +
   Windows CI job** — implementation stays deferred until a Windows host exists. The Office add-in
   (HUM-03) stays a decision-gated **spike report**.

`project_plan.md` §2 M4 exit gate: *"tracked-change accept/reject round-trip lossless on corpus
subset."* Everything else in this phase serves that sentence.

### Starting state — measured 2026-09-16 (not assumed)

- M3 tree **gate-green, re-verified independently**: 1 644 tests, 0 failures; clippy/fmt/deny clean;
  `phase3_gate.sh` 26/26; tag `m3` = `1e0749b`, pushed to origin with owner approval.
- Revisions: **write exists** (`adapters-ooxml/revision_writer.rs`, `create_as_revisions`), **read
  is partial** (docx reader sees revisions in IR/census), **accept/reject does not exist** —
  `gp-core` has no revision model (only an `executor.revisions: bool` flag).
- HTTP transport (`mcp/http.rs`): `POST /` JSON-RPC + session store only; GET returns 404 — the
  overlay needs new routes (GET assets/data/pages + token check).
- Host facts (this box): pandoc 3.7.0.2 ✅, chromium-browser ✅, soffice 26.2.6.1 ✅, xmllint ✅,
  pdftoppm ✅; typst/qpdf/verapdf ✗ (CI installs; honest-absent paths shipped in M3 stay).
  **No Windows host.** Agent shells here have **no `/dev/tty`** and no `script` binary — the
  M2/M3 gate scripts' `tee /dev/tty` pattern silently false-passes headless (found during the M3
  audit; `docs/m3-acceptance.md` §7) → fixed by **P4-01**, and all Phase-4 gates must run headless.
- Found during the M3 audit, owned by this phase: two dependency-DAG violations of
  `project_plan.md` §3.2 (`adapters-ooxml → lint`, `engines → adapters-ooxml`) that
  `tools/check-deps.sh` does not currently detect → owner ratification/refactor in **P4-02**.
- Corpus: 151 fixtures (69 ODF); `pr-subset.txt` 27 entries. **No tracked-change fixtures exist
  beyond M2's synthetic revision tests** → P4-07 extends the generator.

## 2. The ship

- **`stichos revisions list|accept|reject`** + MCP `list_revisions` / `accept_revisions` /
  `reject_revisions` (C7 additive **v1.3**): per-id, by-author, `--all`; `dry_run` everywhere;
  every mutation journaled like `apply`.
- **Lossless accept/reject** on OOXML, and on ODF via the new revision write:
  **reject-all of an agent edit restores the original byte-identically** (our-written revisions);
  accept-all equals the direct-edit result (IR-equality assertion). LO render goldens pin both states.
- **`stichos review`** opens a local review session (URL + token printed); the browser overlay
  lists revisions per page, accepts/rejects through the session API, drafts comments.
- **Comments flow back**: overlay comment → real comment part in the docx/odt, anchored by C2 ids.
- **`stichos open`** hands the current version to the platform's office handler (LO/default),
  honest when absent; `doctor` gains a handoff row.
- **COM**: ADR-0008 (worker process model, ProgID probe design, `windows`-crate feature skeleton
  compiled in CI for `x86_64-pc-windows-msvc`, never run) + add-in spike report. No behavior ships.

## 3. Product surface (new, C7 v1.3 additive)

| Tool / verb | Returns | Notes |
|---|---|---|
| `list_revisions` (MCP) / `stichos revisions list` | `{ revisions[], counts, doc_version }` — id, kind, author, date, anchor element id, page/bbox where derivable | read-only, idempotent (INV-2 provenance as always) |
| `accept_revisions` / `reject_revisions` (MCP) / `stichos revisions accept\|reject` | ExecuteResult envelope: byte-diff report + fidelity budget + journal ids + `dry_run` diff | args: `ids[]` \| `all` \| `author=`, `dry_run`, `session`; non-idempotent; policy-gated (C10) |
| `add_comment` (MCP) / `stichos edit --kind comment` | same envelope + comment id | anchors `commentRangeStart/End` (docx) / `office:annotation` (odt) |
| `review` verb | session id + URL + token; optional `--no-browser` | starts HTTP session on the doc version; refuses when doc dirty vs store head |
| review HTTP surface (extension of `serve --http`) | GET `/review/<token>/` viewer, `/pages/<n>.png`, `/data.json`; mutations via existing `POST /` with `session` arg | localhost bind default; token required on every GET+POST of the session; no ambient start |
| `open` verb (HUM-02) | exit 0 + what it launched, or honest refusal + install hint | `--app libreoffice|default`, `--version <journal-id>`; probes platform handler |

The MCP tool registry grows 16 → 19 specs (v1.3.0); CLI verbs 16 → 18 (`revisions`, `review`,
`open` — `edit --kind comment` is a kind, not a verb).

## 4. Capability matrix (M4)

| Path | Input → output | Engine/deps | Explicitly not |
|---|---|---|---|
| OOXML accept/reject | docx with revisions → docx accepted/rejected | own (kernel + span writer) | move-from/move-to revisions (refuse with C3); field-revision edge cases beyond insert/delete/format (refuse) |
| ODF revision write | edits on odt → `text:inserted`/`text:deleted` | own | ODS/ODP revision write (odt only this phase) |
| ODF accept/reject | odt with revisions → accepted/rejected | own | — |
| Review overlay | docx/odt version → browser session | own HTTP + raster ladder (LO/poppler for pages) | PDF review loop; multi-user concurrency |
| Comment write-back | overlay/CLI → comment part in docx/odt | own | threaded replies; PDF annot write-back via UI |
| Handoff `open` | store version → platform app | xdg-open/open/Start | Collabora/ONLYOFFICE server integration (URL handoff documented, not built) |
| COM | — (design + compile skeleton) | CI windows-msvc compile only | any runtime behavior (S3) |

## 5. Behavior promises (each test-bound)

- **M4 lossless gate** (two-tier, per §8): for revisions **written by us** (agent edits,
  create-as-revisions), reject-all ⇒ **byte-identical original**; accept-all ⇒ IR-equal to the
  `--direct` result. For **foreign revisions** (third-party docx), accept/reject ⇒ asserted IR +
  visible-text equality (byte identity not promised). Pinned on a revision fixture set derived from
  the corpus + LO-produced revision files.
- **INV-1 extended**: accept/reject are span-surgical; unrelated parts byte-identical; property
  tests over all revision fixtures.
- **INV-3 / C6**: every accept/reject/comment is journaled; `revert` restores.
- **INV-2 / C4 / C10**: budget + `rendered_by` on every response; destructive bulk ops pass the
  policy gate (deny-by-default for `all` without dry-run seen).
- **Revision enumeration parity**: counts match census (`w:ins`/`w:del`/`text:inserted` counters)
  on every fixture; nested (ins-in-del) handled; unsupported kinds **refuse, never silently drop**.
- **Session security (ADR-0007)**: localhost bind by default; token required on all session
  routes; path-traversal + content-type negative suites green; document text rendered
  HTML-escaped with strict CSP; XSS corpus fuzzed; session invalidated when the doc version hash
  changes (optimistic concurrency).
- **Honest absence**: no raster engine ⇒ session still works data-only (boxes without page image);
  no platform handler ⇒ `open` refuses with hints. CI runs the real-raster path.
- Inherited unchanged: C2 anchoring (accept/reject must not break stable IDs of untouched elements),
  C8 payload rules for the LLM-facing tools, INV-4 determinism (revision ops are byte-deterministic).

## 6. Explicitly out of scope (where it lands)

| Deferred | Lands |
|---|---|
| COM worker **implementation**, G3 Word tier | when a Windows host exists (Phase 5+); Phase 4 = ADR-0008 + compile skeleton only (S3) |
| pdf.js text-layer overlay | later wave, only if D22's revisit trigger fires |
| PDF human-review loop (annotation round-trip in UI) | post-M4 — PDF annot write helpers exist since M3 |
| ODS/ODP revision write | post-M4 (odt first, §4) |
| Word-interop verification of accept/reject | post-M4 (LO render + our reader are the shipped proof; Word pass needs licensed hosts) |
| PAdES signing, content-stream PDF surgery | post-M3 list, unchanged |
| xlsx formula evaluation (OQ2-adjacent) | open question, unchanged |
| Charts/pivots/SmartArt creation | open, unchanged |
| G5 pdfium | stays gated |
| Multi-user / hosted review | not planned; local single-reviewer by design |

## 7. M4 exit gate (measurable; asserted by `tools/phase4_gate.sh`, step P4-20)

1. **Lossless accept/reject (OOXML)** on the revision corpus subset per §5's two-tier assertion,
   LO render goldens green for accept-all and reject-all states.
2. **ODF revisions**: revision write round-trips (our reader + LO), accept/reject lossless on the
   odt subset.
3. **Review session e2e (headless HTTP)**: create → list → accept/reject via token → doc updated +
   journaled; security suite green (traversal, token, CSP, XSS corpus).
4. **Comment write-back**: UI/CLI comment → read-back via our reader + LO render golden (docx+odt).
5. **Fuzz round 3** (revision parser hostile trees, session HTTP, viewer XSS corpus) clean + SEC-06
   Phase-4 review + threat-model update.
6. **Handoff + COM design**: `open` host-verified where handler present + honest-absent golden;
   ADR-0008 **accepted**; windows-msvc compile skeleton green in CI.
7. **C7 v1.3 frozen** (additive-only across the full 19-tool matrix; CHANGELOG breaking-change scan).
8. **Hygiene + gate**: P4-01 (tty-safe gates) + P4-02 (DAG ratified + enforced) verified;
   `phase4_gate.sh` green **headless** + CI; tag `m4`.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Accept/reject semantics diverge from Word (nested, move-revs, rsid noise) | scope to insert/delete/format (Word's 95 %); **refuse** move + unknown kinds with C3; two-tier losslessness; LO render goldens as visual cross-check |
| Byte-identity promised for foreign revisions we can't honor | §5 states the two tiers explicitly; gate asserts byte only for ours; honest budget note otherwise |
| Review server becomes an attack surface | localhost default bind, random token required on every route, no ambient startup, CSP + full HTML-escaping of document text, traversal/MIME negative suites, fuzz round 3 (ADR-0007) |
| Session/doc skew (agent edits while human reviews) | version-hash check invalidates session; client must re-open; no silent overwrite of a changed doc |
| ODF change-ids are document-global, not C2-anchored | C11 mapping strategy (change-id ↔ element id via first affected node) + property tests; refusals where anchoring is ambiguous |
| Vendored viewer JS is a supply-chain hole | single small pinned dependency set (or hand-rolled ≤1k LOC, P4-13 decides), hashes pinned in-repo, `cargo deny` source check analog in gate; no CDN at runtime |
| UI scope creep ("while we're at it…") | §6 is hard scope; anything not serving the M4 gate sentence is deferred by rule |
| Prompt-injection via document text reaching the browser | viewer renders text as inert escaped strings, never as HTML/JS; taint labels preserved in `data.json` (C10) |

## 9. Decisions/contracts this phase stands on

**Unchanged:** D2–D21 (incl. D7 as the premise of this whole phase; D12/D13 feed the overlay).
**New:** **D22** (raster overlay over masters, not pdf.js — recorded in `project_concept.md` §16).
**New ADRs:** ADR-0007 (review-session model + security), ADR-0008 (COM worker design,
implementation gated on Windows host).
**Contracts:** **C11 new** (revision model: kinds, ids, anchoring, accept/reject semantics, C3 codes)
· C1 **minor additive** (`Document.revisions`) · C7 **additive v1.3** (19 tools / 18 verbs) ·
C2/C6/C10 consumed frozen · C9 extended (revision-fixture set + lossless metrics).
`docs/adr/README.md` numbers reserved accordingly.

## 10. Owner scope calls 2026-09-16 (this session)

- **S1** — **ODF revision write + accept/reject is IN Phase 4 scope** (extends the M4 gate to `.odt`).
  `project_plan.md` §6.3 W5 row updated accordingly.
- **S2** — **Review overlay = raster over the 150-DPI masters, not pdf.js.** Concept §14 wording
  amended; recorded as **D22**.
- **S3** — **COM = design + CI skeleton only** in Phase 4 (ADR-0008 + compile-only windows-msvc
  skeleton + CI job); implementation stays deferred until a Windows host exists.
- **S4** — Push approved and executed: `main` (`1e0749b`) + tag `m3` are on origin.
- **S5** — **ADR-0004 flipped proposed → accepted** (Typst shell-out; engine shipped).

Flagged to the owner during the M3 audit, owned by this plan: **P4-02** (ratify-or-refactor the two
DAG violations + add real edge enforcement to `check-deps.sh`); **OQ3** (`gp` alias — de facto "no"
since CLI v1, formal closure recommended); m3-acceptance per-crate test table corrected in §7
addendum (counts now authoritative in `CODEBASE.md`).
