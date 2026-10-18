# engines

Render-engine management for stichos: it creates and converts documents by spawning external engines
as subprocesses.

[![crates.io](https://img.shields.io/crates/v/engines.svg)](https://crates.io/crates/engines) [![docs.rs](https://img.shields.io/docsrs/engines.svg)](https://docs.rs/engines)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Creates documents from markdown/HTML/Typst sources (`creation::create_document`) and writes
  revision-tracked edits against an existing docx (`creation::create_as_revisions`).
- Negotiates a renderer from platform order and preference
  (`negotiate::{negotiate_renderer, negotiate_for_conversion, check_engine_available,
  RendererPreference}`) and builds a capability matrix (`capability::build_capability_matrix`).
- Probes the host and renders test fixtures (`doctor::{run_doctor, system_info,
  standard_doctor_engines}`, `probe`).
- Rasterizes PDF pages through a tool ladder (`raster::{rasterize, available_raster_tools}`).
- Wraps the concrete engine types — `LibreOfficeEngine`, `ChromiumEngine`, `PandocEngine`,
  `TypstEngine`, and `MockEngine` — all spawned as subprocesses, never linked.
- Caches rendered masters (`cache::MasterCache`).

## Install

```toml
[dependencies]
engines = "0.2"
```

Optional features:

```toml
[dependencies]
engines = { version = "0.2", features = ["pdfium"] }
```

- `default = ["unix"]`; `unix = ["dep:libc"]` enables Unix pipe handling for LibreOffice subprocess
  capture.
- `ms-office = []` enables the Windows COM automation path (a no-op stub off Windows).
- `pdfium = ["dep:pdfium-render", "dep:image"]` adds dynamic-link pdfium rasterization.

## Usage

Creating a document (delegated to an external engine at runtime):

```rust
use engines::creation::create_document;

fn create() -> Result<(), Vec<stichos_core::diagnostics::Diagnostic>> {
    let markdown = b"# Title\n\nHello from stichos.\n";
    let result = create_document(
        markdown,     // source bytes
        "markdown",   // source type
        "docx",       // target format
        None,         // optional template bytes
        false,        // dry_run
        None,         // optional existing-docx path (create-as-revisions)
        "agent",      // revision author
        false,        // direct (skip revision markup)
    )?;
    println!("created {}", result["document_id"]);
    Ok(())
}
```

Choosing a renderer without rendering:

```rust
use engines::negotiate::{negotiate_renderer, RendererPreference};

fn pick() {
    let choice = negotiate_renderer(&RendererPreference::Default);
    println!(
        "renderer: {} (available: {})",
        choice.engine_id, choice.available
    );
}
```

## When to use

- **With `stichos-core`:** take it when you need to render, convert, or create documents and can rely
  on host-installed engines.
- Use `MockEngine` plus `negotiate`/`doctor` to keep tests and capability decisions independent of
  installed software.
- Prefer it over calling `soffice`/`pandoc`/`chromium` directly when you want negotiation, caching,
  diagnostics, and renderer provenance.

## Limitations

- Actual rendering requires host-installed binaries (LibreOffice, Chromium, Pandoc, Typst; plus
  `pdftoppm`/`gs`/`mutool`/`soffice` for rasterization). All are soft-probed, so absent engines
  return diagnostics rather than failing at compile time.
- Markdown-to-PDF creation requires a LaTeX engine unless a chromium/LibreOffice fallback is
  installed.
- Engines are spawned as subprocesses and never linked in; there are no hard external build
  dependencies.
- `ms-office` COM automation only works on a Windows host; it is a stub elsewhere.

## Related crates

- [`stichos-core`](https://crates.io/crates/stichos-core) — the IR and diagnostic types used
  throughout.
- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — docx/xlsx/pptx creation and
  revision writing.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — odt/ods/odp read and write.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parsing and mutation.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linting over rendered output.
- [`stichos-mcp`](https://crates.io/crates/stichos-mcp) — MCP protocol server.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
