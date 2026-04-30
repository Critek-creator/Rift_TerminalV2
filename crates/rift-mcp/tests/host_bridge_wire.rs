//! Integration test — `HostBridge` wire protocol.
//!
//! Spins up a real `RiftBus` + `IpcServer` and a mock host task that
//! mirrors the handshake + dispatch surface of `src-tauri/src/mcp_host.rs`,
//! then drives the bridge through every documented path:
//!
//! * `handshake_succeeds_with_correct_token`
//! * `handshake_fails_with_wrong_token`
//! * `tool_round_trip_returns_host_result`
//! * `host_error_propagates_as_bridge_tool_error`
//! * `unknown_tool_returns_bridge_tool_error`
//! * `timeout_when_host_silent`
//!
//! End-to-end coverage against the *real* host (`spawn_mcp_host` +
//! on-disk `mcp_token` round-trip) is tracked under C-021's remaining
//! v1.x work — that test belongs in `src-tauri/tests/` because
//! `mcp_host` is a private module of the Tauri shell.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rift_bus::{BusError, Category, Envelope, IpcServer, RiftBus, SubscribeFilter};
use rift_mcp::host_bridge::{BridgeError, HostBridge};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

const TEST_TOKEN: &str = "phase-a-wire-test-token";

/// Stand up the same setup as [`setup`] but also returns a clone of the
/// internal `RiftBus` so the streaming test can publish `mcp.notify.*`
/// envelopes directly. Used only by the Phase A.1 streaming tests.
async fn setup_with_bus(
    accept_token: &'static str,
    client_token: &str,
) -> Result<(HostBridge, RiftBus), BridgeError> {
    let bus = RiftBus::default();
    let name = unique_name();
    let _server = IpcServer::start(bus.clone(), &name)
        .await
        .expect("ipc server start");
    sleep(Duration::from_millis(50)).await;
    spawn_mock_host(bus.clone(), accept_token);

    let bridge = HostBridge::connect(Some(name), client_token).await?;
    Ok((bridge, bus))
}

fn unique_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("rift-mcp-wire-{pid}-{id}.sock")
}

/// Spawn a mock host that answers `mcp.handshake` and `mcp.request.*`
/// with the same envelope shape `mcp_host.rs` publishes. Returns once
/// the bus closes.
fn spawn_mock_host(bus: RiftBus, accept_token: &'static str) {
    tokio::spawn(async move {
        let (snapshot, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::Mcp));
        for env in snapshot {
            handle_envelope(&bus, accept_token, &env);
        }
        loop {
            match sub.recv().await {
                Ok(env) => handle_envelope(&bus, accept_token, &env),
                Err(BusError::Closed) => break,
                Err(BusError::Lagged(_)) => continue,
            }
        }
    });
}

fn handle_envelope(bus: &RiftBus, accept_token: &str, env: &Envelope) {
    let kind = env.kind.as_str();
    // Ignore our own outbound envelopes — same gate as mcp_host.rs.
    if kind == "mcp.invoke"
        || kind.starts_with("mcp.response.")
        || kind == "mcp.handshake.ack"
        || kind == "mcp.handshake.deny"
    {
        return;
    }

    let request_id = env
        .payload
        .get("request_id")
        .cloned()
        .unwrap_or(Value::Null);

    if kind == "mcp.handshake" {
        let provided = env
            .payload
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (reply_kind, payload) = if provided == accept_token {
            ("mcp.handshake.ack", json!({ "request_id": request_id }))
        } else {
            (
                "mcp.handshake.deny",
                json!({ "request_id": request_id, "reason": "token mismatch" }),
            )
        };
        publish(bus, reply_kind, &payload);
        return;
    }

    if let Some(tool) = kind.strip_prefix("mcp.request.") {
        let result: Result<Value, String> = match tool {
            // Phase A
            "bus_history" => Ok(json!({ "envelopes": [], "count": 0 })),
            "git_status" => Ok(json!({ "branch": "main", "dirty": false })),
            "aegis_state" => Ok(json!({ "skill": "aegis", "version": "test" })),
            // Phase B — Tier 1 read tools (D-014 §3)
            "fs_read" => Ok(json!({ "path": "src/lib.rs", "content": "fn main() {}" })),
            "fs_tree" => Ok(json!({
                "name": "root",
                "isDir": true,
                "children": [],
            })),
            "todo_scan" => Ok(json!({ "entries": [], "count": 0 })),
            "pty_list" => Ok(json!({ "sessions": [], "count": 0 })),
            "cockpit_state" => Ok(json!({ "detached": false })),
            "notif_tabs" => Ok(json!({ "tabs": [] })),
            "fail_me" => Err("simulated host failure".into()),
            "silent" => return, // never replies — used for timeout test
            other => Err(format!("unknown MCP tool: {other}")),
        };
        let payload = match result {
            Ok(v) => json!({ "request_id": request_id, "ok": true, "result": v }),
            Err(e) => json!({ "request_id": request_id, "ok": false, "error": e }),
        };
        let reply_kind = format!("mcp.response.{tool}");
        publish(bus, &reply_kind, &payload);
    }
}

