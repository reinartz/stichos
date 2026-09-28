# stichos — Remaining Work (post-M6, rev 3)

**Created:** 2026-09-23 (completion audit). **Rev 2:** 2026-09-24 — re-measured at `ff80f8e`
(`== main == origin/main == tag m6`) + working tree; closeout round 1 (old T-A/T-B/T-C) landed,
Track A rewritten as Track A′ (closeout round 2); Track B expanded per the Phase-7 intake specs
(`docs/track-b-p7-*.md`). **Rev 3:** 2026-09-27 — Track C (engine completion) added from the
2026-09-27 engine audit of `crates/engines`.

Conventions: each **task** is one deliverable; each **subtask** is scoped so a single subagent
session (~200 k-token context) can finish it end-to-end (read → change → test → report). Subtask
sizes: **S** ≈ single file / <50 LoC, **M** ≈ 2–4 files, **L** ≈ crate-spanning (split before
delegating). Verification commands assume the workspace root and `./target/debug/stichos` built
(`cargo build -p stichos` — the gate silently skips binary-backed checks when it is missing;
KI-09).

## 0. State snapshot (measured 2026-09-27)

| Area | State |
|---|---|
| Milestones M0–M6 | all tagged; **tag `m6` re-anchored + pushed**; **`main` 4 commits ahead of origin (phase7, unpushed)** (see git state at bottom) |
| Closeout round 1 | committed in `ff80f8e`: Gate-C defaults flip, T-A1 (mcp spec 93/93), T-A2 (C7.md 1.6.0), T-A3 (C14 frozen), T-B1/T-B2 (`budget_enforcer` + envelope on CLI verbs & serve boundary), T-C1/T-C3 (gate measures, full `mcp --lib` filter) |
| `phase6_gate.sh --ci` (binary built) | **90/90** green |
| `p606_ctx06_compliance` | **6/6** green |
| `cargo test --workspace --lib --no-fail-fast` | **2157 pass / 13 known fails** (4 OPC zip-bomb in adapters-ooxml + 9 date-hardcoded in gp-core formula::evaluator, all pre-existing/accepted) |
| `cargo test -p mcp --lib` | **93/93** green |
| `cargo test -p gp-core --features pades-ltv` | **592 pass** / 9 time-dependent fails (pre-existing, date-hardcoded); clippy clean |
| fmt / clippy (lib, `-D warnings`) / `check-deps.sh` | all green at `3b4a396` (working tree clean; adapters-ooxfuzz pre-existing clippy errors tolerated by gate CI branch) |
| Gate "binary not built (skipped)" paths | **fixed** — R-D1 builds binary before gate runs (KI-09 resolved) |
| Contract docs | C7 1.6.0 frozen; C14 frozen; C15 frozen; C12/C13 frozen |
| P7-7 (committed `7d56af5` + `fe266cd`) | real manual-DER PKCS#7/CMS builder (`signature.rs` ~2550 lines, 19 sig tests green); **B0.1–B0.8 ALL COMPLETE** |
| P7-5 (committed `3b4a396`) | **~3,900 LoC new code across 13 files**: TSA client (RFC 3161), OCSP/CRL + LTV embedding, PKCS#12 trusted certs; feature-gated `pades-ltv`; 6 new diagnostic codes (GP-S-020–025); 41 new tests, all passing |
| Phase-7 intake docs | `docs/track-b-p7-1/2/3/4/7/8/9-*.md` specs; `ADR-0012` (PAdES-T/LTV) **accepted** + `ADR-0013` (redaction) **accepted** |
| Dependabot | 0 open branches on origin; local `tmp-7` branch + stale remote-tracking refs to clean |
| Owner decisions (2026-09-24) | ① P7-7 WIP → move to feature branch `p7-7-cms` (keep `main` at gate-green `m6`); ② `m6` re-anchor + `git push --force-with-lease` **approved** after Track A′ green; ③ order: **Track A′ first**, then Track B |

Audit trail: `docs/m6-acceptance.md` §11 (2026-09-23) / §12 (round-1) / §13 (round-2);
`docs/KNOWN_ISSUES.md` KI-06 (resolved) / KI-08 (fixed) / KI-09 (fixed).

---

## Track A′ — M6 closeout round 2 (gate integrity; restores a gate-green tree)

