//! Enrichment translator — generic §9 capability-class-3 (data enrichment).
//!
//! This is the class-3 analogue of the control-endpoint **action registry**
//! (capability class 2): where actions let any integration *declare invocable
//! actions* Rift renders, enrichment lets any integration *attach metadata to
//! filesystem nodes* Rift surfaces (the dot + tooltip on `Tree.svelte` nodes).
//!
//! Before this module, class 3 was hard-wired to the Abyssal Index: only the
//! vault-walker emitted `Category::Index / kind="enrichment"`. A git-blame
//! translator, an Aegis translator, or any other integration had no way to
//! enrich a path. This generalizes the path so **any** provider can.
//!
//! ## Wire protocol (mirrors the action registry)
//!
//! All three kinds ride [`Category::System`] (additive kinds — no envelope
//! version bump, exactly like `action.declare`/`invoke`/`result`):
//!
//! | kind                  | direction          | meaning                              |
//! |-----------------------|--------------------|--------------------------------------|
//! | `enrichment.declare`  | provider → Rift    | "I am an enrichment provider"        |
//! | `enrichment.attach`   | provider → Rift    | attach metadata to one `fs_path`     |
//! | `enrichment.revoke`   | provider → Rift    | remove a provider's enrichment       |
//!
//! ## Identity + conflict resolution
//!
//! Each attach carries a `provider_id` (the integration namespace, e.g.
//! `"index"`, `"git"`) and an `entry_id` (unique within a given
//! `(provider_id, fs_path)`). The frontend store dedups on
//! `(provider_id, entry_id)` at a path: **last-write-wins per slot**, and two
//! different providers (or two entries) at the same path coexist as separate
//! tooltip rows. There is no merge — each provider owns its own slot. This is
//! the documented v1 conflict-resolution model.
//!
//! The Abyssal Index dogfoods this protocol: the vault-walker now publishes
//! `provider_id="index"`, `entry_id=<vault_id>` so each vault remains a
//! distinct row exactly as before, while the wire path is now generic.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Category, Envelope, RiftBus};

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// Payload carried by a `Category::System / kind="enrichment.declare"` envelope.
///
/// Announces that an integration provides enrichment — the class-3 analogue of
/// `action.declare`. The frontend registry lists declared providers (e.g. for
/// the Integration Capability Inspector); the load-bearing data arrives via
/// [`EnrichmentAttachPayload`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnrichmentDeclarePayload {
    /// Stable integration namespace, e.g. `"index"`, `"git"`.
    pub provider_id: String,
    /// Human-readable provider label for the inspector UI.
    pub label: String,
    /// Optional longer description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Payload carried by a `Category::System / kind="enrichment.attach"` envelope.
///
/// `fs_path` is the canonical absolute filesystem path being enriched,
/// forward-slash-normalized (no trailing slash). `EnrichmentStore.get(node.path)`
/// joins on string equality, so the path must match what `Tree.svelte` holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnrichmentAttachPayload {
    /// Integration namespace, e.g. `"index"`, `"git"`.
    pub provider_id: String,
    /// Unique within `(provider_id, fs_path)` — the dedup slot key.
    pub entry_id: String,
    /// Canonical absolute filesystem path being enriched, forward-slash-normalized.
    pub fs_path: String,
    /// Display label for the tooltip row (falls back to `entry_id` if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Provider-sourced tags attached to the node.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Provider-specific opaque bag (e.g. Index puts `vault_id`/`vault_kind` here).
    #[serde(default)]
    pub data: Value,
}

// ---------------------------------------------------------------------------
// Publish functions
// ---------------------------------------------------------------------------

/// Publish a `Category::System / kind="enrichment.declare"` envelope.
///
/// Fire-and-forget (mirrors the index translator publish convention).
pub fn publish_enrichment_declare(
    bus: &RiftBus,
    provider_id: &str,
    label: &str,
    description: Option<&str>,
) {
    let payload = json!({
        "provider_id": provider_id,
        "label": label,
        "description": description,
    });
    let mut env = Envelope::new(Category::System, "enrichment.declare");
    env.payload = payload;
    bus.publish(env);
}

