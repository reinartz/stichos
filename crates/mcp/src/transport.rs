//! MCP transport initialization.
//!
//! Chooses the transport mode and starts the server.

use std::future::Future;
use std::pin::Pin;

use crate::dispatch::BoxedHandler;
use crate::server;

/// Initialize the MCP transport with the given mode and handler.
///
/// Returns a future that resolves when the server exits.
/// The future must be awaited by the caller (e.g. inside `#[tokio::main]`).
pub fn init(
    mode: TransportMode,
    handler: BoxedHandler,
) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
    server::start(mode, handler)
}

/// Transport mode.
#[derive(Debug, Clone)]
pub enum TransportMode {
    /// JSON-RPC over stdio.
    Stdio,
    /// HTTP with session management.
    Http { port: u16 },
}
