//! MCP host-side subscriber (D-014 Phase A).
//!
//! Listens on `Category::Mcp / kind="mcp.request.*"` envelopes published by
//! the `rift-mcp` translator (which speaks JSON-RPC over stdio externally).
//! Each request is answered with a matching `mcp.response.*` envelope on
//! the same bus, keyed by `request_id` in the payload.
//!
//! Audit: every accepted invocation publishes `mcp.invoke` BEFORE running,
//! so denied or panicking calls are still surfaced in the audit trail.
//!
//! Off-by-default: `RiftConfig.mcp.enabled = false` short-circuits
//! [`spawn_mcp_host`] — the subscriber never starts and no `Category::Mcp`
//! traffic flows.
//!
//! Spec: `decisions/D-014_rift_mcp_v1_plan.md` (locked v1.1 — 2026-04-29).

use std::path::{Path, PathBuf};

use rift_bus::{publish_error, Category, Envelope, McpConfig, RiftBus, SubscribeFilter};
use serde_json::{json, Value};

use crate::git_status;

/// Spawn the MCP host subscriber on the rift-bus runtime.
///
/// No-op when `cfg.enabled = false`. Otherwise:
///
/// 1. Confirms the on-disk token exists (generates one if not).
/// 2. Subscribes to `Category::Mcp`.
/// 3. Routes each `mcp.handshake` to a token-verification reply, and each
///    `mcp.request.*` to its tool handler — publishing `mcp.invoke` audit
///    envelopes BEFORE running.
///
/// `project_root` is captured for tools that need it (e.g. `git_status`).
/// `socket_name` is the live `rift-bus` IPC socket name — written to the
/// MCP discovery file so the standalone `rift-mcp` binary can find this
/// host without `--socket` or `$RIFT_SOCKET_NAME` plumbed through.
pub fn spawn_mcp_host(bus: RiftBus, cfg: McpConfig, project_root: PathBuf, socket_name: String) {
    if !cfg.enabled {
        tracing::debug!("mcp_host: MCP disabled in config — subscriber not spawned");
        return;
    }

    // Ensure the token file exists. Errors are non-fatal but logged — the
    // host still starts, but every handshake will fail closed (the token
    // file is the source of truth used by the rift-mcp client).
    let token_present = match rift_bus::ensure_mcp_token() {
        Ok(_) => true,
        Err(e) => {
            tracing::error!("mcp_host: failed to ensure token: {e}");
            publish_error(&bus, "mcp_host.token", e.to_string(), None);
            false
        }
    };

    // Publish the live socket name so the standalone `rift-mcp` binary —
    // which Claude Code spawns with no env or args — can discover this
    // host. Cleared in the `RunEvent::ExitRequested` handler in lib.rs.
    if let Err(e) = rift_bus::save_mcp_socket(&socket_name) {
        tracing::error!("mcp_host: failed to write discovery file: {e}");
        publish_error(&bus, "mcp_host.discovery", e.to_string(), None);
    }

    let bus_for_task = bus.clone();
    tauri::async_runtime::spawn(async move {
        run_subscriber(bus_for_task, project_root, token_present).await;
    });
}

async fn run_subscriber(bus: RiftBus, project_root: PathBuf, token_present: bool) {
    let (snapshot, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::Mcp));

    // Replay drain — process any envelopes already in the buffer (rare but
    // possible if the host enables MCP mid-session and there are queued
    // requests; protocol clients typically wait for handshake first).
    for env in snapshot {
        handle_envelope(&bus, &project_root, token_present, &env);
    }

    loop {
        match sub.recv().await {
            Ok(env) => handle_envelope(&bus, &project_root, token_present, &env),
            Err(rift_bus::BusError::Lagged(n)) => {
                tracing::warn!("mcp_host: subscriber lagged by {n} envelopes");
            }
            Err(rift_bus::BusError::Closed) => break,
        }
    }
}

