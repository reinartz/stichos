# stichos — Project Concept & Decision Record

**Tagline:** proof before publish.
**Status:** concept agreed · naming agreed · Phase 0 harness live (`docs/phase0-execution.md`), nightly ×3 green; Rust implementation not started; M0 gate pending GOV-01 opinion + GOV-02 sweep
**Last updated:** 2026-09-08 (D18–D20; OQ1/OQ5 resolved; measured identity re-check)
**Project root:** `/localdata/workspace/stichos`
**Related:** [`brand.md`](brand.md) — identity, colour, typography, logo usage

---

## 1. One-line summary

A local, install-anywhere **agentic tool for office documents** — analyse, edit, create — that owns
the file format, delegates layout to a real rendering engine (Microsoft 365 when present, otherwise
OSS), and returns **rendered images plus machine-readable check results** so the calling vision LLM
can literally look at what it produced.

---

## 2. The core insight that shapes everything

There are three distinct layers, and most "AI edits a document" tools fail by conflating them:

| Layer | What it is | Who can own it |
|---|---|---|
| **Object / serialisation** | `.docx/.xlsx/.pptx` = ZIP (OPC) + XML (ECMA-376, ~250 part types); `.odt/.ods/.odp` = ZIP + ODF XML; PDF = page-description language, ISO 32000 | **Us, completely** |
| **Document model / semantics** | styles, numbering, sections, fields, tracked changes, content controls, masters/layouts/placeholders, formulas, pivots, charts, embedded objects | Shared: we build an IR, semantics come from the specs |
| **Layout & rendering** | line breaking, autofit, row splitting, font substitution, pagination | **Nobody owns this without owning an office suite** |

Consequences:

1. **"All document features" is achievable** — OOXML/ODF are open specs; LibreOffice exposes nearly
   every user command through UNO/LOKit.
2. **Visual feedback is the hard half.** A `.docx` contains no layout. Word, LibreOffice and
   `docx-preview.js` disagree about page breaks.
3. Therefore: **edit at the object layer, get truth from a real engine, treat PDF as the lingua
   franca of visual proof, and always name the engine that produced it.**

---

## 3. Goals and non-goals

**Goals**
- G1 Read/analyse/edit/create docx, xlsx, pptx, odt/ods/odp, PDF (plus RTF/HTML/MD via pandoc).
- G2 Access the full feature surface, not just a text-extraction subset.
- G-3 **Visual feedback loop**: every edit can be rendered and returned as images to a vision LLM.
- G4 One-binary install on Linux (.deb/.rpm), macOS (Intel + Apple silicon), Windows (x86-64 + ARM64).
- G5 Use Microsoft 365 locally when available; degrade gracefully to OSS.
- G6 Never silently destroy content; report exactly what an edit cost.

**Non-goals**
- N1 Building a layout/typesetting engine.
- N2 An exhaustive "every OOXML element is a tool" MCP surface.
- N3 Being a WYSIWYG editor — we hand off to Collabora, ONLYOFFICE or Word for that.
- N4 Cloud-dependent core functionality (cloud OCR is an optional adapter).

---

## 4. Reference architecture

```
agent / client LLM
      │  MCP (stdio | streamable HTTP) + plain CLI
      ▼
~40 coarse, semantic, idempotent tools  +  sandboxed Rhai scripting escape hatch
      ▼
KERNEL ── content-addressed store · undo journal · IR ⇄ OOXML/ODF/PDF codecs
      │    patch engine (byte-surgical, not load-modify-save) · fidelity report
      ├── FORMAT ADAPTERS   OPC/XML, openpyxl-class writers, PDF object layer (MuPDF/qpdf/iText)
      ├── ENGINE POOL       LibreOffice (subprocess first), MS Office 365 COM (Windows),
      │                     Chromium/Typst for authoring, optional Word worker
      └── PDF OBJECT LAYER  incremental update · forms · annotations · redaction
      ▼
FEEDBACK / QA
  PDF → PNG (ladder) → word-level bboxes → layout linter → XML schema validation
  → PDF/A + veraPDF → perceptual page diff → images + JSON back to the LLM
```