fn publish(bus: &RiftBus, kind: &str, payload: &Value) {
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(payload) {
        bus.publish(env);
    }
}

/// Stand up RiftBus + IpcServer + mock host. Returns the connected bridge.
async fn setup(accept_token: &'static str, client_token: &str) -> Result<HostBridge, BridgeError> {
    let bus = RiftBus::default();
    let name = unique_name();
    let _server = IpcServer::start(bus.clone(), &name)
        .await
        .expect("ipc server start");
    // Match the 50ms warm-up delay used in rift-bus' own ipc tests.
    sleep(Duration::from_millis(50)).await;
    spawn_mock_host(bus.clone(), accept_token);

    HostBridge::connect(Some(name), client_token).await
}

#[tokio::test]
async fn handshake_succeeds_with_correct_token() {
    let bridge = setup(TEST_TOKEN, TEST_TOKEN).await.expect("connect");
    // If the handshake resolved, the bridge is usable — round-trip a
    // simple call to prove it.
    let result = bridge
        .call("aegis_state", &Value::Null)
        .await
        .expect("aegis_state");
    assert_eq!(result["skill"], "aegis");
}

#[tokio::test]
async fn handshake_fails_with_wrong_token() {
    match setup(TEST_TOKEN, "wrong-token").await {
        Err(BridgeError::HandshakeDenied(reason)) => assert_eq!(reason, "token mismatch"),
        Err(other) => panic!("expected HandshakeDenied, got {other:?}"),
        Ok(_) => panic!("handshake should have denied"),
    }
}

#[tokio::test]
async fn tool_round_trip_returns_host_result() {
    let bridge = setup(TEST_TOKEN, TEST_TOKEN).await.expect("connect");
    let result = bridge
        .call("bus_history", &json!({ "limit": 10 }))
        .await
        .expect("bus_history");
    assert_eq!(result["count"], 0);
    assert!(result["envelopes"].is_array());
}

