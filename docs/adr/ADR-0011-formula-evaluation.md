# ADR-0011 — formula evaluation (C15 + GP-F0xx): own evaluator for xlsx subset

| | |
|---|---|
| Status | **accepted** (2026-09-23, project owner) |
| Gate | F (formula eval: ADR-0011 accepted, C15 frozen, minimal subset with parity lint, GP-F0xx refusals exact) |
| Decided | Phase 6, Wave I / 2026-09-23 |
| Decision owner | **project owner** |
| Blocks | P6-07, P6-08 (minimal evaluator impl), P6-19 (M6 acceptance) |
| Related | D2 (zero-external-deps), D25 (pure-Rust constraint), C14 (context budget), C15 (formula evaluation), GP-F0xx refusal codes |

## 1. Context

xlsx files contain `<f>` elements (formulas) and `<v>` elements (cached pre-computed values).
Currently the xlsx reader (`adapters-ooxml/src/xlsx.rs`) stores formulas as raw strings and uses
cached values as display values — no evaluation occurs. The IR (`gp-core/src/ir.rs`) has
`SheetCell.formula: Option<String>` and `SheetCell.cached_value: Option<String>`.

For fidelity, stichos must evaluate a supported subset and flag unsupported formulas via the
fidelity budget. The zero-external-deps constraint (D2/D25) applies: no external crates for the
evaluator.

## 2. Options

### Option A: Own evaluator (recommended)
Build a minimal pure-Rust evaluator supporting the subset listed below. No external dependencies.
Integrate cached-vs-computed parity lint. Surface unsupported formulas via GP-F0xx refusal codes.

### Option B: Delegate to LibreOffice / external engine
Run the file through LibreOffice or pandoc and read back computed values. Adds a hard dependency
on an external installation, violates D25's spirit, and adds unbounded latency.

### Option C: Do nothing
Continue storing formulas as raw strings with cached values only. Loses fidelity on cells where
the cached value is stale or absent.

## 3. Evidence

- **Constraint D2:** Zero external dependencies for core crates. Every `Cargo.toml` in `crates/`
  must have `dependencies = {}` (no third-party crates).
- **Constraint D25:** Pure-Rust for all adapters. External engines are subprocesses, never linked.
- **Existing IR shape:** `SheetCell` already has `formula: Option<String>` and
  `cached_value: Option<String>` — the evaluator fits without IR changes.
- **Measured cost:** A minimal recursive-descent parser + evaluator for the subset is ~200-400
  LOC. No new crates, no new dependencies.
- **Refusal list completeness:** 11 GP-F0xx codes cover all xlsx formula failure modes the project
  cares about.
- **Measured cost (v1.0.0):** 12 functions in ~500 LOC (evaluator.rs + tests). Phase 7 expansion to
  ~55 functions estimated at ~2,500 LOC across 4 tranches (see P7-3).

## 4. Consequences

### Option A: Own evaluator
- **+~200-400 LOC** in new `adapters-ooxml/src/formula/` module
- **No new dependencies** (pure Rust — D2/D25 satisfied)
- **Fidelity budget entries** for unsupported formulas (by design, not a bug)
- **Parity lint** catches stale cached values (measurable benefit)
- **Versioned:** Formula functions use `C15 v1.0.0` — new functions are additive only
- **Cached-values-always semantics:** If a cell has a cached `<v>` value, use it; if the formula
  is in the supported subset, also compute and run parity check
- **Unsupported functions:** Surface cached `<v>` value + emit fidelity budget entry; never invent
  a value
- **Phased expansion:** Tranche A (P7-3A) adds ~28 functions covering the highest-frequency gaps
  (math, text, date/time, statistical basics). Subsequent tranches add lookup, financial, and
  advanced functions. Each tranche is additive-only (C15 versioning).

### Option B: External engine
- Blocks on LibreOffice / soffice availability
- Unbounded latency (spawn + convert + parse)
- Violates D25 spirit (external linked dependency via subprocess orchestration)
- No parity guarantee (engine may compute differently)

### Option C: Do nothing
- Loses fidelity on stale-cache cases
- No mechanism to flag unsupported formulas
- Future parity lint (P6-08) has nothing to compare against

## Recommendation

**Option A** — build the own evaluator. It satisfies D2/D25, delivers measurable fidelity
improvements (parity lint), and provides a clean refusal surface (GP-F0xx) for unsupported
formulas. The ~200-400 LOC cost is bounded and contained within `adapters-ooxml`.

## Decision (owner, 2026-09-23)

**Accepted: Option A** — build the own evaluator. C15 frozen at v1.0.0.
ADR-0011 status set to accepted. The formula module in `gp-core/src/formula/`
and `adapters-ooxml/src/formula/` implements the supported subset with parity
lint and GP-F0xx refusals. See `docs/contracts/C15-formula-evaluation.md`.
- **Future scope:** The 12-function v1.0.0 subset is the initial delivery. Full formula coverage
  (~55 functions across 4 tranches) is tracked as P7-3 in the Phase-7 backlog. Each tranche
  requires a C15 version bump only if semantics change; new function additions are additive-only
  per C15 §5.
