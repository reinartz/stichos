# P7-3 — Full Formula Coverage (Phase 7 Backlog)

| | |
|---|---|
| Parent task | Track B, P7-3 |
| Contract | C15 (frozen v1.0.0 at M6) |
| ADR | ADR-0011 (accepted 2026-09-23) |
| Size | M per tranche |
| Gate | C15 amendment (additive-only) |

## 1. Current formula coverage

### 1.1 Supported function set (C15 v1.0.0)

12 functions across 3 categories, implemented in `gp-core/src/formula/evaluator.rs`:

| # | Function | Category | File | Lines |
|---|----------|----------|------|-------|
| 1 | `SUM(...)` | Aggregate | evaluator.rs | 211–230 |
| 2 | `AVERAGE(...)` | Aggregate | evaluator.rs | 232–255 |
| 3 | `AVG(...)` | Aggregate (alias) | evaluator.rs | 186 |
| 4 | `COUNT(...)` | Aggregate | evaluator.rs | 257–268 |
| 5 | `MIN(...)` | Aggregate | evaluator.rs | 270–294 |
| 6 | `MAX(...)` | Aggregate | evaluator.rs | 296–320 |
| 7 | `AND(...)` | Logic | evaluator.rs | 322–334 |
| 8 | `OR(...)` | Logic | evaluator.rs | 336–348 |
| 9 | `NOT(...)` | Logic | evaluator.rs | 350–355 |
| 10 | `IF(cond, true, false)` | Logic/Conditional | evaluator.rs | 357–369 |
| 11 | `CONCATENATE(...)` | Text | evaluator.rs | 371–378 |
| 12 | `IFERROR(val, fallback)` | Error Handling | evaluator.rs | 380–388 |

### 1.2 Operators (fully supported)

`+`, `-`, `*`, `/`, `-` (unary), `&` (concat), `=`, `<`, `>`, `<=`, `>=`, `<>`, `()` grouping.

### 1.3 Corpus coverage (formulas actually encountered)

| Fixture | Formulas found | Implemented? |
|----------|---------------|-------------|
| `formulas-cached-xlsx-001` | `SUM(D1:D5)`, `AVERAGE(D1:D5)`, `MIN(D1:D5)`, `MAX(D1:D5)` | 4/4 |
| `formulas-cached-xlsx-002` | `Data!A1*3`, `Data!A2*3` (arithmetic + cross-sheet refs) | 2/2 |
| `formulas-cached-xlsx-003` | `IF(A1>40,"big","small")`, `A2&"s"`, `A1/100` | 3/3 |
| `conditional-formatting-xlsx-002` | `COUNTIF($A$1:$A$10,A1)>1` (CF formula rule) | **0/1** |
| `long-tables-xlsx-002` | `=B{r}*2` on 199 rows (arithmetic + cell ref) | 1/1 |

**Key finding:** The corpus currently exercises **only the 4 implemented aggregate + logic functions** via the 5 formula fixtures (12 formula cells total). The `COUNTIF` in conditional-formatting-xlsx-002 is the **only corpus formula not implemented** — and it is in a conditional formatting `<cfRule>` context, not a cell formula, so it would hit the refuse-list `GP-F-020` path.

---

## 2. Gap analysis

### 2.1 Missing categories (per GP-F0xx refusal codes)

The C15 contract (`docs/contracts/C15-formula-evaluation.md` §6) defines GP-F0xx codes for refused categories:

