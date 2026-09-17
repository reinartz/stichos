# ADR index

Naming: `ADR-NNNN-slug.md` — four digits, zero-padded, numbers never reused. Status flow:
**proposed → accepted | rejected**; only the project owner flips a status. New ADRs start from
[`ADR-0000-template.md`](ADR-0000-template.md) with the next free number.

| ADR | Title | Status | Date |
|---|---|---|---|
| [ADR-0006](ADR-0006-selfupdate-primitive.md) | Selfupdate primitive (DIS-05, G4/S4): `stichos update` opt-in channel | **accepted** | 2026-09-16 |
| [ADR-0005](ADR-0005-rhai-script-sandbox.md) | Rhai script sandbox contract (D17, API-09, C7): embedded escape hatch | **accepted** | 2026-09-16 |
| [ADR-0004](ADR-0004-typst-cli-shellout.md) | Typst engine = CLI shell-out (ENG-11, D3): corrected from "pure-Rust lib" | **accepted** (owner 2026-09-16) | 2026-09-15 |
| [ADR-0001](ADR-0001-pdf-stack-g1.md) | PDF stack for G1 (OQ1): lopdf + pdf-writer pure-Rust core, qpdf shell-out rung, pdfium deferred to G5 (D19) | **accepted** | 2026-09-08 |
| [ADR-0000](ADR-0000-template.md) | Template — not a decision | template | — |

## Legacy (pre-convention filenames, not renumbered)

These predate the `ADR-NNNN` index convention and reuse numbers 0001–0003; kept as-is for the
honest trail. Do not cite them as `ADR-000N` — cite by filename.

| File | Title | Status | Date |
|---|---|---|---|
| [adr-0001-element-ids.md](adr-0001-element-ids.md) | Content-anchored element IDs (C2 design) | accepted | 2026-09-09 |
| [adr-0002-element-ids-risk.md](adr-0002-element-ids-risk.md) | Element ID risk register | living register | 2026-09-09 |
| [adr-0003-mcp-transport.md](adr-0003-mcp-transport.md) | MCP transport: custom JSON-RPC vs rmcp | accepted | 2026-09-10 |