**Contract principles** (the product, not the I/O):

- **P1 Surgical patches, never load-modify-save.** Address the target node, patch the part, leave
  every other byte of the OPC package untouched; incremental update for PDF.
- **P2 Fidelity budget in every response** — `dropped_features[]`, `rendered_by`, confidence.
- **P3 `dry_run` everywhere** — IR-level unified diff + preview before mutation.
- **P4 Stable element IDs** across sessions, so a 40-turn conversation doesn't lose "that third table".
- **P5 Agent edits as tracked changes** by default for high-stakes formats; human accepts.
- **P6 Never write in place** — original preserved, sidecar journal, one revert primitive.

---

## 5. Tool landscape (what already exists, what we build on)

### 5.1 Engines — where features and visual truth live

| Option | Coverage | Fidelity | Notes |
|---|---|---|---|
| **LibreOffice headless** (`--convert-to`, UNO, ScriptForge, `.uno:` dispatcher) | Very broad | authoritative for ODF, close-but-not-Word for `.docx` | MPL-2.0; biggest open asset; needs instance management |
| **LibreOfficeKit (LOKit)** | Same power, C API, callbacks | same | Collabora path; `coolconvert`; **demoted to optional plugin** (ABI fragility) |
| **MS Office 365 via COM** | 100 % of Word/Excel/PPT semantics; `CompareDocuments` = authoritative diff | **only authoritative `.docx` renderer** | Windows session constraints, licensing |
| **ONLYOFFICE DocumentServer** | conversion + builder APIs | good | self-hostable, WOPI |
| **Collabora Online (CODE)** | full LO core in browser | = LibreOffice | human editing surface with the agent's engine |
| **Chromium headless / Playwright** | whatever HTML+CSS expresses | deterministic and *ours* | author in HTML+CSS, screenshot with the engine the user sees |
| **Typst / XeLaTeX / WeasyPrint / FOP / Paged.js** | authoring & typesetting for new docs | high, deterministic | Typst = modern choice |

### 5.2 Format libraries

| Language | Word | Excel | PowerPoint | PDF |
|---|---|---|---|---|
| **Python** | `python-docx` (shallow), `docxtpl`, Aspose | `openpyxl` (MIT; documented feature loss on round-trip), `XlsxWriter`, `formulas`/`pycel` (partial eval) | `python-pptx` | **PyMuPDF** (AGPL), `pikepdf`, `pdfplumber`, `pypdfium2`, `reportlab`, `ocrmypdf`, `docling`, `markitdown` |
| **.NET** | **`DocumentFormat.OpenXml`** (official, broadest open), Aspose/Syncfusion/Spire | ClosedXML/EPPlus | Open XML SDK | iText 7, PdfSharp, **ODFTool** (best ODF anywhere) |
| **Java** | **docx4j** (Apache-2.0), POI | POI XSSF | POI XSLF | PDFBox, iText 7, JODConverter |
| **Rust** | `docx-rs` (young) | `calamine`, `rust_xlsxwriter`, `umya_spreadsheet` | thin | `lopdf`, `pdf-writer`, `pdfium-render`, **typst** as a library |
| **Go** | **unioffice** (AGPL/commercial) | `excelize` | unioffice | `pdfcpu` (forms, stamps, validation) |
| **TS/JS** | `docx`, `docxtemplater`, `mammoth` | `SheetJS`, `exceljs` | `pptxgenjs` | `pdf-lib`, **`pdfjs-dist`** (the viewer), Playwright |

Interchange/IR: **pandoc** (JSON AST, `--reference-doc`), `docling` / `markitdown` / `unstructured` /
`marker`, cloud layout-OCR (Document Intelligence, Textract). Google Docs `batchUpdate` remains the
best design template for an element-addressable tool surface.

### 5.3 QA & visual feedback

