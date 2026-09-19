# C13 — Signature Integrity Contract

| | |
|---|---|
| Owner | CORE, PDF |
| Consumers | all executors (read), adapters-pdf (detection), cli/mcp (surface) |
| Freeze | draft at P5-11 |
| Version | 1.0.0-draft |
| Status | draft at P5-11 |
| Artifacts | `crates/gp-core/src/ir.rs` (SignatureInfo), `crates/gp-core/src/executor.rs` (SignatureInfo + detect_signatures), `crates/adapters-pdf/src/incremental.rs` (detect_signatures, SignatureDetected), `crates/gp-core/src/diagnostics.rs` (GP-S-001..GP-S-xxx) |
| Conformance | detection parity + refusal on mutation + sign-later golden + tamper evidence |

## 1. Scope

This contract defines the signature integrity model for the stichos IR. It covers:
- Detection of existing digital signatures in PDF documents
- Signature state tracking (unsigned / signed-valid / signed-mutation-detected)
- Refusal of mutations to signed documents (with policy override)
- PAdES-B-B sign-later capability (creating signatures over documents)
- Journal tracking of signature-invalidating events

## 2. Types

### 2.1 SignatureInfo (IR-level)

```rust
pub struct SignatureInfo {
    /// Signature type: "pkcs7", "office-xml", "pdf-adobe", "pdf-pades", or "unknown"
    pub sig_type: String,
    /// Human-readable location description
    pub location: String,
    /// Confidence 0.0..=1.0 of detection certainty
    pub confidence: f64,
    /// Optional: X.509 subject CN (if certificate is parseable)
    pub subject: Option<String>,
    /// Optional: digest algorithm OID string (e.g. "2.16.840.1.101.3.4.2.1" for SHA-256)
    pub digest_algorithm: Option<String>,
    /// Optional: signature start byte offset in document
    pub byte_offset: Option<usize>,
    /// Optional: signature end byte offset (byte-range end)
    pub byte_end: Option<usize>,
}
```

Serialized as JSON with snake_case keys. All fields except `sig_type` are optional.

### 2.2 SignatureState

```rust
pub enum SignatureState {
    /// No signatures detected
    Unsigned,
    /// Document has valid signatures, no mutations applied
    SignedValid,
    /// Document had signatures but a mutation would invalidate them
    SignedMutationDetected,
    /// Document was signed by stichos (sign-later path)
    SignedByStichos,
}
```

Serialized as lowercase: `"unsigned"`, `"signed_valid"`, `"signed_mutation_detected"`, `"signed_by_stichos"`.

### 2.3 PdfDocument.signatures

The `PdfDocument` IR type gains an optional field:

```rust
pub struct PdfDocument {
    // ... existing fields ...
    pub signatures: Option<Vec<SignatureInfo>>,
}
```

All `PdfDocument` fields are additive-compat (Option<T>, never replace).

### 2.4 ExecuteResult.invalidated_signatures

Existing field — populated when a mutation would invalidate signatures:

```rust
pub struct ExecuteResult {
    // ... existing fields ...
    pub invalidated_signatures: Vec<SignatureInfo>,
}
```

## 3. Signature States

### 3.1 State Machine

```
Unsigned --[detect sig]--> SignedValid --[mutation]--> SignedMutationDetected
SignedValid --[sign]--> SignedByStichos
SignedByStichos --[mutation]--> SignedMutationDetected
```

- **Unsigned**: No signatures detected in the document. Normal mutation operations proceed.
- **SignedValid**: Document has one or more valid signatures. Mutations are **refused by default** (GP-S-001). Explicit policy override with `--force-signature-invalidation` records `signature_invalidated` journal event (GP-S-002).
- **SignedMutationDetected**: A mutation was applied that invalidated existing signatures. This state is recorded in the fidelity budget and journal.
- **SignedByStichos**: Document was signed by `stichos sign` (PAdES-B-B path). Further mutations follow the same rules as SignedValid.

### 3.2 Detection Rules

Detection runs on every input PDF through `adapters-pdf/src/incremental.rs::detect_signatures()`:

1. `/Adobe.PPKMS` filter -> `sig_type="pdf-adobe"`, confidence=0.95
2. `/adbe.pkcs7.detached` subfilter -> `sig_type="pdf-pades"`, confidence=0.95
3. `/adbe.x509.rsa` subfilter -> `sig_type="pdf-pades"`, confidence=0.90
4. `/ETSI.CAdES.detached` subfilter -> `sig_type="pdf-pades"`, confidence=0.95
5. `/SigFlags` >= 4 -> `sig_type="pdf-pades"`, confidence=0.80
6. `/Type /Sig` -> `sig_type="pdf-pades"`, confidence=0.75
7. No signatures found -> `SignatureState::Unsigned`

## 4. Refusal Semantics

### 4.1 Refuse List

