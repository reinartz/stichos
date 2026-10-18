# adapters-odf

Read and surgically write ODF (`.odt`, `.ods`, `.odp`) packages for the stichos toolchain.

[![crates.io](https://img.shields.io/crates/v/adapters-odf.svg)](https://crates.io/crates/adapters-odf) [![docs.rs](https://img.shields.io/docsrs/adapters-odf.svg)](https://docs.rs/adapters-odf)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Parses ODF ZIP packages and their `META-INF/manifest.xml` (`manifest::{parse_odf, parse_manifest}`)
  and flags encrypted packages (`manifest::detect_encryption`, flag only).
- Reads odt, ods, and odp into the stichos IR (`odt::read_odt`, `ods::read_ods`, `odp::read_odp`),
  each returning `ReadResult` plus diagnostics.
- Applies byte-surgical part mutations with `writer::SurgicalWriter`, preserving each untouched part
  and keeping `mimetype` first and uncompressed.
- Reads and writes ODF tracked changes (`revision_writer::{RevisionWriter, create_as_revisions}`,
  `revision_reader::enumerate_revisions`) and comments (`comment_writer`).
- Optionally validates parts against the ODF RelaxNG schemas via `rng_validate` when `xmllint` is
  present on the host.

## Install

```toml
[dependencies]
adapters-odf = "0.2"
```

No feature flags. It depends on `stichos-core` transitively.

## Usage

Reading a document into IR:

```rust
use adapters_odf::manifest::parse_odf;
use adapters_odf::odt::read_odt;
use stichos_core::diagnostics::Diagnostic;

fn inspect(bytes: &[u8]) -> Result<(), Vec<Diagnostic>> {
    let package = parse_odf(bytes)?;
    let result = read_odt(&package)?;
    println!(
        "IR JSON: {} bytes; {} diagnostic(s)",
        result.ir.len(),
        result.diagnostics.len()
    );
    Ok(())
}
```

Writing an edit as tracked revisions:

```rust
use adapters_odf::revision_writer::{create_as_revisions, RevisionMeta};
use stichos_core::diagnostics::Diagnostic;
use stichos_core::patch::PatchOp;

fn tracked_edit(source: &[u8], ops: &[PatchOp]) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let meta = RevisionMeta::new("Ada Lovelace", "2026-10-18T12:00:00Z");
    // `false` wraps edits in text:inserted / text:deleted markup.
    let result = create_as_revisions(source, ops, meta, false)?;
    Ok(result.output)
}
```

## When to use

- **With `stichos-core`:** take this crate when you need to read or mutate ODF files and already
  work with stichos IR and `PatchOp`.
- Prefer it over generic ZIP/XML libraries when you need ODF structural rules (manifest handling,
  `mimetype` ordering) and tracked-change markup preserved.
- Use it alongside `adapters-ooxml` when your tool must accept both LibreOffice and Office formats.

## Limitations

- Reading is more complete than writing; creation of ODF documents is not offered here (only
  mutation of existing packages).
- Encrypted ODF packages are detected but not decrypted.
- `rng_validate` requires the external `xmllint` binary; without it validation is skipped.
- Reading returns serialized IR JSON rather than a live IR struct.

## Related crates

- [`stichos-core`](https://crates.io/crates/stichos-core) — the IR, patch, and diagnostic types this
  crate produces.
- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — the OOXML counterpart.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parsing and mutation.
- [`engines`](https://crates.io/crates/engines) — render engines for layout QA.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linting.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