fn handle_envelope(bus: &RiftBus, project_root: &Path, token_present: bool, env: &Envelope) {
    // Ignore our own outbound envelopes (responses, audit, errors). The
    // subscriber sees everything in Category::Mcp, including what we publish.
    let kind = env.kind.as_str();
    if kind == "mcp.invoke"
        || kind.starts_with("mcp.response.")
        || kind == "mcp.handshake.ack"
        || kind == "mcp.handshake.deny"
    {
        return;
    }

    if kind == "mcp.handshake" {
        handle_handshake(bus, env, token_present);
        return;
    }

    if let Some(tool_name) = kind.strip_prefix("mcp.request.") {
        // Audit BEFORE running — denied or panicking calls still log.
        let request_id = env
            .payload
            .get("request_id")
            .cloned()
            .unwrap_or(Value::Null);
        publish_audit(
            bus,
            "mcp.invoke",
            json!({
                "tool": tool_name,
                "request_id": request_id.clone(),
                "ts": env.ts,
            }),
        );

        // Streaming tools (D-014 Phase A.1): spawn a subscribe-publish task
        // and return a start-ack synchronously. Subsequent envelopes flow
        // back as `mcp.notify.{tool}` rather than a single `mcp.response`.
        if tool_name == "bus_tail" {
            let result = start_bus_tail(bus.clone(), request_id.clone(), &env.payload);
            publish_response(bus, tool_name, request_id, result);
            return;
        }

        let result = dispatch_tool(bus, project_root, tool_name, &env.payload);
        publish_response(bus, tool_name, request_id, result);
    }
}

fn handle_handshake(bus: &RiftBus, env: &Envelope, token_present: bool) {
    let provided = env
        .payload
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let request_id = env
        .payload
        .get("request_id")
        .cloned()
        .unwrap_or(Value::Null);

    if !token_present {
        publish_audit(
            bus,
            "mcp.handshake.deny",
            json!({ "request_id": request_id, "reason": "no token on disk" }),
        );
        return;
    }

    let on_disk = match rift_bus::load_mcp_token() {
        Ok(Some(t)) => t,
        _ => {
            publish_audit(
                bus,
                "mcp.handshake.deny",
                json!({ "request_id": request_id, "reason": "token unreadable" }),
            );
            return;
        }
    };

    if constant_time_eq(provided.as_bytes(), on_disk.as_bytes()) {
        publish_audit(
            bus,
            "mcp.handshake.ack",
            json!({ "request_id": request_id }),
        );
    } else {
        publish_audit(
            bus,
            "mcp.handshake.deny",
            json!({ "request_id": request_id, "reason": "token mismatch" }),
        );
    }
}

fn dispatch_tool(
    bus: &RiftBus,
    project_root: &Path,
    tool: &str,
    payload: &Value,
) -> Result<Value, String> {
    match tool {
        "bus_history" => tool_bus_history(bus, payload),
        "git_status" => tool_git_status(project_root),
        "aegis_state" => tool_aegis_state(bus),
        // bus_tail is intercepted before dispatch_tool — see handle_envelope.
        "bus_tail" => Err("bus_tail must route through start_bus_tail".into()),
        other => Err(format!("unknown MCP tool: {other}")),
    }
}

/// Phase A.1 — start a streaming `bus_tail` subscription.
///
/// Parses optional `category` + `kind_prefix` filter args, spawns a task
/// that subscribes to the bus and publishes one `mcp.notify.bus_tail`
/// envelope per matching event, and returns a start-ack `Value` for the
/// initial `mcp.response.bus_tail`.
///
/// Per locked v1.0 design, cancellation is implicit — when the stdio
/// client (`rift-mcp`) disconnects, the bus subscription stays live but
/// notifications are dropped on the floor by the broadcast channel. The
/// task itself exits when the bus closes (process shutdown) or when the
/// subscribe channel returns `Closed`.
fn start_bus_tail(bus: RiftBus, request_id: Value, payload: &Value) -> Result<Value, String> {
    // Parse filter args. Both optional; absent = match all.
    let category_filter = payload
        .get("category")
        .and_then(|v| v.as_str())
        .map(str::to_lowercase);
    let kind_prefix = payload
        .get("kind_prefix")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let filter = match category_filter.as_deref() {
        None => SubscribeFilter::All,
        Some(c) => match serde_json::from_value::<Category>(Value::String(c.to_owned())) {
            Ok(cat) => SubscribeFilter::Category(cat),
            Err(_) => return Err(format!("invalid category: {c}")),
        },
    };

    let bus_for_task = bus.clone();
    let request_id_for_task = request_id.clone();
    let kind_prefix_for_task = kind_prefix.clone();
    tauri::async_runtime::spawn(async move {
        run_bus_tail(
            bus_for_task,
            request_id_for_task,
            kind_prefix_for_task,
            filter,
        )
        .await;
    });

    Ok(json!({
        "stream_started": true,
        "request_id": request_id,
        "filter": {
            "category": category_filter,
            "kind_prefix": kind_prefix,
        },
    }))
}

