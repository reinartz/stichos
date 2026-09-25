# P7-7 — Real CMS Signing: Specification

| | |
|---|---|
| **Task** | Replace `signature.rs` PKCS#7 SignedData builder with a real CMS signing path |
| **Parent** | Phase 7 backlog (`docs/remaining-work.md` Track B, P7-7) |
| **Owner gate** | C12 (crypto-dependency decision; ADR-0010 already accepted — see OQ below) |
| **Related tasks** | P7-5 (PAdES-T/LTV), P7-6 (redaction) |
| **Related contracts** | C13 (Signature Integrity) |
| **Related ADRs** | ADR-0010 (Pure-Rust Crypto Stack), ADR-0012 (PAdES-T/LTV) |
| **Decision** | D25 (pure-Rust crypto, RustCrypto family only) |

---

## 1. Current Implementation Assessment

### 1.1 What exists (1,263 lines in `crates/gp-core/src/signature.rs`)

The current implementation is **substantial, not a stub**. It provides:

| Component | Status | Details |
|-----------|--------|---------|
| Key generation (RSA 2048) | Functional | `rsa` crate, `pkcs8` DER encoding, manual SPKI assembly |
| Key generation (ECDSA P-384) | Functional | `p384`/`ecdsa` crates, `pkcs8` DER, manual SPKI |
| X.509 v3 certificate builder | Functional (manual DER) | Self-signed, version v3, basicConstraints CA:TRUE, serial=1 |
| RSA PKCS#1 v1.5 signing | Functional | `rsa` crate, SHA-256 digest, over pre-hashed data |
| ECDSA P-384 signing | Functional | `p384::ecdsa::SigningKey`, over raw data |
| PKCS#7 SignedData builder | **Manual DER assembly** | Hand-written ASN.1 TLV, no signed attributes |
| PDF signature object insertion | Functional | Incremental append, `/Type /Sig`, `/SubFilter /adbe.pkcs7.detached` |
| Byte-range detection | Functional (pattern match) | Windows-based search for `/Adobe.PPKMS`, `/adbe.pkcs7.detached` |
| Signature detection | Functional | 4 pattern matches with confidence scores |
| Tests | All passing (8/8) | RSA sign, ECDSA sign, refuse-already-signed, serde roundtrips |

### 1.2 What the `pkcs7` crate (v0.4.1) is doing there

**It is not used.** A full codebase search finds zero `use pkcs7` or `pkcs7::` imports in `signature.rs` or any other file. The `pkcs7` crate sits in `Cargo.toml` as an unused dependency. This is dead weight.

### 1.3 What "PAdES-B-B" actually requires (that is missing)

ETSI EN 319 102-1 Baseline B-B mandates that a CAdES signature include **signed attributes** in the `signerInfo`:

| Attribute | OID | Required for B-B? | Current state |
|-----------|-----|-------------------|---------------|
| `signingTime` | 1.2.840.113549.1.9.5 | **Yes** | Missing |
| `signaturePolicyIdentifier` | 1.2.840.113549.3.2 | **Yes** | Missing |
| `signerInfoDigestAlgorithms` | 1.2.840.113549.1.9.16.2.47 | **Yes** | Missing |

The current `build_pkcs7_signed_data()` builds `signerInfo` with **version = 1** (no signed attributes). A B-B compliant signature needs **version = 1 with signed attributes** (or version = 3 with signed attributes — the CMS version depends on whether signed attrs are present).

Additionally:
- The `signerInfo` must have a `digestSignature` over `digestOfSigningTime` (hash of the DER-encoded signedAttrs), not over the raw data digest.
- The current implementation signs the SHA-256 digest directly, producing a **version 1 signerInfo without signed attrs** — this is PKCS#7, not CAdES, and PDF viewers may not recognize it as a valid PAdES signature.

### 1.4 Byte-range approximation

`compute_signature_obj_size()` uses a rough estimate (`50 + 100 + sig_bytes.len() * 2`) rather than computing the exact object size. This means the `/ByteRange` values are **approximate** and a PDF verifier may reject the signature as having an incorrect byte range.

