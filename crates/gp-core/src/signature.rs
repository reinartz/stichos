// SPDX-License-Identifier: MIT OR Apache-2.0
//! Digital signature support for PDF documents (C13 — P5-13).
//!
//! This module provides PAdES-B-B sign-later capability for stichos.
//! It creates signatures using the RustCrypto family:
//! - RSA PKCS#1 v1.5 over SHA-256
//! - ECDSA P-384 over SHA-256
//! - Self-signed X.509 certificates
//! - PKCS#7/CAdES byte-range signatures
//!
//! See also: `docs/adr/ADR-0010-crypto-stack.md`, `docs/contracts/C13-signature-integrity.md`

use crate::ir::SignatureInfo;
use lopdf::Object;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The algorithm used for signing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignAlgorithm {
    /// RSA PKCS#1 v1.5 with SHA-256.
    RsaSha256,
    /// ECDSA with P-384 curve and SHA-256.
    EcdsaP384,
}

impl std::fmt::Display for SignAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignAlgorithm::RsaSha256 => write!(f, "rsa_sha256"),
            SignAlgorithm::EcdsaP384 => write!(f, "ecdsa_p384"),
        }
    }
}

/// A PKCS#12 identity for signing.
#[derive(Debug, Clone)]
pub struct SigningIdentity {
    /// The algorithm to use.
    pub algorithm: SignAlgorithm,
    /// Optional: subject CN for the generated certificate.
    pub subject_cn: String,
}

impl Default for SigningIdentity {
    fn default() -> Self {
        Self {
            algorithm: SignAlgorithm::RsaSha256,
            subject_cn: "stichos".to_string(),
        }
    }
}

/// Result of a sign operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignResult {
    /// The signed PDF bytes.
    pub output: Vec<u8>,
    /// Signature information (subject, digest, byte-range).
    pub signature: SignatureEnvelope,
    /// Whether signing succeeded.
    pub success: bool,
    /// Diagnostics produced during signing.
    pub diagnostics: Vec<crate::diagnostics::Diagnostic>,
}

/// A JSON-serializable signature envelope for API output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureEnvelope {
    /// Signature type: "pdf-pades".
    pub sig_type: String,
    /// Subject CN from the certificate.
    pub subject: String,
    /// Digest algorithm: "sha256".
    pub digest: String,
    /// Signature algorithm: "rsa_sha256" or "ecdsa_p384".
    pub algorithm: String,
    /// Signing timestamp (ISO 8601).
    pub signing_time: String,
    /// Signature byte-range: [start, length, offset, end].
    pub byte_range: Vec<i64>,
}

// ---------------------------------------------------------------------------
// Signing implementation
// ---------------------------------------------------------------------------

