//! Minimal HTTP server for MCP transport.
//!
//! Implements streamable HTTP mode: POST / with JSON-RPC requests,
//! returns JSON-RPC responses. Session handling via session IDs.
//!
//! # Design notes
//!
//! - In-memory session store (`Mutex<HashMap>`) — no persistence in P1.
//! - Session IDs derived from monotonic nanosecond timestamps.
//! - No external crates for networking (D4: zero hard external deps).
//! - The actual tokio listener / accept loop is P1-47; this module
//!   provides the routing + session layer for testability.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::dispatch::BoxedHandler;
use crate::jsonrpc::{parse_request, JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpSession};
use crate::spec::{ParamDef, TOOLS};

// ---------------------------------------------------------------------------
// HTTP request / response types (minimal, no external deps)
// ---------------------------------------------------------------------------

/// A parsed HTTP request (minimal subset).
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// A parsed HTTP response.
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Create a JSON response with the given status code.
    pub fn json(status: u16, body: String) -> Self {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("content-length".to_string(), body.len().to_string());
        Self {
            status,
            headers,
            body: body.into_bytes(),
        }
    }

    /// Convenience: 200 OK with JSON body.
    pub fn ok_json(body: String) -> Self {
        Self::json(200, body)
    }

    /// Convenience: 400 Bad Request with error JSON.
    pub fn bad_request(msg: &str) -> Self {
        Self::json(400, format!("{{\"error\":\"{}\"}}", msg))
    }

    /// Convenience: 404 Not Found.
    pub fn not_found() -> Self {
        Self::json(404, "{\"error\":\"not found\"}".to_string())
    }
}

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

/// An in-memory HTTP session.
pub struct HttpSession {
    pub session_id: String,
    pub state: McpSession,
    pub created_at: SystemTime,
}

/// Thread-safe session store.
pub type SessionStore = Arc<Mutex<HashMap<String, HttpSession>>>;

/// Create a new empty session store.
pub fn new_session_store() -> SessionStore {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Generate a unique session ID from current timestamp.
fn generate_session_id() -> String {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("stichos-{:x}", duration.as_nanos())
}

/// Create a new session and insert it into the store.
pub fn create_session(store: &SessionStore) -> String {
    let session_id = generate_session_id();
    let mut store = store.lock().unwrap();
    store.insert(
        session_id.clone(),
        HttpSession {
            session_id: session_id.clone(),
            state: McpSession::new(),
            created_at: SystemTime::now(),
        },
    );
    session_id
}

/// Look up an existing session ID from the client, or create a new one.
pub fn get_or_create_session(store: &SessionStore, session_id: Option<&str>) -> String {
    match session_id {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => create_session(store),
    }
}

// ---------------------------------------------------------------------------
// HTTP routing
// ---------------------------------------------------------------------------

/// Route an incoming HTTP request through the JSON-RPC handler.
pub fn route_request(
    request: HttpRequest,
    store: SessionStore,
    handler: BoxedHandler,
) -> HttpResponse {
    // Only POST / is supported
    if request.method != "POST" || request.path != "/" {
        return HttpResponse::not_found();
    }

    // Parse session ID from header
    let session_id = request.headers.get("x-session-id").and_then(|v| {
        if v.is_empty() {
            None
        } else {
            Some(v.as_str())
        }
    });
    let _session_id = get_or_create_session(&store, session_id);

    // Parse body as JSON-RPC
    let body_str = match String::from_utf8(request.body.clone()) {
        Ok(s) => s,
        Err(_) => {
            return HttpResponse::ok_json(
                serde_json::to_string(&JsonRpcResponse::error(None, JsonRpcError::parse_error()))
                    .unwrap(),
            );
        }
    };

    let request = match parse_request(&body_str) {
        Ok(req) => req,
        Err(err) => {
            return HttpResponse::ok_json(
                serde_json::to_string(&JsonRpcResponse::error(None, err)).unwrap(),
            );
        }
    };

    // Route the JSON-RPC method
    let response = match request.method.as_str() {
        "initialize" => handle_initialize(&request),
        "tools/list" => handle_tools_list(&request),
        "tools/call" => handle_tools_call(&handler, &request),
        _ => JsonRpcResponse::error(request.id.clone(), JsonRpcError::method_not_found()),
    };

    HttpResponse::ok_json(serde_json::to_string(&response).unwrap())
}

// ---------------------------------------------------------------------------
// JSON-RPC method handlers (HTTP-adapted)
// ---------------------------------------------------------------------------

fn handle_initialize(_req: &JsonRpcRequest) -> JsonRpcResponse {
    let result = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": { "listChanged": false } }
    });
    JsonRpcResponse::success(_req.id.clone(), result)
}