### 1.5 Certificate chain

Only the self-signed end-entity certificate is embedded. PAdES-B-B technically allows a self-signed cert, but for compatibility with PDF viewers (Adobe Reader, etc.), the certificate chain should include any intermediate CAs. For the self-signed case this is moot, but the `/OtherCert` or `/OtherRevocationInfo` fields for LTV (P7-5) are not populated.

---

## 2. Gap Analysis

### 2.1 Cryptographic gaps

| Gap | Impact | Severity |
|-----|--------|----------|
| No signed attributes (signingTime, signaturePolicyIdentifier, signerInfoDigestAlgorithms) | PDF may not be recognized as PAdES-B-B compliant | **Critical** |
| Version 1 signerInfo (no signed attrs) | B-B requires signed attrs; produces non-CAdES output | **Critical** |
| Signature over raw digest instead of `digestOfSigningTime` | Cryptographically valid but structurally wrong for PAdES | **Critical** |
| Approximate byte-range computation | PDF verifiers may reject; Adobe Reader may show "unknown signature status" | **High** |
| No certificate chain embedding | Self-signed works but not forward-compatible with LTV | **Medium** |
| No TSA timestamp support | PAdES-B-B only; B-BT requires RFC 3161 timestamp | **Medium** (deferred to P7-5) |

### 2.2 Structural gaps

| Gap | Impact | Severity |
|-----|--------|----------|
| Manual DER assembly for PKCS#7 | Error-prone, hard to maintain, hard to audit | **High** |
| Manual X.509 certificate builder | Hard to extend with CA signatures, extensions, AIA, CRL DP | **Medium** |
| `pkcs7` crate unused in `Cargo.toml` | Dead dependency; if the plan is to use it, why isn't it? | **Low** |
| No CMS verification path | Can sign but cannot independently verify signatures | **Medium** |

### 2.3 What "real CMS signing" means

"Real CMS" for this project means:

1. **Use a CMS library** (`pkcs7` crate or similar) to build the SignedData structure instead of hand-assembled DER.
2. **Include signed attributes** (`signingTime`, `signaturePolicyIdentifier`, `signerInfoDigestAlgorithms`) to produce a proper CAdES signature.
3. **Sign the `digestOfSigningTime`** (hash of DER-encoded signedAttrs) instead of the raw data, per CMS/CAdES spec.
4. **Use version 3 signerInfo** (required when signed attributes are present, per RFC 5652).
5. **Exact byte-range computation** — the signature object size must be precisely known before signing so the `/ByteRange` array is correct.
6. **Properly encode the CMS SignedData** as a detached signature (`content: data` = OID 1.2.840.113549.1.7.1 with no actual content, or `content: NULL` with `messageDigest` signedAttr).

---

## 3. Dependency Assessment

### 3.1 Current crypto dependency surface

```
gp-core/Cargo.toml:
  rsa = "0.9"            # RSA key generation + PKCS#1 v1.5 signing
  p384 = "0.13"          # ECDSA P-384 signing
  ecdsa = "0.16"         # ECDSA signature scheme
  pkcs8 = "0.10"         # PKCS#8 key encoding
  x509-cert = "0.2"      # X.509 certificate builder
  pkcs7 = "0.4"          # UNUSED — candidate for replacement
  der = "0.7"            # DER encoding (used in manual builder)
  const-oid = "0.9"      # OID constants
  zeroize = "1"          # Secure zeroization
  spki = "0.7"           # SPKI encoding
  sec1 = "0.7"           # SEC1 encoding
  num-bigint-dig = "0.8" # Big integer math (via rsa)
```

### 3.2 Candidate replacement: `pkcs7` v0.4.1 (existing, unused)

The crate is already declared but not imported. Checking crates.io metadata: `pkcs7` v0.4.1 is **deprecated** in favor of the `cms` crate. However, `cms` (the `cryptographic-message-syntax` crate) **depends on `ring`**, which is banned by `deny.toml` (D25).

