# ADR-0004 — Typst engine = CLI shell-out (ENG-11, D3): corrected from "pure-Rust lib"

| | |
|---|---|
| Status | **accepted** (owner flip 2026-09-16; proposed 2026-09-15 — engine shipped via P3-05/06, Wave A gate green, M3 accepted) |
| Gate | A (chromium+typst engines probed/registered; deny bans typst crate) |
| Decide by | wave A (2026-09-15) |
| Decision owner | **project owner** |
| Blocks | P3-05, P3-06 (engine implementation depends on this decision) |
| Related | D3, deny.toml, P3-05, P3-06, project_plan.md ENG-11 |

<!-- Status flow: proposed → accepted | rejected (owner only). An accepted ADR that changes a
      decision also requires the owner to record a D-row in project_concept.md §16 — the ADR alone
      does not amend the D-log. -->

## 1. Context

`project_plan.md` ENG-11 originally described the Typst integration as a "pure-Rust lib" (i.e.,
linking the `typst` Rust crate into the stichos binary). This conflicts with the core invariant
expressed in D3 and concept §1: **engines are spawned subprocesses, never linked**.

Linking the `typst` crate would:
- Violate the subprocess-only engine architecture (D3, concept §1).
- Introduce a C-linked dependency tree (typst depends on `rustybuzz`, `fontdb`, `zstd`, etc.),
  breaking the cross-compile guarantee (DIS-01).
- Contradict the `deny.toml` policy that bans C-linked renderers.

The invariant in concept §4 states: "engines are spawned subprocesses, never linked; they are
soft dependencies probed at runtime." The Typst integration must conform to this invariant.

## 2. Options

### Option A: CLI shell-out (recommended)
Spawn `typst compile` as an external subprocess, similar to the existing LibreOffice and pandoc
engine designs. The `typst` binary is probed at runtime; if absent, the engine reports
honest-absent.

- **Pros**: Preserves D3 invariant; no new Rust crate dependencies; cross-compile safe;
  consistent with all existing engine designs.
- **Cons**: Requires `typst` to be installed on the host (acceptable as soft dependency).

### Option B: Link the `typst` crate
Add `typst = "*" ` as a workspace dependency and implement the engine as a Rust library call.

- **Pros**: Potentially lower overhead (no subprocess spawn).
- **Cons**: Violates D3 invariant; introduces C-linked dependency tree; breaks cross-compile
  guarantee; contradicts concept §4; requires `deny.toml` allow-list changes.

### Option C: Defer Typst entirely
Remove ENG-11 from the plan; do not implement Typst support.

- **Pros**: No architecture conflict.
- **Cons**: Abandons ENG-11 scope; loses Typst-as-renderer use case; inconsistent with Wave A
  scope (chromium + typst engines).

## 3. Evidence

- **researched**: concept §1 — "engines are spawned subprocesses, never linked; they are soft
  dependencies probed at runtime."
- **researched**: D3 — "engines are soft dependencies, never linked at build time."
- **researched**: `project_plan.md` ENG-11 — originally stated "pure-Rust lib" for Typst.
- **researched**: `deny.toml` — already bans C-linked renderers (`pdfium-render`, `mupdf`, etc.);
  typst crate would introduce transitive C dependencies.
- **measured**: existing engine runners (`libreoffice`, `pandoc`) use `std::process::Command`
  to spawn subprocesses — this is the established pattern.

## 4. Consequences

### Option A (CLI shell-out)
- **Locks in**: Typst = CLI shell-out via `typst compile`; `typst` crate banned in `deny.toml`.
- **Costs**: None beyond what `deny.toml` already prevents; aligns with existing patterns.
- **Makes easier later**: Cross-compile targets (DIS-01); honest-absent probe; engine negotiation.
- **deny.toml**: Add `typst` and `typst*` to `[bans].deny`.
- **D-rows**: No D-row changes needed; confirms existing D3, D8.

### Option B (link typst crate)
- **Locks in**: `typst` crate as dependency; violates D3.
- **Costs**: Requires D3 revision or exception; breaks cross-compile; adds C-linked deps.
- **deny.toml**: Would need to add `typst` to `[licenses]` allow-list and remove from bans —
  but also requires removing from deny which contradicts the spirit.

### Option C (defer)
- **Locks in**: No Typst support in M3.
- **deny.toml**: No changes.

## Recommendation (non-binding — the owner decides)

**Option A**: Typst = CLI shell-out via `typst compile`. This corrects the `project_plan.md`
ENG-11 wording ("pure-Rust lib" → "CLI shell-out") while keeping concept §4 "engines are
spawned subprocesses, never linked" intact. The `typst` Rust crate is banned in `deny.toml`
to enforce the engine-never-linked invariant at build time.

This ADR is a documentation correction only; no behavior change since P3-06 will implement
the CLI shell-out approach regardless.

## Decision (owner, <date>)

<!-- Owner-only section: the decision as taken, and its follow-through — D-log entry (§16),
     deny.toml state, tasks unblocked, status flip in the header table and docs/adr/README.md. -->
