//! `rift-mcp` library — JSON-RPC dispatch + IPC bridge.
//!
//! Translator-shaped per §9: stdio JSON-RPC outside, `Category::Mcp`
//! bus envelopes inside. The boundary check exempts this crate the same
//! way it exempts `crates/rift-bus/src/translators/`.

#![deny(missing_docs)]

use std::sync::Arc;

use anyhow::Result;
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

/// How long a cached bridge stays alive without a tool call before being
/// dropped. Short enough that stale pipes are detected quickly; long
/// enough that a burst of tool calls reuses the same connection.
const BRIDGE_IDLE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

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

/// Acquire a bridge — reuse cached if alive, otherwise connect fresh.
///
/// This is the core of the connect-per-request architecture. Every tool
/// call flows through here instead of holding a permanent bridge. The
/// cache avoids repeated handshakes during bursts (e.g., rapid tool calls).
/// When Rift restarts, the next tool call finds the stale bridge's
/// `call()` erroring and `get_or_connect` replaces it transparently.
async fn get_or_connect(
    cached: &Mutex<Option<Arc<HostBridge>>>,
    cfg: &McpServerConfig,
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
    ready: watch::Receiver<bool>,
) -> Result<Arc<HostBridge>, BridgeError> {
    {
        let guard = cached.lock().await;
        if let Some(ref b) = *guard {
            return Ok(b.clone());
        }
    }
    let bridge = connect_and_forward(cfg, stdout, ready).await?;
    *cached.lock().await = Some(bridge.clone());
    Ok(bridge)
}

/// Run the stdio JSON-RPC loop.
///
/// Connect-per-request architecture: each `tools/call` acquires a bridge
/// via `get_or_connect` (cached for 5 seconds). When the bridge errors
/// (Rift restarted, pipe died), the cache is cleared and the next call
/// gets a fresh connection. No reconnect loops, no exponential backoff.
///
/// Protocol methods (`initialize`, `ping`, `tools/list`) are answered
/// locally without touching the bridge.
pub async fn run_stdio(cfg: McpServerConfig) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let (ready_tx, ready_rx) = watch::channel(false);

    // Cached bridge — populated on first tool call, cleared on error,
    // expired after BRIDGE_IDLE_TTL of inactivity.
    let cached_bridge: Arc<Mutex<Option<Arc<HostBridge>>>> = Arc::new(Mutex::new(None));

    // Idle timer: drop the cached bridge after BRIDGE_IDLE_TTL so stale
    // pipes from a stopped Rift are detected on the next call.
    let idle_cached = cached_bridge.clone();
    let _idle_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(BRIDGE_IDLE_TTL).await;
            let mut guard = idle_cached.lock().await;
            if guard.is_some() {
                *guard = None;
            }
        }
    });

    // Initial connection — fail fast if Rift isn't running so the user
    // sees a clear error at session start instead of on first tool call.
    match connect_and_forward(&cfg, &stdout, ready_rx.clone()).await {
        Ok(b) => {
            *cached_bridge.lock().await = Some(b);
        }
        Err(e) => {
            eprintln!("rift-mcp: initial connect failed (will retry on first tool call): {e}");
        }
    }

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
                eprintln!("rift-mcp: parse error (dropped): {e}");
                continue;
            }
        };

        let is_notification = req.id.is_null();

        if req.method == "notifications/initialized" || req.method == "initialized" {
            let _ = ready_tx.send(true);
        }

        if is_notification {
            continue;
        }

        let resp = dispatch_with_retry(&cached_bridge, &cfg, &stdout, ready_rx.clone(), req).await;

        write_response(&stdout, &resp).await?;
    }
    Ok(())
}