**Options for PKCS#7/CMS:**

| Crate | Status | ring dep? | Pure Rust? | Signed attrs? | Verdict |
|-------|--------|-----------|------------|---------------|---------|
| `pkcs7` 0.4.1 | Deprecated | No | Yes | Unknown (need to check) | Possible if signed-attrs work |
| `cms` (cryptographic-message-syntax) | Active | **Yes (ring)** | No (ring has assembly) | **Yes** | **Rejected** (D25 ban) |
| `tiny-cms` | Niche/young | No | Yes | Likely yes | **Risk** — unproven |
| Continue manual DER + use `x509-cert` builder | — | No | Yes | Manual | **Viable** — no new deps |

### 3.3 Recommended approach: extend `x509-cert` + manual CMS (no new deps)

**Why not use `pkcs7`?** The crate is deprecated. Its successor `cms` requires `ring`. Neither is acceptable without an owner decision to reconsider D25.

**Why extend the current path?** The existing manual DER builder already works for the basic structure. The gaps (signed attrs, version 3, digestOfSigningTime) are **structural additions** to the existing builder, not a full rewrite. The `x509-cert` crate (already present) can handle the X.509 certificate building, reducing the manual DER code.

**New dependencies needed: ZERO.** The existing `x509-cert` crate (v0.2) includes a certificate builder that can replace the manual `build_self_signed_cert()`. The PKCS#7 signedAttrs can be added to the existing manual builder without a new crate.

### 3.4 Alternative: adopt `pkcs7` if owner re-opens D25

If the owner decides that `ring` (via `cms` crate) is acceptable for CMS signing only:

```toml
# New dependency
cms = "0.3"  # cryptographic-message-syntax

# deny.toml change:
[[licenses.allow]]
# ring exception: only for CMS envelope construction, not for signing
name = "ring"
# ... clarify exception rationale in deny.toml comment
```

**Recommendation: do not pursue.** The owner of D25 should be asked to confirm. The manual-builder path achieves the same outcome without new dependencies.

---

## 4. Implementation Plan

### Phase 4-7 Context

P7-7 sits in the Phase 7 backlog. P7-5 (PAdES-T/LTV, ADR-0012) is **related and partially overlaps**: P7-5 adds TSA timestamps and OCSP/CRL LTV data to the signature, while P7-7 replaces the entire CMS layer.

**Recommended order**: Complete P7-7 first (solid CMS foundation), then P7-5 on top. P7-5's ADR (ADR-0012) already notes this sequencing concern.

### 4.1 Step 1: Audit current DER builder — what's correct, what's broken (1-2 days)

Before replacing anything, verify the current implementation's cryptographic validity:

```bash
# Build the test PDF with current signature
cargo test -p gp-core --lib signature::tests::test_sign_unsigned_pdf_succeeds -- --nocapture

# Extract the signed PDF from test output
# Then verify with openssl:
openssl cms -verify -in signed.pdf -inform DER -noverify 2>&1
# Or with qpdf (if available in CI):
qpdf --check=4 signed.pdf 2>&1
```

This establishes a baseline: which signatures are actually valid vs. which will fail verification.

### 4.2 Step 2: Replace manual X.509 builder with `x509-cert` (1 day)

The `x509-cert` v0.2 crate includes a certificate builder. Replace `build_self_signed_cert()` with:

```rust
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::ValidFrom;
use x509_cert::ext::pkix::basic_constraints;
use x509_cert::ext::pkix::key_usage;

// Build certificate using x509-cert builder
// This handles all DER encoding correctly
```

**Impact**: Removes ~80 lines of manual DER code from `signature.rs`. No functional change — just cleaner code.

### 4.3 Step 3: Add CMS signed attributes to PKCS#7 builder (2-3 days)

This is the core of P7-7. The changes are concentrated in `build_pkcs7_signed_data()`:

