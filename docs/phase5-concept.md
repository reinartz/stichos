# stichos — Phase 5 concept: the PDF-native loop and trust (M5)

**Status:** planning artifact · derived from [`project_concept.md`](../project_concept.md) §14 + §16
(decisions D1–D22) + [`m4-acceptance.md`](m4-acceptance.md) §4 (Phase-5 intake) + owner scope calls
2026-09-18 (§10) · **proposes decision rows D23–D26** (ratified + recorded into `project_concept.md`
§16 by P5-01; the concept file is not edited by executing agents).
**Companion:** [`phase5-plan.md`](phase5-plan.md) (steps) · [`phase5-status.json`](phase5-status.json) (progress).
**Last updated:** 2026-09-18

---

## 1. What Phase 5 is

M4 closed the human loop for OOXML/ODF: revisions accepted/rejected losslessly, a raster review
overlay, comments written back. What the loop **cannot yet do** is turn a `review` session over a
**PDF** — the format the project itself declares the lingua franca of visual proof (§2 of the
concept) — and it cannot answer the trust question: *did this edit break a signature, and can the
proof be signed afterwards?* M5 ships exactly those two halves:

1. **PDF annotation round-trip (new kernel).** A C12 annotation-review model in `gp-core`
   (highlight/text/freeText/stamp kinds, page+rect+quote anchor, author/date, resolve state),
   enumeration from PDF annotation dictionaries, and **surgical incremental write-back** (the M3
   append-only writer, INV-1-safe, journaled) — refuse, never silently drop, unknown subtypes.
2. **Review session over PDF.** Reuse of the shipped ADR-0007 session + raster overlay (D22):
   page images + annotation boxes + resolve/reject + comment pins on a PDF, end-to-end headless.
3. **pdf.js text layer, opt-in (D22 revisit — owner call C4).** A vendored, hash-pinned pdf.js
   **text layer only** for sessions that opt in: text selection anchors a comment by bbox + quote.
   Raster overlay stays the default medium; pdf.js is not the viewer UI; no CDN at runtime.
4. **Trust: signature integrity + sign-later (new C13).** Edits to signed documents are
   **detected and reported, never silent** (the PAT-05 stub becomes real); a **PAdES-B-B sign-later**
   path with pure-Rust crypto (D25 — C stays out, D2 honored); PDF/A revalidation wired into the
   mutation QA loop with honest-absent preserved.
5. **Launch prep (not launch — owner call C2).** GOV hygiene pack + publish **dry-runs** only;
   the D18 private→public flip stays an owner-timed action (D26).

M6 (next milestone, intake §6) holds the rest of the M4 intake: **COM worker implementation** (owner
call C1: a Windows host will exist mid-M5 — COM-2..6 land in M6), **xlsx formula evaluation**
(ADR + minimal core subset, owner call C3), **ODS/ODP revision write**, Word-interop verification,
and the actual public launch.

### Starting state — measured 2026-09-18 (not assumed)

- M4 **gate-green, re-verified independently**: 1 851 tests, 0 failures; clippy/fmt clean;
  `phase4_gate.sh --ci` 26/26 headless; planted-violation caught; tag `m4` = `12490be`
  (re-anchored to the gate-green commit — the initial tag sat on the M3-closeout commit while
  Phase-4 work was uncommitted; see `m4-acceptance.md` Git-State), `main` + `m4` pushed to origin.
- **Surface truth is measured, not documented**: MCP tool registry = **20 tools**
  (`spec.rs:824` asserts `TOOLS.len() == 20`), CLI = **19 verbs** (`enum Command`). The Phase-4
  docs (support-matrix, CODEBASE, m4-acceptance) say 19/18 — off by one; corrected by **P5-01**.
- Host facts (this box): pandoc 3.7.0.2 ✅, soffice 26.2.6.1 ✅, pdftoppm ✅, xmllint ✅,
  **node v22.23.1 + npm 10.9.8 ✅** (viewer logic tests can run in node);
  typst ✗, qpdf ✗, veraPDF ✗ (CI installs; honest-absent paths stay). **No Windows host today**;
  owner expects one mid-M5 (C1) — nothing in M5 depends on it.
- Corpus: 157 manifests (151 M0-era + revision fixtures from P4-07); `pr-subset.txt` 27 entries.
  **No PDF-annotation or signed-PDF fixtures exist** → P5-02 extends the generator.
- **M4 lessons baked into this phase's rules:** (a) a milestone tag moves only to a **committed**
  gate-green tree — commit → gate → tag → push, in that order; (b) every verify command produces
  its verdict from **exit codes**, headless (no `/dev/tty`); (c) counts in docs must be measured
  from code, never carried forward.

## 2. The ship