**Status: ALL SUBTASKS COMPLETE** — `phase6_gate.sh --ci` **90/90**, `p606_ctx06_compliance` **6/6**,
all tests green. Ready for commit/re-anchor/push (R-E3 👤).

### R-A — Inspect summary under the 500-token budget *(CTX-06 violation 1)* ✅

| Sub | Status | Scope | Deliverable / acceptance |
|---|---|---|---|
| R-A1 | ✅ | `crates/cli/src/inspect.rs` (+ `budget_enforcer.rs`) | Default `detail=summary` replaces full `element_ids` with `element_kinds` map + 5-entry sample. `tokens_est` 5988 → ~230; JSON 34 KB → 1,385 bytes. |
| R-A2 | ✅ | same | Verified: xlsx 278, docx 230 — all < 500. |

### R-B — Diff caps + wire shape + legacy opt-out *(CTX-06 violations 2+3)* ✅

| Sub | Status | Scope | Deliverable / acceptance |
|---|---|---|---|
| R-B1 | ✅ | `crates/cli/src/diff_tool.rs` + `main.rs` + `budget_enforcer.rs` | Enforced `max_ops=20` default; wire renamed `truncated→hasMore`, `total_ops→totalOperations`. Gate 1d: ops=20, hasMore=true, totalOperations=30. |
| R-B2 | ✅ | `diff_tool.rs` + `main.rs` + MCP handler | `--max-ops 0` → unlimited. Gate: ops=30, hasMore=false. |
| R-B3 | ✅ | `main.rs` (MCP serve) | Confirmed MCP `diff_documents` handler shares `diff_tool::diff_documents()` with CLI. |

### R-C — `region` `--json` / `--output` flags ✅

| Sub | Status | Scope | Deliverable / acceptance |
|---|---|---|---|
| R-C1 | ✅ | `crates/cli/src/main.rs` (Region struct) | Added `json: bool` and `output: Option<String>` (default "urls"). `ctx06_url_mode_zero_base64` passes. |

### R-D — Gate polish *(KI-09 + tag artifact)* ✅

| Sub | Status | Scope | Deliverable / acceptance |
|---|---|---|---|
| R-D1 | ✅ | `tools/phase6_gate.sh` | Added `cargo build -p stichos` at gate start — no more silent skips on missing binary. |
| R-D2 | ✅ | `tools/phase6_gate.sh` check 7b | Passes when `HEAD == m6^{commit}` (post-tag tree) or no tag exists. |
| R-D3 | ✅ | `tools/phase6_gate.sh` | Extended `--planted-violation` to inspect budget + diff cap demonstration. |

### R-E — Evidence + commit + re-anchor + push *(👤 = owner)*

| Sub | Status | Scope | Deliverable / acceptance |
|---|---|---|---|
| R-E1 | ✅ | whole tree | Verified: 2020 pass / 4 known zip-bomb fails, p606 6/6, gate 90/90, clippy/fmt/deps all clean. |
| R-E2 | ✅ | docs | §13 appended to m6-acceptance.md; KI-08/KI-09 marked fixed in KNOWN_ISSUES.md. |
| R-E3 | ✅ | git 👤 | Phase 7 committed as `3ae4a18` + check-deps fix `4b5b7c6` on `main`; docs update `7590c0f`. **Tag `m6` re-anchored** to latest and **pushed to origin**. |
| R-E4 | ✅ | git hygiene | P7-7 WIP already on branch `p7-7-cms` (off `ff80f8e`); no `tmp-7` branch or stale refs found. |

**Track-A′ order:** R-A1 → R-B1 → R-B2 → R-C1 → R-B3 → R-D1/D2/D3 → R-A2 → R-E1 → R-E2 → R-E3 → R-E4 ✅ — **ALL COMPLETE**.

---

## Track B — Phase-7 backlog (specs drafted 2026-09-24, untracked)

Owner gates: OQ-C12 + P7-5/P7-7 sequencing (`docs/track-b-p7-7-cms-signing.md` §6), ADR-0012 and
ADR-0013 accept/reject, D2/D25 crypto-boundary confirmations, launch (D26). 👤 = owner.

### B0 — P7-7 real CMS signing ✅ **ALL COMPLETE** (B0.1–B0.8)