| Refusal code | Category | Example functions | Est. Excel usage share |
|-------------|----------|-------------------|----------------------|
| `GP-F-005` | Date/Time | `TODAY`, `NOW`, `DATEDIF`, `EOMONTH`, `WORKDAY`, `NETWORKDAYS`, `YEAR`, `MONTH`, `DAY`, `WEEKDAY`, `DATEVALUE` | ~15–20% |
| `GP-F-006` | Statistical (extended) | `STDEV`, `STDEVP`, `VAR`, `VARP`, `CORREL`, `PERCENTILE`, `RANK`, `FORECAST`, `SLOPE`, `INTERCEPT`, `COUNTA`, `COUNTBLANK` | ~10–15% |
| `GP-F-007` | Financial | `NPV`, `IRR`, `PMT`, `PV`, `FV`, `RATE`, `NPER`, `IPMT`, `PPMT`, `XNPV`, `XIRR` | ~3–5% |
| `GP-F-008` | Text (extended) | `LEN`, `TRIM`, `MID`, `LEFT`, `RIGHT`, `SUBSTITUTE`, `SEARCH`, `FIND`, `TEXT`, `REPT`, `CONCAT`, `TEXTJOIN`, `DISTINCT` | ~10–15% |
| `GP-F-009` | Logical (extended) | `XOR`, `IFS`, `SWITCH`, `IFNA`, `IFNOT` | ~3–5% |

### 2.2 Missing operators

| Operator | Description | Impact |
|----------|-------------|--------|
| `^` (exponentiation) | Power operator | `POWER()`, `A1^2` common in finance/science |
| `%` (percent) | Percent operator (unary, right-associative) | `A1*10%` is common shorthand |
| `$` (absolute refs) | `$A$1`, `$A1`, `A$1` | Critical for formula correctness; parser treats `$` as unrecognized char |
| Named ranges | `SumRevenue` defined names | Requires lookup via `FormulaContext` |

### 2.3 Missing parser features

| Feature | Status | Impact |
|---------|--------|--------|
| `$` absolute/relative cell references | **Not parsed** — `$` is unrecognized; `=A1*3` works but `$A1*3` fails with "unexpected character" | High — breaks corpus formula rules that use `$` (e.g. `COUNTIF($A$1:$A$10,A1)>1`) |
| Named range resolution | Not implemented | Medium |
| 3D references | `Sheet1:Sheet3!A1` | Low — rarely needed |
| Structured references | Table-qualified `[Column1]` | Low — OOXML tables use structured ref format |
| Array literals | `{1,2,3}` | Low — array formulas are GP-F-002 refused |

### 2.4 Frequency ranking by corpus + industry data

The corpus is too small for statistical ranking, but industry surveys and LibreOffice usage data suggest this priority:

| Rank | Function | Category | Corpus hits | Est. industry freq | Priority |
|------|----------|----------|-------------|-------------------|----------|
| 1 | `LEN` | Text (GP-F-008) | 0 | Very high | **P7-3A** |
| 2 | `TRIM` | Text (GP-F-008) | 0 | Very high | **P7-3A** |
| 3 | `CONCAT` / `TEXTJOIN` | Text (GP-F-008) | 0 | High | **P7-3A** |
| 4 | `TODAY` / `NOW` | Date/Time (GP-F-005) | 0 | High | **P7-3A** |
| 5 | `DATEDIF` / `EOMONTH` | Date/Time (GP-F-005) | 0 | Medium | **P7-3A** |
| 6 | `COUNTA` | Statistical (GP-F-006) | 0 | High | **P7-3A** |
| 7 | `IFNA` | Logical (GP-F-009) | 0 | Medium | **P7-3A** |
| 8 | `VLOOKUP` / `HLOOKUP` / `XLOOKUP` | Lookup | 0 | Very high (but complex) | **P7-3B** |
| 9 | `INDEX` / `MATCH` | Lookup | 0 | Very high (but complex) | **P7-3B** |
| 10 | `ABS` / `ROUND` / `ROUNDUP` / `ROUNDDOWN` | Math | 0 | Very high | **P7-3A** |
| 11 | `MOD` / `INT` / `ROUND` | Math | 0 | High | **P7-3A** |
| 12 | `STDEV` / `VAR` | Statistical (GP-F-006) | 0 | Medium | **P7-3B** |
| 13 | `NPV` / `IRR` / `PMT` | Financial (GP-F-007) | 0 | Low | **P7-3C** |
| 14 | `$` ref syntax | Parser | N/A (breaks existing corpus) | Universal | **P7-3A (parser fix)** |
| 15 | `^` exponentiation | Parser | N/A | High | **P7-3A (parser fix)** |
| 16 | `%` percent | Parser | N/A | Medium | **P7-3A (parser fix)** |

