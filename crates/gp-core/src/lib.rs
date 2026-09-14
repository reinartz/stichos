//! gp-core: core types, IR, diagnostics, traits, and content-addressed store for stichos.
//!
//! This crate defines the intermediate representation (IR) used by all stichos codecs,
//! the diagnostic model for structured error reporting, and the traits that adapters,
//! engines, and linters implement.

pub mod diagnostics;
pub mod fidelity;
pub mod ir;
pub mod macro_check;
pub mod store;
pub mod testkit;
pub mod traits;
