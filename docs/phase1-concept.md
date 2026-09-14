# stichos — Phase 1 concept: read-only ship (M1)

**Status:** planning artifact · derived from [`project_concept.md`](../project_concept.md) §14 Phase 1 +
[`project_plan.md`](../project_plan.md) §2 M1 · **introduces no new decisions**; anything that would change
a D-row goes to the owner first (CONTRIBUTING: decision-log discipline).
**Companion:** [`phase1-plan.md`](phase1-plan.md) (steps) · [`phase1-status.json`](phase1-status.json) (progress).
**Last updated:** 2026-09-08

---

## 1. What Phase 1 is

Ship a **read-only** stichos: one binary per platform (6 targets) that can **inspect** any supported office
document into structure-with-coordinates, **render** it through a real engine into images a vision LLM can
look at, report **what the read cost** (feature census, unknown parts, provenance), and **doctor** the
machine's real capabilities. Concept §14: *"read-only ships safely and is immediately useful."*

Phase 1 buys the two hardest trust pieces without touching a user's bytes: the IR/reader layer and the
render→lint→payload loop. Edits (Phase 2) inherit both.

## 2. The ship

- `stichos` binary: CLI verbs `inspect` · `render` · `diff` · `doctor` · `serve`. **No `apply`** — edit
  tools are absent from the binary at M1 (gate).
- MCP server (stdio + `serve --http :8731`) exposing the read tool set (§3).
- Packages: .deb/.rpm, tarball+install.sh, macOS `.pkg` + Homebrew cask, Windows MSI + portable ZIP +
  winget manifest. `Recommends:` only, never `Depends:` (D8).

## 3. Product surface (read tool set, C7 v1 subset)

| Tool | Returns | Notes |
|---|---|---|
| `inspect_document` | IR structure + element IDs + coordinates-of-structure manifest + fidelity budget | IDs per P4/C2; stable across sessions |
| `inspect_region` | cropped image from the 300-DPI master + `bbox_pt` ↔ pixel conversion data | ~600-token detail (§10-3, D12) |
| `render_document` | page images (≤20 blocks + contact sheet) + manifest (`rendered_by`, dpi, engine version) | payload rules D13/C8 |
| `diff_documents` | IR structural diff + XML part diff between two stored documents | anchored to element IDs |
| `doctor` | engine × fixture capability matrix + font probe table | capability = real conversion (D8) |

Every response carries: `rendered_by` (or `visual_feedback: "unavailable"`), fidelity budget, and C3
diagnostic codes — INV-2 is asserted by a response-schema test on **every** tool.

## 4. Capability matrix (what "read" means per format)

| Format | Phase 1 read |
|---|---|
| .docx | body/tables, styles/numbering/sections/headers-footers, fields, comments, tracked changes **read-only**, XSD validation (strict/loose) |
| .xlsx | cells, **formulas and cached values**, shared strings, styles, merged ranges; charts/pivots detected → budget entries |
| .pptx | slides, layouts, masters, placeholders, notes |
| .odt/.ods/.odp | package + manifest, styles, cells/formulas+cached, slides/layouts; encryption detection (flag only) |
| PDF | object layer (xref/streams), page tree, text + **word-level bboxes**, AcroForm read (XFA flagged unsupported), annotations, metadata/outline, tagged-structure extraction (a11y feed), `qpdf --check` |
| all | render via engine ladder to 150-DPI masters (300-DPI on demand for crops); layout lint; perceptual SSIM |

## 5. Behavior promises inherited (non-negotiable in Phase 1)

- **INV-2 provenance (D6):** every response names its engine + version, or says `visual_feedback:
  "unavailable"` — an honest, supported state (§9).
- **Fidelity budget on read (P2):** feature census + unknown-part inventory; "what did reading cost" is
  already a real question (extraction ≠ document).
- **Nothing is ever written to the input file** — the read binary opens inputs read-only; the store holds
  copies/journals of *reads*, originals immutable (P6 trivially holds).
- **INV-4 determinism:** render-cache key `sha256(doc)+engine+version+fontset_hash+dpi` against the pinned
  container (M0 goldens are the reference).
- **Payload rules (D13/C8):** ≤20 image blocks, pre-resized to known sizes, estimator
  `⌈w/28⌉×⌈h/28⌉`; crop-on-demand beats full high-res.
- **INV-5 hostile input:** every reader is fuzzed (OPC/XML/PDF ≥24 h per release); macros detected,
  flagged, never enabled (SEC-05).

