// SPDX-License-Identifier: MIT OR Apache-2.0
//! RFC 3161 timestamp client (P7-5 Wave 1 — B-BT).
//!
//! Builds DER-encoded `TimestampRequest` structures, sends them to a TSA server
//! via HTTP POST, and parses `TimestampResponse` (PKIStatus + CMS SignedData).
//!
//! Honest-absent: if the TSA is unreachable, callers fall back to PAdES-B-B
//! with diagnostic code `GP-S-020`.
//!
//! See also: `docs/adr/ADR-0012-pades-t-ltv.md`, RFC 3161 §2.5

use crate::diagnostics::Diagnostic;
use crate::diagnostics::Severity;

// ---------------------------------------------------------------------------
// RFC 3161 OID constants
// ---------------------------------------------------------------------------

/// OID for the digestAlgorithm sha-256 (2.16.840.1.101.3.4.2.1)
const OID_SHA256: &str = "2.16.840.1.101.3.4.2.1";
/// Content type OID for timestamp query (1.2.840.113549.1.9.16.1.4)
const OID_CT_TIMESTAMP_QUERY: &str = "1.2.840.113549.1.9.16.1.4";
/// Content type OID for timestamp reply (1.2.840.113549.1.9.16.1.5)
const OID_CT_TIMESTAMP_REPLY: &str = "1.2.840.113549.1.9.16.1.5";
/// OID for message digest in CMS (1.2.840.113549.1.9.4)
#[allow(dead_code)]
const OID_MESSAGE_DIGEST: &str = "1.2.840.113549.1.9.4";

// ---------------------------------------------------------------------------
// Manual DER helpers (copied from signature.rs to avoid coupling)
// ---------------------------------------------------------------------------

/// Encode a DER length into the buffer
fn encode_der_length(len: usize, buf: &mut Vec<u8>) {
    if len < 0x80 {
        buf.push(len as u8);
    } else {
        let bytes = usize_to_der_bytes(len);
        buf.push(0x80 | (bytes.len() as u8));
        buf.extend_from_slice(&bytes);
    }
}

fn usize_to_der_bytes(mut val: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    while val > 0 {
        bytes.insert(0, (val & 0xFF) as u8);
        val >>= 8;
    }
    bytes
}

/// TLV encoding helper
fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + value.len());
    out.push(tag);
    encode_der_length(value.len(), &mut out);
    out.extend_from_slice(value);
    out
}

/// Build a SEQUENCE wrapping a single byte slice.
#[allow(dead_code)]
fn sequence(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + value.len());
    out.push(0x30);
    encode_der_length(value.len(), &mut out);
    out.extend_from_slice(value);
    out
}

/// Build a SET wrapping a single byte slice.
#[allow(dead_code)]
fn set(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + value.len());
    out.push(0x31);
    encode_der_length(value.len(), &mut out);
    out.extend_from_slice(value);
    out
}

