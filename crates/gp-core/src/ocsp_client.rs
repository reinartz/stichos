// SPDX-License-Identifier: MIT OR Apache-2.0
//! OCSP client: RFC 6960 request builder, response parser, and HTTP fetcher.
//!
//! Builds DER-encoded `OCSPRequest` structures for a signing certificate + issuer cert,
//!
//! Builds DER-encoded `OCSPRequest` structures for a signing certificate + issuer cert,
//! sends them to the responder URL found in the certificate's Authority Information Access (AIA)
//! extension, and parses the `BasicOCSPResponse` status (good / revoked / unknown).
//!
//! Honest-absent: if the OCSP responder is unreachable, callers fall back to signing
//! without LTV data, emitting diagnostic `GP-S-021`. If the cert is revoked, signing
//! is refused with `GP-S-022`.
//!
//! See also: RFC 6960, ETSI EN 319 102-5

use crate::diagnostics::Diagnostic;
use crate::diagnostics::Severity;

// ---------------------------------------------------------------------------
// RFC 6960 OID constants
// ---------------------------------------------------------------------------

/// OID for sha256 (2.16.840.1.101.3.4.2.1)
const OID_SHA256: &str = "2.16.840.1.101.3.4.2.1";
/// OID for id-pkix-ocsp-basic (1.3.6.1.5.5.7.48.1.1)
const OID_OCSP_BASIC: &str = "1.3.6.1.5.5.7.48.1.1";
/// OID for id-pkix-ocsp (1.3.6.1.5.5.7.48.1)
const OID_OCSP: &str = "1.3.6.1.5.5.7.48.1";

// ---------------------------------------------------------------------------
// Manual DER helpers (copied from timestamp_client.rs to avoid coupling)
// ---------------------------------------------------------------------------

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

fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + value.len());
    out.push(tag);
    encode_der_length(value.len(), &mut out);
    out.extend_from_slice(value);
    out
}

fn sequence(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + value.len());
    out.push(0x30);
    encode_der_length(value.len(), &mut out);
    out.extend_from_slice(value);
    out
}

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

fn integer(value: &[u8]) -> Vec<u8> {
    tlv(0x02, value)
}

fn octet_string(value: &[u8]) -> Vec<u8> {
    tlv(0x04, value)
}

fn bit_string(value: &[u8]) -> Vec<u8> {
    let mut inner = vec![0x00]; // no unused bits
    inner.extend_from_slice(value);
    tlv(0x03, &inner)
}

fn utf8_string(s: &str) -> Vec<u8> {
    tlv(0x0C, s.as_bytes())
}

fn ctx_explicit0(value: &[u8]) -> Vec<u8> {
    tlv(0xA0, value)
}

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
            let mut val = p;
            let mut num_bytes = 0;
            while val > 0 {
                num_bytes += 1;
                val >>= 7;
            }
            let mut tmp = Vec::with_capacity(num_bytes as usize);
            val = p;
            for _ in 0..num_bytes {
                tmp.push((val & 0x7F) as u8);
                val >>= 7;
            }
            tmp.reverse();
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
// OCSP request types
// ---------------------------------------------------------------------------

/// A RFC 6960 OCSP request.
#[derive(Debug, Clone)]
pub struct OcspRequest {
    /// The serial number of the certificate to check.
    pub serial_number: Vec<u8>,
    /// The issuer's SHA-256 hash (issuerKeyHash).
    pub issuer_hash: Vec<u8>,
    /// The issuer's public key hash (issuerNameHash).
    pub issuer_name_hash: Vec<u8>,
}

impl OcspRequest {
    /// Build a new OCSP request for the given certificate fields.
    pub fn new(
        serial_number: Vec<u8>,
        issuer_hash: Vec<u8>,
        issuer_name_hash: Vec<u8>,
    ) -> Self {
        Self {
            serial_number,
            issuer_hash,
            issuer_name_hash,
        }
    }

