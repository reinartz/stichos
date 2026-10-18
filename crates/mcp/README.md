# stichos-mcp

MCP (Model Context Protocol) transport, JSON-RPC dispatch, and review sessions for the stichos
toolchain.

[![crates.io](https://img.shields.io/crates/v/stichos-mcp.svg)](https://crates.io/crates/stichos-mcp) [![docs.rs](https://img.shields.io/docsrs/stichos-mcp.svg)](https://docs.rs/stichos-mcp)

Part of the [stichos](https://github.com/reinartz/stichos) document-fidelity toolchain. See the
[workspace README](https://github.com/reinartz/stichos#readme) for the product overview.

## What it does

- Serves the Model Context Protocol over stdio or HTTP (`transport::{init, TransportMode}` →
  `server::start`) with a minimal JSON-RPC 2.0 layer
  (`jsonrpc::{parse_request, JsonRpcResponse, McpSession}`).
- Declares the 24 stichos tool specs (`spec::{TOOLS, get_tool}`, e.g. `inspect_document`,
  `render_document`, `apply_patch`) — descriptors only, no implementations.
- Routes `tools/call` to a caller-supplied handler
  (`dispatch::{ToolHandler, BoxedHandler, boxed_handler, FnHandler}`).
- Provides token-authenticated review sessions (`session::{new_review_session_store,
  create_review_session}`) and HTTP review routes, including static viewer assets
  (`http::{route_request, route_review_request}`).
- Keeps protocol, dispatch, transport, and session state independent of any document logic.

## Install

```toml
[dependencies]
stichos-mcp = "0.2"
```

No feature flags. Requires a tokio runtime. It depends on `stichos-core`.

## Usage

Starting a stdio MCP server with your own tool handler:

```rust
use serde_json::{json, Value};
use stichos_mcp::dispatch::boxed_handler;
use stichos_mcp::transport::{init, TransportMode};

#[tokio::main]
async fn main() -> Result<(), String> {
    let handler = boxed_handler(|name: &str, args: &Value| {
        let name = name.to_string();
        let args = args.clone();
        async move {
            match name.as_str() {
                "inspect_document" => Ok(json!({ "path": args["path"] })),
                other => Err(format!("unsupported tool: {other}")),
            }
        }
    });

    init(TransportMode::Stdio, handler).await
}
```

Inspecting the declared tool surface:

```rust
use stichos_mcp::spec::{get_tool, TOOLS};

fn describe() {
    println!("{} tools advertised", TOOLS.len());
    if let Some(spec) = get_tool("apply_patch") {
        println!("apply_patch: {}", spec.description);
    }
}
```

## When to use

- **Standalone:** as a generic MCP transport/server for any tool surface — provide your own
  `ToolHandler` and it will speak MCP over stdio or HTTP.
- Use it when embedding stichos tools into an agent host and you want the protocol, JSON-RPC, and
  review-session plumbing handled for you.
- Pair it with the rest of the workspace when you need the stichos tool set; the CLI is the reference
  implementation of the real handlers.

## Limitations

- It advertises 24 stichos tool specs but ships **no** handlers: `tools/call` fails unless you supply
  a `ToolHandler` that implements them.
- To be "the stichos MCP server" you must reproduce the CLI's wiring; the real handlers live in the
  binary-only `crates/cli`.
- Requires a tokio runtime.
- The HTTP review routes shell out to `soffice`/`pdftoppm`/`gs`/`mutool` for rendering and text
  extraction.
- It does not parse, render, or edit documents itself.

## Related crates

- [`stichos-core`](https://crates.io/crates/stichos-core) — core shared types.
- [`engines`](https://crates.io/crates/engines) — render engines the CLI's handlers drive.
- [`stichos-lint`](https://crates.io/crates/stichos-lint) — fidelity linting used by the tool
  surface.
- [`adapters-ooxml`](https://crates.io/crates/adapters-ooxml) — docx/xlsx/pptx read and write.
- [`adapters-odf`](https://crates.io/crates/adapters-odf) — odt/ods/odp read and write.
- [`adapters-pdf`](https://crates.io/crates/adapters-pdf) — PDF parsing and mutation.

## License

Dual-licensed under **MIT OR Apache-2.0**. See [LICENSE-MIT](https://github.com/reinartz/stichos/blob/main/LICENSE-MIT)
and [LICENSE-APACHE](https://github.com/reinartz/stichos/blob/main/LICENSE-APACHE).
