# adapters-pdf

Parse PDF documents and apply append-only incremental mutations for the stichos toolchain.

[![crates.io](https://img.shields.io/crates/v/adapters-pdf.svg)](https://crates.io/crates/adapters-pdf) [![docs.rs](https://img.shields.io/docsrs/adapters-pdf.svg)](https://docs.rs/adapters-pdf)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Parses PDF objects, xref tables, and the page tree (`parser::{parse_pdf_data, parse_pdf,
  page_count, PdfReader}`).
- Extracts text with word-level bounding boxes (`text::{extract_text_bytes,
  extract_text_with_bboxes}`).
- Extracts and censuses annotations (`annot::{extract_annotations, census_annotations}`), AcroForm
  fields and XFA detection (`form::{parse_acroform, detect_xra}`), outlines (`outline`), and XMP
  metadata (`metadata`).
- Writes annotations and form values back with an append-only incremental writer
  (`incremental::{IncrementalWriter, PdfMutation, PdfMutationSet, add_annotation}`,
  `annot_write::{apply_annot_ops, AnnotOp}`), and supports content redaction (`redact`).
- Optionally validates a file with the external `qpdf --check` (`qpdf::{check_pdf,
  is_qpdf_available}`).
- Detects existing digital signatures so mutations can refuse rather than invalidate them.

## Install

```toml
[dependencies]
adapters-pdf = "0.2"
# adapters-pdf leaks `lopdf` 0.45 types in its public API; pin it yourself.
lopdf = "0.45"
```

No feature flags. `qpdf` is an optional external binary, not a Rust dependency.

## Usage

Reading pages and extracting positioned text:

```rust
use adapters_pdf::parser::{page_count, parse_pdf_data};
use adapters_pdf::text::extract_text_bytes;
use stichos_core::diagnostics::Diagnostic;

fn inspect(bytes: &[u8]) -> Result<(), Vec<Diagnostic>> {
    println!("{} page(s)", page_count(bytes)?);

    let doc = parse_pdf_data(bytes)?;
    println!("parsed {} page(s)", doc.pages.len());

    for word in extract_text_bytes(bytes)? {
        println!("{} @ {:?}", word.text, word.bbox);
    }
    Ok(())
}
```

Appending an annotation with an incremental update:

```rust
use adapters_pdf::incremental::{add_annotation, IncrementalWriter, PdfMutationSet};
use stichos_core::diagnostics::Diagnostic;

fn add_note(source: &[u8]) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut set = PdfMutationSet::default();
    set.mutations.push(add_annotation(
        (100, 0),                         // object number, generation
        1,                                // page (1-indexed)
        "Text",                           // annotation subtype
        [72.0, 700.0, 200.0, 720.0],      // rect in PDF points
        Some("Reviewed by stichos".into()),
    ));

    let writer = IncrementalWriter::new(source);
    writer.check_writable()?;
    Ok(writer.write(&set)?.bytes)
}
```

## When to use

- **With `stichos-core`:** use it when you need to inspect PDF structure, text, annotations, or forms
  and feed the result into stichos IR, patching, or review flows.
- Prefer it over a general PDF library when you need byte-level, append-only mutation that never
  rewrites the original objects.
- Pair it with the `engines` crate when you need pagination or rasterization; this crate does not
  render.

## Limitations

- The public API leaks `lopdf` 0.45 types: `incremental::BTreeVec` is `BTreeMap<Vec<u8>,
  lopdf::Object>`, and `PdfMutation` variants and `add_annotation` use `lopdf::Object`. `lopdf` is
  **not** re-exported, so consumers must add `lopdf = "0.45"` to their own manifest.
- The incremental writer is append-only and refuses PDFs that use xref streams or object streams, or
  that are encrypted.
- Mutating a signed PDF is refused by default; callers may override explicitly, which records a
  signature-invalidation diagnostic.
- `qpdf::{check_pdf, is_qpdf_available}` require the `qpdf` binary; text/annotation/outline reading
  is pure Rust.
- No rendering or pagination: output is a parsed IR and/or mutated bytes, not page images.

## Related crates

- [`stichos-core`](https://crates.io/crates/stichos-core) — the IR, annotation model, and signature
  types used here.
- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — Office (OOXML) counterpart.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — ODF counterpart.
- [`engines`](https://crates.io/crates/engines) — render engines for page images and layout QA.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linting.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