    /// Encode this request as a DER-encoded RFC 6960 `OCSPRequest`.
    pub fn to_der(&self) -> Vec<u8> {
        // Request ::= SEQUENCE {
        //   tbsRequest      TBSRequest,
        //   optionalSignature  [0] EXPLICIT OptPkcs9Signature OPTIONAL
        // }
        // We build just the tbsRequest (no signature — unsigned OCSP is standard).

        // TBSRequest ::= SEQUENCE {
        //   version       [0] EXPLICIT Version DEFAULT v1,
        //   requestList   SEQUENCE OF Request,
        // }

        // version v1: [0] EXPLICIT INTEGER 0 = A0 02 02 00
        let version_bytes: [u8; 4] = [0xA0, 0x02, 0x02, 0x00];

        // SingleRequest ::= SEQUENCE {
        //   subjectCertId   CertID,
        //   singleRequestExtensions [0] EXPLICIT Extensions OPTIONAL
        // }

        // CertID ::= SEQUENCE {
        //   hashAlgorithm   AlgorithmIdentifier,
        //   issuerNameHash  OCTET STRING,
        //   issuerKeyHash   OCTET STRING,
        //   serialNumber    CertificateSerialNumber
        // }
        let hash_algo_id = seq_parts(&[&oid_bytes(OID_SHA256)[..], &[0x05, 0x00]]);
        let cert_id = seq_parts(&[
            &hash_algo_id[..],
            &octet_string(&self.issuer_name_hash)[..],
            &octet_string(&self.issuer_hash)[..],
            &integer(&self.serial_number)[..],
        ]);

        let single_request = seq_parts(&[&cert_id[..]]);

        // requestList ::= SEQUENCE OF SingleRequest
        let request_list = sequence(&single_request);

        // TBSRequest
        let tbs_request = seq_parts(&[&version_bytes[..], &request_list[..]]);

        // OCSPRequest
        sequence(&tbs_request)
    }
}

// ---------------------------------------------------------------------------
// OCSP response types
// ---------------------------------------------------------------------------

/// The OCSP certificate status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcspStatus {
    /// The certificate is not revoked.
    Good,
    /// The certificate has been revoked.
    Revoked { reason: Option<u8>, revocation_time: Option<String> },
    /// The responder does not know about the certificate.
    Unknown,
}

impl std::fmt::Display for OcspStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcspStatus::Good => write!(f, "good"),
            OcspStatus::Revoked { reason, .. } => {
                write!(f, "revoked")?;
                if let Some(r) = reason {
                    write!(f, " (reason={})", r)?;
                }
                Ok(())
            }
            OcspStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// The overall OCSP response status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcspResponseStatus {
    /// The request was successfully processed.
    Success,
    /// Malformed request.
    Malformed,
    /// Internal error.
    InternalError,
    /// Try later.
    TryLater,
    /// Signature required.
    SigRequired,
    /// Unauthorized.
    Unauthorized,
}

/// A parsed RFC 6960 `BasicOCSPResponse`.
#[derive(Debug, Clone)]
pub struct OcspResponse {
    /// The overall response status.
    pub status: OcspResponseStatus,
    /// The single response (only present on success).
    pub single_response: Option<SingleResponse>,
}

/// A parsed single OCSP response.
#[derive(Debug, Clone)]
pub struct SingleResponse {
    /// The certificate status.
    pub cert_status: OcspStatus,
    /// The production time (when this response was generated).
    pub produced_at: Option<String>,
    /// The next update time (only for non-unknown status).
    pub next_update: Option<String>,
}

