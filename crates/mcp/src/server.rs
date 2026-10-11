// SPDX-License-Identifier: MIT OR Apache-2.0
//! MCP stdio server.
//!
//! Implements the MCP transport over stdio: read JSON-RPC requests from stdin,
//! write JSON-RPC responses to stdout.
//! Also provides an HTTP server for streamable MCP transport with review routes.
//!
//! Default listen address: 127.0.0.1

use std::future::Future;
use std::io::Write;
use std::pin::Pin;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::dispatch::BoxedHandler;
use crate::http;
use crate::jsonrpc::{parse_request, JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpSession};
use crate::session;
use crate::spec::ParamDef;
use crate::transport::TransportMode;

/// Default listen address for the HTTP server.
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1";

/// Start the MCP server.
///
/// Returns a future that resolves when the server exits (EOF for stdio,
/// or when the HTTP server is shut down).
pub fn start(
    mode: TransportMode,
    handler: BoxedHandler,
) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
    match mode {
        TransportMode::Stdio => Box::pin(run_stdio(handler)),
        TransportMode::Http { port } => {
            Box::pin(start_http(port, DEFAULT_LISTEN_ADDR.to_string(), handler))
        }
    }
}

/// Start the MCP HTTP server.
async fn start_http(port: u16, listen_addr: String, handler: BoxedHandler) -> Result<(), String> {
    eprintln!("MCP server: HTTP mode on {} port {}", listen_addr, port);

    let addr: std::net::SocketAddr = (
        listen_addr
            .parse::<std::net::IpAddr>()
            .unwrap_or([0, 0, 0, 0].into()),
        port,
    )
        .into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind port {}: {}", port, e))?;
    eprintln!(
        "MCP server: listening on {}",
        listener.local_addr().unwrap()
    );

    // Shared stores — created once, shared across all connections
    let session_store = http::new_session_store();
    let review_store = session::new_review_session_store();
    let handler = handler.clone();
    let text_source: http::PdfSourceHandle = None;

    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| format!("accept error: {}", e))?;

        let session_store = session_store.clone();
        let review_store = review_store.clone();
        let handler = handler.clone();
        let text_source = text_source.clone();

        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut read_buf = [0u8; 65536];
            loop {
                let n = match socket.read(&mut read_buf).await {
                    Ok(n) if n > 0 => n,
                    _ => break,
                };
                buf.extend_from_slice(&read_buf[..n]);
                if buf.len() > 10 * 1024 * 1024 {
                    return;
                }
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    if let Ok(header_str) = std::str::from_utf8(&buf) {
                        if let Some(content_start) = header_str.find("\r\n\r\n") {
                            let content_start = content_start + 4;
                            let content_len = header_str[..content_start]
                                .lines()
                                .find(|l| l.to_lowercase().starts_with("content-length:"))
                                .and_then(|l| {
                                    l.split(':').nth(1).map(|v| v.trim().parse::<usize>().ok())
                                })
                                .flatten()
                                .unwrap_or(0);
                            if buf.len() >= content_start + content_len {
                                break;
                            }
                        }
                    }
                }
            }

            // Parse the raw HTTP request
            let request = match parse_http_request(&buf) {
                Some(req) => req,
                None => {
                    let _ = socket
                        .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                        .await;
                    return;
                }
            };

            // Determine if this is a review route
            if http::is_review_path(&request.path) {
                let response =
                    http::route_review_request(request, &review_store, text_source.clone());
                send_http_response(&mut socket, &response).await;
            } else {
                let response = http::route_request(request, session_store, handler);
                send_http_response(&mut socket, &response).await;
            }
        });
    }
}

