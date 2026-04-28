//! Index translator — publishes `Category::Index` envelopes.
//!
//! This translator is the **sole entry point** for Abyssal Index events into
//! the Rift bus. External integrations (e.g. the Aegis vault-walker added in
//! Phase 8.5) call [`publish_vault_update`] and [`publish_index_enrichment`]
//! directly; no internal watcher is spawned in Phase 8.1.
//!
//! ## Design notes
//!
//! This module mirrors the Phase 6.1 `fs.rs` translator pattern: publish-only
//! API + typed payload structs + serde round-trip tests. The vault-walker
//! source that drives `publish_vault_update` automatically is deferred to
//! Phase 8.5.
//!
//! ### Kind taxonomy
//!
//! Two kinds are emitted under `Category::Index` in v1:
//!
//! | kind             | trigger                                        |
//! |------------------|------------------------------------------------|
//! | `"vault.update"` | a vault file was created, modified, or deleted |
//! | `"enrichment"`   | a filesystem path receives vault metadata      |
//!
//! Adding further kinds is additive and does NOT bump `CURRENT_VERSION`
//! (per `envelope-version-additive-categories-no-bump`).
//!
//! ### Passive observer model
//!
//! The index translator is a passive observer: it does not own a watcher
//! thread and does not maintain any internal state beyond the bus itself.
//! A lightweight cache (mirrors the rift-aegis pattern) may be added in
//! Phase 8.5 when the vault-walker source lands.
//!
//! ## Payload shapes
//!
//! ```json
//! // vault.update
//! {
//!   "vault_id":   "p006",
//!   "path":       "vaults/p006.md",
//!   "change_kind": "modified"
//! }
//!
//! // enrichment
//! {
//!   "fs_path":    "src/translators/index.rs",
//!   "vault_id":   "p006",
//!   "vault_kind": "p",
//!   "tags":       ["phase8", "index"]
//! }
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Category, Envelope, RiftBus};

// ---------------------------------------------------------------------------
// VaultChangeKind
// ---------------------------------------------------------------------------

/// The nature of the change to a vault file.
///
/// Serialized in snake_case so that the JSON payload uses `"created"`,
/// `"modified"`, `"deleted"` — matching the `change_kind` taxonomy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultChangeKind {
    /// A new vault file was created.
    Created,
    /// An existing vault file was modified.
    Modified,
    /// A vault file was deleted.
    Deleted,
}

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// Payload carried by a `Category::Index / kind="vault.update"` envelope.
///
/// `change_kind` is one of `"created"`, `"modified"`, `"deleted"`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultUpdatePayload {
    /// Vault identifier, e.g. `"p006"` or `"pr001"`.
    pub vault_id: String,
    /// Relative path of the vault file, forward-slash normalized.
    pub path: String,
    /// Nature of the change.
    pub change_kind: VaultChangeKind,
}

/// Payload carried by a `Category::Index / kind="enrichment"` envelope.
///
/// `vault_kind` is the Abyssal Index category in long form, derived from the
/// vault id prefix by the walker's `derive_vault_type`:
/// `"project"`, `"practices"`, `"research"`, `"skill"`, `"lore"`, `"agent"`,
/// `"hook"`.
///
/// `fs_path` is the canonical absolute filesystem path being enriched,
/// forward-slash-normalized (no trailing slash). For project-root enrichment
/// (Phase 8.6 v1), this equals the canonicalized project root path
/// `Tree.svelte` holds for its root node — `EnrichmentStore.get(node.path)`
/// joins on string equality.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexEnrichmentPayload {
    /// Canonical absolute filesystem path being enriched, forward-slash-normalized.
    pub fs_path: String,
    /// Vault identifier that provides the enrichment.
    pub vault_id: String,
    /// Abyssal Index category in long form (`"project"`, `"practices"`, etc.).
    pub vault_kind: String,
    /// Vault-sourced tags attached to the filesystem node.
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// publish_vault_update
// ---------------------------------------------------------------------------

/// Publish a `Category::Index / kind="vault.update"` envelope onto the bus.
///
/// Fire-and-forget: if the bus publish itself fails (bus closed, zero
/// capacity) the error is logged via `tracing::warn!` and the function
/// returns normally. Callers are already in an integration callback and
/// must not be disrupted by a secondary bus failure.
///
/// # Arguments
///
/// * `bus`         — the shared [`RiftBus`] instance.
/// * `vault_id`    — identifier of the affected vault (e.g. `"p006"`).
/// * `path`        — path to the vault file; stored as a forward-slash
///   normalized relative string in the payload.
/// * `change_kind` — nature of the change ([`VaultChangeKind`]).
///
/// # Path normalization
///
/// The path is stored as-is (the caller is responsible for ensuring it is
/// relative and forward-slash normalized). This mirrors the fs translator
/// convention where normalization happens at the watcher layer.
pub fn publish_vault_update(
    bus: &RiftBus,
    vault_id: &str,
    path: &Path,
    change_kind: VaultChangeKind,
) {
    let path_str = path.to_string_lossy().replace('\\', "/");
    let payload = json!({
        "vault_id":    vault_id,
        "path":        path_str,
        "change_kind": change_kind,
    });
    let mut env = Envelope::new(Category::Index, "vault.update");
    env.payload = payload;
    bus.publish(env);
}