- **`stichos annots list|resolve`** + MCP `list_annotations` / `resolve_annotations`
  (C7 additive **v1.4**): per-id / by-author / `--all`; `dry_run`; journaled like every mutation.
- **Annotation write-back is incremental**: annot writes append to the PDF (append-only prefix
  invariant from M3 holds), the pre-edit bytes stay a verifiable prefix; read-back by our own
  reader; render goldens (LO/gs) show highlights and comments.
- **`stichos review` accepts PDFs**: same session/token model (ADR-0007), same raster overlay,
  annotation boxes instead of revision boxes; resolve/reject writes through the C12 path.
- **pdf.js text layer, per-session opt-in** (`review --text-layer`): vendored pdf.js release,
  pinned hash verified by the gate, text layer served offline; selection → comment draft anchored
  by page + bbox + verbatim quote (C12 `quote`).
- **`stichos sign` + MCP `sign_document`**: PAdES-B-B sign-later over the incremental writer;
  self-signed or provided PKCS#12 identity; `--digest sha256`; TSA timestamp honest-absent.
  Existing signatures on mutation: **detected, reported in the fidelity budget, refused by
  default** — proceeding requires explicit policy (C10).
- **PDF/A revalidation** wired into mutation QA + a `doctor` row (veraPDF/`qpdf --check` when
  present, honest-absent otherwise).
- **Launch prep, no launch**: LICENSE dual-file audit, headers, CONTRIBUTING/SECURITY/DCO, a
  public-audience docs pass, and `cargo publish --dry-run` / winget-validate / brew-audit
  rehearsals recorded — the flip itself stays owner-timed (D26).

## 3. Product surface (new, C7 v1.4 additive)

| Tool / verb | Returns | Notes |
|---|---|---|
| `list_annotations` (MCP) / `stichos annots list` | `{ annotations[], counts, doc_version }` — id, kind, page, rect, author, date, state, quote | read-only, idempotent; PDF inputs |
| `resolve_annotations` (MCP) / `stichos annots resolve` | ExecuteResult envelope: byte-diff report + budget + journal ids + `dry_run` diff | args: `ids[]` \| `all` \| `author=`, `state=resolved\|rejected\|accepted`, `dry_run`, `session`; non-idempotent; policy-gated |
| `sign_document` (MCP) / `stichos sign` | same envelope + signature info (subject, digest, byte-range span) | PAdES-B-B; refuses to sign a doc with a pending unsigned mutation unless re-signed explicitly; PKCS#12 in, honest-absent TSA |
| `review` verb + session (extended) | PDF docs accepted; `--text-layer` opts into pdf.js | same routes; `data.json` gains `mode: "pdf-annot"` + optional `text_layer` |
| review HTTP surface (extended) | `GET /review/<token>/text/<n>.json` (text-layer spans); `/viewer/pdf/*` (vendored assets, hash-pinned) | token on every route as before; CSP updated for text layer |

Baseline is the **measured** 20 tools / 19 verbs; M5 target: 23 tools / 21 verbs (`annots`, `sign`).
No existing tool is modified (additive-only rule).

## 4. Capability matrix (M5)

| Path | Input → output | Engine/deps | Explicitly not |
|---|---|---|---|
| PDF annotation enumerate | pdf with annots → C12 model | own (lopdf core) | XFA/XDP (refuse, flagged since M1); unknown subtypes → refuse |
| PDF annotation write-back | resolve-state updates + new highlight/text annots → incremental pdf | own (append-only writer) | redaction (post-M5, needs content-stream surgery); form-fill round-trip in UI |
| PDF review session | pdf version → browser session | own HTTP + raster ladder | multi-user; hosted review |
| pdf.js text layer | session opt-in → selectable text over page | vendored pdf.js (pinned) | pdf.js as viewer UI; annotations rendered by pdf.js |
| Signature detection | signed pdf → signature info in IR + budget | own | decrypt/encrypt; incremental signing-chain verification against CRLs/OCSP |
| Sign-later PAdES-B-B | unsigned pdf → signed pdf (self-signed / given identity) | own pure-Rust crypto | trusted-certificate issuance; TSA (honest-absent); PAdES-T/-LT/-LTV |
| PDF/A revalidation | mutated pdf → conformance status | veraPDF/qpdf when probed | auto-fix conformance |
| Launch prep | repo state → publish-ready + recorded dry-runs | cargo/winget/brew tooling | the publish itself (D26) |

## 5. Behavior promises (each test-bound)

- **M5 lossless annotation gate**: enumeration parity vs annot-dictionary counts on every PDF
  fixture; nested/multiparent annots handled; unsupported → **GP-A0xx refusal, never silent drop**.
  Write-back: **append-only prefix invariant** (pre-edit bytes are a prefix of post-edit bytes);
  unrelated objects untouched; journaled (INV-3); `revert` restores byte-identically.