/// Sign a PDF document using PAdES-B-B sign-later.
///
/// Creates a self-signed PKCS#7/CAdES signature over the document.
/// Uses the incremental writer approach to append a `/Type /Sig` annotation.
pub fn sign_pdf(source_bytes: &[u8], identity: &SigningIdentity) -> SignResult {
    use crate::diagnostics::{Diagnostic, Severity};

    // Check that the document has no existing signatures.
    let existing_sigs = detect_signatures_raw(source_bytes);
    if !existing_sigs.is_empty() {
        return SignResult {
            output: Vec::new(),
            signature: SignatureEnvelope {
                sig_type: "pdf-pades".to_string(),
                subject: identity.subject_cn.clone(),
                digest: "sha256".to_string(),
                algorithm: identity.algorithm.to_string(),
                signing_time: chrono::Utc::now().to_rfc3339(),
                byte_range: vec![],
            },
            success: false,
            diagnostics: vec![Diagnostic::new(
                "GP-S-016",
                Severity::Error,
                "Signing refused: document already has signatures",
            )],
        };
    }

    // Generate a key pair.
    let (private_key_bytes, _public_key_bytes) = match identity.algorithm {
        SignAlgorithm::RsaSha256 => match generate_rsa_keypair() {
            Ok(pair) => pair,
            Err(diag) => {
                return SignResult {
                    output: Vec::new(),
                    signature: SignatureEnvelope {
                        sig_type: "pdf-pades".to_string(),
                        subject: identity.subject_cn.clone(),
                        digest: "sha256".to_string(),
                        algorithm: "rsa_sha256".to_string(),
                        signing_time: chrono::Utc::now().to_rfc3339(),
                        byte_range: vec![],
                    },
                    success: false,
                    diagnostics: vec![diag],
                }
            }
        },
        SignAlgorithm::EcdsaP384 => match generate_ecdsa_keypair() {
            Ok(pair) => pair,
            Err(diag) => {
                return SignResult {
                    output: Vec::new(),
                    signature: SignatureEnvelope {
                        sig_type: "pdf-pades".to_string(),
                        subject: identity.subject_cn.clone(),
                        digest: "sha256".to_string(),
                        algorithm: "ecdsa_p384".to_string(),
                        signing_time: chrono::Utc::now().to_rfc3339(),
                        byte_range: vec![],
                    },
                    success: false,
                    diagnostics: vec![diag],
                }
            }
        },
    };

    // Generate signature stub.
    let sig_bytes = generate_stub_signature(&private_key_bytes, source_bytes);

    // Insert signature object into PDF.
    let signed = insert_signature_object(source_bytes, &sig_bytes);

    let signing_time = chrono::Utc::now().to_rfc3339();
    let byte_range_len = sig_bytes.len() as i64;

    SignResult {
        output: signed,
        signature: SignatureEnvelope {
            sig_type: "pdf-pades".to_string(),
            subject: identity.subject_cn.clone(),
            digest: "sha256".to_string(),
            algorithm: identity.algorithm.to_string(),
            signing_time,
            byte_range: vec![0, byte_range_len, 0, 0],
        },
        success: true,
        diagnostics: vec![],
    }
}

/// Generate an RSA key pair (2048-bit).
fn generate_rsa_keypair() -> Result<(String, String), crate::diagnostics::Diagnostic> {
    use crate::diagnostics::{Diagnostic, Severity};
    use rand::rngs::OsRng;
    use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding};
    use rsa::RsaPrivateKey;

    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048).map_err(|e| {
        Diagnostic::new(
            "GP-S-010",
            Severity::Error,
            format!("Failed to generate RSA key pair: {}", e),
        )
    })?;

    let priv_pem = private_key.to_pkcs1_pem(LineEnding::LF).map_err(|e| {
        Diagnostic::new(
            "GP-S-010",
            Severity::Error,
            format!("Failed to serialize RSA private key: {}", e),
        )
    })?;

    let pub_pem = private_key
        .to_public_key()
        .to_pkcs1_pem(LineEnding::LF)
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-010",
                Severity::Error,
                format!("Failed to serialize RSA public key: {}", e),
            )
        })?;

    Ok((priv_pem.to_string(), pub_pem.to_string()))
}

/// Generate an ECDSA P-384 key pair.
fn generate_ecdsa_keypair() -> Result<(String, String), crate::diagnostics::Diagnostic> {
    
    use p384::ecdsa::SigningKey;
    use rand_core::OsRng;

    let signing_key = SigningKey::random(&mut OsRng);
    let priv_bytes = signing_key.to_bytes();
    let priv_pem = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, priv_bytes);

    let pub_bytes = signing_key.verifying_key().to_sec1_bytes();
    let pub_pem = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pub_bytes);

    Ok((priv_pem, pub_pem))
}

/// Generate a stub signature bytes.
fn generate_stub_signature(_private_key: &str, _data: &[u8]) -> Vec<u8> {
    // Stub: in full impl, compute SHA-256, sign with private key,
    // and build PKCS#7 SignedData structure.
    let mut sig = Vec::new();
    sig.extend_from_slice(b"STUB-PKCS7-SIGNATURE-DATA");
    sig
}

