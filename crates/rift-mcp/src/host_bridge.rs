//! Host bridge — connects rift-mcp to a running Rift host over the bus IPC
//! socket.
//!
//! Architecture (D-014 Phase A → A.1):
//!
//! 1. [`HostBridge::connect`] opens an [`IpcClient`] to the Rift host, sends
//!    a `Category::Mcp / mcp.handshake` envelope with the configured token,
//!    and awaits the host's `mcp.handshake.ack` (or `mcp.handshake.deny`).
//! 2. After ack the connection splits into read + write halves. A detached
//!    **router task** owns the reader and demuxes incoming envelopes:
//!    * `mcp.response.{tool}` → look up the matching `request_id` in the
//!      pending map and fulfill its oneshot.
//!    * `mcp.notify.{tool}`   → broadcast on the notification channel for
//!      streaming-tool subscribers (`bus_tail` etc.).
//!
//!    Everything else is dropped.
//! 3. [`HostBridge::call`] registers a oneshot in the pending map, sends a
//!    `mcp.request.{tool}` envelope through the writer mutex, and awaits
//!    the oneshot. Concurrent calls multiplex on the wire — the router
//!    task is the demux point, not a per-call recv loop.
//! 4. [`HostBridge::subscribe_notifications`] returns a broadcast receiver
//!    that emits every `mcp.notify.*` envelope. Streaming-tool callers
//!    filter by `request_id` to isolate their own stream.
//!
//! When the host disconnects, the router task's `recv()` returns `Err` and
//! the task exits. Pending oneshots are dropped, surfacing `RecvError` to
//! waiting callers — they translate it to [`BridgeError::Disconnected`].

use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;

use rift_bus::{Category, Envelope, IpcClient, IpcError, IpcReader, IpcWriter};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::{broadcast, oneshot, Mutex};
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
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Notification broadcast channel capacity. Bursty bus traffic can push past
/// this — the router task surfaces a `Lagged` notification (see
/// [`Self::subscribe_notifications`]) so consumers know they missed events.
const NOTIFY_CHANNEL_CAPACITY: usize = 1024;

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
    /// Router task exited (host disconnected, IPC closed) before the call's
    /// response arrived.
    #[error("host disconnected while waiting for mcp.response.{tool}")]
    Disconnected {
        /// Tool name we were waiting on.
        tool: String,
    },
}

/// Pending-call map: request_id → oneshot sender. The router task drains
/// completed entries; `call()` inserts entries before sending and removes
/// them on timeout.
type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Envelope>>>>;

/// Router-task state. Shared between the spawned reader task and
/// [`HostBridge`] via `Arc`s — the bridge just holds clones.
pub struct HostBridge {
    writer: Mutex<IpcWriter>,
    pending: PendingMap,
    notify_tx: broadcast::Sender<Envelope>,
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

        // Handshake: send mcp.handshake with token, await ack/deny BEFORE
        // splitting. We do this on the unsplit client because it's a
        // strict request/response pair — no router needed yet.
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

        // Handshake confirmed — split into halves and start the router.
        let (reader, writer) = client.split();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (notify_tx, _notify_rx) = broadcast::channel(NOTIFY_CHANNEL_CAPACITY);

        let pending_for_router = pending.clone();
        let notify_tx_for_router = notify_tx.clone();
        tokio::spawn(async move {
            run_router(reader, pending_for_router, notify_tx_for_router).await;
        });

        Ok(Self {
            writer: Mutex::new(writer),
            pending,
            notify_tx,
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
            let map = payload
                .as_object_mut()
                .ok_or_else(|| BridgeError::Malformed("payload is not an object".into()))?;
            for (k, v) in obj {
                if k == "request_id" {
                    continue; // never let callers override
                }
                map.insert(k.clone(), v.clone());
            }
        }

        let request_kind = format!("mcp.request.{tool}");
        let env = Envelope::new(Category::Mcp, request_kind)
            .with_payload(&payload)
            .map_err(|e| BridgeError::Malformed(format!("request build: {e}")))?;

        // Register the oneshot BEFORE sending so a fast host that responds
        // before the await can't race ahead of the registration.
        let (resp_tx, resp_rx) = oneshot::channel::<Envelope>();
        self.pending
            .lock()
            .await
            .insert(request_id.clone(), resp_tx);

        // Send under the writer lock; drop the lock immediately so other
        // calls can interleave. The router task handles recv concurrently.
        if let Err(e) = self.writer.lock().await.send(&env).await {
            // Send failed — clean up the pending entry so it doesn't leak.
            self.pending.lock().await.remove(&request_id);
            return Err(BridgeError::Ipc(e));
        }

        let response = match timeout(self.call_timeout, resp_rx).await {
            Ok(Ok(env)) => env,
            Ok(Err(_recv_err)) => {
                // Sender dropped — router task exited mid-flight.
                self.pending.lock().await.remove(&request_id);
                return Err(BridgeError::Disconnected {
                    tool: tool.to_owned(),
                });
            }
            Err(_elapsed) => {
                self.pending.lock().await.remove(&request_id);
                return Err(BridgeError::Timeout {
                    tool: tool.to_owned(),
                });
            }
        };

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

    /// Subscribe to all `mcp.notify.*` envelopes flowing in from the host.
    ///
    /// Streaming-tool consumers (`bus_tail`) call this once and filter the
    /// resulting [`broadcast::Receiver`] by `request_id` — multiple
    /// concurrent streams share the channel.
    ///
    /// The receiver is bounded ([`NOTIFY_CHANNEL_CAPACITY`]); slow
    /// consumers that lag past the capacity will receive
    /// [`broadcast::error::RecvError::Lagged`] and should treat it as a
    /// missed-events sentinel.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<Envelope> {
        self.notify_tx.subscribe()
    }
}

/// Router-task body. Demuxes incoming envelopes onto the pending oneshot
/// map (responses) and the notify broadcast channel (notifications). Exits
/// silently when [`IpcReader::recv`] returns `Err` (host disconnect).
async fn run_router(
    mut reader: IpcReader,
    pending: PendingMap,
    notify_tx: broadcast::Sender<Envelope>,
) {
    loop {
        let env = match reader.recv().await {
            Ok(e) => e,
            Err(_) => break,
        };

        if env.category != Category::Mcp {
            continue;
        }

        if env.kind.starts_with("mcp.response.") {
            let request_id = match env
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
            {
                Some(id) => id,
                None => continue,
            };
            // Pop the matching oneshot. If absent (already timed out, or
            // never registered), drop the response silently.
            if let Some(tx) = pending.lock().await.remove(&request_id) {
                // If the receiver was dropped, send returns Err — also fine.
                let _ = tx.send(env);
            }
            continue;
        }

        if env.kind.starts_with("mcp.notify.") {
            // Non-blocking send. If no subscribers exist yet, send returns
            // Err and the envelope is dropped — exactly the right behavior
            // (nobody is listening).
            let _ = notify_tx.send(env);
            continue;
        }

        // Other Category::Mcp envelopes (e.g. audit `mcp.invoke`) are not
        // routed back to the client — they live on the host bus.
    }

    // Reader exited — leaving the pending map populated would leak.
    // Dropping the senders surfaces RecvError to waiting callers, which
    // translate to BridgeError::Disconnected.
    pending.lock().await.clear();
}

// Integration tests live alongside the host (`src-tauri/tests/`) — the
// bridge needs a real `IpcServer` + `spawn_mcp_host` + on-disk `mcp_token`
// to exercise the full wire shape. See C-021 for the Phase A test set.
