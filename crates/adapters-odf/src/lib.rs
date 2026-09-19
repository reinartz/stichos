// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-odf: ODF (.odt, .ods, .odp) readers for stichos.
//!
//! Provides:
//! - ODF package parsing via `parse_odf()` (ZIP archive with manifest)
//! - ODF manifest parsing via `parse_manifest()` (META-INF/manifest.xml)
//! - Encryption detection via `detect_encryption()` (flag only, no decryption)
//! - ODF census (feature counting) via `census` module
//! - RelaxNG validation gate via `rng_validate` module
//!
//! Diagnostic codes:
//! - ODF-E-001: ODF package error
//! - ODF-E-002: Manifest parse error
//! - ODF-E-003: Encryption detected

pub mod census;
pub mod comment_writer;
pub mod lossless_suite;
pub mod manifest;
pub mod odf;
pub mod odp;
pub mod ods;
pub mod odt;
pub mod patch;
pub mod patch_executor;
pub mod revision_executor;
pub mod revision_reader;
pub mod revision_writer;
pub mod rng_validate;
pub mod writer;
pub mod xml;