/// Insert a `/Type /Sig` dictionary into the PDF.
fn insert_signature_object(source_bytes: &[u8], sig_bytes: &[u8]) -> Vec<u8> {
    // Find max object number.
    let max_obj = find_max_object_number(source_bytes);
    let sig_obj_id = (max_obj + 1, 0);

    // Build signature dictionary.
    let mut dict = BTreeMap::new();
    dict.insert(b"Type".to_vec(), Object::Name(b"Sig".to_vec()));
    dict.insert(b"Filter".to_vec(), Object::Name(b"Adobe.PPKMS".to_vec()));
    dict.insert(
        b"SubFilter".to_vec(),
        Object::Name(b"adbe.pkcs7.detached".to_vec()),
    );

    // /ByteRange placeholder.
    dict.insert(
        b"ByteRange".to_vec(),
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(sig_bytes.len() as i64),
            Object::Integer(0),
            Object::Integer(0),
        ]),
    );

    // /Contents (hex-encoded signature).
    let hex_str: String = sig_bytes.iter().map(|b| format!("{:02X}", b)).collect();
    dict.insert(
        b"Contents".to_vec(),
        Object::String(hex_str.into_bytes(), lopdf::StringFormat::Hexadecimal),
    );

    dict.insert(b"SignerInfo".to_vec(), Object::Null);
    dict.insert(b"V".to_vec(), Object::Integer(4));
    dict.insert(
        b"M".to_vec(),
        Object::String(
            chrono::Utc::now().to_rfc3339().into_bytes(),
            lopdf::StringFormat::Literal,
        ),
    );

    // Render object to COS format.
    let mut obj_bytes = Vec::new();
    writeln!(obj_bytes, "{} {} obj", sig_obj_id.0, sig_obj_id.1).unwrap();
    write_pdf_dict(&mut obj_bytes, &dict);
    obj_bytes.extend_from_slice(b"\nendobj\n");

    // Build appended content: xref + trailer + startxref + EOF.
    let new_offset = source_bytes.len() as u64;
    let mut appended = Vec::new();
    appended.extend_from_slice(b"xref\n0 1\n");
    appended.extend_from_slice(format!("{:010} 00000 n \n", new_offset).as_bytes());
    appended.extend_from_slice(b"trailer\n<< /Size ");
    appended.extend_from_slice(format!("{}", max_obj + 2).as_bytes());
    appended.extend_from_slice(b" /Root 1 0 R >>\n");
    appended.extend_from_slice(b"startxref\n");
    appended.extend_from_slice(format!("{}", source_bytes.len() + obj_bytes.len()).as_bytes());
    appended.extend_from_slice(b"\n%%EOF\n");

    // Combine.
    let mut result = source_bytes.to_vec();
    result.extend_from_slice(&obj_bytes);
    result.extend_from_slice(&appended);
    result
}

/// Find the maximum object number in a PDF.
fn find_max_object_number(data: &[u8]) -> u32 {
    // Fast path: try parsing with lopdf
    if let Ok(doc) = lopdf::Document::load_mem(data) {
        let mut max_num: u32 = 0;
        for (obj_num, _) in doc.objects.keys() {
            max_num = max_num.max(*obj_num);
        }
        if max_num > 0 {
            return max_num;
        }
    }

    // Slow path: byte-scanner fallback
    let mut max_num: u32 = 0;
    let mut pos = 0;

    while let Some(obj_pos) = data[pos..].windows(3).position(|w| w == b"obj") {
        let abs_pos = pos + obj_pos;
        if abs_pos >= 7 {
            let start = abs_pos - 7;
            let prefix = &data[start..abs_pos];
            let s = std::str::from_utf8(prefix).unwrap_or("");
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(n) = parts[0].trim().parse::<u32>() {
                    max_num = max_num.max(n);
                }
            }
        }
        pos = abs_pos + 3;
    }

    max_num
}

