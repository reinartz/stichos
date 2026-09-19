// SPDX-License-Identifier: MIT OR Apache-2.0
//! MCP JSON-RPC 2.0 implementation.
//!
//! Minimal JSON-RPC 2.0 layer for the MCP protocol.
//! Supports: initialize, tools/list, tools/call, notifications, errors.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    pub id: Option<Value>,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<Value>,
}

/// A JSON-RPC 2.0 error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".into(),
            data: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: -32600,
            message: "Invalid Request".into(),
            data: None,
        }
    }

    pub fn method_not_found() -> Self {
        Self {
            code: -32601,
            message: "Method not found".into(),
            data: None,
        }
    }

    pub fn invalid_params() -> Self {
        Self {
            code: -32602,
            message: "Invalid params".into(),
            data: None,
        }
    }

    pub fn internal_error() -> Self {
        Self {
            code: -32603,
            message: "Internal error".into(),
            data: None,
        }
    }
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: Option<Value>, err: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(err),
            id,
        }
    }
}

/// Parse a JSON-RPC request from a string.
pub fn parse_request(input: &str) -> Result<JsonRpcRequest, JsonRpcError> {
    let req: JsonRpcRequest =
        serde_json::from_str(input).map_err(|_| JsonRpcError::parse_error())?;

    if req.jsonrpc != "2.0" {
        return Err(JsonRpcError::invalid_request());
    }

    if req.method.is_empty() {
        return Err(JsonRpcError::invalid_request());
    }

    Ok(req)
}

/// Build a JSON-RPC request value for a given method and optional params.
pub fn build_request(method: &str, params: Option<Value>, id: impl Into<Value>) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": id.into()
    })
}

/// An MCP session state.
#[derive(Debug, Clone, Default)]
pub struct McpSession {
    pub protocol_version: String,
    pub client_name: Option<String>,
}

impl McpSession {
    pub fn new() -> Self {
        Self {
            protocol_version: "2024-11-05".to_string(),
            client_name: None,
        }
    }

    /// Initialize the session from an initialize request.
    pub fn handle_initialize(&mut self, params: Option<&Value>) {
        if let Some(p) = params {
            if let Some(ver) = p.get("protocolVersion").and_then(|v| v.as_str()) {
                self.protocol_version = ver.to_string();
            }
            if let Some(client) = p
                .get("clientInfo")
                .and_then(|ci| ci.get("name"))
                .and_then(|n| n.as_str())
            {
                self.client_name = Some(client.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_parse_valid_request — valid JSON-RPC 2.0
    #[test]
    fn test_parse_valid_request() {
        let input = r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#;
        let req = parse_request(input).expect("valid request");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, Some(serde_json::json!(1)));
    }

    /// test_parse_notification — notification without id
    #[test]
    fn test_parse_notification() {
        let input = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req = parse_request(input).expect("notification is valid");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "notifications/initialized");
        assert_eq!(req.id, None);
    }

    /// test_parse_parse_error — invalid JSON → parse error
    #[test]
    fn test_parse_parse_error() {
        let input = r#"not json at all"#;
        let err = parse_request(input).expect_err("should fail");
        assert_eq!(err.code, -32700);
        assert_eq!(err.message, "Parse error");
    }

    /// test_parse_wrong_version — non-2.0 → invalid request
    #[test]
    fn test_parse_wrong_version() {
        let input = r#"{"jsonrpc":"1.0","method":"test","id":1}"#;
        let err = parse_request(input).expect_err("should fail");
        assert_eq!(err.code, -32600);
    }

    /// test_parse_no_method — empty method → invalid request
    #[test]
    fn test_parse_no_method() {
        let input = r#"{"jsonrpc":"2.0","method":"","id":1}"#;
        let err = parse_request(input).expect_err("should fail");
        assert_eq!(err.code, -32600);
    }

    /// test_response_success — creates success response
    #[test]
    fn test_response_success() {
        let resp =
            JsonRpcResponse::success(Some(serde_json::json!(1)), serde_json::json!({"ok": true}));
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.error.is_none());
        assert_eq!(resp.id, Some(serde_json::json!(1)));
        assert_eq!(resp.result.as_ref().unwrap()["ok"], true);
    }

    /// test_response_error — creates error response
    #[test]
    fn test_response_error() {
        let err = JsonRpcError::method_not_found();
        let resp = JsonRpcResponse::error(Some(serde_json::json!(2)), err.clone());
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        assert_eq!(resp.error.as_ref().unwrap().code, -32601);
        assert_eq!(resp.id, Some(serde_json::json!(2)));
    }

    /// test_jsonrpc_error_codes — verify standard error codes
    #[test]
    fn test_jsonrpc_error_codes() {
        let parse = JsonRpcError::parse_error();
        assert_eq!(parse.code, -32700);
        assert_eq!(parse.message, "Parse error");

        let invalid = JsonRpcError::invalid_request();
        assert_eq!(invalid.code, -32600);
        assert_eq!(invalid.message, "Invalid Request");

        let not_found = JsonRpcError::method_not_found();
        assert_eq!(not_found.code, -32601);
        assert_eq!(not_found.message, "Method not found");

        let invalid_params = JsonRpcError::invalid_params();
        assert_eq!(invalid_params.code, -32602);
        assert_eq!(invalid_params.message, "Invalid params");

        let internal = JsonRpcError::internal_error();
        assert_eq!(internal.code, -32603);
        assert_eq!(internal.message, "Internal error");
    }

    /// test_serialize_deserialize_request — roundtrip
    #[test]
    fn test_serialize_deserialize_request() {
        let original = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: Some(
                serde_json::json!({"name": "inspect_document", "arguments": {"path": "doc.docx"}}),
            ),
            id: Some(serde_json::json!(42)),
        };
        let serialized = serde_json::to_string(&original).expect("serialize request");
        let deserialized: JsonRpcRequest =
            serde_json::from_str(&serialized).expect("deserialize request");
        assert_eq!(deserialized, original);
    }

    /// test_serialize_deserialize_response — roundtrip
    #[test]
    fn test_serialize_deserialize_response() {
        let original = JsonRpcResponse::success(
            Some(serde_json::json!(3)),
            serde_json::json!({"tools": [{"name": "inspect_document"}]}),
        );
        let serialized = serde_json::to_string(&original).expect("serialize response");
        let deserialized: JsonRpcResponse =
            serde_json::from_str(&serialized).expect("deserialize response");
        assert_eq!(deserialized, original);
    }
}