---

## 3. Parity lint strategy

### 3.1 Current parity lint

`gp-core/src/formula/parity.rs` implements **cached-vs-computed parity**:
- `check_parity(cached, computed)` — compares cached `<v>` against computed result
- `parity_refused(code)` — returns `ParityResult::Refused(GpFCode)` for refused formulas
- Canonicalizes numbers (strip trailing zeros), booleans, and text for comparison

This catches **stale cache** (`LIN-B-010`) and correctly surfaces refused formulas.

### 3.2 Gap: No function-level parity lint

The current parity lint does **not** answer:
- What functions does the corpus use that we don't implement?
- Are our implementations producing correct results vs. LibreOffice/Excel?
- Which functions are most common in the corpus?

### 3.3 Proposed parity-lint gap analysis

#### Step 1: Formula inventory pass over corpus

Run a pass over all corpus fixtures (both xlsx and ods) that:
1. Extracts every `<f>` element (xlsx) and `table:formula` (ods) attribute
2. Tokenizes each formula string and extracts function names
3. Categorizes each function: `supported`, `refused`, or `parse_error`
4. Produces a frequency table: `{function_name: count, category: frequency}`

This is a **read-only analysis pass** — no evaluation needed, just parsing + counting.

**Implementation:** New function `inventory_formulas(corpus_dir: &Path) -> FormulaInventory` in `gp-core/src/formula/` or `crates/lint/src/formula_parity.rs`.

#### Step 2: LibreOffice reference evaluation

For each unique formula pattern found:
1. Build a minimal fixture with known input data
2. Open in LibreOffice → export as xlsx
3. Read back the computed `<v>` values
4. Compare against stichos evaluator output
5. Record: `{formula, expected, computed, match: bool}`

This is an **external engine step** (allowed by D25 — engines as subprocesses).

#### Step 3: Gap report generation

Output a JSON report:
```json
{
  "corpus_stats": {
    "total_formula_cells": 247,
    "unique_formulas": 18,
    "implemented_pct": 94.7,
    "refused_pct": 5.3
  },
  "formula_frequency": [
    {"function": "SUM", "count": 89, "category": "aggregate"},
    {"function": "IF", "count": 34, "category": "logic"},
    {"function": "COUNTIF", "count": 2, "category": "statistical", "status": "refused"},
    ...
  ],
  "parity_results": [
    {"formula": "SUM(A1:A5)", "expected": "15", "computed": "15", "match": true},
    ...
  ]
}
```

#### Step 4: Frequency-based tranche ranking

Sort gaps by corpus frequency, then by industry prevalence as tiebreaker. The ranking in §2.4 is the starting point; Step 3 data refines it.

### 3.4 Implementation location

| File | Content |
|------|---------|
| `crates/lint/src/formula_parity.rs` | New module: formula inventory + parity check harness |
| `crates/lint/src/lib.rs` | Export `formula_parity` |
| `corpus/fixtures/formula-parity/` | New corpus subcategory: formula-parity fixtures with known-good inputs/outputs |
| `tools/parity_formula.sh` | CLI runner for formula parity lint |

---

## 4. Implementation plan

### Tranche A (highest-frequency gaps)

**Goal:** Add the 10–12 most frequently-needed functions + parser fixes for `$`, `^`, `%`.

#### Subtask A1: Parser fixes ($, ^, %) — M
- [ ] Add `$` token to tokenizer (not an operator, just a prefix on cell refs)
- [ ] Add `^` token as exponentiation operator (right-associative, higher precedence than `*`)
- [ ] Add `%` token as unary postfix percent (multiply by 0.01)
- [ ] Update `FormulaNode` if needed (e.g. `UnaryPercent`)
- [ ] Update `evaluator.rs` evaluation for `^` and `%`
- [ ] Tests: `$A$1` refs, `A1^2`, `10%`, `A1^2*10%`
- Files: `parser.rs`, `evaluator.rs`

