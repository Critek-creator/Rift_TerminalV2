//! `rift-mcp` library — JSON-RPC dispatch + IPC bridge.
//!
//! Translator-shaped per §9: stdio JSON-RPC outside, `Category::Mcp`
//! bus envelopes inside. The boundary check exempts this crate the same
//! way it exempts `crates/rift-bus/src/translators/`.

#![deny(missing_docs)]

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub mod host_bridge;
pub mod jsonrpc;
pub mod tools;

use host_bridge::{BridgeError, HostBridge};
use jsonrpc::{ErrorCode, Request, Response};
use tools::{tool_catalog, ToolCall};

/// MCP server configuration passed to [`run_stdio`].
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Override IPC socket name. `None` = platform default.
    pub socket_name: Option<String>,
    /// Token sent in the `Category::Mcp / mcp.handshake` envelope.
    pub token: String,
}

/// Server name reported in `initialize` responses (D-014 §11 q6).
pub const SERVER_NAME: &str = "Rift";

/// Server version reported in `initialize` responses.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// MCP protocol version we speak (locked at the v1.0 spec revision).
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Run the stdio JSON-RPC loop.
///
/// Reads newline-delimited JSON-RPC requests on stdin, dispatches to a
/// handler, writes responses to stdout. Long-running tools (e.g.
/// `bus_tail`) emit notifications.
///
/// Connects to the Rift host and completes the MCP handshake before the
/// first request is read. Handshake failures bubble up to `main`.
pub async fn run_stdio(cfg: McpServerConfig) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let bridge = HostBridge::connect(cfg.socket_name.clone(), &cfg.token)
        .await
        .context("connect to Rift host")?;

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    while let Some(line) = reader.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::error(
                    Value::Null,
                    ErrorCode::ParseError,
                    format!("invalid json: {e}"),
                );
                write_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        let resp = dispatch(&bridge, req).await;
        write_response(&mut stdout, &resp).await?;
    }
    Ok(())
}

async fn write_response<W>(stdout: &mut W, resp: &Response) -> Result<()>
where
    W: tokio::io::AsyncWriteExt + Unpin,
{
    let s = serde_json::to_string(resp)?;
    stdout.write_all(s.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

/// Dispatch a single JSON-RPC request. `tools/call` routes through the
/// host bridge; everything else is answered locally.
async fn dispatch(bridge: &HostBridge, req: Request) -> Response {
    let id = req.id.clone();
    match dispatch_protocol(req, id.clone()) {
        Ok(resp) => resp,
        Err(call_req) => {
            handle_tools_call(bridge, id, call_req.params.unwrap_or(Value::Null)).await
        }
    }
}

/// Pure routing for protocol-level methods. Returns `Err(req)` when the
/// request requires the host bridge (currently only `tools/call`).
fn dispatch_protocol(req: Request, id: Value) -> Result<Response, Request> {
    match req.method.as_str() {
        "initialize" => Ok(handle_initialize(id)),
        "ping" => Ok(Response::ok(id, json!({}))),
        "tools/list" => Ok(handle_tools_list(id)),
        // MCP defines "notifications/initialized" as a no-response
        // notification; we tolerate it silently.
        "notifications/initialized" => Ok(Response::ok(id, Value::Null)),
        "tools/call" => Err(req),
        other => Ok(Response::error(
            id,
            ErrorCode::MethodNotFound,
            format!("method not found: {other}"),
        )),
    }
}

fn handle_initialize(id: Value) -> Response {
    let result = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION,
        },
        "capabilities": {
            "tools": { "listChanged": false },
        },
    });
    Response::ok(id, result)
}

fn handle_tools_list(id: Value) -> Response {
    let tools: Vec<Value> = tool_catalog()
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema.clone(),
            })
        })
        .collect();
    Response::ok(id, json!({ "tools": tools }))
}

async fn handle_tools_call(bridge: &HostBridge, id: Value, params: Value) -> Response {
    let call: ToolCall = match serde_json::from_value(params) {
        Ok(c) => c,
        Err(e) => {
            return Response::error(
                id,
                ErrorCode::InvalidParams,
                format!("invalid tools/call params: {e}"),
            );
        }
    };

    // bus_tail streaming is deferred to Phase A.1 — host returns the same
    // hint, but we shortcut here so we don't make a wire round-trip just
    // to surface it.
    if call.name == "bus_tail" {
        return Response::ok(
            id,
            tool_result_text(
                "bus_tail streaming not yet wired in Phase A — \
                 use bus_history for one-shot replay.",
                true,
            ),
        );
    }

    match bridge.call(&call.name, &call.arguments).await {
        Ok(result) => Response::ok(id, tool_result_json(result)),
        Err(BridgeError::Tool { tool, message }) => Response::ok(
            id,
            tool_result_text(format!("tool '{tool}' failed: {message}"), true),
        ),
        Err(e) => Response::error(id, ErrorCode::InternalError, e.to_string()),
    }
}

/// Wrap a host result `Value` as an MCP `tools/call` response body.
///
/// MCP requires `content` (an array of typed parts). We encode structured
/// payloads as a single `text` part containing the JSON serialisation, and
/// also surface the structured form via `structuredContent` for clients
/// that prefer to consume it directly.
fn tool_result_json(result: Value) -> Value {
    let text = serde_json::to_string(&result).unwrap_or_else(|_| "null".into());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": result,
        "isError": false,
    })
}

/// Wrap an arbitrary message as an MCP `tools/call` response body.
fn tool_result_text(text: impl Into<String>, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text.into() }],
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(method: &str, id: i64) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            id: json!(id),
            method: method.into(),
            params: None,
        }
    }

    #[test]
    fn initialize_returns_locked_protocol_version() {
        let req = make_req("initialize", 1);
        let resp = dispatch_protocol(req, json!(1)).expect("local response");
        let result = resp.result.expect("ok response");
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
    }

    #[test]
    fn tools_list_contains_phase_a_catalog() {
        let req = make_req("tools/list", 2);
        let resp = dispatch_protocol(req, json!(2)).expect("local response");
        let result = resp.result.expect("ok response");
        let names: Vec<&str> = result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in ["bus_history", "bus_tail", "git_status", "aegis_state"] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let req = make_req("no_such_method", 3);
        let resp = dispatch_protocol(req, json!(3)).expect("local response");
        let err = resp.error.expect("error response");
        assert_eq!(err.code, ErrorCode::MethodNotFound as i32);
    }

    #[test]
    fn tools_call_routes_to_bridge() {
        let req = make_req("tools/call", 4);
        let routed = dispatch_protocol(req, json!(4));
        assert!(
            routed.is_err(),
            "tools/call must escape to the bridge, not be answered locally"
        );
    }
}
