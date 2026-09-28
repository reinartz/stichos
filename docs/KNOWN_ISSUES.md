# Known Issues

Tracked issues from Phase 1 engine-connectivity work (2026-09-11).

---

## KI-01: PDF parser returns 0 pages for LibreOffice-generated PDFs

**Status:** fixed 2026-09-12
**Crate:** `adapters-pdf` — `crates/adapters-pdf/src/parser.rs`
**Impact (resolved):** `inspect_document` on a PDF returned `document_type: "unknown"`
with 0 page elements; `render_document` page-count detection fell back to `pdfinfo`.

**Root cause (three layers):**
1. `scan_indirect_objects()` searched only a 100-byte window before each `endobj` for
   the `N M obj` header. LO writes object dictionaries whose body between the header
   and `endobj` is longer than 100 bytes, so those objects were never found. Worse,
   binary stream payloads that merely resemble `N M obj ... endobj` produced garbage
   `Integer(N)` entries.
2. The xref fill-in pass called the object parser at the raw byte offset without
   skipping the `N M obj` header, overwriting the real Catalog/Pages/Page objects with
   the same garbage and breaking the page-tree walk.
3. In the scanner's fallback path, when the tail (xref/trailer) contained no further
   `endobj`, each byte triggered a full rescan of the remaining file (O(n²)) — ~27 s on
   a 460 KB, 3000-object deck.

**Fix:**
- Replaced the ad-hoc parse with `parse_object_payload()` (stream-aware) shared by the
  scanner and the xref pass; added `parse_indirect_object_at()` which skips the
  `N M obj` header and finds the real `endobj`.
- The xref fill-in now parses each object from its xref offset and overwrites scanner
  garbage (objects the byte scanner cannot reach are recovered, so long-dict LO PDFs
  resolve correctly); stream data is decompressed via `try_decompress`.
- Raised the object cap 500 → 200 000 with an in-loop timeout check.
- Scanner now `break`s once the remainder holds no further `endobj`, eliminating the
  O(n²) tail rescan (27 s → ~30 ms on the 460 KB deck).
- `find_stream_kw` in `parser.rs` no longer underflows for object bodies shorter than
  5 bytes; `tokenize_stream` in `text.rs` no longer spins on unrecognized binary bytes
  (skips them, `pos += 1`).

**Regression tests added** (`crates/adapters-pdf`):
- `test_parse_lo_style_long_dict_pdf`: LO-style synthetic PDF (long dicts > 100 bytes,
  spurious `endobj` inside a stream, standard xref) must resolve to 1 page via the xref
  fill-in.
- `test_scan_stops_at_last_endobj_without_tail_rescan`: a 400 KB endobj-free tail must
  not trigger the O(n²) rescan.
- `test_tokenize_binary_appended_after_extra_spaces`: tokenizer terminates on binary
  bytes.

**Verification:** all 151 `goldens/staging/*/render.pdf` now pass `stichos inspect`
with no timeouts and no `type: unknown` (all previously failed large LO decks included).
Full `adapters-pdf` suite: 67 tests green.

---

## KI-02: `rasterize_with_soffice` ignores the page argument

**Status:** mitigated (duplicate-output guard + page-count cap)
**Crate:** `engines` — `crates/engines/src/raster.rs:288`

**Impact:** When soffice is the only raster tool available, calling `rasterize(pdf, N, dpi)`
for any page N returns the first page of the PDF (soffice converts the entire PDF, ignoring
the `page` parameter). Without mitigation this produces 100 identical copies of page 1.

**Mitigations applied:**
1. `rasterize_all_pages` now caps the loop at `expected_pages` when known (from `pdfinfo`),
   so only the correct number of pages are rasterized.
2. A duplicate-output guard breaks the loop if the same bytes are returned consecutively
   (catches the soffice case where page 1 is repeated).
3. Hard cap remains at 100 for the unknown-page-count fallback.

**Remaining concern:** If `pdfinfo` is also unavailable and the PDF parser returns 0 pages,
the loop still relies on `rasterize(page)` failing for non-existent pages. With the
soffice fallback that never fails, the loop runs to the 100-page hard cap. This is unlikely
in practice (pdftoppm or gs are almost always present).

---

## KI-03: `pdftoppm` output written to process CWD (now fixed)

**Status:** fixed 2026-09-11
**Crate:** `engines` — `crates/engines/src/raster.rs:100`

**What happened:** `pdftoppm` writes output relative to the process's current working
directory, but the code read the output from the temp PDF's directory. The output prefix
was `"page"` (relative), so the PNG landed in the repo root while the code looked in
`/tmp/stichos/`.

**Fix:** Output prefix now uses the temp PDF's absolute path
(`path.with_file_name("page")`), so pdftoppm writes to the same temp directory.

---

## KI-04: Pandoc reports "FAIL" for all doctor fixtures (expected)

**Status:** partially resolved (2026-09-27, Track C-C1.1)
**Crate:** `engines` — `crates/engines/src/doctor.rs`

**What happens:** `stichos doctor` probes all engines with the standard fixture set
(document → PDF). Pandoc correctly reports `works: false` for every capability because
pandoc cannot produce PDF output (it produces markdown, HTML, etc.). This is accurate and
not a bug.