#### Subtask A2: Math functions — M
- [ ] `ABS(x)` — absolute value
- [ ] `ROUND(x, d)` / `ROUNDUP(x, d)` / `ROUNDDOWN(x, d)` — rounding
- [ ] `MOD(dividend, divisor)` — remainder
- [ ] `INT(x)` — truncate to integer
- [ ] `POWER(base, exponent)` — alternative to `^`
- [ ] Tests: round-trip with known inputs/outputs
- Files: `evaluator.rs`

#### Subtask A3: Text functions (common subset) — M
- [ ] `LEN(text)` — string length
- [ ] `TRIM(text)` — whitespace trimming
- [ ] `UPPER(text)` / `LOWER(text)` — case conversion
- [ ] `SUBSTITUTE(text, old, new, [n])` — string replacement
- [ ] `LEFT(text, [n])` / `RIGHT(text, [n])` — substring
- [ ] `MID(text, start, n)` — substring
- [ ] `REPT(text, n)` — repeat string
- [ ] `CONCAT(...)` — modern variadic concat (synonym for CONCATENATE)
- [ ] Tests: each function with edge cases (empty strings, unicode)
- Files: `evaluator.rs`

#### Subtask A4: Date/Time (basic) — M
- [ ] `TODAY()` — current date (return 0.0 or a configurable epoch date)
- [ ] `NOW()` — current date/time (same as above — no real clock in zero-dep Rust)
- [ ] `YEAR(date)` / `MONTH(date)` / `DAY(date)` — date decomposition
- [ ] `DATEDIF(d1, d2, unit)` — date difference (unit: "d", "m", "y")
- [ ] `EOMONTH(start, months)` — end of month
- [ ] Design note: without `std::time`, use a fixed "analysis date" configurable via `FormulaContext` trait
- [ ] Tests: deterministic fixtures with known dates
- Files: `evaluator.rs`, `evaluator.rs::FormulaContext` trait addition

#### Subtask A5: Statistical (common subset) — M
- [ ] `COUNTA(...)` — count non-empty
- [ ] `COUNTBLANK(...)` — count empty cells
- [ ] `STDEV(...)` / `STDEVP(...)` — standard deviation
- [ ] `VAR(...)` / `VARP(...)` — variance
- [ ] `RANK(value, range, [order])` — rank within range
- [ ] Tests: statistical correctness vs. known values
- Files: `evaluator.rs`

#### Subtask A6: Error handling expansion — S
- [ ] `#DIV/0!` — already produced, needs matching in parity comparison
- [ ] `#VALUE!` — type mismatch in functions
- [ ] `#NUM!` — numeric overflow/underflow
- [ ] `#N/A` — value not available (used by VLOOKUP/MATCH later)
- [ ] `ISERROR(val)` / `ISNUMBER(val)` / `ISTEXT(val)` — type checks
- [ ] `IFNA(value, fallback)` — improved error handling over IFERROR
- [ ] Tests: each error type, type-check functions
- Files: `evaluator.rs`, `parity.rs` (error matching)

### Tranche B (lookup + statistical)

**Goal:** Add VLOOKUP/HLOOKUP, INDEX/MATCH, advanced statistics.

- [ ] `VLOOKUP(lookup_val, table, col_idx, [exact])` — vertical lookup (M)
- [ ] `HLOOKUP(lookup_val, row, row_idx, [exact])` — horizontal lookup (M)
- [ ] `INDEX(array, row, [col])` — array index (S)
- [ ] `MATCH(lookup_val, range, [type])` — position lookup (M)
- [ ] `CORREL(x, y)` — correlation coefficient (S)
- [ ] `PERCENTILE(range, p)` — percentile (S)

### Tranche C (financial + remaining)

**Goal:** Financial functions, advanced date/time, text advanced.

- [ ] Financial: `NPV`, `IRR`, `PMT`, `PV`, `FV`, `RATE`, `NPER` (M per 2)
- [ ] Date/Time: `WORKDAY`, `NETWORKDAYS`, `WEEKNUM`, `DATEVALUE` (M)
- [ ] Text: `TEXT(value, format)`, `TEXTJOIN`, `SEARCH`, `FIND`, `LEFT`/`RIGHT` byte variants (M)
- [ ] Logical: `XOR`, `IFS`, `SWITCH` (S per)

