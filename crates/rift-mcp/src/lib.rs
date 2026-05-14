//! `rift-mcp` library — JSON-RPC dispatch + IPC bridge.
//!
//! Translator-shaped per §9: stdio JSON-RPC outside, `Category::Mcp`
//! bus envelopes inside. The boundary check exempts this crate the same
//! way it exempts `crates/rift-bus/src/translators/`.

#![deny(missing_docs)]

use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::{watch, Mutex};

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

/// JSON-RPC method name used for `bus_tail` streaming notifications
/// (D-014 Phase A.1, locked 2026-04-29). Custom Rift namespace —
/// clients dispatch on this exact method to feed their tail consumer.
pub const BUS_TAIL_NOTIFY_METHOD: &str = "notifications/rift/bus_tail";

/// JSON-RPC method name used to surface a router-side `Lagged` event —
/// emitted when the notification broadcast channel overruns the consumer.
/// Clients should treat receipt as "you missed envelopes; consider
/// reissuing bus_history to resync."
pub const BUS_TAIL_LAGGED_METHOD: &str = "notifications/rift/bus_tail.lagged";

/// Connect to the host and spawn a notification forwarder task.
///
/// Returns the bridge wrapped in an `Arc`. The notification forwarder
/// dies automatically when the bridge (and its broadcast sender) is
/// dropped — no explicit cancellation needed.
///
/// The `ready` gate holds the forwarder from writing to stdout until the
/// MCP `initialized` handshake has completed. Premature writes crash
/// Claude Code's message parser.
async fn connect_and_forward(
    cfg: &McpServerConfig,
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
    ready: watch::Receiver<bool>,
) -> Result<Arc<HostBridge>, BridgeError> {
    let bridge = Arc::new(HostBridge::connect(cfg.socket_name.clone(), &cfg.token).await?);

    let notify_rx = bridge.subscribe_notifications();
    let stdout_for_notify = stdout.clone();
    tokio::spawn(async move {
        run_notification_forwarder(notify_rx, stdout_for_notify, ready).await;
    });

    Ok(bridge)
}

/// Run the stdio JSON-RPC loop.
///
/// Reads newline-delimited JSON-RPC requests on stdin, dispatches to a
/// handler, writes responses to stdout. Long-running tools (e.g.
/// `bus_tail`) emit notifications via a parallel task that listens on
/// the host bridge's notification channel.
///
/// Connects to the Rift host and completes the MCP handshake before the
/// first request is read. Handshake failures bubble up to `main`.
///
/// Auto-reconnects when the host disconnects (e.g. Rift restarts). The
/// MCP server stays alive on stdin/stdout throughout — only tool calls
/// that arrive mid-reconnect receive a transient error.
pub async fn run_stdio(cfg: McpServerConfig) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    // Stdout is shared between the request/response loop and the
    // notification-forwarder task. Mutex serialises frames so partial
    // writes can't interleave on the wire.
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));

    // Ready gate: the notification forwarder holds until the MCP
    // initialized handshake completes. Writing to stdout before the
    // client is ready crashes Claude Code's message parser.
    let (ready_tx, ready_rx) = watch::channel(false);

    let mut bridge = connect_and_forward(&cfg, &stdout, ready_rx.clone())
        .await
        .context("connect to Rift host")?;

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    while let Some(line) = reader.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                // Cannot determine the request id from unparseable JSON.
                // Responding with id:null crashes Claude Code's parser,
                // so log to stderr and drop instead.
                eprintln!("rift-mcp: parse error (dropped): {e}");
                continue;
            }
        };

        // JSON-RPC 2.0: notifications have no id (absent → null via
        // serde default). Servers MUST NOT respond to notifications.
        let is_notification = req.id.is_null();

        // Open the notification-forwarder gate once the client confirms
        // it is ready for server-initiated messages.
        if req.method == "notifications/initialized" || req.method == "initialized" {
            let _ = ready_tx.send(true);
        }

        if is_notification {
            continue;
        }

        let resp = dispatch(&bridge, req.clone()).await;

        // Reconnect on disconnect/pipe errors — retry the call once.
        if needs_reconnect(&resp) {
            eprintln!("rift-mcp: host disconnected, reconnecting...");
            match try_reconnect(&cfg, &stdout, ready_rx.clone()).await {
                Some(new_bridge) => {
                    bridge = new_bridge;
                    eprintln!("rift-mcp: reconnected");
                    let retry = dispatch(&bridge, req).await;
                    write_response(&stdout, &retry).await?;
                    continue;
                }
                None => {
                    eprintln!("rift-mcp: reconnect failed — returning error to client");
                }
            }
        }

        write_response(&stdout, &resp).await?;
    }
    Ok(())
}