/// Publish a `Category::System / kind="enrichment.attach"` envelope.
///
/// `fs_path` is normalized to forward slashes before publishing.
pub fn publish_enrichment_attach(
    bus: &RiftBus,
    provider_id: &str,
    entry_id: &str,
    fs_path: &Path,
    label: Option<&str>,
    tags: Vec<String>,
    data: Value,
) {
    let fs_path_str = fs_path.to_string_lossy().replace('\\', "/");
    let payload = json!({
        "provider_id": provider_id,
        "entry_id": entry_id,
        "fs_path": fs_path_str,
        "label": label,
        "tags": tags,
        "data": data,
    });
    let mut env = Envelope::new(Category::System, "enrichment.attach");
    env.payload = payload;
    bus.publish(env);
}

/// Publish a `Category::System / kind="enrichment.revoke"` envelope.
///
/// When `fs_path` is `None`, the provider's enrichment is removed from **all**
/// paths (e.g. on integration shutdown). When `Some`, only that path's slot is
/// removed.
pub fn publish_enrichment_revoke(bus: &RiftBus, provider_id: &str, fs_path: Option<&Path>) {
    let fs_path_str = fs_path.map(|p| p.to_string_lossy().replace('\\', "/"));
    let payload = json!({
        "provider_id": provider_id,
        "fs_path": fs_path_str,
    });
    let mut env = Envelope::new(Category::System, "enrichment.revoke");
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
    use std::path::Path;

    #[test]
    fn declare_envelope_shape() {
        let bus = RiftBus::default();
        publish_enrichment_declare(&bus, "index", "Abyssal Index", Some("vault metadata"));
        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1);
        let env = &snapshot[0];
        assert_eq!(env.category, Category::System);
        assert_eq!(env.kind, "enrichment.declare");
        assert_eq!(env.payload["provider_id"], "index");
        assert_eq!(env.payload["label"], "Abyssal Index");
        assert_eq!(env.payload["description"], "vault metadata");
    }

    #[test]
    fn attach_envelope_shape() {
        let bus = RiftBus::default();
        publish_enrichment_attach(
            &bus,
            "index",
            "p006",
            Path::new("C:/proj"),
            Some("p006"),
            vec!["project".into()],
            json!({ "vault_id": "p006", "vault_kind": "project" }),
        );
        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1);
        let env = &snapshot[0];
        assert_eq!(env.category, Category::System);
        assert_eq!(env.kind, "enrichment.attach");
        assert_eq!(env.payload["provider_id"], "index");
        assert_eq!(env.payload["entry_id"], "p006");
        assert_eq!(env.payload["fs_path"], "C:/proj");
        assert_eq!(env.payload["label"], "p006");
        assert_eq!(env.payload["tags"][0], "project");
        assert_eq!(env.payload["data"]["vault_kind"], "project");
    }

    #[test]
    fn attach_normalizes_backslashes() {
        let bus = RiftBus::default();
        publish_enrichment_attach(
            &bus,
            "git",
            "blame",
            Path::new("C:\\proj\\src"),
            None,
            vec![],
            Value::Null,
        );
        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].payload["fs_path"], "C:/proj/src");
    }

    #[test]
    fn revoke_all_paths_has_null_fs_path() {
        let bus = RiftBus::default();
        publish_enrichment_revoke(&bus, "index", None);
        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        let env = &snapshot[0];
        assert_eq!(env.category, Category::System);
        assert_eq!(env.kind, "enrichment.revoke");
        assert_eq!(env.payload["provider_id"], "index");
        assert!(env.payload["fs_path"].is_null());
    }

    #[test]
    fn attach_payload_roundtrip() {
        let original = EnrichmentAttachPayload {
            provider_id: "index".into(),
            entry_id: "r004".into(),
            fs_path: "C:/proj".into(),
            label: Some("r004".into()),
            tags: vec!["research".into()],
            data: json!({ "vault_id": "r004" }),
        };
        let s = serde_json::to_string(&original).expect("serialize");
        let back: EnrichmentAttachPayload = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back.provider_id, original.provider_id);
        assert_eq!(back.entry_id, original.entry_id);
        assert_eq!(back.fs_path, original.fs_path);
        assert_eq!(back.tags, original.tags);
    }

    #[test]
    fn version_not_bumped() {
        let bus = RiftBus::default();
        publish_enrichment_declare(&bus, "x", "X", None);
        let (snapshot, _) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].version, crate::CURRENT_VERSION);
    }
}