**Pandoc's real capability:** `extract_text()` (docx → markdown via stdin pipe) works
correctly and is tested in the full test suite (e.g., `test_pandoc_extract_text_from_markdown`).

**Future fix:** The standard fixture set should include a markdown-source fixture so pandoc
can demonstrate at least one working capability in `doctor`. Currently the fixture set is
document → PDF only.
Resolved in 2026-09-27 (Track C-C1.1): markdown-source probe fixture added to `standard_fixtures()`.

---

## KI-05: `render_document` output directory uses relative path

**Status:** active
**Crate:** `cli` — `crates/cli/src/render.rs`

**What happens:** The `render_document` MCP tool accepts an absolute file path, but writes
output to a directory derived from the input filename. If the MCP process CWD differs from
the caller's expectation, output may be written to an unexpected location. The CLI `stichos
render` command uses the same logic, but since it runs in the caller's CWD, it works as
expected.

**Impact:** The MCP `render_document` tool currently does not report the output directory
path back to the caller, making it impossible to locate the rendered images without
inspecting the JSON response for `page_count` and guessing. A future enhancement should
return the output path or content hashes.

---

## KI-06: Gate-C defaults flip uncommitted (M6 closeout blocking)

**Status:** resolved (committed 2026-09-24 in `ff80f8e`, pushed) — residual compliance
violations re-filed as KI-08
**Area:** CLI defaults, MCP envelope stamping, gate checks
**Created:** 2026-09-23 (post-M6 audit)

**What was committed in `ff80f8e`:**
1. `--apply` opt-out, `dry_run=true` default, `spec.rs` 10 tools → `schema_version: "1.6.0"`
2. `__budget` + `diagnostics` + `rendered_by` envelope stamped on all CLI verb outputs (T-B1,
   `crates/cli/src/budget_enforcer.rs`)
3. Envelope stamped on all 23 MCP tool responses in serve handler (T-B2)
4. Gate checks 1b/1d/1e/1f/1g/1i hardened to fail on violations (T-C1); gate filter broadened
   (T-C3); `mcp --lib` ceiling tests green 93/93 (T-A1); C7.md 1.6.0 + C14.md frozen (T-A2/T-A3)
5. Tag `m6` re-anchored to `ff80f8e`; `main` + `m6` pushed.

**Residual (see KI-08):** with the binary built, the hardened gate measures 3 real CTX-06
violations — the tree the `m6` tag sits on is not fully gate-green.

---

## KI-07: Toothless gate checks (historical, fixed via T-C1)

**Status:** fixed — tracked here for audit trail
**Area:** `tools/phase6_gate.sh` checks 1b, 1d, 1e, 1f

**What happened:** Four Gate-1 checks had `pass` in both branches — they called `fail` on
the success path and `pass` with a status note on the failure path. This meant the gate
reported green even when the contract was violated.

**Fix:** All four checks now call `fail` on violation with a descriptive message. The
`--planted-violation` test mode confirms the gate catches each fixed check.

**Evidence:** `phase6_gate.sh --ci` on the post-fix tree reports 5 failures (all expected
due to KI-06 — the Gate-C flip is still uncommitted). Without T-C1, all 88 checks passed
despite the violations.

---

## KI-08: CTX-06 contract violations measured by the hardened gate (M6 closeout round 2)

**Status:** fixed — resolved by Track A′ (R-A1, R-B1, R-B2, R-C1) in 2026-09-24 working tree
**Area:** `cli` inspect summary, diff caps/wire shape, region flags
**Created:** 2026-09-24 (measured at `ff80f8e` with `./target/debug/stichos` built)
**Fixed:** 2026-09-24

Before (measured at `ff80f8e` with built binary): `phase6_gate.sh --ci` **86/90**, `p606_ctx06_compliance` **3/6**.

After (working tree + `cargo build -p stichos`): `phase6_gate.sh --ci` **90/90**, `p606_ctx06_compliance` **6/6**.

Fixes applied:
1. **Inspect summary budget:** Replaced full `element_ids` array with `element_kinds` count map + 5-entry sample in `inspect.rs`. `tokens_est` dropped from 5988 to ~230 (target < 500). JSON payload: 1,385 bytes (was 34 KB).
2. **Diff default cap:** Applied `max_ops=20` default in `diff_tool.rs`; added `--max-ops 0` → unlimited normalization.
3. **Wire shape:** Renamed `truncated` → `hasMore`, `total_ops` → `totalOperations` in `ir_diff` JSON.
4. **Region flags:** Added `--json` and `--output urls|base64|both` flags to CLI `region` verb.
5. **Journal wire:** Added `total_entries` field to journal output.

---

## KI-09: Gate "binary not built (skipped)" paths can mask failures

**Status:** fixed — resolved by Track A′ (R-D1) in 2026-09-24 working tree
**Area:** `tools/phase6_gate.sh` checks 1b–1f
**Created:** 2026-09-24
**Fixed:** 2026-09-24

Fixed: R-D1 added `cargo build -p stichos` at the beginning of the gate script. The gate now always builds the binary before running binary-dependent checks — no more silent skips. KI-08 violations were masked by this skip path at tag time.

---

*Last updated: 2026-09-24*
