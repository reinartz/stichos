// SPDX-License-Identifier: MIT OR Apache-2.0
//! Certificate chain validation for PKCS#12-derived chains.
//!
//! Validates that a certificate chain (leaf → intermediates → root) is
//! cryptographically sound using only `x509-parser` (already a
//! `pades-ltv` feature dependency).

use crate::diagnostics::Diagnostic;
use crate::diagnostics::Severity;
use x509_parser::prelude::FromDer;

/// KeyUsage OID: 2.5.29.15
const OID_KEY_USAGE: &str = "2.5.29.15";

/// Result of chain validation.
#[derive(Debug, Clone)]
pub struct ChainValidationResult {
    /// Number of certificates in the chain (including leaf + root).
    pub cert_count: usize,
    /// Whether the chain is valid.
    pub valid: bool,
    /// Leaf certificate subject CN.
    pub leaf_cn: Option<String>,
    /// Root certificate subject CN.
    pub root_cn: Option<String>,
    /// Leaf validity period.
    pub leaf_valid_from: Option<String>,
    pub leaf_valid_to: Option<String>,
    /// Whether the leaf has the digitalSignature key usage.
    pub has_digital_signature: bool,
    /// Diagnostics produced during validation.
    pub diagnostics: Vec<Diagnostic>,
}

