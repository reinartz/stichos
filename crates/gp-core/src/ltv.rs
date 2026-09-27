// SPDX-License-Identifier: MIT OR Apache-2.0
//! LTV (Long-Term Validation) entry builder and PDF embedding helpers.
//!
//! Builds PDF `/LTV` dictionary entries per ETSI EN 319 102-5 and provides
//! mutation builders for the incremental PDF writer.
//!
//! LTV data is stored in the signature dictionary as a `/LTV` array:
//! ```text
//! /LTV [
//!   << /Type /LTV /Subtype /PDFCA
//!      /V <DER-encoded OCSP response>
//!      /Cert [<signing cert DER>] >>
//!   << /Type /LTV /Subtype /ETSI.CAdES.detached
//!      /V <DER-encoded CRL>
//!      /Cert [<signing cert DER>, <issuer cert DER>] >>
//! ]
//! ```

use crate::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// LTV data source types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LtvDataSource {
    /// OCSP response.
    Ocsp,
    /// CRL.
    Crl,
}

impl std::fmt::Display for LtvDataSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LtvDataSource::Ocsp => write!(f, "OCSP"),
            LtvDataSource::Crl => write!(f, "CRL"),
        }
    }
}

/// An LTV entry to embed in the PDF signature dictionary.
#[derive(Debug, Clone)]
pub struct LtvEntry {
    /// The data source type.
    pub source: LtvDataSource,
    /// The DER-encoded validation data (OCSP response or CRL).
    pub validation_data: Vec<u8>,
    /// Signing certificate DER (for /Cert chain).
    pub signing_cert: Vec<u8>,
    /// Issuing certificate DER (optional, for chain).
    pub issuing_cert: Option<Vec<u8>>,
}

/// LTV result used by the signature integration.
#[derive(Debug, Clone)]
pub struct LtvResult {
    /// Whether LTV data was successfully collected.
    pub success: bool,
    /// The LTV entries to embed.
    pub entries: Vec<LtvEntry>,
    /// Diagnostics produced during LTV collection.
    pub diagnostics: Vec<Diagnostic>,
}

impl LtvResult {
    /// Create a success result with LTV entries.
    pub fn success(entries: Vec<LtvEntry>) -> Self {
        Self {
            success: true,
            entries,
            diagnostics: Vec::new(),
        }
    }

    /// Create a failure result (honest-absent).
    pub fn absent(diag: Diagnostic) -> Self {
        Self {
            success: false,
            entries: Vec::new(),
            diagnostics: vec![diag],
        }
    }
}

// ---------------------------------------------------------------------------
// Mutation builder for LTV embedding
// ---------------------------------------------------------------------------

/// Build the `/LTV` array object for embedding in a signature dictionary.
///
/// Returns a lopdf `Object::Array` containing the LTV entry dictionaries.
/// Callers can use this to construct a `PdfMutation::Replace` on the signature
/// dictionary via the incremental writer.
pub fn build_ltv_array(entries: &[LtvEntry]) -> lopdf::Object {
    use lopdf::Object;

    let ltv_items: Vec<Object> = entries.iter().map(build_ltv_dict).collect();

    Object::Array(ltv_items)
}

/// Build a single LTV dictionary for an entry.
///
/// Returns a `lopdf::Object::Dictionary` suitable for embedding.
pub fn build_ltv_dict(entry: &LtvEntry) -> lopdf::Object {
    use lopdf::Object;

    let mut dict = lopdf::Dictionary::new();

    dict.set(b"Type", Object::Name(b"LTV".to_vec()));

    // /Subtype
    let subtype: &'static [u8] = match entry.source {
        LtvDataSource::Ocsp => b"PDFCA",
        LtvDataSource::Crl => b"ETSI.CAdES.detached",
    };
    dict.set(b"Subtype", Object::Name(subtype.to_vec()));

    // /V — DER-encoded validation data as hex string
    dict.set(
        b"V",
        Object::String(entry.validation_data.clone(), lopdf::StringFormat::Hexadecimal),
    );

    // /Cert — array of certificate DER bytes (hex strings)
    let mut certs: Vec<Object> = vec![Object::String(
        entry.signing_cert.clone(),
        lopdf::StringFormat::Hexadecimal,
    )];
    if let Some(issuing) = &entry.issuing_cert {
        certs.push(Object::String(
            issuing.clone(),
            lopdf::StringFormat::Hexadecimal,
        ));
    }
    dict.set(b"Cert", Object::Array(certs));

    Object::Dictionary(dict)
}

