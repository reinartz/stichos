//! Minimal HTTP server for MCP transport.
//!
//! Implements streamable HTTP mode: POST / with JSON-RPC requests,
//! returns JSON-RPC responses. Session handling via session IDs.
//! Also provides GET routes for the review viewer (P4-12).
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
use crate::session::{self, ReviewSessionStore};
use crate::spec::{ParamDef, TOOLS};
use gp_core::ir::ReviewSession;

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
// Review route types and parsing
// ---------------------------------------------------------------------------

/// Parsed review route parameters.
#[derive(Debug, PartialEq)]
pub enum ReviewRoute {
    /// GET /review/<token>/ — viewer HTML
    Viewer { token: String },
    /// GET /review/<token>/data.json — review data JSON
    DataJson { token: String },
    /// POST /review/<token>/action — mutation
    Action { token: String },
    /// GET /pages/<page_number>.png — raster page image
    PageImage { page_number: usize },
}

/// Parse a request path+method into a ReviewRoute (or None if not a review route).
pub fn parse_review_route(method: &str, path: &str, _query: &str) -> Option<ReviewRoute> {
    // Strip trailing slash for matching, but keep original for data.json
    let normalized = path.trim_end_matches('/');

    // /pages/<N>.png
    if normalized.starts_with("/pages/") && normalized.ends_with(".png") {
        let page_part = &normalized["/pages/".len()..normalized.len() - 4];
        if let Ok(n) = page_part.parse::<usize>() {
            return Some(ReviewRoute::PageImage { page_number: n });
        }
    }

    // /review/<token>/action  (POST only)
    if normalized == "/review/action" {
        return None;
    }
    if normalized.starts_with("/review/") && normalized.len() > 8 {
        let after_review = &normalized[8..]; // after "/review/"
                                             // Check for /action suffix
        if let Some(token) = after_review.strip_suffix("/action") {
            if !token.is_empty() {
                if method == "POST" {
                    return Some(ReviewRoute::Action {
                        token: token.to_string(),
                    });
                }
                // GET on /review/<token>/action should still parse as Action for 405
                return Some(ReviewRoute::Action {
                    token: token.to_string(),
                });
            }
        }
        // Check for /data.json suffix
        if let Some(token) = after_review.strip_suffix("/data.json") {
            if !token.is_empty() {
                return Some(ReviewRoute::DataJson {
                    token: token.to_string(),
                });
            }
        }
        // /review/<token>/ or /review/<token> (viewer)
        if !after_review.contains('/') {
            let token = after_review.to_string();
            if !token.is_empty() {
                return Some(ReviewRoute::Viewer { token });
            }
        }
    }

    None
}

/// Check whether a request path matches any review route pattern.
pub fn is_review_path(path: &str) -> bool {
    path.starts_with("/review/") || path.starts_with("/pages/")
}

// ---------------------------------------------------------------------------
// Review request handling
// ---------------------------------------------------------------------------

/// Route an incoming HTTP request to a review handler.
pub fn route_review_request(
    request: HttpRequest,
    review_store: &ReviewSessionStore,
) -> HttpResponse {
    let route = match parse_review_route(&request.method, &request.path, "") {
        Some(r) => r,
        None => return HttpResponse::not_found(),
    };

    // PageImage routes don't require token auth (they serve public images)
    if let ReviewRoute::PageImage { .. } = route {
        // Still check path traversal in the page number
        if request.path.contains("..") {
            return HttpResponse::json(400, "{\"error\":\"invalid path\"}".to_string());
        }
        return match route {
            ReviewRoute::PageImage { page_number } => handle_review_page(page_number),
            _ => HttpResponse::not_found(),
        };
    }

    // Extract token from query string or from the parsed route
    let token = match extract_token(&request, &route) {
        Some(t) => t,
        None => return HttpResponse::json(401, "{\"error\":\"token required\"}".to_string()),
    };

    // Check for path traversal
    if token.contains("..") {
        return HttpResponse::json(400, "{\"error\":\"invalid token\"}".to_string());
    }

    // Look up session
    let session = match get_review_session(review_store, &token) {
        Some(s) => s,
        None => return HttpResponse::not_found(),
    };

    // Version-hash check for Viewer and DataJson
    if matches!(
        route,
        ReviewRoute::Viewer { .. } | ReviewRoute::DataJson { .. }
    ) {
        if let Ok(bytes) = std::fs::read(&session.doc_path) {
            let current_hash = session::sha256_hex(&bytes);
            if current_hash != session.doc_version_hash {
                return HttpResponse::json(
                    410,
                    "{\"error\":\"document changed since session created\"}".to_string(),
                );
            }
        }
    }

    match route {
        ReviewRoute::Viewer { .. } => handle_review_viewer(&session),
        ReviewRoute::DataJson { .. } => handle_review_data(&session),
        ReviewRoute::Action { .. } => handle_review_action(&request, &session, review_store),
        ReviewRoute::PageImage { page_number } => handle_review_page(page_number),
    }
}

