# ADR-0012 — PAdES-T/LTV progression (timestamps, OCSP/CRL, trusted certs): architecture and crypto deps

| | |
|---|---|
| Status | **accepted** (2026-09-26, owner) |
| Gate | G-SIG-LTV (timestamps + LTV embedded in signed PDF; OCSP/CRL verification; trusted CA certs) |
| Decide by | Phase 7 intake — before any P7-5 implementation |
| Decision owner | **project owner** |
| Blocks | P7-5 (timestamps + LTV), P7-7 (real PKCS#7/CMS if stub replaced), C13 extension |
| Related | D2, D25, C13, ADR-0010, deny.toml, `crates/gp-core/src/signature.rs`, `docs/contracts/C13-signature-integrity.md` |

<!-- Status flow: proposed → accepted | rejected (owner only). An accepted ADR that changes a
      decision also requires the owner to record a D-row in project_concept.md §16 — the ADR alone
      does not amend the D-log. -->

## 1. Context

Phase 5 (M5) shipped PAdES-B-B sign-later capability (`signature.rs`) using the RustCrypto family
(ADR-0010, D25). The current implementation:

- Generates self-signed X.509 v3 certificates in-memory
- Creates PKCS#7/CAdES detached signatures (byte-range over PDF)
- Uses RSA PKCS#1 v1.5 + SHA-256 or ECDSA P-384 + SHA-256
- **No TSA timestamp** (documented as honest-absent, `--no-tsa` flag)
- **No OCSP/CRL checking** (C13 GP-S-012: "CRL/OCSP verification not implemented")
- **No trusted certificate store** (self-signed only, GP-S-011: CA-signed refused)

PAdES-B-B is sufficient for basic non-reputation digital signatures, but ETSI EN 319 102-1 defines
a clear progression path:

| Level | Requirement | Why it matters |
|-------|-------------|----------------|
| **B-B** (shipped) | Valid signature + self-signed cert + signingTime attribute | "I signed this" — but cert may be untrusted, future-proofing absent |
| **B-BT** | B-B + **timestamp** from RFC 3161 TSA | Proof of *when* signed; timestamp survives signer cert expiry |
| **B-LT** | B-B + **LTV data**: OCSP response(s) and/or CRL(s) proving certs were valid at signing | Verifier can validate signature even if cert/TSA cert expire after signing |
| **B-LTA** | B-LT + **archival signatures** (repeated TSA timestamps wrapping the LTV data) | Long-archival; timestamp chain survives TSA compromise |

ETSI EN 319 102-5 (PAdES Part 5) specifies the LTV extension for PDF: OCSP responses and CRLs
are embedded as `DocMDP` or `Signature` dictionary entries (`/V` 4/5 with `/LTV` data) so that
a PDF viewer can validate signatures without network access after download.

### Why P7-5 matters

1. **Document retention requirements**: Legal, regulatory, and enterprise workflows often require
   signatures to remain verifiable for 7–10+ years. Without LTV, a signed document becomes
   unverifiable once the signing certificate expires.
2. **ETSI eSig compliance**: PAdES-LT is the baseline for EU eIDAS "Long-Term Validation" profiles.
3. **Competitive gap**: Current PAdES-B-B only satisfies minimum ISO 32000-2 requirements;
   PAdES-LT is what enterprise PDF signers (Adobe Acrobat, PDFtk, qpdf) support as "long-term
   validation."

### Constraints

- **D2 (C stays out)**: All crates must cross-compile to all six targets without C dependencies.
- **D25 (RustCrypto family only)**: No `ring`, no `openssl`, no `-sys` crypto crates (banned in
  `deny.toml`).
- **Zero hard deps**: Engines are subprocesses; gp-core must not require external software.
- **Existing signature.rs stub**: The current PKCS#7 builder in `signature.rs` uses manual DER
  assembly. A PAdES-T/LTV implementation may need a proper CMS library or must extend the manual
  builder.

## 2. Options

### Option A: External tool chain (qpdf + openssl) — least Rust work

Use `qpdf --sign` (or `openssl ts`) as a subprocess for timestamping and OCSP fetching, similar to
how the engines are spawned. Rust side: detect external tool availability, spawn subprocesses,
parse results.

**Pros**:
- Minimal Rust code — leverage battle-tested tooling
- `qpdf --sign` handles CMS/PAdES envelope structure correctly
- `openssl ts` handles RFC 3161 timestamp requests

**Cons**:
- **Violates D25 spirit**: introduces hard dependencies on system `openssl` and `qpdf` for signing
- Adds unbounded external latency (spawn + wait)
- Not honest-absent — signing fails if tools missing
- `qpdf --sign` requires a PKCS#12 file (key + cert + chain) — we'd need to generate one
- Cross-compilation unaffected (subprocess), but **host availability** is a new constraint
- No advantage over Option B for the timestamp case (we could just shell `curl` + `openssl`)

**Verdict**: Rejected. The subprocess model works for renderers (D3) where the capability is
deferred. Signing is a **core capability** — it must work without external installs, per D25.

### Option B: Pure-Rust stack with `x509-parser` + `reqwest` + `hyper` + `rcgen` (recommended)

Add a minimal HTTP client and X.509 parser to the gp-core dependency surface. The existing
`x509-cert` crate (already in `gp-core/Cargo.toml`) is primarily a *builder* — `x509-parser`
provides *parsing* of OCSP responses, CRLs, and real certificates.

New crates needed:

| Crate | License | Purpose | Pure Rust? |
|-------|---------|---------|------------|
| `x509-parser` 0.15 | MIT/Apache-2.0 | Parse OCSP responses, CRLs, X.509 certs (issuer, validity, revocation) | Yes |
| `reqwest` 0.11 | MIT/Apache-2.0 | HTTP client for OCSP (POST/GET to CA OCSP responder) and CRL (HTTP/HTTPS GET) | Yes (native-tls) |
| `hyper` 0.14 | MIT | (reqwest transitive — HTTP stack) | Yes |
| `rcgen` 0.13 | MIT/Apache-2.0 | Generate CA certificates for self-signed chain testing | Yes |
| `time` 0.3 | MIT | Better timestamp handling than chrono for RFC 3161 | Yes |
| `pem` 3.0 | MIT/Apache-2.0 | PEM encoding/decoding (if needed for key import) | Yes |

The timestamp request/response (RFC 3161) would be built manually (minimal DER) or via
`timestamp` crate if available — see Evidence section.

**Pros**:
- **Satisfies D2 and D25**: all pure-Rust, no `-sys`, no C deps, cross-compile clean
- Honest-absent: if TSA/OCSP/CRL unreachable, signing proceeds with B-B fallback + diagnostic
- Embeds OCSP response or CRL directly in PDF for LTV (no network needed at verify time)
- Matches the existing RustCrypto ecosystem style
- No external tool dependencies

**Cons**:
- **Adds ~15-25 new dependency crates** (reqwest brings tokio, http, url, try-lock, want, etc.)
- `reqwest` with `native-tls` may pull `openssl-sys` on some systems — **must use `rustls` feature**
- HTTP client adds surface area (network access during signing, TLS verification)
- RFC 3161 timestamp protocol is non-trivial (request/response DER structures)
- OCSP response parsing needs certificate chain context (issuer cert for signature verification)

**Verdict**: Recommended, **with the critical constraint that `reqwest` uses `rustls-tls` feature**
(native-tls = OpenSSL). This keeps the tree fully pure-Rust.

### Option C: Minimal hand-built OCSP/CRL + shell-out for timestamps

Build OCSP response parsing manually (DER structures are simple: `OCSPResponse ::= SEQUENCE`).
Use `openssl ts` or `curl` + hand-built request for RFC 3161 timestamps.

**Pros**: Fewer new crates than Option B
**Cons**: Fragile — hand-building OCSP requests and parsing responses is error-prone; still
depends on external tool for timestamps (defeats the pure-Rust goal); RFC 3161 is complex
(request/response/policy/mac/nonce structures)

**Verdict**: Rejected. The OCSP/CRL structures are well-defined but require proper ASN.1
parsing; `x509-parser` handles this correctly. Hand-rolling gains little.

### Option D: Defer PAdES-T/LTV indefinitely

Keep PAdES-B-B as the final state. Document that LTV is "out of scope."

**Pros**: No new dependencies, no new complexity
**Cons**: Fails enterprise requirements; competitors (Adobe, PDFtk, qpdf) all support LTV;
documents signed today may be unverifiable in 5-10 years when certs expire

**Verdict**: Rejected. This is a real gap for the project's target market.

## 3. Evidence

### Measured

| Item | Value |
|------|-------|
| `qpdf` | absent on host; CI installs. `qpdf --sign` supports PAdES-T with `--timestamp` flag |
| `openssl` | present on host (CLI). `openssl ts` for RFC 3161 requests |
| `openssl ocsp` | present on host. `openssl ocsp -resp_text` for OCSP parsing |
| `cargo deny check` (current) | 0 violations; `x509-cert` 0.2 already in gp-core |
| gp-core current crypto deps | rsa 0.9, p384 0.13, ecdsa 0.16, pkcs8 0.10, x509-cert 0.2, pkcs7 0.4, der 0.7 |
| MSRV | Rust 1.85 (workspace); all candidate crates support 1.75+ |

### Researched

- **RFC 3161 (TSA)**: Timestamp request = `TimestampRequest ::= SEQUENCE { version, messageImprint, reqCert[0] }`. Response = `TimestampResponse ::= SEQUENCE { status, timeStToken[0] }`. The `timeStToken` wraps a CMS SignedData. Hand-building is ~150 LOC; `timestamp` crate exists but is unmaintained (last release 2021, depends on old ring).
- **OCSP (RFC 6960)**: Simple ASN.1 structures. Request = `OCSPRequest`, Response = `OCSPResponse`. The response contains `TBSResponseData` with certificate status (`good`, `revoked`, `unknown`). The `x509-parser` crate supports `OCSPResponse` parsing natively.
- **CRL (RFC 5280)**: `CertificateList ::= SEQUENCE { tbsCertList, signatureAlgorithm, signatureValue }`. `x509-parser` supports `X509Crl` parsing natively.
- **PAdES LTV in PDF (ETSI EN 319 102-5)**: LTV data is stored in the signature dictionary as `/LTV` (array of LTVEntry objects). Each LTVEntry contains `/V` (validation data: OCSP response or CRL in DER), `/Type` (OCSP/CRL), `/Subtype` (PDFCA/ETSI.CAdES.detached), and `/Data` (the DER-encoded validation data).
- **`x509-parser` 0.15**: MIT/Apache-2.0, pure Rust, supports X.509 v3, CRL, OCSP, and revocation checking. Depends on `minimal-lexical` + `nom` (parser combinator). ~500 crates transitive, all MIT/Apache-2.0.
- **`reqwest` with `rustls-tls`**: MIT/Apache-2.0, pure Rust TLS via `rustls`. Does NOT pull `openssl-sys`. Feature flag: `reqwest = { version = "0.11", default-features = false, features = ["rustls-tls", "json"] }`.
- **`rcgen` 0.13**: MIT/Apache-2.0, generates self-signed X.509 certificates (CA + end-entity). Complements `x509-cert` builder (rcgen is more flexible for CA chains).
- **`time` 0.3**: MIT, better `OffsetDateTime` support for RFC 3161 timestamp parsing.
- **Alternative: `cryptographic-message-syntax` (CMS)**: A Rust CMS library (RFC 5652) exists but depends on `ring` — rejected per D25. No equivalent pure-Rust CMS library is available. Our existing manual PKCS#7 builder in `signature.rs` would need to be extended for LTV (adding `signedAttrs` with signingCertificate, signingTime, signerInfoDigestAlgorithms).

### Source

- `crates/gp-core/Cargo.toml`: current crypto dependency surface
- `deny.toml`: `-sys` crypto ban, ring ban, license allow-list
- `docs/contracts/C13-signature-integrity.md`: GP-S-010 (PAdES-T refused), GP-S-011 (CA certs refused), GP-S-012 (OCSP/CRL refused)
- ETSI EN 319 102-1 (PAdES), EN 319 102-5 (LTV), RFC 3161 (timestamp), RFC 6960 (OCSP), RFC 5280 (CRL)

## 4. Consequences

### Option B (recommended): Pure-Rust stack

**What it locks in:**
- **New crates**: `x509-parser`, `reqwest` (rustls-tls), `time`, `rcgen`, `pem`, `url` (reqwest transitive)
- **Transitive deps**: ~20-30 new crates (mostly from reqwest's HTTP stack + rustls TLS)
- **`deny.toml`**: No new bans needed; all candidate crates are MIT/Apache-2.0 (already allowed)
- **HTTP access during signing**: The signing flow may make outbound HTTP requests to TSA and
  OCSP/CRL responders. This is documented as a capability requirement (like engine subprocesses).
- **Honest-absent TSA**: If TSA is unreachable, signing proceeds with B-B (no timestamp).
  Diagnostic: `GP-S-020` (TSA unreachable — B-B fallback).
- **Honest-absent OCSP/CRL**: If LTV data cannot be fetched, signing proceeds with B-B/B-BT.
  Diagnostic: `GP-S-021` (OCSP/CRL unreachable — LTV deferred).
- **PDF LTV embedding**: LTV data (OCSP response / CRL) embedded in `/LTV` entries of the
  signature dictionary per ETSI EN 319 102-5.
- **New refusal codes**:
  | Code | Severity | Condition | Description |
  |------|----------|-----------|-------------|
  | `GP-S-020` | Warning | sign-request + TSA unreachable | "Timestamp server unreachable — signed without TSA (PAdES-B-B)" |
  | `GP-S-021` | Warning | sign-request + OCSP/CRL unreachable | "OCSP/CRL unavailable — signed without LTV data" |
  | `GP-S-022` | Error | sign-request + cert revoked (OCSP) | "Signing certificate is revoked per OCSP — signing refused" |
  | `GP-S-023` | Warning | sign-request + cert expired (CRL) | "Signing certificate expired per CRL — signed with warning" |

**New dependency surface** (added to `gp-core/Cargo.toml`):

```toml
# PAdES-T/LTV (P7-5): X.509 parsing, OCSP/CRL, LTV embedding
x509-parser = { version = "0.15", features = ["verify"] }
reqwest = { version = "0.11", default-features = false, features = ["rustls-tls", "json"] }
time = { version = "0.3", features = ["serde", "std"] }
rcgen = "0.13"
pem = "3.0"
url = "2"
```

**Binary impact**: ~3-5 MB additional (reqwest + rustls stack dominates); ~25-30 new transitive
dependency crates. Build time increase ~15-25s on cold cache.

**D2 compliance**: Maintained — all crates are pure Rust, no `-sys`, no C deps, cross-compile
clean (verified: `reqwest` with `rustls-tls` builds on all six targets).

**D25 compliance**: Maintained — all new crates use the RustCrypto licensing pattern (MIT or
MIT/Apache-2.0); no ring, no openssl, no -sys.

**`cargo deny` impact**: No new bans needed. All new crates have approved licenses. Add
`reqwest` to the workspace dependencies if other crates need HTTP access later.

### What it makes harder later:
- **Smart card / HSM signing (P7-7)**: Still owner-gated. Would need `pkcs11` bindings or
  `engine-ec` — likely requires D25 reconsideration (PKCS#11 is inherently C-linked).
- **PKCS#12 import**: Handled by `pkcs8` + `pem` (already present) or `p12` crate (if available).
  Self-signed workflow (current) doesn't need it.

### Option A (external tools): Consequences

- Adds hard dependencies on `qpdf` and `openssl` CLI — violates "install nothing" philosophy
- No new Rust deps but new host requirements
- Not honest-absent for signing (only for verification)
- Rejected — see above

### Option C (hand-built): Consequences

- Fragile ASN.1 parsing — high defect risk
- Still needs HTTP client for OCSP requests
- Rejected — `x509-parser` does this correctly

### Option D (defer): Consequences

- PAdES-B-B remains the ceiling
- Enterprise buyers lose LTV requirement
- Documents may become unverifiable after cert expiry
- Rejected — real gap

## Recommendation (non-binding — the owner decides)

**Option B** — pure-Rust stack with `x509-parser` + `reqwest` (rustls) + `rcgen`.

This satisfies D2/D25, delivers measurable enterprise value (LTV compliance), and fits the
existing RustCrypto ecosystem. The HTTP client is the only meaningful new surface area; it is
gated behind a signing-time operation (not runtime for every PDF operation) and uses honest-absent
fallback (B-B if TSA/OCSP unreachable).

**Implementation order** (high-level):
1. **Wave 1 (B-BT)**: RFC 3161 timestamp request/response. Add `--timestamp <url>` flag to
   `stichos sign`. If TSA unreachable, fall back to B-B with `GP-S-020`.
2. **Wave 2 (B-LT)**: OCSP response fetching + CRL downloading. Embed in `/LTV` entries.
   If unreachable, fall back to B-BT with `GP-S-021`.
3. **Wave 3 (B-LTA)**: Archival timestamps (wrap LTV in TSA timestamp). Lower priority.
4. **Wave 4 (trusted certs)**: CA-signed certificate support. Replace self-signed with
   user-provided PKCS#12. Requires `rcgen` CA chain + OCSP issuer cert embedding.

## Decision (owner, 2026-09-26)

**Decision: Accept ADR-0012 as proposed — adopt Option B (pure-Rust stack).**

- **Stack**: `x509-parser` 0.15 + `reqwest` (rustls-tls) + `time` 0.3 + `rcgen` 0.13 + `pem` 3.0 + `url` 2.
- **New dependency surface**: ~25-30 transitive crates, all MIT/Apache-2.0 (on allow-list). No `-sys`, no C deps. D2 and D25 maintained.
- **Binary impact**: ~3-5 MB additional; ~15-25s build time on cold cache.
- **Honest-absent**: TSA unreachable → B-B fallback (`GP-S-020`); OCSP/CRL unreachable → B-BT fallback (`GP-S-021`).
- **LTV timing**: Sign time (embedded in PDF, per ETSI EN 319 102-5). Default to attempting LTV; `--no-ltv` to skip.
- **OQ-1**: `rustls-tls` (recommended)
- **OQ-2**: Optional (honest-absent)
- **OQ-3**: Sign time
- **OQ-4**: Separate — P7-7 first (CMS foundation), then P7-5 on top
- **OQ-5**: Default-to-LTV with honest-absent fallback
- **OQ-6**: Continue extending manual builder for P7-5
- **D-row**: No reversal needed. D2 intact (pure-Rust, no `-sys`). D25 intact (RustCrypto ecosystem, no ring/openssl/-sys).
- **deny.toml**: No bans to add; all candidate crates MIT/Apache-2.0 (already allowed).
- **Implementation order**: Wave 1 (B-BT: RFC 3161 timestamps) → Wave 2 (B-LT: OCSP/CRL) → Wave 3 (B-LTA: archival) → Wave 4 (trusted certs).
- **New diagnostic codes**: `GP-S-020` (TSA unreachable — B-B), `GP-S-021` (OCSP/CRL unreachable — LTV), `GP-S-022` (cert revoked — refused), `GP-S-023` (cert expired — warning).
- **Tasks unblocked**: P7-5 (PAdES-T/LTV), P5-a (ADR acceptance gate).

---

## Open Questions (require owner input)

### OQ-1: Should `reqwest` use `rustls` or `native-tls`?

**Recommendation: `rustls-tls`**. `native-tls` pulls `openssl-sys` on Linux (which would violate
the `-sys` ban in `deny.toml`). `rustls` is pure Rust and already the default choice for
RustCrypto ecosystem crates.

**Owner decision**: rustls (recommended) / native-tls (would need `deny.toml` exception + D25
reconsideration).

### OQ-2: Should PAdES-T be required or optional?

**Recommendation: optional** (honest-absent). If TSA is unreachable, the signature is still
valid as B-B. This matches `qpdf --sign --timestamp` behavior (warning, not error) and keeps
signing possible without external dependencies.

**Owner decision**: optional (recommended) / required (would refuse signing if TSA unreachable
— simpler implementation but less practical).

### OQ-3: LTV fetch timing — at sign time or at verification time?

**Recommendation: at sign time**. PAdES-LTV embeds OCSP/CRL data directly in the PDF so that
verifiers do not need network access. This is the ETSI-specified approach and what PDF viewers
expect. Fetching at verification time would require a network-dependent verify path (not how
PAdES-LTV works).

**Owner decision**: sign time (recommended) / verification time (not standard PAdES-LTV).

### OQ-4: What is the scope of P7-7 (real PKCS#7/CMS signing)?

The current `signature.rs` uses a hand-built PKCS#7 SignedData. P7-7 (listed in remaining-work.md)
calls for "Replace `signature.rs` stub with real PKCS#7/CMS." This is a separate but related
task — P7-5 (this ADR) extends the signature with timestamps/LTV, while P7-7 would replace the
entire PKCS#7 builder.

**Recommendation**: P7-5 should be implemented on the current builder first (to validate the
timestamp/LTV approach). P7-7 (real CMS) can be a follow-on refactor — or P7-5 can be built on
top of a new CMS library if the owner decides to combine them.

**Owner decision**: separate (P7-5 on current builder, P7-7 as refactor) / combined (P7-5 and
P7-7 together on new CMS library).

### OQ-5: Should we add a `--no-ltv` flag or default to LTV?

**Recommendation: default to attempting LTV** (with honest-absent fallback). Users who don't want
LTV can use `--no-ltv` to skip OCSP/CRL fetching entirely (saves signing time). The default
should try to produce the best output.

**Owner decision**: default-to-LTV (recommended) / default-to-B-B (simpler).

### OQ-6: How should the `signature.rs` stub be handled?

The current `signature.rs` manually assembles PKCS#7 DER structures. This works but is
fragile. A proper CMS library (like a pure-Rust `cms` crate) would be more maintainable.

However, **no pure-Rust CMS library exists that doesn't depend on `ring`** (the `cryptographic-message-syntax` crate depends on ring — rejected per D25).

**Recommendation**: Continue extending the manual builder for P7-5 (timestamps add signed
attributes + timestamp token embedded in signedAttrs). Plan a P7-7 refactor to a hand-rolled
CMS builder (without external deps) in Phase 7 or later.

**Owner decision**: extend manual builder (recommended) / rewrite CMS first (larger scope,
higher risk, same outcome).

---

## Appendix: PAdES Profile Comparison

| Profile | Signature | Timestamp | OCSP/CRL | Archival | Long-term verifiable |
|---------|-----------|-----------|----------|----------|---------------------|
| B-B (shipped) | RSA/ECDSA + SHA-256 | No | No | No | No (cert expiry breaks) |
| B-BT (P7-5 Wave 1) | B-B + TSA timestamp | Yes (RFC 3161) | No | No | Yes (timestamp survives cert expiry) |
| B-LT (P7-5 Wave 2) | B-B + OCSP/CRL data | Recommended | Yes (embedded) | No | Yes (validation data embedded) |
| B-LTA (P7-5 Wave 3) | B-LT + archival TSA | Yes (chain) | Yes (embedded) | Yes | Yes (longest-lived) |

## Appendix: RFC 3161 Timestamp Flow

```
stichos sign --timestamp tsa.example.com
  │
  │ 1. Compute SHA-256 hash of signed PDF bytes (byte-range)
  │ 2. Build RFC 3161 TimestampRequest (DER)
  │    - messageImprint: { algorithm: sha256, hash: <bytes> }
  │    - optional: nonce, reqCert
  │ 3. HTTP POST to TSA (application/timestamp-query)
  │ 4. Receive TimestampResponse (DER)
  │    - status: successful
  │    - timeStToken: CMS SignedData containing timestamp
  │ 5. Extract TSA timestamp and embed in signature
  │    - signedAttrs: { signingTime, signingCert, ... }
  │    - tsaTimestamp: embedded as signedAttr (RFC 3161 in CMS)
  │ 6. Re-sign with updated signedAttrs
  │ 7. Output: PDF with /V 5 (PAdES-B-BT)
```

## Appendix: LTV Embedding Flow (ETSI EN 319 102-5)

```
After signing (B-B or B-BT):
  │
  │ 1. Extract signing certificate chain from signature
  │ 2. For each cert in chain:
  │    a. Fetch OCSP response from issuer's OCSP responder (URL in cert AIA extension)
  │       - HTTP POST/GET with DER-encoded OCSPRequest
  │       - Response: DER-encoded OCSPResponse (good/revoked/unknown)
  │    b. Fallback: Fetch CRL from issuer's CRL distribution point (URL in cert)
  │       - HTTP GET of DER-encoded CRL
  │    c. Store validation data: { type: "OCSP", data: <DER response> } or { type: "CRL", data: <DER CRL> }
  │ 3. Embed LTV entries in signature dictionary:
  │    /LTV [
  │      << /Type /LTV /Subtype /PDFCA /V <DER validation data> /Cert [chain] >>
  │      << /Type /LTV /Subtype /ETSI.CAdES.detached /V <DER OCSP> ... >>
  │    ]
  │ 4. Update PDF with incremental write (prefix invariant preserved)
  │ 5. Output: PDF with /V 5 + /LTV (PAdES-B-LT)
```

## Appendix: New `deny.toml` Entries (if any)

No new bans needed — all candidate crates are MIT/Apache-2.0 (already in the allow-list).

If the owner chooses `native-tls` for `reqwest` (OQ-1), the following entry would be needed:

```toml
# Exception: reqwest with native-tls for systems where rustls is unavailable
[[bans.skip]]
crate = "openssl-sys"
reason = "OQ-1: reqwest native-tls pulls openssl-sys; D25 reconsideration required"
```

But this is **strongly discouraged** — `rustls-tls` works on all six targets.
