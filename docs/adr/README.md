# ADR index

Naming: `ADR-NNNN-slug.md` — four digits, zero-padded, numbers never reused. Status flow:
**proposed → accepted | rejected**; only the project owner flips a status. New ADRs start from
[`ADR-0000-template.md`](ADR-0000-template.md) with the next free number.

| ADR | Title | Status | Date |
|---|---|---|---|
| [ADR-0006](ADR-0006-selfupdate-primitive.md) | Selfupdate primitive (DIS-05, G4/S4): `stichos update` opt-in channel | **accepted** | 2026-09-16 |
| [ADR-0005](ADR-0005-rhai-script-sandbox.md) | Rhai script sandbox contract (D17, API-09, C7): embedded escape hatch | **accepted** | 2026-09-16 |
| [ADR-0004](ADR-0004-typst-cli-shellout.md) | Typst engine = CLI shell-out (ENG-11, D3): corrected from "pure-Rust lib" | **proposed** | 2026-09-15 |
| [ADR-0001](ADR-0001-pdf-stack-g1.md) | PDF stack for G1 (OQ1): lopdf + pdf-writer pure-Rust core, qpdf shell-out rung, pdfium deferred to G5 (D19) | **accepted** | 2026-09-08 |
| [ADR-0000](ADR-0000-template.md) | Template — not a decision | template | — |