#[tokio::test]
async fn host_error_propagates_as_bridge_tool_error() {
    let bridge = setup(TEST_TOKEN, TEST_TOKEN).await.expect("connect");
    let err = bridge
        .call("fail_me", &Value::Null)
        .await
        .expect_err("expected tool error");
    match err {
        BridgeError::Tool { tool, message } => {
            assert_eq!(tool, "fail_me");
            assert_eq!(message, "simulated host failure");
        }
        other => panic!("expected BridgeError::Tool, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_tool_returns_bridge_tool_error() {
    let bridge = setup(TEST_TOKEN, TEST_TOKEN).await.expect("connect");
    let err = bridge
        .call("no_such_tool", &Value::Null)
        .await
        .expect_err("expected tool error");
    match err {
        BridgeError::Tool { tool, message } => {
            assert_eq!(tool, "no_such_tool");
            assert!(message.starts_with("unknown MCP tool"), "got: {message}");
        }
        other => panic!("expected BridgeError::Tool, got {other:?}"),
    }
}

/// Phase B — every newly-added Tier 1 read tool round-trips through the
/// bridge and returns the mock host's payload unchanged. One test instead
/// of six because the wire is identical per tool — only the payload shape
/// differs, and the bridge is payload-opaque.
#[tokio::test]
async fn phase_b_tools_round_trip() {
    let bridge = setup(TEST_TOKEN, TEST_TOKEN).await.expect("connect");

    let cases: &[(&str, Value, &str)] = &[
        ("fs_read", json!({ "path": "src/lib.rs" }), "path"),
        ("fs_tree", json!({}), "name"),
        ("todo_scan", json!({}), "count"),
        ("pty_list", json!({}), "count"),
        ("cockpit_state", json!({}), "detached"),
        ("notif_tabs", json!({}), "tabs"),
    ];

    for (tool, args, expected_field) in cases {
        let result = bridge
            .call(tool, args)
            .await
            .unwrap_or_else(|e| panic!("{tool} call failed: {e:?}"));
        assert!(
            result.get(*expected_field).is_some(),
            "{tool} response missing '{expected_field}': {result}"
        );
    }
}

/// Phase A.1 — `subscribe_notifications` surfaces `mcp.notify.*` envelopes
/// emitted by the host. Publishes a synthetic notify envelope and asserts
/// the bridge's broadcast receiver delivers it.
#[tokio::test]
async fn streaming_notification_surfaces_via_bridge() {
    let (bridge, bus) = setup_with_bus(TEST_TOKEN, TEST_TOKEN)
        .await
        .expect("connect");

    // Subscribe BEFORE the host publishes — broadcast channels drop
    // events emitted before any subscriber exists.
    let mut rx = bridge.subscribe_notifications();

    // Pretend a streaming tool task on the host published a notify.
    let payload = json!({
        "request_id": "test-stream-1",
        "envelope": {
            "category": "Aegis",
            "kind": "aegis.session.skill_loaded",
            "ts": 1_700_000_000_000u64,
            "payload": { "skill": "aegis", "version": "test" },
        },
    });
    let env = Envelope::new(Category::Mcp, "mcp.notify.bus_tail")
        .with_payload(&payload)
        .expect("payload build");
    bus.publish(env);

    // Bridge router task should forward it within tens of ms.
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("notification arrived")
        .expect("recv ok");
    assert_eq!(received.kind, "mcp.notify.bus_tail");
    assert_eq!(received.payload["request_id"], "test-stream-1");
    assert_eq!(
        received.payload["envelope"]["kind"],
        "aegis.session.skill_loaded"
    );
}

/// Phase A.1 — multiple notify envelopes arrive in order through a single
/// bridge subscription. Validates the broadcast channel doesn't reorder.
#[tokio::test]
async fn streaming_notifications_preserve_order() {
    let (bridge, bus) = setup_with_bus(TEST_TOKEN, TEST_TOKEN)
        .await
        .expect("connect");

    let mut rx = bridge.subscribe_notifications();

    for i in 0..5u64 {
        let payload = json!({
            "request_id": "test-stream-order",
            "envelope": { "kind": format!("seq.{i}"), "category": "System" },
        });
        let env = Envelope::new(Category::Mcp, "mcp.notify.bus_tail")
            .with_payload(&payload)
            .expect("payload build");
        bus.publish(env);
    }

    for i in 0..5u64 {
        let received = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("notification arrived")
            .expect("recv ok");
        assert_eq!(received.payload["envelope"]["kind"], format!("seq.{i}"));
    }
}

#[tokio::test]
async fn timeout_when_host_silent() {
    let bridge = setup(TEST_TOKEN, TEST_TOKEN).await.expect("connect");
    // Mock host explicitly drops `silent` requests on the floor; bridge
    // default timeout is 5s — wrap in a generous outer timeout so a
    // misbehaving test never hangs CI.
    let outcome = timeout(Duration::from_secs(10), bridge.call("silent", &Value::Null))
        .await
        .expect("outer timeout — bridge should have surfaced its own");
    match outcome {
        Err(BridgeError::Timeout { tool }) => assert_eq!(tool, "silent"),
        other => panic!("expected BridgeError::Timeout, got {other:?}"),
    }
}
