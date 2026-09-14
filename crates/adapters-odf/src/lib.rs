//! adapters-odf: ODF (.odt, .ods, .odp) readers for stichos.
//!
//! Provides:
//! - ODF package parsing via `parse_odf()` (ZIP archive with manifest)
//! - ODF manifest parsing via `parse_manifest()` (META-INF/manifest.xml)
//! - Encryption detection via `detect_encryption()` (flag only, no decryption)
//!
//! Diagnostic codes:
//! - ODF-E-001: ODF package error
//! - ODF-E-002: Manifest parse error
//! - ODF-E-003: Encryption detected

pub mod manifest;
pub mod odf;
pub mod odp;
pub mod ods;
pub mod odt;
pub mod xml;
