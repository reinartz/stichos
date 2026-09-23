// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula evaluation: minimal xlsx formula subset.
//!
//! Supports arithmetic, comparison, logic, aggregate, and text functions from the
//! C15 v1.0.0 supported set. Hostile or unsupported formulas are refused with
//! GP-F0xx diagnostics, and cached values are surfaced instead of invented.
//!
//! This module re-exports the shared formula evaluation engine from `gp-core`
//! and includes xlsx-specific test fixtures.
//!
//! See also: `docs/contracts/C15-formula-evaluation.md`

mod fixtures;

// Re-export the shared formula engine from gp-core
pub use gp_core::formula::*;

// Re-export fixtures for xlsx-specific tests
pub use fixtures::*;
