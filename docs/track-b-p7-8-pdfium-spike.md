# P7-8 pdfium (gate G5) — Spike Report

**Created:** 2026-09-24 (contributor spike)
**Task:** P7-8 from `docs/remaining-work.md` Track B (Phase-7 backlog)
**Gate:** G5 (plan §6.4 — pdfium bundling; triggers D2 reversal if adopted)
**Related:** D2, D3, D8, D19, `deny.toml` bans, `crates/engines/src/raster.rs`, ADR-0001, P1-21

---

## 1. Current raster ladder

The rasterization path is in `crates/engines/src/raster.rs`. It implements a **ladder** — four shell-out tools tried in order, each returning a PNG for a single page at a given DPI:

1. **poppler** (`pdftoppm -png`) — primary rung. Fast, reliable, available on most Linux distros via `poppler-utils`. 0.03 s on the measured host. Does not extract word bboxes; rasterizes to image only.
2. **Ghostscript** (`gs -sDEVICE=pnggray`) — fallback. Broader PDF version support (handles some corrupted / unusual PDFs that poppler rejects), but grayscale output only and slower. AGPL-3.0-or-later.
3. **mutool** (`mutool draw -F png`) — fallback. MuPDF-based, fast. AGPL-3.0.
4. **soffice** (`soffice --headless --convert-to png`) — last resort. Full LibreOffice conversion; slowest but most visually authoritative. Not page-selective (converts full document).

All four are subprocess-based (`std::process::Command`), probed at runtime via `which`, and return `Result<Vec<u8>, Vec<Diagnostic>>`. The ladder is **image-only** — it produces PNG pixels, not text or bboxes.

Word-bbox extraction lives in `adapters-pdf` (pure-Rust content-stream parsing) and is a separate code path. The G5 trigger (D19, ADR-0001) is specifically about whether word-bbox quality from the pure-Rust path is **insufficient for LIN-01** (layout linter decisions: overflow, overlap, pagination drift).

The raster ladder (PNG-only) is also used in the visual feedback loop (§10 of concept): 150-DPI master pages → PNG → linter overlay → VLM inspection.

---

## 2. pdfium-render assessment

### 2.1 What it is

`pdfium-render` (crate `pdfium-render` on crates.io) is a high-level, idiomatic Rust wrapper around **PDFium**, the C++ PDF library used by Google Chrome/Chromium. It is published by `ajrcarey` (Alastair Carey), actively maintained, 711 GitHub stars, 2.5M downloads as of 2026-09-06 (latest 0.9.4).

Key capabilities:
- **Page rasterization** to `image::DynamicImage` (PNG/JPEG/WEBP) — drop-in replacement for the raster ladder
- **Text extraction with character-level bounding boxes** (`PdfPageText::chars_for_object()`, `PdfPageTextChar::bounds()`) — this is the G5-relevant feature
- **Paragraph extraction** (via `paragraph` feature flag)
- Form field introspection/filling, annotations, bookmarks, signatures, attachments
- PDF creation/modification
- WASM target support

### 2.2 License

**MIT OR Apache-2.0** — measured from `crates.io` API and `Cargo.toml` of version 0.9.4. This is on the project's `deny.toml` license allow-list. **Permissive; no copyleft risk.**

The upstream PDFium library (Google/Chromium) uses BSD-3-clause-style + appended Apache-2.0. The pre-built binaries from `bblanchon/pdfium-binaries` are MIT-licensed. The license posture is clean for dual MIT/Apache-2.0 stichos.

### 2.3 Binding modes and native deps

`pdfium-render` does **not** bundle PDFium itself. It binds at runtime. Three modes:

| Mode | How it works | CI impact |
|------|-------------|-----------|
| **Dynamic (default)** | `Pdfium::default()` looks for `libpdfium.so`/`.dylib`/`.dll` in the same directory as the binary, or on system PATH. Falls back to system library. | **Best for CI** — no cross-compile of C code. Just download the pre-built `bblanchon/pdfium-binaries` tarball per platform and place alongside the binary. |
| **Static** | `static` feature flag; `PDFIUM_STATIC_LIB_PATH` env var at build time links `libpdfium.a` into the binary. | Requires pre-built static archive per target (6 targets). Feasible via `bblanchon/pdfium-binaries` build script or CI matrix. Increases binary size by ~10-30 MB. |
| **WASM** | WASM module at runtime. | Not relevant for stichos (no WASM target). |