/// Build a SEQUENCE by concatenating multiple byte slices.
fn seq_parts(parts: &[&[u8]]) -> Vec<u8> {
    let total: usize = parts.iter().map(|p| p.len()).sum();
    let mut out = Vec::with_capacity(2 + total);
    out.push(0x30);
    encode_der_length(total, &mut out);
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

/// INTEGER encoding
fn integer(value: &[u8]) -> Vec<u8> {
    tlv(0x02, value)
}

/// OCTET STRING encoding
fn octet_string(value: &[u8]) -> Vec<u8> {
    tlv(0x04, value)
}

/// Context-specific tag [0] EXPLICIT
fn ctx_explicit0(value: &[u8]) -> Vec<u8> {
    tlv(0xA0, value)
}

/// Context-specific tag [1] EXPLICIT
fn ctx_explicit1(value: &[u8]) -> Vec<u8> {
    tlv(0xA1, value)
}

/// Encode OID bytes from a dotted-decimal string.
fn oid_bytes(oid: &str) -> Vec<u8> {
    let parts: Vec<u64> = oid.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.is_empty() {
        return vec![0x06, 0x00];
    }
    let mut out = Vec::new();
    out.push(0x06); // OID tag
    let first: u64 = parts[0];
    let second: u64 = if parts.len() > 1 { parts[1] } else { 0 };
    let mut encoded: Vec<u8> = vec![((first * 40) + second) as u8];
    for &p in &parts[2..] {
        if p == 0 {
            encoded.push(0x00);
        } else {
            // Encode p in base-128 (RFC 5280 §4.1).
            // First count how many 7-bit chunks we need.
            let mut val = p;
            let mut num_bytes = 0;
            while val > 0 {
                num_bytes += 1;
                val >>= 7;
            }
            // Build bytes from least significant to most significant.
            let mut tmp = Vec::with_capacity(num_bytes as usize);
            val = p;
            for _ in 0..num_bytes {
                tmp.push((val & 0x7F) as u8);
                val >>= 7;
            }
            // Reverse so most significant is first.
            tmp.reverse();
            // Set the high bit on all bytes except the last.
            for i in 0..tmp.len() - 1 {
                tmp[i] |= 0x80;
            }
            encoded.extend_from_slice(&tmp);
        }
    }
    out.push(encoded.len() as u8);
    out.extend_from_slice(&encoded);
    out
}

/// Parse a DER length field. Returns (content_length, total_bytes_consumed).
fn parse_der_length(data: &[u8]) -> Option<(usize, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first < 0x80 {
        return Some((first as usize, 1));
    }
    let num_bytes = (first & 0x7F) as usize;
    if num_bytes == 0 || 1 + num_bytes > data.len() {
        return None;
    }
    let len_bytes = &data[1..1 + num_bytes];
    let mut length: usize = 0;
    for &b in len_bytes {
        length = (length << 8) | b as usize;
    }
    Some((length, 1 + num_bytes))
}

/// Parse a DER SEQUENCE and return the inner content slice.
fn parse_sequence(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 2 {
        return None;
    }
    if data[0] != 0x30 {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[1..])?;
    let total = 1 + len_off;
    let (content_len, _) = parse_der_length(&data[1..])?;
    if total + content_len > data.len() {
        return None;
    }
    Some(&data[total..total + content_len])
}

/// Parse a DER SET and return the inner content slice.
#[allow(dead_code)]
fn parse_set(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 2 {
        return None;
    }
    if data[0] != 0x31 {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[1..])?;
    let total = 1 + len_off;
    let (content_len, _) = parse_der_length(&data[1..])?;
    if total + content_len > data.len() {
        return None;
    }
    Some(&data[total..total + content_len])
}

/// Parse a DER OCTET STRING and return the inner content slice.
#[allow(dead_code)]
fn parse_octet_string(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 2 {
        return None;
    }
    if data[0] != 0x04 {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[1..])?;
    let total = 1 + len_off;
    let (content_len, _) = parse_der_length(&data[1..])?;
    if total + content_len > data.len() {
        return None;
    }
    Some(&data[total..total + content_len])
}

/// Parse a DER INTEGER and return the inner content bytes.
fn parse_integer_bytes(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || data[0] != 0x02 {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[1..])?;
    let total = 1 + len_off;
    let (content_len, _) = parse_der_length(&data[1..])?;
    if total + content_len > data.len() {
        return None;
    }
    Some(data[total..total + content_len].to_vec())
}

/// Parse a DER OID and return the dotted-decimal string.
fn parse_oid(data: &[u8]) -> Option<String> {
    if data.is_empty() || data[0] != 0x06 {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[1..])?;
    let (content_len, _) = parse_der_length(&data[1..])?;
    let oid_bytes = &data[1 + len_off..1 + len_off + content_len];
    if oid_bytes.is_empty() {
        return None;
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
    Some(
        parts
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Find a context-specific tag by its raw byte value, returning the inner content.
#[allow(dead_code)]
fn find_context_tag(data: &[u8], tag: u8) -> Option<Vec<u8>> {
    let mut pos = 0;
    while pos < data.len() {
        if pos + 1 >= data.len() {
            return None;
        }
        if data[pos] == tag {
            let (_, len_off) = parse_der_length(&data[pos + 1..])?;
            let total = 1 + len_off;
            let (content_len, _) = parse_der_length(&data[pos + 1..])?;
            let inner = data[pos + total..pos + total + content_len].to_vec();
            return Some(inner);
        }
        let (_, len_off) = parse_der_length(&data[pos + 1..])?;
        let total = 1 + len_off;
        let (content_len, _) = parse_der_length(&data[pos + 1..])?;
        let tlv_len = total + content_len;
        if pos + tlv_len > data.len() {
            return None;
        }
        pos += tlv_len;
    }
    None
}

/// Consume one TLV at the given position, return position after it.
#[allow(dead_code)]
fn consume_tlv(data: &[u8], start: usize) -> Option<usize> {
    if start + 1 >= data.len() {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[start + 1..])?;
    let total = 1 + len_off;
    let (content_len, _) = parse_der_length(&data[start + 1..])?;
    Some(start + total + content_len)
}

// ---------------------------------------------------------------------------
// Timestamp request types
// ---------------------------------------------------------------------------

/// A RFC 3161 timestamp request.
#[derive(Debug, Clone)]
pub struct TimestampRequest {
    /// The SHA-256 hash to be timestamped.
    pub message_hash: Vec<u8>,
    /// Optional nonce for replay protection.
    pub nonce: Option<u64>,
}

impl TimestampRequest {
    /// Build a new timestamp request for the given SHA-256 hash.
    pub fn new(message_hash: Vec<u8>) -> Self {
        Self {
            message_hash,
            nonce: None,
        }
    }

    /// Set an optional nonce for replay protection.
    pub fn with_nonce(mut self, nonce: u64) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// Encode this request as a DER-encoded RFC 3161 `TimestampRequest`.
    pub fn to_der(&self) -> Vec<u8> {
        // AlgorithmIdentifier: SEQUENCE { OID sha256, NULL }
        let sha256_oid = oid_bytes(OID_SHA256);
        let algo_parts: Vec<Vec<u8>> = vec![sha256_oid, vec![0x05, 0x00]];
        let algo_id = seq_parts(&algo_parts.iter().map(|p| p.as_slice()).collect::<Vec<_>>());

        // messageImprint: SEQUENCE { algorithm, hashedMessage }
        let hash_octet = octet_string(&self.message_hash);
        let message_imprint = seq_parts(&[&algo_id[..], hash_octet.as_slice()]);

        // version: INTEGER 1
        let version = integer(&[0x01]);

        // Build optional nonce [1] EXPLICIT
        let mut parts: Vec<Vec<u8>> = vec![version, message_imprint];

        if let Some(nonce) = self.nonce {
            let nonce_bytes = u64_to_der_int(nonce);
            let nonce_tagged = ctx_explicit1(&nonce_bytes);
            parts.push(nonce_tagged);
        }

        // TimestampRequest ::= SEQUENCE { version, messageImprint, [optional...] }
        let request_der = seq_parts(&parts.iter().map(|p| p.as_slice()).collect::<Vec<_>>());

        // Wrap in ContentInfo: SEQUENCE { contentType, content [0] EXPLICIT }
        let ct_oid = oid_bytes(OID_CT_TIMESTAMP_QUERY);
        let request_wrapped = ctx_explicit0(&request_der);
        let content_info = seq_parts(&[&ct_oid[..], request_wrapped.as_slice()]);

        content_info
    }
}

/// Encode a u64 as a DER INTEGER (big-endian, minimal).
fn u64_to_der_int(n: u64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    // Skip leading zeros
    let mut start = 0;
    while start < bytes.len() && bytes[start] == 0 {
        start += 1;
    }
    let inner = &bytes[start..];
    // Prepend zero if high bit is set (to ensure positive integer encoding)
    let needs_padding = !inner.is_empty() && inner[0] >= 0x80;
    if needs_padding {
        let mut prefix = vec![0u8];
        prefix.extend_from_slice(inner);
        integer(&prefix)
    } else if inner.is_empty() {
        integer(&[0x00])
    } else {
        integer(inner)
    }
}

// ---------------------------------------------------------------------------
// Timestamp response types
// ---------------------------------------------------------------------------

/// The status returned by the TSA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PKIStatus {
    /// Success — timestamp token is present.
    Granted,
    /// Warning — timestamp granted with modifications.
    GrantedWithMods,
    /// The revocation status of the TSA's certificate could not be determined.
    Rejection,
    /// The TSA could not complete the request.
    Failure,
}

impl std::fmt::Display for PKIStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PKIStatus::Granted => write!(f, "Granted"),
            PKIStatus::GrantedWithMods => write!(f, "GrantedWithMods"),
            PKIStatus::Rejection => write!(f, "Rejection"),
            PKIStatus::Failure => write!(f, "Failure"),
        }
    }
}

