# ADR-0003 — MCP Transport: custom JSON-RPC vs rmcp

| | |
|---|---|
| Status | **accepted** (2026-09-10) |
| Gate | Wave E — API-02 |
| Decision owner | **project owner** |
| Blocks | P1-45, P1-46 |
| Related | D2, D4, P1-44 (tool spec v1) |

## 1. Context

The stichos MCP server needs a transport layer to communicate with LLM agents.
The MCP protocol specifies two transport options: stdio (JSON-RPC 2.0 over stdin/stdout)
and HTTP (SSE + POST).

Two implementation approaches were evaluated for the JSON-RPC layer:

- **rmcp**: the official Rust SDK for MCP
- **Custom JSON-RPC**: a minimal hand-rolled implementation using only `serde`/`serde_json`

Constraints from D2 (zero hard external deps for core crates) and D4 (clean cross-compilation)
favor minimal dependencies.

## 2. Options

### Option A: rmcp SDK

Use `rmcp` as the transport layer.

### Option B: Custom JSON-RPC (recommended)

Build a minimal JSON-RPC 2.0 layer using only `serde` + `serde_json` (already workspace dependencies).

## 3. Evidence

| # | Source | Measured |
|---|--------|----------|
| 1 | `rmcp` crate — `Cargo.toml` deps | `rmcp` depends on `tokio`, `serde`, `serde_json`, `reqwest`, `url`, `tokio-util` — 6+ crates vs 0 for custom |
| 2 | MCP JSON-RPC spec | The protocol defines `initialize`, `tools/list`, `tools/call`, and error codes — all coverable in ~100 lines |
| 3 | D4 cross-compilation | `rmcp`'s `reqwest` + TLS deps complicate bare-metal and static linking targets |
| 4 | Phase 2 swap path | Custom layer exposes only `parse_request` + handler trait — rmcp can be swapped behind the same surface |

## 4. Consequences

### Option A: rmcp

**Pros:**
- Officially maintained, well-tested
- Handles edge cases in JSON-RPC parsing
- HTTP transport built in

**Cons:**
- Introduces 6+ external dependencies (reqwest, tokio-util, url, etc.)
- Complicates cross-compilation (TLS, native deps)
- Violates D2's zero-external-deps principle for P1
- Harder to audit for security review

### Option B: Custom JSON-RPC (chosen)

**Pros:**
- Zero new external dependencies
- Clean cross-compilation (D2, D4)
- ~100 lines of transport code
- Fully auditable
- Can swap to rmcp in Phase 2 without breaking the tool surface
- Explicit control over JSON-RPC behavior

**Cons:**
- Must implement own JSON-RPC edge case handling
- No built-in HTTP transport (implemented in P1-46)
- Must maintain own test coverage for parse/error paths

## Decision

Use a **custom JSON-RPC 2.0 implementation** for P1.

This implements:
- JSON-RPC 2.0 parse/serialize
- `initialize` handshake (protocolVersion + capabilities)
- `tools/list` — returns tool definitions from `spec::TOOLS`
- `tools/call` — P1 stub (actual tool implementation in P1-48+)
- Standard JSON-RPC error codes (-32700, -32600, -32601, -32602, -32603)
- stdio transport loop

HTTP transport is deferred to P1-46.