| Code | Severity | Condition | Description |
|------|----------|-----------|-------------|
| `GP-S-001` | Error | mutation + signed doc, no policy | "Document has digital signatures — write refused" |
| `GP-S-002` | Warning | mutation + signed doc + policy override | "Signature invalidated: mutation applied with explicit policy override" |
| `GP-S-003` | Info | mutation + unsigned doc | "No signatures detected — mutation proceeds normally" |
| `GP-S-010` | Error | sign-request + TSA required | "PAdES-T/LTV signing not supported (TSA absent)" |
| `GP-S-011` | Error | sign-request + CA-signed cert | "CA-signed certificate signing not supported (self-signed only for B-B)" |
| `GP-S-012` | Error | verify-request + CRL/OCSP | "CRL/OCSP verification not implemented (B-B only checks signature validity)" |
| `GP-S-013` | Error | sign-request + smart card | "Smart card / HSM signing not supported (in-memory keys only)" |
| `GP-S-014` | Error | sign-request + PEM input | "PKCS#12 input required (use openssl pkcs12 to convert)" |
| `GP-S-015` | Error | sign-request + /AcroForm /SigFields | "Signing refused: /AcroForm already contains /SigFields (conflicting signature fields)" |
| `GP-S-016` | Error | sign-request + already signed | "Signing refused: document already has signatures (re-sign after re-signing original)" |

### 4.2 Refusal Flow

```
Input PDF -> detect_signatures() -> signatures present?
  Yes -> mutation requested?
    Yes -> policy override present?
      Yes -> record GP-S-002 in diagnostics, proceed with mutation, record journal event
      No  -> refuse with GP-S-001, return ExecuteResult with invalidated_signatures populated
    No  -> proceed (read-only operation)
  No  -> proceed (unsigned document)
```

## 5. Sign-Later Semantics (PAdES-B-B)

The sign-later path creates a PAdES-B-B compliant signature over an existing (unsigned) PDF:

1. **Input**: unsigned PDF, identity (PKCS#12 or self-generated key pair)
2. **Byte range**: `/ByteRange [0 <sig-value-len> <sig-value-offset> <remaining-len>]`
   - Covers all bytes from start of document through the signature value placeholder
3. **Digest**: SHA-256 over the byte range content
4. **Signature**: RSA PKCS#1 v1.5 or ECDSA P-384 over the digest
5. **Certificate**: Self-signed X.509 v3 certificate (subject, issuer = same)
6. **Attributes** (B-B requirement):
   - `signingTime` (current timestamp)
   - `signaturePolicyIdentifier` (OID 1.2.840.113549.3.2 — SHA-256 with RSA)
   - `signerInfoDigestAlgorithms` (signed digest algorithms)
7. **Output**: PDF with `/Filter /Adobe.PPKMS`, `/SubFilter /adbe.pkcs7.detached`
8. **Verification**: `qpdf --check` (CI), openssl byte-range verify (independent)

### 5.1 Honest Absent

- **qpdf**: Not installed on this host. CI installs it. Honest-absent path: skip qpdf check,
  log `PDF-I-001` (qpdf not available).
- **veraPDF**: Not installed on this host. CI installs it. Honest-absent path: skip PDF/A check.
- **TSA**: No TSA server available. TSA timestamp is PAdES-T (beyond B-B scope).
  Documented as `--no-tsa` flag in sign command.
- **openssl**: Installed on this host (CLI). Used for independent byte-range verification.

## 6. Journal Integration

Signature-invalidating mutations record a journal event:

```rust
pub enum JournalAction {
    // ... existing variants ...
    /// Mutation applied to a signed document (signature invalidated)
    SignatureInvalidated {
        /// Number of signatures invalidated
        signature_count: usize,
        /// Diagnostic codes for the invalidation
        codes: Vec<String>,
    },
}
```

Signing (sign-later) also records a journal event:

```rust
pub enum JournalAction {
    // ... existing variants ...
    /// Document signed via sign-later (PAdES-B-B)
    SignedDocument {
        /// Signature subject (from certificate)
        subject: String,
        /// Digest algorithm (e.g. "sha256")
        digest: String,
        /// Signature type (e.g. "pdf-pades")
        signature_type: String,
    },
}
```

## 7. Fidelity Budget Integration

Signature invalidation is recorded in the fidelity budget as a dropped feature:

```json
{
  "dropped_features": [
    {
      "feature": "digital_signature",
      "reason": "incremental_update_invalidates_signature",
      "count": 1
    }
  ],
  "part_warnings": [
    {
      "part": "pdf:signature",
      "message": "Document had 1 signature(s); mutation invalidated signature(s)"
    }
  ]
}
```

## 8. Anchoring Rules

### 8.1 C1 Additive Compatibility

`PdfDocument.signatures` is `Option<Vec<SignatureInfo>>` — additive only. Existing
documents without this field deserialize to `None`, which is treated as unsigned.

### 8.2 Detection Parity

Detection must be parity-exact: every `/Type /Sig` annotation found in the PDF must
produce exactly one `SignatureInfo` entry. No silent drops.

### 8.3 Mutation Path Coverage

Every mutation executor path (apply, edit, annots resolve, revisions accept/reject)
must consult signature detection before proceeding. There are no escape hatches.