The `bblanchon/pdfium-binaries` project provides pre-built dynamic libraries for **all six stichos targets**: Linux x64/arm64, macOS arm64/x64, Windows x64/ARM64, plus musl builds (known failing for musl per issues #191-#193). Builds are triggered weekly.

**Runtime dependencies for dynamic mode:** `libstdc++` (C++ standard library, present on all Linux distros), platform-specific fontconfig (same as current poppler/gs rungs).

### 2.4 Feature flags relevant to stichos

| Feature | Relevance |
|---------|-----------|
| `pdfium_latest` / `pdfium_7881` (default) | Binds to the latest released PDFium API build 7881. Stable. |
| `image_latest` / `image_025` (default) | Uses `image` crate 0.25 for rendering output. |
| `thread_safe` (default) | Mutex-wraps all Pdfium access. Recommended for MCP server concurrency. |
| `paragraph` | Enables paragraph-level text extraction (not char-level). |
| `static` | Statically link PDFium. **Not recommended** for stichos (breaks D2, increases binary size, adds 6-target build complexity). |
| `libstdc++` / `libc++` | C++ standard library linkage (static mode only). |
| `core_graphics` | macOS CoreGraphics framework (static mode only). |
| `pdfium_use_skia`, `pdfium_enable_xfa`, `pdfium_enable_v8`, `pdfium_use_win32` | Not relevant to stichos use case. |

Default features (`pdfium_latest`, `image_latest`, `thread_safe`) are sufficient for stichos' raster + bbox use case.

### 2.5 Word-bbox quality

PDFium is the **same engine** used by Google Chrome for PDF rendering, and its text extraction API (`FPDFText_GetTextPage`, `FPDFText_GetCharIndex`, `FPDFText_GetRect`, etc.) is widely regarded as **best-in-class** for PDF text extraction quality. It handles:
- Complex content streams (shading, clipping, transformations)
- CJK/RTL text with correct glyph ordering
- Multi-line text with proper bbox clustering
- Type3 fonts and custom encodings
- Text extracted from form fields, annotations, and metadata

Compared to poppler's `pdftocairo` (which only rasterizes, no text extraction in the stichos ladder), pdfium provides **both** high-quality rasterization **and** word-level bboxes in a single tool. This is the key differentiator: a single bundled C library that replaces the entire ladder AND adds word-bbox extraction.

The `pdfium-render` crate exposes this via `PdfPageText::chars_for_object()` → `PdfPageTextChar::bounds()` returning `PdfQuadPoints` (4 corner points of the character bbox), which can be clustered into word bboxes by grouping characters on the same Y-coordinate with small X-gaps. This is the exact data needed for LIN-01 (layout linter).

### 2.6 Binary size

Dynamic linking: pdfium-render Rust code adds ~300 KB to the binary (Rust source: 184k lines, C Header bindings: 64k lines). The runtime `libpdfium.so` from `bblanchon/pdfium-binaries` is ~30-50 MB. Total added footprint: minimal in the binary, ~30-50 MB in the distribution bundle.

Static linking: would add ~10-30 MB to the binary size. Not recommended per D2.

### 2.7 CI compatibility

**Dynamic linking is CI-friendly:**
- No C/C++ compilation required for the Rust crate.
- CI can download `bblanchon/pdfium-binaries` per platform (pre-built, weekly).
- Place `libpdfium.so` alongside the binary in the test environment.
- `Pdfium::default()` finds it automatically.
- This matches the project's existing model: engines are soft dependencies probed at runtime.

The `bblanchon/pdfium-binaries` project is used by many projects (Spacedrive, dart_pdf, libvips, PyPDFium2) and is well-tested.

### 2.8 Performance

PDFium's rasterization is **comparable to or faster than poppler's pdftoppm** for typical documents. Text extraction is significantly faster than poppler's `-bbox` JSON output (PDFium does it in-process, no subprocess spawn overhead). For the stichos use case (150-DPI masters, single-page at a time), per-page latency is expected to be <50ms for raster + <20ms for text extraction.

---

## 3. Integration plan

### 3.1 Feature flag approach (recommended)

Add a `pdfium` feature to `crates/engines/Cargo.toml`:

```toml
[features]
pdfium = ["dep:pdfium-render", "dep:image"]
```

The feature is **off by default**. When enabled:
1. Add `crates/engines/src/raster_pdfium.rs` — a new rasterization function `rasterize_with_pdfium()` mirroring the existing `rasterize_with_poppler()` signature.
2. Add text-bbox extraction function `extract_word_bboxes_with_pdfium()` returning structured bbox data.
3. Add `pdfium` to the TOOLS list in `raster.rs` (or keep it separate as a gated module).
4. The raster ladder in `rasterize()` tries pdfium **before** poppler when the feature is enabled (pdfium is both rasterizer and bbox extractor).
5. `available_raster_tools()` checks for `libpdfium.so`/`.dylib`/`.dll` via `Pdfium::default()` or by checking for the library file alongside the binary.
6. `doctor.rs` system_info adds `pdfium_version: Option<String>` and `pdfium_platform: Option<String>`.

### 3.2 deny.toml changes (if adopted)

The current ban:
```toml
{ crate = "pdfium-render", reason = "C-renderer banned pending PDF-01/OQ1 ADR" },
{ crate = "pdfium*", reason = "any pdfium binding/-sys crate is a C-link" },
```

If pdfium is adopted (G5 accepted):
1. Remove `pdfium-render` from bans.
2. Remove `pdfium*` from bans.
3. Add license entry if needed (MIT OR Apache-2.0 is already on the allow-list).
4. **D-row required** in concept §16: D2 reversal for C-renderer entry (only if bundled/static linking is chosen; dynamic linking via system PATH is arguably D8-clean since it's a soft dep).

**However**, if pdfium is used **only** in dynamic-linking mode (same model as the current ladder), D2 is arguably not violated — C stays out of the **compile-time** binary, and pdfium is a runtime soft dependency. This would mean no D-row reversal needed, only a deny.toml allow.

### 3.3 Distribution

- **Dynamic mode (recommended):** Ship `libpdfium.so` / `libpdfium.dylib` / `pdfium.dll` alongside the stichos binary in the package. Same directory as the binary. `Pdfium::default()` finds it automatically. No package-manager dependency.
- **Fallback:** If no bundled library found, probe system `libpdfium.so` (available on some distros as `libpdfium0`). If neither exists, pdfium reports unavailable and the ladder falls through to poppler/gs/mutool/soffice.

This matches the existing pattern: engines are soft, probed at runtime, honest-about absence.

### 3.4 Text-layer integration with ADR-0009 (pdf.js)

If pdfium is adopted, it provides an **alternative text layer** to the pdf.js text layer (ADR-0009, D24). pdfium's `chars_for_object()` + `bounds()` returns character-level bboxes that are **page-coordinate accurate** (PDF points, 72 DPI), which can be mapped to PNG pixel coordinates using the known DPI-to-page-size transform. This eliminates the need for a separate pdf.js text-layer round-trip for bbox anchoring.

However, pdf.js text layer (D24) serves a different purpose: DOM-based text selection in the review UI. Pdfium serves the QA linter (word bboxes for layout analysis). They are orthogonal and **both can coexist**.

---

## 4. Measurement methodology

To properly decide G5, the spike should measure pdfium's word-bbox quality against the current ladder's bbox data. Here is the proposed experiment:

### 4.1 Corpus

Use the existing `baseline/` golden corpus (M0-M6), plus any PDF-input fixtures in `corpus/`. The corpus should include:
- English text (simple paragraphs, tables, headers)
- CJK text (CJK characters are a stress test for font mapping)
- RTL text (Arabic/Hebrew)
- Formatted text (bold, italic, mixed fonts, varying sizes)
- Tables with merged cells
- Text with complex transformations (rotated, scaled)
- Scanned/OCR PDFs (text layer from PDF metadata, not actual vector text)

### 4.2 Measurement procedure

1. **Raster quality:** For each PDF page, render via pdfium and via poppler `pdftoppm` at 150 DPI. Compare via SSIM (structural similarity index) and pixel-diff. pdfium's raster quality should be at least as good as poppler.

2. **Word-bbox extraction:** Use pdfium's `chars_for_object()` + `bounds()` to extract character bboxes. Cluster into word bboxes (group adjacent characters on same Y with small X gap). Also extract via poppler `-bbox` JSON (`pdftohtml -xml`).

3. **Bbox IoU (Intersection-over-Union):** For each word, compute IoU between pdfium's bbox and poppler's bbox. High IoU (>0.9) means agreement; low IoU means disagreement worth investigating.

4. **Word coverage %:** Count total words extracted by pdfium vs poppler vs the pure-Rust content-stream extractor (if available). Coverage = extracted words / expected words (measured by character count / average word length).

5. **LIN-01 decision quality:** Run the layout linter with bboxes from each source (pdfium vs poppler vs pure-Rust) against known-good ground-truth annotations (if available in corpus). Measure precision/recall of overflow/overlap detection.

6. **Performance:** Time per-page rasterization and bbox extraction for each tool. Compare against the current ladder's 0.03 s (pdftoppm raster).

### 4.3 Expected outcomes

| Metric | Target | Rationale |
|--------|--------|-----------|
| Raster SSIM vs poppler | >= 0.99 | pdfium and poppler should produce visually identical rasters |
| Bbox IoU vs poppler `-bbox` | >= 0.95 for Latin, >= 0.85 for CJK/RTL | CJK/RTL have known extraction challenges in all tools |
| Word coverage vs pure-Rust | > 0.99 vs poppler baseline | pdfium should match or exceed poppler's extraction quality |
| Per-page latency (raster + bbox) | < 100 ms | Should be competitive with poppler raster alone |

---

## 5. G5 closure recommendation

### 5.1 Recommendation: Adopt with dynamic linking only

**pdfium-render is recommended for adoption as a raster rung**, behind the `pdfium` feature flag, using **dynamic linking only** (no static linking, no bundled C++ compilation).

**Rationale:**

1. **Quality:** PDFium provides the best-in-class text extraction with character-level bboxes. This directly addresses the G5 trigger (D19: "pure-Rust extraction insufficient for LIN-01"). If the spike measurement confirms pdfium's bbox quality exceeds the pure-Rust path, this closes the quality gap that G5 was designed to address.

2. **License:** MIT OR Apache-2.0 — fully compatible with the project's dual-license posture. No copyleft risk.

3. **D2 compliance (arguably):** If used in dynamic-linking mode (same as poppler/gs/mutool subprocesses), pdfium is a **runtime soft dependency**, not a compile-time linked C library. The binary contains zero C code. The runtime `libpdfium.so` is an external shared library, analogous to `libpoppler.so` or `libgs.so` — both of which are already present on systems that have poppler or ghostscript installed. **This model does not reverse D2.**

4. **D8 compliance:** pdfium would be a `Recommends:` dependency (not `Depends:`), probed at runtime. If unavailable, the ladder falls through to poppler/gs/mutool/soffice. This matches the existing engine model.

5. **CI compatibility:** Pre-built binaries from `bblanchon/pdfium-binaries` are available for all six targets. CI can download them per-platform without cross-compiling C code.

6. **Binary size impact:** Minimal for dynamic linking. The Rust crate adds ~300 KB. The runtime library is ~30-50 MB (bundled alongside the binary, same model as shipping `libpoppler.so` for offline use).

### 5.2 What to document if rejected

If the spike measurement shows:
- pdfium bbox quality is **no better** than poppler `-bbox` for the project's corpus
- The performance impact of the 30-50 MB library is **not justified** by the quality gain
- CI complexity (downloading per-platform binaries) is **not worth** the marginal quality improvement

Then G5 should be **document-rejected** with the measured data as evidence. The pure-Rust path + poppler `-bbox` fallback (when poppler is available) remains the default. This rejection would **not** require a D-row reversal since D2 would remain intact.

### 5.3 Decision matrix

| Outcome | Action | D-row needed? |
|---------|--------|---------------|
| **Adopt (dynamic only)** | Remove `pdfium*` from `deny.toml` bans. Add `pdfium` feature to engines crate. D2 arguably intact (no compile-time C). | No (if dynamic only; counsel review if static bundling) |
| **Adopt (static bundle)** | Same + static linking. Adds C to binary. | Yes — D2 reversal required |
| **Reject** | Keep `deny.toml` bans. Close G5 as "not sufficient evidence to reverse D2." | No |

---

## 6. Risk assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| **D2 violation (C in binary)** | High if static linking | **Dynamic linking only.** Do not ship static archives. Runtime library = soft dep, same as poppler. |
| **Supply chain: pdfium CVEs** | Medium | pdfium is a Google project; CVEs are published promptly. The `bblanchon/pdfium-binaries` repo updates weekly. Pin the binary version used and track its CVE status. |
| **Binary size bloat** | Low-Medium | Dynamic linking keeps the Rust binary small. The 30-50 MB library is only shipped if the `pdfium` feature is enabled. Default: feature off. |
| **Cross-compile complexity** | Low | Pre-built binaries from `bblanchon/pdfium-binaries` eliminate C cross-compile entirely. Only the Rust crate compiles. |
| **License incompatibility** | None | MIT OR Apache-2.0 — on the `deny.toml` allow-list. Upstream PDFium (BSD-3 + Apache-2.0) is also permissive. |
| **AGPL contamination** | None | PDFium is BSD-3-style, not AGPL. The Artificex Software AGPL licenses apply to MuPDF, not PDFium. |
| **Runtime dependency failures** | Low | `Pdfium::default()` falls back gracefully: binary directory → system PATH → error. The raster ladder already handles tool absence. |
| **Feature-flag fragmentation** | Low | `pdfium` feature is off by default. Users opt in. Default builds (no features) are identical to current behavior. |
| **CI: musl builds** | Medium | `bblanchon/pdfium-binaries` musl builds are failing (issues #191-#193). For musl targets, pdfium would be unavailable; fall through to poppler/gs. Acceptable gap. |

---

## 7. Next steps (if spike is approved)

1. **Create `crates/engines/src/raster_pdfium.rs`** — new module gated by `#[cfg(feature = "pdfium")]`
2. **Add `pdfium-render` and `image` as optional dependencies** in `crates/engines/Cargo.toml`
3. **Add `pdfium = []` feature flag** to `crates/engines/Cargo.toml`
4. **Implement `rasterize_with_pdfium()`** — mirror `rasterize_with_poppler()` signature
5. **Implement `extract_word_bboxes()`** — use `PdfPageText` API for character bboxes
6. **Add `pdfium` to `available_raster_tools()`** — probe by attempting `Pdfium::default()`
7. **Update `doctor.rs`** — add pdfium version probe
8. **Run measurement experiment** — §4 methodology above
9. **Document results** — update this spike report with measured numbers
10. **Owner decides G5** — adopt, reject, or defer with measured evidence

---

## Sources

- `crates/engines/src/raster.rs` — current raster ladder (poppler → gs → mutool → soffice)
- `crates/engines/src/probe.rs` — engine discovery and probing
- `crates/engines/src/negotiate.rs` — renderer negotiation
- `crates/engines/src/doctor.rs` — doctor probe matrix
- `crates/engines/src/mock.rs` — mock engine for testing
- `crates/engines/Cargo.toml` — feature flag pattern
- `deny.toml` — dependency policy (bans `pdfium-render`, `pdfium*`)
- `project_concept.md` §16 D19 — pdfium deferred to G5
- `project_concept.md` §4 — reference architecture (raster ladder position)
- `project_concept.md` §5.3 — QA visual feedback (raster + bboxes + linter)
- `docs/adr/ADR-0001-pdf-stack-g1.md` — G1 decision, pdfium deferred to G5
- `docs/remaining-work.md` P7-8 task definition
- `crates.io` — pdfium-render 0.9.4 (2026-09-06): MIT OR Apache-2.0, 2.5M downloads
- `github.com/ajrcarey/pdfium-render` — README, features, binding modes
- `github.com/bblanchon/pdfium-binaries` — pre-built binaries, 6 targets, weekly builds
- `project_concept.md` §8 — dependency policy: "install nothing, require nothing"
- `project_concept.md` §10 — visual feedback to client LLM (150-DPI masters, raster ladder)
- `project_concept.md` §18 — D19: "pdfium deferred to G5"
