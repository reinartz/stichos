# adapters-ooxml

Read, create, and surgically write OOXML (`.docx`, `.xlsx`, `.pptx`) packages for the stichos
toolchain.

[![crates.io](https://img.shields.io/crates/v/adapters-ooxml.svg)](https://crates.io/crates/adapters-ooxml) [![docs.rs](https://img.shields.io/docsrs/adapters-ooxml.svg)](https://docs.rs/adapters-ooxml)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Parses OPC packages (`opc::parse_opc`) and validates package integrity
  (`opc_validate::validate_package_bytes`) — dangling relationships, orphan parts, content-type
  coverage.
- Reads docx, xlsx, and pptx into the stichos IR (`docx::read_docx`, `xlsx::read_xlsx`,
  `pptx::read_pptx`, all returning `ReadResult` plus fidelity diagnostics).
- Creates new minimal Office documents from IR (`create::{create_minimal_docx, create_minimal_xlsx,
  create_minimal_pptx}`).
- Applies byte-surgical part mutations with `writer::SurgicalWriter`, preserving package and ZIP
  metadata for every untouched part.
- Reads and writes tracked changes (`revision_reader`/`revision_writer`) and document comments
  (`comment_writer`).
- Works as an in-process library with no mandatory external binaries.

## Install

```toml
[dependencies]
adapters-ooxml = "0.2"
```

No feature flags. It depends on `stichos-core` and `stichos-lint` transitively.

## Usage

Reading a document into IR and validating the package:

```rust
use adapters_ooxml::docx::read_docx;
use adapters_ooxml::opc::parse_opc;
use adapters_ooxml::opc_validate::validate_package_bytes;
use stichos_core::diagnostics::Diagnostic;

fn inspect(bytes: &[u8]) -> Result<(), Vec<Diagnostic>> {
    let package = parse_opc(bytes)?;
    let result = read_docx(&package)?;
    println!(
        "IR JSON: {} bytes; {} fidelity diagnostic(s)",
        result.ir.len(),
        result.diagnostics.len()
    );
    Ok(())
}

fn check_package(bytes: &[u8]) {
    for diag in validate_package_bytes(bytes) {
        eprintln!("{}: {}", diag.code, diag.message);
    }
}
```

Replacing one part while preserving everything else byte-for-byte:

```rust
use adapters_ooxml::writer::{PartMutation, SurgicalWriter};
use stichos_core::diagnostics::Diagnostic;
use std::collections::BTreeMap;

fn replace_part(
    source: &[u8],
    part_name: &str,
    new_xml: Vec<u8>,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let writer = SurgicalWriter::new(source);
    let mut mutations = BTreeMap::new();
    mutations.insert(
        part_name.to_string(),
        PartMutation::Replace {
            data: new_xml,
            content_type: None,
        },
    );
    Ok(writer.write(mutations)?.bytes)
}
```

## When to use

- **With `stichos-core`:** use the crate directly when you need to read or write Office files but
  manage your own IR pipeline; `ReadResult` carries serialized IR bytes ready for `stichos-core`
  types.
- Prefer this crate over the CLI when embedding document reads or surgical writes inside a larger
  Rust application.
- Choose it over a full Office library when you want format fidelity and byte-level preservation of
  untouched parts rather than a round-trip re-serialization.

## Limitations

- Uses the pure-Rust `zip` crate; it does not handle encrypted (password-protected) packages.
- `read_docx`/`read_xlsx`/`read_pptx` return serialized IR JSON rather than a live IR struct.
- `SurgicalWriter` operates at the OPC/XML part level; it does not lay out or paginate. Rendering is
  handled by the `engines` crate.
- Creation is intentionally minimal: `create_minimal_*` emits a valid but barebones package, not a
  full template.

## Related crates

- [`stichos-core`](https://crates.io/crates/stichos-core) — the IR, patch, and diagnostic types this
  crate produces.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — the ODF counterpart.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parsing and mutation.
- [`engines`](https://crates.io/crates/engines) — render engines for layout QA.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linting.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