- **Session e2e over PDF (headless)**: create → list annotations → resolve via token POST → pdf
  updated + journaled; version-hash invalidation as in M4.
- **Text-layer promises**: vendored hash verified by the gate; no runtime CDN; document text in
  the text layer is **inert** (escaped/`textContent` only, CSP `script-src 'self'`); selection →
  anchored comment round-trips bbox + quote exactly.
- **Signature integrity (the trust promise)**: every mutation path (`apply`, `edit`, revisions
  accept/reject, `annots resolve`) **detects an existing `/Sig`** → budget entry + refusal by
  default; explicit policy pass records `signature_invalidated` in the journal. **Never silent.**
- **Sign-later correctness**: signed output passes `qpdf --check` and an independent byte-range
  verify (openssl); tamper-evidence test (one byte changed after signing ⇒ verify fails);
  signing a doc that already carries `/AcroForm` sig-field conflicts → refuse with GP-S0xx.
- **Ordering rule**: mutations after signing require the explicit policy pass (same as above);
  sign-later is the documented happy path.
- **Inherited unchanged**: INV-1/2/3/4, C2 anchoring, C8 payload rules, honest absence
  (no raster engine ⇒ data-only session; no veraPDF ⇒ skipped-with-note in doctor, CI runs real).

## 6. Explicitly out of scope (where it lands)

| Deferred | Lands |
|---|---|
| COM worker **implementation** (COM-2..6), Word tier G3 | **M6** — host expected mid-M5 (C1); M5 carries zero Windows dependency |
| xlsx formula evaluation | **M6** — ADR + minimal core subset (C3) |
| Actual public launch (D18 flip, registry publishes) | **M6 or later**, owner-timed (D26); M5 ships prep only |
| ODS/ODP revision write | M6 (odt-first shipped; spreadsheet/presentation revisions behind) |
| Word-interop verification of accept/reject | with the M6 Windows host (same box makes it cheap) |
| PDF redaction, content-stream surgery | post-M5 (needs its own ADR; PDF/A death + signature death interplay) |
| PAdES-T/-LT/-LTV, trusted certs, OCSP/CRL | post-M5 once B-B ships; TSA hook left honest-absent |
| pdf.js annotation rendering / full viewer swap | only if D24's own revisit trigger fires (human needs native annot rendering) |
| Multi-user / hosted review | not planned (local single-reviewer by design) |
| G5 pdfium | stays gated |

## 7. M5 exit gate (measurable; asserted by `tools/phase5_gate.sh`, step P5-18)

1. **PDF annotation round-trip**: enumeration parity + GP-A refusals on ≥ 12 new PDF-annot
   fixtures; write-back append-only prefix + journal + byte-identical revert; LO/gs render goldens
   show highlight + comment states.
2. **PDF review session e2e (headless HTTP)**: create → list → resolve via token → pdf updated +
   journaled; security suite green (token, traversal, CSP, XSS **including text-layer strings**).
3. **pdf.js pinned**: vendored release + hash manifest in-repo; gate verifies hashes; no-CDN test
   (asset fetch works with network disabled); selection→quote anchor round-trip asserted.
4. **Trust**: signed-doc mutation detection on every mutation path (goldens); PAdES-B-B golden
   verifies via `qpdf --check` + openssl byte-range + tamper-evidence test; PDF/A revalidation
   wired (honest-absent preserved).
5. **Fuzz round 4** (annotation dictionaries, signature/byte-range structures, text-layer data)
   clean + SEC-06 Phase-5 review + threat-model update.
6. **Launch prep complete**: hygiene pack merged; publish dry-runs green and recorded — **no
   publish performed** (verified by absence: no version tags pushed, registries untouched).
7. **C7 v1.4 frozen additive** (23 tools / 21 verbs measured; CHANGELOG breaking-change scan).
8. **Hygiene + gate**: `phase5_gate.sh` green **headless** + CI; **tag `m5` moves only to a
   committed, gate-green tree** (M4 lesson), acceptance report committed before the tag.

## 8. Risks

