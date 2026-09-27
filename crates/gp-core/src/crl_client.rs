// SPDX-License-Identifier: MIT OR Apache-2.0
//! CRL client: download and parse X.509 v2 Certificate Revocation Lists.
//!
//! Downloads DER-encoded CRLs from CRL Distribution Point URLs (HTTP GET),
//! parses them using `x509-parser`, and checks whether a signing certificate
//! serial number appears in the revoked certificates list.
//!
//! Honest-absent: if the CRL is unreachable, signing proceeds without CRL data,
//! emitting diagnostic `GP-S-021`. If the certificate is expired per CRL, a
//! warning `GP-S-023` is emitted but signing proceeds.
//!
//! See also: RFC 5280, ETSI EN 319 102-5

use crate::diagnostics::Diagnostic;
use crate::diagnostics::Severity;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A parsed CRL entry for a specific serial number.
#[derive(Debug, Clone, Default)]
pub struct CrlEntry {
    /// Whether the serial number was found in the CRL.
    pub found: bool,
    /// Whether the certificate was revoked.
    pub revoked: bool,
    /// The revocation reason code, if revoked.
    pub reason: Option<u8>,
    /// The revocation time, if known.
    pub revocation_time: Option<String>,
}

/// Metadata about a downloaded CRL.
#[derive(Debug, Clone)]
pub struct CrlInfo {
    /// The raw DER-encoded CRL bytes.
    pub der: Vec<u8>,
    /// The issuer of the CRL.
    pub issuer: String,
    /// The last update time.
    pub last_update: Option<String>,
    /// The next update time (if present).
    pub next_update: Option<String>,
    /// The CRL number, if present.
    pub crl_number: Option<String>,
    /// Total number of revoked certificates in this CRL.
    pub revoked_count: usize,
}

/// Result of a CRL check for a signing certificate.
#[derive(Debug, Clone)]
pub struct CrlCheckResult {
    /// Whether the CRL was successfully downloaded and parsed.
    pub success: bool,
    /// Information about the CRL.
    pub crl_info: Option<CrlInfo>,
    /// Whether the signing certificate was found revoked.
    pub entry: CrlEntry,
    /// Diagnostics produced during the CRL exchange.
    pub diagnostics: Vec<Diagnostic>,
}

impl CrlCheckResult {
    /// Create a success result with a parsed CRL.
    pub fn success(crl_info: CrlInfo, entry: CrlEntry) -> Self {
        Self {
            success: true,
            crl_info: Some(crl_info),
            entry,
            diagnostics: Vec::new(),
        }
    }

    /// Create a graceful-failure result (honest-absent).
    pub fn absent(diag: Diagnostic) -> Self {
        Self {
            success: false,
            crl_info: None,
            entry: CrlEntry::default(),
            diagnostics: vec![diag],
        }
    }
}

// ---------------------------------------------------------------------------
// CRL downloader
// ---------------------------------------------------------------------------

/// Download a DER-encoded CRL from a URL.
///
/// Uses `reqwest` (rustls-tls) to perform an HTTP GET against the CRL
/// Distribution Point URL.
#[cfg(feature = "pades-ltv")]
pub async fn download_crl(
    url: &str,
    timeout_secs: u64,
) -> Result<Vec<u8>, Diagnostic> {
    use reqwest::Client;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-021",
                Severity::Warning,
                format!("Failed to build HTTP client for CRL download: {}", e),
            )
        })?;

    let response = client
        .get(url)
        .header("Accept", "application/pkix-crl")
        .send()
        .await
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-021",
                Severity::Warning,
                format!("CRL download failed: {}", e),
            )
        })?;

    if !response.status().is_success() {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Warning,
            format!(
                "CRL server returned HTTP {}: {}",
                response.status(),
                response.status().canonical_reason().unwrap_or("unknown")
            ),
        ));
    }

    let body = response
        .bytes()
        .await
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-021",
                Severity::Warning,
                format!("Failed to read CRL response body: {}", e),
            )
        })?;

    if body.is_empty() {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Warning,
            "CRL server returned empty response".to_string(),
        ));
    }

    Ok(body.to_vec())
}

// ---------------------------------------------------------------------------
// CRL parser
// ---------------------------------------------------------------------------

