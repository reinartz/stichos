# P7-4: Lint encode/ssim refresh — Spec Section

**Task:** Re-triage "deferred to Phase 2" notes: `lint::encode` fidelity tests, `lint::ssim` with updated baselines.
**Size:** M (2–4 files)
**Gates:** C9 (corpus manifest + metric definitions), REN-02 (token estimator, C8)
**Parent:** Track B — Phase-7 backlog (`docs/remaining-work.md` §P7-4)

---

## 1. Current State

### 1.1 `lint::encode` (`crates/lint/src/encode.rs`, 745 lines)

**What exists:**
- `encode_baseline()` — self-describing stub format: `[SN][width:4BE][height:4BE][quality:1][quantized data]`. NOT a real image format; designed for byte-exact accounting with deterministic quantization.
- `encode_jpeg()` — P1 stub that delegates to `encode_baseline()`.
- `encode_model_copy()` — full resize+encode pipeline: pick_resize_target → compute_resize_target → resize_grayscale → encode (JPEG/WebP).
- `resize_grayscale()` — area-based averaging stub (nearest-neighbor in practice for P1).
- `pick_resize_target()` / `compute_resize_target()` — standard resize table (768x1024, 1024x1024, 2000x2000).
- `EncodedImage` — serializable result with visual tokens, byte cost, dimensions.

**Current tests (30+ tests, all passing):**
- Visual token computation (4 tests): `ceil(w/28) * ceil(h/28)` — table-driven, covers edge cases including 0x0.
- Resize (6 tests): same-size identity, down, up, aspect-ratio preservation (square/rectangular/vertical).
- `pick_resize_target` (4 tests): small/medium/large/fallback, boundary conditions.
- `compute_resize_target` (3 tests): exact fit, ratio preservation, clamping.
- `encode_baseline` (6 tests): structure, determinism, quality variance, header size, quantization, min/max quality.
- `encode_jpeg` (1 test): returns Ok with stub format.
- `encode_model_copy` (7 tests): full pipeline, WebP format, landscape/portrait, max-size enforcement, byte accounting, serialization roundtrip, empty image.
- Constants (1 test): MAX_ENCODE_SIZE == 2000, JPEG_QUALITY == 85.

**What is deferred (Phase 2):**
- Line 7 comment: *"Real JPEG/WebP encoders are deferred to Phase 2."*
- `encode_jpeg()` returns stub, not real JPEG bytes.
- `EncodeFormat::Webp` path uses baseline encoding, not real WebP.
- No fidelity tests that compare real encoder output (JPEG/WebP) against baselines.

### 1.2 `lint::ssim` (`crates/lint/src/ssim.rs`, 448 lines)

**What exists:**
- `ssim()` — full SSIM implementation: 11x11 Gaussian window, sigma=1.5, sliding window over grayscale rasters.
- `ssim_with_threshold()` — threshold-gated variant returning `Diagnostic` on failure.
- `rgb_to_grayscale()` — BT.601 weights (0.299*R + 0.587*G + 0.114*B).
- `png_to_grayscale()` — P1 stub: raw grayscale passthrough only. Comment: *"Full PNG decoding is deferred to Phase 2."*
- `compare()` — dimension-mismatch-aware wrapper.

**C9 compliance (verified in code):**
- Window: 11x11, sigma=1.5 (lines 15–17).
- Constants: C1 = 6.5025, C2 = 58.5225, DATA_RANGE = 255.0 (lines 8–13).
- Trimming to common per-axis minimum on dimension mismatch (lines 54–58).
- No SSIM clamping — uses `clamp(-1.0, 1.0)` per-window only (line 109).

**Current tests (13 tests, all passing):**
- SSIM core: identical images (~1.0), completely different (~0.0), partial match.
- Edge cases: image too small (returns 0.0), dimension mismatch (uses min).
- Color conversion: RGB to grayscale with known BT.601 outputs.
- Gaussian window: positive values, sums to 1.0.
- Constants: exact C9 values verified.
- Threshold: pass/fail cases with diagnostic code LIN-I-001.
- Parity reference: solid color patches.
- `png_to_grayscale`: raw passthrough, wrong-size error.

**What is deferred (Phase 2):**
- Real PNG decoding — `png_to_grayscale()` only accepts raw grayscale bytes.
- C9 §8 parity: Rust SSIM vs Python `skimage` reference on identical input rasters (`ε_parity := TBD-1`).
- SSIM against real golden rasters from `corpus/` (not synthetic test vectors).

