# stichos-lint

Fidelity linting and QA metrics over rendered pages and stichos IR for the stichos toolchain.

[![crates.io](https://img.shields.io/crates/v/stichos-lint.svg)](https://crates.io/crates/stichos-lint) [![docs.rs](https://img.shields.io/docsrs/stichos-lint.svg)](https://docs.rs/stichos-lint)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Runs a layout linter (`linter::LayoutLinter`, implementing `gp_core::traits::{Linter,
  BatchLinter}`) that flags page overflow, off-page elements, negative coordinates, overlaps, and
  autofit-shrink risks.
- Measures structural similarity between 8-bit grayscale rasters
  (`ssim::{ssim, compare, rgb_to_grayscale}`) with an optional threshold check.
- Enforces the vision-payload budget
  (`budget::{compute_visual_tokens, check_payload_budget, enforce_budget, ImageBlock}`): at most 20
  image blocks, at most 2000 px per side, token estimate `⌈w/28⌉ × ⌈h/28⌉`.
- Checks WCAG contrast (`contrast::luminance_contrast_ratio`) and font problems
  (`font::{detect_substitutions, detect_tofu_risks}`).
- Converts points to pixels (`coords::CoordContext`) and crops/zooms token-budgeted regions
  (`cropper::crop_region`).
- Diffs IR documents and XML parts (`diff::{diff_ir, diff_xml_parts}`), builds page contact sheets
  (`sheet::build_sheet_from_pages`), encodes images (`encode::encode_image`), and annotates pages
  with numbered lint boxes (`annotator::{annotate_pages, build_legend}`).
- Optionally shells out to external validators (`validate::validate_xml_schema`, `a11y`,
  `verapdf`).

## Install

```toml
[dependencies]
stichos-lint = "0.2"
```

No feature flags. Pure metrics (SSIM, budget, contrast, font, coordinates, crop, encode) need no
external binaries; the `a11y`, `verapdf`, and `validate` paths shell out (see Limitations). Depends
on `image` 0.25 and `stichos-core`.

## Usage

Comparing two rendered page rasters and enforcing the payload budget:

```rust
use stichos_core::diagnostics::Diagnostic;
use stichos_lint::budget::{enforce_budget, ImageBlock};
use stichos_lint::coords::CoordContext;
use stichos_lint::ssim::compare;

fn qa(before: &[u8], after: &[u8]) -> Result<(), Diagnostic> {
    // Both buffers are 8-bit grayscale page rasters of the same page.
    let score = compare(before, after, 1275, 1650, 1275, 1650);
    println!("SSIM: {:.4}", score);

    // Map a point-space bbox to pixels at the render DPI.
    let ctx = CoordContext::new(150.0, 1275, 1650);
    let bbox_px = ctx.bbox_pt_to_px([72.0, 72.0, 200.0, 50.0]);
    println!("bbox px: {:?}", bbox_px);

    // Enforce the payload budget on image blocks bound for a model context.
    let blocks = vec![ImageBlock::new(1275, 1650, 90_000, "jpeg".into())];
    let report = enforce_budget(blocks)?;
    println!(
        "{} tokens, within budget: {}",
        report.total_tokens, report.within_budget
    );
    Ok(())
}
```

Linting laid-out pages:

```rust
use stichos_core::traits::{Linter, LinterPage};
use stichos_lint::linter::LayoutLinter;

fn lint_pages(pages: &[LinterPage]) {
    let diagnostics = LayoutLinter.lint(&[], pages);
    for diag in &diagnostics {
        println!("{}: {}", diag.code, diag.message);
    }
}
```

## When to use

- **With `stichos-core`:** it consumes `IRDocument`s and bounding boxes and emits
  `stichos_core::diagnostics::Diagnostic`, so it slots into the same pipeline as the adapters and
  `engines`.
- Use it when you already have rendered page rasters or IR and need objective QA (SSIM, layout,
  budget, contrast, fonts) before publishing.
- Reach for the pure metrics when you cannot install external binaries; only `validate`, `a11y`, and
  `verapdf` require them.

## Limitations

- `a11y`, `verapdf`, and `validate` shell out to external binaries (`java` + `verapdf`, `xmllint`,
  `qpdf`). The pure metrics need none, and missing tools degrade to skipped checks.
- It does not render: it consumes rasters, bounding boxes, and IR produced by `engines` and the
  adapters.
- `ssim` compares 8-bit single-channel grayscale buffers and trims both images to the common
  per-axis minimum dimensions.
- Outputs are `stichos_core::diagnostics::Diagnostic`; the crate is meant to be used alongside
  `stichos-core`.

## Related crates

- [`stichos-core`](https://crates.io/crates/stichos-core) — the IR, bbox, and diagnostic types this
  crate consumes and produces.
- [`engines`](https://crates.io/crates/engines) — renders the pages and PDFs that feed these
  metrics.
- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — docx/xlsx/pptx read and write.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — odt/ods/odp read and write.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parsing and mutation.
- [`stichos-mcp`](https://crates.io/crates/stichos-mcp) — MCP protocol server.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
