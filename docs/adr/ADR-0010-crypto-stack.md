# ADR-0010: Pure-Rust Crypto Stack for PAdES-B-B Signing (M5 Wave T)

**Status**: proposed (2026-09-18, contributor) -> **accepted** (2026-09-18, owner)
**Decisions**: D25 (pure-Rust crypto for PAdES-B-B)

**Phase 7 update (2026-09-26):** Implementation uses manual DER assembly for PKCS#7
SignedData (owner OQ-C12, Option A — no `pkcs7` crate). Signed attributes
(content-type, message-digest, signing-time, signingCertificateV2) present.
`verify_signature()` provides independent verification. `pkcs7` crate dependency
removed (unused, deprecated).

## 1. Context

Phase 5 (M5) introduces the trust wave (Wave T) with signature integrity detection and
PAdES-B-B sign-later capability. D2 mandates that C stays out of the binary — no C libraries
linked into Rust crates. D4 mandates zero hard external dependencies for the Rust crates
beyond what is already present.

PAdES-B-B (ETSI EN 319 102-1 Baseline B-B) requires:
- RSA PKCS#1 v1.5 or ECDSA signatures over SHA-256
- X.509 certificate chain (self-signed is sufficient for B-B)
- PKCS#7/CAdES byte-range signature over the PDF
- TSA timestamp is **not required** for B-B (honest-absent)

The signing path uses the existing incremental writer: sign-later is the happy path;
post-sign mutations require explicit policy and are refused by default (GP-S002).

## 2. Options

### Option A: RustCrypto family (recommended)

Use only pure-Rust crates from the RustCrypto project:

| Crate | Version | License | Purpose |
|-------|---------|---------|---------|
| `rsa` | 0.9 | MIT/Apache-2.0 | RSA PKCS#1 v1.5 signing |
| `sha2` | 0.10 | MIT/Apache-2.0 | SHA-256 digest |
| `p384` | 0.13 | MIT/Apache-2.0 | ECDSA P-384 curves |
| `ecdsa` | 0.16 | MIT/Apache-2.0 | ECDSA signature scheme |
| `pkcs8` | 0.10 | MIT/Apache-2.0 | PKCS#8 key encoding |
| `x509-cert` | 0.2 | MIT/Apache-2.0 | X.509 certificate handling |
| `signature` | 2 | MIT/Apache-2.0 | Signature trait |
| `der` | 0.7 | MIT/Apache-2.0 | DER encoding |
| `const-oid` | 0.9 | MIT/Apache-2.0 | ASN.1 OID constants |
| `zeroize` | 1 | Apache-2.0 | Secure secret zeroization |

All crates are:
- Pure Rust (no `-sys` or C dependencies)
- Dual-licensed MIT/Apache-2.0 (compatible with project)
- Actively maintained
- Cargo-deny friendly

**What it costs:**
- ~200-300 additional dependency crates (mostly transitive)
- Build time increase for gp-core (~30s on cold cache)
- Larger binary size (~2-4 MB for crypto code)

### Option B: ring crate

Use `ring` for RSA/ECDSA + SHA-256.

**Rejected**: ring ships a complex build system with platform-specific assembly;
it has its own license (BSD-like, not MIT/Apache-2.0); it is not on the
cargo-deny allow-list. Ring also does not natively support PKCS#7 envelope
building — we would need to build the ASN.1 layer ourselves.

### Option C: openssl-sys

Use Rust bindings to system OpenSSL.

**Rejected**: violates D2 (C stays out); introduces a C-linked `-sys` dependency;
breaks cross-compilation targets in deny.toml.

### Option D: No signing in Rust

Defer signing to an external tool (qpdf, openssl CLI) and only implement
signature detection in Rust.

**Rejected**: D25 explicitly requires pure-Rust crypto for PAdES-B-B;
external tool dependency is less reliable (honest-absent paths are fine
for qpdf verification but not for signing which is a core capability).

## 3. Evidence

### Measured

| Item | Value |
|------|-------|
| Host: pandoc | 3.7.0.2 |
| Host: soffice | 26.2.6.1 |
| Host: pdftoppm | present |
| Host: qpdf | **absent** (CI installs) |
| Host: veraPDF | **absent** (CI installs) |
| Host: node | v22.23.1 + npm |
| Host: openssl | present (CLI, for verification only) |
| rustc | 1.98 (MSRV) |
| gp-core current deps | rsa 0.9, sha2 0.10, hmac 0.12, p384 0.13 already present |