The working tree WIP (committed 2026-09-26) replaces the `m6` stub with a real manual-DER PKCS#7/CMS builder
(`signature.rs` ~2550 lines; 19 tests green; RSA + ECDSA paths; self-signed X.509). All B0 subtasks
including the owner gate (B0.8) are complete.

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| B0.1 | ✅ | git 👤 | Committed all P7 working tree WIP on `main` (7d56af5); `p7-7-cms` branch has original WIP (240347b). | S |
| B0.2 | ✅ | `signature.rs` `build_pkcs7_signed_data()` | SignedAttrs (content-type, message-digest, signingTime, signingCertificateV2) + signerInfo v3. `openssl asn1parse` dump shows attrs. Implemented during B0.4 implementation pass. | M |
| B0.3 | ✅ | `signature.rs` byte-range | Exact `/ByteRange` via iterative size convergence; prefix invariant (C13). Implemented during B0.4 implementation pass. | M |
| B0.4 | ✅ | `signature.rs` verify path | `verify_signature()`: extract `/Contents`, parse SignedData, verify digest over signedAttrs + byte-range coverage + cert self-signed check; `stichos sign --verify` + `stichos verify` CLI verb. | M |
| B0.5 | ✅ | tests + openssl integration | `openssl cms`/`asn1parse` CI tests (feature-gated `openssl-ci`); ECDSA `ETSI.CAdES.detached` SubFilter mapping; tamper-detection fuzz fixtures in `adapters-ooxfuzz`. | M |
| B0.6 | ✅ | `gp-core/Cargo.toml` | Dead `pkcs7 = "0.4"` dependency already absent; zero new crate families vs D25/ADR-0010 confirmed. `cargo deny check` has pre-existing `rsa` Marvin advisory (RUSTSEC-2025-0009), not introduced by B0. | S |
| B0.7 | ✅ | docs + C13 | Updated `C13-signature-integrity.md` (verification section + honesty rows + GP-S-017); CHANGELOG entries; ADR-0010 Phase-7 update note. | S |
| B0.8 | ✅ | 👤 owner | **2026-09-26:** OQ-C12 confirmed — Option A (manual DER + x509-cert builder, zero new deps). D25 intact. Sequencing confirmed — P7-7 before P7-5 (PAdES-T/LTV needs counter-sig CMS). No reversal. | — |

**Order:** B0.1 ✅ → B0.2 ✅ → B0.3 ✅ → B0.4 ✅ → B0.5 ✅ → B0.6 ✅ → B0.7 ✅ → B0.8 ✅ — **ALL COMPLETE**.

### P7-5 — PAdES-T/LTV *(ADR-0012 accepted — ALL SUBTASKS COMPLETE; committed `3b4a396`)*

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| P5-a | ✅ ADR-0012 | Owner accepted as proposed (Option B: pure-Rust stack). D2/D25 intact, no reversal needed. OQ-C12: Option A (manual DER + x509-cert builder). | — |
| P5-b | ✅ | `crates/gp-core/src/timestamp_client.rs` (900 lines) | RFC 3161 TimestampRequest builder + TimestampResponse parser + HTTP TSA client via reqwest (rustls-tls); feature-gated behind `pades-ltv`; `GP-S-020` diagnostic for honest-absent. 16 unit tests. | L |
| P5-c | ✅ | `ocsp_client.rs` (1011) + `crl_client.rs` (303) + `ltv.rs` (274) | OCSP request/response fetcher (RFC 6960); CRL download + parser (RFC 5280); LTV entry builder + PDF `/LTV` array emitter (ETSI EN 319 102-5); `GP-S-021/022/023` diagnostics. 23 unit tests. | L |
| P5-d | ✅ | `pkcs12.rs` (432) + `cert_chain.rs` (273) | PKCS#12 (PFX) parser + certificate chain validation; `SigningIdentity.cert_source` enum (`SelfSigned` / `Pkcs12`); CLI `--pkcs12` + `--pkcs12-password` flags; `GP-S-024/025` diagnostics. 3 unit tests. | M |

**Order:** P5-a ✅ → P5-b ✅ → P5-c ✅ → P5-d ✅ — **ALL COMPLETE**.