/// A parsed RFC 3161 `TimestampResponse`.
#[derive(Debug, Clone)]
pub struct TimestampResponse {
    /// The PKI status from the TSA.
    pub status: PKIStatus,
    /// The raw CMS SignedData DER (only present on success).
    pub timestamp_token: Option<Vec<u8>>,
}

/// Parse a DER-encoded `TimestampResponse` from raw bytes.
///
/// Returns `Err` only on structural parsing failures.
/// A successful parse may still yield `status != Granted`.
pub fn parse_timestamp_response(raw: &[u8]) -> Result<TimestampResponse, Diagnostic> {
    // ContentInfo: SEQUENCE { contentType OID, content [0] EXPLICIT }
    let content_info = parse_sequence(raw).ok_or_else(|| {
        Diagnostic::new(
            "GP-S-020",
            Severity::Error,
            "Timestamp response is not a valid DER SEQUENCE".to_string(),
        )
    })?;

    // Check contentType OID
    // content_info[0] is the OID tag (0x06), content_info[1] is the length
    let oid_tag = content_info[0];
    if oid_tag != 0x06 {
        return Err(Diagnostic::new(
            "GP-S-020", Severity::Error,
            format!("ContentInfo: expected OID tag (0x06), got 0x{:02X}", oid_tag),
        ));
    }
    let (_, len_off) = parse_der_length(&content_info[1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-020", Severity::Error, "ContentInfo: invalid OID length".to_string())
    })?;
    let (oid_clen, _) = parse_der_length(&content_info[1..]).ok_or_else(|| {
        Diagnostic::new(
            "GP-S-020", Severity::Error,
            "ContentInfo: invalid OID content length".to_string(),
        )
    })?;
    // Rebuild TLV with tag for parse_oid (which expects tag byte at start)
    let oid_bytes = &content_info[0..1 + len_off + oid_clen];
    let oid_str = parse_oid(oid_bytes).unwrap_or_default();
    if oid_str != OID_CT_TIMESTAMP_REPLY {
        return Err(Diagnostic::new(
            "GP-S-020", Severity::Error,
            format!("ContentInfo OID mismatch: got {}, expected {}", oid_str, OID_CT_TIMESTAMP_REPLY),
        ));
    }

    // Skip the OID TLV to get to the content
    let pos = 1 + len_off + oid_clen;

    // content is [0] EXPLICIT wrapping TimestampReply
    if pos >= content_info.len() || content_info[pos] != 0xA0 {
        return Err(Diagnostic::new(
            "GP-S-020",
            Severity::Error,
            format!(
                "ContentInfo content: expected [0] EXPLICIT (0xA0), got 0x{:02X}",
                content_info.get(pos).unwrap_or(&0)
            ),
        ));
    }
    let (_, loff) = parse_der_length(&content_info[pos + 1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-020", Severity::Error, "ContentInfo: invalid [0] length".to_string())
    })?;
    let (clen, _) = parse_der_length(&content_info[pos + 1..]).ok_or_else(|| {
        Diagnostic::new(
            "GP-S-020",
            Severity::Error,
            "ContentInfo: invalid [0] content length".to_string(),
        )
    })?;
    let reply_start = pos + 1 + loff;
    let reply_bytes = &content_info[reply_start..reply_start + clen];

    // TimestampReply ::= SEQUENCE { status PKIStatus, timeStToken [0] EXPLICIT OPTIONAL }
    let reply_inner = parse_sequence(reply_bytes).ok_or_else(|| {
        Diagnostic::new("GP-S-020", Severity::Error, "TimestampReply is not a valid SEQUENCE".to_string())
    })?;

    // Parse PKIStatus (INTEGER)
    let status_int = parse_integer_bytes(reply_inner).ok_or_else(|| {
        Diagnostic::new(
            "GP-S-020",
            Severity::Error,
            "TimestampReply: cannot parse PKIStatus INTEGER".to_string(),
        )
    })?;

    let status = match status_int.as_slice() {
        &[0x00] => PKIStatus::Granted,
        &[0x01] => PKIStatus::GrantedWithMods,
        &[0x02] => PKIStatus::Rejection,
        &[0x03] => PKIStatus::Failure,
        other => {
            return Err(Diagnostic::new(
                "GP-S-020",
                Severity::Error,
                format!("Unknown PKIStatus value: {:?}", other),
            ));
        }
    };

    // Parse optional timeStToken [0] EXPLICIT
    // Skip past the status INTEGER to find [0].
    // The status INTEGER TLV = 1 (tag) + 1 (len) + 1 (value) = 3 bytes for single-byte value.
    let status_tlv_len = if status_int.len() == 1 {
        3
    } else {
        2 + status_int.len()
    };
    let pos = status_tlv_len;

    let timestamp_token = if pos < reply_inner.len() {
        // Check for [0] EXPLICIT (tag 0xA0)
        if reply_inner[pos] == 0xA0 {
            let (_, loff) = parse_der_length(&reply_inner[pos + 1..])
                .ok_or_else(|| {
                    Diagnostic::new(
                        "GP-S-020", Severity::Error,
                        "TimestampToken: invalid [0] length".to_string(),
                    )
                })?;
            let (clen, _) = parse_der_length(&reply_inner[pos + 1..])
                .ok_or_else(|| {
                    Diagnostic::new(
                        "GP-S-020", Severity::Error,
                        "TimestampToken: invalid [0] content length".to_string(),
                    )
                })?;
            let token_inner =
                &reply_inner[pos + 1 + loff..pos + 1 + loff + clen];
            Some(token_inner.to_vec())
        } else {
            None
        }
    } else {
        None
    };

    Ok(TimestampResponse {
        status,
        timestamp_token,
    })
}