Rasterize: `pdftoppm` → `gs` → `mutool` → bundled `libpdfium` → `soffice --convert-to png`.
Layout lint: frame overflow, off-page/negative coords, overlaps, autofit shrink, **font substitution**,
missing glyphs, page-count delta, contrast. Diff: IR structural → XML part → perceptual (SSIM/`pixelmatch`);
authoritative track-changes via Word COM `CompareDocuments` or Aspose `Compare`.
Validation: ECMA/OASIS XSD, Open XML SDK validator, `qpdf --check`, veraPDF (PDF/A, PDF/UA).

### 5.4 Agentic prior art

Claude Agent Skills (`anthropics/skills`: `docx`, `pdf`, `pptx`, `xlsx`) — the reference implementation
of "code + libraries + render-and-check" instead of 500 MCP tools. Dozens of MCP servers
(GongRzhe Office-\*, SecurityRonin `docx-mcp` with `paraId` validation, `mcp-genoffice`
byte-preserving, `mcp-libreoffice`, `opendocswork-mcp`, Foxit PDF, Arcade M365).
**The gap:** they are thin wrappers with a fidelity ceiling and no layout verification loop —
which is exactly the gap stichos occupies.

---

## 6. Language decision

**Chosen: Rust** (all six targets verified installed locally: `x86_64/aarch64` × `unknown-linux-gnu`,
`apple-darwin`, `pc-windows-msvc`).

- One static artifact per platform; no shipped Python/Node/.NET runtime.
- **Pure-Rust COM** via the `windows` crate — Office 365 automation on Windows x64 *and* ARM64, no CGO.
- Cross-compilation stays clean **only if C stays out**: no engine is linked, engines are spawned.
- Memory-safe parsing of hostile input (malformed OPC, zip bombs, font tables, prompt-injected text).
- Embedded long-tail scripting via **Rhai** (pure Rust), not Lua/JS.

**Considered:** Go (credible alternative — static, CGO-free COM via `go-ole`, faster dev loop, but
AGPL-or-commercial unioffice and weaker zero-copy FFI story). .NET (the sleeper if the team were
Microsoft-centric: official Open XML SDK + COM interop; worse Linux/macOS packaging).
Python (rejected for the client: runtime + native wheels = install pain on five platforms; kept for
our own eval harness). Electron/C++ rejected outright for the client role.

---

## 7. Distribution matrix

Verified upstream artifacts (download.documentfoundation.org, stable tree): LibreOffice ships
`LibreOffice_26.2.5_Win_x86-64.msi` **and `Win_aarch64.msi`**, `MacOS_aarch64.dmg` **and** `MacOS_x86-64.dmg`,
and `deb/` + `rpm/` for both `x86_64` and `aarch64`. Disk footprint ≈1.5 GB (Win) / ≈800 MB (macOS) —
hence **soft dependency, never bundled**.