// ---------------------------------------------------------------------------
// publish_index_enrichment
// ---------------------------------------------------------------------------

/// Publish a `Category::Index / kind="enrichment"` envelope onto the bus.
///
/// Fire-and-forget: if the bus publish itself fails (bus closed, zero
/// capacity) the error is logged via `tracing::warn!` and the function
/// returns normally.
///
/// # Arguments
///
/// * `bus`        — the shared [`RiftBus`] instance.
/// * `fs_path`    — project-relative filesystem path being enriched
///   (forward-slash normalized). Joined by Tree.svelte against
///   graph node positions in Phase 8.6.
/// * `vault_id`   — vault that supplies the enrichment metadata.
/// * `vault_kind` — Abyssal Index category prefix
///   (`"p"`, `"pr"`, `"r"`, `"s"`, `"lore"`, `"agt"`, `"h"`).
/// * `tags`       — vault-sourced tags to attach to the filesystem node.
pub fn publish_index_enrichment(
    bus: &RiftBus,
    fs_path: &Path,
    vault_id: &str,
    vault_kind: &str,
    tags: Vec<String>,
) {
    let fs_path_str = fs_path.to_string_lossy().replace('\\', "/");
    let payload = json!({
        "fs_path":    fs_path_str,
        "vault_id":  vault_id,
        "vault_kind": vault_kind,
        "tags":       tags,
    });
    let mut env = Envelope::new(Category::Index, "enrichment");
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

    // T1 — vault_update_envelope_shape
    // Verify Category, kind, and payload fields for a vault.update envelope.
    #[test]
    fn vault_update_envelope_shape() {
        let bus = RiftBus::default();
        publish_vault_update(
            &bus,
            "p006",
            Path::new("vaults/p006.md"),
            VaultChangeKind::Modified,
        );

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1, "expected exactly one envelope");
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Index);
        assert_eq!(env.kind, "vault.update");
        assert_eq!(env.payload["vault_id"], "p006");
        assert_eq!(env.payload["path"], "vaults/p006.md");
        assert_eq!(env.payload["change_kind"], "modified");
    }

    // T2 — enrichment_envelope_shape
    // Verify Category, kind, and payload fields for an enrichment envelope.
    #[test]
    fn enrichment_envelope_shape() {
        let bus = RiftBus::default();
        publish_index_enrichment(
            &bus,
            Path::new("src/translators/index.rs"),
            "p006",
            "p",
            vec!["phase8".into(), "index".into()],
        );

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1, "expected exactly one envelope");
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Index);
        assert_eq!(env.kind, "enrichment");
        assert_eq!(env.payload["fs_path"], "src/translators/index.rs");
        assert_eq!(env.payload["vault_id"], "p006");
        assert_eq!(env.payload["vault_kind"], "p");
        assert_eq!(env.payload["tags"][0], "phase8");
        assert_eq!(env.payload["tags"][1], "index");
    }

    // T3 — vault_update_payload_json_roundtrip
    // VaultUpdatePayload serializes and deserializes losslessly.
    #[test]
    fn vault_update_payload_json_roundtrip() {
        let original = VaultUpdatePayload {
            vault_id: "pr001".into(),
            path: "vaults/pr001.md".into(),
            change_kind: VaultChangeKind::Created,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let back: VaultUpdatePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.vault_id, original.vault_id);
        assert_eq!(back.path, original.path);
        assert_eq!(back.change_kind, original.change_kind);
    }

    // T4 — enrichment_payload_json_roundtrip
    // IndexEnrichmentPayload serializes and deserializes losslessly.
    #[test]
    fn enrichment_payload_json_roundtrip() {
        let original = IndexEnrichmentPayload {
            fs_path: "crates/rift-bus/src/translators/index.rs".into(),
            vault_id: "r004".into(),
            vault_kind: "r".into(),
            tags: vec!["tauri2".into(), "svelte5".into()],
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let back: IndexEnrichmentPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.fs_path, original.fs_path);
        assert_eq!(back.vault_id, original.vault_id);
        assert_eq!(back.vault_kind, original.vault_kind);
        assert_eq!(back.tags, original.tags);
    }

    // T5 — vault_change_kind_snake_case_roundtrip
    // VaultChangeKind serializes as snake_case strings.
    #[test]
    fn vault_change_kind_snake_case_roundtrip() {
        let cases = [
            (VaultChangeKind::Created, "\"created\""),
            (VaultChangeKind::Modified, "\"modified\""),
            (VaultChangeKind::Deleted, "\"deleted\""),
        ];
        for (variant, expected_json) in &cases {
            let serialized = serde_json::to_string(variant).expect("serialize");
            assert_eq!(
                &serialized, expected_json,
                "VaultChangeKind::{variant:?} should serialize to {expected_json}"
            );
            let back: VaultChangeKind = serde_json::from_str(&serialized).expect("deserialize");
            assert_eq!(
                back, *variant,
                "VaultChangeKind round-trip failed for {variant:?}"
            );
        }
    }

    // T6 — version_not_bumped
    // Index envelopes carry CURRENT_VERSION (additive kind, no version bump).
    #[test]
    fn version_not_bumped() {
        let bus = RiftBus::default();
        publish_vault_update(&bus, "smoke", Path::new("any.md"), VaultChangeKind::Deleted);
        let (snapshot, _) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].version, crate::CURRENT_VERSION);
    }
}