## 6. Explicitly out of scope (where it lives instead)

| Deferred | Lands in |
|---|---|
| Any writer / patch path: OOX-02/09/10/11, ODF-04, PDF-03 | Phase 2 start (still before PAT-01) |
| Undo journal, patch model, policy engine: CORE-06/07/09, C6, C10 | Phase 2 |
| Edit tools + `dry_run` + tracked-changes write: API-07, PAT-*, OOX-10 | Phase 2 |
| Windows COM worker (ENG-09/COM-1..6), G3 Word tier | mid-Phase 2 |
| Linter-box annotator REN-06, taint/quarantine SEC-03, allow-list SEC-04 | Phase 2 |
| veraPDF / LIN-04/05 schema+a11y gates, OPS-02 perf gate | Phase 2 |
| Creation (AUT-*), Rhai hatch (API-09), Chromium/Typst (ENG-10/11) | Phase 3 |
| Human loop (HUM-*), LOKit plugin (ENG-12) | Phase 4 / D3-trigger |

## 7. M1 exit gate (measurable)

From plan §2 M1, each item bound to a plan step and asserted by `tools/phase1_gate.sh` (step P1-61):

1. Installs clean on all 6 targets (P1-56..59) — CI + acceptance runbook.
2. Every tool response carries provenance (P1-25, P1-52 schema matrix).
3. Read-path fuzz-clean ≥24 h (P1-53).
4. `doctor` matrix = real fixture conversions (P1-27, P1-31).
5. Edit tools **absent** from CLI + MCP surface (P1-47, asserted in P1-61).
6. Global DoD per task (plan §8) + all six wave gates in [`phase1-plan.md`](phase1-plan.md) verified.

**Precondition note:** M0's last blocker (GOV-02 registration sweep) is owner-bound and does **not** block
Phase 1 development on the private repo (D18); it **does** block any public announcement of an M1 build.

## 8. Phase-1-specific risks

| Risk | Mitigation |
|---|---|
| Pure-Rust word-bbox quality (PDF-04) below poppler → LIN-01 starved | P1-21 delivers the G5 deciding experiment (ADR-0001 follow-up) with parity data **before** linter work depends on it |
| ECMA-376 read breadth underestimated | corpus-driven manifest assertions per step; census turns gaps into budget, not bugs (P1-15) |
| Element-ID scheme (P4) needs rework before M2 freeze | P1-04 ADR-first + property tests; M1 only requires read-stability, edit-stability tests gate C2 freeze in Phase 2 |
| Distro LO variability pollutes render expectations | probes (D8) + pinned-container goldens as the only pass/fail reference (INV-4) |
| Agent-serial wall-clock vs plan §6 parallel estimate | plan §5 sums ≈ 115.5 p-w of task estimates; agent execution is serial — treat waves + gates, not calendar, as progress |

## 9. Decisions/contracts this phase stands on (no changes)

D2 (Rust, 6 targets), D3 (subprocess engines), D4 (no shipped runtime), D5/D6 (surgical + provenance),
D8 (zero hard deps, capability=conversion), D11 (raster ladder), D12/D13 (crop-on-demand, ≤20 blocks),
D14 (fonts first-class), D15 (pinned engines N/N-1), D18 (private repo), D19/ADR-0001 (lopdf+pdf-writer,
qpdf shell-out), D20/D21. Contracts: C1, C3, C4(read shape), C5, C7(v1 subset), C8, C9(CI parity), C2-draft.

## 10. Sequencing deviations from `project_plan.md` §6.3 (flagged for owner)

Dependency-driven re-sequencing for single-agent serial execution; **no decision or contract change**:

1. Writers/journal/patch (OOX-02/09, ODF-04, PDF-03, CORE-06/07/09) moved out of W1 — M1 ships read-only;
   they re-enter at Phase 2 start ahead of their dependents (PAT-01).
2. CORE-04 (element IDs) moved **forward** into Wave A — `inspect_document`/`diff_documents` outputs
   require IDs at read time (P4).
3. REN-05 (cropper) + REN-03 (contact sheet) pulled into Phase 1 because API-05 already promises
   `inspect_region`/`render_document` at M1.
4. OPS-02 (2 s edit-loop perf gate) deferred to Phase 2 — the loop doesn't exist yet; Wave C instead
   **measures and records** read-loop latency as data.