| Target | Rust target | Packages |
|---|---|---|
| Linux x86-64 / aarch64 | `*-unknown-linux-gnu` (old-glibc container) + `*-musl` fallback | **.deb and .rpm wrap the same binary**, tarball + `install.sh`, optional AppImage |
| macOS arm64 / Intel | `aarch64-apple-darwin`, `x86_64-apple-darwin`, universal `.pkg` via `lipo` | signed + **notarised** `.pkg`, Homebrew cask |
| Windows x86-64 / ARM64 | `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | **MSI (WiX)** + portable ZIP + `winget` + choco, Authenticode-signed with timestamp |

Client integration: one binary = CLI + MCP server + `doctor` + `selfupdate`.

```
claude mcp add office -- /usr/local/bin/stichos serve      # stdio
stichos serve --http :8731                                 # servers
```

---

## 8. Dependency policy: install nothing, require nothing, prove everything

1. **Baked in:** OPC zip/xml, OOXML/ODF codecs, IR + patch engine, PDF parse/write, linter, encode, MCP.
   → editing works with **zero** external software.
2. **Soft, probed at runtime:** LibreOffice, MS 365, poppler, Ghostscript, pandoc, Chromium, Typst.
3. **Optional profiles:** OCI image (LO + poppler + pinned fonts) for servers/CI; Word worker.

Metadata must never fail an install: DEB/RPM use `Recommends:` (not `Depends:`/`Requires:`);
Homebrew suggestions, winget hints. Package names are **resolved through the distro package manager
at doctor time**, never hardcoded — names drift between Ubuntu/Debian/Fedora/openSUSE/RHEL+EPEL.

> **Key discipline: a capability = a real conversion, not an installed package.**
> `stichos doctor` renders three bundled fixtures (docx/xlsx/pptx) through every discovered
> engine and reports what actually worked. Distro `libreoffice-core` without `libreoffice-impress`,
> a broken fontconfig cache, or Word stuck at activation all look "installed" to a naive probe.

**Fonts are first-class QA errors.** Verified here: `fc-match Calibri → Carlito`, `Cambria → Caladea`
(metric-compatible, freely redistributable, may be bundled). Non-metric substitutes are the #1 cause
of pagination drift and silent overflow on Linux. On Windows-with-Office you get the real
Calibri/Segoe UI — a second reason Office-first matters there.

---

## 9. Microsoft 365 engine negotiation

Order is cached per session and overridable: `renderer.preference = ["office","libreoffice","none"]`.

| Platform | Order | Mechanism | Design constraints |
|---|---|---|---|
| **Windows x64/ARM64** | **Office 365 → LibreOffice → built-in** | probe ProgIDs (`Word.Application`, `Excel.Application`, `PowerPoint.Application`), `HKCR\...\CurVer`, ClickToRun `ProductReleaseIds`, `WINWORD.EXE` in `root\Office16`; then COM | (a) Office automation is **unsupported from services / non-interactive contexts** ⇒ run as a **per-user agent started at logon**, not a Windows service; unattended batch = scheduled task in a logged-on session or a dedicated worker VM. (b) `DisplayAlerts=None`, `AutomationSecurity=High`, add-ins disabled, handle activation/first-run dialogs. (c) one document per instance, hard timeout, kill on wedge. (d) Store-packaged edition exposes ProgIDs differently from Click-to-Run → **probe, never assume**. (e) automating your own licensed install is fine; multi-tenant server use is a different licensing question |
| **macOS (arm64/Intel)** | **LibreOffice → Office 365 (opt-in) → built-in** | LO subprocess/UNO; Office via JXA/AppleScript | Office scripting needs per-app **TCC "Automation" consent**, which a launchd daemon cannot obtain silently and only works in a GUI session ⇒ opt-in, lowest priority, documented as such |
| **Linux** | **LibreOffice → built-in** | subprocess | distro LO may be years old ⇒ `doctor` reports version and refuses to be blamed for its gaps |
| **all** | always | own codecs | `visual_feedback: "unavailable"` is an honest, supported state |

Two invariants: **the renderer is never on the write path**, and **every result names its engine + version**
(`rendered_by: "Word 16.0 (17xxx)"` vs `"LibreOffice 26.2.5"`), because pagination differs and users must
be able to blame the right thing.

---

## 10. Visual feedback to the client LLM (with the cost model)

**Mechanism (spec-verified):** MCP results carry `content: ContentBlock[]` where
`ImageContent = { type:"image", data:<base64>, mimeType }` is valid, alongside `structuredContent`
for the QA JSON; tool errors go in `isError`, not a protocol error. One `edit_*` call returns
text + manifest + images in a single round trip.

**Cost model (Claude vision, verified):** visual tokens = `⌈w/28⌉ × ⌈h/28⌉`. Standard tier caps at
1568 px long edge / 1568 visual tokens; Claude 4.7+ high-res tier caps at 2576 px / 4784.
**>20 images in one request triggers a stricter per-image dimension limit — keep ≤20 image blocks or ≤2000 px per side.**

| Page | DPI | px | sent (standard) | tokens | high-res | tokens |
|---|---|---|---|---|---|---|
| A4 | 96 | 794×1123 | unchanged | ~1189 | unchanged | 1189 |
| A4 | **150 (master)** | 1240×1754 | downscaled | **~1551** | unchanged | 2835 |
| A4 | 300 | 2480×3508 | downscaled | ~1551 | 1623×2295 | 4756 |
| Letter | 150 | 1275×1650 | downscaled | ~1496 | unchanged | 2714 |
| 10 pages @150 | | | | **~15.5k** | | ~28.4k |
| 20 pages @150 | | | | ~31k | | ~56.7k |
| detail crop 25 %×20 % @2× | 620×702 | unchanged | **~598** | | 598 |

**Loop design**
1. **150-DPI masters always on disk**, cache key `sha256(doc) + engine + engine_version + fontset_hash + dpi`.
2. **Default payload = changed pages only** + a contact sheet; never re-send unchanged pages every turn
   (they count toward the 20-image limit permanently, including from earlier turns).
3. **Crop-on-demand is the token killer:** `inspect_region(page=3, bbox_pt=[72,240,300,320], zoom=2)`
   → ~600 tokens instead of a full high-res page, while still using 300-DPI masters.
4. **Annotate before you ask:** the deterministic linter draws numbered boxes; the image ships with them
   plus matching JSON. Ask "is region 4 actually clipped?", not "look at this page". Send marked + clean.
5. **Coordinate bookkeeping:** the model returns pixel coords in the *resized* image space. Pre-resize to
   a known size, store the scale, expose `bbox_pt` conversion — otherwise the crop loop silently drifts.
6. **Encoding:** PNG masters; JPEG q≈85/WebP for the model copy, inspecting the actual bytes sent
   (stacked lossy passes destroy small text).
7. **PDF document blocks** are an alternative input path, but count toward the same 20-block limit and
   forfeit annotation/crop control.

Measured on this machine (plain docx, cold): pandoc→docx **0.29 s**, `soffice --convert-to pdf`
**1.88 s**, `pdftoppm` **0.03 s**, pdfminer text+block extraction **0.08 s** → a full
*edit → render → look* cycle ≈ **2 s**, affordable **on every edit**.

---

## 11. Security & trust model

- **The document is hostile input.** Malformed OPC, zip bombs, XXE (openpyxl's docs explicitly warn
  about billion-laughs), font-parsing CVEs, external entity references, macro-bearing formats
  (xlsm/docm: macros disabled by default, flagged, never silently enabled).
- **Prompt injection via document text** is the top risk for an editing agent: ingested third-party
  content is taint-marked, quarantined from the instruction channel, and edits run against an
  allowlist policy (which parts may change; no exfiltration paths).
- **Integrity:** any edit invalidates digital signatures → explicit sign-later / PAdES workflow,
  never fake "signed"; PDF/A conformance dies on redaction → warn and re-validate.
- **Supply chain:** static Rust binary owns its dependency CVEs (`cargo audit` + Dependabot, one file
  to replace). LibreOffice and Office are patched by the distro and by Microsoft — **another reason
  not to vendor them**.
- **Determinism:** pinned LO version + fixed font set in containers; LO version changes layout, so
  cache keys include engine version and font-set hash; support N and N-1 engines only.
- **Telemetry (opt-in):** engine detection failures, fixture-conversion failures, fallback rates,
  per-engine latency histograms.
- **Safety of writes:** never in place; original preserved; journal; one revert primitive; sandboxed.

---

## 12. Naming

**`stichos`** — a galley proof is *the printout you inspect before the document is final*:
it names the differentiator (proofing), not the commodity (editing). Verb-friendly
(`stichos apply/render/diff/doctor`), no third-party marks, no `ai`/`gpt`/`agent` in the name
(dates the project, crowds it into a search bin with thousands of lookalikes).

Measured availability (404 = free), checked 2026-08-30:

| Name | crates.io | npm | PyPI | GitHub org | distro pkg | domain |
|---|---|---|---|---|---|---|
| **stichos** | **free** | **free** | **free** | **free** | none (Repology) | `.dev` free, `.com` taken |
| proofdesk | free | free | free | free | none | check |
| docgalley | free | free | free | free | none | check |
| recto | free | taken (abandoned Bootstrap fork) | free | taken | none | `recto.sh` free |
| overshot | free | free | free | taken | — | free |
| imprimatur | free | taken | taken | — | — | — |
| **rejected:** loupe | chai's `loupe` is everywhere in JS | | | | | |
| **rejected:** bindery | adjacent-domain crowding: 415★ web book-layout tool, PyPI C++ binding generator, ebook managers | | | | | |
| **rejected:** docent | collides inside AI-tooling (TransluceAI/docent + "Docent SDK" on PyPI) | | | | | |
| **rejected:** copydesk | live npm package in our exact space ("prose gate for AI agents") | | | | | |
| **rejected:** reticle | `reticlehq/reticle` is an AI-agent repo | | | | | |

Rules applied: no third-party trademarks (Office/Word/Excel/Copilot/LibreOffice/Writer/Calc/Impress);
no `ai`/`gpt`/`agent`; must survive as a CLI token (lowercase, no hyphen, ≤11 chars, spellable
aloud); **must not collide with a distro package name** (apt/dnf is the primary channel); free on all
three language registries so the install story needs no `@scope/`, `python3-` or `-rs` suffixes.

**Still to do:** trademark search in classes 9 and 42 (EUIPO/USPTO/WIPO Madrid); registries + domain
**the same day** (GitHub presence done 2026-09-07 — private repo on personal account, D18);
`apt-cache`/`dnf`/`brew`/`choco` searches on the five targets (done, GOV-03); MCP registry + skill-marketplace
id check (`io.github.reinartz/stichos`, 0 collisions measured).
**Re-measured 2026-09-08:** `stichos.org` RDAP **404 — free again** (the 2026-07-06 registration lapsed;
levships use-history still for counsel, clearance-memo §3); `stichos.dev` still free; GitHub handle
`stichos`/`Stichos` now **taken** (dormant personal account) — the org path is closed, `reinartz/stichos`
stands (D18). Domain decision (OQ5): **skip `stichos.com`, no homepage planned; `stichos.dev` only** (D20).

---

## 13. Brand & assets

See [`brand.md`](brand.md). Mark = **a galley sheet carrying a proofreader's tick**; three tiers
(full / tight / glyph) because one drawing cannot survive 1024px→16px; palette `ink #101A2E`,
`paper #FBF7EE`, `slate #93A0B8`, single accent `vermilion #E2483A` (deliberately not Office blue);
typography Inter + JetBrains Mono; always-lowercase wordmark with the vermilion tick crossing the
`y` descender; horizontal and vertical lockups; hero tile with crop marks and registration target.

