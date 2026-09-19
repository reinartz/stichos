// SPDX-License-Identifier: MIT OR Apache-2.0
//! MCP tool dispatch: handler trait + boxed type for the transport layer.
//!
//! The `cli` crate provides a concrete `ToolHandler` that routes calls to the
//! real tool implementations (`inspect`, `render`, `diff`, `doctor`, etc.).
//! The `mcp` server stores a `BoxedHandler` and invokes it from
//! `handle_tools_call`.

use serde_json::Value;

/// Async handler for a single tool invocation.
///
/// Implement this trait in the `cli` crate (or provide a closure) to wire up
/// real tool logic.  The `mcp` server stores an `Arc<dyn ToolHandler>` and
/// calls `dispatch` for every `tools/call` request.
///
/// # Returns
/// * `Ok(Value)` — the tool's structured result (becomes `structuredContent`)
/// * `Err(String)` — an error message (will be returned as text content with
///   `isError: true`)
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    /// Dispatch a tool call with the given name and arguments.
    async fn dispatch(&self, name: &str, args: &Value) -> Result<Value, String>;
}

/// A heap-allocated, dynamically dispatched handler.
pub type BoxedHandler = std::sync::Arc<dyn ToolHandler>;

/// A simple handler adapter so closures can be used as `BoxedHandler`.
///
/// Stores the closure and the future it produces.  The future is stored in a
/// `Box` so the `FnHandler` has a fixed size.
pub struct FnHandler<F, Fut>(F, std::marker::PhantomData<fn() -> Fut>);

impl<F, Fut> FnHandler<F, Fut> {
    pub fn new(f: F) -> Self {
        Self(f, std::marker::PhantomData)
    }
}

#[async_trait::async_trait]
impl<F, Fut> ToolHandler for FnHandler<F, Fut>
where
    F: Fn(&str, &Value) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<Value, String>> + Send + 'static,
{
    async fn dispatch(&self, name: &str, args: &Value) -> Result<Value, String> {
        (self.0)(name, args).await
    }
}

/// Helper to create a boxed handler from an async closure.
pub fn boxed_handler<F, Fut>(f: F) -> BoxedHandler
where
    F: Fn(&str, &Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Value, String>> + Send + 'static,
{
    std::sync::Arc::new(FnHandler::new(f))
}