/// Dispatch a request. On bridge error, clear cache and retry ONCE with a
/// fresh connection. No backoff — if Rift is alive, the retry connects in
/// <100ms. If Rift is dead, the retry fails immediately.
async fn dispatch_with_retry(
    cached: &Mutex<Option<Arc<HostBridge>>>,
    cfg: &McpServerConfig,
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
    ready: watch::Receiver<bool>,
    req: Request,
) -> Response {
    let id = req.id.clone();

    // Protocol methods don't need the bridge.
    match dispatch_protocol(req.clone(), id.clone()) {
        Ok(resp) => return resp,
        Err(_call_req) => {}
    }

    // First attempt — use cached bridge or connect fresh.
    let bridge = match get_or_connect(cached, cfg, stdout, ready.clone()).await {
        Ok(b) => b,
        Err(e) => {
            return Response::error(id, ErrorCode::InternalError, format!("connect failed: {e}"));
        }
    };

    let resp = handle_tools_call(
        &bridge,
        id.clone(),
        req.params.clone().unwrap_or(Value::Null),
    )
    .await;

    if !needs_reconnect(&resp) {
        return resp;
    }

    // Bridge is broken — clear cache and retry once.
    eprintln!("rift-mcp: bridge error, reconnecting...");
    *cached.lock().await = None;

    let bridge = match get_or_connect(cached, cfg, stdout, ready).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("rift-mcp: reconnect failed: {e}");
            return Response::error(
                id,
                ErrorCode::InternalError,
                format!("reconnect failed: {e}"),
            );
        }
    };

    eprintln!("rift-mcp: reconnected");
    handle_tools_call(&bridge, id, req.params.unwrap_or(Value::Null)).await
}

/// Check whether a response indicates the host bridge is broken.
fn needs_reconnect(resp: &Response) -> bool {
    resp.error.as_ref().is_some_and(|e| {
        e.code == ErrorCode::InternalError as i32
            && (e.message.contains("disconnected")
                || e.message.contains("pipe")
                || e.message.contains("ipc:")
                || e.message.contains("timed out"))
    })
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

/// Pure routing for protocol-level methods. Returns `Err(req)` when the
/// request requires the host bridge (currently only `tools/call`).
fn dispatch_protocol(req: Request, id: Value) -> Result<Response, Request> {
    match req.method.as_str() {
        "initialize" => Ok(handle_initialize(id)),
        "ping" => Ok(Response::ok(id, json!({}))),
        "tools/list" => Ok(handle_tools_list(id)),
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
        "instructions": SERVER_INSTRUCTIONS,
    });
    Response::ok(id, result)
}

/// Instructions sent to the LLM in the `initialize` response.
/// Tells Claude what Rift is and when to use its tools proactively.
const SERVER_INSTRUCTIONS: &str = "\
You are running inside Rift Terminal — a standalone terminal + GUI cockpit that \
observes your activity in real time. All terminal I/O flows through a typed event \
bus; the cockpit surfaces categorized notification tabs (errors, hooks, commands, \
agents, filesystem, MCP, sentinel, git, sessions).\n\
\n\
USE PROACTIVELY:\n\
- `screenshot` — after UI/visual changes to verify rendering, or when the user \
  asks what something looks like. Captures the live Rift window.\n\
- `rift_diagnose` — when debugging terminal issues, checking health, or starting \
  a diagnostic session. Returns PTY state, bus stats, errors, config.\n\
- `bus_history` — to understand recent activity, correlate events, or debug \
  unexpected behavior. Filter by category (pty, hook, agent, fs, index, aegis, \
  status, system, mcp).\n\
- `todo_scan` — before wrapping up work, to catch leftover markers.\n\
- `notif_tabs` — to see unread counts and which notification categories are active.\n\
- `fs_tree` — for project structure overview (honors ignore globs).\n\
- `git_status` — current branch, staged/unstaged changes, recent commits.\n\
- `llm_prompt` — OFFLOAD grunt subtasks (summarize, reformat, log/diff \
  digestion, boilerplate scaffolding, docstrings, extraction) to a LOCAL model \
  instead of spending your own tokens generating rote output. Routes via the \
  Ensemble Router (local-first on auto profiles). Best for bulky, low-judgment \
  work; skip one-liners where the round-trip costs more than doing it yourself.\n\
\n\
The terminal uses lane classification — output lines are color-coded by source \
(user input, Claude, agents, hooks, aegis, system, errors). Your output appears \
in the blue (Claude) lane.\n\
\n\
When you use Rift tools, the activity appears in the MCP notification tab. \
The user can see your tool usage in real time via the cockpit.\
";

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
            "bus_history",
            "bus_tail",
            "git_status",
            "aegis_state",
            "fs_read",
            "fs_tree",
            "todo_scan",
            "pty_list",
            "cockpit_state",
            "notif_tabs",
            "dom_snapshot",
            "screenshot",
            "js_eval",
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