Assets live in [`assets/`](assets/); regenerate every raster size with `assets/build-icons.sh`
(requires librsvg/ImageMagick; `.icns` on macOS via `iconutil`).

Design findings baked into the rules (verified by rasterising and *looking*):
- the full-detail mark **dissolves at 16px** → the tight tier exists (no text bars, 2px tick at 16px, grid-snapped);
- a **monochrome knockout glyph fails at 16px** (tick fills in) → use outline glyph ≥20px, tight tier below;
- a same-colour wordmark tick **without a knockout gap fuses `y` and `p`** → `wordmark-mono.svg` carries a mask gap;
- the tight tier reads as a **checkbox** at lockup sizes → lockups embed the full tier art.

---

## 14. Roadmap

**Phase 0 — harness before features.** Container with pinned LibreOffice + fixed font set;
150–200 genuinely nasty documents (merged cells, SmartArt, embedded OLE, tracked changes, XFA forms,
scanned OCR, CJK/RTL, 300-slide decks); metrics: per-page SSIM vs last approved render, page-count
delta, feature-retention assertions (`w:tbl`/`w:drawing`/`w:fldSimple`/comment counts before vs after).
Every library decision gets measured against this, not against marketing pages.

**Phase 1 — read.** Structure-aware extraction with coordinates (body/tables/headers/footnotes;
sheets with formulas *and* cached values; decks with masters/placeholders), PDF forms and annotations,
thumbnail rendering. Read-only ships safely and is immediately useful.