### Tranche D (cleanup + polish)

**Goal:** Named range support, structured references, 3D references.

- [ ] Named range resolution (requires OPC package-level name lookup) (M)
- [ ] 3D references `Sheet1:Sheet3!A1` (S)
- [ ] Structured table references `[Column1]` (M)

---

## 5. Verification strategy

### 5.1 Corpus parity test suite

Create `corpus/fixtures/formula-parity/` with fixtures containing known input/output pairs:

| Fixture ID | Purpose | Formulas |
|-----------|---------|----------|
| `formula-parity-math-001` | Math function correctness | `=ROUND(3.14159,2)`, `=MOD(10,3)`, `=ABS(-5)`, `=INT(3.9)` |
| `formula-parity-text-001` | Text function correctness | `=LEN("hello")`, `=TRIM("  hello  ")`, `=UPPER("hello")` |
| `formula-parity-date-001` | Date function correctness | `=YEAR(44927)`, `=MONTH(DATE(2024,6,15))` |
| `formula-parity-stats-001` | Statistical correctness | `=STDEV(1,2,3,4,5)`, `=VAR(1,2,3,4,5)` |
| `formula-parity-lookup-001` | Lookup correctness | `=VLOOKUP("B",A1:B2,2,0)` |
| `formula-parity-ops-001` | Operator correctness | `=A1^2`, `=10%`, `=$A$1` |
| `formula-parity-error-001` | Error handling correctness | `=IFERROR(1/0,"err")`, `=IFNA(#N/A,"not found")` |

### 5.2 Round-trip test pattern

For each new function:
1. Create a formula fixture with deterministic inputs
2. Run stichos `inspect render` and read computed values
3. Verify against expected outputs (embedded in fixture manifest)
4. Compare with LibreOffice output (if external engine available)

### 5.3 Parity lint CI check

New CI step (`tools/parity_formula.sh --ci`):
1. Run formula inventory over corpus
2. Assert 100% of corpus formulas are either implemented or refused with GP-F0xx
3. Report any formula that causes a parser error (should not happen — all corpus formulas must be parseable)
4. Output frequency table as JSON artifact

### 5.4 Refusal-list conformance test

Extend `refuse.rs` conformance test to verify:
- Every formula in the corpus that is refused has a corresponding GP-F0xx code
- No formula in the corpus causes a panic/unwrap failure
- The `supported_functions()` list is exhaustive (no function in `eval_call` missing from `supported_functions()`)

---

## 6. Size estimate

| Tranche | Functions | Size | Dependencies |
|---------|-----------|------|-------------|
| A1 | Parser fixes (`$`, `^`, `%`) | M | none |
| A2 | Math (ABS, ROUND, MOD, INT, POWER) | M | A1 |
| A3 | Text (LEN, TRIM, UPPER, LOWER, SUBSTITUTE, LEFT, RIGHT, MID, REPT, CONCAT) | M | A1 |
| A4 | Date/Time (TODAY, NOW, YEAR, MONTH, DAY, DATEDIF, EOMONTH) | M | A1 |
| A5 | Statistical (COUNTA, COUNTBLANK, STDEV, VAR, RANK) | M | A1 |
| A6 | Error handling (ISERROR, ISNUMBER, ISTEXT, IFNA) + error types | S | A1 |
| **Tranche A total** | **~28 functions + 3 parser ops** | **5M** | A1 |
| B1 | Lookup (VLOOKUP, HLOOKUP, INDEX, MATCH) | M | A–F |
| B2 | Statistical advanced (CORREL, PERCENTILE) | S | A |
| **Tranche B total** | **~6 functions** | **1.5M** | A–F |
| C1 | Financial (NPV, IRR, PMT, PV, FV, RATE, NPER) | M | A–F |
| C2 | Date/Time advanced (WORKDAY, NETWORKDAYS, WEEKNUM, DATEVALUE) | M | A |
| C3 | Text advanced (TEXT, TEXTJOIN, SEARCH, FIND) | S | A |
| C4 | Logical advanced (XOR, IFS, SWITCH) | S | A |
| **Tranche C total** | **~18 functions** | **3M** | A–F |
| D1 | Named range resolution | M | A–F |
| D2 | 3D references | S | A |
| D3 | Structured table references | M | A |
| **Tranche D total** | **3 features** | **2M** | A–F |
| **Grand total** | **~55 functions** | **~11.5M** | — |

