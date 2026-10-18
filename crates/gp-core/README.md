# stichos-core

Core IR, patch/diff, content store, journal, diagnostics, and signing primitives shared by every
stichos component.

[![crates.io](https://img.shields.io/crates/v/stichos-core.svg)](https://crates.io/crates/stichos-core) [![docs.rs](https://img.shields.io/docsrs/stichos-core.svg)](https://docs.rs/stichos-core)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Defines the tagged intermediate representation (`ir`) for Word-, spreadsheet-, presentation- and
  PDF-like documents, including content-addressed `ElementId`s, annotations, and signature metadata.
- Applies patch operations (`patch::{PatchOp, apply_op}`) and reports the changed element IDs plus a
  fidelity budget.
- Diffs two IR documents (`diff::diff_documents`) and round-trips patches to and from JSON.
- Stores blobs and document versions in a content-addressed on-disk store (`store::ContentStore`)
  and records an auditable edit chain (`journal::Journal`) with pre/post hashes.
- Defines the trait surface that adapters, engines, and linters implement
  (`traits::{Codec, Engine, EngineExt, Linter, PatchExecutor, Probe, ...}`).
- Ships PDF signing and verification (`signature`), a Rhai script sandbox (`script`), a formula
  evaluator (`formula`), structured diagnostics (`diagnostics`), and context-budget response
  envelopes (`envelope`).

## Install

```toml
[dependencies]
stichos-core = "0.2"
```

Optional features:

```toml
[dependencies]
stichos-core = { version = "0.2", features = ["pades-ltv"] }
```

- `pades-ltv` enables PAdES long-term-validation helpers (`cert_chain`, `crl_client`, `ltv`,
  `ocsp_client`, `pkcs12`).
- `openssl-ci` is a CI-only compatibility flag.

## Usage

```rust
use stichos_core::diff::diff_documents;
use stichos_core::ir::{ElementId, IRDocument};
use stichos_core::patch::{apply_op, op_from_json, op_to_json, PatchOp};

fn replace_text(
    ir: &mut IRDocument,
    id: &str,
    old: &str,
    new: &str,
) -> Result<(), Vec<stichos_core::diagnostics::Diagnostic>> {
    let op = PatchOp::ReplaceText {
        elem: ElementId::from_str(id),
        old: old.to_string(),
        new: new.to_string(),
    };

    // Patches are JSON-serializable for transport between processes.
    let wire = op_to_json(&op).expect("serialize patch");
    let decoded = op_from_json(wire.as_bytes()).expect("deserialize patch");

    let result = apply_op(ir, &decoded)?;
    for changed in &result.changed_element_ids {
        println!("changed: {}", changed.as_str());
    }
    Ok(())
}

fn summarize_change(before: &IRDocument, after: &IRDocument) {
    let diff = diff_documents(before, after);
    println!("{} hunk(s)", diff.hunks.len());
}
```

Recording an edit in the content-addressed journal:

```rust
use stichos_core::journal::{Journal, JournalAction, XmlDiff};
use stichos_core::store::ContentStore;

fn record(before: &[u8], after: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let store = ContentStore::open()?;
    let mut journal = Journal::new(store, "session-1".to_string());

    match journal.commit(
        JournalAction::Replace,
        before,
        after,
        "1.0.0".to_string(),
        Vec::<XmlDiff>::new(),
    ) {
        Ok(entry_id) => println!("journal entry {entry_id}"),
        Err(diags) => eprintln!("commit refused: {diags:?}"),
    }
    Ok(())
}
```

## When to use

- **Standalone:** the IR model, patch engine, diff, store, and journal are usable without any
  document adapter, for tooling that only needs to manipulate stichos IR.
- **With adapters:** every `adapters-*` crate depends on `stichos-core`; this is where their shared
  types (`IRDocument`, `ReadResult`, `Diagnostic`, `PatchOp`, `FidelityBudget`) live.
- Reach for it directly when you need the content-addressed store, the edit journal, PDF
  signing/verification, or the Rhai script sandbox without pulling in an adapter.

## Limitations

- The `pades-ltv` feature pulls a heavy RustCrypto dependency tree; leave it off if you do not need
  long-term validation.
- The `pades-ltv`-gated `ltv` module operates on `lopdf::Object` but does **not** re-export `lopdf`,
  so consumers of that module must pin `lopdf = "0.45"` themselves.
- The Rhai sandbox and formula evaluator are deliberately scoped subsets, not full language or
  spreadsheet engines; unsupported constructs return explicit refusal diagnostics.
- PDF signing produces PAdES-B-B sign-later output only; it does not itself apply a visible
  appearance stream.

## Related crates

- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — docx/xlsx/pptx read and write.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — odt/ods/odp read and write.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parse and incremental mutation.
- [`engines`](https://crates.io/crates/engines) — render-engine adapters.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linters.
- [`stichos-mcp`](https://crates.io/crates/stichos-mcp) — MCP protocol server.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