| Risk | Mitigation |
|---|---|
| pdf.js DOM is a new XSS surface (document text becomes live nodes) | text layer only, no viewer chrome; CSP `script-src 'self'`; our data stays in escaped JSON; XSS corpus through text-layer strings (fuzz round 4); pinned vendored build, gate hash check; opt-in keeps the default session JS-free |
| PAdES byte-range bugs silently produce invalid signatures | independent verification is part of every signing golden (`qpdf --check` + openssl), not our own reader alone; tamper-evidence test proves the verify actually discriminates; refuse-list on `/AcroForm` conflicts |
| Crypto supply chain breaks D2 ("C stays out") | ADR-0010 restricts to pure-Rust RustCrypto family (`rsa`/`ecdsa`/`sha2`/pkcs/x509); `cargo deny` keeps `-sys` crypto crates out; minimal feature flags |
| Annotation semantics differ across PDF readers (statuses, subtypes) | ship the accepted state vocabulary only (accepted/rejected); unknown subtype → refuse; LO render goldens as visual cross-check; no promises about third-party reader UI |
| Incremental-update + signing interplay (annotate-after-sign) | §5 ordering rule: sign is last; post-sign mutations need explicit policy pass, recorded; refuse-by-default |
| Scope bleed from M6 (COM host arrives mid-phase, "while we're at it") | §6 is hard scope; the host lands **for M6**; a mid-phase Windows box changes no M5 gate; UI scope follows the same rule as M4 |
| Launch prep accidentally triggers a publish | dry-runs only, `--dry-run` pinned in every recorded command; gate asserts absence of publishes; flip is D26 owner action |
| Fixture licensing for signed/annotated PDFs | generator-made fixtures (python: pypdf/reportlab-class in harness; our own writer for simple cases); license review inside P5-02 |

## 9. Decisions/contracts this phase stands on

**Unchanged:** D2–D21; **D22 refined by D24** (raster overlay stays default; opt-in vendored pdf.js
text layer is the revisit-trigger's concrete shape).
**Proposed decision rows (ratified + recorded into `project_concept.md` §16 by P5-01):**

| # | Decision | Rationale | Revisit when |
|---|---|---|---|
| D23 | Office add-in **stays deferred** (ratifies the spike report's proposal row) | distribution blocker (side-load/AppSource) unchanged; browser review shipped | addin-spike triggers 1–4 |
| D24 | **pdf.js text-layer wave** fires — vendored, hash-pinned, **text layer only, opt-in per session**; raster remains default | M4 intake accepted the PDF review loop; text-selection anchoring is D22's stated trigger; containment beats bundling a viewer | humans need native annot rendering or pdf.js as primary medium |
| D25 | **Pure-Rust crypto** for PAdES-B-B (RustCrypto family; no `ring`, no OpenSSL, no `-sys`) | D2/D4 preserved; signing must not drag C into the binary; `cargo deny` extends the `-sys` ban to crypto | a hard feature gap (e.g. required smart-card/TSA interop) forces a rung |
| D26 | **Launch is owner-timed**; M5 ships prep only; D18 stays until the flip | registry/visibility flip is one-way; prep is reversible | owner call in M6 |

**New ADRs:** **ADR-0009** (pdf.js vendoring: release pinning, hash manifest, offline serving,
text-layer-only scope, XSS containment), **ADR-0010** (crypto stack + PAdES subset; D25's concrete
crate set and refuse-list). Reserved numbers; `docs/adr/README.md` updated by P5-01.
**Contracts:** **C12 new** (PDF annotation-review model: kinds, anchors incl. quote, states,
GP-A0xx refusals) · **C13 new** (signature & integrity states: detection, invalidation event,
sign-later semantics, GP-S0xx) · C1 minor additive (`Document.annotations`, signature flag) ·
C4 extended (signature-invalidation budget entry) · C7 additive **v1.4** · C9 extended
(PDF-annot/signed fixtures + goldens) · C6/C10 consumed frozen.
ADR-0007 extended (session over PDF), ADR-0001/D19 consumed (incremental writer is the write path).

## 10. Owner scope calls 2026-09-18 (this session)

- **C1** — **Windows host with M365 will exist mid-M5**; COM implementation therefore **schedules
  into M6**, not M5 (no M5 gate depends on a Windows box; host arrives ready for M6 kickoff).
- **C2** — **Public launch is prep-only in M5**: hygiene pack + dry-runs land; the flip stays
  owner-timed (D26).
- **C3** — **xlsx formula evaluation**: ADR + minimal core subset (arithmetic/aggregate/logic +
  cached-vs-computed parity lint), scheduled **M6**.
- **C4** — **PDF review medium**: raster overlay (reuse D22 pattern) as default + **opt-in vendored
  pdf.js text layer**; recorded as D24.
- **C5** — **Milestone split**: M5 = PDF loop + trust (+ launch prep, fuzz, gate); M6 intake =
  COM implementation, formula eval, actual launch, ODS/ODP revisions, Word-interop verification.

Flagged at intake, owned by this plan: **P5-01** fixes the documented surface counts (docs say
19 tools / 18 verbs; measured 20 / 19) and formally closes **OQ3** (G2, `gp` alias — de facto "no"
since CLI v1).
