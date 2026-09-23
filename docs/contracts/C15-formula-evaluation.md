# C15 — Formula Evaluation Contract

| | |
|---|---|
| Owner | adapters-ooxml |
| Consumers | cli, lint, gp-core (budget) |
| Version | 1.0.0 |
| Status | frozen — M6 (C7 v1.6) |
| Artifacts | `adapters-ooxml/src/formula/` module |
| Conformance | parity lint test + refuse-list enumeration test |

## 1. Scope

This contract defines the supported formula function set for xlsx evaluation, cached-vs-computed
parity rules, and GP-F0xx refusal semantics. It applies to all xlsx reading and formula evaluation
paths in the stichos toolchain.

## 2. Supported Function Set (v1.0.0)

### 2.1 Arithmetic operators

| Operator | Description |
|----------|-------------|
| `+` | Addition |
| `-` | Subtraction |
| `*` | Multiplication |
| `/` | Division |
| `-` (unary) | Negation |
| `()` | Parentheses for grouping |

### 2.2 Comparison operators

| Operator | Description |
|----------|-------------|
| `=` | Equal |
| `<` | Less than |
| `>` | Greater than |
| `<=` | Less than or equal |
| `>=` | Greater than or equal |
| `<>` | Not equal |

### 2.3 Logic functions

| Function | Description |
|----------|-------------|
| `AND(...)` | Logical AND of all arguments |
| `OR(...)` | Logical OR of all arguments |
| `NOT(...)` | Logical NOT |
| `IF(logic, true_val, false_val)` | Conditional expression |

### 2.4 Aggregate functions

| Function | Description |
|----------|-------------|
| `SUM(...)` | Sum of all arguments |
| `AVERAGE(...)` / `AVG(...)` | Arithmetic mean |
| `COUNT(...)` | Count of numeric arguments |
| `MIN(...)` | Minimum of all arguments |
| `MAX(...)` | Maximum of all arguments |

### 2.5 Text functions

| Function / Operator | Description |
|---------------------|-------------|
| `CONCATENATE(...)` | Concatenate all string arguments |
| `&` | String concatenation operator |

## 3. Cached-vs-Computed Parity Rules

1. If a cell has a cached `<v>` value AND the formula is in the supported subset, the evaluator
   **MUST** compute the result and compare it to the cached value.
2. If computed != cached → emit `LIN-B-010` (budget: stale cache) diagnostic.
3. If the formula is **NOT** in the supported subset → emit fidelity budget entry + surface
   cached value (never invent a value).
4. If the formula matches the refuse list → emit the specific GP-F0xx diagnostic + surface
   cached value.
5. If no cached value exists → compute if possible, otherwise emit `OOX-W-160`
   (no cached value, formula unsupported).

## 4. Refusal Semantics

### 4.1 Unsupported functions

Unsupported (non-refuse-list) functions produce:
- A fidelity budget entry with the function name
- The cached `<v>` value, if present
- **Never** a computed wrong value

### 4.2 Refuse-list items

GP-F0xx refuse-list items produce:
- The exact GP-F0xx code diagnostic (from `gp-core` registry)
- The cached `<v>` value, if present
- **Never** evaluation or side effects

### 4.3 Parity lint

Parity lint runs on **EVERY** evaluated cell — no exceptions. If a cell is skipped,
the evaluator must emit a diagnostic explaining why.

## 5. Versioning

- New functions are **additive only**. Adding a supported function does not require a C15
  version bump.
- Changing existing supported semantics (removing functions, changing operator behavior)
  requires a C15 version bump.
- The evaluator module exposes:
  ```rust
  pub fn supported_functions() -> &'static [&'static str]
  ```
  This must return the complete list of supported function names for testability.

## 6. GP-F0xx Refuse List

| Code | Description |
|------|-------------|
| `GP-F-001` | External references (`[workbook]sheet!cell` patterns) |
| `GP-F-002` | Array/matrix functions (`MMULT`, `TRANSPOSE` as array) |
| `GP-F-003` | Macro/VBA references |
| `GP-F-004` | External workbook references (DDE/OLE) |
| `GP-F-005` | Date/Time functions outside basic arithmetic |
| `GP-F-006` | Statistical functions outside the supported subset |
| `GP-F-007` | Financial functions |
| `GP-F-008` | Text functions outside CONCATENATE |
| `GP-F-009` | Logical functions outside AND/OR/NOT/IF |
| `GP-F-010` | Deeply nested formulas (>32 levels, Excel limit) |
| `GP-F-011` | Invalid formula syntax (parsec error — not our error to report, just use cached) |
| `GP-F-020` | Unsupported function name encountered |

## 7. Conformance Tests

1. **Enumerate all GP-F0xx codes** — they must be machine-readable (testable via
   `supported_functions()` + refuse list).
2. **Parity lint catches injected stale-cache fixture** — inject a known-wrong cached value,
   verify `LIN-B-010` is emitted.
3. **Hostile formulas refuse with exact GP-F0xx** — deep nesting, external refs, macro calls
   each produce the correct refusal code.

## 8. Revision History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0-draft | 2026-09-21 | Initial draft (P6-07) |
| 1.0.0 | 2026-09-23 | Frozen at M6 (P6-34, ADR-0011 accepted) |