// ---------------------------------------------------------------------------
// TSA client (HTTP transport)
// ---------------------------------------------------------------------------

/// Send a timestamp request to a TSA server and return the raw response.
///
/// Uses `reqwest` (rustls-tls) to POST the DER-encoded `TimestampRequest`
/// to the TSA endpoint. The TSA expects the body to be the raw DER bytes
/// with content type `application/timestamp-query`.
///
/// Returns `Err` if the TSA is unreachable, returns a non-200 status,
/// or the response cannot be parsed as DER.
#[cfg(feature = "pades-ltv")]
pub async fn fetch_timestamp(
    tsa_url: &str,
    request: &TimestampRequest,
    timeout_secs: u64,
) -> Result<Vec<u8>, Diagnostic> {
    use reqwest::Client;

    let request_der = request.to_der();

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-020",
                Severity::Warning,
                format!("Failed to build HTTP client for TSA: {}", e),
            )
        })?;

    let response = client
        .post(tsa_url)
        .header("Content-Type", "application/timestamp-query")
        .header("Accept", "application/timestamp-reply")
        .body(request_der)
        .send()
        .await
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-020",
                Severity::Warning,
                format!("TSA request failed: {}", e),
            )
        })?;

    if !response.status().is_success() {
        return Err(Diagnostic::new(
            "GP-S-020",
            Severity::Warning,
            format!(
                "TSA returned HTTP {}: {}",
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
                "GP-S-020",
                Severity::Warning,
                format!("Failed to read TSA response body: {}", e),
            )
        })?;

    if body.is_empty() {
        return Err(Diagnostic::new(
            "GP-S-020",
            Severity::Warning,
            "TSA returned empty response".to_string(),
        ));
    }

    Ok(body.to_vec())
}