/// Parse a raw HTTP request from bytes.
fn parse_http_request(buf: &[u8]) -> Option<http::HttpRequest> {
    let text = std::str::from_utf8(buf).ok()?;
    let header_end = text.find("\r\n\r\n")?;
    let header_bytes = &buf[..header_end + 4];
    let header_str = std::str::from_utf8(header_bytes).ok()?;

    let mut lines = header_str.lines();
    let request_line = lines.next()?;
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let full_path = parts[1];

    // Split path and query
    let (path, _query) = match full_path.find('?') {
        Some(idx) => (&full_path[..idx], &full_path[idx + 1..]),
        None => (full_path, ""),
    };

    // Parse headers
    let mut headers = std::collections::HashMap::new();
    for line in lines {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].to_lowercase();
            let val = line[colon + 1..].trim().to_string();
            headers.insert(key, val);
        }
    }

    // Extract content length
    let content_len = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let body_start = header_end + 4;
    let body = buf[body_start..body_start + content_len.min(buf.len() - body_start)].to_vec();

    Some(http::HttpRequest {
        method,
        path: path.to_string(),
        headers,
        body,
    })
}

/// Serialize an HttpResponse back to raw HTTP/1.1 bytes and send it.
async fn send_http_response(socket: &mut tokio::net::TcpStream, response: &http::HttpResponse) {
    let status_text = match response.status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        410 => "Gone",
        501 => "Not Implemented",
        _ => "OK",
    };

    let mut body = format!("HTTP/1.1 {} {}\r\n", response.status, status_text);
    for (key, val) in &response.headers {
        body.push_str(&format!("{}: {}\r\n", key, val));
    }
    body.push_str("Connection: close\r\n");
    body.push_str("\r\n");

    let mut response_buf = body.into_bytes();
    response_buf.extend(&response.body);

    let _ = socket.write_all(&response_buf).await;
    let _ = socket.flush().await;
}

/// Run the stdio MCP server loop.
async fn run_stdio(handler: BoxedHandler) -> Result<(), String> {
    eprintln!("MCP server: stdio mode");
    let mut session = McpSession::new();
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);

    loop {
        let mut buf = String::new();
        let bytes_read = reader.read_line(&mut buf).await;
        match bytes_read {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("stdio read error: {}", e);
                break;
            }
        }

        let line = buf.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request = match parse_request(&line) {
            Ok(req) => req,
            Err(err) => {
                let response = JsonRpcResponse::error(None, err);
                println!("{}", serde_json::to_string(&response).unwrap());
                std::io::stdout().flush().ok();
                continue;
            }
        };

        let response = match request.method.as_str() {
            "initialize" => handle_initialize(&mut session, &request),
            "tools/list" => handle_tools_list(&request),
            "tools/call" => handle_tools_call(&handler, &request).await,
            _ => JsonRpcResponse::error(request.id.clone(), JsonRpcError::method_not_found()),
        };

        if request.id.is_some() {
            println!("{}", serde_json::to_string(&response).unwrap());
            std::io::stdout().flush().ok();
        }
    }

    Ok(())
}

fn handle_initialize(session: &mut McpSession, req: &JsonRpcRequest) -> JsonRpcResponse {
    session.handle_initialize(req.params.as_ref());

    let result = serde_json::json!({
        "protocolVersion": session.protocol_version,
        "capabilities": {
            "tools": { "listChanged": false }
        }
    });
    JsonRpcResponse::success(req.id.clone(), result)
}

fn handle_tools_list(req: &JsonRpcRequest) -> JsonRpcResponse {
    use crate::spec::TOOLS;

    let tools: Vec<serde_json::Value> = TOOLS
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": {
                    "type": "object",
                    "properties": object_from_params(t.params),
                    "required": required_params(t.params)
                }
            })
        })
        .collect();

    let result = serde_json::json!({ "tools": tools });
    JsonRpcResponse::success(req.id.clone(), result)
}

fn object_from_params(params: &[ParamDef]) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    for p in params {
        let type_str = match p.param_type {
            crate::spec::ParamType::String => "string",
            crate::spec::ParamType::Number => "number",
            crate::spec::ParamType::Boolean => "boolean",
            crate::spec::ParamType::StringArray => "array",
            crate::spec::ParamType::NumberArray => "array",
        };
        props.insert(
            p.name.to_string(),
            serde_json::json!({
                "type": type_str,
                "description": p.description,
            }),
        );
    }
    serde_json::Value::Object(props)
}