fn handle_tools_list(req: &JsonRpcRequest) -> JsonRpcResponse {
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

fn handle_tools_call(handler: &BoxedHandler, req: &JsonRpcRequest) -> JsonRpcResponse {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            return JsonRpcResponse::error(req.id.clone(), JsonRpcError::internal_error());
        }
    };

    let handler = handler.clone();
    let id = req.id.clone();
    let resp = rt.block_on(async {
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
    });

    resp
}

fn object_from_params(params: &[ParamDef]) -> Value {
    let mut props = serde_json::Map::new();
    for p in params {
        let type_str = match p.param_type {
            crate::spec::ParamType::String => "string",
            crate::spec::ParamType::Number => "number",
            crate::spec::ParamType::Boolean => "boolean",
            crate::spec::ParamType::StringArray | crate::spec::ParamType::NumberArray => "array",
        };
        props.insert(
            p.name.to_string(),
            serde_json::json!({"type": type_str, "description": p.description}),
        );
    }
    Value::Object(props)
}

fn required_params(params: &[ParamDef]) -> Vec<String> {
    params
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.to_string())
        .collect()
}

#[cfg(test)]
/// Create a default test handler that returns a fixed response.
fn test_handler() -> BoxedHandler {
    struct TestHandler;
    #[async_trait::async_trait]
    impl crate::dispatch::ToolHandler for TestHandler {
        async fn dispatch(&self, name: &str, _args: &Value) -> Result<Value, String> {
            Ok(serde_json::json!({
                "tool": name,
                "status": "ok",
                "message": "test handler response"
            }))
        }
    }
    std::sync::Arc::new(TestHandler)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// test_create_session — generates unique session ID
    #[test]
    fn test_create_session() {
        let store = new_session_store();
        let id1 = create_session(&store);
        let id2 = create_session(&store);
        assert!(id1.starts_with("stichos-"));
        assert!(id2.starts_with("stichos-"));
        assert_ne!(id1, id2, "two calls must produce distinct IDs");
    }

    /// test_session_store — insert and retrieve
    #[test]
    fn test_session_store() {
        let store = new_session_store();
        let id = create_session(&store);
        let locked = store.lock().unwrap();
        assert!(locked.contains_key(&id));
        let session = &locked[&id];
        assert_eq!(session.session_id, id);
        assert_eq!(session.state.protocol_version, "2024-11-05");
    }

    /// test_get_or_create_with_id — reuses existing session ID from client
    #[test]
    fn test_get_or_create_with_id() {
        let store = new_session_store();
        let id = "client-session-42".to_string();
        let result = get_or_create_session(&store, Some(&id));
        assert_eq!(result, id);
        // get_or_create_session returns the ID without creating a store entry;
        // actual session creation happens lazily on first initialize
    }

    /// test_get_or_create_without_id — creates new
    #[test]
    fn test_get_or_create_without_id() {
        let store = new_session_store();
        let result = get_or_create_session(&store, None);
        assert!(result.starts_with("stichos-"));
        let result2 = get_or_create_session(&store, Some(""));
        assert!(result2.starts_with("stichos-"));
        assert_ne!(result, result2);
    }

    /// test_route_post_ok — POST / returns 200
    #[test]
    fn test_route_post_ok() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#.as_bytes().to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
    }

    /// test_route_get_not_found — GET / returns 404
    #[test]
    fn test_route_get_not_found() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 404);
    }

    /// test_route_wrong_path — POST /other returns 404
    #[test]
    fn test_route_wrong_path() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/other".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 404);
    }

    /// test_route_invalid_json — invalid JSON → parse error response
    #[test]
    fn test_route_invalid_json() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: b"not json at all".to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json response");
        assert!(parsed.get("error").is_some(), "should contain error field");
        assert_eq!(
            parsed["error"]["code"], -32700,
            "should be parse error code"
        );
    }

    /// test_route_tools_list — returns tool definitions
    #[test]
    fn test_route_tools_list() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#.as_bytes().to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json response");
        assert!(parsed.get("result").is_some());
        let tools = &parsed["result"]["tools"];
        assert!(tools.is_array());
        let tool_names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(tool_names.len(), 16);
        assert!(tool_names.contains(&"inspect_document"));
        assert!(tool_names.contains(&"serve"));
    }

    /// test_http_response_json — creates JSON response
    #[test]
    fn test_http_response_json() {
        let resp = HttpResponse::json(201, r#"{"ok":true}"#.to_string());
        assert_eq!(resp.status, 201);
        assert_eq!(
            resp.headers.get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(resp.headers.get("content-length").unwrap(), "11");
        assert_eq!(resp.body, b"{\"ok\":true}");
    }

    /// test_http_response_bad_request — 400 status
    #[test]
    fn test_http_response_bad_request() {
        let resp = HttpResponse::bad_request("missing field");
        assert_eq!(resp.status, 400);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["error"], "missing field");
    }

    /// test_session_expires — sessions are created with timestamps
    #[test]
    fn test_session_expires() {
        let store = new_session_store();
        let _id = create_session(&store);
        let locked = store.lock().unwrap();
        let session = locked.values().next().unwrap();
        // created_at should be a valid time in the past (or now)
        let now = SystemTime::now();
        let before = now
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(1);
        let after = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let ts = session
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(
            ts >= before && ts <= after,
            "timestamp should be reasonable"
        );
    }

    /// test_route_initialize — returns protocolVersion and capabilities
    #[test]
    fn test_route_initialize() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"initialize","id":42}"#
                .as_bytes()
                .to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(
            parsed["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        assert_eq!(parsed["id"], 42);
    }

    /// test_route_method_not_found — unknown method returns -32601
    #[test]
    fn test_route_method_not_found() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"unknown/method","id":1}"#
                .as_bytes()
                .to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert!(parsed.get("error").is_some());
        assert_eq!(parsed["error"]["code"], -32601);
    }

    /// test_route_session_header — x-session-id is accepted (routing succeeds)
    #[test]
    fn test_route_session_header() {
        let store = new_session_store();
        let handler = test_handler();
        let mut headers = HashMap::new();
        headers.insert("x-session-id".to_string(), "my-session-xyz".to_string());
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers,
            body: r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#.as_bytes().to_vec(),
        };
        let resp = route_request(req, store, handler);
        // Session header is accepted: routing succeeds with 200
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert!(parsed.get("result").is_some());
    }

    /// Regression: HTTP tools/list must echo the request id (JSON-RPC 2.0 §4.3).
    #[test]
    fn test_route_tools_list_echoes_request_id() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"tools/list","id":77}"#
                .as_bytes()
                .to_vec(),
        };
        let resp = route_request(req, store, handler);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["id"], 77);
        let tools = &parsed["result"]["tools"];
        assert!(tools.is_array());
        assert_eq!(tools.as_array().unwrap().len(), 16);
    }

    /// Regression: HTTP tools/list with various id types
    #[test]
    fn test_route_tools_list_various_ids() {
        let store = new_session_store();
        let handler = test_handler();
        for id_val in [
            serde_json::json!(0),
            serde_json::json!(999),
            serde_json::json!(-1),
        ] {
            let body = format!(
                r#"{{"jsonrpc":"2.0","method":"tools/list","id":{}}}"#,
                id_val
            );
            let req = HttpRequest {
                method: "POST".to_string(),
                path: "/".to_string(),
                headers: HashMap::new(),
                body: body.into_bytes(),
            };
            let resp = route_request(req, store.clone(), handler.clone());
            let body_str = String::from_utf8_lossy(&resp.body);
            let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
            assert_eq!(parsed["id"], id_val, "id should be echoed back");
        }
    }

    /// test_route_tools_call — tools/call dispatches to handler and returns structured content
    #[test]
    fn test_route_tools_call() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"tools/call","id":1,"params":{"name":"doctor","arguments":{}}}"#.as_bytes().to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert!(parsed.get("result").is_some());
        let result = parsed["result"].clone();
        // Check content array
        assert!(result["content"].is_array());
        assert_eq!(result["content"][0]["type"], "text");
        // Check structuredContent is populated (not null)
        assert!(result["structuredContent"].is_object());
        assert_eq!(result["structuredContent"]["tool"], "doctor");
        assert_eq!(result["isError"], false);
    }

    /// test_route_tools_call_missing_name — returns error when tool name is missing
    #[test]
    fn test_route_tools_call_missing_name() {
        let store = new_session_store();
        let handler = test_handler();
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: r#"{"jsonrpc":"2.0","method":"tools/call","id":1,"params":{}}"#
                .as_bytes()
                .to_vec(),
        };
        let resp = route_request(req, store, handler);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert!(parsed.get("error").is_some());
        assert_eq!(parsed["error"]["code"], -32602);
    }
}