/// Send a timestamp request to a TSA server with a simplified API.
///
/// Returns the parsed `TimestampResponse` on success.
#[cfg(feature = "pades-ltv")]
pub async fn fetch_timestamp_parsed(
    tsa_url: &str,
    request: &TimestampRequest,
    timeout_secs: u64,
) -> Result<TimestampResponse, Diagnostic> {
    let raw = fetch_timestamp(tsa_url, request, timeout_secs).await?;
    parse_timestamp_response(&raw)
}

/// TSA result used by the signature integration.
#[derive(Debug, Clone)]
pub struct TsaResult {
    /// Whether the timestamp was successfully obtained.
    pub success: bool,
    /// The CMS SignedData DER bytes (timestamp token) from the TSA.
    pub timestamp_token: Option<Vec<u8>>,
    /// Diagnostics produced during the timestamp exchange.
    pub diagnostics: Vec<Diagnostic>,
}

impl TsaResult {
    /// Create a success result with a timestamp token.
    pub fn granted(token: Vec<u8>) -> Self {
        Self {
            success: true,
            timestamp_token: Some(token),
            diagnostics: Vec::new(),
        }
    }

    /// Create a graceful-failure result (honest-absent).
    pub fn absent(diag: Diagnostic) -> Self {
        Self {
            success: false,
            timestamp_token: None,
            diagnostics: vec![diag],
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    /// Build a minimal DER-encoded TimestampResponse with status=Granted
    /// and a mock CMS SignedData token.
    fn make_mock_response(token: &[u8]) -> Vec<u8> {
        // PKIStatus INTEGER 0x00
        let status = integer(&[0x00]);
        // timeStToken [0] EXPLICIT wrapping the token
        let token_wrapped = ctx_explicit0(token);
        // TimestampReply ::= SEQUENCE { status, timeStToken }
        let reply_bytes = [&status[..], &token_wrapped[..]].concat();
        let reply = sequence(&reply_bytes);
        // ContentInfo ::= SEQUENCE { contentType OID, content [0] EXPLICIT }
        let ct_oid = oid_bytes(OID_CT_TIMESTAMP_REPLY);
        let reply_tagged = ctx_explicit0(&reply);
        // ContentInfo = SEQUENCE { OID, [0] EXPLICIT reply }
        let ci_bytes = [&ct_oid[..], &reply_tagged[..]].concat();
        sequence(&ci_bytes)
    }

    #[test]
    fn test_request_der_without_nonce() {
        let hash = sha2::Sha256::digest(b"test message");
        let request = TimestampRequest::new(hash.to_vec());
        let der = request.to_der();
        assert_eq!(der[0], 0x30);
        let ci_inner = parse_sequence(&der).expect("valid ContentInfo");
        assert_eq!(ci_inner[0], 0x06);
    }

    #[test]
    fn test_request_der_with_nonce() {
        let hash = sha2::Sha256::digest(b"test message");
        let request = TimestampRequest::new(hash.to_vec()).with_nonce(12345);
        let der = request.to_der();
        assert_eq!(der[0], 0x30);
        // The nonce [1] EXPLICIT should be in the request
        let has_nonce = der.windows(1).any(|w| !w.is_empty() && w[0] == 0xA1);
        assert!(has_nonce, "nonce should be present in request");
    }

    #[test]
    fn test_request_has_sha256_oid() {
        let hash = sha2::Sha256::digest(b"test");
        let request = TimestampRequest::new(hash.to_vec());
        let der = request.to_der();
        let sha256_oid_bytes = oid_bytes(OID_SHA256);
        assert!(der.windows(sha256_oid_bytes.len()).any(|w| w == sha256_oid_bytes));
    }

    #[test]
    fn test_parse_granted_response() {
        let mock_token = vec![0x30, 0x02, 0x01, 0x02];
        let response_der = make_mock_response(&mock_token);
        let response = parse_timestamp_response(&response_der).expect("parse should succeed");
        assert_eq!(response.status, PKIStatus::Granted);
        assert!(response.timestamp_token.is_some());
        assert_eq!(response.timestamp_token.as_deref(), Some(mock_token.as_slice()));
    }

    #[test]
    fn test_parse_failure_response() {
        let status = integer(&[0x03]);
        let reply = sequence(&status);
        let ct_oid = oid_bytes(OID_CT_TIMESTAMP_REPLY);
        let reply_tagged = ctx_explicit0(&reply);
        let ci_bytes = [&ct_oid[..], &reply_tagged[..]].concat();
        let content_info = sequence(&ci_bytes);
        let response = parse_timestamp_response(&content_info).expect("parse should succeed");
        assert_eq!(response.status, PKIStatus::Failure);
        assert!(response.timestamp_token.is_none());
    }

    #[test]
    fn test_parse_rejection_response() {
        let status = integer(&[0x02]);
        let reply = sequence(&status);
        let ct_oid = oid_bytes(OID_CT_TIMESTAMP_REPLY);
        let reply_tagged = ctx_explicit0(&reply);
        let ci_bytes = [&ct_oid[..], &reply_tagged[..]].concat();
        let content_info = sequence(&ci_bytes);
        let response = parse_timestamp_response(&content_info).expect("parse should succeed");
        assert_eq!(response.status, PKIStatus::Rejection);
    }

    #[test]
    fn test_parse_response_without_token() {
        let status = integer(&[0x00]);
        let reply = sequence(&status);
        let ct_oid = oid_bytes(OID_CT_TIMESTAMP_REPLY);
        let reply_tagged = ctx_explicit0(&reply);
        let ci_bytes = [&ct_oid[..], &reply_tagged[..]].concat();
        let content_info = sequence(&ci_bytes);
        let response = parse_timestamp_response(&content_info).expect("parse should succeed");
        assert_eq!(response.status, PKIStatus::Granted);
        assert!(response.timestamp_token.is_none());
    }

    #[test]
    fn test_parse_invalid_content_type() {
        let status = integer(&[0x00]);
        let reply = sequence(&status);
        let wrong_oid = oid_bytes("1.2.3.4.5");
        let reply_tagged = ctx_explicit0(&reply);
        let ci_bytes = [&wrong_oid[..], &reply_tagged[..]].concat();
        let content_info = sequence(&ci_bytes);
        let result = parse_timestamp_response(&content_info);
        assert!(result.is_err());
        if let Err(diag) = result {
            assert_eq!(diag.code, "GP-S-020");
        }
    }

    #[test]
    fn test_parse_invalid_der() {
        let invalid: Vec<u8> = vec![0xFF, 0xFF, 0xFF];
        let result = parse_timestamp_response(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_tsa_result_granted() {
        let result = TsaResult::granted(vec![0x30, 0x01, 0x02]);
        assert!(result.success);
        assert!(result.timestamp_token.is_some());
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_tsa_result_absent() {
        let diag = Diagnostic::new("GP-S-020", Severity::Warning, "TSA unreachable".to_string());
        let result = TsaResult::absent(diag.clone());
        assert!(!result.success);
        assert!(result.timestamp_token.is_none());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "GP-S-020");
    }

    #[test]
    fn test_pkistatus_display() {
        assert_eq!(format!("{}", PKIStatus::Granted), "Granted");
        assert_eq!(format!("{}", PKIStatus::GrantedWithMods), "GrantedWithMods");
        assert_eq!(format!("{}", PKIStatus::Rejection), "Rejection");
        assert_eq!(format!("{}", PKIStatus::Failure), "Failure");
    }

    #[test]
    fn test_request_nonce_encoding() {
        let nonce_bytes = u64_to_der_int(1);
        assert!(!nonce_bytes.is_empty());
        assert_eq!(nonce_bytes[0], 0x02);
        assert_eq!(u64_to_der_int(0), vec![0x02, 0x01, 0x00]);
        assert_eq!(u64_to_der_int(0xFF), vec![0x02, 0x02, 0x00, 0xFF]);
    }

    #[test]
    fn test_oid_bytes_sha256() {
        let bytes = oid_bytes(OID_SHA256);
        assert_eq!(bytes[0], 0x06);
        let expected: Vec<u8> = vec![0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
        assert_eq!(bytes, expected);
    }

    #[test]
    fn test_roundtrip_minimal_request() {
        let hash = [0u8; 32];
        let request = TimestampRequest::new(hash.to_vec());
        let der = request.to_der();
        assert_eq!(der[0], 0x30);
        let ci = parse_sequence(&der).expect("valid ContentInfo");
        assert!(ci.len() > 2);
    }

    #[test]
    fn test_parse_granted_with_token() {
        let token = vec![0x30, 0x04, 0x02, 0x01, 0x02, 0x01];
        let response_der = make_mock_response(&token);
        let response = parse_timestamp_response(&response_der).expect("parse should succeed");
        assert_eq!(response.status, PKIStatus::Granted);
        assert_eq!(response.timestamp_token.as_deref(), Some(token.as_slice()));
    }
}