/// Parse a DER-encoded CRL and check if a serial number is revoked.
///
/// Uses `x509-parser` for CRL parsing. The serial number is compared
/// as a raw byte sequence.
pub fn check_crl(
    crl_der: &[u8],
    serial_number: &[u8],
) -> Result<CrlCheckResult, Diagnostic> {
    use x509_parser::prelude::FromDer;
    use x509_parser::revocation_list::CertificateRevocationList;

    let (_, crl) = CertificateRevocationList::from_der(crl_der).map_err(|e| {
        Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            format!("Failed to parse CRL DER: {}", e),
        )
    })?;

    let issuer = format!("{}", crl.issuer());

    let last_update = crl.last_update().to_string();
    let next_update = crl.next_update().map(|t| t.to_string());
    let crl_number = crl.crl_number().map(|n| n.to_str_radix(16));
    let revoked_count = crl.iter_revoked_certificates().count();

    let crl_info = CrlInfo {
        der: crl_der.to_vec(),
        issuer,
        last_update: Some(last_update),
        next_update,
        crl_number,
        revoked_count,
    };

    // Check if the serial number is in the revoked list
    let mut entry = CrlEntry::default();

    for revoked_cert in crl.iter_revoked_certificates() {
        // Compare serial numbers as raw bytes (big-endian, minimal encoding)
        let cert_serial = revoked_cert.raw_serial();
        if cert_serial == serial_number {
            entry.found = true;
            entry.revoked = true;
            entry.reason = revoked_cert
                .reason_code()
                .map(|(_, rc)| rc.0);
            entry.revocation_time = Some(revoked_cert.revocation_date.to_string());
            break;
        }
    }

    Ok(CrlCheckResult::success(crl_info, entry))
}

/// Download and check a CRL for a serial number in one call.
#[cfg(feature = "pades-ltv")]
pub async fn fetch_and_check_crl(
    url: &str,
    serial_number: &[u8],
    timeout_secs: u64,
) -> Result<CrlCheckResult, Diagnostic> {
    let crl_der = download_crl(url, timeout_secs).await?;
    check_crl(&crl_der, serial_number)
}

/// Extract CRL Distribution Point URLs from a certificate.
///
/// Parses the certificate using `x509-parser` and returns the URIs
/// from the CRLDistributionPoints extension.
pub fn extract_crl_urls(cert_der: &[u8]) -> Result<Vec<String>, Diagnostic> {
    use x509_parser::extensions::CRLDistributionPoints;
    use x509_parser::oid_registry::OID_X509_EXT_CRL_DISTRIBUTION_POINTS;
    use x509_parser::prelude::FromDer;

    let (_, cert) = x509_parser::parse_x509_certificate(cert_der).map_err(|e| {
        Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            format!("Failed to parse certificate for CRL URLs: {}", e),
        )
    })?;

    let mut urls = Vec::new();

    if let Ok(Some(ext)) = cert.get_extension_unique(&OID_X509_EXT_CRL_DISTRIBUTION_POINTS) {
        if let Ok((_, crl_dps)) = CRLDistributionPoints::from_der(ext.value) {
            for point in &crl_dps.points {
                if let Some(distribution_point_name) = &point.distribution_point {
                    match distribution_point_name {
                        x509_parser::extensions::DistributionPointName::FullName(names) => {
                            for name in names {
                                if let x509_parser::extensions::GeneralName::URI(uri) = name {
                                    urls.push(uri.to_string());
                                }
                            }
                        }
                        x509_parser::extensions::DistributionPointName::NameRelativeToCRLIssuer(_) => {
                            // Relative names require the issuer's CRL — skip for now
                        }
                    }
                }
            }
        }
    }

    Ok(urls)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crl_entry_default() {
        let entry = CrlEntry::default();
        assert!(!entry.found);
        assert!(!entry.revoked);
        assert!(entry.reason.is_none());
        assert!(entry.revocation_time.is_none());
    }

    #[test]
    fn test_crl_check_result_absent() {
        let diag = Diagnostic::new("GP-S-021", Severity::Warning, "CRL unreachable".to_string());
        let result = CrlCheckResult::absent(diag.clone());
        assert!(!result.success);
        assert!(result.crl_info.is_none());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "GP-S-021");
    }
}
