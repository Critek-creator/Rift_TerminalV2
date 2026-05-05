//! Errors translator — publishes `Category::System` / `kind="error"` envelopes.
//!
//! This is the second translator in the Rift Integration Protocol.
//! All Tauri command `Err` paths route through [`publish`] before returning
//! to the frontend, giving the Errors notification tab live error feed.
//!
//! ## Payload shape
//!
//! ```json
//! {
//!   "source":  "tauri.command.pty_start",
//!   "message": "<human-readable error string>",
//!   "context": <Value | null>
//! }
//! ```
//!
//! `kind` is always `"error"` — the only System kind emitted in v1.
//! Adding further kinds under `Category::System` is additive and does NOT
//! bump `CURRENT_VERSION` (per `envelope-version-additive-categories-no-bump`).

use serde_json::{json, Value};

use crate::{Category, Envelope, RiftBus};

/// Publish a `Category::System / kind="error"` envelope.
///
/// Fire-and-forget: if the bus publish itself fails (bus closed, zero
/// capacity) the error is logged to stderr and the function returns normally.
/// Callers are already in an error path and must not be disrupted by a
/// secondary bus failure.
///
/// # Arguments
///
/// * `bus`     — the shared [`RiftBus`] instance.
/// * `source`  — structured source identifier, e.g. `"tauri.command.pty_start"`.
/// * `message` — human-readable error string.
/// * `context` — optional structured context; `None` → `null` in payload.
pub fn publish(bus: &RiftBus, source: &str, message: impl AsRef<str>, context: Option<Value>) {
    let message = message.as_ref().to_string();

    let payload = json!({
        "source":  source,
        "message": message,
        "context": context.unwrap_or(Value::Null),
    });
    let mut env = Envelope::new(Category::System, "error");
    env.payload = payload;
    bus.publish(env);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RiftBus, SubscribeFilter};
    use tokio::time::{timeout, Duration};

    /// RT7a — publish emits Category::System / kind="error" with correct payload.
    #[test]
    fn publish_shape_no_context() {
        let bus = RiftBus::default();
        publish(&bus, "tauri.command.pty_start", "spawn failed", None);

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1, "expected exactly one envelope");
        let env = &snapshot[0];
        assert_eq!(env.category, Category::System);
        assert_eq!(env.kind, "error");
        assert_eq!(env.payload["source"], "tauri.command.pty_start");
        assert_eq!(env.payload["message"], "spawn failed");
        assert!(
            env.payload["context"].is_null(),
            "context should be null when None"
        );
    }

    /// RT7a (context variant) — structured context is preserved in the payload.
    #[test]
    fn publish_shape_with_context() {
        let bus = RiftBus::default();
        let ctx = json!({ "session_id": 42, "rows": 24 });
        publish(&bus, "tauri.command.pty_resize", "resize failed", Some(ctx));

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1);
        let env = &snapshot[0];
        assert_eq!(env.payload["source"], "tauri.command.pty_resize");
        assert_eq!(env.payload["message"], "resize failed");
        assert_eq!(env.payload["context"]["session_id"], 42);
        assert_eq!(env.payload["context"]["rows"], 24);
    }

    /// RT7b — envelope is observable via subscribe after publish.
    #[tokio::test]
    async fn subscribe_roundtrip() {
        let bus = RiftBus::default();
        let (_snapshot, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::System));

        publish(&bus, "tauri.command.pty_kill", "kill failed", None);

        let received = timeout(Duration::from_secs(1), sub.recv())
            .await
            .expect("recv within 1s")
            .expect("ok");

        assert_eq!(received.category, Category::System);
        assert_eq!(received.kind, "error");
        assert_eq!(received.payload["source"], "tauri.command.pty_kill");
    }

    /// Version must not be bumped (additive kind under existing Category).
    #[test]
    fn version_not_bumped() {
        let bus = RiftBus::default();
        publish(&bus, "test", "msg", None);
        let (snapshot, _) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].version, crate::CURRENT_VERSION);
    }
}