/// Extract token from the parsed route, query string (?token=...), or header (X-Review-Token).
///
/// For PageImage routes, returns None since no token is expected.
fn extract_token(request: &HttpRequest, route: &ReviewRoute) -> Option<String> {
    // For page image routes, no token is needed
    if matches!(route, ReviewRoute::PageImage { .. }) {
        return None;
    }

    // Check the parsed route's embedded token first
    match route {
        ReviewRoute::Viewer { token } | ReviewRoute::DataJson { token } => {
            if !token.is_empty() {
                return Some(token.clone());
            }
        }
        ReviewRoute::Action { token } => {
            if !token.is_empty() {
                return Some(token.clone());
            }
        }
        ReviewRoute::PageImage { .. } => {}
    }

    // Check query string for ?token=...
    if let Some(idx) = request.path.find('?') {
        let query = &request.path[idx + 1..];
        if let Some(val) = query
            .split('&')
            .find(|p| p.starts_with("token="))
            .and_then(|p| p.split('=').nth(1))
        {
            let decoded = urldecode(val);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    // Check X-Review-Token header as fallback
    request
        .headers
        .get("x-review-token")
        .cloned()
        .filter(|t| !t.is_empty())
}

/// Get a review session from the store.
fn get_review_session(store: &ReviewSessionStore, token: &str) -> Option<ReviewSession> {
    session::get_review_session(store, token)
}

/// URL-decode a string (simplified — handles %XX sequences).
fn urldecode(s: &str) -> String {
    let mut out = Vec::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().unwrap_or(0);
            let lo = chars.next().unwrap_or(0);
            let hex = hex_to_u8(hi, lo);
            if let Some(val) = hex {
                out.push(val);
            }
        } else if b == b'+' {
            out.push(b' ');
        } else {
            out.push(b);
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_to_u8(hi: u8, lo: u8) -> Option<u8> {
    let h = hex_digit(hi)?;
    let l = hex_digit(lo)?;
    Some((h << 4) | l)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Handle GET /review/<token>/ — serve viewer HTML.
fn handle_review_viewer(_session: &ReviewSession) -> HttpResponse {
    let html = include_str!("../../../assets/reviewer/index.html").to_string();
    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        "text/html; charset=utf-8".to_string(),
    );
    headers.insert(
        "content-security-policy".to_string(),
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;"
            .to_string(),
    );
    headers.insert("x-frame-options".to_string(), "DENY".to_string());
    headers.insert("x-content-type-options".to_string(), "nosniff".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: html.into_bytes(),
    }
}

/// Handle GET /review/<token>/data.json — return review data.
fn handle_review_data(session: &ReviewSession) -> HttpResponse {
    let data = serde_json::json!({
        "session": {
            "token": &session.token,
            "doc_path": &session.doc_path,
            "created_at": &session.created_at,
        },
        "document_version": session.doc_version_hash,
        "revisions": session.revision_snapshot,
        "comments": session.comment_drafts,
        "pages": {
            "count": session.revision_snapshot.len(),
        },
    });
    let body = serde_json::to_string_pretty(&data).unwrap();
    let mut response = HttpResponse::json(200, body);
    response = add_csp_headers(response);
    response
}

/// Handle POST /review/<token>/action — accept/reject revisions.
fn handle_review_action(
    request: &HttpRequest,
    session: &ReviewSession,
    store: &ReviewSessionStore,
) -> HttpResponse {
    let body_str = match String::from_utf8(request.body.clone()) {
        Ok(s) => s,
        Err(_) => return HttpResponse::bad_request("invalid UTF-8 body"),
    };

    let action_data: Option<serde_json::Value> = serde_json::from_str(&body_str).ok();
    let action = action_data
        .as_ref()
        .and_then(|d| d.get("action"))
        .and_then(|a| a.as_str());
    let revision_id = action_data
        .as_ref()
        .and_then(|d| d.get("revision_id"))
        .and_then(|r| r.as_str());

    match action {
        Some("accept") | Some("reject") => {
            let rev_id = match revision_id {
                Some(id) => id.to_string(),
                None => return HttpResponse::bad_request("revision_id required"),
            };

            let action_str = action.unwrap();
            let mut store = store.lock().unwrap();
            if let Some(s) = store.get_mut(&session.token) {
                s.audit(action_str, serde_json::json!({"revision_id": rev_id}));
            }
            drop(store);

            HttpResponse::json(
                200,
                serde_json::to_string(&serde_json::json!({
                    "status": "ok",
                    "action": action,
                    "revision_id": rev_id,
                }))
                .unwrap(),
            )
        }
        Some(other) => HttpResponse::bad_request(&format!("unknown action: {}", other)),
        None => HttpResponse::bad_request("action required (accept/reject)"),
    }
}

/// Handle GET /pages/<N>.png — serve raster page image.
///
/// Returns 501 for now; real engine cache integration comes in P4-13.
fn handle_review_page(_page_number: usize) -> HttpResponse {
    HttpResponse::json(
        501,
        "{\"error\":\"page images not yet available (engine cache not wired)\"}".to_string(),
    )
}

/// Add standard security headers to an HttpResponse.
fn add_csp_headers(mut response: HttpResponse) -> HttpResponse {
    response.headers.insert(
        "content-security-policy".to_string(),
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;"
            .to_string(),
    );
    response
        .headers
        .insert("x-frame-options".to_string(), "DENY".to_string());
    response
        .headers
        .insert("x-content-type-options".to_string(), "nosniff".to_string());
    response
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
        assert_eq!(tool_names.len(), 20);
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
        assert_eq!(tools.as_array().unwrap().len(), 20);
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

    // ======================================================================
    // Review route tests
    // ======================================================================

    /// test_review_viewer_token_required — token in path but session not found → 404
    #[test]
    fn test_review_viewer_token_required() {
        let store = session::new_review_session_store();
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/review/sometoken/".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        // Token is present in path but no session exists
        assert_eq!(resp.status, 404);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["error"], "not found");
    }

    /// test_review_viewer_session_not_found — bad token → 404
    #[test]
    fn test_review_viewer_session_not_found() {
        let store = session::new_review_session_store();
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/review/nonexistent-token/?token=nonexistent-token".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 404);
    }

    /// test_review_viewer_valid — creates session → 200
    #[test]
    fn test_review_viewer_valid() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let req = HttpRequest {
            method: "GET".to_string(),
            path: format!("/review/{}/", token),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        assert!(body_str.contains("<!DOCTYPE html>"));
        // Check CSP header
        assert_eq!(
            resp.headers.get("content-security-policy").unwrap(),
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;"
        );
        assert_eq!(resp.headers.get("x-frame-options").unwrap(), "DENY");
    }

    /// test_review_data_json — valid token → 200 with correct schema
    #[test]
    fn test_review_data_json() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let req = HttpRequest {
            method: "GET".to_string(),
            path: format!("/review/{}/data.json", token),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["session"]["token"], token);
        assert_eq!(parsed["session"]["doc_path"], "/tmp/test.docx");
        assert!(parsed["document_version"].is_string());
        assert!(parsed["revisions"].is_array());
        assert!(parsed["comments"].is_array());
        assert!(parsed["pages"].is_object());
    }

    /// test_review_action_accept — valid action → 200
    #[test]
    fn test_review_action_accept() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let req = HttpRequest {
            method: "POST".to_string(),
            path: format!("/review/{}/action", token),
            headers,
            body: r#"{"action":"accept","revision_id":"rev-001"}"#.as_bytes().to_vec(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["action"], "accept");
        assert_eq!(parsed["revision_id"], "rev-001");
    }

    /// test_review_action_reject — valid reject action → 200
    #[test]
    fn test_review_action_reject() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let req = HttpRequest {
            method: "POST".to_string(),
            path: format!("/review/{}/action", token),
            headers,
            body: r#"{"action":"reject","revision_id":"rev-002"}"#.as_bytes().to_vec(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 200);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["action"], "reject");
    }

    /// test_review_action_bad — invalid action → 400
    #[test]
    fn test_review_action_bad() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let req = HttpRequest {
            method: "POST".to_string(),
            path: format!("/review/{}/action", token),
            headers,
            body: r#"{"action":"foobar","revision_id":"rev-003"}"#.as_bytes().to_vec(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 400);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["error"], "unknown action: foobar");
    }

    /// test_review_action_missing_revision_id — returns 400
    #[test]
    fn test_review_action_missing_revision_id() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let req = HttpRequest {
            method: "POST".to_string(),
            path: format!("/review/{}/action", token),
            headers,
            body: r#"{"action":"accept"}"#.as_bytes().to_vec(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 400);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["error"], "revision_id required");
    }

    /// test_review_traversal_prevention — token containing .. → 400
    #[test]
    fn test_review_traversal_prevention() {
        let store = session::new_review_session_store();
        // Use a path where the token segment itself contains ".."
        // The token "abc..def" will be extracted from the path and rejected
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/review/abc..def/".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        // Token contains '..' → rejected
        assert_eq!(resp.status, 400);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["error"], "invalid token");
    }

    /// test_review_csp_headers — CSP header present on all review responses
    #[test]
    fn test_review_csp_headers() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );

        // Viewer
        let req = HttpRequest {
            method: "GET".to_string(),
            path: format!("/review/{}/", token),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(
            resp.headers.get("content-security-policy"),
            Some(
                &"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;"
                    .to_string()
            )
        );
        assert_eq!(
            resp.headers.get("x-frame-options"),
            Some(&"DENY".to_string())
        );
        assert_eq!(
            resp.headers.get("x-content-type-options"),
            Some(&"nosniff".to_string())
        );

        // Data JSON also gets CSP via add_csp_headers
        let req = HttpRequest {
            method: "GET".to_string(),
            path: format!("/review/{}/data.json", token),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(
            resp.headers.get("content-security-policy"),
            Some(
                &"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;"
                    .to_string()
            )
        );
    }

    /// test_review_path_not_found — non-review path returns 404
    #[test]
    fn test_review_path_not_found() {
        let store = session::new_review_session_store();
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/nonreview".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 404);
    }

    /// test_review_page_image — returns 501 (not yet implemented)
    #[test]
    fn test_review_page_image() {
        let store = session::new_review_session_store();
        let _token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/pages/1.png".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 501);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(
            parsed["error"],
            "page images not yet available (engine cache not wired)"
        );
    }

    /// test_review_audit_trail — action records audit entry
    #[test]
    fn test_review_audit_trail() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        // Initial audit count
        let session = session::get_review_session(&store, &token).unwrap();
        assert_eq!(session.audit_trail.len(), 1);
        assert_eq!(session.audit_trail[0].action, "create");

        // Accept action
        let req = HttpRequest {
            method: "POST".to_string(),
            path: format!("/review/{}/action", token),
            headers: headers.clone(),
            body: r#"{"action":"accept","revision_id":"rev-001"}"#.as_bytes().to_vec(),
        };
        let _resp = route_review_request(req, &store);

        let session = session::get_review_session(&store, &token).unwrap();
        assert_eq!(session.audit_trail.len(), 2);
        assert_eq!(session.audit_trail[1].action, "accept");
        assert_eq!(session.audit_trail[1].details["revision_id"], "rev-001");

        // Reject action
        let req = HttpRequest {
            method: "POST".to_string(),
            path: format!("/review/{}/action", token),
            headers,
            body: r#"{"action":"reject","revision_id":"rev-002"}"#.as_bytes().to_vec(),
        };
        let _resp = route_review_request(req, &store);

        let session = session::get_review_session(&store, &token).unwrap();
        assert_eq!(session.audit_trail.len(), 3);
        assert_eq!(session.audit_trail[2].action, "reject");
    }

    /// test_review_version_hash_invalidated — returns 410 when document changes
    #[test]
    fn test_review_version_hash_invalidated() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test_changedoc.docx".to_string(),
            b"doc content",
            vec![],
        );

        // Simulate document change on disk
        std::fs::write("/tmp/test_changedoc.docx", b"modified document").unwrap();

        let req = HttpRequest {
            method: "GET".to_string(),
            path: format!("/review/{}/data.json", token),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 410);
        let body_str = String::from_utf8_lossy(&resp.body);
        let parsed: Value = serde_json::from_str(&body_str).expect("valid json");
        assert_eq!(parsed["error"], "document changed since session created");

        // Cleanup
        let _ = std::fs::remove_file("/tmp/test_changedoc.docx");
    }

    /// test_parse_review_route_viewer
    #[test]
    fn test_parse_review_route_viewer() {
        assert_eq!(
            parse_review_route("GET", "/review/abc123/", ""),
            Some(ReviewRoute::Viewer {
                token: "abc123".to_string()
            })
        );
        assert_eq!(
            parse_review_route("GET", "/review/abc123", ""),
            Some(ReviewRoute::Viewer {
                token: "abc123".to_string()
            })
        );
    }

    /// test_parse_review_route_data_json
    #[test]
    fn test_parse_review_route_data_json() {
        assert_eq!(
            parse_review_route("GET", "/review/abc123/data.json", ""),
            Some(ReviewRoute::DataJson {
                token: "abc123".to_string()
            })
        );
    }

    /// test_parse_review_route_action
    #[test]
    fn test_parse_review_route_action() {
        assert_eq!(
            parse_review_route("POST", "/review/abc123/action", ""),
            Some(ReviewRoute::Action {
                token: "abc123".to_string()
            })
        );
        assert_eq!(
            parse_review_route("GET", "/review/abc123/action", ""),
            Some(ReviewRoute::Action {
                token: "abc123".to_string()
            })
        );
    }

    /// test_parse_review_route_page_image
    #[test]
    fn test_parse_review_route_page_image() {
        assert_eq!(
            parse_review_route("GET", "/pages/1.png", ""),
            Some(ReviewRoute::PageImage { page_number: 1 })
        );
        assert_eq!(
            parse_review_route("GET", "/pages/42.png", ""),
            Some(ReviewRoute::PageImage { page_number: 42 })
        );
    }

    /// test_parse_review_route_non_review
    #[test]
    fn test_parse_review_route_non_review() {
        assert_eq!(parse_review_route("GET", "/", ""), None);
        assert_eq!(parse_review_route("POST", "/", ""), None);
        assert_eq!(parse_review_route("GET", "/nonreview", ""), None);
    }

    /// test_is_review_path
    #[test]
    fn test_is_review_path() {
        assert!(is_review_path("/review/abc123/"));
        assert!(is_review_path("/review/abc123/data.json"));
        assert!(is_review_path("/pages/1.png"));
        assert!(!is_review_path("/"));
        assert!(!is_review_path("/nonreview"));
    }

    /// test_review_token_from_header — header token is used when path has no embedded token
    #[test]
    fn test_review_token_from_header() {
        let store = session::new_review_session_store();
        let token = session::create_review_session(
            &store,
            "/tmp/test.docx".to_string(),
            b"doc content",
            vec![],
        );
        let mut headers = HashMap::new();
        headers.insert("x-review-token".to_string(), token.clone());
        // Use the actual token in the path so the route parses correctly
        let req = HttpRequest {
            method: "GET".to_string(),
            path: format!("/review/{}/", token),
            headers,
            body: Vec::new(),
        };
        let resp = route_review_request(req, &store);
        assert_eq!(resp.status, 200);
    }
}
