# stichos — Phase 5 plan: agent-sized steps (M5)

**Status:** active plan · derived from [`phase5-concept.md`](phase5-concept.md) (scope, gate) and
`project_plan.md` §5 tracks (PAT, API, PDF, CORE, ENG, LIN, SEC, OPS, GOV) ·
**19 steps, 6 waves (0, P, J, T, G, X), 6 gates**.
**Canonical progress state:** [`phase5-status.json`](phase5-status.json) — this file defines the steps;
the JSON tracks them. Do not maintain status in two places.
**Last updated:** 2026-09-18

---

## 1. How this plan is executed (agent rules)

- **One agent, one step, one session.** Budget ~200k tokens. Load only: your step row + wave gate
  here, the named contracts in `docs/contracts/` + ADRs, the crate(s) you touch, the named corpus
  subset. Fetch the specific ISO 32000 clause you need (§12.4 Annotations, §12.7.4 Digital
  Signatures, Table 169 annot subtypes; PAdES B-B = ETSI TS 102 778-3 / ISO 32000-2 §4.5.5), not
  the spec.
- **Sizing rule:** every row scoped to one 200k session (≈ ≤1 200 new LOC + tests). **2s** rows =
  two sequential sessions with a written handoff in the step's status entry. If a step grows past
  scope, stop, record `blocked: split-needed` + evidence; do not sprawl.
- Write only inside the step's deliverable paths + its row in `phase5-status.json`.
- `project_concept.md` / `project_plan.md` / `phase5-concept.md` are **not edited by executing
  agents** (P5-01 is the one owner-authorized write: decision rows D23–D26 into §16). A
  contradiction stops the step and flags the owner. Measured vs assumed, always.
- Definition of done: `project_plan.md` §8 + the step's **Verify** column, run and recorded with
  command + result + date.
- Status: `pending → active → verified | blocked(reason)`.
- **Headless discipline (M4-proven):** verdicts from **exit codes**, never from grepping piped
  output; everything must work with no `/dev/tty`.