**Phase 2 — surgical edit + closed loop.** OPC-level patching with an ID map, byte-diff report,
LO/Word render + layout lint on every mutation, dry-run diffs.

**Phase 3 — creation from IR.** Markdown/HTML/Typst → `pandoc --reference-doc` / Chromium / Typst
for branded output; tracked-changes and comment output.

**Phase 4 — human loop.** pdf.js annotation overlay, accept/reject, in-suite add-in and/or Collabora embed.

Crate layout: `core` (IR + patch engine, dep-free) · `adapters-ooxml` · `adapters-odf` · `adapters-pdf` ·
`engines` (probe + subprocess runners) · `lint` · `mcp` · `cli`.

---

## 15. Gaps this project fills

| Gap | Why it matters |
|---|---|
| Byte-surgical OOXML/ODF editing + machine-readable **drop report** | nobody reliably answers "what did this edit cost me?" |
| **Layout CI**: deterministic overflow/substitution/overlap assertions per edit, VLM only at gates | turns "looks fine" into a regression test; ≈2 s cycles proven above |
| Agent edits **as tracked changes** with anchored comments and accept/reject | the enterprise wedge; current MCP servers barely touch it |
| Stable element IDs + semantic diff + undo journal — **"git for office documents"** | multi-turn, reviewable, auditable editing |
| Engine-agnostic renderer abstraction with mandatory provenance | the only honest way to talk about `.docx` fidelity |
| Accessibility remediation at scale (Tagged PDF/PDF-UA) verified by veraPDF | regulatory tailwind, badly underserved |
| Injection-hardened ingest with edit-allowlist policy | becomes a blocker the moment external files are ingested |

