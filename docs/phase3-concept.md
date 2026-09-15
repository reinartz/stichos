# stichos — Phase 3 concept: creation + write-path completion (M3)

**Status:** planning artifact · derived from [`project_concept.md`](../project_concept.md) §14 Phase 3 +
[`project_plan.md`](../project_plan.md) §2 M3 + [`m2-acceptance.md`](m2-acceptance.md) §6 (Phase-2b intake) +
[`phase2-concept.md`](phase2-concept.md) §6 (items deferred to Phase 3) · introduces **no new D-rows**;
four owner scope calls (§10, 2026-09-15) are plan-level; the G4 call also resolves OQ4 and is written back
to `project_concept.md` §18.
**Companion:** [`phase3-plan.md`](phase3-plan.md) (steps) · [`phase3-status.json`](phase3-status.json) (progress).
**Last updated:** 2026-09-15

---

## 1. What Phase 3 is

Two halves that share one gate:

1. **Creation.** A doc is produced from `markdown` / `html` / `typst` / raw **IR**, via four paths —
   pandoc `--reference-doc`, Chromium print-to-pdf, Typst compile, and our **own native OOXML writer**
   (IR → docx/xlsx/pptx, template-preserving branding). Every creation output goes through the same
   QA loop the editor uses: render → lint → schema gate → fidelity budget → image payload.
2. **Write-path completion (Phase-2b intake, Linux-verifiable half).** ODF gets the surgical write
   path (ODF-01 writer / ODF-04 patch / ODF-05 census+RelaxNG) and PDF gets the incremental
   append-only writer (PDF-03). The M2 refusals for `.odt/.ods/.odp` and PDF are retired — honestly,
   with INV-1 suites behind them. The Windows COM worker stays out (no Windows host to verify on).

Concept §14: *"Markdown/HTML/Typst → pandoc --reference-doc / Chromium / Typst for branded output;
tracked-changes and comment output."*

### Starting state — measured 2026-09-15 (not assumed)

`phase2-status.json` claims 35/35 + 7/7 gates, but re-running `bash tools/phase2_gate.sh` on the
current tree today: **exit 1**. Functional items all pass (1 280 tests; zero-patch ×151; revert;
refusals; fuzz 80; budget; schema gate; 9 revision read-back tests), but: **clippy fails** — 5 errors
in `cli` (unused `super::*`, 2× `map_or` simplifiable, 2× if-then-else-bool at
`crates/cli/src/diff_tool.rs:259,268`); **fmt fails** — 10 diffs across 6 files
(`apply_tool.rs`, `diff_tool.rs`, `main.rs`, `engines/libreoffice.rs`, `engines/raster.rs`,
`gp-core/executor.rs`). **Waves C–F (P2-18…35) were never committed** (HEAD = `3f1cb2a` "Wave B") and
the **`m2` tag points at that Wave-B commit** — it certifies a tree without the M2 surface. Branch is
3 commits ahead of `origin/main` unpushed.

Environment (measured 2026-09-15): pandoc **3.7.0.2** ✅, `chromium-browser` ✅, `xmllint` ✅,
soffice **26.2.6.1** ✅, `pdftoppm` ✅; `typst` ✗, `qpdf` ✗, `verapdf` ✗ (CI installs the last three;
probes must stay honest when absent). Corpus: 151 fixtures — docx 35 / xlsx 21 / pptx 26 /
**odt 26 / ods 21 / odp 22 (69 ODF)**; `pr-subset.txt` has 27 entries, all OOXML.

**Phase 3 therefore opens with Wave 0 (precedent S1): close the Phase-2 debt; nothing is built on a
red gate.**

## 2. The ship

- **Create verbs enter the ship**: CLI `create` + MCP `create_document` (source `ir|markdown|html|typst`,
  format `docx|xlsx|pptx|odt|pdf`, `template`, `dry_run`), and the **`script`** tool (sandboxed Rhai, D17).
  C7 **additive v1.2** (15 MCP tools, 16 CLI verbs incl. `update`).
- **Native creation** (OOX-11): docx/xlsx/pptx written from IR by our own writer, byte-valid against
  the schema gate, openable by LO, IR-round-trip equal; **templates preserve branding byte-for-byte**
  (style/theme/master parts copied untouched from the reference package).
- **ODF + PDF writable**: the write story is now all three families on Linux; refusals remain only for
  encrypted/signed inputs (explicit C3 codes, never silent).
- **Create-as-revisions** (AUT-05): `create` against an existing target lands changes as
  `w:ins`/`w:del` (D7 suggestion mode).
- **Distribution automation**: G4 decided (§10 S4) — GitHub Releases + winget/Homebrew, no own repos;
  release CI ×6 targets, `selfupdate` with verify+rollback, OCI profile image.

