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

**Status:** by-design
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

*Last updated: 2026-09-12*