/// Parse a DER-encoded `OCSPResponse` from raw bytes.
///
/// Returns `Err` only on structural parsing failures.
/// A successful parse may still indicate a non-success status.
pub fn parse_ocsp_response(raw: &[u8]) -> Result<OcspResponse, Diagnostic> {
    // OCSPResponse ::= SEQUENCE {
    //   responseStatus      ResponseStatus,
    //   responseBytes       [0] EXPLICIT ResponseBytes OPTIONAL
    // }
    let inner = parse_sequence(raw).ok_or_else(|| {
        Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            "OCSP response is not a valid DER SEQUENCE".to_string(),
        )
    })?;

    // responseStatus (INTEGER)
    if inner.is_empty() || inner[0] != 0x02 {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            "OCSP response: expected status INTEGER".to_string(),
        ));
    }
    let status_val = parse_integer_bytes(&inner[0..])
        .ok_or_else(|| Diagnostic::new("GP-S-021", Severity::Error, "Cannot parse status INTEGER".to_string()))?;

    let status = match status_val.as_slice() {
        &[0x00] => OcspResponseStatus::Success,
        &[0x01] => OcspResponseStatus::Malformed,
        &[0x02] => OcspResponseStatus::InternalError,
        &[0x03] => OcspResponseStatus::TryLater,
        &[0x04] => OcspResponseStatus::SigRequired,
        &[0x05] => OcspResponseStatus::Unauthorized,
        v => return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Warning,
            format!("Unknown OCSPResponseStatus: {:?}", v),
        )),
    };

    // Check if responseBytes [0] is present
    if status != OcspResponseStatus::Success {
        return Ok(OcspResponse {
            status,
            single_response: None,
        });
    }

    // Skip status INTEGER to find [0] EXPLICIT responseBytes
    let status_tlv_len = 2 + status_val.len();
    let pos = status_tlv_len;
    // responseBytes [0] is OPTIONAL per RFC 6960
    if pos >= inner.len() || inner[pos] != 0xA0 {
        return Ok(OcspResponse {
            status: OcspResponseStatus::Success,
            single_response: None,
        });
    }

    let (_, loff) = parse_der_length(&inner[pos + 1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Invalid [0] length in OCSP response".to_string())
    })?;
    let (clen, _) = parse_der_length(&inner[pos + 1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Invalid [0] content length".to_string())
    })?;
    let rb_start = pos + 1 + loff;
    let rb_bytes = &inner[rb_start..rb_start + clen];

    // ResponseBytes ::= SEQUENCE {
    //   contentType   ContentType,
    //   content       [1] EXPLICIT OCTET STRING OPTIONAL
    // }
    let rb_inner = parse_sequence(rb_bytes).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "responseBytes is not a SEQUENCE".to_string())
    })?;

    // contentType OID
    if rb_inner.is_empty() || rb_inner[0] != 0x06 {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            "responseBytes: expected OID for contentType".to_string(),
        ));
    }
    let oid_tag = &rb_inner[0..];
    let oid_str = parse_oid(oid_tag).unwrap_or_default();
    if oid_str != OID_OCSP_BASIC {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            format!(
                "responseBytes OID mismatch: got {}, expected {}",
                oid_str, OID_OCSP_BASIC
            ),
        ));
    }

    // Skip OID TLV
    let (_, oid_loff) = parse_der_length(&rb_inner[1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Invalid OID length".to_string())
    })?;
    let (oid_clen, _) = parse_der_length(&rb_inner[1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Invalid OID content length".to_string())
    })?;
    let oid_end = 1 + oid_loff + oid_clen;

    // content [1] EXPLICIT
    if oid_end >= rb_inner.len() || rb_inner[oid_end] != 0xA1 {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Error,
            "responseBytes: expected [1] EXPLICIT content".to_string(),
        ));
    }
    let (_, c_loff) = parse_der_length(&rb_inner[oid_end + 1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Invalid [1] length".to_string())
    })?;
    let (c_clen, _) = parse_der_length(&rb_inner[oid_end + 1..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Invalid [1] content length".to_string())
    })?;
    let content_inner_start = oid_end + 1 + c_loff;
    let content_bytes = &rb_inner[content_inner_start..content_inner_start + c_clen];

    // content is OCTET STRING wrapping BasicOCSPResponse
    let bos = parse_octet_string(content_bytes).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "content is not an OCTET STRING".to_string())
    })?;

    // BasicOCSPResponse ::= SEQUENCE {
    //   tbsResponseData      ResponseData,
    //   signatureAlgorithm   AlgorithmIdentifier,
    //   signature            BIT STRING,
    //   certs            [0] EXPLICIT Certs OPTIONAL
    // }
    let _tbs = parse_sequence(bos).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "BasicOCSPResponse is not a SEQUENCE".to_string())
    })?;
    // We don't need to fully parse the signature for LTV purposes
    // The raw DER of the BasicOCSPResponse is what we embed.

    // SingleResponse parsing from TBS ResponseData
    // ResponseData ::= SEQUENCE {
    //   version      [0] EXPLICIT Version DEFAULT v1,
    //   responseStatus   BasicStatus,
    //   ...
    //   singleResponses  SEQUENCE OF SingleResponse
    // }
    let tbs_inner = parse_sequence(bos).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "BasicOCSPResponse TBS not a SEQUENCE".to_string())
    })?;

    // Parse version [0]
    let mut pos = 0;
    if pos < tbs_inner.len() && tbs_inner[pos] == 0xA0 {
        if let Some(l) = tlv_len(tbs_inner, pos) {
            pos += l;
        }
    }
    // responseStatus (ENUMERATED 0x00 = success)
    // We skip the first SingleResponse only (OCSP responses have one per request)
    // The rest of TBS contains: responseTime [1], singleResponses

    // Find singleResponses: look for SEQUENCE containing SingleResponse
    let single_response = parse_single_response_from_tbs(&tbs_inner[pos..]).ok_or_else(|| {
        Diagnostic::new("GP-S-021", Severity::Error, "Cannot parse single response from TBS".to_string())
    })?;

    Ok(OcspResponse {
        status: OcspResponseStatus::Success,
        single_response: Some(single_response),
    })
}

