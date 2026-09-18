//! MCP (Model Context Protocol) transport server for stichos.
//!
//! Implements stdio JSON-RPC transport. Exposes the stichos tool
//! surface to LLM agents.
//!
//! # Architecture
//!
//! ```text
//! LLM Agent ──stdin/stdout──> [JSON-RPC 2.0] ──> [Tool Router]
//!                              <──stdout─────────  [Response Writer]
//! ```

pub mod dispatch;
pub mod http;
pub mod jsonrpc;
pub mod server;
pub mod session;
pub mod spec;
pub mod tools;
pub mod transport;
