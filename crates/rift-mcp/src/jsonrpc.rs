//! JSON-RPC 2.0 wire types — minimal subset used by MCP.
//!
//! MCP rides on JSON-RPC 2.0 with a few well-known methods (`initialize`,
//! `tools/list`, `tools/call`, `ping`). We don't need a full JSON-RPC SDK
//! for that — these `serde` types cover the request/response shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC request frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request id. Number, string, or null. Notifications use `null` and
    /// expect no response, but we treat them uniformly and discard.
    pub id: Value,
    /// Method name (e.g. `"initialize"`, `"tools/call"`).
    pub method: String,
    /// Method parameters. May be omitted (None) for parameterless methods.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC response frame. Either `result` or `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Echoes the request id.
    pub id: Value,
    /// Success payload. `None` when `error` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error payload. `None` when `result` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Successful response.
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Error response.
    pub fn error(id: Value, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code: code as i32,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Numeric error code per the JSON-RPC spec or MCP extensions.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Standard JSON-RPC 2.0 error codes (plus MCP extensions).
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub enum ErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
}