### 1.3 Related modules

| Module | Relationship to P7-4 |
|--------|---------------------|
| `lint::budget` (550 lines) | REN-02 token estimator `compute_visual_tokens(w, h)` — formula `ceil(w/28)*ceil(h/28)` matches `encode.rs` duplicate. Shared reference for fidelity testing. |
| `lint::diff` (837 lines) | XML part diff — P1 line-by-line stub. *"Full XML-aware diff is deferred to Phase 2."* Not directly in P7-4 scope but related fidelity concern. |
| `lint::sheet` | *"Full pixel compositing is deferred to Phase 2."* Not in P7-4 scope. |
| `lint::validate` | *"Best-effort per the Phase 2 plan."* Not in P7-4 scope. |
| `gp-core/src/envelope.rs` | REN-02 re-statement: `estimate_visual_tokens()` mirrors `lint::budget::compute_visual_tokens` with parity unit test. |

### 1.4 Harness SSIM reference

- `harness/metrics/metrics/ssim.py` — Python reference using `skimage.metrics.structural_similarity` with `gaussian_weights=True, use_sample_covariance=False, data_range=255, win_size=11, sigma=1.5`.
- `harness/metrics/tests/test_ssim.py` — 6 harness-side SSIM tests covering identical, shifted, blank-vs-text, size-mismatch trim, missing pages, empty dirs.

### 1.5 C9 contract (C9 §4, §8)

- **Per-page SSIM**: 8-bit grayscale, 150 DPI, 11x11 Gaussian sigma=1.5, trim to common minimum, arithmetic mean over aligned pages.
- **SSIM parity** (C9 §8): Rust implementation must agree with Python reference on identical input rasters; `ε_parity := TBD-1` (measured, not hardcoded).

### 1.6 REN-02 (C8)

- Token estimator: `ceil(width/28) * ceil(height/28)`.
- Already implemented in `lint::budget` (authoritative) and `encode` (duplicate).
- Already mirrored in `gp-core::envelope::estimate_visual_tokens` with parity test.
- C8 frozen at M1; additive-only changes permitted.

---

## 2. Deferred Items

### D1: Real JPEG encoder fidelity tests (encode.rs)
**Status:** Deferred to Phase 2, re-triaged to Phase 7.
**Details:** `encode_jpeg()` still delegates to `encode_baseline()`. No test validates real JPEG encoding quality at q=85. When a pure-Rust JPEG encoder is available (or a feature-gated dep lands), tests should:
1. Compare `encode_jpeg()` output against a known-good JPEG encoder (e.g., `jpeg` crate).
2. Verify byte-exact determinism (same input → identical bytes).
3. Measure quality factor impact on output size and perceptual fidelity.

### D2: Real WebP encoder fidelity tests (encode.rs)
**Status:** Deferred to Phase 2, re-triaged to Phase 7.
**Details:** `EncodeFormat::Webp` uses baseline encoding. Same test pattern as D1.

### D3: Real PNG decoding for SSIM (ssim.rs)
**Status:** Deferred to Phase 2, re-triaged to Phase 7.
**Details:** `png_to_grayscale()` is a raw-bytes passthrough. When PNG decoder is available, tests should:
1. Decode known PNG fixtures from `corpus/` to grayscale.
2. Compare against harness `PIL.Image.convert("L")` output for parity.

### D4: C9 SSIM parity against harness reference (ssim.rs)
**Status:** Open — `ε_parity := TBD-1` per C9 §8.
**Details:** Run Rust `ssim()` and Python `structural_similarity()` on identical golden raster pairs from `corpus/`. Measure max absolute deviation. This is the core parity gate.

### D5: SSIM against corpus goldens (ssim.rs)
**Status:** Open — no SSIM tests use real corpus golden rasters.
**Details:** All current SSIM tests use synthetic vectors (`vec![128u8; N]`). Real goldens from `corpus/render-goldens/` would validate the full pipeline.

### D6: `encode_baseline` fidelity vs real JPEG at q=85
**Status:** Stub format is by design for P1, but should be benchmarked when real encoder lands.
**Details:** Baseline quantization step = `max(1, 256/quality)` does not correspond to real DCT-based JPEG quality.

---

## 3. Baseline Status

### 3.1 Are current baselines valid?

**SSIM:** YES. The SSIM algorithm is deterministic and matches C9 specification exactly. The Rust implementation is a direct port of the skimage reference algorithm (same window, same constants, same formula). No refresh needed — the algorithm is sound.

