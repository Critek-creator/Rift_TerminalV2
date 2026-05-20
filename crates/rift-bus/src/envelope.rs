//! Rift Integration Protocol — envelope schema.
//!
//! Wire format for every event flowing through the bus or across the IPC
//! boundary. Spec: `decisions/§10.15_real-time_update_mechanism.md`.
//!
//! ## Versioning rule
//!
//! `version` only bumps on **schema breaks**. Adding a new [`Category`]
//! variant or a new `kind` value within an existing category is **additive**
//! and does NOT bump the version. (V1 lesson:
//! `envelope-version-additive-categories-no-bump`.)

use serde::{Deserialize, Serialize};

/// Current envelope schema version. Bump only on schema breaks.
pub const CURRENT_VERSION: u16 = 1;

/// Coarse-grained event source. Translators publish to a single category;
/// subscribers filter by one or many.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    /// PTY byte streams + lifecycle (start, exit, resize).
    Pty,
    /// Claude Code hook events.
    Hook,
    /// Sub-agent activity (dispatch, complete, error).
    Agent,
    /// Filesystem watcher events (read, write, create, delete).
    Fs,
    /// Abyssal Index vault events (vault.update, enrichment).
    Index,
    /// Aegis private translator events (private, optional integration).
    Aegis,
    /// Status-line snapshots (ctx %, session use, model, …).
    Status,
    /// System-level signals (errors, warnings, lifecycle).
    System,
    /// MCP server traffic — `mcp.handshake`, `mcp.invoke` (audit), and
    /// `mcp.request.*`/`mcp.response.*` (tool round-trip). Off the wire
    /// entirely until `RiftConfig.mcp.enabled = true`.
    /// Spec: `decisions/D-014_rift_mcp_v1_plan.md`.
    Mcp,
    /// Sentinel watchdog events — rule violations, detection status,
    /// health checks. Integration-provided (D-010, post-v1).
    Sentinel,
}

/// The wire envelope. Every event on the bus and across the IPC boundary
/// is one of these.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Envelope {
    /// Schema version. Always [`CURRENT_VERSION`] for newly-built envelopes.
    pub version: u16,
    /// Source category.
    pub category: Category,
    /// Type within the category (e.g. `"pty.output"`, `"hook.pre_edit"`).
    /// Adding new kinds is additive — does NOT bump [`CURRENT_VERSION`].
    pub kind: String,
    /// Unix epoch milliseconds.
    pub ts: u64,
    /// Free-form payload. Use [`Envelope::with_payload`] to populate.
    pub payload: serde_json::Value,
    /// Root correlation identifier. All events in a causal chain share this.
    /// Optional, additive — does NOT bump [`CURRENT_VERSION`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Direct parent envelope identifier (the event that caused this one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

impl Envelope {
    /// Create a fresh envelope with `version`, `ts`, and a `Null` payload.
    pub fn new(category: Category, kind: impl Into<String>) -> Self {
        Self {
            version: CURRENT_VERSION,
            category,
            kind: kind.into(),
            ts: now_unix_ms(),
            payload: serde_json::Value::Null,
            correlation_id: None,
            parent_id: None,
        }
    }

    /// Attach a typed payload by serialising it via serde_json.
    pub fn with_payload<T: Serialize>(mut self, payload: &T) -> Result<Self, serde_json::Error> {
        self.payload = serde_json::to_value(payload)?;
        Ok(self)
    }

    /// Set correlation fields for causal chain linking.
    pub fn with_correlation(
        mut self,
        correlation_id: impl Into<String>,
        parent_id: Option<String>,
    ) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self.parent_id = parent_id;
        self
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_no_payload() {
        let env = Envelope::new(Category::Hook, "pre_edit");
        let json = serde_json::to_string(&env).expect("encode");
        let back: Envelope = serde_json::from_str(&json).expect("decode");
        assert_eq!(env, back);
    }

    #[test]
    fn round_trip_with_payload() {
        #[derive(Serialize)]
        struct Payload {
            file: &'static str,
            ok: bool,
        }
        let env = Envelope::new(Category::Hook, "post_edit")
            .with_payload(&Payload {
                file: "pty.rs",
                ok: true,
            })
            .expect("payload");
        let json = serde_json::to_string(&env).expect("encode");
        let back: Envelope = serde_json::from_str(&json).expect("decode");
        assert_eq!(env, back);
        assert_eq!(back.payload["file"], "pty.rs");
        assert_eq!(back.payload["ok"], true);
    }

    /// Per pr003 lesson `serialize-deserialize-asymmetry-bidirectional-protocol`:
    /// every category MUST round-trip. If a future variant is added, this
    /// test catches a serde-rename/lowercase regression.
    #[test]
    fn every_category_round_trips() {
        for c in [
            Category::Pty,
            Category::Hook,
            Category::Agent,
            Category::Fs,
            Category::Index,
            Category::Aegis,
            Category::Status,
            Category::System,
            Category::Mcp,
            Category::Sentinel,
        ] {
            let env = Envelope::new(c, "smoke");
            let json = serde_json::to_string(&env).expect("encode");
            let back: Envelope = serde_json::from_str(&json).expect("decode");
            assert_eq!(env.category, back.category, "round-trip {c:?}");
        }
    }

    #[test]
    fn category_serialises_lowercase() {
        let env = Envelope::new(Category::Aegis, "loaded");
        let json = serde_json::to_string(&env).expect("encode");
        assert!(
            json.contains(r#""category":"aegis""#),
            "expected lowercase category, got {json}"
        );
    }

    #[test]
    fn current_version_stamped() {
        let env = Envelope::new(Category::System, "boot");
        assert_eq!(env.version, CURRENT_VERSION);
    }

    #[test]
    fn round_trip_with_correlation() {
        let env = Envelope::new(Category::Fs, "create")
            .with_correlation("corr-abc-123", Some("parent-xyz".to_string()));
        let json = serde_json::to_string(&env).expect("encode");
        assert!(json.contains("corr-abc-123"));
        assert!(json.contains("parent-xyz"));
        let back: Envelope = serde_json::from_str(&json).expect("decode");
        assert_eq!(back.correlation_id, Some("corr-abc-123".to_string()));
        assert_eq!(back.parent_id, Some("parent-xyz".to_string()));
    }

    #[test]
    fn correlation_fields_absent_by_default() {
        let env = Envelope::new(Category::Hook, "test");
        let json = serde_json::to_string(&env).expect("encode");
        assert!(!json.contains("correlation_id"));
        assert!(!json.contains("parent_id"));
    }

    #[test]
    fn deserialize_without_correlation_fields() {
        let json = r#"{"version":1,"category":"hook","kind":"test","ts":0,"payload":null}"#;
        let env: Envelope = serde_json::from_str(json).expect("decode");
        assert_eq!(env.correlation_id, None);
        assert_eq!(env.parent_id, None);
    }
}