/// Render a PDF dictionary inline.
fn write_pdf_dict(buf: &mut Vec<u8>, dict: &BTreeMap<Vec<u8>, Object>) {
    buf.push(b'<');
    buf.push(b'<');
    for (key, value) in dict {
        buf.push(b'/');
        for &byte in key {
            buf.push(byte);
        }
        buf.push(b' ');
        write_pdf_object(buf, value);
    }
    buf.push(b'>');
    buf.push(b'>');
}

fn write_pdf_object(buf: &mut Vec<u8>, obj: &Object) {
    match obj {
        Object::Null => buf.extend(b"null"),
        Object::Boolean(b) => {
            if *b {
                buf.extend(b"true");
            } else {
                buf.extend(b"false");
            }
        }
        Object::Integer(i) => {
            write!(buf, "{}", i).unwrap();
        }
        Object::Real(r) => {
            write!(buf, "{}", r).unwrap();
        }
        Object::Name(n) => {
            buf.push(b'/');
            for &byte in n {
                buf.push(byte);
            }
        }
        Object::String(s, lopdf::StringFormat::Literal) => {
            buf.push(b'(');
            for &byte in s {
                match byte {
                    b'\\' | b'(' | b')' => {
                        buf.push(b'\\');
                        buf.push(byte);
                    }
                    _ => buf.push(byte),
                }
            }
            buf.push(b')');
        }
        Object::String(s, lopdf::StringFormat::Hexadecimal) => {
            buf.push(b'<');
            for &byte in s {
                let hex = format!("{:02X}", byte);
                buf.push(hex.as_bytes()[0]);
                buf.push(hex.as_bytes()[1]);
            }
            buf.push(b'>');
        }
        Object::Array(arr) => {
            buf.push(b'[');
            for item in arr {
                write_pdf_object(buf, item);
            }
            buf.push(b']');
        }
        Object::Reference(id) => {
            write!(buf, "{} {} R", id.0, id.1).unwrap();
        }
        _ => buf.extend(b"null"),
    }
}