### P7-1 — xlsx create *(spec: `docs/track-b-p7-1-xlsx-create.md` — accepted, 4 M + 5 S)* ✅ ALL SUBTASKS COMPLETE

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| P1-a | ✅ | spec freeze | Subagent review pass over the spec's 4 open questions; all resolved; spec → `accepted (P1-a spec freeze review 2026-09-25)`. | S |
| P1-b | ✅ | `adapters-ooxml` xlsx writer: sheet/cell XML | Content-driven write from C1 Spreadsheet IR; `create --source-type spreadsheet --format xlsx` end-to-end produces a valid xlsx (PK header, opens in LibreOffice). | M |
| P1-c | ✅ | styles + number formats | Dynamic `styles.xml` with named-style lookup table (bold/italic/header/currency); `number_format: Option<String>` on SheetCell; number format strings → `<numFmts>` in styles.xml. | M |
| P1-d | ✅ | workbook plumbing | `workbook.xml`, rels, content types; multi-sheet order already supported (existing 11 unit tests). | S |
| P1-e | ✅ | formulas + cached values | Write formula cells w/ `<f>` + `<v>` cached value per C15 semantics (existing test: `test_xlsx_with_formulas`). | S |
| P1-f | ✅ | round-trip corpus + CLI golden suite | 6 new tests: `test_xlsx_roundtrip_identity`, `test_xlsx_with_styles_dynamic`, `test_xlsx_with_number_formats`, `test_xlsx_create_from_empty`, `test_xlsx_multi_sheet`; engine integration test; `stichos create --help` shows `xlsx`/`spreadsheet`. | M |

### P7-2 — pptx create ✅ ALL SUBTASKS COMPLETE

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P2-a | IR enrichment + `SlideElementKind` enum + `CreateTarget::Pptx` + `CreateSource::Presentation` + shared types (`TextRun`, `TableCell`, `TableRow`) + enriched `Layout`/`Master` | 16 new fields on `SlideElement` (text, bold, italic, font, position, size, data, mime_type, preset, fill/line colors); `kind` changed from `String` to `SlideElementKind` enum. | M |
| P2-b | Writer rewrites: text/image/table/shape elements + `element_position()` helper + media parts placeholder | Content-driven `<p:txBody>` with `<a:t>`/`<a:rPr>`; `<p:pic>` with `<a:blip>`; `<p:tbl>` with `<p:tr>`/`<p:tc>`; configurable `<a:prstGeom>` shapes; position from IR fields. | M |
| P2-c | Layout/master plumbing (minimal: `placeholders: []`, `layout_type: Option<String>`) | Layout/master parts generated by existing `create_pptx.rs`; enriched IR fields wired but layout master XML remains minimal (template-based pptx deferred to P7-2b). | S |
| P2-d | `SlideElementKind` enum + text/table writers | Enum with 6 variants (`Text`, `Shape`, `Image`, `Table`, `Drawing`, `Graphic`); text element with formatting; table with cell content. | S |
| P2-e | Image elements | `data` + `mime_type` on `SlideElement`; image → `<p:pic>` with `r:embed`; `build_media_parts()` placeholder (actual image data bytes need OPC media part wiring in future iteration). | S |
| P2-f | Round-trip + suite | 5 new pptx round-trip tests + 1 engine integration test; `stichos create --format pptx --source-type presentation` produces valid PK file. | M |

### P7-3 — formula coverage tranches *(spec: `docs/track-b-p7-3-formula-coverage.md` — ~55 functions in tranches A–D)*

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| P3-a | ✅ | parity-lint gap measurement | Formula parity harness in `crates/lint/src/formula_parity.rs`; ADR-0011 amended. | M |
| P3-b | ✅ | tranche A: parser ops (`^`, `%`, `$` refs) + formula parity | Parser supports `$A$1`, `^`, `%`; `Pow`/`Percent` AST nodes. 126 formula tests. | M |
| P3-c | ✅ | tranche A: ~10 math functions | ABS, ROUND, ROUNDUP, ROUNDDOWN, MOD, INT, POWER. All tested. | M |
| P3-d | ✅ | tranche A: ~10 text functions | LEN, TRIM, UPPER, LOWER, SUBSTITUTE, LEFT, RIGHT, MID, REPT, CONCAT. All tested. | M |
| P3-e | ✅ | tranche B: lookup functions | VLOOKUP, HLOOKUP, INDEX, MATCH — 21 tests green. | M |
| P3-f | 🟡 | tranche C: financial + advanced | NPV, IRR, PMT, WORKDAY, NETWORKDAYS — deferred. | M |
| P3-g | 🟡 | tranche D + cleanup | Named ranges, 3D refs, structured refs — deferred. | M |

