//! Host bridge — connects rift-mcp to a running Rift host over the bus IPC
//! socket.
//!
//! Phase A architecture (D-014 §11):
//!
//! 1. [`HostBridge::connect`] opens an [`IpcClient`] to the Rift host, sends
//!    a `Category::Mcp / mcp.handshake` envelope with the configured token,
//!    and awaits the host's `mcp.handshake.ack` (or `mcp.handshake.deny`).
//! 2. [`HostBridge::call`] sends a `Category::Mcp / mcp.request.{tool}`
//!    envelope with a fresh `request_id` (uuid v4) and reads frames until
//!    a matching `mcp.response.{tool}` arrives, returning its `result` or
//!    `error` payload.
//!
//! The IPC client is owned behind a [`Mutex`] so concurrent `tools/call`
//! invocations serialise on the wire. Streaming tools (`bus_tail`, deferred
//! to Phase A.1) will need a router task that demuxes responses by
//! `request_id` into per-call channels — out of scope for Phase A.

use std::env;
use std::time::Duration;

use rift_bus::{Category, Envelope, IpcClient, IpcError};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::timeout;
use uuid::Uuid;

/// `$RIFT_SOCKET_NAME` — same env var rift-cli reads.
const SOCKET_ENV_VAR: &str = "RIFT_SOCKET_NAME";

/// Display path for the discovery file, formatted for the
/// [`BridgeError::NoSocketName`] message. Resolved lazily so the format
/// string can stay `const`.
fn discovery_path_display() -> String {
    rift_bus::mcp_socket_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<config dir unavailable>".to_string())
}

/// Default per-call timeout if none provided.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors raised by the host bridge.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// No socket name was provided, `$RIFT_SOCKET_NAME` was unset, AND no
    /// discovery file exists — so no Rift host appears to be running.
    #[error(
        "no Rift host found. Start Rift and enable MCP in Settings, or pass --socket <name>, \
         or set ${SOCKET_ENV_VAR}, or write a socket name to {0}"
    )]
    NoSocketName(String),
    /// IPC transport error from the bus client.
    #[error("ipc: {0}")]
    Ipc(#[from] IpcError),
    /// Host denied the handshake (token mismatch, missing on disk, …).
    #[error("handshake denied: {0}")]
    HandshakeDenied(String),
    /// The host returned an error payload for a tool call.
    #[error("tool '{tool}' failed: {message}")]
    Tool {
        /// Tool name as published.
        tool: String,
        /// Host-side error message from `mcp.response.{tool}.error`.
        message: String,
    },
    /// No matching response arrived before the timeout.
    #[error("timed out waiting for mcp.response.{tool}")]
    Timeout {
        /// Tool name we were waiting on.
        tool: String,
    },
    /// Response payload was structurally invalid.
    #[error("malformed response: {0}")]
    Malformed(String),
}

/// Owns the [`IpcClient`] connection to the running Rift host. Built once at
/// startup, shared across `tools/call` invocations.
pub struct HostBridge {
    client: Mutex<IpcClient>,
    /// Per-call timeout. Configurable later if needed.
    call_timeout: Duration,
}