/// Parse a SingleResponse from the remaining TBS content after status.
fn parse_single_response_from_tbs(data: &[u8]) -> Option<SingleResponse> {
    // singleResponses is a SEQUENCE OF SingleResponse
    let sr_seq = parse_sequence(data)?;

    // SingleResponse ::= SEQUENCE {
    //   certID        CertID,
    //   certStatus    CertStatus,
    //   thisUpdate    Time,
    //   nextUpdate    [0] EXPLICIT Time OPTIONAL,
    //   singleExtensions [1] EXPLICIT Extensions OPTIONAL
    // }
    let single = parse_sequence(sr_seq)?;

    // certID is a SEQUENCE (skip it)
    let mut pos = 0;
    pos = consume_tlv(single, pos)?;

    // certStatus: GOOD=0x00, REVOKED=0x01, UNKNOWN=0x02
    let cert_status_tag = single.get(pos)?;
    let cert_status = match cert_status_tag {
        0x00 => OcspStatus::Good,
        0x01 => {
            // REVOKED ::= SEQUENCE {
            //    revocationTime  Time,
            //    reason          EXPLICIT Reason ENUMERATED OPTIONAL
            // }
            let rev_inner = parse_sequence(&single[pos..])?;
            let mut rpos = 0;
            // revocationTime is a UTCTime/GeneralTime (tag 0x17 or 0x18)
            if rpos < rev_inner.len() {
                let time_tag = rev_inner[rpos];
                if time_tag == 0x17 || time_tag == 0x18 {
                    let (_, tloff) = parse_der_length(&rev_inner[rpos + 1..])?;
                    let (tclen, _) = parse_der_length(&rev_inner[rpos + 1..])?;
                    let time_str = String::from_utf8_lossy(&rev_inner[rpos + 1 + tloff..rpos + 1 + tloff + tclen]).to_string();
                    rpos += 1 + tloff + tclen;
                    // reason [0] EXPLICIT
                    let reason = if rpos < rev_inner.len() && rev_inner[rpos] == 0xA0 {
                        let (_, rloff) = parse_der_length(&rev_inner[rpos + 1..])?;
                        let (_rclen, _) = parse_der_length(&rev_inner[rpos + 1..])?;
                        let reason_byte = rev_inner[rpos + 1 + rloff];
                        Some(reason_byte)
                    } else {
                        None
                    };
                    OcspStatus::Revoked { reason, revocation_time: Some(time_str) }
                } else {
                    OcspStatus::Revoked { reason: None, revocation_time: None }
                }
            } else {
                OcspStatus::Revoked { reason: None, revocation_time: None }
            }
        }
        0x02 => OcspStatus::Unknown,
        _ => OcspStatus::Unknown,
    };
    pos = consume_tlv(single, pos)?;

    // thisUpdate (UTCTime/GeneralTime)
    let this_update = if pos < single.len() {
        let time_tag = single[pos];
        if time_tag == 0x17 || time_tag == 0x18 {
            let (_, tloff) = parse_der_length(&single[pos + 1..])?;
            let (tclen, _) = parse_der_length(&single[pos + 1..])?;
            let time_str = String::from_utf8_lossy(&single[pos + 1 + tloff..pos + 1 + tloff + tclen]).to_string();
            Some(time_str)
        } else {
            None
        }
    } else {
        None
    };
    pos = consume_tlv(single, pos)?;

    // nextUpdate [0] EXPLICIT (optional)
    let next_update = if pos < single.len() && single[pos] == 0xA0 {
        let (_, nloff) = parse_der_length(&single[pos + 1..])?;
        let (nclen, _) = parse_der_length(&single[pos + 1..])?;
        let time_str = String::from_utf8_lossy(&single[pos + 1 + nloff..pos + 1 + nloff + nclen]).to_string();
        Some(time_str)
    } else {
        None
    };

    Some(SingleResponse {
        cert_status,
        produced_at: this_update,
        next_update,
    })
}