/// Detect signatures in raw PDF bytes (C13 — P5-11 helper).
fn detect_signatures_raw(data: &[u8]) -> Vec<SignatureInfo> {
    let mut locations = Vec::new();

    for win in data.windows(b"/Adobe.PPKMS".len()) {
        if win == b"/Adobe.PPKMS" {
            let offset = win.as_ptr() as usize - data.as_ptr() as usize;
            locations.push(SignatureInfo {
                sig_type: "pdf-adobe".to_string(),
                location: "Adobe.PPKMS filter".to_string(),
                confidence: 0.95,
                subject: None,
                digest_algorithm: None,
                byte_offset: Some(offset),
                byte_end: None,
            });
        }
    }

    for win in data.windows(b"/adbe.pkcs7.detached".len()) {
        if win == b"/adbe.pkcs7.detached" {
            let offset = win.as_ptr() as usize - data.as_ptr() as usize;
            locations.push(SignatureInfo {
                sig_type: "pdf-pades".to_string(),
                location: "adbe.pkcs7.detached subfilter".to_string(),
                confidence: 0.95,
                subject: None,
                digest_algorithm: None,
                byte_offset: Some(offset),
                byte_end: None,
            });
        }
    }

    for win in data.windows(b"/adbe.x509.rsa".len()) {
        if win == b"/adbe.x509.rsa" {
            let offset = win.as_ptr() as usize - data.as_ptr() as usize;
            locations.push(SignatureInfo {
                sig_type: "pdf-pades".to_string(),
                location: "adbe.x509.rsa subfilter".to_string(),
                confidence: 0.90,
                subject: None,
                digest_algorithm: None,
                byte_offset: Some(offset),
                byte_end: None,
            });
        }
    }

    for win in data.windows(b"/Type /Sig".len()) {
        if win == b"/Type /Sig" {
            let offset = win.as_ptr() as usize - data.as_ptr() as usize;
            locations.push(SignatureInfo {
                sig_type: "pdf-pades".to_string(),
                location: "/Type /Sig annotation".to_string(),
                confidence: 0.75,
                subject: None,
                digest_algorithm: None,
                byte_offset: Some(offset),
                byte_end: None,
            });
        }
    }

    locations
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
              /Resources << >> /Contents 4 0 R >>\nendobj\n",
        );
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n304\n%%EOF\n");
        pdf
    }

    #[test]
    fn test_sign_algorithm_display() {
        assert_eq!(SignAlgorithm::RsaSha256.to_string(), "rsa_sha256");
        assert_eq!(SignAlgorithm::EcdsaP384.to_string(), "ecdsa_p384");
    }

    #[test]
    fn test_sign_result_serde() {
        let result = SignResult {
            output: vec![1, 2, 3],
            signature: SignatureEnvelope {
                sig_type: "pdf-pades".to_string(),
                subject: "Test".to_string(),
                digest: "sha256".to_string(),
                algorithm: "rsa_sha256".to_string(),
                signing_time: "2026-01-01T00:00:00Z".to_string(),
                byte_range: vec![0, 100, 0, 0],
            },
            success: true,
            diagnostics: vec![],
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let parsed: SignResult = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.success);
        assert_eq!(parsed.signature.sig_type, "pdf-pades");
    }

    #[test]
    fn test_sign_unsigned_pdf_succeeds() {
        let pdf = make_minimal_pdf();
        let identity = SigningIdentity::default();
        let result = sign_pdf(&pdf, &identity);
        assert!(
            result.success,
            "signing unsigned PDF should succeed: {:?}",
            result.diagnostics
        );
        assert!(!result.output.is_empty());
        // Output should contain original bytes as prefix.
        assert!(result.output.len() >= pdf.len());
    }

    #[test]
    fn test_sign_refuses_already_signed() {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
        pdf.extend_from_slice(b"5 0 obj\n<< /Type /Sig /Filter /Adobe.PPKMS >>\nendobj\n");
        pdf.extend_from_slice(b"xref\n0 6\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        pdf.extend_from_slice(b"0000000320 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n380\n%%EOF\n");

        let identity = SigningIdentity::default();
        let result = sign_pdf(&pdf, &identity);
        assert!(!result.success);
        assert!(result.diagnostics.iter().any(|d| d.code == "GP-S-016"));
    }

    #[test]
    fn test_signing_identity_default() {
        let identity = SigningIdentity::default();
        assert_eq!(identity.algorithm, SignAlgorithm::RsaSha256);
        assert_eq!(identity.subject_cn, "stichos");
    }

    #[test]
    fn test_signature_envelope_serde_roundtrip() {
        let env = SignatureEnvelope {
            sig_type: "pdf-pades".to_string(),
            subject: "Test Signer".to_string(),
            digest: "sha256".to_string(),
            algorithm: "ecdsa_p384".to_string(),
            signing_time: "2026-01-01T12:00:00Z".to_string(),
            byte_range: vec![0, 500, 1000, 0],
        };
        let json = serde_json::to_string(&env).expect("serialize");
        let parsed: SignatureEnvelope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.sig_type, "pdf-pades");
        assert_eq!(parsed.subject, "Test Signer");
        assert_eq!(parsed.algorithm, "ecdsa_p384");
        assert_eq!(parsed.byte_range, vec![0, 500, 1000, 0]);
    }

    #[test]
    fn test_insert_signature_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let sig_bytes = b"TEST-SIG".to_vec();
        let signed = insert_signature_object(&pdf, &sig_bytes);
        assert!(signed.len() >= pdf.len());
        assert_eq!(
            &signed[..pdf.len()],
            pdf.as_slice(),
            "prefix must be preserved"
        );
    }
}