impl HostBridge {
    /// Connect to the Rift host and complete the MCP handshake.
    ///
    /// Socket name resolution order:
    /// 1. Explicit `socket_name` argument (from `--socket` CLI flag).
    /// 2. `$RIFT_SOCKET_NAME` env var (same one rift-cli reads).
    /// 3. The on-disk discovery file written by the running Rift host
    ///    ([`rift_bus::load_mcp_socket`]) — the path Claude Code's MCP
    ///    spawn uses when no env or args are plumbed through.
    ///
    /// Errors with [`BridgeError::NoSocketName`] when all three are absent.
    pub async fn connect(socket_name: Option<String>, token: &str) -> Result<Self, BridgeError> {
        let name = socket_name
            .or_else(|| env::var(SOCKET_ENV_VAR).ok())
            .or_else(|| rift_bus::load_mcp_socket().ok().flatten())
            .ok_or_else(|| BridgeError::NoSocketName(discovery_path_display()))?;
        let mut client = IpcClient::connect(&name).await?;

        // Handshake: send mcp.handshake with token, await ack/deny.
        let request_id = Uuid::new_v4().to_string();
        let handshake = Envelope::new(Category::Mcp, "mcp.handshake")
            .with_payload(&json!({ "request_id": request_id, "token": token }))
            .map_err(|e| BridgeError::Malformed(format!("handshake build: {e}")))?;
        client.send(&handshake).await?;

        // Drain frames until we see ack/deny matching our request_id, or the
        // connection closes. The replay snapshot may include unrelated
        // envelopes — skip until we see ours.
        loop {
            let env = timeout(DEFAULT_CALL_TIMEOUT, client.recv())
                .await
                .map_err(|_| BridgeError::Timeout {
                    tool: "handshake".into(),
                })??;
            if env.category != Category::Mcp {
                continue;
            }
            let id_match = env
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s == request_id);
            if !id_match {
                continue;
            }
            match env.kind.as_str() {
                "mcp.handshake.ack" => break,
                "mcp.handshake.deny" => {
                    let reason = env
                        .payload
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unspecified")
                        .to_owned();
                    return Err(BridgeError::HandshakeDenied(reason));
                }
                _ => continue,
            }
        }

        Ok(Self {
            client: Mutex::new(client),
            call_timeout: DEFAULT_CALL_TIMEOUT,
        })
    }

    /// Run a tool call. Sends `mcp.request.{tool}` and returns the host's
    /// `result` payload (`Ok(Value)`) or surfaces its error message
    /// (`Err(BridgeError::Tool)`).
    pub async fn call(&self, tool: &str, arguments: &Value) -> Result<Value, BridgeError> {
        let request_id = Uuid::new_v4().to_string();

        // Payload merges {request_id, ...arguments}. The host pulls
        // request_id off the top; per-tool fields live alongside.
        let mut payload = json!({ "request_id": request_id });
        if let Some(obj) = arguments.as_object() {
            let map = payload.as_object_mut().expect("just built");
            for (k, v) in obj {
                if k == "request_id" {
                    continue; // never let callers override
                }
                map.insert(k.clone(), v.clone());
            }
        }

        let request_kind = format!("mcp.request.{tool}");
        let response_kind = format!("mcp.response.{tool}");
        let env = Envelope::new(Category::Mcp, request_kind)
            .with_payload(&payload)
            .map_err(|e| BridgeError::Malformed(format!("request build: {e}")))?;

        let mut client = self.client.lock().await;
        client.send(&env).await?;

        let response = timeout(self.call_timeout, async {
            loop {
                let env = client.recv().await?;
                if env.category != Category::Mcp || env.kind != response_kind {
                    continue;
                }
                let id_match = env
                    .payload
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == request_id);
                if id_match {
                    return Ok::<Envelope, IpcError>(env);
                }
            }
        })
        .await
        .map_err(|_| BridgeError::Timeout {
            tool: tool.to_owned(),
        })??;
        drop(client);

        // Host wraps the result as { request_id, ok: bool, result|error }.
        let ok = response
            .payload
            .get("ok")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| BridgeError::Malformed("missing 'ok' field".into()))?;
        if ok {
            response
                .payload
                .get("result")
                .cloned()
                .ok_or_else(|| BridgeError::Malformed("ok=true but no 'result'".into()))
        } else {
            let message = response
                .payload
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown host error")
                .to_owned();
            Err(BridgeError::Tool {
                tool: tool.to_owned(),
                message,
            })
        }
    }
}

// Integration tests live alongside the host (`src-tauri/tests/`) — the
// bridge needs a real `IpcServer` + `spawn_mcp_host` + on-disk `mcp_token`
// to exercise the full wire shape. Adding those is tracked in C-021.