- **Ship-state rule (new, from the M4 tag incident):** a milestone tag moves only to a
  **committed** gate-green tree — commit → gate → tag → push, in that order; acceptance report
  committed before the tag. Doc claims carry **measured** numbers (baseline today: 20 tools /
  19 verbs / 1 851 tests — the Phase-4 docs' "19/18" are wrong; P5-01 fixes them).
- **Host facts (measured 2026-09-18):** pandoc 3.7.0.2, soffice 26.2.6.1, pdftoppm, xmllint,
  **node v22.23.1 + npm** present; **qpdf, veraPDF, typst absent** (CI installs; honest-absent
  paths stay first-class). No Windows host during M5 scope decisions — **nothing here depends on
  one** (owner C1: host arrives for M6). Corpus: 157 manifests, `pr-subset` 27; **no PDF-annot or
  signed-PDF fixtures yet** (P5-02).
- Parallel lanes: **P ∥ T ∥ J-prep** after Wave 0 (different crates: `adapters-pdf` vs
  `gp-core`/`lint` vs `assets`/vendoring). Collision hotspots — **sequential edits only**:
  `mcp/http.rs` (P5-06 then P5-09), `mcp/spec.rs` + `docs/contracts/C7.md` (P5-07 then P5-13),
  `gp-core/src/ir.rs` (P5-03 then P5-11), `assets/reviewer/index.html` (P5-06 then P5-10).

## 2. Waves and gates

| Wave | Theme | Gate (command + pass condition) |
|---|---|---|
| **0** | intake hygiene + fixture corpus | decision rows D23–D26 recorded; doc counts measured-and-corrected; OQ3 closed; ≥12 PDF-annot + signed + PDF/A fixtures registered; gate scripts still headless |
| **P** | PDF annotation round-trip | enumeration parity + refusals on annot fixtures; write-back append-only prefix + journal + revert; `annots` verb + MCP tools e2e; C7 v1.4 part 1 |
| **J** | pdf.js text layer (opt-in) | vendored hash manifest gate-verifiable; offline asset test; selection→quote round-trip; XSS suite green through text layer |
| **T** | trust: signatures + PDF/A | signed-mutation detection goldens on every mutation path; PAdES-B-B golden verifies (qpdf + openssl + tamper test); PDF/A revalidation wired honest-absent |
| **G** | launch prep (no launch) | hygiene pack merged; publish dry-runs exit 0 and recorded; absence of any publish asserted |
| **X** | security + ship | fuzz round 4 + SEC-06; `phase5_gate.sh` green headless + CI; C7 v1.4 additive scan; tag `m5` on committed tree |

Waves ordered; inside a wave run in Deps order; parallel only where deps allow and files don't collide.

## 3. Steps

`Implements` cites `project_plan.md` §5; `Verify` = evidence the step must produce.
**Size:** 1 = one 200k session, 2s = two with handoff.

### Wave 0 — intake hygiene + fixture corpus

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P5-01 | Record owner calls: append **D23–D26** rows to `project_concept.md` §16 (text from `phase5-concept.md` §9); reserve ADR-0009/0010 in `docs/adr/README.md`; correct the documented surface counts to **measured** 20 tools / 19 verbs in `docs/support-matrix.md`, `CODEBASE.md`, `docs/m4-acceptance.md` (addition: re-measure via `spec.rs` assert + `enum Command` count, cite the command); formally close **OQ3/G2** (`gp` alias: no) in `project_concept.md` §18 | hygiene, GOV | — | 1 | decision rows present; `grep -c "19 tools" docs/support-matrix.md` → 0; a one-line verify script re-derives both counts from code and exits 0 |
| P5-02 | PDF review/signed fixture set: extend `harness/generator` with builders for **annotated PDFs** (Highlight+comment, FreeText, Stamp, mixed, damaged/orphan annot dict, ≥3-page multipage), **signed PDFs** (self-signed simple sig via harness-side tooling — license-checked pure-python or our writer), **PDF/A** sample; register ≥ 12 manifests + reference + goldens content-addressed | HAR-02-ext, M5-gate | — | 2s | ≥ 12 new manifests in `corpus/manifests` (script counts, exit code); every fixture parseable by our reader (or asserted refusal for the damaged one); LO/gs goldens stored |

### Wave P — PDF annotation round-trip

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P5-03 | C12 model in `gp-core`: `PdfAnnotation { id, kind(highlight/text/freeText/stamp), page, rect_pt, author, date, quote, state(applied/resolved/rejected) }`, `Document.annotations` (C1 minor additive), resolve-state transition table, GP-A0xx refusal codes registered (C3), write `docs/contracts/C12-pdf-annotations.md` incl. anchoring rules and the refuse-list (unknown subtypes, XFA) | CORE (C12, C1) | 0 | 1 | contract doc exists; model round-trips serde; C1 additive check (old IR deserializes without `annotations`); codes in C3 registry |
| P5-04 | `adapters-pdf` annotation enumeration → C12: annot dictionaries + `/Annots` arrays → `PdfAnnotation`s (Highlight `/QuadPoints`→rect, Text `/Contents`, FreeText, Stamp); page mapping; orphan/dup/malformed → GP-A refusal **never silent-drop**; census parity counters | PDF, C12 | 03 | 1 | enumeration parity vs dictionary counts on every P5-02 annot fixture; damaged fixture refuses with exact GP-A codes; nested/multiparent handled |
| P5-05 | Annotation write-back executor: resolve-state update + new Highlight/Text annot **via the M3 incremental writer** (append-only prefix, INV-1); span-surgical on the increment; journaled (C6); `dry_run` = IR diff + zero writes; refuse mutation of a signed doc → GP-S0xx (wire the detection stub from P5-11 if landed, else refuse on `/Sig` presence) | PAT, PDF-06-ext | 04 | 2s | property tests: pre-edit bytes are exact prefix of post-edit (every fixture); revert ⇒ byte-identical original; dry_run fs-assert zero writes; unrelated objects untouched |
| P5-06 | Review session over PDF: ADR-0007 session + ADR-0007 routes accept PDF docs; `data.json` gains `mode: "pdf-annot"` + annotation boxes; overlay renders annot boxes + resolve/reject buttons through the same action endpoint; version-hash invalidation unchanged; headless e2e | HUM-01-ext, ADR-0007 | 05 | 1 | headless e2e: `review` on pdf fixture → create → list → resolve via token POST → pdf updated + journaled; invalidation test; existing OOXML session tests unchanged (regression) |
| P5-07 | Surface: `stichos annots list\|resolve` + MCP `list_annotations` / `resolve_annotations` at schema **1.4.0**; C7 matrix + CHANGELOG (additive-only scan); measured counts in the matrix (22 tools after this step) | API-07-ext, C7 v1.4 | 06 | 1 | e2e CLI+MCP on fixtures; matrix green; additive-only scan passes; count assertion updated in `spec.rs` test with measured number |

### Wave J — pdf.js text layer (opt-in, vendored)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P5-08 | **ADR-0009** (pdf.js vendoring: exact release, hash manifest format, offline serving, text-layer-only scope, no viewer chrome, XSS containment plan) + vendoring landed: chosen pdf.js release files under `assets/vendor/pdfjs/` + `HASHES.toml`; gate-check snippet (hash verify) | D24, ADR-0009, SEC | 0 | 1 | ADR accepted by owner; `bash -c 'verify hashes'` exit 0; planted wrong-hash → exit 1; no build step, no npm at runtime |
| P5-09 | Server: `GET /viewer/pdf/*` (vendored assets, immutable cache headers), `GET /review/<token>/text/<n>.json` (text spans + rects + quote anchors from our extractor — **not** pdf.js output as truth); session opt-in `--text-layer`; CSP adjusted for text layer, still `script-src 'self'` | API-03-ext, HUM-01-ext | 06, 08 | 1 | routes tested headless; token required; traversal suite on the new asset routes; text spans come from our extraction, pdf.js used for layout/render only |
| P5-10 | Viewer: opt-in text layer over page images (node-testable logic split), selection → comment draft anchored `{page, rect, quote}` (C12 quote); text nodes created via `textContent` only; keyboard flow unchanged; ≤ 1k LOC growth budget for our JS | HUM-01-ext, D22/D24 | 09 | 2s | node logic tests (node v22 present); selection→quote round-trip e2e (fixture with known string); XSS corpus strings in fixture text render inert (server-side escaping + CSP asserted); our-JS LOC delta recorded |

### Wave T — trust: signature integrity + sign-later + PDF/A

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P5-11 | **ADR-0010** (crypto stack per D25: RustCrypto `rsa`/`ecdsa`/`sha2`/pkcs/x509 crates, feature flags, `-sys` crypto ban in `cargo deny`) + C13 model in `gp-core`: `SignatureInfo`, states `unsigned / signed-valid / signed-mutation-detected`, GP-S0xx codes, `docs/contracts/C13-signature-integrity.md`; read-side detection of `/Sig` fields into IR + budget (C4 extended) | CORE (C13), SEC, D25 | 0 | 2s | ADR accepted; `cargo deny` bans `-sys` crypto (planted-crate test fails); detection on P5-02 signed fixtures: presence + subject + digest surfaced in `inspect` |
| P5-12 | Mutation-path enforcement: every executor path (`apply`, `edit`, revisions accept/reject, `annots resolve`) consults C13 detection → **refuse by default** (GP-S0xx), explicit policy pass (C10) records `signature_invalidated` journal event + budget; goldens per mutation kind | PAT-05 upgrade, SEC-04 | 11 | 1 | goldens: each mutation kind on signed fixture ⇒ refusal w/ code; policy-pass ⇒ journal carries invalidation event; unsigned fixtures fully unchanged (regression) |
| P5-13 | **`stichos sign` + MCP `sign_document`** (C7 v1.4): PAdES-B-B over incremental writer — `/Sig` field, byte-range covering, SHA-256, self-signed or provided PKCS#12; TSA honest-absent (flag documented); refuse on pre-existing sig-field conflicts; verify story: own verifier + `qpdf --check` + openssl | API-07-ext, PAT, D25 | 12 | 2s | sign fixture → `qpdf --check` exit 0 (CI); independent byte-range verify (openssl) exit 0; **tamper test**: flip one byte ⇒ verify fails; sign-after-sign refuses w/ GP-S0xx; `--json` envelope + budget + journal id |
| P5-14 | PDF/A revalidation wired: mutation QA loop calls veraPDF/qpdf runner (LIN-05 glue exists) when probed; `doctor` row `pdfa_revalidation`; honest-absent path = skipped-with-note (host) vs real (CI); PDF/A fixture round-trip expectations | LIN-05-ext, OPS | 02, 12 | 1 | on CI: PDF/A fixture mutated → conformance report attached to budget; on host: honest-absent note asserted in doctor output; no new hard dependency |

### Wave G — launch prep (no launch)

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P5-15 | GOV hygiene pack: LICENSE dual (MIT/Apache-2.0) file audit vs per-crate manifests, source-header sweep, `CONTRIBUTING.md`, `SECURITY.md` (vuln reporting), DCO note, code of conduct; public-audience docs pass (README/CODEBASE: no local paths, no internal scratch refs); telemetry opt-in statement review (OPS-01 design doc) | GOV-05-ext, ship | 0 | 1 | files present; header-scan script exit 0; `grep` scan for local absolute paths in shipped docs → 0 hits |
| P5-16 | Publish dry-runs (recorded, **no publish**): `cargo publish --dry-run` (workspace, `--allow-dirty` forbidden — clean tree), winget manifest validation (schema-validated locally), Homebrew cask `--audit` syntax rehearsal, GitHub Release asset list rehearsal from `dist/` artifacts; all commands + exits recorded | DIS-ext, GOV-02-ext | 15 | 1 | every recorded command exit 0; record asserts **no registry contact** (dry-run flags in command lines); D26 citation: flip pending owner |

### Wave X — security + ship

| ID | Build → deliverable | Implements | Deps | Size | Verify |
|---|---|---|---|---|---|
| P5-17 | Fuzz round 4 + SEC-06: hostile annotation dictionaries (deep nesting, ref loops, oversized rects), signature/byte-range structures (maligned ranges, huge /ByteRange ints, conflicting /AcroForm), text-layer data.json strings (XSS corpus reuse), vendored-asset path attacks; regressions locked in `adapters-ooxfuzz`; threat-model Phase-5 section + SEC-06 checklist | SEC-01, SEC-06 | 05, 10, 13 | 2s | new fuzz tests green as regressions; clippy clean on fuzz crate; SEC-06 checklist committed; ≥ 24 h nightly-clean evidence path recorded (CI) |
| P5-18 | `tools/phase5_gate.sh`: asserts concept §7 items 1–8 **headless** (annot suites, session e2e, pinned-hash verify, signing goldens incl. tamper test, PDF/A path with honest-absent, fuzz status, C7 v1.4 additive scan, prep-absence check, hygiene); planted-violation proof; CI job `phase5-gate.yml` | ship | 16, 17 | 1 | gate exit 0 headless + CI green; planted violation (silenced annot test / wrong hash) → exit 1 |
| P5-19 | M5 acceptance report `docs/m5-acceptance.md`: all gate checks with measured vs assumed (what stayed CI-only); **commit → gate → tag `m5` → push** in that order (ship-state rule); Phase-6 intake (COM impl on arriving host COM-2..6, formula eval ADR+subset per C3, actual launch per D26, ODS/ODP revisions, Word-interop verification, PAdES-T/LTV) | ship | 18 | 1 | report committed **before** tag; `test "$(git rev-parse 'm5^{commit}')" = "$(git rev-parse HEAD)"` exit 0; push with owner approval |

## 4. M5 exit-gate ↔ steps map

| Gate item (concept §7) | Steps | Asserted by |
|---|---|---|
| PDF annotation round-trip | P5-02..07 | gate runs parity + write-back + revert suites; goldens present |
| PDF review session e2e + security | P5-06, 09, 10 | gate runs headless e2e + security suite (incl. text layer) |
| pdf.js pinned + offline + opt-in | P5-08..10 | gate verifies hash manifest + no-CDN + quote-anchor |
| Trust: sig detection + PAdES-B-B + PDF/A | P5-11..14 | gate runs detection goldens, signing verify + tamper test, PDF/A path |
| fuzz round 4 + SEC-06 | P5-17 | nightly status + review doc |
| launch prep w/o launch | P5-15, 16 | hygiene files + recorded dry-runs; absence asserted |
| C7 v1.4 frozen additive | P5-07, 13, 18 | schema matrix + changelog scan |
| hygiene + headless gate + tag rule | P5-01, 18, 19 | gate runs headless itself; tag==committed-tree check |

## 5. Sizing & context notes (the 200k discipline)

- **2s steps** (P5-02, 05, 10, 11, 13, 17): split = build-core first, wire/test second; handoff
  names files landed + the exact next assertion. Quote the spec clause (ISO 32000-1 §12.4/§12.7.4;
  ETSI TS 102 778-3 B-B; ISO 32000-2 §4.5.5) into the module doc in part 1.
- Corpus never loaded into context: fixture matrices run **in code**; steps cite counts.
- **pdf.js vendoring is the only new third-party *artifact*** — no new Rust crate outside the
  ADR-0010 crypto allow-list; anything else needs an ADR (concept §8 rule unchanged).
- Wave P ∥ Wave T ∥ P5-08 after Wave 0 (disjoint crates). Wave J integration (P5-09/10) needs
  P5-06 (session) and collides on `mcp/http.rs` + `assets/reviewer/index.html` — sequential.
  Wave G independent of P/J/T; X last.
- Honest absence stays first-class (qpdf/veraPDF/TSA); a step is **not** verified by skipping
  where CI can run the real thing.
- M4 carry-over watch-items: gate scripts already headless (no rework); counts measured per step;
  tag moves only after commit — P5-19 encodes the order.
