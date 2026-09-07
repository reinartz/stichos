# harness/metrics — C9 Phase-0 fidelity metrics (HAR-04)

Python reference implementation of the four C9 metrics: per-page SSIM (§4),
page-count delta (§5), feature-retention (§6), text-extraction F1 (§7).
Parity target: the Rust binary in `crates/lint` (LIN-06) must agree on
identical inputs (C9 §8); the skimage invocation flags in `metrics/ssim.py`
are the frozen reference.

## Requirements

Python 3.14 + Pillow, numpy, scikit-image (all verified present per
`docs/phase0-execution.md` environment). Feature counting uses only stdlib
(`zipfile`, `xml.etree.ElementTree`). Tests use `pytest` (already installed
system-wide, 8.4.2). **No new pip dependencies were added or needed.**

## Layout

```
harness/metrics/
  metrics/
    __init__.py    # public API re-exports, __version__
    __main__.py    # `python -m metrics` CLI
    errors.py      # MetricsError(code, message)
    ssim.py        # ssim_pages          (C9 §4)
    pages.py       # page_count_delta    (C9 §5)
    features.py    # feature_counts, feature_retention (C9 §6)
    text.py        # tokenize, text_f1   (C9 §7)
  tests/           # pytest suite; run: python -m pytest harness/metrics/tests -q
```

## Public API

```python
ssim_pages(a_dir, b_dir, dpi=150) -> dict
    {"dpi", "aligned", "min", "mean",
     "pages": [{"page", "ssim", "width", "height", "flags": [...]}, ...]}
    flags: "missing" | "size_mismatch" | "converted_to_grayscale"

page_count_delta(a_n: int, b_n: int) -> dict
    {"a_pages", "b_pages", "page_delta", "abs_page_delta", "pass"}

feature_counts(path) -> dict
    flat: feature keys + "format" ("ooxml"|"odf"), "part_count",
    "uncompressed_bytes" (declared). OOXML: "w:tbl", "w:drawing",
    "w:fldSimple", "w:instrText", "w:comment", "w:commentReference".
    ODF: "table:table", "draw:frame", "text:note", "text:field",
    "office:annotation".

feature_retention(before_counts, after_counts) -> dict
    {"features": {key: {"before", "after", "dropped", "changed"}},
     "changed": [key, ...], "pass": bool}

text_f1(reference_text, extracted_text) -> dict
    {"f1", "precision", "recall", "reference_tokens", "extracted_tokens",
     "common"}

tokenize(text) -> list[str]   # C9 §7 tokenization + CJK split (see below)
```

Errors raise `metrics.MetricsError` (`.code` is a stable string, see
`errors.py`).

## CLI

Run from `harness/metrics/` (or put it on `PYTHONPATH`). Every verb prints
one JSON object on stdout — `{"ok": true, ...}` or
`{"ok": false, "error": {"code", "message"}}` — and exits nonzero on error
(2 for domain/usage errors, 1 for unexpected internals).

```
python -m metrics ssim A_DIR B_DIR [--dpi 150]
python -m metrics feature-counts FILE.(docx|xlsx|pptx|odt|ods|odp)
python -m metrics retention BEFORE.json AFTER.json      # '-' = stdin
python -m metrics text-f1 REFERENCE.txt EXTRACTED.txt   # '-' = stdin
python -m metrics page-delta CANDIDATE_N GOLDEN_N
```

`feature-counts` output feeds `retention` directly. `--help` works on every
verb.

## Security limits (hostile-input guards)

* `MAX_COMPRESSED_BYTES` = 500 MiB — file refused before opening the zip.
* `MAX_UNCOMPRESSED_BYTES` = 2 GiB — enforced twice: against the
  central-directory declared total **before any part is read** (manifest-lie
  zip-bomb guard) and again as a byte budget charged to every streamed read.
* Parts are parsed with `iterparse` straight from the zip stream; nothing
  is ever written to disk.

## Tests

```
python -m pytest harness/metrics/tests -q     # 40 passed
```

## Deviations and C9 ambiguities resolved (flagged for C9 amendment)

1. **Dimension mismatch policy.** The HAR-04 brief's fallback ("mismatch =
   score 0 + flag") applies only if C9 leaves the policy open; C9 §4 defines
   it (trim both to the common per-axis minimum), so mismatched pages are
   **trimmed and still scored**, with a `size_mismatch` flag.
2. **Pages on one side only.** C9 §4 says the unaligned tail "is not
   scored"; the brief asks for a `ssim 0` + `missing` entry. Both honored:
   the entry appears in `pages` with `ssim: 0.0` and flag `missing`, but
   `min`/`mean` aggregate over aligned pages only (C9 semantics).
3. **`w:instrText` count.** Required by the brief but absent from C9 §6's
   table. Implemented as the number of **complex fields** (`fldChar
   begin…end`) that contain at least one `w:instrText`, attributed to the
   innermost open field. Suggest adding the row (or dropping the metric).
4. **Comment pair.** C9 §6 says "reported as a pair": emitted as two keys,
   `w:comment` (from `word/comments.xml`) and `w:commentReference` (from
   `word/document.xml`).
5. **ODF `text:note` vs `text:field`.** C9 §6 lists `text:field`; the brief
   asked for `text:note`. Both are counted in `content.xml` so either
   reading of C9 is served.
6. **CJK tokenization.** C9 §7 has no CJK rule. Deviation implemented and
   documented: Han/kana codepoints (ranges listed in `text.py`) become
   one-token-per-character; all other runs keep punctuation attached.
7. **Raster smaller than the 11×11 window.** Undefined by C9; raises
   `MetricsError("image_too_small")` rather than silently shrinking the
   window (keeps §4 parameters frozen).
8. **Page ordering.** C9 aligns "by page index" but doesn't say how a
   directory yields an order: files with raster extensions
   (png/pgm/pbm/ppm/pnm/tif) are **naturally sorted** (so `page-2` precedes
   `page-10`).
9. **Non-grayscale inputs.** C9 assumes `pdftoppm -gray` output; a
   non-gray raster is converted to 8-bit L and flagged
   `converted_to_grayscale` instead of rejected.
10. **SSIM clamping.** None — raw `skimage` output is returned so the Rust
    parity check (C9 §8) compares like with like (identical inputs yield
    exactly `1.0` in practice).
11. **`feature_retention` PASS.** C9's PASS is against the manifest
    `features{}` expectation, which this function never sees; here PASS =
    no feature count changed at all, and the per-feature `changed`/`dropped`
    entries implement §6's "any change is a report entry". Expectation
    checking stays with the caller.
12. **Metadata keys.** `part_count`/`uncompressed_bytes`/`format` don't
    match the manifest feature-key pattern and are excluded from
    `feature_retention`.
13. **Precision/recall with one empty side.** C9 fixes F1 (0.0); P/R are
    reported as 0.0 on the empty side (division guard).
