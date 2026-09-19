# Security Policy

## Supported Versions

Only the latest release is supported. As of M5, stichos has no public
release — security patches apply to the `main` branch and are shipped
with the next milestone tag.

## Reporting a Vulnerability

We take security seriously. If you discover a security vulnerability in
stichos, please report it responsibly.

1. **Do not** open a public GitHub issue for security-sensitive findings.
2. Email **orei+security@proton.me** with a description of the issue,
   steps to reproduce, and your assessed impact.
3. We aim to acknowledge receipt within **48 hours** and provide a
   preliminary assessment within **one week**.
4. We will credit reporters by default unless they wish to remain
   anonymous.

### What to report

- Remote code execution via document parsing (OOXML, ODF, PDF)
- Signature bypass (PAdES/B-B tamper-evidence)
- XSS vectors in the review session text layer
- Path traversal or zip-bomb in OPC/OPF archives
- Unsafe `cargo` dependency supply-chain attacks
- Rhai sandbox escapes (D17/ADR-0005)

### What not to report

- Issues already disclosed publicly (CVEs with patches, public PoCs)
- Issues in vendored dependencies that are already fixed upstream
- Issues in the host environment (OS, browser, LibreOffice, pandoc)

## Security Features

| Feature | Description |
|---------|-------------|
| Incremental writer (INV-1) | Append-only; never modifies original bytes |
| Content-addressed store | Immutable blob storage, no in-place edits |
| Journal (C6) | Every mutation journaled; revert restores byte-identical original |
| Taint propagation (C2) | Hostile document content flagged in IR |
| Signature detection (C13) | Pre-mutation `/Sig` presence → refusal by default (GP-S-001) |
| CSP (review session) | `script-src 'self'`; text layer uses `textContent` only |
| Vendored pdf.js | Hash-pinned (HASHES.toml), no CDN, opt-in only |
| Rhai sandbox (D17) | Bounded CPU/memory; no FFI; no filesystem/network |
| `cargo deny` | License allow-list; `-sys` crate ban; `-sys` crypto ban (D25) |

## Hunting & Testing

Run fuzz regressions from the `adapters-ooxfuzz` crate:

```
cargo test --package adapters-ooxfuzz --lib
```

Fuzz round 4 (P5-17) covers hostile annotation dictionaries, malformed
signature/byte-range structures, text-layer XSS strings, and
vendored-asset path attacks.

## Supply Chain

- stichos has **zero hard external dependencies** in the compiled binary
  (engines are spawned subprocesses — LibreOffice, pandoc, Chromium,
  Typst).
- Workspace `deny.toml` enforces the license allow-list and bans the
  `typst` crate.
- Vendored artifacts (pdf.js) are pinned by SHA-256 in
  `assets/vendor/pdfjs/HASHES.toml`, verified by the gate script.
- Dev-only harness deps (pinned `harness/requirements-dev.txt`) never
  ship.
