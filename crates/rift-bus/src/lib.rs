// rift-bus — Rift Integration Protocol bus crate.
// Spec: decisions/§10.15_real-time_update_mechanism.md.
//
// Phase 4.1 ships envelope schema + Category enum.
// Phase 4.2 ships RiftBus (broadcast + replay buffer + SubscribeFilter).
// Phase 4.3 ships IpcServer + IpcClient (UDS + named pipe via interprocess).
// Future: translator-module registry.

pub mod bus;
pub mod config;
pub mod envelope;
pub mod ipc;
pub mod translators;

pub use bus::{BusError, RiftBus, SubscribeFilter, Subscription};
pub use envelope::{Category, Envelope, CURRENT_VERSION};
pub use ipc::{IpcClient, IpcError, IpcServer, MAX_FRAME_BYTES};

/// Publish a `Category::System / kind="error"` envelope via the errors translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_error(...)`.
/// Full path also works: `rift_bus::translators::errors::publish(...)`.
pub use translators::errors::publish as publish_error;

/// Publish a `Category::Pty / kind="command.submitted"` envelope via the commands translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_command(...)`.
/// Full path also works: `rift_bus::translators::commands::publish(...)`.
pub use translators::commands::publish as publish_command;

/// Re-export [`CommandBuffer`] so callers can write `rift_bus::CommandBuffer`.
pub use translators::commands::CommandBuffer;

/// Publish a `Category::Fs` envelope via the filesystem translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_fs_event(...)`.
/// Full path also works: `rift_bus::translators::fs::publish_fs_event(...)`.
pub use translators::fs::publish_fs_event;

/// Spawn a filesystem watcher that publishes `Category::Fs` envelopes.
///
/// Convenience re-export so callers can write `rift_bus::spawn_fs_watcher(...)`.
/// Full path also works: `rift_bus::translators::fs::spawn_fs_watcher(...)`.
pub use translators::fs::spawn_fs_watcher;

/// Re-export [`FsWatcher`] so callers can write `rift_bus::FsWatcher` (needed
/// for Tauri state management).
pub use translators::fs::FsWatcher;

/// Re-export [`FsWatcherError`] so callers can write `rift_bus::FsWatcherError`.
pub use translators::fs::FsWatcherError;

/// Re-export [`TreeNode`] so callers can write `rift_bus::TreeNode`.
pub use translators::fs::TreeNode;

/// Build a static filesystem tree snapshot.
///
/// Convenience re-export so callers can write `rift_bus::build_tree(...)`.
/// Full path: `rift_bus::translators::fs::build_tree(...)`.
pub use translators::fs::build_tree;

/// Default walk depth for [`build_tree`].
pub use translators::fs::FS_TREE_DEFAULT_MAX_DEPTH;

/// Validate that a project-relative path is under the project root and not
/// ignored. Returns the canonicalized absolute [`PathBuf`] on success.
///
/// Convenience re-export; full path: `rift_bus::translators::fs::validate_project_path(...)`.
pub use translators::fs::validate_project_path;

/// Read the text content of a project-relative file path (with size cap).
///
/// Convenience re-export; full path: `rift_bus::translators::fs::read_text(...)`.
pub use translators::fs::read_text;

/// Write text content to an existing project-relative file path.
///
/// Convenience re-export; full path: `rift_bus::translators::fs::write_text(...)`.
pub use translators::fs::write_text;

// ---------------------------------------------------------------------------
// Index translator re-exports (Phase 8.1)
// ---------------------------------------------------------------------------

/// Publish a `Category::Index / kind="vault.update"` envelope via the index translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_vault_update(...)`.
/// Full path also works: `rift_bus::translators::index::publish_vault_update(...)`.
pub use translators::index::publish_vault_update;

/// Publish a `Category::Index / kind="enrichment"` envelope via the index translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_index_enrichment(...)`.
/// Full path also works: `rift_bus::translators::index::publish_index_enrichment(...)`.
pub use translators::index::publish_index_enrichment;

/// Re-export [`VaultUpdatePayload`] so callers can write `rift_bus::VaultUpdatePayload`.
pub use translators::index::VaultUpdatePayload;

/// Re-export [`IndexEnrichmentPayload`] so callers can write `rift_bus::IndexEnrichmentPayload`.
pub use translators::index::IndexEnrichmentPayload;

/// Re-export [`VaultChangeKind`] so callers can write `rift_bus::VaultChangeKind`.
pub use translators::index::VaultChangeKind;

// ---------------------------------------------------------------------------
// config re-exports (Phase 6.7)
// ---------------------------------------------------------------------------

/// Top-level Rift configuration struct.
pub use config::{CockpitConfig, DetachedPos, FsConfig, ProjectEntry, RiftConfig};

/// Load config from the platform config directory (default on missing file).
pub use config::load_config;

/// Save config to the platform config directory (atomic write).
pub use config::save_config;

/// Canonical default filesystem ignore globs (single source of truth).
pub use config::DEFAULT_IGNORE_GLOBS;