---

## 7. ADR-0011 amendment proposal (ADR-0011-amend-formula-coverage)

### 7.1 Current ADR-0011 content

ADR-0011 decided to build a minimal evaluator supporting 12 functions (~200–400 LOC) with parity lint and GP-F0xx refusals. The ADR estimated cost at "~200-400 LOC" which was accurate for v1.0.0.

### 7.2 What needs to change

1. **Cost estimate update:** The original "~200-400 LOC" estimate covered only the 12-function subset. With full coverage (~55 functions), the evaluator module will grow to approximately **2,500–3,000 LOC**. The ADR should be updated to reflect that this is a **phased expansion**, not a one-time build.

2. **Phase gate update:** Add a reference to the tranche roadmap. The ADR's "Decision" section should note that v1.0.0 is the initial subset and future tranche additions (A–D) are tracked under P7-3.

3. **Refusal list completeness:** ADR-0011 §3 claims "11 GP-F0xx codes cover all xlsx formula failure modes." This is correct as stated — the refusal codes are comprehensive. However, the ADR should acknowledge that implementing functions from refused categories will convert GP-F0xx codes into supported functions (e.g., implementing `STDEV` removes the need for `GP-F-006` for that function).

4. **Parity lint scope:** ADR-0011 §2 (Option A) mentions "parity lint catches stale cached values." The proposed parity-lint gap analysis (§3 above) extends this to include corpus-level formula frequency analysis, which should be noted as a Phase 7 enhancement.

### 7.3 Proposed ADR-0011 text additions

Add to ADR-0011 §3 Evidence:
> - **Measured cost (v1.0.0):** 12 functions in ~500 LOC (evaluator.rs + tests). Phase 7 expansion to ~55 functions estimated at ~2,500 LOC across 4 tranches (see P7-3).

Add to ADR-0011 §4 Consequences — Option A:
> - **Phased expansion:** Tranche A (P7-3A) adds ~28 functions covering the highest-frequency gaps (math, text, date/time, statistical basics). Subsequent tranches add lookup, financial, and advanced functions. Each tranche is additive-only (C15 versioning).

Add to ADR-0011 §Decision:
> - **Future scope:** The 12-function v1.0.0 subset is the initial delivery. Full formula coverage (~55 functions across 4 tranches) is tracked as P7-3 in the Phase-7 backlog. Each tranche requires a C15 version bump only if semantics change; new function additions are additive-only per C15 §5.

### 7.4 ADR status update

| Field | Current | Proposed |
|-------|---------|----------|
| Status | accepted | accepted (amended) |
| Version | v1.0.0 (12 functions) | v1.0.0 → v1.1 (tranche A) → ... → v2.0 (full coverage) |
| Scope note | "minimal subset" | "minimal subset, phased expansion to full coverage via P7-3" |

---

## 8. First action items (before any code)

1. **Run corpus formula inventory** — parse all corpus `<f>` elements, extract function names, produce frequency table
2. **Build formula-parity fixtures** — create `corpus/fixtures/formula-parity/` with known-good input/output pairs for tranche A functions
3. **Implement parity-lint inventory pass** — `crates/lint/src/formula_parity.rs` with corpus traversal
4. **Submit ADR-0011 amendment** — update cost estimates and scope notes before tranche A starts
5. **Gate C15 version bump** — increment to v1.1.0-draft for tranche A (additive, per C15 §5)

---

*Document created 2026-09-24 for P7-3 intake. Gap rankings based on corpus scan of 6 formula fixtures + industry usage data. Corpus currently covers only ~4% of Excel's ~475 built-in functions.*