/// Body of the streaming subscriber spawned by [`start_bus_tail`].
async fn run_bus_tail(
    bus: RiftBus,
    request_id: Value,
    kind_prefix: Option<String>,
    filter: SubscribeFilter,
) {
    // Skip the snapshot replay — `bus_tail` is "events from now on" by
    // design; clients use `bus_history` if they want backfill.
    let (_snapshot, mut sub) = bus.subscribe(filter);

    loop {
        match sub.recv().await {
            Ok(env) => {
                if let Some(prefix) = kind_prefix.as_deref() {
                    if !env.kind.starts_with(prefix) {
                        continue;
                    }
                }
                publish_notify(&bus, "bus_tail", &request_id, &env);
            }
            Err(rift_bus::BusError::Lagged(n)) => {
                tracing::warn!("mcp_host: bus_tail subscriber lagged by {n} envelopes");
                // Surface the lag to the client as a notification with no
                // envelope so the stream can resync if it cares.
                publish_notify_raw(
                    &bus,
                    "bus_tail",
                    json!({
                        "request_id": request_id,
                        "lagged": n,
                    }),
                );
            }
            Err(rift_bus::BusError::Closed) => break,
        }
    }
}

fn tool_bus_history(bus: &RiftBus, payload: &Value) -> Result<Value, String> {
    let category_filter = payload
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());
    let limit = payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(1000) as usize;

    let filter = match category_filter {
        None => SubscribeFilter::All,
        Some(c) => match serde_json::from_value::<Category>(Value::String(c.clone())) {
            Ok(cat) => SubscribeFilter::Category(cat),
            Err(_) => return Err(format!("invalid category: {c}")),
        },
    };

    let (snapshot, _sub) = bus.subscribe(filter);
    let take_from = snapshot.len().saturating_sub(limit);
    let envelopes: Vec<Value> = snapshot
        .into_iter()
        .skip(take_from)
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    let count = envelopes.len();
    Ok(json!({ "envelopes": envelopes, "count": count }))
}

fn tool_git_status(project_root: &Path) -> Result<Value, String> {
    let snap = git_status::snapshot(project_root)?;
    serde_json::to_value(snap).map_err(|e| e.to_string())
}

fn tool_aegis_state(bus: &RiftBus) -> Result<Value, String> {
    let (snapshot, _sub) = bus.subscribe(SubscribeFilter::Category(Category::Aegis));
    let last = snapshot
        .into_iter()
        .rev()
        .find(|e| e.kind == "aegis.session.skill_loaded");
    match last {
        Some(env) => Ok(env.payload),
        None => Ok(Value::Null),
    }
}

fn publish_response(bus: &RiftBus, tool: &str, request_id: Value, result: Result<Value, String>) {
    let kind = format!("mcp.response.{tool}");
    let payload = match result {
        Ok(v) => json!({ "request_id": request_id, "ok": true, "result": v }),
        Err(e) => json!({ "request_id": request_id, "ok": false, "error": e }),
    };
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

fn publish_audit(bus: &RiftBus, kind: &str, payload: Value) {
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

/// Publish a streaming-tool notification — `mcp.notify.{tool}` envelope
/// carrying `{ request_id, envelope }`. The rift-mcp translator's
/// notification forwarder converts these to JSON-RPC notifications on
/// stdout (method `notifications/rift/{tool}`).
fn publish_notify(bus: &RiftBus, tool: &str, request_id: &Value, envelope: &Envelope) {
    let kind = format!("mcp.notify.{tool}");
    let inner = serde_json::to_value(envelope).unwrap_or(Value::Null);
    let payload = json!({
        "request_id": request_id.clone(),
        "envelope": inner,
    });
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

/// Lower-level notify helper — emit a `mcp.notify.{tool}` with a custom
/// payload (e.g. lag sentinel). Use [`publish_notify`] for the standard
/// envelope-carrying form.
fn publish_notify_raw(bus: &RiftBus, tool: &str, payload: Value) {
    let kind = format!("mcp.notify.{tool}");
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

/// Constant-time byte comparison to defeat timing-based token leakage.
/// Returns false on length mismatch without scanning further.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// (Audit publishes use `publish_audit` directly — no extension trait needed.)