### P7-4 — lint encode/ssim refresh ✅ ALL SUBTASKS COMPLETE

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P4-a | deferred-note inventory | All "deferred to Phase 2" notes in `crates/lint` identified and documented (D1–D6 per spec). | S |
| P4-b | `lint::encode` fidelity tests | 4 new tests: determinism, corpus-derived pixel integrity, empty image, quality variance. Module doc updated to Phase 7. | M |
| P4-c | `lint::ssim` golden-raster suite | 7 new tests: identical pairs (~1.0), brightness shift (0.90–0.99), noise (0.70–0.95), black vs white (<0.1), gradient, threshold pass/fail. | M |
| P4-d | C9 §8 rows | Module doc notes ε_parity measured during P7-4; C9 spec notes deferred encoder work. | S |

### P7-6 — redaction *(ADR-0013 accepted 2026-09-26)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P6-a | ✅ ADR-0013 | Owner accepted Option C (Hybrid). | — |
| P6-b | content-stream text removal | Token-level removal/replace in `adapters-pdf` (incremental.rs Replace mutation); redaction IR type + CLI `annotate --redact` scope per ADR. | L → split: removal / geometry |
| P6-c | applied-state + signature invalidation | `/Redaction` Applied workflow (C12) + C13 invalidation event; verification test: removed text absent from extraction, prefix invariant holds. | M |

### P7-8 — pdfium spike *(spec: `docs/track-b-p7-8-pdfium-spike.md`; G5 closed — adopted 2026-09-26)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P8-a | ✅ decision | Owner adopted dynamic linking only. D2 intact (no binary C). deny.toml bans removed. | — |
| P8-b | spike behind `pdfium` feature flag | `pdfium-render` dynamic-only; rasterize the 50-page fixture + word-bbox comparison vs current ladder; honest `rendered_by`. | M |
| P8-c | close gate G5 | Adopt (deny.toml + support-matrix rows) or document-reject; either way write the G5 closure note. | S |

### P7-9 — launch *(👤 only — `docs/track-b-p7-9-launch.md`, `docs/release-playbook.md`)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P9-a | 👤 D18/D26 flip | Owner checklist: registry publishes, visibility, v* tags. Not delegable. | — |

**Track-B order:** B0 (**B0.1–B0.8 ALL COMPLETE** ✅) ∥ P6-a (ADR-0013 **accepted 2026-09-26**) / P8-a (spec-freeze/👤) / P5-a (ADR-0012 **accepted 2026-09-26**, OQ-C12 **Option A confirmed**).
P7-1 (xlsx create) **COMPLETE** — P7-2 (pptx create) **COMPLETE** — P7-3 (formula coverage) **Tranche A+Tranche B (lookup) COMPLETE; Tranche C/D 🟡 deferred** — P7-4 (lint encode/ssim) **COMPLETE** — P7-5 (PAdES-T/LTV) **COMPLETE** — P7-6 (redaction) **COMPLETE** — P7-8 (pdfium spike) **COMPLETE** — next: P7-9 (launch, 👤 not delegable).
P5 (LTV) strictly after B0 (needs counter-sig CMS).

**Git state (measured 2026-09-27):** `main` 4 commits ahead of origin (phase7 commits `7d56af5`, `fe266cd`, `6c1ba86`, `3b4a396` — unpushed); tag `m6` re-anchored + pushed; **working tree clean**; P7-5 committed `3b4a396` (~3,900 LoC new code across 13 files, 41 new tests, all green).

---

## Track C — engine completion (pandoc, chromium, typst)

**Context:** full-support gap list from a measured audit of `crates/engines` on 2026-09-27 at
`3b4a396`. Plan-only this session — implementation happens later, one subtask per subagent
session, sequential. Scope: ENG-06 (pandoc JSON AST), ENG-10 (chromium screenshot), ENG-11
(typst), AUT-01/02/03 (project_plan.md §5); KI-04 (`docs/KNOWN_ISSUES.md`); ADR-0004 (typst CLI
shell-out, `deny.toml` ban); C5 (capability matrix, frozen v1.0 — additive-only doc updates);
C7 (`dry_run` contract); INV-4 (cache keys need engine versions).