### Researched

- ETSI EN 319 102-1 (PAdES) baseline: B-B requires signature policy, signer
  attributes, and a valid signature certificate (no trusted third-party validation
  required for B-B — just cryptographic validity)
- ISO 32000-2 §12.8 (digital signatures): byte-range signature covers data up to
  the signature value, which is a DER-encoded PKCS#7 SignedData
- PKCS#7/CAdES structure: SignedData contains digestAlgorithms, certificates,
  signerInfo with digestSignature over the byte-range content

### Source

- `crates/gp-core/Cargo.toml`: rsa, sha2, hmac, p384 already in [dependencies]
- RustSec advisory-db: no critical advisories for the selected crate versions
- ISO 32000-2 §12.8.3: PKCS#7 signature format for PDF digital signatures

## 4. Consequences

### Adopted: RustCrypto family

**What it locks in:**
- Only RustCrypto-family crates for crypto operations in gp-core
- `-sys` crypto crates banned in `deny.toml` (new deny entry)
- Signing path: self-signed PKCS#12 (generated in-memory) or provided PKCS#12
- Digest algorithm: SHA-256 only (no SHA-1, no SHA-512 — B-B minimum)
- Signature types: RSA PKCS#1 v1.5 and ECDSA P-384
- PKCS#7 envelope built with manual DER assembly (no `pkcs7` crate — deprecated/unused)
- X.509 certificate generated with `x509-cert` builder
- Honest-absent TSA: `--no-tsa` flag documented; TSA is post-B-B (PAdES-T)

**Refuse list (nothing below B-B is supported):**
- `GP-S-010`: PAdES-T / LTV signing refused (TSA unavailable for B-B)
- `GP-S-011`: Trusted certificate (CA-signed) signing refused (B-B accepts self-signed)
- `GP-S-012`: OCSP/CRL verification refused (not part of B-B)
- `GP-S-013`: Smart card / HSM signing refused (in-memory keys only)
- `GP-S-014`: PKCS#12 input required — raw PEM keys refused (use `openssl pkcs12` to convert)
- `GP-S-015`: Pre-existing `/AcroForm /SigFields` conflict — signing refused
- `GP-S-016`: Already-signed document — re-signing refused (GP-S002 with policy override)

**deny.toml change:**
```toml
[[bans.deny]]
crate = "ring"
reason = "D2/D25: ring banned — RustCrypto family only for crypto"
[[bans.deny]]
crate = "openssl"
reason = "D2/D25: openssl-sys banned — C dependency"
[[bans.deny]]
crate = "*-sys"
reason = "D2/D25: -sys crypto banned — C-linked crypto violates D2"
```

Note: The `*-sys` ban is a broad catch-all. The intent is to ban C-linked
cryptographic libraries specifically. The `lopdf` and `pdf-writer` crates
(which are already used for PDF parsing/writing) are not `-sys` crates and
are unaffected.

**C13 contract**: New contract (C13-signature-integrity.md) defines the
`SignatureInfo` model, signature states, and refusal semantics.

**C7 v1.4**: New MCP tool `sign_document` (v1.5.0) + CLI verb `sign`.

**Binary impact**: ~2-4 MB additional for crypto code; ~200-300 additional
transitive dependency crates (all pure-Rust).

**No impact on existing tools**: All existing 22 MCP tools and 20 CLI verbs
are unchanged. `sign_document` and `stichos sign` are additive.

## 5. Migration

- No migration needed: signing is a new capability. Existing documents are
  unaffected. The incremental writer already refuses writes to signed PDFs
  (PDF-E-006).

## 6. Related

- D2 (engines never linked): satisfied — RustCrypto crates are pure Rust
- D4 (zero hard deps for engines): not affected — signing is in gp-core,
  not in any engine crate
- D25 (pure-Rust crypto for PAdES-B-B): this ADR is the concrete implementation
- C13 (signature-integrity contract): defines the data model
- C7 v1.4/v1.5: new MCP tools for signing
- ADR-0001 (PDF stack): uses lopdf + pdf-writer; crypto is orthogonal
- Phase 5 plan §Wave T: P5-11 (ADR + model), P5-13 (signing implementation)
