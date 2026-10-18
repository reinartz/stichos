# stichos

The `stichos` command-line tool — inspect, render, diff, create, and surgically edit Office and PDF
documents.

[![crates.io](https://img.shields.io/crates/v/stichos.svg)](https://crates.io/crates/stichos) [![docs.rs](https://img.shields.io/docsrs/stichos.svg)](https://docs.rs/stichos)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Exposes 24 verbs: `inspect region render diff doctor serve schema apply edit annotate revert
  journal perf create script update revisions annots review open sign verify validate finish`.
- Inspects documents for structure, element IDs, coordinates, and a fidelity budget
  (`inspect`, `region`).
- Renders documents to page images and runs layout QA (`render`, `annotate`).
- Applies surgical edits as tracked revisions, or directly with `--direct`, and keeps a revertible
  edit history (`apply`, `edit`, `revert`, `journal`).
- Creates new documents from markdown/HTML/Typst (`create`), compares versions (`diff`), and checks
  OPC package integrity (`validate`).
- Serves the stichos tool surface to agents over MCP (`serve`) and covers review, signing, and
  verification flows (`review`, `annots`, `sign`, `verify`, `finish`).

## Install

```bash
cargo install stichos
```

Or build the workspace in-tree:

```bash
cargo build --release -p stichos    # → target/release/stichos
```

## Usage

```bash
stichos doctor
stichos inspect report.docx --json
stichos render report.docx --output ./out --dpi 150
stichos region report.docx --page 1 --bbox "72,72,200,50" --zoom 2.0
stichos diff v1.docx v2.docx --json
stichos edit --kind text report.docx --element-id id-1a2b3c4d5e6f \
    --old-text "Draft" --new-text "Final"
stichos apply report.docx --apply \
    --ops '[{"op":"replace_text","elem":"id-1a2b3c4d5e6f","old":"Draft","new":"Final"}]'
stichos revert report.docx --steps 1
stichos create --source report.md --format docx --source-type markdown
stichos validate report.pptx
stichos serve --transport stdio
```

Previews are the default for `apply`/`edit`; pass `--apply` to write, and `--direct` to skip
tracked-revision markup.

## When to use

- Use it as a standalone application, or as the reference wiring for the stichos toolchain.
- Use it to drive document workflows end to end (inspect → edit → render → validate) without writing
  Rust.
- Use `serve` to expose the same capabilities to LLM agents over MCP.
- Embed the workspace libraries instead when you need stichos inside a larger Rust program.

## Limitations

- Binary only: there is no library target, so `stichos` cannot be used as a Rust dependency. Depend
  on the workspace libraries instead.
- `render` and `create` require the external engines from the `engines` crate at runtime.
- Format coverage varies by verb: tracked revisions are docx/odt-specific, and `finish` requires
  PowerPoint COM on a Windows host.
- `perf` measures edit→render→look latency against a corpus subset and is intended for CI gating,
  not general use.

## Related crates

The `stichos` binary is the reference wiring for the toolchain; its reusable pieces are published as
separate crates:

- [`stichos-core`](https://crates.io/crates/stichos-core) — IR, patch/diff, content store, journal,
  diagnostics, and signing.
- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — docx/xlsx/pptx read and write.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — odt/ods/odp read and write.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parsing and mutation.
- [`engines`](https://crates.io/crates/engines) — render-engine subprocess management.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linting and QA metrics.
- [`stichos-mcp`](https://crates.io/crates/stichos-mcp) — MCP transport, dispatch, and review
  sessions.

Prefer those libraries for embedding; use this binary for the complete command surface. See the
[workspace README](https://github.com/reinartz/stichos#readme) and
[`docs/`](https://github.com/reinartz/stichos/tree/main/docs) for full command documentation.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