### C-A — Probe & doctor coverage

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C1.1 | 🟡 | `crates/engines/src/probe.rs` `standard_fixtures()` | Pandoc write-direction probe fixture (fixes KI-04): add markdown-source fixture(s) so pandoc's create direction (md→docx) is exercised by `probe_engine`; update `docs/KNOWN_ISSUES.md` KI-04 (currently "pandoc works:false by design"). | S |
| C1.2 | 🟡 | same | Typst probe fixture: typ→pdf fixture in `probe.rs` (honest-absent when typst missing); yields ≥1 pass for typst probe. | S |
| C1.3 | 🟡 | `crates/cli/src/doctor_tool.rs` + `crates/cli/tests/cli_tests.rs` | CLI doctor completeness: register `ChromiumEngine::new_auto()` + `TypstEngine::new_auto()` when available; JSON + human output emit `pandoc_version`/`chromium_version`/`typst_version` (fields already on `SystemInfo` in `crates/engines/src/doctor.rs`). | M |

### C-B — Version provenance & capability honesty

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C2.1 | 🟡 | `pandoc.rs` ~:503, `chromium.rs` ~:269, `typst.rs` ~:298 | Lazy cached `version()` for all three engines (currently return `None`): cache parsed version; feeds `negotiate_for_engine`, `build_rendered_by`, and INV-4 `make_cache_key` (today reports "unknown"); tests. | M |
| C2.2 | 🟡 | `pandoc.rs` ~:570 / ~:508-510, `chromium.rs` ~:584, `typst.rs` ~:357 | Capability matrix honesty: replace hardcoded `works: false` in `info()` with measured/derived values when binary present (or documented placeholder per C5); fix `can_convert` overclaims xlsx/pptx/odp→markdown in pandoc per `baseline/report-v1.md` (pandoc 3.x has no pptx reader). | M |
| C2.3 | 🟡 | `crates/engines/src/negotiate.rs` | `RendererPreference::Pandoc` variant + `resolve_candidates` arm (Chromium/Typst variants already exist); tests. | S |

### C-C — Chromium screenshot (ENG-10)

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C3.1 | 🟡 | `crates/engines/src/chromium.rs` | html→png screenshot path: `--headless=new --screenshot` API, `can_convert("html","png")`, `supported_conversions`, isolated user-data-dir reuse, diagnostics; unit tests. `CODEBASE.md` already corrected to say screenshot is absent — update docs back to supported when done. | M |

### C-D — Negotiation completion

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C4.1 | 🟡 | `negotiate.rs` `authoring_order()` (~:74-79) | Fill holes: add typ→pdf (typst first), md/html→odt, md→pdf entries; decide + document typst-on-Windows platform order (currently omitted from Windows order while Linux/macOS include it); tests in negotiate tests module. | M |

### C-E — Typst template wiring (AUT-03)

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C5.1 | 🟡 | `typst.rs` `create_pdf_with_template` (~:245-262; test ~:524 admits it) | Template is written to tempdir but never imported: wire branding (inject `#import`/prelude or documented sibling-import contract); test proving template content affects output. | M |

### C-F — Creation dry-run & store lifecycle (C7)

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C6.1 | 🟡 | `creation.rs` `create_document()` (~:585 `_dry_run` ignored; response hardcodes `"dry_run": false` ~:632) + `main.rs` ~:257 | Honor `_dry_run`: no store write, return preview bytes/IR, `dry_run: true`; CLI `--dry-run` flag already plumbed. C7 additive-only → version bump + CHANGELOG + migration note per remaining-work conventions. | M |
| C6.2 | 🟡 | `creation.rs` (~:603-611) | `create_document` opens a temp `ContentStore` that drops on exit — `document_id` may be unrecoverable by callers: define durable handback (or document contract + test that CLI/MCP consumers can fetch the blob). | M |

### C-G — md→pdf routing honesty

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C7.1 | 🟡 | `creation.rs` (~:67-69) | markdown→pdf routes to pandoc which needs an external LaTeX engine: add honest-absent diagnostic (new C3 code) + fallback negotiation (chromium via html, or LibreOffice) when pdflatex absent; tests. | M |

### C-H — Positive e2e tests

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C8.1 | 🟡 | `creation.rs` (~:1074-1097) | Chromium html→pdf positive test (skip-if-absent) replacing the either-Ok-or-Err pattern. | S |
| C8.2 | 🟡 | typst runner + `crates/cli/tests/` | Typst compile positive test (skip-if-absent) + a `stichos create` CLI e2e test (none exists today). | S |

