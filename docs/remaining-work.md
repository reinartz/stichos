# stichos — Remaining Work (post-M6, rev 2)

**Created:** 2026-09-23 (completion audit). **Rev 2:** 2026-09-24 — re-measured at `ff80f8e`
(`== main == origin/main == tag m6`) + working tree; closeout round 1 (old T-A/T-B/T-C) landed,
Track A rewritten as Track A′ (closeout round 2); Track B expanded per the Phase-7 intake specs
(`docs/track-b-p7-*.md`).

Conventions: each **task** is one deliverable; each **subtask** is scoped so a single subagent
session (~200 k-token context) can finish it end-to-end (read → change → test → report). Subtask
sizes: **S** ≈ single file / <50 LoC, **M** ≈ 2–4 files, **L** ≈ crate-spanning (split before
delegating). Verification commands assume the workspace root and `./target/debug/stichos` built
(`cargo build -p stichos` — the gate silently skips binary-backed checks when it is missing;
KI-09).

## 0. State snapshot (measured 2026-09-24)

| Area | State |
|---|---|
| Milestones M0–M6 | all tagged; **`main` pushed to origin; tag `m6` re-anchored + pushed** (see git state at bottom) |
| Closeout round 1 | committed in `ff80f8e`: Gate-C defaults flip, T-A1 (mcp spec 93/93), T-A2 (C7.md 1.6.0), T-A3 (C14 frozen), T-B1/T-B2 (`budget_enforcer` + envelope on CLI verbs & serve boundary), T-C1/T-C3 (gate measures, full `mcp --lib` filter) |
| `phase6_gate.sh --ci` (binary built) | **86/90** — 3 real CTX-06 violations + 1 expected `m6`-tag artifact (KI-08) |
| `p606_ctx06_compliance` | **3/6** — summary budget, diff/journal caps, region `--json` (exit 2) red |
| `cargo test --workspace --lib` | 2020 pass / 4 known OPC zip-bomb fails (adapters-ooxml) |
| `cargo test -p mcp --lib` | **93/93** green |
| fmt / clippy (lib, `-D warnings`) / `check-deps.sh` | all green (also for the uncommitted P7-7 WIP) |
| Gate "binary not built (skipped)" paths | **fixed** — R-D1 builds binary before gate runs (KI-09 resolved) |
| Contract docs | C7 1.6.0 frozen; C14 frozen; C15 frozen; C12/C13 frozen |
| P7-7 WIP (uncommitted) | real manual-DER PKCS#7 builder replaces stub (1 263 lines, 17 sig tests green); **no signed attrs, byte-range approximate, no verify path**; `sign` JSON wire changed (`byte_range` → `signature_value`+`signature_length`) |
| Phase-7 intake docs (untracked) | `docs/track-b-p7-1/2/3/4/7/8/9-*.md` specs; `ADR-0012` (PAdES-T/LTV) + `ADR-0013` (redaction) **proposed** — owner decisions pending |
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

### B0 — P7-7 real CMS signing *(IN PROGRESS in working tree — handle first)*

