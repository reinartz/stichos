# stichos — Support Matrix

> **Note:** This document tracks engine support, distro compatibility, deprecation notices,
> and the evolving M4 phase-4 surface. It is a living reference — update it when tools/engines
> are added or retired.

## Engines (N/N-1 support)

| Engine | Supported versions | Notes |
|--------|-------------------|-------|
| LibreOffice | 24.2+ (current 26.2.6) | Headless mode; 26.2 confirmed on this host |
| Pandoc | 2.0+ (current 3.7.0) | Text extraction + creation; 3.7 confirmed |
| Chromium | Any (headless) | `--headless=new --print-to-pdf`; version probed |
| Typst | — | Not installed on this host; honest-absent path shipped |

## Distro Grid

| Distribution | Rust 1.85+ | LibreOffice | Pandoc | Chromium |
|-------------|-----------|-------------|--------|----------|
| Ubuntu 22.04 | ✓ | ✓ (PPA) | ✓ (APT) | ✓ (APT) |
| Ubuntu 24.04 | ✓ | ✓ (APT) | ✓ (APT) | ✓ (APT) |
| Fedora 39+ | ✓ | ✓ (DNF) | ✓ (DNF) | ✓ (DNF) |
| Debian 12+ | ✓ | ✓ (APT) | ✓ (APT) | ✓ (APT) |
| Alpine 3.19+ | ✓ | ✓ (APKG) | limited | limited |

## Deprecation Notices

- **M1 tool surface (v1.0.0):** Frozen. No removals planned; additive changes only.
- **Typst engine:** Marked as honest-absent (not installed); future implementation may depend on D15.
- **MS-Office COM engine:** Compile-only skeleton (ms-office feature flag); deferred until Windows host exists.
- **pdf.js:** Not in scope for M4 (D22 — raster overlay uses 150-DPI masters).

## Phase-4 Surface

- **20 MCP tools** (measured 2026-09-18: `mcp/spec.rs:824` asserts `TOOLS.len() == 20`): Added `list_revisions`, `accept_revisions`, `reject_revisions`, `add_comment`
- **19 CLI verbs** (measured from `enum Command` in `cli/src/main.rs`): Added `revisions`, `review`, `open`, `schema` (19 total: inspect, region, render, diff, doctor, serve, schema, apply, edit, annotate, revert, journal, perf, create, script, update, revisions, review, open)
- **Schema version:** 1.3.0 (additive-only since v1.0.0)
- **Contracts:** C1 (minor additive), C7 (v1.3.0), C11 (frozen at M4)