## 3. Product surface (new, C7 v1.2 additive)

| Tool / verb | Returns | Notes |
|---|---|---|
| `create_document` (MCP) / `stichos create` | new doc id + bytes in store + fidelity budget + rendered pages + lint manifest + `rendered_by` | paths: pandoc / chromium / typst / native; `dry_run` returns preview render + IR, writes nothing (P3) |
| `script` (MCP) / `stichos script` | script result + journal ids of any patches it applied | Rhai, host fns scoped to store/IR/patch only — **no fs/net/env**; ops/depth/time limits (D17) |
| `stichos update` | verify → swap → rollback | GitHub Releases channel (G4), minisign + sha256 verify, downloader = shell-out curl/wget (no new TLS stack) |
| executor (internal) | ODF + PDF no longer refuse | refusal matrix shrinks to: encrypted, signed-PDF, macro-write — all explicit C3 codes |

Creation responses carry everything edit responses do: provenance (INV-2), budget (D6), QA loop, §10
payload rules.

## 4. Capability matrix (M3)

| Path | Input → output | Engine | Non-surface |
|---|---|---|---|
| pandoc | md/html → **docx, odt** | pandoc `--reference-doc` (soft) | tracked-changes out of pandoc (AUT-05 adds them via own writer) |
| Chromium | html → **pdf** | `--headless=new --print-to-pdf` (soft) | JS-heavy dynamic pages (documented caveat; screenshot fallback) |
| Typst | typ → **pdf** | `typst compile` CLI shell-out (S3; **not linked**) | typst package downloads (offline templates only) |
| native | IR → **docx, xlsx, pptx** | own writer (zero external deps) | charts/pivots/SmartArt creation (IR doesn't model them; stays budget-flagged on read) |
| template | any of the above + `template=` | reference docx base; content parts replaced | template *editing* (create only) |
| ODF edit | IR patch → odt/ods/odp | own surgical writer | **ODF tracked-change write** (patch-only; revisions stay read) |
| PDF edit | IR patch → annotations, form values, metadata | lopdf + pdf-writer (D19) | content-stream surgery; **signed PDFs → refuse** |

## 5. Behavior promises (each test-bound)

- **INV-1 extended to ODF + PDF**: ODF zero-patch round-trip **byte-identical ×69**; unrelated-edit
  hash tests; PDF results satisfy the **append-only prefix invariant** (original bytes are a strict
  prefix of the output; the increment *is* the byte-diff report).
- **Creation QA gate**: every created doc passes lint + schema gate (P2-31 infra) and **IR round-trip
  equality** (create → Phase-1 reader re-read → equal IR, C1).
- **Create-as-revisions**: md→existing-docx produces `w:ins`/`w:del` read back by our reader and
  rendered by LO (reuse the P2-17 proof machinery).
- **Sandbox**: escape-attempt suite green (fs read, net, env, loop bomb, deep recursion → C3
  diagnostics, zero side effects).
- **Provenance & determinism for new engines**: `rendered_by` names chromium/typst/pandoc + version;
  render cache keys include engine+version (INV-4) — chromium/LO version drift changes goldens, and
  the golden store accepts per-version entries.
- **Honest absence**: typst/qpdf/verapdf/chromium absent ⇒ probe reports `unavailable` with install
  hint; no test passes by silently skipping where the tool is pinned present (CI installs them).
- Inherited unchanged: INV-2 (budget on 100 % of creations and edits), INV-3 (journal/revert now also
  for ODF+PDF edits), INV-5 (new write surfaces get fuzz targets before ship).

## 6. Explicitly out of scope (where it lands)

| Deferred | Lands |
|---|---|
| Windows COM worker ENG-09/COM-1..6, G3 Word tier | Phase 3b / whenever a Windows host exists (unchanged from 2b) |
| Tracked-change accept/reject UI (HUM-01), add-ins | Phase 4 |
| ODF revision **write** (text:insert/mark-deleted) | open — not M3; ODF edits are direct + journaled |
| Formula evaluation (xlsx) | open question (unchanged); `stale_cached_values[]` still the answer |
| Content-stream PDF surgery, PDF signing production (PAdES) | post-M3 (M3 only *refuses* signed inputs and warns per PAT-05) |
| Charts/pivots/SmartArt creation | open — IR models them as opaque; creation is text/table/style scope |
| G5 pdfium | stays gated |

## 7. M3 exit gate (measurable; asserted by `tools/phase3_gate.sh`, step P3-32)

1. **Creation matrix measured**: 4 paths × corpus-derived fixture set → created docs pass lint +
   schema gate + IR round-trip; measurement table published (page-count delta, SSIM vs LO golden,
   latency, drop counts) — concept §14 "measured against corpus".
2. **INV-1 across three families**: OOXML suites still green; ODF zero-patch ×69 + unrelated-edit;
   PDF append-only prefix invariant on the measured set.
3. **Create-as-revisions read-back** on docx (D7 suggestion mode).
4. **Sandbox proof**: Rhai escape suite green; `script` tool responses journaled like any edit.
5. **Write-path fuzz round 2** (ODF writer, PDF increment, creation IR, Rhai corpus) ≥24 h clean +
   SEC-06 Phase-3 review (new attack surface: pandoc args, chromium flags, typst input, sandbox).
6. **Distribution**: release CI builds ×6 targets from one version bump; `selfupdate` verify+rollback
   proven against a fixture channel; OCI profile image doctor golden; **OQ4/G4 recorded** (S4).
7. **C7 v1.2 frozen** (additive-only check across the full 15-tool schema matrix).
8. Wave 0 done: `phase2_gate.sh` green on the **committed, pushed** tree; `m2` tag fixed to point at
   real M2 code.

## 8. Phase-3-specific risks

| Risk | Mitigation |
|---|---|
| Own OOXML writer emits files LO opens but Word rejects | schema gate (xmllint/structural) + LO render + round-trip is necessary-not-sufficient ⇒ golden **external-open** checks via LO as proxy, plus hand-verify 10 outputs in a real Word/LibreOffice interop pass; keep created-part set minimal and spec-literal |
| Chromium version drift changes print-to-pdf | engine+version in cache key (INV-4); goldens per version; pin CI chromium |
| PDF xref-stream inputs make append-only awkward | allowed by spec (classic xref section after xref stream = hybrid), but risky ⇒ **M3 refuses xref-stream+object-stream-complex inputs with explicit code**, supports classic-xref PDFs; broaden post-M3 |
| pandoc drops brand formatting silently | budget: census of created doc vs template expectations; `--reference-doc` validated by round-trip IR |
| Rhai sandbox escape | deny-by-default host API (no `register_package`, no fs/net/env fns), `set_max_operations`/depth/size/time limits, fuzz corpus of scripts in Wave G |
| ODF zip quirks (mimetype first+stored, manifest hashes) break INV-1 | dedicated synthetic per-quirk fixtures (phase-2 zip-class pattern) + ×69 zero-patch gate |
| ODF creation via pandoc differs from ODF-04 expectations | creation via pandoc is *engine output* (budgeted); only IR-native creation goes through our writer, and it is OOXML-only in M3 |
| New external crates (Rhai, minisign-verify) grow the tree | both pure-Rust, D2-safe; ADRs required (ADR-0005/0006); `cargo deny` advisory gate unchanged |
| selfupdate becomes a supply-chain hole | signature verify **before** swap, backup+rollback, opt-in invocation, no ambient network in every command |

## 9. Decisions/contracts this phase stands on

**Unchanged:** D2–D17, D18–D21. **New ADRs:** ADR-0004 (Typst = CLI shell-out; engine-never-linked
invariant preserved; `typst` crate banned in `deny.toml`), ADR-0005 (Rhai dependency, sandbox
contract), ADR-0006 (update channel = GitHub Releases; shell-out downloader; verify-before-swap).
**Contracts:** C1 **minor additive** (template metadata on IR); C4 extended (creation budget +
template-drop); C5 extended (engine ids `chromium`, `typst`); C9 extended (creation-path metrics);
C7 **additive v1.2 freeze**. C2/C6/C10 consumed frozen. No new D-rows; OQ4 resolved by S4 (§18).

## 10. Owner scope calls 2026-09-15 (this session) + deviations from `project_plan.md` (flagged)

- **S1** — Phase-2 debt (clippy/fmt fix, commit Waves C–F, re-tag `m2`, push) = **Wave 0 of Phase 3**
  (M1-closeout precedent).
- **S2** — **Phase 3 = creation + Linux-verifiable Phase-2b** (ODF-04/05, PDF-03 join W4 contents);
  COM worker remains deferred. Ratified here; `project_plan.md` §6.3 W4 row updated with pointer.
- **S3** — **Typst = CLI shell-out**, correcting `project_plan.md` ENG-11's "pure-Rust lib" wording;
  keeps concept §4 "engines are spawned subprocesses, never linked" intact. ADR-0004.
- **S4** — **G4/OQ4 decided: GitHub Releases + winget/Homebrew only; no own apt/rpm repos.**
  DIS-05/06/08 in Phase 3 scope. Written back to `project_concept.md` §18-4 as resolved.