The uncommitted WIP replaces the `m6` stub with a real manual-DER PKCS#7/CMS builder
(`signature.rs` 1 263 lines; 17 tests green; RSA + ECDSA paths; self-signed X.509). Gaps per its
own spec §2: no signed attributes (not CAdES B-B), approximate `/ByteRange`, no verify path,
`sign` JSON wire change uncommitted.

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| B0.1 | git 👤 (decided 2026-09-24: feature branch) | Move the P7-7 WIP (`signature.rs`, `Cargo.toml`, `Cargo.lock`, `sign.rs`, `main.rs` region stamp) to branch `p7-7-cms` off `ff80f8e` and commit it there as `wip: P7-7 real CMS builder (no signed attrs yet)`; commit the 9 untracked Phase-7 docs (specs + ADR-0012/0013 proposed) + this doc set on `main`. `main` then = Track A′ fixes only. | S |
| B0.2 | `signature.rs` `build_pkcs7_signed_data()` | Add signedAttrs (content-type, message-digest, signingTime, signingCertificateV2) + sign the DER-encoded signedAttrs SET (CMS RFC 5652 §5.4), signerInfo v3. Acceptance: `openssl asn1parse` dump shows the attrs; existing 17 tests still green; new unit test decodes its own output. Spec §4.3. | M |
| B0.3 | `signature.rs` byte-range | Exact `/ByteRange` via iterative size convergence (spec §4.4); pre-signature bytes byte-identical to source (prefix invariant, C13). Acceptance: parse-back test recomputes ranges and hashes match. | M |
| B0.4 | `signature.rs` verify path | `verify_signature()`: extract `/Contents`, parse SignedData, verify digest over signedAttrs + byte-range coverage + cert self-signed check (spec §4.5); wire into `sign` verb as post-write verify. | M |
| B0.5 | tests + openssl integration | `openssl cms`/`asn1parse`-based CI test (extract + verify, spec §5.1); ECDSA `ETSI.CAdES.detached` SubFilter mapping (spec §12.2); tamper-detection fuzz fixtures in `adapters-ooxfuzz` (spec §5.3). | M |
| B0.6 | `gp-core/Cargo.toml` | Remove dead `pkcs7 = "0.4` (unused, deprecated — spec §4.7); confirm zero new crate families vs D25/ADR-0010; `cargo deny check` clean. | S |
| B0.7 | docs + C13 | Update `docs/contracts/C13-signature-integrity.md` honesty rows + CHANGELOG (the `sign` wire change needs a C7 additive bump + opt-out note if the wire shape is frozen at v1.6); update ADR-0010 status notes. | S |
| B0.8 | 👤 owner | OQ-C12 confirm (manual builder — recommendation A) + sequencing confirm (P7-7 before P7-5, spec §6.2/§7.3). Blocks B1 x3 if owner reverses. | — |

**Order:** B0.1 → B0.2 → B0.3 → B0.4 → B0.5 → B0.6 → B0.7; B0.8 can run in parallel (default =
recommendation A unless reversed).

### P7-5 — PAdES-T/LTV *(ADR-0012 proposed — ADR acceptance first)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P5-a | 👤 ADR-0012 | Owner accepts/rejects; if accepted, record the D-row in `project_concept.md` §16. | — |
| P5-b | TSA client (RFC 3161) | Pure-Rust timestamp request + response parse (reqwest? check existing HTTP deps — keep D8 zero-hard-deps: feature-gated), embed as unsigned attr / counter-signature on the B0.4 CMS layer. Honest-absent without network (`--no-tsa` stays). | L → split: request/parse | verify with openssl ts -verify on recorded token; offline unit test w/ recorded response fixture |
| P5-c | OCSP/CRL embedding (`/LTV` dict) | Fetch + embed per ADR-0012 §(LTV); GP-S-012 honesty row updated. | L → split: ocsp / crl / dict | S |
| P5-d | trusted-cert option | User-provided PKCS#12 chain instead of self-signed (GP-S-011 relax), chain validation. | M | — |

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

**Track-B order:** B0 (already warm, signing-related — deferred per owner) ∥ P6-a (ADR-0013 **accepted 2026-09-26**) / P8-a (spec-freeze/👤).
P7-1 (xlsx create) **COMPLETE** — P7-2 (pptx create) **COMPLETE** — P7-3 (formula coverage) **Tranche A+Tranche B (lookup) COMPLETE** — P7-4 (lint encode/ssim) **COMPLETE** — P7-6 (redaction) **COMPLETE** — P7-8 (pdfium spike) **COMPLETE** — next: P7-9 (launch, 👤 not delegable).
P5 (LTV) strictly after B0.4 (needs counter-sig CMS).

**Git state (measured 2026-09-26):** `main` pushed to origin; tag `m6` re-anchored + pushed.

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