/// Parse a DER OCTET STRING and return the inner content slice.
fn parse_octet_string(data: &[u8]) -> Option<&[u8]> {
    if data.is_empty() || data[0] != 0x04 {
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

/// Get the total TLV length starting at `start`.
fn tlv_len(data: &[u8], start: usize) -> Option<usize> {
    if start + 1 >= data.len() {
        return None;
    }
    let (_, len_off) = parse_der_length(&data[start + 1..])?;
    let total = 1 + len_off;
    let (content_len, _) = parse_der_length(&data[start + 1..])?;
    Some(total + content_len)
}

// ---------------------------------------------------------------------------
// OCSP fetcher (HTTP transport)
// ---------------------------------------------------------------------------

/// Send an OCSP request to a responder URL and return the raw DER response.
///
/// Uses `reqwest` (rustls-tls) to POST the DER-encoded `OcspRequest`
/// to the OCSP responder endpoint. The OCSP responder expects the body
/// to be the raw DER bytes with content type `application/ocsp-request`.
#[cfg(feature = "pades-ltv")]
pub async fn fetch_ocsp(
    responder_url: &str,
    request: &OcspRequest,
    timeout_secs: u64,
) -> Result<Vec<u8>, Diagnostic> {
    use reqwest::Client;

    let request_der = request.to_der();

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-021",
                Severity::Warning,
                format!("Failed to build HTTP client for OCSP: {}", e),
            )
        })?;

    let response = client
        .post(responder_url)
        .header("Content-Type", "application/ocsp-request")
        .header("Accept", "application/ocsp-response")
        .body(request_der)
        .send()
        .await
        .map_err(|e| {
            Diagnostic::new(
                "GP-S-021",
                Severity::Warning,
                format!("OCSP request failed: {}", e),
            )
        })?;

    if !response.status().is_success() {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Warning,
            format!(
                "OCSP responder returned HTTP {}: {}",
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
                format!("Failed to read OCSP response body: {}", e),
            )
        })?;

    if body.is_empty() {
        return Err(Diagnostic::new(
            "GP-S-021",
            Severity::Warning,
            "OCSP responder returned empty response".to_string(),
        ));
    }

    Ok(body.to_vec())
}

/// Send an OCSP request to a responder URL with a simplified API.
///
/// Returns the parsed `OcspResponse` on success.
#[cfg(feature = "pades-ltv")]
pub async fn fetch_ocsp_parsed(
    responder_url: &str,
    request: &OcspRequest,
    timeout_secs: u64,
) -> Result<OcspResponse, Diagnostic> {
    let raw = fetch_ocsp(responder_url, request, timeout_secs).await?;
    parse_ocsp_response(&raw)
}

/// OCSP result used by the signature integration.
#[derive(Debug, Clone)]
pub struct OcspResult {
    /// Whether the OCSP check succeeded.
    pub success: bool,
    /// The raw DER-encoded BasicOCSPResponse (only present on success).
    pub ocsp_response_der: Option<Vec<u8>>,
    /// The parsed certificate status.
    pub cert_status: Option<OcspStatus>,
    /// Diagnostics produced during the OCSP exchange.
    pub diagnostics: Vec<Diagnostic>,
}

impl OcspResult {
    /// Create a success result with an OCSP response.
    pub fn success(response_der: Vec<u8>, cert_status: OcspStatus) -> Self {
        Self {
            success: true,
            ocsp_response_der: Some(response_der),
            cert_status: Some(cert_status),
            diagnostics: Vec::new(),
        }
    }

