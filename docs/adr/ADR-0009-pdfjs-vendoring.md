# ADR-0009: pdf.js Vendoring (M5 Wave J)

**Status**: proposed (2026-09-18, contributor) → **accepted** (2026-09-18, owner)
**Decisions**: D24 (pdf.js text-layer wave)

## 1. Context

Phase 4 (M4) shipped a review overlay using raster page images. D24's revisit trigger
(text-selection anchoring) is now active: reviewers need to select text from a PDF page
and anchor comments by bbox + verbatim quote. This requires a text layer over the raster
page images.

D2 mandates that the stichos C stays out of the binary — no JS engine linked, no native
bindings. D4 mandates zero hard external dependencies for the Rust crates. The text layer
must be a **vendored web asset**, served by the existing review server, with hash-pinned
integrity.

The existing `crates/mcp/src/http.rs` review server already serves the viewer HTML and page
images. P5-09 will extend it to serve vendored pdf.js assets.

## 2. Options

### Option A: Vendored pdf.js (recommended)

Vendor the **text-layer components only** from `pdfjs-dist@3.11.174`:
- `build/pdf.min.js` — core PDF.js library (320 KB)
- `build/pdf.worker.min.js` — worker thread (1.1 MB)
- `web/pdf_viewer.js` — TextLayerBuilder + utilities (232 KB)
- `web/pdf_viewer.css` — minimal styles (56 KB)
- Supporting: cmaps (168 files, CJK support), standard fonts (16 files), images (12 files)

Total core files: ~1.7 MB uncompressed. No build step. No npm at runtime.
Served as static files from the existing review HTTP server.

**Hash manifest**: `assets/vendor/pdfjs/HASHES.toml` — SHA-256 of each core file,
verified by `tools/p5_08_verify_hashes.sh`.

### Option B: CDN-hosted pdf.js

Serve pdf.js from a CDN (e.g., unpkg, cdnjs).

**Rejected**: violates no-CDN requirement (§8 of phase5-concept); hash cannot be pinned;
network dependency at review time.

### Option C: Custom text layer

Build a minimal text-layer renderer from scratch using only our own PDF parser.

**Rejected**: reinvents complex text-extraction heuristics already solved by pdf.js;
would introduce maintenance burden; CJK text handling alone is non-trivial.

## 3. Evidence

### Measured

| Item | Value |
|------|-------|
| pdfjs-dist version | 3.11.174 (latest 3.x stable) |
| Core files (4) | 1,696,146 bytes total |
| CJK cmaps | 168 files |
| Standard fonts | 16 files |
| Images | 12 SVG/GIF files |
| License | Apache-2.0 (Mozilla Foundation) |
| Node version on host | v22.23.1 (for viewer logic tests) |

### Researched

- pdf.js is licensed Apache-2.0 (compatible with project's MIT/Apache-2.0 dual licensing)
- TextLayerBuilder is a stable, well-tested module used in the full pdf.js viewer
- The `pdf_viewer.js` bundle exports TextLayerBuilder as a named export
- No DOM dependency in the core library; viewer components (TextLayerBuilder) need DOM

### Source

- `npm view pdfjs-dist@3.11.174` — version info
- `npm pack pdfjs-dist@3.11.174` — package contents

## 4. Consequences

### Adopted: Vendored pdf.js

**What it locks in:**
- pdf.js 3.11.174 as the text-layer renderer (pinned via HASHES.toml)
- Opt-in per session (`--text-layer` flag)
- Vendored assets served from `assets/vendor/pdfjs/`
- No npm at runtime, no build step
- Text selection anchors comments by `{page, rect, quote}`
- CSP stays `script-src 'self'` (our JS only)
- Text layer is the only new third-party **artifact** (D24), not a new Rust crate (D2)

**What it costs:**
- ~1.7 MB of additional repo size (PDF-specific, justifiable for the feature)
- Viewer JS grows by ≤1k LOC (our code, not pdf.js code)
- New route: `GET /viewer/pdf/*` for serving vendored assets

**What it makes harder:**
- Upgrading pdf.js requires re-verifying hashes (but HASHES.toml makes this explicit)
- Text layer strings become DOM nodes (XSS surface — mitigated by server-side escaping
  + `document.createTextNode` usage in our code + CSP `script-src 'self'`)

**deny.toml**: No change (pdf.js is not a Rust crate).

**C7 v1.4**: No tool change (text layer is viewer behavior, not an MCP tool).
