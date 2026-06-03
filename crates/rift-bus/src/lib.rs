// rift-bus — Rift Integration Protocol bus crate.
// Spec: decisions/§10.15_real-time_update_mechanism.md.
//
// Phase 4.1 ships envelope schema + Category enum.
// Phase 4.2 ships RiftBus (broadcast + replay buffer + SubscribeFilter).
// Phase 4.3 ships IpcServer + IpcClient (UDS + named pipe via interprocess).
// Future: translator-module registry.

pub mod bus;
pub mod compaction;
pub mod config;
pub mod correlation;
pub mod envelope;
pub mod ipc;
pub mod keyring;
pub mod session_compare;
pub mod session_logger;
pub mod session_reader;
pub mod snapshot;
pub mod translators;

pub use bus::{BusError, RiftBus, SubscribeFilter, Subscription};
pub use envelope::{Category, Envelope, CURRENT_VERSION};
pub use ipc::{IpcClient, IpcError, IpcReader, IpcServer, IpcWriter, MAX_FRAME_BYTES};

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

/// Generic §9 class-3 enrichment registry — publish functions + payloads so any
/// integration (not just the Index vault-walker) can enrich filesystem nodes.
pub use translators::enrichment::{
    publish_enrichment_attach, publish_enrichment_declare, publish_enrichment_revoke,
    EnrichmentAttachPayload, EnrichmentDeclarePayload,
};

// ---------------------------------------------------------------------------
// Status translator re-exports (D-012 unblocked slice — DIR/GIT/REPO)
// ---------------------------------------------------------------------------

/// Run the status translator loop that publishes `Category::Status / kind="usage"`
/// envelopes every 5 seconds with `{ dir, git, repo, ts }`.
///
/// This is an `async fn` — callers must wrap it in `tauri::async_runtime::spawn`
/// (or equivalent) per the Phase 7.1 setup() pattern (mirrors `spawn_vault_walker`).
/// The `shutdown` parameter is a `tokio::sync::Notify` that the host (`src-tauri`)
/// signals from its `RunEvent::ExitRequested` handler so the loop stops cleanly
/// when the main window closes — without this, the status tick continues
/// spawning `git.exe` children after window close (visible terminal flashes
/// until the process is force-killed via Task Manager):
/// ```ignore
/// let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
/// tauri::async_runtime::spawn(async move {
///     rift_bus::spawn_status_translator(bus, project_root, shutdown).await;
/// });
/// ```
///
/// Convenience re-export; full path: `rift_bus::translators::status::spawn_status_translator(...)`.
pub use translators::status::spawn_status_translator;

// ---------------------------------------------------------------------------
// Vault-walker re-exports (Phase 8.5)
// ---------------------------------------------------------------------------

/// Run the Abyssal Index vault-walker (boot walk + live notify watcher).
///
/// This is an `async fn` — callers must wrap it in `tauri::async_runtime::spawn`
/// (or equivalent) per the Phase 7.1 setup() pattern:
/// ```ignore
/// tauri::async_runtime::spawn(async move {
///     rift_bus::spawn_vault_walker(bus, vault_root).await;
/// });
/// ```
///
/// Convenience re-export; full path: `rift_bus::translators::vault_walker::spawn_vault_walker(...)`.
pub use translators::vault_walker::spawn_vault_walker;

/// Convenience re-export; full path: `rift_bus::translators::sentinel::spawn_sentinel_translator(...)`.
pub use translators::sentinel::spawn_sentinel_translator;

// ---------------------------------------------------------------------------
// config re-exports (Phase 6.7)
// ---------------------------------------------------------------------------

/// Top-level Rift configuration struct.
pub use config::{
    CockpitConfig, DetachedPos, FsConfig, IndexDensity, IndexLabelVisibility, McpConfig,
    NotifFilterConfig, ProjectEntry, RiftConfig, SeverityLevel, ShellPref, StatusLineConfig,
    TerminalConfig, TreeConfig, TERMINAL_DEFAULT_FONT_FAMILY, TERMINAL_DEFAULT_FONT_SIZE,
    TERMINAL_DEFAULT_LINE_HEIGHT, TERMINAL_DEFAULT_SCROLLBACK, TERMINAL_MAX_FONT_SIZE,
    TERMINAL_MIN_FONT_SIZE,
};

/// Load config from the platform config directory (default on missing file).
pub use config::load_config;

/// Save config to the platform config directory (atomic write).
pub use config::save_config;

/// Canonical default filesystem ignore globs (single source of truth).
pub use config::DEFAULT_IGNORE_GLOBS;

/// Resolve the platform path for the MCP token file (D-014).
pub use config::mcp_token_path;

/// MCP token helpers (D-014): generate, load, save, ensure.
pub use config::{ensure_mcp_token, generate_mcp_token, load_mcp_token, save_mcp_token};

/// MCP socket discovery helpers (D-014). Host writes its live IPC socket
/// name to this file on startup so the standalone `rift-mcp` binary can
/// connect without `--socket` or `$RIFT_SOCKET_NAME` plumbed through.
pub use config::{clear_mcp_socket, load_mcp_socket, mcp_socket_path, save_mcp_socket};

// ---------------------------------------------------------------------------
// Lane classifier re-exports (D-018)
// ---------------------------------------------------------------------------

/// Lane classifier + prelude injection for live PTY-stream lane classification.
pub use translators::lane::{Lane, LaneClassifier, PreludeInjection, SentinelEvent};

/// Prepare the lane-classification shell prelude for a given shell binary.
pub use translators::lane::prepare_lane_prelude;

// ---------------------------------------------------------------------------
// Session logger re-exports
// ---------------------------------------------------------------------------

/// Run the session event persistence logger (bus → .jsonl file on disk).
///
/// This is an `async fn` — callers must wrap it in `tauri::async_runtime::spawn`
/// per the Phase 7.1 setup() pattern (mirrors `spawn_vault_walker`):
/// ```ignore
/// tauri::async_runtime::spawn(async move {
///     rift_bus::spawn_session_logger(bus, cfg, shutdown).await;
/// });
/// ```
pub use session_logger::spawn_session_logger;

/// Run the idle session-compaction watcher (digests the older session prefix
/// into a sidecar summary on bus idle). `async fn` — wrap in
/// `tauri::async_runtime::spawn`, injecting an app-built summarizer provider:
/// ```ignore
/// tauri::async_runtime::spawn(async move {
///     rift_bus::spawn_compaction(bus, cfg, shutdown, provider).await;
/// });
/// ```
pub use compaction::spawn_compaction;

/// On-demand session compaction (the `rift session compact` trigger). Ignores
/// the idle gate; honors `keep_suffix_events`. See [`compaction::compact_now`].
pub use compaction::compact_now;

/// Re-export [`SessionConfig`] so callers can write `rift_bus::SessionConfig`.
pub use config::SessionConfig;

/// Resolve the platform sessions directory path.
pub use config::sessions_dir;

// ---------------------------------------------------------------------------
// Restart-safe session snapshots (Stage 2)
// ---------------------------------------------------------------------------

/// Persist / load / clear the terminal VT snapshot a re-opened Rift uses to
/// re-hydrate the terminal after a restart. The frontend serializes the xterm
/// buffer (`@xterm/addon-serialize`); these helpers persist it next to the
/// launch's `.jsonl` audit log + `.summary.json` digest. See [`snapshot`].
pub use snapshot::{
    clear_snapshot, latest_snapshot, write_snapshot, PaneSnapshot, RestorePayload, SessionSnapshot,
};