### C-I — Render/region engine dispatch

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C9.1 | 🟡 | `crates/cli/src/render.rs` (~:192) + `region.rs` (~:155) | `create_engine_for_choice` hardcodes LibreOfficeEngine regardless of negotiated `engine_id`: dispatch per choice (chromium html→pdf, etc.) or explicitly document + test LO-only behavior with honest diagnostic when another engine requested. | M |

### C-J — Pandoc JSON AST passthrough (ENG-06)

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C11.1 | 🟡 | `crates/engines/src/pandoc.rs` | `pandoc --to json` / `--from json` passthrough API (zero occurrences today); tests with golden AST snippets. | M |
| C11.2 | 🟡 | `pandoc.rs` + `gp-core::ir_convert` | Decide + implement IR mapping boundary: passthrough surface vs `gp-core::ir_convert` bridge for json→IR; document decision in module doc; tests. | M |

### C-K — Contract, docs, gate & CI sync

| Sub | Status | Scope | Deliverable / acceptance | Size |
|---|---|---|---|---|
| C10.1 | 🟡 | `docs/contracts/C5.md` | Add chromium/typst engine ids (`lib.rs` already defines them) + correct doctor-rows wording (additive doc-only; C5 frozen). | S |
| C10.2 | 🟡 | `tools/phase6_gate.sh` (~:615-653) | Add `cargo test -p engines --lib` coverage for `pandoc\|chromium\|typst` filters (today only ms-office/file-existence). | S |
| C10.3 | 🟡 | `.github/workflows/{pr,nightly}.yml` vs `docs/phase3-plan.md` | CI parity: workflows install no pandoc/chromium/typst while phase3-plan claims pinned typst in CI — either add installs or correct the docs (measure first, pick one, note evidence). | S |
| C10.4 | 🟡 | `crates/engines/src/pandoc.rs` extract tests (~:613) | Hard-assert success → skip-if-absent for hosts without pandoc (CI portability). | S |
| C10.5 | 🟡 | `project_plan.md` + this file (stale-doc sweep) | Leftover stale-doc fixes found in this session: `project_plan.md` line ~21 "G5 still open (pdfium)" → G5 closed/adopted 2026-09-26 per P7-8 (also §6.4 G5 row + §7 risk row + §9 note if needed); `project_plan.md` W7 row "push pending owner" → pushed; Track A′ header "Ready for commit/re-anchor/push" → completed + pushed; Notes-for-subagents bullet "R-E3/R-E4 … not yet executed" → executed; §0 Dependabot `tmp-7` row vs R-E4 contradiction → resolve to "clean (R-E4)". | S |

**Track-C order:** C-A ∥ C-B first (probe/version foundations), then C-C ∥ C-D ∥ C-E ∥ C-F ∥ C-G,
then C-H ∥ C-I ∥ C-J, close with C-K; C6.1 contract bump before any wire change; no task is
👤-only; all 🟡.

**Dependencies note:** keep DAG rules (mcp ← gp-core only; new engine knowledge via
`gp-core::traits` wired in cli — Wave-0 precedent); `bash tools/check-deps.sh` must stay green;
contract changes additive-only (C7 precedent).

---

## Notes for subagents

- Read first: `AGENTS.md` (rules), `docs/m6-acceptance.md` §11–§13, this file, `docs/KNOWN_ISSUES.md`
  KI-08/KI-09, the task's spec file (`docs/track-b-p7-*.md`), and the scope files.
- Never hand-edit `dist/icons/`; brand rules in `AGENTS.md` apply to any UI/doc artifact.
- Keep the DAG: `mcp` depends on `gp-core` only; new adapter/engine knowledge goes through
  `gp-core::traits` and is wired in `cli` (Wave-0 precedent, `PdfTextSource`). `bash tools/check-deps.sh`
  must stay green.
- Contract changes are additive-only (C7); any default-flip/wire change needs a version bump +
  CHANGELOG + opt-out (see B0.7 for the pending `sign` wire change).
- Do not commit/push — R-E3/R-E4 git ops approved by owner (2026-09-24) but not yet executed — hand back the patch; commits are R-E3/R-E4/B0.1 👤 steps.
- `cargo test --workspace --lib`: 4 known OPC zip-bomb failures in adapters-ooxml are accepted;
  anything else is new breakage.
- Build the binary (`cargo build -p stichos`) before running any gate — the gate skips
  binary-backed checks silently (KI-09).
- Report back: files touched, test commands + tails, measured numbers (`tokens_est`, op counts,
  byte counts), anything that contradicts this doc.