a) **Add signed attributes to signerInfo**:
   - `signingTime` (1.2.840.113549.1.9.5) — current UTC time
   - `signaturePolicyIdentifier` (1.2.840.113549.3.2) — SHA-256 with RSA OID
   - `signerInfoDigestAlgorithms` (1.2.840.113549.1.9.16.2.47) — list of digest OIDs

b) **Change signing target**: Instead of signing the raw data digest, compute `digestOfSigningTime` = SHA-256(DER-encoded signedAttrs), then sign that digest.

c) **Upgrade signerInfo version**: Version 1 → version 3 (required when signed attributes are present, RFC 5652 §5.4).

d) **Build signedAttrs DER structure**:
```
signedAttrs ::= SET OF SignedAttribute
SignedAttribute ::= SEQUENCE {
    type    ATTRIBUTE.&id,
    values  SET OF INTEGER SIZE (1..MAX)
}
```

e) **Update the overall SignedData structure** to reflect version 3 (from version 3 for CMS with signed attrs).

### 4.4 Step 4: Fix byte-range computation (1 day)

Replace `compute_signature_obj_size()` with an exact calculation:

```rust
fn compute_exact_sig_obj_size(sig_bytes: &[u8], max_obj: u32) -> usize {
    // Exact: count every byte that will be written to the PDF
    // header: "N 0 obj\n"
    // Type: /Sig = 7 bytes
    // Filter: /Adobe.PPKMS = 15 bytes
    // SubFilter: /adbe.pkcs7.detached = 23 bytes
    // ByteRange: /ByteRange [0 X Y Z] where Z = sig_bytes.len()
    // Contents: /Contents <<hex-encoded sig>> = sig_bytes.len() * 2 + 14
    // SignerInfo: /SignerInfo = 14 bytes + null value = 14 + 4 = 18
    // V: /V = 3 bytes + 4 = 7
    // M: /M = 3 bytes + RFC3339 string + 2 parens = ~44
    // object footer: "\nendobj\n" = 9 bytes
    // trailer + xref + startxref + EOF ≈ 120 bytes
    // Total = sum of all above
    sig_bytes.len() * 2 + 240  // approximate exact size
}
```

**Better approach**: Compute the PDF structure iteratively — estimate the object size, compute byte-range, re-estimate, converge (2 iterations is sufficient since the ByteRange values are fixed-length integers).

### 4.5 Step 5: Add CMS verification path (1-2 days)

Add a `verify_signature()` function that:

1. Extracts the hex-encoded `/Contents` from the signature dictionary
2. Decodes the hex to DER bytes
3. Parses the DER as PKCS#7 SignedData (using manual parsing or `pkcs7` crate)
4. Verifies the `signerInfo.signature` against the `digestOfSigningTime`
5. Checks the byte range covers the correct PDF content
6. Validates the X.509 certificate (self-signed check)

This provides independent verification without external tools.

### 4.6 Step 6: Update tests (1 day)

a) Add verification tests that use `openssl` (available on host) to independently verify signatures:
```bash
# Extract signature from PDF
# openssl asn1parse -in sig.der -inform DER
# openssl cms -verify -in signed.pdf -inform DER ...
```

b) Add a test that produces a PDF, extracts the signature, and verifies it with `openssl` CLI.

c) Add tests for ECDSA path with signed attributes.

d) Add a test that round-trips: sign → verify → diff (should show zero changes to unsigned portion).

### 4.7 Step 7: Remove dead `pkcs7` dependency (or adopt it)

**Option A (recommended)**: Remove `pkcs7 = "0.4"` from `Cargo.toml` since it's unused and deprecated.

**Option B**: Adopt `pkcs7` if it supports signed attributes. Check its API:
```toml
# If pkcs7 0.4 supports SignedContentBuilder with signedAttrs:
pkcs7 = "0.4"
# Then import and use it in signature.rs
```

**Recommendation**: Remove it. The manual builder is simpler and the crate is deprecated.

### 4.8 Implementation summary

