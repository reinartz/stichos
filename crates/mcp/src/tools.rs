// SPDX-License-Identifier: MIT OR Apache-2.0
//! mcp: MCP (Model Context Protocol) tool definitions.
//!
//! Re-exports the C7-compliant tool specification from `spec` module.

use crate::spec::ToolSpec;

/// The list of tools exposed by stichos via MCP.
/// Phase 1: read-only tools only (no apply).
///
/// This re-export uses the C7-compliant ToolSpec types from the `spec` module.
pub use crate::spec::TOOLS;

/// Check if a tool name is valid per C7.
pub fn is_valid_tool(name: &str) -> bool {
    crate::spec::is_valid_tool(name)
}

/// Get a tool spec by name.
pub fn get_tool(name: &str) -> Option<&'static ToolSpec> {
    crate::spec::get_tool(name)
}

/// Get all non-deprecated tool names.
pub fn active_tool_names() -> Vec<&'static str> {
    crate::spec::active_tool_names()
}

/// Verify C7 compliance: all tools have schema_version "1.0.0" and are idempotent.
pub fn verify_c7_compliance() -> Result<Vec<&'static str>, String> {
    crate::spec::verify_c7_compliance()
}