---

## 16. Decision log

| # | Decision | Rationale | Revisit when |
|---|---|---|---|
| D1 | Project named **stichos** | names the differentiator; free on all registries/orgs; no trademark in name | trademark search fails |
| D2 | **Rust**, all six targets | static single binary; pure-Rust COM; hostile-input parsing; clean cross-compile | team is Microsoft-centric → .NET |
| D3 | **Subprocess-first** engines; LOKit demoted to optional plugin | LOKit ABI must match installed LO build; install layouts differ per OS | need for >2 s/cycle throughput |
| D4 | No Python/Node/.NET runtime shipped | "installs cleanly everywhere" is the primary constraint | — |
| D5 | **Surgical patches, never load-modify-save** | libraries silently drop what they don't model | — |
| D6 | Fidelity budget + renderer provenance in every response | users forgive ugly, never silently destroyed documents | — |
| D7 | Tracked changes as the **default** collaboration primitive | legal/finance buyers; human accepts/rejects | — |
| D8 | Zero hard deps; `Recommends:` only; capability = real conversion | distro metadata lies; installs must never fail | — |
| D9 | Office 365 first on Windows, LO elsewhere, own codecs always | Office is the only authoritative `.docx` renderer | Office automation constraints bite |
| D10 | Per-user logon agent, not a Windows service | Office automation unsupported in non-interactive contexts | worker-VM option chosen |
| D11 | Rasterization ladder ending in `soffice --convert-to png` | never a dead end when only Office or only LO exists | — |
| D12 | 150-DPI masters + crop-on-demand regions | detail at ~600 tokens instead of a full high-res page | — |
| D13 | ≤20 image blocks per call, pre-resize to known size | stricter many-image dimension limit; coordinate drift | — |
| D14 | Fonts first-class; bundle Carlito/Caladea/Liberation/Noto | pagination drift and silent overflow | MS fonts become redistributable |
| D15 | Pinned engines, N and N-1 supported, golden corpus gates upgrades | LO releases change layout | — |
| D16 | Three-tier mark + Inter/JetBrains Mono + vermilion accent | 16px survival measured, not assumed | — |
| D17 | Rhai for the long-tail escape hatch | pure Rust keeps cross-compiling C-free | need for a JS sandbox |
| D18 | GitHub presence = **private repo under owner's personal account** (`reinartz/stichos`); no org | owner call 2026-09-07; solo private phase needs no org; MCP namespace follows owner handle (`io.github.reinartz/stichos`) | public launch; or squat of the still-unclaimed `stichos` handle forces the call — **measured 2026-09-08: handle `Stichos` taken (dormant personal account), org path closed — D18 stands** |
| D19 | PDF stack: **lopdf + pdf-writer** pure-Rust core, qpdf shell-out rung, mutool/gs optional D11 rungs, pdfium deferred to G5 | G1 accepted by owner 2026-09-08 (ADR-0001); D2/D8 preserved, AGPL never in-process | G5 experiment (PDF-04 word-bbox) shows pure-Rust extraction insufficient for LIN-01 |
| D20 | Web identity: **`stichos.dev` only**; no homepage planned; **skip `stichos.com`** | owner call 2026-09-08 (OQ5 domain part); `.com` parked at NameBright since 2016, no need to buy or monitor | a marketing site becomes a requirement |