**Encode:** N/A. The `encode_baseline` format is intentionally not a real image format. There are no "baselines" to refresh because there is no real encoder to compare against yet.

**Harness SSIM:** The Python harness SSIM implementation uses `skimage` and is the reference. It is current and valid.

### 3.2 Do baselines need refresh?

**No.** The C9 contract was frozen at M0. The SSIM parameters (11x11, sigma=1.5, 150 DPI) are normative and unchanged. The encode parameters (JPEG q=85, MAX_ENCODE_SIZE=2000, resize table) are from C8/M1 and unchanged.

What needs to happen is **new baselines** — golden raster pairs from the corpus that can be used for:
1. SSIM parity testing (Rust vs Python on same inputs).
2. Encode fidelity testing (when real encoder lands).

### 3.3 Corpus golden availability

The corpus at `corpus/fixtures/` contains source documents (.docx, .xlsx, .pptx, .pdf, etc.) and manifests at `corpus/manifests/`. Golden render rasters would be generated by the harness render pipeline (`harness/render/golden.py`) at 150 DPI grayscale. These exist or can be regenerated via:
```bash
cd harness/ && python3 render/golden.py --dpi 150 --gray --format png
```

---

## 4. Implementation Plan

### Step 1: Inventory deferred notes in `crates/lint` (S)
- Document all "deferred to Phase 2" comments found (listed in §2 above).
- Verify no additional deferred notes exist in other lint modules.
- Update module doc comments in `encode.rs` and `ssim.rs` to reflect Phase 7 status.

### Step 2: Refresh SSIM golden pairs from corpus (M)
- Select a representative subset of corpus fixtures (3–5 documents across docx/xlsx/pptx/pdf).
- Generate golden rasters at 150 DPI grayscale using the harness pipeline.
- Create synthetic "perturbed" variants (small brightness/contrast shifts, Gaussian noise) to test SSIM sensitivity.
- Store golden and perturbed pairs in a dedicated test directory (e.g., `crates/lint/tests/fixtures/ssim/`).

### Step 3: Add SSIM parity reference tests (M)
- Add tests in `ssim.rs` that load golden raster pairs from `crates/lint/tests/fixtures/ssim/`.
- Verify:
  - Identical pairs score SSIM >= 0.999.
  - Perturbed pairs score in expected ranges (e.g., brightness shift → 0.95–0.99, noise → 0.80–0.95).
  - Black vs white pairs score < 0.1.
- Add a **C9 parity test** that runs both Rust `ssim()` and Python harness `ssim_pages()` on the same golden pair, asserts `|ssim_rs - ssim_py| < ε` where ε is measured (start with ε = 0.01 for floating-point tolerance).

### Step 4: Add encode baseline integrity tests (S)
- Add tests verifying `encode_baseline()` structure and determinism with corpus-derived raw pixel data.
- Add a benchmark comparison: `encode_baseline()` output size vs expected JPEG size for same-resolution/quality (document the divergence — baseline is not real JPEG).
- These tests do NOT require a real encoder — they document that the stub is a stub.

### Step 5: Update module documentation (S)
- Update `encode.rs` module doc: change "deferred to Phase 2" to "deferred to Phase 7".
- Update `ssim.rs` module doc: note that PNG decoding is Phase 7 and C9 parity is TBD-1.
- Update `lib.rs` if module-level docs need updating.

### Step 6: Update remaining-work.md (S)
- Mark P7-4 as in-progress or complete with a link to this spec.
- Record any new deferred items discovered.

### Step 7: Verify all tests pass
```bash
cargo test -p lint --lib
cargo test -p lint --test '*'
```

---

## 5. Verification Strategy

### 5.1 Unit tests
```bash
cargo test -p lint --lib encode::
cargo test -p lint --lib ssim::
```
All existing tests must continue to pass. New tests must be added in the `#[cfg(test)]` modules of `encode.rs` and `ssim.rs`.

### 5.2 C9 parity check
```bash
# In harness/
cd harness/ && python3 -m metrics --ssim <golden_dir> <candidate_dir>
# Compare output with Rust ssim() on same inputs
```
Acceptance: max absolute deviation `|ssim_py - ssim_rs|` < ε where ε is recorded in this spec after measurement.

### 5.3 No regressions
```bash
cargo test --workspace --lib  # Full workspace test suite
cargo clippy -p lint -- -D warnings  # Lint hygiene
```

