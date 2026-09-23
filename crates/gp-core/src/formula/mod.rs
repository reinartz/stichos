// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula evaluation: minimal formula subset for C15 parity.
//!
//! Supports arithmetic, comparison, logic, aggregate, and text functions from the
//! C15 v1.0.0 supported set. Hostile or unsupported formulas are refused with
//! GP-F0xx diagnostics, and cached values are surfaced instead of invented.
//!
//! This module is shared by both `adapters-ooxml` (xlsx) and `adapters-odf` (ods) codecs
//! to avoid circular dependencies in the crate DAG.
//!
//! See also: `docs/contracts/C15-formula-evaluation.md`

mod evaluator;
mod parity;
pub mod parser;
mod refuse;

pub use evaluator::*;
pub use parity::*;
pub use parser::FormulaNode;
pub use refuse::*;