/// Build a `/LTV` key-value pair for a signature dictionary.
///
/// Returns `Some((b"LTV".to_vec(), array_object))` if entries is non-empty,
/// `None` otherwise.
pub fn build_ltv_key_value(entries: &[LtvEntry]) -> Option<(Vec<u8>, lopdf::Object)> {
    if entries.is_empty() {
        return None;
    }
    Some((b"LTV".to_vec(), build_ltv_array(entries)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Severity;

    #[test]
    fn test_ltv_data_source_display() {
        assert_eq!(format!("{}", LtvDataSource::Ocsp), "OCSP");
        assert_eq!(format!("{}", LtvDataSource::Crl), "CRL");
    }

    #[test]
    fn test_ltv_result_success() {
        let entry = LtvEntry {
            source: LtvDataSource::Ocsp,
            validation_data: vec![0x30, 0x01, 0x02],
            signing_cert: vec![0x30, 0x02, 0x03],
            issuing_cert: Some(vec![0x30, 0x03, 0x04]),
        };
        let result = LtvResult::success(vec![entry]);
        assert!(result.success);
        assert_eq!(result.entries.len(), 1);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_ltv_result_absent() {
        let diag = Diagnostic::new("GP-S-021", Severity::Warning, "No LTV data".to_string());
        let result = LtvResult::absent(diag);
        assert!(!result.success);
        assert!(result.entries.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
    }

    #[test]
    fn test_ltv_entry_ocsp() {
        let entry = LtvEntry {
            source: LtvDataSource::Ocsp,
            validation_data: vec![1, 2, 3],
            signing_cert: vec![4, 5, 6],
            issuing_cert: None,
        };
        assert_eq!(entry.source, LtvDataSource::Ocsp);
    }

    #[test]
    fn test_ltv_entry_crl() {
        let entry = LtvEntry {
            source: LtvDataSource::Crl,
            validation_data: vec![7, 8, 9],
            signing_cert: vec![10, 11, 12],
            issuing_cert: Some(vec![13, 14, 15]),
        };
        assert_eq!(entry.source, LtvDataSource::Crl);
        assert!(entry.issuing_cert.is_some());
    }

    #[test]
    fn test_build_ltv_dict_ocsp() {
        let entry = LtvEntry {
            source: LtvDataSource::Ocsp,
            validation_data: vec![0x30, 0x01, 0x02],
            signing_cert: vec![0x30, 0x02],
            issuing_cert: None,
        };
        let obj = build_ltv_dict(&entry);
        let dict = obj.as_dict().unwrap();
        assert_eq!(dict.get(b"Type").unwrap(), &lopdf::Object::Name(b"LTV".to_vec()));
        assert_eq!(dict.get(b"Subtype").unwrap(), &lopdf::Object::Name(b"PDFCA".to_vec()));
        assert!(dict.has(b"V"));
        assert!(dict.has(b"Cert"));
    }

    #[test]
    fn test_build_ltv_dict_crl() {
        let entry = LtvEntry {
            source: LtvDataSource::Crl,
            validation_data: vec![0x30, 0x01],
            signing_cert: vec![0x30, 0x02],
            issuing_cert: Some(vec![0x30, 0x03]),
        };
        let obj = build_ltv_dict(&entry);
        let dict = obj.as_dict().unwrap();
        assert_eq!(dict.get(b"Subtype").unwrap(), &lopdf::Object::Name(b"ETSI.CAdES.detached".to_vec()));
    }

    #[test]
    fn test_build_ltv_key_value_empty() {
        let entries: Vec<LtvEntry> = vec![];
        assert!(build_ltv_key_value(&entries).is_none());
    }

    #[test]
    fn test_build_ltv_key_value_nonempty() {
        let entry = LtvEntry {
            source: LtvDataSource::Ocsp,
            validation_data: vec![1],
            signing_cert: vec![2],
            issuing_cert: None,
        };
        let entries = vec![entry];
        let result = build_ltv_key_value(&entries);
        assert!(result.is_some());
        let (key, _value) = result.unwrap();
        assert_eq!(key, b"LTV".to_vec());
    }

    #[test]
    fn test_ltv_array_builds_object_array() {
        let entry = LtvEntry {
            source: LtvDataSource::Ocsp,
            validation_data: vec![1, 2],
            signing_cert: vec![3, 4],
            issuing_cert: None,
        };
        let entries = vec![entry];
        let array = build_ltv_array(&entries);
        assert!(matches!(array, lopdf::Object::Array(_)));
    }
}