### 5.4 Corpus golden regression
When corpus goldens are regenerated:
1. Record SHA-256 of each golden raster.
2. Add SHA-256 assertions in parity tests to catch golden drift.
3. If goldens change, re-measure ε and update this spec.

---

## 6. Size Estimate

| Step | Size | Effort |
|------|------|--------|
| 1. Inventory deferred notes | S | ~15 min |
| 2. Generate SSIM golden pairs | S | ~30 min |
| 3. Add SSIM parity reference tests | M | ~2h |
| 4. Add encode baseline integrity tests | S | ~30 min |
| 5. Update module documentation | S | ~15 min |
| 6. Update remaining-work.md | S | ~5 min |
| 7. Verify all tests pass | S | ~10 min |
| **Total** | **M** | **~3–4 hours** |

---

## 7. Dependencies

### C9 — Corpus manifest + metric definitions
- **C9 §4** (SSIM metric): Defines SSIM parameters. The Rust implementation already matches; tests should validate against this.
- **C9 §8** (SSIM parity): `ε_parity := TBD-1` — must be measured during this task.
- **C9 §10** (Implementation resolutions): Resolutions 1–13 apply; particularly #7 (tiny rasters < 11x11) and #10 (no SSIM clamping).

### REN-02 — Token estimator (C8)
- `lint::budget::compute_visual_tokens` is the authoritative implementation.
- `encode::EncodedImage::compute_visual_tokens` is a duplicate (should reference or unit-test for parity).
- `gp_core::envelope::estimate_visual_tokens` is a re-statement with parity test.
- C8 frozen at M1; no contract changes needed.

### REN-02 — Token estimator (C8)
- `lint::budget::compute_visual_tokens` is the authoritative implementation.
- `encode::EncodedImage::compute_visual_tokens` is a duplicate (should reference or unit-test for parity).
- `gp_core::envelope::estimate_visual_tokens` is a re-statement with parity test.
- C8 frozen at M1; no contract changes needed.

### Phase 7 dependency chain
- **C9 parity measurement** must happen before P7-4 is considered complete.
- Real JPEG/WebP/PNG encoder work (if any Phase 7 subtask includes it) is a separate task.
- P7-4 is **not gated** on real encoder implementation — it validates the stub behavior and establishes golden baselines for when encoders land.

---

## 8. Risk Register

| Risk | Impact | Mitigation |
|------|--------|------------|
| Corpus goldens unavailable or outdated | Cannot run parity tests against real data | Use harness to regenerate; document source of goldens |
| ε_parity larger than expected | C9 §8 requires parity; large deviation may indicate algorithmic drift | Investigate floating-point differences; document as known if < ε = 0.02 |
| Real encoder not available in Phase 7 | Encode fidelity tests remain stub-based | Document stub-vs-expected divergence clearly; defer real encoder tests to Phase 7+ |
| New external dep for PNG/JPEG | Adds dependency not in P7-4 scope | Keep tests stub-only; defer real encoder work to a separate Phase 7 subtask |

---

## 9. Files to Touch

| File | Change |
|------|--------|
| `crates/lint/src/encode.rs` | Update module doc comment; add baseline integrity tests |
| `crates/lint/src/ssim.rs` | Update module doc comment; add golden-pair parity tests |
| `crates/lint/src/lib.rs` | Possibly update module-level docs (unlikely) |
| `crates/lint/tests/fixtures/ssim/` (new dir) | Golden raster pairs for SSIM testing |
| `docs/remaining-work.md` | Update P7-4 status |
| `docs/contracts/C9.md` | Record measured ε_parity value in §8 (additive only) |

---

## 10. Open Questions

1. **Should ε_parity have a hard upper bound?** C9 §8 says "TBD-1 (measured)." P7-4 is the natural place to set this. Suggested starting point: ε = 0.01 for 8-bit grayscale, 11x11 window, σ = 1.5.

2. **Should P7-4 include a real encoder spike?** This spec treats encoder implementation as out-of-scope. If a pure-Rust encoder (e.g., `jpeg`, `image` crate with feature) is lightweight enough, it could be included, but that's a separate D2 (new dependency) ADR.

3. **What corpus subset for golden generation?** Suggested: 1 docx (text-heavy), 1 xlsx (table-heavy), 1 pptx (slide layout), 1 PDF (annotated). Total ~4–6 pages of golden rasters.