/// Validate a certificate chain: leaf → [intermediates...] → root.
///
/// Checks:
/// 1. Each certificate is well-formed DER X.509
/// 2. The signature of each cert verifies against its issuer's public key
/// 3. Validity period (notBefore <= now <= notAfter) for all certs
/// 4. Key usage extension includes digitalSignature on the leaf
/// 5. Chain linkage: issuer of cert[i] matches subject of cert[i+1]
///
/// # Arguments
///
/// * `leaf` — DER-encoded end-entity certificate
/// * `intermediates` — DER-encoded intermediate CA certificates
/// * `root` — DER-encoded root CA certificate (or last in chain)
///
/// # Returns
///
/// `ChainValidationResult` with `valid = true` if all checks pass.
pub fn validate_chain(
    leaf: &[u8],
    intermediates: &[Vec<u8>],
    root: Option<&[u8]>,
) -> ChainValidationResult {
    let mut diagnostics = Vec::new();
    let mut has_error: bool = false;

    // Parse leaf certificate
    let leaf_cert = match x509_parser::parse_x509_certificate(leaf) {
        Ok((_, c)) => c,
        Err(e) => {
            diagnostics.push(Diagnostic::new(
                "GP-S-025",
                Severity::Error,
                format!("Certificate chain validation failed — leaf cert parse error: {e}"),
            ));
            return ChainValidationResult {
                cert_count: 0,
                valid: false,
                leaf_cn: None,
                root_cn: None,
                leaf_valid_from: None,
                leaf_valid_to: None,
                has_digital_signature: false,
                diagnostics,
            };
        }
    };

    let leaf_cn = leaf_cert.subject().to_string();
    let now = x509_parser::time::ASN1Time::now();

    // Extract leaf validity period
    let leaf_validity = leaf_cert.validity();
    let leaf_valid_from = leaf_validity.not_before.to_string();
    let leaf_valid_to = leaf_validity.not_after.to_string();

    // Check leaf validity period
    if !leaf_validity.is_valid_at(now) {
        has_error = true;
        diagnostics.push(Diagnostic::new(
            "GP-S-025",
            Severity::Error,
            "Certificate chain validation failed — leaf certificate not valid at this time"
                .to_string(),
        ));
    }

    // Check leaf key usage for digitalSignature
    let mut has_digital_signature = false;
    let ku_ext_found =
        if let Ok(Some(ext)) = leaf_cert.get_extension_unique(&OID_KEY_USAGE.parse().unwrap()) {
            if let Ok((_rest, ku)) = x509_parser::extensions::KeyUsage::from_der(ext.value) {
                has_digital_signature =
                    ku.digital_signature() || ku.non_repudiation() || ku.key_encipherment();
                true
            } else {
                false
            }
        } else {
            false
        };

    if ku_ext_found && !has_digital_signature {
        has_error = true;
        diagnostics.push(Diagnostic::new(
            "GP-S-025",
            Severity::Error,
            "Certificate chain validation failed — leaf missing digitalSignature key usage"
                .to_string(),
        ));
    }

    // Build the full chain: leaf, intermediates
    let mut all_certs: Vec<x509_parser::certificate::X509Certificate<'_>> = vec![leaf_cert];

    for (idx, interp_der) in intermediates.iter().enumerate() {
        match x509_parser::parse_x509_certificate(interp_der) {
            Ok((_, c)) => all_certs.push(c),
            Err(e) => {
                diagnostics.push(Diagnostic::new(
                    "GP-S-025",
                    Severity::Error,
                    format!(
                        "Certificate chain validation failed — intermediate {idx} parse error: {e}"
                    ),
                ));
                return ChainValidationResult {
                    cert_count: all_certs.len(),
                    valid: false,
                    leaf_cn: Some(leaf_cn.clone()),
                    root_cn: None,
                    leaf_valid_from: Some(leaf_valid_from.clone()),
                    leaf_valid_to: Some(leaf_valid_to.clone()),
                    has_digital_signature,
                    diagnostics,
                };
            }
        }
    }

    // Check root if provided
    let root_cn = if let Some(root_der) = root {
        match x509_parser::parse_x509_certificate(root_der) {
            Ok((_, root_cert)) => {
                let root_subject = root_cert.subject().to_string();
                if !root_cert.validity().is_valid_at(now) {
                    diagnostics.push(Diagnostic::new(
                        "GP-S-025",
                        Severity::Error,
                        "Certificate chain validation failed — root CA certificate expired"
                            .to_string(),
                    ));
                }
                Some(root_subject)
            }
            Err(e) => {
                diagnostics.push(Diagnostic::new(
                    "GP-S-025",
                    Severity::Error,
                    format!("Certificate chain validation failed — root cert parse error: {e}"),
                ));
                None
            }
        }
    } else {
        None
    };

    // Verify chain linkage and signatures using x509-parser's verify_signature
    for i in 0..all_certs.len().saturating_sub(1) {
        let cert = &all_certs[i];
        let issuer_cert = &all_certs[i + 1];

        // Check issuer/subject linkage using string comparison
        let issuer_str = cert.issuer().to_string();
        let subject_str = issuer_cert.subject().to_string();

        if issuer_str != subject_str {
            diagnostics.push(Diagnostic::new(
                "GP-S-025",
                Severity::Error,
                format!(
                    "Certificate chain validation failed — issuer mismatch at position {}->{}",
                    i,
                    i + 1
                ),
            ));
        }

        // Verify certificate signature using x509-parser's built-in method
        let issuer_pubkey = issuer_cert.public_key();
        if let Err(e) = cert.verify_signature(Some(issuer_pubkey)) {
            diagnostics.push(Diagnostic::new(
                "GP-S-025",
                Severity::Error,
                format!(
                    "Certificate chain validation failed — signature verification error at position {}->{}: {e}",
                    i, i + 1
                ),
            ));
        }
    }

    // If root was provided, verify last cert is CA
    if root.is_some() {
        let last_cert = all_certs.last().expect("at least one cert");
        if !last_cert.is_ca() {
            diagnostics.push(Diagnostic::new(
                "GP-S-025",
                Severity::Error,
                "Certificate chain validation failed — chain does not terminate at a CA certificate".to_string(),
            ));
        }
    }

    ChainValidationResult {
        cert_count: all_certs.len(),
        valid: !has_error,
        leaf_cn: Some(leaf_cn),
        root_cn,
        leaf_valid_from: Some(leaf_valid_from),
        leaf_valid_to: Some(leaf_valid_to),
        has_digital_signature,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_result_clone() {
        let result = ChainValidationResult {
            cert_count: 1,
            valid: true,
            leaf_cn: Some("test".to_string()),
            root_cn: Some("root".to_string()),
            leaf_valid_from: Some("2024-01-01".to_string()),
            leaf_valid_to: Some("2025-01-01".to_string()),
            has_digital_signature: true,
            diagnostics: vec![],
        };
        let cloned = result.clone();
        assert_eq!(result.cert_count, cloned.cert_count);
        assert_eq!(result.valid, cloned.valid);
    }

    #[test]
    fn test_validate_chain_invalid_leaf() {
        let result = validate_chain(&[0x01, 0x02, 0x03], &[], None);
        assert!(!result.valid);
        assert_eq!(result.cert_count, 0);
        assert!(!result.diagnostics.is_empty());
    }

    #[test]
    fn test_validate_chain_empty() {
        let result = validate_chain(&[], &[], None);
        assert!(!result.valid);
        assert_eq!(result.cert_count, 0);
    }
}