/// Check whether a response indicates the host bridge is broken.
fn needs_reconnect(resp: &Response) -> bool {
    resp.error.as_ref().is_some_and(|e| {
        e.code == ErrorCode::InternalError as i32
            && (e.message.contains("disconnected")
                || e.message.contains("pipe")
                || e.message.contains("ipc:"))
    })
}

/// Attempt to reconnect with exponential backoff (5 attempts, 1s base,
/// ~31s total window). Re-reads the discovery file each attempt since the
/// socket name changes on Rift restart.
async fn try_reconnect(
    cfg: &McpServerConfig,
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
    ready: watch::Receiver<bool>,
) -> Option<Arc<HostBridge>> {
    for attempt in 0..5u32 {
        let delay = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
        tokio::time::sleep(delay).await;
        match connect_and_forward(cfg, stdout, ready.clone()).await {
            Ok(b) => return Some(b),
            Err(e) => {
                eprintln!("rift-mcp: reconnect attempt {} failed: {e}", attempt + 1);
            }
        }
    }
    None
}

/// Serialize and write a JSON-RPC frame (response OR notification) to
/// stdout under the shared mutex. Each frame is newline-terminated and
/// the stream is flushed before returning.
async fn write_frame(stdout: &Mutex<tokio::io::Stdout>, frame: &Value) -> Result<()> {
    let s = serde_json::to_string(frame)?;
    let mut guard = stdout.lock().await;
    guard.write_all(s.as_bytes()).await?;
    guard.write_all(b"\n").await?;
    guard.flush().await?;
    Ok(())
}

async fn write_response(stdout: &Mutex<tokio::io::Stdout>, resp: &Response) -> Result<()> {
    let frame = serde_json::to_value(resp)?;
    write_frame(stdout, &frame).await
}

/// Notification-forwarder task body. Listens on the bridge's broadcast
/// receiver and converts each `mcp.notify.{tool}` envelope into a
/// JSON-RPC notification on stdout.
///
/// Dispatch by tool kind:
///   - `mcp.notify.bus_tail` → method [`BUS_TAIL_NOTIFY_METHOD`]
///   - other kinds: dropped (no client consumer in Phase A.1)
///
/// Lag handling: a [`broadcast::error::RecvError::Lagged`] surfaces a
/// dedicated [`BUS_TAIL_LAGGED_METHOD`] notification carrying the lag
/// count. The client knows envelopes were dropped and can decide whether
/// to resync via `bus_history`. Recv errors other than `Lagged` (closed
/// channel) terminate the task — happens only when the bridge is
/// dropped, which means the process is shutting down anyway.
async fn run_notification_forwarder(
    mut rx: tokio::sync::broadcast::Receiver<rift_bus::Envelope>,
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    mut ready: watch::Receiver<bool>,
) {
    // Wait for the MCP initialized handshake before writing any
    // notifications. Premature writes crash Claude Code's parser.
    while !*ready.borrow_and_update() {
        if ready.changed().await.is_err() {
            return;
        }
    }
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match rx.recv().await {
            Ok(env) => {
                let kind = env.kind.as_str();
                let method = if kind == "mcp.notify.bus_tail" {
                    BUS_TAIL_NOTIFY_METHOD
                } else {
                    // Unknown notify kind — no client consumer wired yet.
                    continue;
                };
                let request_id = env
                    .payload
                    .get("request_id")
                    .cloned()
                    .unwrap_or(Value::Null);
                let inner = env.payload.get("envelope").cloned().unwrap_or(Value::Null);
                let frame = json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": {
                        "request_id": request_id,
                        "envelope": inner,
                    },
                });
                if write_frame(&stdout, &frame).await.is_err() {
                    // stdout closed — host is shutting us down; just exit.
                    return;
                }
            }
            Err(RecvError::Lagged(n)) => {
                let frame = json!({
                    "jsonrpc": "2.0",
                    "method": BUS_TAIL_LAGGED_METHOD,
                    "params": { "skipped": n },
                });
                if write_frame(&stdout, &frame).await.is_err() {
                    return;
                }
            }
            Err(RecvError::Closed) => return,
        }
    }
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

    // bus_tail returns immediately with {stream_started: true} after the
    // host has spawned its subscribe-publish task. Subsequent envelopes
    // arrive via the notification forwarder, NOT through this response.
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
        for expected in [
            // Phase A
            "bus_history",
            "bus_tail",
            "git_status",
            "aegis_state",
            // Phase B — Tier 1 read tools
            "fs_read",
            "fs_tree",
            "todo_scan",
            "pty_list",
            "cockpit_state",
            "notif_tabs",
            // Phase C — Tier 2 inspection tools
            "dom_snapshot",
            "screenshot",
            "js_eval",
            // Phase D — Tier 3 mutating + read tools
            "pty_input",
            "pty_read",
            "bus_publish",
            "fs_write",
            "git_action",
            "simulate_click",
            "simulate_drag",
        ] {
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