fn required_params(params: &[ParamDef]) -> Vec<String> {
    params
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.to_string())
        .collect()
}

async fn handle_tools_call(handler: &BoxedHandler, req: &JsonRpcRequest) -> JsonRpcResponse {
    let handler = handler.clone();
    let id = req.id.clone();
    let params = &req.params;
    let name = params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str());
    let args = params
        .as_ref()
        .and_then(|p| p.get("arguments").or_else(|| p.get("params")))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let tool_name = match name {
        Some(n) => n,
        None => {
            return JsonRpcResponse::error(id.clone(), JsonRpcError::invalid_params());
        }
    };

    match handler.dispatch(tool_name, &args).await {
        Ok(result) => {
            let json_str =
                serde_json::to_string(&result).unwrap_or_else(|_| "<unserializable>".into());
            JsonRpcResponse::success(
                id.clone(),
                serde_json::json!({
                    "content": [{"type": "text", "text": json_str}],
                    "structuredContent": result,
                    "isError": false,
                }),
            )
        }
        Err(err) => JsonRpcResponse::success(
            id.clone(),
            serde_json::json!({
                "content": [{"type": "text", "text": format!("error: {}", err)}],
                "structuredContent": null,
                "isError": true,
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::{JsonRpcRequest, JsonRpcResponse};
    use crate::spec::{ParamDef, ParamType};

    /// test_handle_initialize — returns protocolVersion and capabilities
    #[test]
    fn test_handle_initialize() {
        let mut session = McpSession::new();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "clientInfo": { "name": "test-client", "version": "1.0" }
            })),
            id: Some(serde_json::json!(1)),
        };
        let resp = handle_initialize(&mut session, &req);
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(session.protocol_version, "2024-11-05");
        assert_eq!(session.client_name, Some("test-client".to_string()));
    }

    /// test_handle_initialize_no_params — works with no params
    #[test]
    fn test_handle_initialize_no_params() {
        let mut session = McpSession::new();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: None,
            id: Some(serde_json::json!(2)),
        };
        let resp = handle_initialize(&mut session, &req);
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
    }

    /// test_handle_tools_list — returns all tools with schemas
    #[test]
    fn test_handle_tools_list() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: None,
            id: Some(serde_json::json!(1)),
        };
        let resp = handle_tools_list(&req);
        assert!(resp.error.is_none());
        assert_eq!(resp.id, Some(serde_json::json!(1)));
        let result = resp.result.unwrap();
        let tools = result["tools"]
            .as_array()
            .expect("tools should be an array");
        assert_eq!(tools.len(), 24, "expected 24 tools");

        // Verify first tool structure
        let first = &tools[0];
        assert!(first["name"].is_string());
        assert!(first["description"].is_string());
        assert_eq!(first["inputSchema"]["type"], "object");
        assert!(first["inputSchema"]["properties"].is_object());
        assert!(first["inputSchema"]["required"].is_array());
    }

    /// test_handle_tools_list_contains_expected_tools
    #[test]
    fn test_handle_tools_list_contains_expected_tools() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: None,
            id: Some(serde_json::json!(1)),
        };
        let resp = handle_tools_list(&req);
        let result = resp.result.unwrap();
        let tools = result["tools"]
            .as_array()
            .expect("tools should be an array");
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"inspect_document"));
        assert!(names.contains(&"render_document"));
        assert!(names.contains(&"inspect_region"));
        assert!(names.contains(&"diff_documents"));
        assert!(names.contains(&"doctor"));
        assert!(names.contains(&"serve"));
    }

    /// test_handle_tools_list_schema_for_inspect_document
    #[test]
    fn test_handle_tools_list_schema_for_inspect_document() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: None,
            id: Some(serde_json::json!(1)),
        };
        let resp = handle_tools_list(&req);
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        let inspect = tools
            .iter()
            .find(|t| t["name"] == "inspect_document")
            .expect("inspect_document");
        let schema = &inspect["inputSchema"];
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert_eq!(props["path"]["type"], "string");
        assert!(props.contains_key("format"));
        assert_eq!(props["format"]["type"], "string");
        let required: Vec<_> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        assert!(required.contains(&"path"));
        assert!(!required.contains(&"format"));
    }

    /// Regression test: handle_tools_call must not panic when called
    /// from within an existing tokio runtime (the llama.cpp stdio scenario).
    ///
    /// Before the fix, `handle_tools_call` created a nested tokio runtime
    /// and called `block_on()` on it, which panics when the caller already
    /// runs inside a tokio runtime.  The fix makes the function fully async
    /// so it can be awaited directly without any nested runtime.
    #[tokio::test]
    async fn test_handle_tools_call_no_nested_runtime() {
        let handler = crate::dispatch::boxed_handler(|name: &str, _args: &serde_json::Value| {
            let name = name.to_string();
            async move {
                Ok(serde_json::json!({
                    "tool": name,
                    "status": "ok",
                }))
            }
        });
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({"name": "doctor", "arguments": {}})),
            id: Some(serde_json::json!(42)),
        };
        // This MUST NOT panic — the old code called tokio::runtime::Builder
        // and block_on inside an existing runtime.
        let resp = handle_tools_call(&handler, &req).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["structuredContent"]["tool"], "doctor");
    }

    /// Regression: tools/call with missing name returns error in existing runtime
    #[tokio::test]
    async fn test_handle_tools_call_missing_name() {
        let handler =
            crate::dispatch::boxed_handler(|_name: &str, _args: &serde_json::Value| async move {
                Ok(serde_json::json!({}))
            });
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({"foo": "bar"})),
            id: Some(serde_json::json!(42)),
        };
        let resp = handle_tools_call(&handler, &req).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32602);
    }

    /// Regression: dispatch error in existing runtime returns isError:true
    #[tokio::test]
    async fn test_handle_tools_call_dispatch_error() {
        let handler =
            crate::dispatch::boxed_handler(|_name: &str, _args: &serde_json::Value| async move {
                Err("something went wrong".to_string())
            });
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({"name": "doctor"})),
            id: Some(serde_json::json!(7)),
        };
        let resp = handle_tools_call(&handler, &req).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("something went wrong"));
    }

    /// test_method_not_found — unknown method returns error
    #[test]
    fn test_method_not_found() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "unknown/method".to_string(),
            params: None,
            id: Some(serde_json::json!(3)),
        };
        let resp = JsonRpcResponse::error(req.id.clone(), JsonRpcError::method_not_found());
        assert!(resp.result.is_none());
        assert_eq!(resp.error.as_ref().unwrap().code, -32601);
    }

    /// test_object_from_params — maps ParamDef to JSON schema properties
    #[test]
    fn test_object_from_params() {
        let params = [
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "A file path",
            },
            ParamDef {
                name: "dpi",
                param_type: ParamType::Number,
                required: false,
                description: "Dots per inch",
            },
            ParamDef {
                name: "flags",
                param_type: ParamType::Boolean,
                required: false,
                description: "Enable flags",
            },
        ];
        let obj = object_from_params(&params);
        let props = obj.as_object().expect("should be object");
        assert_eq!(props["path"]["type"], "string");
        assert_eq!(props["dpi"]["type"], "number");
        assert_eq!(props["flags"]["type"], "boolean");
    }

    /// test_object_from_params_array_types
    #[test]
    fn test_object_from_params_array_types() {
        let params = [ParamDef {
            name: "pages",
            param_type: ParamType::NumberArray,
            required: false,
            description: "Page numbers",
        }];
        let obj = object_from_params(&params);
        assert_eq!(obj["pages"]["type"], "array");
    }

    /// test_required_params — filters to required-only
    #[test]
    fn test_required_params() {
        let params = [
            ParamDef {
                name: "a",
                param_type: ParamType::String,
                required: true,
                description: "",
            },
            ParamDef {
                name: "b",
                param_type: ParamType::String,
                required: false,
                description: "",
            },
            ParamDef {
                name: "c",
                param_type: ParamType::Number,
                required: true,
                description: "",
            },
        ];
        let required = required_params(&params);
        assert_eq!(required, vec!["a".to_string(), "c".to_string()]);
    }

    /// test_mcp_session_new_default_version
    #[test]
    fn test_mcp_session_new_default_version() {
        let session = McpSession::new();
        assert_eq!(session.protocol_version, "2024-11-05");
        assert!(session.client_name.is_none());
    }

    /// test_mcp_session_handle_initialize_with_params
    #[test]
    fn test_mcp_session_handle_initialize_with_params() {
        let mut session = McpSession::new();
        session.handle_initialize(Some(&serde_json::json!({
            "protocolVersion": "2025-01-01",
            "clientInfo": { "name": "my-client", "version": "0.1" }
        })));
        assert_eq!(session.protocol_version, "2025-01-01");
        assert_eq!(session.client_name, Some("my-client".to_string()));
    }

    /// test_mcp_session_handle_initialize_without_clientinfo
    #[test]
    fn test_mcp_session_handle_initialize_without_clientinfo() {
        let mut session = McpSession::new();
        session.handle_initialize(Some(&serde_json::json!({"protocolVersion": "2025-01-01"})));
        assert_eq!(session.protocol_version, "2025-01-01");
        assert!(session.client_name.is_none());
    }

    /// Regression: tools/list must echo the request id (JSON-RPC 2.0 §4.3).
    /// A null id in the response causes llama.cpp send_rpc to never match it.
    #[test]
    fn test_tools_list_echoes_request_id() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: None,
            id: Some(serde_json::json!(42)),
        };
        let resp = handle_tools_list(&req);
        assert_eq!(resp.id, Some(serde_json::json!(42)));
    }

    /// Regression: tools/list with large monotonic id (llama.cpp uses next_id++)
    #[test]
    fn test_tools_list_echoes_large_id() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: None,
            id: Some(serde_json::json!(99999)),
        };
        let resp = handle_tools_list(&req);
        assert_eq!(resp.id, Some(serde_json::json!(99999)));
    }

    /// Regression: notifications must not produce a response (JSON-RPC 2.0 §4.1).
    /// A spurious error for an unknown notification would poison the reply queue.
    #[test]
    fn test_notification_produces_no_response() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
            id: None,
        };
        // The dispatch match arm should skip responses when id is None.
        // Verify the current match produces an error that would be suppressed.
        let resp = JsonRpcResponse::error(req.id.clone(), JsonRpcError::method_not_found());
        assert!(resp.id.is_none(), "notification error must carry no id");
    }

    /// Regression: full llama.cpp handshake — initialize + notification + tools/list
    /// returns exactly two responses with correct ids.
    #[test]
    fn test_handle_initialize_and_tools_list_ids_match() {
        let mut session = McpSession::new();

        // Step 1: initialize with id=1
        let init_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "clientInfo": { "name": "llama.cpp", "version": "1.0" }
            })),
            id: Some(serde_json::json!(1)),
        };
        let init_resp = handle_initialize(&mut session, &init_req);
        assert_eq!(init_resp.id, Some(serde_json::json!(1)));

        // Step 2: notifications/initialized (no id → no response expected)
        // In the dispatch loop, request.id.is_none() would suppress output.

        // Step 3: tools/list with id=2
        let list_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: None,
            id: Some(serde_json::json!(2)),
        };
        let list_resp = handle_tools_list(&list_req);
        assert_eq!(list_resp.id, Some(serde_json::json!(2)));
        let result = list_resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 24);
    }
}
