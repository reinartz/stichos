// SPDX-License-Identifier: MIT OR Apache-2.0
//! gp-core: core types, IR, diagnostics, traits, and content-addressed store for stichos.
//!
//! This crate defines the intermediate representation (IR) used by all stichos codecs,
//! the diagnostic model for structured error reporting, and the traits that adapters,
//! engines, and linters implement.

#[cfg(test)]
mod c10_contract_tests;
pub mod diagnostics;
pub mod diff;
pub mod edit_stability;
pub mod envelope;
pub mod executor;
pub mod fidelity;
pub mod file_access;
pub mod formula;
pub mod ir;
pub mod ir_convert;
pub mod journal;
pub mod macro_check;
pub mod patch;
pub mod policy;
pub mod script;
pub mod selfupdate;
pub mod signature;
pub mod store;
pub mod tempfiles;
pub mod testkit;
pub mod timestamp_client;
pub mod traits;

#[cfg(feature = "pades-ltv")]
#[allow(dead_code)]
pub mod cert_chain;
#[cfg(feature = "pades-ltv")]
#[allow(dead_code)]
pub mod crl_client;
#[cfg(feature = "pades-ltv")]
#[allow(dead_code)]
pub mod ltv;
#[cfg(feature = "pades-ltv")]
#[allow(dead_code)]
pub mod ocsp_client;
#[cfg(feature = "pades-ltv")]
#[allow(dead_code)]
pub mod pkcs12;