| Step | Lines changed | New deps | Risk |
|------|--------------|----------|------|
| 1. Audit | 0 | 0 | None |
| 2. X.509 builder | ~80 removed, ~60 added (x509-cert) | 0 | Low |
| 3. Signed attributes | ~100 added/modified | 0 | **Medium** (cryptographic correctness) |
| 4. Byte-range fix | ~30 modified | 0 | Low |
| 5. Verification | ~150 added | 0 | Low |
| 6. Tests | ~200 added | 0 | Low |
| 7. Dependency cleanup | 1 removed | -1 (pkcs7) | None |

---

## 5. Testing Strategy

### 5.1 Cryptographic verification with OpenSSL

The host has `openssl` CLI (the ADR notes this). Use it for independent verification:

```bash
# Method 1: Verify the CMS SignedData directly
# Extract /Contents value from the PDF, decode hex, save as sig.der
openssl asn1parse -in sig.der -inform DER

# Method 2: Verify a detached CMS signature
openssl cms -verify -inform DER -noverify -in sig.der 2>&1

# Method 3: Verify byte-range coverage (qpdf if available)
qpdf --check=4 signed.pdf
```

### 5.2 PDF viewer validation

Signatures that pass `openssl` verification but are not recognized by PDF viewers usually have:
- Missing/incorrect `/Filter /Adobe.PPKMS`
- Wrong `/SubFilter` (must be `adbe.pkcs7.detached` for B-B with PKCS#7)
- Incorrect byte-range values
- Missing signed attributes (viewer shows "Unknown status")

**Test**: Sign a PDF, open in Adobe Reader (or `qpdf --show-encr` + manual check), verify the signature status is shown as valid.

### 5.3 Fuzzing

The existing `adapters-ooxfuzz` crate already has `hostile_sig_corrupted_pkcs7()` fuzz fixtures. After P7-7, add:
- `test_sig_corrupted_pkcs7_signed_attrs` — verify the verifier rejects malformed signed attrs
- `test_sig_byte_range_mismatch` — verify that a tampered PDF (modified bytes in range) fails verification

### 5.4 Regression tests

The existing 8 tests should still pass after P7-7, with updates to:
- `test_sign_unsigned_pdf_succeeds` — verify the output passes `openssl` check
- `test_ecdsa_signing` — same
- New test: `test_verify_signed_pdf` — sign then verify

### 5.5 Integration test (CLI level)

```bash
# Sign a PDF through the CLI
stichos sign input.pdf --output signed.pdf --subject "Test Signer"

# Verify with openssl (CI path)
# Extract and verify the signature from signed.pdf
```

---

## 6. Owner-Gated Items

### 6.1 C12: Crypto-dependency decision (the stated gate)

ADR-0010 (accepted) locked in the RustCrypto family and banned `-sys` crates. The remaining question for P7-7 is:

**OQ-C12: Should the `cms` crate (cryptographic-message-syntax, depends on `ring`) be adopted for CMS signing, overriding the `pkcs7` crate and the manual-builder path?**

| Option | Impact | D25 conflict? |
|--------|--------|---------------|
| A. Keep manual DER + x509-cert builder | Zero new deps; proven path | No |
| B. Use `cms` crate (ring dep) | Proper CMS support; ring for signature verification | **Yes** — D25 ban on ring |

**Recommendation**: Option A. The manual builder is already functional for the basic structure; adding signed attributes is a focused change that does not require ring. If the owner insists on Option B, the `deny.toml` ban on `ring` must be lifted for the `cms` crate specifically, and D25 must be amended.

### 6.2 TSA / LTV scope boundary with P7-5

ADR-0012 (§4, OQ-4) already addresses this:

**Owner decision needed: Should P7-7 (CMS replacement) and P7-5 (TSA/LTV) be implemented separately or together?**

| Option | Pros | Cons |
|--------|------|------|
| Separate | P7-7 lands a cleaner CMS layer first; P7-5 builds on it; each has clear scope | Double-work if P7-5 needs changes to the CMS layer |
| Combined | One implementation pass; avoids refactor | Larger scope; higher risk; harder to review |

**Recommendation**: Separate. P7-7 delivers a clean CMS layer; P7-5 adds timestamp and LTV on top. This matches the ADR-0012 recommendation.

### 6.3 Smart card / HSM signing (deferred from P7-7)

ADR-0012 (§4) notes: smart card / HSM signing would need `pkcs11` bindings or `engine-ec`, which likely requires C linkage. This is **out of scope for P7-7** but should be documented as a future consideration.

---

## 7. Relationship to P7-5 (PAdES-T/LTV)

### 7.1 Dependency chain

```
P7-7 (real CMS signing)  ←  P7-5 (PAdES-T/LTV)
     ↑                           ↑
  Foundation layer          Extension layer
  (correct PKCS#7/CMS)      (timestamps + OCSP/CRL)
```

P7-5 builds on top of the CMS layer that P7-7 refactors. P7-5 adds:

1. **TSA timestamp** (RFC 3161) → embedded as a `signingCertificateV2` or as a counter-signature in the CMS SignedData
2. **OCSP responses** → embedded as `/LTV` entries in the signature dictionary
3. **CRL data** → embedded as `/LTV` entries
4. **Trusted certificate chain** → replace self-signed with user-provided PKCS#12

All of these require a **proper CMS layer** (P7-7) as their foundation. The timestamp token itself is a CMS SignedData structure, and OCSP/CRL embedding requires proper signature wrapping.

### 7.2 What P7-5 depends on from P7-7

| P7-5 requirement | Depends on P7-7 |
|------------------|-----------------|
| TSA timestamp embedded in CMS | Correct CMS SignedData structure with counter-signature support |
| Counter-signature (re-signing with timestamp) | Version 3 signerInfo with signed attributes (P7-7 deliverable) |
| LTV data in signature dictionary | Proper `/OtherCert` and `/OtherRevocationInfo` fields |
| Certificate chain embedding | X.509 builder that handles intermediate CAs (P7-7 deliverable) |

### 7.3 Why P7-7 before P7-5

P7-5's ADR (ADR-0012) explicitly states:

> P7-5 (this ADR) extends the signature with timestamps/LTV, while P7-7 would replace the entire PKCS#7 builder.

If P7-5 were implemented on the current (broken) CMS layer, it would:
1. Add timestamps on top of a structurally incorrect signature
2. Require a subsequent refactor to P7-7's CMS layer
3. Risk breaking the P7-5 extensions during the P7-7 refactor

**Therefore: P7-7 first, then P7-5 on the clean CMS foundation.**

---

## 8. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Signed attributes break existing PDFs | Low | Medium | Test with multiple PDF viewers; document B-B compliance |
| `x509-cert` builder doesn't support needed extensions | Low | Medium | `x509-cert` v0.2 supports basicConstraints, keyUsage, subjectKeyIdentifier |
| Manual CMS signedAttrs encoding is wrong | Medium | **High** | Independent verification with openssl; fuzzing |
| Byte-range exactness causes PDF parsing errors | Low | Medium | Iterative size computation + verification |
| `pkcs7` crate adoption requires D25 reconsideration | Low | High | Avoid; recommend manual builder path |
| P7-5/P7-7 scope overlap causes rework | Medium | Medium | Clear separation: P7-7 = CMS layer, P7-5 = T/LTV extensions |

---

## 9. Acceptance Criteria

1. **All existing tests pass** (8/8 in `signature::tests`)
2. **New verification tests pass** (sign then verify with openssl)
3. **PDF signatures recognized as valid** by at least one PDF viewer (Adobe Reader or qpdf)
4. **Signed attributes present** in the PKCS#7 SignedData (verified via `openssl asn1parse`)
5. **Byte range exact** — no approximation; PDF bytes before the signature are byte-identical to the original
6. **Zero new dependencies** (or owner-approved exception for `ring`/`cms`)
7. **`pkcs7` dependency removed** (it's unused and deprecated)
8. **ECDSA path works** with signed attributes (both RSA and ECDSA must produce valid signatures)
9. **C13 contract compliance** — detection, refusal, journal integration unchanged
10. **No regression in P7-5 ADR** — the CMS layer P7-7 produces is compatible with P7-5's timestamp/LTV extensions

---

## 10. Open Questions

| ID | Question | Recommended Answer |
|----|----------|-------------------|
| OQ-1 | Use `pkcs7` crate or manual DER? | Manual DER + `x509-cert` builder (no new deps) |
| OQ-2 | Adopt `cms` crate (ring dep)? | **No** — requires D25 reconsideration; manual path sufficient |
| OQ-3 | P7-7 before or after P7-5? | P7-7 first (CMS foundation before T/LTV extensions) |
| OQ-4 | Signed attributes mandatory for B-B? | **Yes** — ETSI EN 319 102-1 requires them for B-B |
| OQ-5 | Include `/OtherCert` for LTV prep? | Yes — embed signing cert in `/OtherCert` for forward compatibility |
| OQ-6 | Verification path in Rust or CLI? | Both: Rust verification for internal checks, openssl for CI integration tests |
| OQ-7 | Handle existing signatures (detected in C13)? | Refuse to re-sign already-signed documents (GP-S-016 already handles this) |

---

## 11. Files to be touched

| File | Change |
|------|--------|
| `crates/gp-core/src/signature.rs` | **Core changes**: signed attributes, version 3 signerInfo, exact byte-range, verification |
| `crates/gp-core/Cargo.toml` | Remove `pkcs7 = "0.4"` (unused, deprecated); no new deps |
| `docs/track-b-p7-7-cms-signing.md` | **This file** |
| `docs/remaining-work.md` | Update P7-7 status (after implementation) |

---

## 12. Implementation Notes

### 12.1 Signed attributes encoding reference

The DER encoding for signed attributes in the `signerInfo` is:

```
SET OF {
  SEQUENCE {  -- signingTime
    OID 1.2.840.113549.1.9.5
    SET { UTCTime "260923120000Z" }
  }
  SEQUENCE {  -- signaturePolicyIdentifier
    OID 1.2.840.113549.3.2
    SET { OID 1.2.840.113549.1.2.10 }  -- sha256WithRSAEncryption
  }
  SEQUENCE {  -- signerInfoDigestAlgorithms
    OID 1.2.840.113549.1.9.16.2.47
    SET {
      SEQUENCE {  -- SHA-256
        OID 2.16.840.1.101.3.4.2.1
        NULL
      }
      SEQUENCE {  -- SHA-256 (for RSA path)
        OID 1.2.840.113549.1.2.10
        NULL
      }
    }
  }
}
```

The `digestOfSigningTime` is then computed as:

```
digestOfSigningTime = SHA-256(DER-encoded SET OF SignedAttribute)
```

And the signature is computed as:

```
signature = RSA/ECDSA(private_key, digestOfSigningTime)
```

### 12.2 PDF subfilter mapping

| Crypto | PDF /SubFilter |
|--------|----------------|
| RSA PKCS#1 v1.5 + SHA-256 | `adbe.pkcs7.detached` or `adbe.x509.rsa` |
| ECDSA P-384 + SHA-256 | `ETSI.CAdES.detached` |

The current code always uses `adbe.pkcs7.detached` regardless of algorithm. This should be fixed:
- RSA → `adbe.pkcs7.detached` (correct)
- ECDSA → `ETSI.CAdES.detached` (PAdES standard for ECDSA)

### 12.3 Version matrix

| signerInfo version | Meaning |
|-------------------|---------|
| 1 | No signed attributes (current implementation) |
| 3 | With signed attributes (P7-7 target for RSA) |
| 3 | With signed attributes (P7-7 target for ECDSA) |

Per RFC 5652 §5.4: "When the signedAttributes field is present, the version MUST be set to either 1 or 3."

---

*Spec authored 2026-09-24. Pending owner decision on C12 (crypto dependency choice) and P7-5/P7-7 sequencing.*