---

## 17. Verification log (measured, not assumed)

| Claim | Method | Result |
|---|---|---|
| LibreOffice ships Windows ARM64 + both mac arches + deb/rpm both arches | directory listing of the stable download tree | `Win_x86-64.msi`, `Win_aarch64.msi` (+ `_sdk`), `MacOS_aarch64.dmg`, `MacOS_x86-64.dmg`, `deb/{x86_64,aarch64}`, `rpm/{x86_64,aarch64}` |
| Structured filter options work | `soffice --convert-to 'pdf:writer_pdf_Export:{"PageRange":…}'` | converted as specified |
| Rasterizing works without LO's own PNG path | `pdftoppm -r 100 -png` | 3 pages → PNG, 0.03 s |
| Latency budget for the loop | timed cold run | pandoc 0.29 s · soffice→pdf 1.88 s · pdftoppm 0.03 s · pdfminer 0.08 s |
| Metric-compatible metric-compatible font substitutes exist | `fc-match Calibri` / `fc-match Cambria` | → Carlito / Caladea (redistributable) |
| MCP results can carry images | spec schema (`ImageContent`, `CallToolResult.content`, `structuredContent`, `isError`) | confirmed in `schema.ts` |
| Vision cost model | current Claude vision docs | `⌈w/28⌉×⌈h/28⌉`; standard 1568 px / 1568 tokens; 4.7+ 2576 px / 4784; >20 images → stricter limits |
| Icon survives 16px | librsvg raster, sizes 16–256, plus 12× nearest-neighbour zoom | full detail fails at 16px; tight tier legible → tiers adopted |
| Wordmark geometry | PIL font metrics, Inter Medium | `y` span x 501.4–627.8, cap 168.0, descender 46.0 |

---

## 18. Open questions

1. ~~License split~~ **RESOLVED 2026-09-08 (G1, owner):** lopdf + pdf-writer pure-Rust core, qpdf shell-out;
   AGPL never in-process (ADR-0001 accepted; D19).
2. Word-rendering tier for non-Windows users: paid Aspose/Syncfusion path or a hosted Word worker?
3. Is a `gp` short binary alias wanted for the CLI, or `stichos` spelled out everywhere?
4. Distribution: build our own apt/rpm repos, or rely on GitHub Releases + winget/Homebrew only?
5. Trademark classes to file in (open — counsel). ~~whether to secure `stichos.com`~~ **RESOLVED 2026-09-08:**
   skip; no homepage planned, `stichos.dev` only (D20).

---

## 19. Next actions

1. ~~Stand up the fidelity harness (Phase 0) and the fixture corpus.~~ Done 2026-09-07 (runbook SA-1..SA-10); M0 gate closure pending human items + 2 CI bugs (`docs/phase0-execution.md` gate status).
2. Cargo workspace scaffold with engine trait + probe/`doctor` fixture probe.
3. `render_document` / `inspect_region` returning MCP image blocks + structured manifest.
4. Deterministic layout linter before any VLM-in-the-loop.
5. Windows COM worker behind the same engine trait, logon-session process model.
6. Name hygiene: trademark search, register registries/domain the same day (GitHub = private repo on owner's personal account, D18).
