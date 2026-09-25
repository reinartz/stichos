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
| Milestones M0–M6 | all tagged; **`main` == `ff80f8e` pushed; tag `m6` re-anchored to `ff80f8e` and pushed** (old T-D2/T-D3 done) |
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
| R-E3 | 🟡 | git 👤 (force-push **approved** 2026-09-24) | Commit Track A′ files on `main`; **re-anchor tag `m6`** to the committed gate-green tree; `git push origin main` + `git push --force-with-lease origin m6`. Precedent: `docs/m4-acceptance.md` Git-State. |
| R-E4 | 🟡 | git 👤 (hygiene) | Move P7-7 WIP to branch `p7-7-cms`; delete local `tmp-7` branch + stale remote-tracking refs. |

**Track-A′ order:** R-A1 → R-B1 → R-B2 → R-C1 → R-B3 → R-D1/D2/D3 → R-A2 → R-E1 → R-E2 ✅ — **R-E3 👤 → R-E4 👤** (pending owner git operations).

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

### P7-1 — xlsx create *(spec: `docs/track-b-p7-1-xlsx-create.md` — draft, ~3–5 days, 2 M + 6 S)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P1-a | spec freeze | Subagent review pass over the spec's open questions; owner sign-off list; update spec → `accepted`. | S |
| P1-b | `adapters-ooxml` xlsx writer: sheet/cell XML (`sheet.xml`, shared strings vs inline) | Content-driven write from C1 Spreadsheet IR; `create --type xlsx` end-to-end produces a file LibreOffice opens. | M |
| P1-c | styles + number formats | Minimal styles.xml + cell-style mapping; corpus round-trip. | M |
| P1-d | workbook plumbing | `workbook.xml`, rels, content types; multi-sheet order. | S |
| P1-e | formulas + cached values | Write formula cells w/ cached value per C15 semantics. | S |
| P1-f | round-trip corpus + CLI golden suite | create → inspect → diff == empty; `stichos create --help` doc; CHANGELOG/C7 additive row (new `--type` value). | M |

### P7-2 — pptx create *(spec: `docs/track-b-p7-2-pptx-create.md` — ~10–15 days, 3 L + 5 M + 11 S; MVP = content-driven from scratch, groups/template deferred)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P2-a | spec freeze + shared `Table` type decision (spec recommends shared gp-core type) | Owner/spec agent closes the 4 open recommendations. | S |
| P2-b | slide writer core | `slideN.xml` + spTree w/ text elements; `create --type pptx` → opens in LibreOffice Impress. | L → split per element kind |
| P2-c | layout/master plumbing | Minimal slideLayout/slideMaster set; theme1.xml reuse strategy. | M |
| P2-d | text/table writers | `SlideElementKind` enum tagging (spec rec.), table writer via shared type. | M |
| P2-e | image elements | `data: Option<Vec<u8>>` path (spec rec. keep), media parts + rels. | S |
| P2-f | round-trip + suite | create → inspect → diff empty on corpus; C7 additive row. | M |

### P7-3 — formula coverage tranches *(spec: `docs/track-b-p7-3-formula-coverage.md` — ~55 functions in tranches A–D)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P3-a | parity-lint gap measurement | Run parity lint over corpus; produce ranked gap list (frequency × format); pick tranche A set; ADR-0011 amendment draft (👤 accept). | M |
| P3-b | tranche A: parser ops (`^`, `%`, unary/inline-string edge cases) | C15 evaluator parser extension + fuzz. | M |
| P3-c | tranche A: ~15 date/time functions | Implement + parity tests (cached-vs-computed). | M |
| P3-d | tranche A: ~10 text functions | Same pattern. | M |
| P3-e | tranche B: ~6 lookup-ish functions | Same pattern. | M |
| P3-f | tranche C: ~18 statistical | Same pattern, split further if >200 k. | M |
| P3-g | tranche D: remainder + refuse-list hygiene | GP-F0xx codes updated for still-unsupported; C15 version bump (additive). | M |

### P7-4 — lint encode/ssim refresh *(spec: `docs/track-b-p7-4-lint-encode-ssim.md`)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P4-a | deferred-note inventory | Grep `crates/lint` for "deferred"/stub notes; classify vs C9 §8 open questions; update spec → accepted. | S |
| P4-b | `lint::encode` fidelity tests | Real-encoder benchmark per spec (stub-by-design note → measured numbers). | M |
| P4-c | `lint::ssim` golden-raster suite | Wire real corpus golden rasters (spec: none today); ε_parity TBD-1 — needs 👤 owner constant decision. | M |
| P4-d | C9 §8 rows | Update contract open-question statuses with measurements. | S |

### P7-6 — redaction *(ADR-0013 proposed — ADR acceptance first)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P6-a | 👤 ADR-0013 | Owner accept/reject (content-stream surgery vs annotation-only honesty). | — |
| P6-b | content-stream text removal | Token-level removal/replace in `adapters-pdf` (incremental.rs Replace mutation); redaction IR type + CLI `annotate --redact` scope per ADR. | L → split: removal / geometry |
| P6-c | applied-state + signature invalidation | `/Redaction` Applied workflow (C12) + C13 invalidation event; verification test: removed text absent from extraction, prefix invariant holds. | M |

### P7-8 — pdfium spike *(spec: `docs/track-b-p7-8-pdfium-spike.md`; recommendation: adopt dynamic-only behind feature; touches D2/D19 boundary — 👤)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P8-a | 👤 decision | D2 boundary call: dynamic runtime load of `libpdfium` vs staying pure-Rust (spec §5.3 matrix); amend D2/D19 if adopted. | — |
| P8-b | spike behind `pdfium` feature flag | `pdfium-render` dynamic-only; rasterize the 50-page fixture + word-bbox comparison vs current ladder; honest `rendered_by`. | M |
| P8-c | close gate G5 | Adopt (deny.toml + support-matrix rows) or document-reject; either way write the G5 closure note. | S |

### P7-9 — launch *(👤 only — `docs/track-b-p7-9-launch.md`, `docs/release-playbook.md`)*

| Sub | Scope | Deliverable / acceptance | Size |
|---|---|---|---|
| P9-a | 👤 D18/D26 flip | Owner checklist: registry publishes, visibility, v* tags. Not delegable. | — |

**Track-B order:** B0 (already warm) ∥ P1-a/P2-a/P3-a/P4-a/P6-a/P8-a (spec-freeze/👤 starters).
Implementation follows the per-task chains. P5 (LTV) strictly after B0.4 (needs counter-sig CMS).
P7-2 after P7-1 shares the OPC write plumbing.

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