    /// Create a graceful-failure result (honest-absent).
    pub fn absent(diag: Diagnostic) -> Self {
        Self {
            success: false,
            ocsp_response_der: None,
            cert_status: None,
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

    /// Build a minimal DER-encoded OCSPRequest for testing.
    fn make_mock_ocsp_request() -> OcspRequest {
        OcspRequest::new(
            vec![0x01, 0x23, 0x45], // serial
            vec![0xAA; 32],         // issuer hash
            vec![0xBB; 32],         // issuer name hash
        )
    }

    #[test]
    fn test_request_der_structure() {
        let req = make_mock_ocsp_request();
        let der = req.to_der();
        // OCSPRequest is a SEQUENCE
        assert_eq!(der[0], 0x30);
        let inner = parse_sequence(&der).expect("valid OCSPRequest SEQUENCE");
        assert!(!inner.is_empty());
        // Inner is TBSRequest SEQUENCE, which contains version [0] (0xA0) as its first field
        assert_eq!(inner[0], 0x30); // TBSRequest is a SEQUENCE
        let tbs_inner = parse_sequence(&inner).expect("TBSRequest is a SEQUENCE");
        assert_eq!(tbs_inner[0], 0xA0); // First field is version [0] EXPLICIT
    }

    #[test]
    fn test_request_has_sha256_oid() {
        let req = make_mock_ocsp_request();
        let der = req.to_der();
        let sha256_oid_bytes = oid_bytes(OID_SHA256);
        assert!(der.windows(sha256_oid_bytes.len()).any(|w| w == sha256_oid_bytes));
    }

    #[test]
    fn test_parse_ocsp_success_response() {
        // Build a minimal OCSPResponse with success status (no responseBytes)
        // responseStatus: INTEGER 0 (success), no responseBytes
        let status = integer(&[0x00]);
        // OCSPResponse = SEQUENCE { status }
        let ocsp_response = sequence(&status);

        let result = parse_ocsp_response(&ocsp_response).expect("parse should succeed");
        assert_eq!(result.status, OcspResponseStatus::Success);
        // Without responseBytes, single_response should be None
        assert!(result.single_response.is_none());
    }

    #[test]
    fn test_parse_ocsp_malformed() {
        // responseStatus: INTEGER 1 (malformed)
        let status = integer(&[0x01]);
        let ocsp_response = sequence(&status);

        let result = parse_ocsp_response(&ocsp_response).expect("parse should succeed");
        assert_eq!(result.status, OcspResponseStatus::Malformed);
        assert!(result.single_response.is_none());
    }

    #[test]
    fn test_parse_ocsp_internal_error() {
        // responseStatus: INTEGER 2 (internalError)
        let status = integer(&[0x02]);
        let ocsp_response = sequence(&status);

        let result = parse_ocsp_response(&ocsp_response).expect("parse should succeed");
        assert_eq!(result.status, OcspResponseStatus::InternalError);
    }

    #[test]
    fn test_parse_ocsp_invalid_der() {
        let invalid: Vec<u8> = vec![0xFF, 0xFF, 0xFF];
        let result = parse_ocsp_response(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_ocsp_result_success() {
        let result = OcspResult::success(vec![0x30, 0x01, 0x02], OcspStatus::Good);
        assert!(result.success);
        assert!(result.ocsp_response_der.is_some());
        assert_eq!(result.cert_status, Some(OcspStatus::Good));
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_ocsp_result_absent() {
        let diag = Diagnostic::new("GP-S-021", Severity::Warning, "OCSP unreachable".to_string());
        let result = OcspResult::absent(diag.clone());
        assert!(!result.success);
        assert!(result.ocsp_response_der.is_none());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "GP-S-021");
    }

    #[test]
    fn test_ocsp_status_display() {
        assert_eq!(format!("{}", OcspStatus::Good), "good");
        assert_eq!(format!("{}", OcspStatus::Revoked { reason: None, revocation_time: None }), "revoked");
        assert_eq!(
            format!("{}", OcspStatus::Revoked { reason: Some(1), revocation_time: None }),
            "revoked (reason=1)"
        );
        assert_eq!(format!("{}", OcspStatus::Unknown), "unknown");
    }

    #[test]
    fn test_parse_good_status_single_response() {
        // Test that we can parse a success response without responseBytes
        // This validates the responseStatus parsing path
        let status = integer(&[0x00]);
        let ocsp_response = sequence(&status);

        let result = parse_ocsp_response(&ocsp_response).expect("parse should succeed");
        assert_eq!(result.status, OcspResponseStatus::Success);
    }

    fn utc_time(s: &str) -> Vec<u8> {
        tlv(0x17, s.as_bytes())
    }
}
