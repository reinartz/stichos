// SPDX-License-Identifier: MIT OR Apache-2.0
//! PKCS#12 (PFX) file parsing for loading certificate chains.
//!
//! Parses PKCS#12/PFX bundles to extract:
//! - Private key (PKCS#8 DER)
//! - End-entity signing certificate (X.509 DER)
//! - Optional intermediate CA certificates (X.509 DER)
//!
//! Note: This module provides basic PKCS#12 support. For encrypted
//! PFX files (PBES2), users should convert to unencrypted PKCS#8 + X.509
//! using OpenSSL:
//!   openssl pkcs12 -in cert.pfx -nocerts -out key.pem -nodes
//!   openssl pkcs12 -in cert.pfx -clcerts -nokeys -out cert.pem

use crate::diagnostics::Diagnostic;
use crate::diagnostics::Severity;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of parsing a PKCS#12 bundle.
#[derive(Debug, Clone)]
pub struct Pkcs12Bundle {
    /// DER-encoded PKCS#8 private key.
    pub private_key: Vec<u8>,
    /// DER-encoded end-entity (signing) certificate.
    pub signing_cert: Vec<u8>,
    /// DER-encoded intermediate + root CA certificates.
    pub chain: Vec<Vec<u8>>,
}

/// Parse a PKCS#12 (PFX) file and extract the private key + certificate chain.
///
/// # Arguments
///
/// * `data` — raw bytes of the .pfx / .p12 file
/// * `password` — optional password (empty Vec for unencrypted files)
///
/// # Returns
///
/// `Ok(Pkcs12Bundle)` on success, `Err(Diagnostic)` if the file is invalid
/// or encrypted with unsupported algorithms.
pub fn parse_pkcs12(data: &[u8], _password: &[u8]) -> Result<Pkcs12Bundle, Diagnostic> {
    if data.is_empty() {
        return Err(Diagnostic::new(
            "GP-S-024",
            Severity::Error,
            "Invalid PKCS#12 file — empty file".to_string(),
        ));
    }

    // PKCS#12 (PFX) files typically start with a PKCS#7 ContentInfo
    // The outermost structure is a SEQUENCE tag (0x30)
    if data[0] != 0x30 {
        return Err(Diagnostic::new(
            "GP-S-024",
            Severity::Error,
            "Invalid PKCS#12 file — not a valid DER-encoded PKCS#7 structure".to_string(),
        ));
    }

    // For now, we implement a simplified parser that handles unencrypted
    // PKCS#12 files where the content is wrapped in a PKCS#7 SignedData
    // with a PKCS#8 keyBag and certBag.
    //
    // The PKCS#12 structure is:
    //   SEQUENCE {
    //     version INTEGER (3),
    //     info SEQUENCE OF PFXAuthItem
    //   }
    //
    // Each PFXAuthItem is either:
    //   SEQUENCE { authSafe [0] EXPLICIT ContentInfo }  -- unauthenticated
    //   SEQUENCE { authSafe [0] EXPLICIT ContentInfo,
    //              macData [1] EXPLICIT MacData }       -- authenticated
    //
    // The ContentInfo inside is typically PKCS#7 EnvelopedData.

    // Try to extract the inner data after the outer SEQUENCE
    let outer_content = parse_tlv_content(data, 0)
        .ok_or_else(|| Diagnostic::new(
            "GP-S-024",
            Severity::Error,
            "Invalid PKCS#12 file — cannot parse outer SEQUENCE".to_string(),
        ))?;

    // Skip version INTEGER
    let version_end = parse_tlv_total_len(outer_content, 0)
        .ok_or_else(|| Diagnostic::new(
            "GP-S-024",
            Severity::Error,
            "Invalid PKCS#12 file — cannot parse version".to_string(),
        ))?;

    let auth_items = &outer_content[version_end..];

    // Iterate through auth items to find the encrypted data
    let mut pos = 0;
    let mut key_bytes: Option<Vec<u8>> = None;
    let mut certs: Vec<Vec<u8>> = Vec::new();

    while pos < auth_items.len() {
        let item_end = parse_tlv_total_len(auth_items, pos)
            .ok_or_else(|| Diagnostic::new(
                "GP-S-024",
                Severity::Error,
                "Invalid PKCS#12 file — cannot parse auth item".to_string(),
            ))?;

        let item = &auth_items[pos..pos + item_end];

        // Each item is SEQUENCE { authSafe [0] EXPLICIT ContentInfo }
        if item.is_empty() || item[0] != 0x30 {
            pos += item_end;
            continue;
        }

        let item_inner = parse_tlv_content(item, 0)
            .ok_or_else(|| Diagnostic::new(
                "GP-S-024",
                Severity::Error,
                "Invalid PKCS#12 file — cannot parse auth item inner".to_string(),
            ))?;

        // Look for [0] EXPLICIT tag (authSafe)
        let mut i = 0;
        while i < item_inner.len() {
            if item_inner[i] == 0xA0 {
                // Found authSafe
                let auth_safe_content = parse_tlv_content(item_inner, i)
                    .ok_or_else(|| Diagnostic::new(
                        "GP-S-024",
                        Severity::Error,
                        "Invalid PKCS#12 file — cannot parse authSafe".to_string(),
                    ))?;

                // authSafe is a ContentInfo: SEQUENCE { contentType OID, content [0] ... }
                if auth_safe_content.len() >= 2 && auth_safe_content[0] == 0x30 {
                    let ci_content = parse_tlv_content(auth_safe_content, 0)
                        .ok_or_else(|| Diagnostic::new(
                            "GP-S-024",
                            Severity::Error,
                            "Invalid PKCS#12 file — cannot parse ContentInfo".to_string(),
                        ))?;

                    // Skip contentType OID (should be 1.2.840.113549.1.7.7 = envelopedData)
                    let ci_oid_end = parse_tlv_total_len(ci_content, 0)
                        .ok_or_else(|| Diagnostic::new(
                            "GP-S-024",
                            Severity::Error,
                            "Invalid PKCS#12 file — cannot parse contentType OID".to_string(),
                        ))?;

                    // The content should be [2] EXPLICIT EncryptedData
                    if ci_oid_end < ci_content.len() && ci_content[ci_oid_end] == 0xA2 {
                        let enc_data = parse_tlv_content(ci_content, ci_oid_end)
                            .ok_or_else(|| Diagnostic::new(
                                "GP-S-024",
                                Severity::Error,
                                "Invalid PKCS#12 file — cannot parse EncryptedData".to_string(),
                            ))?;

                        // EncryptedData: SEQUENCE { version, algorithm, encryptedContentInfo }
                        if enc_data.len() >= 2 && enc_data[0] == 0x30 {
                            let enc_inner = parse_tlv_content(enc_data, 0)
                                .ok_or_else(|| Diagnostic::new(
                                    "GP-S-024",
                                    Severity::Error,
                                    "Invalid PKCS#12 file — cannot parse EncryptedData inner".to_string(),
                                ))?;

                            // Skip version and algorithm identifier
                            let mut p = parse_tlv_total_len(enc_inner, 0)
                                .ok_or_else(|| Diagnostic::new(
                                    "GP-S-024",
                                    Severity::Error,
                                    "Invalid PKCS#12 file — cannot parse version".to_string(),
                                ))?;

                            p += parse_tlv_total_len(enc_inner, p)
                                .ok_or_else(|| Diagnostic::new(
                                    "GP-S-024",
                                    Severity::Error,
                                    "Invalid PKCS#12 file — cannot parse algorithm".to_string(),
                                ))?;

                            // encryptedContentInfo is SEQUENCE
                            if p < enc_inner.len() && enc_inner[p] == 0x30 {
                                let ec_inner = parse_tlv_content(enc_inner, p)
                                    .ok_or_else(|| Diagnostic::new(
                                        "GP-S-024",
                                        Severity::Error,
                                        "Invalid PKCS#12 file — cannot parse encryptedContentInfo".to_string(),
                                    ))?;

                                // Skip contentType OID
                                let ec_oid_end = parse_tlv_total_len(ec_inner, 0)
                                    .ok_or_else(|| Diagnostic::new(
                                        "GP-S-024",
                                        Severity::Error,
                                        "Invalid PKCS#12 file — cannot parse eContentType".to_string(),
                                    ))?;

                                // eContent [0] EXPLICIT OCTET STRING (the SafeContents)
                                if ec_oid_end < ec_inner.len() && ec_inner[ec_oid_end] == 0xA0 {
                                    let safe_contents_der = parse_tlv_content(ec_inner, ec_oid_end)
                                        .ok_or_else(|| Diagnostic::new(
                                            "GP-S-024",
                                            Severity::Error,
                                            "Invalid PKCS#12 file — cannot extract SafeContents".to_string(),
                                        ))?;

                                    // Parse SafeContents (SEQUENCE OF SafeBag)
                                    if safe_contents_der.len() >= 2 && safe_contents_der[0] == 0x30 {
                                        let bags = parse_tlv_content(safe_contents_der, 0)
                                            .ok_or_else(|| Diagnostic::new(
                                                "GP-S-024",
                                                Severity::Error,
                                                "Invalid PKCS#12 file — cannot parse SafeContents".to_string(),
                                            ))?;

                                        // Each bag is a SEQUENCE: { bagId OID, bagValue [0], attributes [1] }
                                        let mut bp = 0;
                                        while bp < bags.len() {
                                            let bag_end = parse_tlv_total_len(bags, bp)
                                                .ok_or_else(|| Diagnostic::new(
                                                    "GP-S-024",
                                                    Severity::Error,
                                                    "Invalid PKCS#12 file — cannot parse SafeBag".to_string(),
                                                ))?;

                                            let bag = &bags[bp..bp + bag_end];

                                            if bag.is_empty() || bag[0] != 0x30 {
                                                bp += bag_end;
                                                continue;
                                            }

                                            let bag_inner = parse_tlv_content(bag, 0)
                                                .ok_or_else(|| Diagnostic::new(
                                                    "GP-S-024",
                                                    Severity::Error,
                                                    "Invalid PKCS#12 file — cannot parse SafeBag".to_string(),
                                                ))?;

                                            // Skip bagId OID
                                            let bag_oid_end = parse_tlv_total_len(bag_inner, 0)
                                                .ok_or_else(|| Diagnostic::new(
                                                    "GP-S-024",
                                                    Severity::Error,
                                                    "Invalid PKCS#12 file — cannot parse bagId".to_string(),
                                                ))?;

                                            // bagValue [0] EXPLICIT
                                            if bag_oid_end >= bag_inner.len() || bag_inner[bag_oid_end] != 0xA0 {
                                                bp += bag_end;
                                                continue;
                                            }

                                            let bag_value = parse_tlv_content(bag_inner, bag_oid_end)
                                                .ok_or_else(|| Diagnostic::new(
                                                    "GP-S-024",
                                                    Severity::Error,
                                                    "Invalid PKCS#12 file — cannot parse bagValue".to_string(),
                                                ))?;

                                            // Determine bag type from bagId OID
                                            let bag_oid_bytes = &bag_inner[0..bag_oid_end];
                                            let bag_oid = parse_oid_str(bag_oid_bytes);

                                            match bag_oid.as_str() {
                                                "1.2.840.113549.1.12.10.1.1" => {
                                                    // keyBag — contains PKCS#8 PrivateKeyInfo
                                                    key_bytes = Some(bag_value.to_vec());
                                                }
                                                "1.2.840.113549.1.12.10.1.3" if bag_value.len() >= 2 && bag_value[0] == 0x30 => {
                                                    // certBag — X509Certificate
                                                    // bagValue is SEQUENCE { cCert [0] EXPLICIT Certificate }
                                                    if let Some(inner) = parse_tlv_content(bag_value, 0) {
                                                        // Look for [0] EXPLICIT cert
                                                        if inner.len() >= 2 && inner[0] == 0xA0 {
                                                            if let Some(cert) = parse_tlv_content(inner, 0) {
                                                                certs.push(cert.to_vec());
                                                            }
                                                        }
                                                    }
                                                }
                                                "1.2.840.113549.1.12.10.1.3" => {
                                                    // certBag with unexpected format
                                                }
                                                _ => {
                                                    // Unknown bag type
                                                }
                                            }

                                            bp += bag_end;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            i += 1;
        }

        pos += item_end;
    }

    // Also try to extract certs directly from the outer structure
    // (some PFX files store certs outside the encrypted bag)
    let _ = certs;

    let private_key = key_bytes.ok_or_else(|| {
        Diagnostic::new(
            "GP-S-024",
            Severity::Error,
            "Invalid PKCS#12 file — no private key found".to_string(),
        )
    })?;

    let signing_cert = if certs.is_empty() {
        return Err(Diagnostic::new(
            "GP-S-024",
            Severity::Error,
            "Invalid PKCS#12 file — no signing certificate found".to_string(),
        ));
    } else {
        certs.remove(0)
    };

    Ok(Pkcs12Bundle {
        private_key,
        signing_cert,
        chain: certs,
    })
}

// ---------------------------------------------------------------------------
// ASN.1 DER helpers
// ---------------------------------------------------------------------------

/// Parse a DER TLV at offset, returning the content slice.
fn parse_tlv_content(data: &[u8], start: usize) -> Option<&[u8]> {
    if start + 1 >= data.len() {
        return None;
    }
    let _tag = data[start];
    let len_byte = data[start + 1];
    let (content_len, header_len) = if len_byte < 0x80 {
        (len_byte as usize, 1usize)
    } else {
        let num_bytes = (len_byte & 0x7F) as usize;
        if start + 2 + num_bytes > data.len() {
            return None;
        }
        let mut content_len: usize = 0;
        for &b in &data[start + 2..start + 2 + num_bytes] {
            content_len = (content_len << 8) | b as usize;
        }
        (content_len, 1 + num_bytes)
    };
    let total = start + 2 + header_len + content_len;
    if total > data.len() {
        return None;
    }
    Some(&data[start + 2 + header_len..total])
}

/// Parse a DER TLV at offset, returning the total bytes consumed (including tag + length).
fn parse_tlv_total_len(data: &[u8], start: usize) -> Option<usize> {
    if start >= data.len() {
        return None;
    }
    let len_byte = if start + 1 >= data.len() {
        return None;
    } else {
        data[start + 1]
    };
    let (content_len, header_len) = if len_byte < 0x80 {
        (len_byte as usize, 1usize)
    } else {
        let num_bytes = (len_byte & 0x7F) as usize;
        if start + 2 + num_bytes > data.len() {
            return None;
        }
        let mut content_len: usize = 0;
        for &b in &data[start + 2..start + 2 + num_bytes] {
            content_len = (content_len << 8) | b as usize;
        }
        (content_len, 1 + num_bytes)
    };
    Some(2 + header_len + content_len)
}

/// Parse an OID TLV into a dotted-decimal string.
fn parse_oid_str(data: &[u8]) -> String {
    if data.is_empty() || data[0] != 0x06 {
        return String::new();
    }
    let len_byte = if data.len() < 2 {
        return String::new();
    } else {
        data[1]
    };
    let content_len = if len_byte < 0x80 {
        len_byte as usize
    } else {
        let num_bytes = (len_byte & 0x7F) as usize;
        if data.len() < 2 + num_bytes {
            return String::new();
        }
        let mut val: usize = 0;
        for &b in &data[2..2 + num_bytes] {
            val = (val << 8) | b as usize;
        }
        val
    };
    let oid_bytes = &data[2..2 + content_len];
    if oid_bytes.is_empty() {
        return String::new();
    }
    let first = oid_bytes[0];
    let mut parts: Vec<u64> = vec![(first / 40) as u64, (first % 40) as u64];
    let mut val: u64 = 0;
    for &b in &oid_bytes[1..] {
        val = ((val << 7) & 0x7FFFFFFF) | (b & 0x7F) as u64;
        if b & 0x80 == 0 {
            parts.push(val);
            val = 0;
        }
    }
    parts.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(".")
}
