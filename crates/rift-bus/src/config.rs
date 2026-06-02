//! Rift project configuration — `rift-config.toml` persistence.
//!
//! Provides [`RiftConfig`], [`load_config`], and [`save_config`] for reading
//! and writing the user's per-install configuration. The file is located at
//! the platform config directory for the `com.abyssal.rift` app identifier:
//!
//! * Windows: `%APPDATA%\com.abyssal.rift\config\config.toml`
//! * macOS:   `~/Library/Application Support/com.abyssal.rift/config.toml`
//! * Linux:   `$XDG_CONFIG_HOME/rift/config.toml` (or `~/.config/rift/`)
//!
//! ## Schema versioning
//!
//! All top-level and sub-structs carry `#[serde(default)]`, so new fields
//! added in future releases fall back to their `Default` values when reading
//! older config files. This is the same additive-versioning strategy used by
//! the envelope schema.
//!
//! ## Atomic write
//!
//! [`save_config`] writes to a `.tmp` file then renames to the final path to
//! avoid torn writes on crash or power loss.
//!
//! ## DEFAULT_IGNORE_GLOBS
//!
//! The canonical list of default filesystem ignore patterns lives here and is
//! the single source of truth for the entire codebase. Previously duplicated
//! in `translators/fs.rs` — Phase 6.7 consolidation closure (Validator 6.2
//! lesson `fs_tree-config-duplication-with-watcher-setup`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// DEFAULT_IGNORE_GLOBS — canonical single source of truth (Phase 6.7)
// ---------------------------------------------------------------------------

/// Default filesystem ignore patterns used by both the watcher and `fs_tree`.
///
/// This is the **canonical source of truth** for ignore globs in the Rift
/// codebase. `translators/fs.rs` imports this constant and builds its
/// [`globset::GlobSet`] from it. Do not define a parallel list elsewhere.
pub const DEFAULT_IGNORE_GLOBS: &[&str] = &[
    ".git",
    ".git/**",
    "node_modules",
    "node_modules/**",
    "target",
    "target/**",
    "dist",
    "dist/**",
    ".svelte-kit",
    ".svelte-kit/**",
    "*.log",
];

/// Default filesystem tree walk depth.
///
/// Re-exported here alongside `DEFAULT_IGNORE_GLOBS` so config defaults are
/// co-located. Matches [`crate::translators::fs::FS_TREE_DEFAULT_MAX_DEPTH`].
pub const FS_CONFIG_DEFAULT_MAX_DEPTH: u32 = 6;

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

/// Top-level Rift configuration.
///
/// `#[serde(default)]` ensures that fields absent from an older config file
/// fall back to their `Default` implementation — additive versioning.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RiftConfig {
    /// Recent project entries, sorted by `last_used_ms` descending.
    pub projects: Vec<ProjectEntry>,
    /// Filesystem-watcher and tree-walk settings.
    pub fs: FsConfig,
    /// Cockpit GUI settings (window position etc.).
    pub cockpit: CockpitConfig,
    /// Abyssal Index graph settings (Phase 8.7).
    pub index: IndexConfig,
    /// MCP server settings (D-014, Phase A+).
    /// Off by default. Token is auto-generated on first launch with
    /// `enabled = true`; lives next to `config.toml` as `mcp_token`.
    pub mcp: McpConfig,
    /// Session event persistence (bus → .jsonl file on disk).
    /// Enabled by default; 7-day retention; 100 MiB cap per session file.
    pub session: SessionConfig,
    /// Terminal surface settings (shell preference, font, scrollback, lanes).
    /// Defaults to `Auto` shell discovery (pwsh > powershell > %COMSPEC% > cmd
    /// on Windows, $SHELL > /bin/zsh > /bin/bash > /bin/sh on Unix), 13px
    /// JetBrains Mono, 1.55 line-height, 1000-line scrollback, lanes enabled.
    pub terminal: TerminalConfig,
    /// Notification tab filter rules. Per-tab severity thresholds control
    /// which events render in notification tabs. Default: info.
    pub notif_filters: NotifFilterConfig,
    /// Filesystem tree display settings (D-020 heatmap groundwork).
    pub tree: TreeConfig,
    /// StatusLine segment visibility and color overrides (§10.2).
    pub statusline: StatusLineConfig,
    /// Alert rules — user-configurable threshold-based event alerting.
    /// Each rule watches a notification tab for event bursts and triggers
    /// a visual/audio action (flash badge, auto-promote, tone).
    pub alerts: AlertsConfig,
    /// Set to `true` after the welcome overlay is dismissed on first launch.
    /// Defaults to `false` via `#[serde(default)]` so existing configs
    /// (which lack this field) trigger the welcome experience.
    pub first_run_completed: bool,
    /// Optional integration toggles (Aegis, Index). Runtime-gated — the
    /// binary compiles both features but only activates translators when
    /// the user opts in via the welcome overlay or Settings panel.
    pub integrations: IntegrationsConfig,
    /// Ensemble Router — multi-model LLM orchestration (Phase 1+).
    /// Off by default. When enabled, Rift manages local llama-server
    /// processes and routes prompts to configured models.
    pub ensemble: EnsembleConfig,
}

/// A recently-used project entry stored in the config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectEntry {
    /// User-friendly display name (defaults to the directory basename).
    pub name: String,
    /// Absolute, canonicalized path to the project directory.
    pub path: PathBuf,
    /// Unix epoch milliseconds at the last `project_swap` invocation for
    /// this path. Matches `Date.now()` shape on the frontend.
    pub last_used_ms: u64,
}

/// Filesystem-watcher and tree-walk configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FsConfig {
    /// Glob patterns matched against relative paths; matching paths are
    /// silently skipped by both the watcher and `fs_tree`.
    /// Defaults to [`DEFAULT_IGNORE_GLOBS`].
    pub ignore_globs: Vec<String>,
    /// Maximum directory depth for `fs_tree` walks.
    /// Defaults to [`FS_CONFIG_DEFAULT_MAX_DEPTH`].
    pub max_depth: u32,
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            ignore_globs: DEFAULT_IGNORE_GLOBS.iter().map(|s| s.to_string()).collect(),
            max_depth: FS_CONFIG_DEFAULT_MAX_DEPTH,
        }
    }
}

/// Cockpit GUI configuration (window layout etc.).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CockpitConfig {
    /// Detached cockpit window position / size.
    /// Future migration target for the Phase 6.4 `localStorage` usage.
    pub detached_pos: Option<DetachedPos>,
}

/// Serializable window position + size for the detached cockpit window.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetachedPos {
    /// Window left edge in logical pixels.
    pub x: i32,
    /// Window top edge in logical pixels.
    pub y: i32,
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
}

/// Abyssal Index graph configuration (Phase 8.7).
///
/// `#[serde(default)]` ensures that configs written before this field existed
/// (i.e. without an `[index]` section) parse without error, falling back to
/// the `Default` impl which gives `SyncMode::Live` and empty collections.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexConfig {
    /// Glob patterns for vault entries the graph should ignore.
    /// Empty by default (no entries ignored).
    pub ignore_globs: Vec<String>,
    /// Sync strategy for the vault graph. Defaults to [`SyncMode::Live`].
    pub sync_mode: SyncMode,
    /// Reserved: opaque camera/zoom/pan transform blob.
    /// Not read or written in v1 — present for forward-compat config parsing.
    pub camera_transform: Option<serde_json::Value>,
    /// Reserved: opaque per-node position snapshot blob.
    /// Not read or written in v1 — transient D3 positions are never persisted.
    pub node_positions: Option<serde_json::Value>,
    /// Phase 8.7p — IndexGraph node label visibility policy.
    /// `Always` shows id + subtitle on every node; `HoverOnly` hides them
    /// until hover; `OnZoom2x` requires zoom factor ≥ 2.0 to render text.
    pub label_visibility: IndexLabelVisibility,
    /// Phase 8.7p — IndexGraph cluster density. Scales the radial RADIUS
    /// constant: Compact = 0.85, Standard = 1.0, Spacious = 1.2.
    pub density: IndexDensity,
}

/// IndexGraph label visibility policy (Phase 8.7p).
///
/// `#[non_exhaustive]` mirrors [`SyncMode`] — newer-version configs
/// degrade to `Unknown` instead of erroring.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IndexLabelVisibility {
    /// Render id + subtitle text on every node always (default).
    #[default]
    Always,
    /// Render text only on the currently hovered node.
    HoverOnly,
    /// Render text only when the d3-zoom transform `k` is ≥ 2.0.
    OnZoom2x,
    /// Forward-compat catch-all.
    #[serde(other)]
    Unknown,
}

/// IndexGraph cluster density (Phase 8.7p). Scales the radial RADIUS
/// constant in `IndexGraph.svelte`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IndexDensity {
    /// 0.85× the standard RADIUS — fits more vaults in a small viewport.
    Compact,
    /// 1.0× — current behavior (default).
    #[default]
    Standard,
    /// 1.2× — more breathing room around every cluster.
    Spacious,
    /// Forward-compat catch-all.
    #[serde(other)]
    Unknown,
}

/// Severity level for notification tab filtering.
///
/// Events whose derived severity falls below a tab's threshold are not
/// rendered (but still captured by the session logger). Debug is the
/// lowest (show everything); Error is the highest (show only errors).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SeverityLevel {
    /// Show everything including debug-level events.
    Debug,
    /// Default threshold — show info and above.
    #[default]
    Info,
    /// Show only warnings and errors.
    Warn,
    /// Show only errors.
    Error,
    /// Forward-compat catch-all. Treated as Info at runtime.
    #[serde(other)]
    Unknown,
}

/// Notification filter configuration.
///
/// `default_threshold` applies to any tab not listed in `per_tab`.
/// The BusTail tab defaults to Debug (firehose by design) unless
/// explicitly overridden.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifFilterConfig {
    /// Minimum severity for events to appear in notification tabs.
    pub default_threshold: SeverityLevel,
    /// Per-tab threshold overrides keyed by tab ID (e.g. "errors", "hooks",
    /// "bustail", "aegis", "agents", "todo", "git", "index", "commands").
    pub per_tab: std::collections::HashMap<String, SeverityLevel>,
}

impl Default for NotifFilterConfig {
    fn default() -> Self {
        Self {
            default_threshold: SeverityLevel::Info,
            per_tab: std::collections::HashMap::new(),
        }
    }
}

/// Session event persistence configuration.
///
/// When enabled, a dedicated bus subscriber writes every envelope to a
/// per-launch JSON-lines file under the platform sessions directory. The
/// cleanup sweep deletes files older than `retention_days` at startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Master switch. When `false`, the session logger task returns
    /// immediately and no file is created.
    pub enabled: bool,
    /// Auto-delete session files older than this many days at startup.
    pub retention_days: u32,
    /// Stop writing to the current session file once it exceeds this size.
    /// The file is capped, not rotated — the next launch gets a fresh file.
    pub max_file_size_mb: u32,
    /// Idle-compaction trigger: minutes of bus inactivity before the older
    /// prefix of the active session log is summarized into a sidecar.
    /// `0` (default) disables compaction — opt-in, mirroring the rest of this
    /// struct's conservative defaults. Recommended: `15` (≈ the 5-min/1-hr
    /// prompt-cache TTL window, so stale context is digested before it expires
    /// unused). See `compaction.rs`.
    pub idle_compact_after_minutes: u32,
    /// Number of most-recent envelopes kept verbatim (NOT summarized) when
    /// compaction fires. Only the older prefix beyond this suffix is digested.
    pub keep_suffix_events: usize,
    /// Restart-safe sessions (Stage 2): when `true`, on startup the frontend
    /// re-hydrates the terminal from the most recent prior-launch snapshot
    /// (scrollback + cwd + the compaction digest) before spawning a fresh
    /// shell. `false` (default) = no restore — opt-in, mirroring this struct's
    /// conservative defaults. See `snapshot.rs`.
    pub restore_on_startup: bool,
    /// How often (seconds) the frontend serializes the active terminal buffer
    /// to its snapshot sidecar. A best-effort capture also fires on window
    /// close. `0` disables periodic capture (close-only). Ignored unless
    /// `restore_on_startup` will consume it on the next launch.
    pub snapshot_interval_seconds: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 7,
            max_file_size_mb: 100,
            idle_compact_after_minutes: 0,
            keep_suffix_events: 100,
            restore_on_startup: false,
            snapshot_interval_seconds: 30,
        }
    }
}

/// Resolve the platform sessions directory for Rift.
///
/// Sibling of `config.toml`:
/// * Windows: `%APPDATA%\com.abyssal.rift\config\sessions\`
/// * macOS:   `~/Library/Application Support/com.abyssal.rift/sessions/`
/// * Linux:   `$XDG_CONFIG_HOME/rift/sessions/`
pub fn sessions_dir() -> Result<PathBuf, ConfigError> {
    let dirs =
        directories::ProjectDirs::from("com", "abyssal", "rift").ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().join("sessions"))
}

/// MCP server configuration (D-014, locked 2026-04-29).
///
/// Off by default. Three tiered toggles gate progressively riskier tool
/// surfaces. Token is generated on first enable and persisted as a sibling
/// `mcp_token` file alongside `config.toml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Master switch. When `false`, the host does not subscribe to
    /// `Category::Mcp` envelopes and the `rift-mcp` binary's handshake
    /// fails closed.
    pub enabled: bool,
    /// Allow DOM snapshot + screenshot tools (Phase C). Default `false`.
    pub allow_inspection: bool,
    /// Allow `js_eval` tool (Phase C). Separate toggle from
    /// `allow_inspection` so users can opt into read-only inspection
    /// without enabling JS execution. Default `false`.
    pub allow_js_eval: bool,
    /// Allow mutating tools — `bus_publish`, `pty_input`, `fs_write`,
    /// `git_action` (Phase D). Default `false`.
    pub allow_mutations: bool,
}

/// Resolve the platform path for the MCP token file.
///
/// Sibling of `config.toml`, NOT a separate `~/.rift/` directory — see
/// D-014 §11 question 5.
///
/// * Windows: `%APPDATA%\com.abyssal.rift\config\mcp_token`
/// * macOS:   `~/Library/Application Support/com.abyssal.rift/mcp_token`
/// * Linux:   `$XDG_CONFIG_HOME/rift/mcp_token`
pub fn mcp_token_path() -> Result<PathBuf, ConfigError> {
    let dirs =
        directories::ProjectDirs::from("com", "abyssal", "rift").ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().join("mcp_token"))
}

/// Generate a 32-byte hex token (64 chars) suitable for MCP handshake.
///
/// Uses `getrandom` (CSPRNG: `/dev/urandom` on Unix, `BCryptGenRandom` on
/// Windows). Caller is responsible for persisting it via [`save_mcp_token`].
pub fn generate_mcp_token() -> Result<String, ConfigError> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| ConfigError::Io(std::io::Error::other(format!("getrandom: {e}"))))?;
    let mut s = String::with_capacity(64);
    for b in bytes.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    Ok(s)
}

/// Load the MCP token from the platform token path, or `None` if absent.
pub fn load_mcp_token() -> Result<Option<String>, ConfigError> {
    let path = mcp_token_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// Atomically write `token` to the MCP token path. Creates parent dir.
/// On Unix, applies `chmod 600` after rename so the token is owner-only.
pub fn save_mcp_token(token: &str) -> Result<(), ConfigError> {
    let path = mcp_token_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token)?;
    std::fs::rename(&tmp, &path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

/// Ensure a token exists on disk; generate + persist on first call.
/// Returns the token (newly created or pre-existing).
pub fn ensure_mcp_token() -> Result<String, ConfigError> {
    if let Some(t) = load_mcp_token()? {
        return Ok(t);
    }
    let t = generate_mcp_token()?;
    save_mcp_token(&t)?;
    Ok(t)
}

/// Resolve the platform path for the MCP discovery file (D-014).
///
/// Sibling of `mcp_token`. Holds the IPC socket name of the currently-running
/// Rift host so the standalone `rift-mcp` binary (spawned by Claude Code with
/// no env or args) can reach it. Written on host startup, removed on exit.
pub fn mcp_socket_path() -> Result<PathBuf, ConfigError> {
    let dirs =
        directories::ProjectDirs::from("com", "abyssal", "rift").ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().join("mcp_socket"))
}

/// Read the discovery file and return the recorded socket name, or `None`
/// if the file is absent or the owning process is dead.
///
/// When a sidecar `.pid` file exists alongside the discovery file, validates
/// the PID against the OS process table. A stale file from a crashed Rift is
/// detected and removed here — the caller never sees a socket name whose
/// host is confirmed dead. The socket file itself stays plain text (just the
/// socket name) for backwards compatibility with older `rift-mcp` binaries.
pub fn load_mcp_socket() -> Result<Option<String>, ConfigError> {
    let path = mcp_socket_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let pid_path = path.with_extension("pid");
            if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    if !is_process_alive(pid) {
                        tracing::info!(
                            pid,
                            socket = %trimmed,
                            "mcp_socket discovery: stale file (PID dead) — removing"
                        );
                        let _ = std::fs::remove_file(&path);
                        let _ = std::fs::remove_file(&pid_path);
                        return Ok(None);
                    }
                }
            }
            Ok(Some(trimmed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// Atomically write the current host's socket name to the discovery file,
/// plus a sidecar `.pid` file so readers can detect stale sockets from
/// crashed hosts without attempting a connect (which hangs for seconds on
/// Windows named pipes). The socket file stays plain text for backwards compat.
pub fn save_mcp_socket(socket_name: &str) -> Result<(), ConfigError> {
    let path = mcp_socket_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, socket_name)?;
    std::fs::rename(&tmp, &path)?;
    let _ = std::fs::write(path.with_extension("pid"), std::process::id().to_string());
    Ok(())
}

/// Check if a process with the given PID is alive.
/// Uses `tasklist` on Windows, `kill -0` via libc on Unix.
#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(true) // assume alive on error — let the connect timeout decide
}

#[cfg(not(windows))]
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Remove the discovery file. No-op if absent. Called from the host's
/// `ExitRequested` handler so the next Rift launch starts with a clean
/// slate (and so a stopped host can't masquerade as live).
pub fn clear_mcp_socket() -> Result<(), ConfigError> {
    let path = mcp_socket_path()?;
    let _ = std::fs::remove_file(path.with_extension("pid"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ConfigError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// TreeConfig (D-020 — temporal activity heatmap)
// ---------------------------------------------------------------------------

/// Filesystem tree display configuration (D-020 heatmap groundwork).
///
/// Controls the activity heatmap overlay that color-codes filesystem tree
/// nodes by touch frequency. Off by default — no visual change until the
/// user opts in via Settings → Tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TreeConfig {
    /// Master switch for the activity heatmap overlay on the filesystem tree.
    /// When `false`, tree nodes render without frequency-based coloring.
    pub heatmap_enabled: bool,
    /// Sliding window (in minutes) over which touch frequency is aggregated.
    /// Supported values: 5, 15, 60. Out-of-range values are clamped at the
    /// frontend; the backend stores whatever the frontend sends.
    pub heatmap_window_minutes: u32,
}

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            heatmap_enabled: false,
            heatmap_window_minutes: 15,
        }
    }
}

// ---------------------------------------------------------------------------
// StatusLineConfig (§10.2 — segment visibility + color overrides)
// ---------------------------------------------------------------------------

/// Per-segment visibility and optional color override for the status line.
///
/// Each segment can be individually toggled. Color overrides are CSS color
/// strings (hex, rgb, hsl) — when `None`, the default palette from
/// `styles.css` Phase 8.7g.2 applies.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusLineConfig {
    pub show_dir: bool,
    pub show_git: bool,
    pub show_repo: bool,
    pub show_session: bool,
    pub show_skill: bool,
    pub show_thinking: bool,
    pub show_effort: bool,
    pub show_model: bool,
    pub show_ctx: bool,
    pub show_session_use: bool,
    pub show_week: bool,
    pub show_cost: bool,
    /// Optional per-segment color overrides (CSS color strings).
    /// Keys: "dir", "git", "repo", "session", "skill", "thinking",
    /// "effort", "model", "ctx", "session_use", "week", "cost".
    pub color_overrides: std::collections::HashMap<String, String>,
}

impl Default for StatusLineConfig {
    fn default() -> Self {
        Self {
            show_dir: true,
            show_git: true,
            show_repo: true,
            show_session: true,
            show_skill: true,
            show_thinking: true,
            show_effort: true,
            show_model: true,
            show_ctx: true,
            show_session_use: true,
            show_week: true,
            show_cost: true,
            color_overrides: std::collections::HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// AlertsConfig — user-configurable event alerting rules
// ---------------------------------------------------------------------------

/// Alert rules configuration.
///
/// Each rule watches a specific notification tab for event rate thresholds
/// (via the per-tab SparklineBuffer) and triggers an action when exceeded.
/// Rules persist in `config.toml` under `[alerts]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertsConfig {
    /// Ordered list of alert rules. Evaluated per-envelope in the master
    /// subscription; first matching rule per tab per envelope wins.
    pub rules: Vec<AlertRule>,
}

/// A single alert rule targeting one notification tab.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlertRule {
    /// Unique identifier (nanoid or UUID).
    pub id: String,
    /// Notification tab this rule watches (e.g. "errors", "hooks", "mcp").
    pub tab_id: String,
    /// Minimum event severity to count toward the threshold.
    pub severity: SeverityLevel,
    /// Number of qualifying events within `window_secs` that triggers the action.
    pub threshold: u32,
    /// Sliding window in seconds (1–60). Evaluated against SparklineBuffer.
    pub window_secs: u32,
    /// Action to execute when the threshold is met.
    pub action: AlertAction,
    /// Whether this rule is active.
    pub enabled: bool,
}

/// Action triggered by an alert rule.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AlertAction {
    /// Flash the tab badge red (CSS animation, 3 cycles).
    #[default]
    Flash,
    /// Auto-promote the tab to the side pane.
    Promote,
    /// Play a short audio tone (Web Audio API).
    Tone,
    /// Forward-compat catch-all.
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// IntegrationsConfig — runtime toggles for optional Aegis + Index
// ---------------------------------------------------------------------------

/// Runtime integration toggles for optional subsystems.
///
/// The release binary compiles with both `aegis-detect` and `index` Cargo
/// features, but translators only spawn when the user opts in here. This
/// decouples compile-time capability from runtime activation, matching the
/// §9 integration-decoupling principle.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationsConfig {
    /// Aegis agent observability — spawn detection probe + translator.
    pub aegis_enabled: bool,
    /// Abyssal Index knowledge cockpit — spawn vault walker + bridge.
    pub index_enabled: bool,
}

// ---------------------------------------------------------------------------
// TerminalConfig (D-018 groundwork — shell preference + font + lanes)
// ---------------------------------------------------------------------------

/// Default xterm font size in CSS pixels.
pub const TERMINAL_DEFAULT_FONT_SIZE: u16 = 13;
/// Minimum font size honored by the zoom keybinds.
pub const TERMINAL_MIN_FONT_SIZE: u16 = 8;
/// Maximum font size honored by the zoom keybinds.
pub const TERMINAL_MAX_FONT_SIZE: u16 = 48;
/// Default xterm CSS line-height multiplier.
pub const TERMINAL_DEFAULT_LINE_HEIGHT: f32 = 1.55;
/// Default xterm scrollback ring buffer line count.
pub const TERMINAL_DEFAULT_SCROLLBACK: u32 = 1000;

/// Terminal surface configuration.
///
/// All fields are `#[serde(default)]` per the additive-versioning convention —
/// configs written before this section existed parse without error and fall
/// back to the defaults below.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Which shell `pty_start` should spawn. `Auto` (the default) walks a
    /// per-platform discovery list; `Custom` overrides with an explicit path.
    pub shell: ShellPref,
    /// xterm font size in CSS pixels. Clamped at runtime to
    /// [`TERMINAL_MIN_FONT_SIZE`]..=[`TERMINAL_MAX_FONT_SIZE`].
    pub font_size: u16,
    /// CSS font-family stack for the entire app. Applied via `--font-family`
    /// custom property. Defaults to `"'JetBrains Mono', monospace"`.
    pub font_family: String,
    /// xterm line-height multiplier (1.0 = no extra leading).
    pub line_height: f32,
    /// xterm scrollback ring buffer line count.
    pub scrollback: u32,
    /// Whether Rift-emitted lines (session-exited, pty-failed, etc.) carry
    /// §10.1 lane tag prefixes + ANSI lane colors. When `false`, those lines
    /// fall through as plain text. Live-PTY-stream lane classification is
    /// tracked separately under DEFERRED.md D-018.
    pub lanes_enabled: bool,
    /// Named color palette applied to the xterm.js theme. Defaults to `"amber"`
    /// (the original CRT look). Frontend resolves the name to a full ITheme object.
    pub color_palette: String,
    /// User-defined color overrides for the `"custom"` palette. Keys are xterm
    /// ITheme property names (`background`, `foreground`, `cursor`, `black`,
    /// `red`, etc.); values are CSS hex color strings. Ignored when
    /// `color_palette` is not `"custom"`.
    pub custom_palette: std::collections::HashMap<String, String>,
    /// Cursor shape, mapped to the xterm.js `cursorStyle` option
    /// (`block` / `bar` / `underline`). Defaults to [`CursorStyle::Block`].
    pub cursor_style: CursorStyle,
    /// Whether the cursor blinks, mapped to the xterm.js `cursorBlink` option.
    /// Defaults to `true`.
    pub cursor_blink: bool,
}

/// Terminal cursor shape — maps to the xterm.js `cursorStyle` option.
///
/// `#[non_exhaustive]` + `#[serde(other)] Unknown` mirrors [`ShellPref`]: a
/// newer-version config naming a shape this build doesn't recognize degrades
/// to `Unknown` (rendered as `Block`) instead of failing to parse.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CursorStyle {
    /// Full block cursor (default).
    #[default]
    Block,
    /// Vertical bar / I-beam cursor.
    Bar,
    /// Underline cursor.
    Underline,
    /// Forward-compat catch-all. Treated as `Block` at render time.
    #[serde(other)]
    Unknown,
}

/// Default CSS font-family stack.
pub const TERMINAL_DEFAULT_FONT_FAMILY: &str = "'JetBrains Mono', monospace";

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: ShellPref::default(),
            font_size: TERMINAL_DEFAULT_FONT_SIZE,
            font_family: TERMINAL_DEFAULT_FONT_FAMILY.to_string(),
            line_height: TERMINAL_DEFAULT_LINE_HEIGHT,
            scrollback: TERMINAL_DEFAULT_SCROLLBACK,
            lanes_enabled: true,
            color_palette: "amber".to_string(),
            custom_palette: std::collections::HashMap::new(),
            cursor_style: CursorStyle::default(),
            cursor_blink: true,
        }
    }
}

/// Shell preference for the terminal PTY.
///
/// `#[non_exhaustive]` + `#[serde(other)] Unknown` mirrors [`SyncMode`] —
/// older Rift versions reading newer configs fall through to `Unknown`
/// instead of erroring. `Custom` carries an explicit executable path; it
/// is the user's responsibility to ensure the binary exists.
///
/// `Auto` (the default) defers to platform discovery in
/// `rift_core::shell::resolve_shell` — see that function for the order.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "path")]
#[non_exhaustive]
pub enum ShellPref {
    /// Per-platform discovery walk.
    #[default]
    Auto,
    /// PowerShell 7 (`pwsh.exe` / `pwsh`).
    Pwsh,
    /// Windows PowerShell 5.1 (`powershell.exe`).
    Powershell,
    /// `cmd.exe`.
    Cmd,
    /// `/bin/bash`.
    Bash,
    /// `/bin/zsh`.
    Zsh,
    /// `/bin/sh`.
    Sh,
    /// Explicit executable path. Caller is responsible for existence + exec
    /// permissions; failure surfaces through `pty_start`'s error envelope.
    Custom(PathBuf),
    /// Catch-all for any `kind` string not recognized by this Rift version.
    /// Allows newer-version configs to be read by older Rift without crashing.
    /// Treat as `Auto` at resolve time.
    #[serde(other)]
    Unknown,
}

/// Sync strategy for the Abyssal Index vault graph.
///
/// New variants in future Rift versions are caught by [`SyncMode::Unknown`]
/// via `#[serde(other)]`, so an older Rift reading a newer config does not
/// panic or error. `#[non_exhaustive]` prevents exhaustive `match` arms
/// outside this crate, ensuring callers handle future variants.
///
/// Note: `SyncMode::Unknown` is a deserialize-only catch-all. It cannot be
/// serialized back as a known TOML value — round-trip of `Unknown` would
/// fail. Callers must never construct `Unknown` directly; it is reachable
/// only via deserialization of an unrecognized string.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SyncMode {
    /// Live sync: vault-walker notify-watcher 100ms debounce (current v1 behavior).
    #[default]
    Live,
    /// Manual sync: placeholder variant for v1.x forward-compat.
    /// Read from config but has no effect in v1 — no manual-refresh behavior
    /// is implemented. Variant exists so configs specifying `sync_mode = "manual"`
    /// do not error or fall through to `Unknown`.
    Manual,
    /// Catch-all for any `sync_mode` string not recognized by this Rift version.
    /// Allows newer-version configs to be read by older Rift without crashing.
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// EnsembleConfig (Ensemble Router — multi-model LLM orchestration)
// ---------------------------------------------------------------------------

/// Ensemble Router configuration. Controls multi-model LLM management,
/// routing profiles, and local llama-server process settings.
///
/// Off by default — zero impact on startup for single-model users.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EnsembleConfig {
    /// Master switch. When `false`, no LLM translators start and no
    /// `Category::Llm` envelopes are published.
    pub enabled: bool,
    /// Active routing profile. `Manual` = user picks model explicitly.
    pub active_profile: RoutingProfile,
    /// Model ID to use when no routing rule matches (fallback).
    pub default_model: String,
    /// Configured models (cloud APIs, local llama-server, remote endpoints).
    pub models: Vec<ModelConfig>,
    /// Optional tiny model used to refine `TaskType::Other` classifications
    /// under auto profiles. Its `id` must match one of `models`. `None` =
    /// pure keyword routing (fully backward compatible — zero extra latency).
    #[serde(default)]
    pub classifier_model_id: Option<String>,
}

impl Default for EnsembleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            active_profile: RoutingProfile::Manual,
            default_model: String::new(),
            models: Vec::new(),
            classifier_model_id: None,
        }
    }
}

/// Routing profile — how the router decides which model handles a prompt.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingProfile {
    /// No auto-routing. User selects model via quick switcher.
    #[default]
    Manual,
    /// Routes to cheapest viable model. Local → Server → Flash → Pro.
    CostOptimized,
    /// Routes to most capable model for the detected task type.
    QualityFirst,
    /// Heuristic blend of cost and capability.
    Balanced,
}

/// A configured LLM model available in Rift.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Internal unique identifier (e.g. `"claude-opus"`, `"local-gemma"`).
    pub id: String,
    /// User-facing display name.
    pub display_name: String,
    /// Provider type — determines which translator handles requests.
    pub provider: ProviderType,
    /// Provider-specific model identifier (e.g. `"claude-opus-4-6"`,
    /// `"gemma-4-27b-it-Q4_K_M.gguf"`).
    pub model_identifier: String,
    /// How the model is hosted — cloud API, local process, or remote endpoint.
    pub hosting: HostingMode,
    /// API endpoint URL (auto-filled for known cloud providers).
    pub endpoint: String,
    /// Keyring service name for the API key. `None` for local/remote
    /// llama-server (no auth needed). The actual key is never stored here.
    pub api_key_ref: Option<String>,
    /// CSS variable name for the model indicator color (e.g. `"--model-claude"`).
    pub color: String,
    /// 2-4 character short identifier for the gutter glyph (e.g. `"CLD"`).
    pub short_id: String,
    /// Model capabilities — used for routing decisions.
    pub capabilities: ModelCapabilities,
    /// Whether the model is available for use. Disabled models are skipped by
    /// the router and hidden from model pickers. Defaults to `true` (and for
    /// configs written before this field existed).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Default for [`ModelConfig::enabled`] — models are enabled unless toggled off.
fn default_enabled() -> bool {
    true
}

impl ModelConfig {
    /// Build the recommended tiny task-type classifier — a Llama-3.2-1B GGUF
    /// served by a dedicated local llama-server on its own port. Used to
    /// refine the router's ambiguous [`TaskType::Other`](crate) bucket
    /// (see `EnsembleConfig::classifier_model_id`). Small context, full GPU
    /// offload, zero cost. The caller decides `enabled` / `auto_start` based
    /// on whether the GGUF is actually present.
    pub fn llama_classifier(model_path: std::path::PathBuf, port: u16) -> Self {
        ModelConfig {
            id: "llama-classifier".to_string(),
            display_name: "Llama 3.2 1B (classifier)".to_string(),
            provider: ProviderType::LlamaServer,
            model_identifier: "Llama-3.2-1B-Instruct-Q6_K.gguf".to_string(),
            hosting: HostingMode::Local {
                process_config: LlamaServerConfig {
                    model_path,
                    ctx_size: 2048,
                    n_gpu_layers: 99,
                    port,
                    auto_start: true,
                    ..LlamaServerConfig::default()
                },
            },
            endpoint: format!("http://127.0.0.1:{port}"),
            api_key_ref: None,
            color: "--model-local".to_string(),
            short_id: "CLS".to_string(),
            capabilities: ModelCapabilities {
                cost_per_1m_input: 0.0,
                cost_per_1m_output: 0.0,
                max_context_tokens: 2048,
                supports_streaming: false,
                ..ModelCapabilities::default()
            },
            enabled: true,
        }
    }
}

/// Provider type — determines which LLM translator handles requests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Anthropic,
    Google,
    LlamaServer,
    OpenAiCompat,
    /// External command-line tool (e.g. the `gemini` CLI). Authenticates via
    /// the tool's own session — no API key. The model's `endpoint` field holds
    /// the command template; see `translators::llm_cli`.
    Cli,
}

/// How a model is hosted — determines process management and health-check
/// behavior.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum HostingMode {
    /// Cloud API (Anthropic, Google, etc.) — Rift sends HTTP requests.
    Cloud,
    /// Local llama-server process — Rift manages the process lifecycle.
    Local {
        #[serde(flatten)]
        process_config: LlamaServerConfig,
    },
    /// Remote llama-server — Rift connects but does not manage the process.
    Remote { health_check_interval_secs: u64 },
}

/// Configuration for a Rift-managed local llama-server instance.
///
/// All fields map to llama-server CLI flags. Rift's Settings UI writes these;
/// the process manager reads them when spawning.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LlamaServerConfig {
    /// Path to the GGUF model file.
    pub model_path: PathBuf,
    /// `--flash-attn` — hardware-accelerated attention kernels.
    pub flash_attention: bool,
    /// `--ctx-size` — maximum context window in tokens.
    pub ctx_size: u32,
    /// `--cache-type-k` — KV cache key quantization type.
    pub cache_type_k: KvCacheType,
    /// `--cache-type-v` — KV cache value quantization type.
    pub cache_type_v: KvCacheType,
    /// `--n-gpu-layers` — transformer layers offloaded to GPU. 99 = all.
    /// A negative value omits the flag entirely so llama-server's own
    /// device-memory fitter chooses the offload split (recommended on
    /// VRAM-constrained cards — a hardcoded 99 aborts the fitter).
    pub n_gpu_layers: i32,
    /// `--cpu-moe` — keep ALL Mixture-of-Experts tensors on the CPU, leaving
    /// only attention/shared weights on the GPU. Large VRAM saving for MoE
    /// models (e.g. Gemma 4 A4B) at a modest speed cost.
    pub cpu_moe: bool,
    /// `--n-cpu-moe N` — offload MoE expert tensors for the first N layers to
    /// the CPU. Finer-grained than `cpu_moe`; tune N to fit VRAM exactly.
    /// Ignored when `cpu_moe` is true (which offloads all experts). `None`
    /// omits the flag.
    pub n_cpu_moe: Option<u32>,
    /// `--cache-ram N` — host-RAM prompt-reuse cache size in MiB (llama-server
    /// default 8192). `Some(0)` disables it, `Some(n)` caps it, `None` omits
    /// the flag (uses the 8 GiB default). This is a speed cache in system RAM,
    /// NOT model weights — disabling it frees RAM at the cost of prompt reuse.
    pub cache_ram: Option<u32>,
    /// `--threads` — CPU thread count. `None` = auto-detect.
    pub threads: Option<u32>,
    /// `--parallel` — concurrent request slots.
    pub parallel: u32,
    /// `--port` — HTTP server port.
    pub port: u16,
    /// `CUDA_VISIBLE_DEVICES` env var for GPU selection on multi-GPU systems.
    pub cuda_visible_devices: Option<String>,
    /// Launch this model automatically when Rift starts.
    pub auto_start: bool,
    /// Automatically restart this server if it crashes. The health monitor
    /// re-spawns it with a bounded retry (capped attempts per time window);
    /// once the cap is hit the model is left in an error state rather than
    /// restart-looping. Off by default — a crash then simply surfaces as an
    /// error status in the UI with no auto-recovery.
    pub auto_restart: bool,
    /// Additional CLI flags (validated against known llama-server flags).
    pub extra_flags: Vec<String>,
}

impl Default for LlamaServerConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            flash_attention: true,
            ctx_size: 32768,
            cache_type_k: KvCacheType::Q8_0,
            cache_type_v: KvCacheType::Q8_0,
            n_gpu_layers: 99,
            cpu_moe: false,
            n_cpu_moe: None,
            cache_ram: None,
            threads: None,
            parallel: 1,
            port: 8081,
            cuda_visible_devices: None,
            auto_start: false,
            auto_restart: false,
            extra_flags: Vec::new(),
        }
    }
}

/// KV cache quantization type for llama-server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(non_camel_case_types)]
pub enum KvCacheType {
    F32,
    F16,
    #[serde(rename = "bf16")]
    BF16,
    #[serde(rename = "q8_0")]
    Q8_0,
    #[serde(rename = "q4_0")]
    Q4_0,
    #[serde(rename = "q4_1")]
    Q4_1,
    #[serde(rename = "iq4_nl")]
    IQ4_NL,
    #[serde(rename = "q5_0")]
    Q5_0,
    #[serde(rename = "q5_1")]
    Q5_1,
}

impl KvCacheType {
    /// Return the llama-server CLI flag value for this cache type.
    pub fn as_flag(&self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F16 => "f16",
            Self::BF16 => "bf16",
            Self::Q8_0 => "q8_0",
            Self::Q4_0 => "q4_0",
            Self::Q4_1 => "q4_1",
            Self::IQ4_NL => "iq4_nl",
            Self::Q5_0 => "q5_0",
            Self::Q5_1 => "q5_1",
        }
    }
}

/// Model capabilities — used by the routing rules engine for model selection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCapabilities {
    /// Maximum context window in tokens.
    pub max_context_tokens: u64,
    /// Whether the provider supports streaming responses.
    pub supports_streaming: bool,
    /// Whether the provider supports tool use / function calling.
    pub supports_tool_use: bool,
    /// Cost per 1M input tokens (USD). 0.0 for local/server models.
    pub cost_per_1m_input: f64,
    /// Cost per 1M output tokens (USD). 0.0 for local/server models.
    pub cost_per_1m_output: f64,
    /// Free-form strength tags for routing (e.g. `["code", "fast", "large-context"]`).
    pub strength_tags: Vec<String>,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            max_context_tokens: 32768,
            supports_streaming: true,
            supports_tool_use: false,
            cost_per_1m_input: 0.0,
            cost_per_1m_output: 0.0,
            strength_tags: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfigError
// ---------------------------------------------------------------------------

/// Errors that can occur when loading or saving the config file.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// An I/O error occurred while reading or writing the config file.
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The config file exists but could not be parsed as valid TOML.
    #[error("config parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    /// The config could not be serialized to TOML (should not happen in
    /// practice with well-formed structs, but reported rather than panicked).
    #[error("config serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    /// The `directories` crate returned `None` for the config directory.
    /// Occurs on systems where the platform config path is not determinable.
    #[error("could not determine platform config directory for com.abyssal.rift")]
    NoConfigDir,
}

// ---------------------------------------------------------------------------
// Config file path resolution
// ---------------------------------------------------------------------------

/// Resolve the platform config directory path for Rift using the `directories`
/// crate.
///
/// Returns `Err(ConfigError::NoConfigDir)` when the platform provides no
/// determinable config path.
fn config_file_path() -> Result<PathBuf, ConfigError> {
    let dirs =
        directories::ProjectDirs::from("com", "abyssal", "rift").ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().join("config.toml"))
}

// ---------------------------------------------------------------------------
// load_config / load_config_at
// ---------------------------------------------------------------------------

/// Load the Rift config from the platform config directory.
///
/// Returns [`RiftConfig::default()`] when the file does not yet exist (first
/// launch) — this is **not** an error. Parse errors return
/// [`ConfigError::TomlParse`].
///
/// # Errors
///
/// - [`ConfigError::NoConfigDir`] — platform config path unavailable.
/// - [`ConfigError::Io`] — read error (other than file-not-found).
/// - [`ConfigError::TomlParse`] — file exists but contains invalid TOML.
pub fn load_config() -> Result<RiftConfig, ConfigError> {
    let path = config_file_path()?;
    load_config_at(&path)
}

/// Inner, path-explicit variant of [`load_config`] used by unit tests that
/// must not depend on the `directories` crate's platform resolution.
///
/// Same semantics: returns `RiftConfig::default()` when `path` does not exist.
pub fn load_config_at(path: &Path) -> Result<RiftConfig, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let cfg: RiftConfig = toml::from_str(&text)?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // First launch — return defaults without error.
            Ok(RiftConfig::default())
        }
        Err(e) => Err(ConfigError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// save_config / save_config_at
// ---------------------------------------------------------------------------

/// Serialize `cfg` and atomically write it to the platform config directory.
///
/// Creates the parent directory if it does not yet exist. Uses a temp file +
/// rename to avoid torn writes.
///
/// # Errors
///
/// - [`ConfigError::NoConfigDir`] — platform config path unavailable.
/// - [`ConfigError::TomlSerialize`] — serialization failed.
/// - [`ConfigError::Io`] — directory creation, write, or rename failed.
pub fn save_config(cfg: &RiftConfig) -> Result<(), ConfigError> {
    let path = config_file_path()?;
    save_config_at(cfg, &path)
}

/// Inner, path-explicit variant of [`save_config`] used by unit tests.
pub fn save_config_at(cfg: &RiftConfig, path: &Path) -> Result<(), ConfigError> {
    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let toml_str = toml::to_string_pretty(cfg)?;

    // Atomic write: write to a temp file, then rename.
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &toml_str)?;
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (T16–T24)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // T16 — default_config_round_trips
    //
    // RiftConfig::default() must survive a TOML serialise → deserialise round
    // trip with all fields equal. Catches #[serde(default)] annotation drift.
    #[test]
    fn default_config_round_trips() {
        let original = RiftConfig::default();
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: RiftConfig = toml::from_str(&toml_str).expect("deserialize");

        // Structural equality for primitive fields.
        assert_eq!(back.fs.ignore_globs, original.fs.ignore_globs);
        assert_eq!(back.fs.max_depth, original.fs.max_depth);
        assert!(back.projects.is_empty());
        assert!(back.cockpit.detached_pos.is_none());
        // Phase 8.7 — IndexConfig default round-trips with sync_mode = Live.
        assert_eq!(back.index.sync_mode, SyncMode::Live);
        // D-018 groundwork — TerminalConfig defaults round-trip.
        assert_eq!(back.terminal.shell, ShellPref::Auto);
        assert_eq!(back.terminal.font_size, TERMINAL_DEFAULT_FONT_SIZE);
        assert!((back.terminal.line_height - TERMINAL_DEFAULT_LINE_HEIGHT).abs() < f32::EPSILON);
        assert_eq!(back.terminal.scrollback, TERMINAL_DEFAULT_SCROLLBACK);
        assert!(back.terminal.lanes_enabled);
        // D-020 — TreeConfig defaults round-trip.
        assert!(!back.tree.heatmap_enabled);
        assert_eq!(back.tree.heatmap_window_minutes, 15);
    }

    // D-018 groundwork — ShellPref Custom variant round-trips through both
    // TOML (config file) and serde_json (Tauri command boundary).
    #[test]
    fn shell_pref_custom_round_trips_json() {
        let original = ShellPref::Custom(PathBuf::from("/usr/local/bin/fish"));
        let json = serde_json::to_string(&original).expect("serialize");
        // Adjacently-tagged shape: {"kind":"custom","path":"/usr/local/bin/fish"}
        assert!(json.contains("\"kind\":\"custom\""));
        assert!(json.contains("\"path\":\"/usr/local/bin/fish\""));
        let back: ShellPref = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, original);
    }

    #[test]
    fn shell_pref_named_variants_round_trip_json() {
        for variant in [
            ShellPref::Auto,
            ShellPref::Pwsh,
            ShellPref::Powershell,
            ShellPref::Cmd,
            ShellPref::Bash,
            ShellPref::Zsh,
            ShellPref::Sh,
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let back: ShellPref = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant);
        }
    }

    // T17 — load_config_returns_default_on_missing_file
    //
    // When the config file does not exist, load_config_at must return
    // RiftConfig::default() rather than an error. This is the first-launch
    // path; surfacing an error here would break startup.
    #[test]
    fn load_config_returns_default_on_missing_file() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("nonexistent.toml");

        let cfg = load_config_at(&path).expect("should succeed with defaults");

        assert!(
            cfg.projects.is_empty(),
            "projects should be empty on first launch"
        );
        assert_eq!(
            cfg.fs.ignore_globs,
            DEFAULT_IGNORE_GLOBS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
            "ignore_globs should be the defaults"
        );
        assert_eq!(cfg.fs.max_depth, FS_CONFIG_DEFAULT_MAX_DEPTH);
    }

    // T18 — save_then_load_round_trip
    //
    // A modified config written with save_config_at must come back identical
    // when read with load_config_at.
    #[test]
    fn save_then_load_round_trip() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let mut cfg = RiftConfig::default();
        cfg.projects.push(ProjectEntry {
            name: "my-project".to_string(),
            path: PathBuf::from("/home/user/my-project"),
            last_used_ms: 1_700_000_000_000,
        });
        cfg.fs.ignore_globs = vec!["*.log".to_string(), ".git/**".to_string()];
        cfg.fs.max_depth = 4;

        save_config_at(&cfg, &path).expect("save should succeed");
        let back = load_config_at(&path).expect("load should succeed");

        assert_eq!(back.projects.len(), 1);
        assert_eq!(back.projects[0].name, "my-project");
        assert_eq!(back.projects[0].last_used_ms, 1_700_000_000_000);
        assert_eq!(
            back.projects[0].path,
            PathBuf::from("/home/user/my-project")
        );
        assert_eq!(back.fs.ignore_globs, cfg.fs.ignore_globs);
        assert_eq!(back.fs.max_depth, 4);
        assert!(back.cockpit.detached_pos.is_none());
    }

    // T19 — project_entry_lru_eviction
    //
    // Inserting 11 entries and capping to 10 must evict the entry with the
    // smallest last_used_ms. Validates the LRU cap logic exercised in
    // project_swap.
    #[test]
    fn project_entry_lru_eviction() {
        let mut entries: Vec<ProjectEntry> = (0u64..11)
            .map(|i| ProjectEntry {
                name: format!("proj-{i}"),
                path: PathBuf::from(format!("/proj/{i}")),
                last_used_ms: i * 1_000, // proj-0 has the smallest timestamp
            })
            .collect();

        // Sort descending and cap to 10 — same logic as project_swap.
        entries.sort_by_key(|e| std::cmp::Reverse(e.last_used_ms));
        entries.truncate(10);

        assert_eq!(entries.len(), 10, "must have exactly 10 entries after cap");

        // proj-0 (last_used_ms = 0) must be gone.
        assert!(
            entries.iter().all(|e| e.name != "proj-0"),
            "proj-0 (oldest) must be evicted; remaining: {:?}",
            entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        // proj-10 (last_used_ms = 10_000) must be first (most recent).
        assert_eq!(entries[0].name, "proj-10");
    }

    // T20 — default_ignore_globs contain required patterns
    #[test]
    fn default_ignore_globs_contain_required_patterns() {
        const REQUIRED: &[&str] = &[
            ".git",
            ".git/**",
            "node_modules",
            "node_modules/**",
            "target",
            "target/**",
            "dist",
            "dist/**",
            "*.log",
        ];

        for pattern in REQUIRED {
            assert!(
                DEFAULT_IGNORE_GLOBS.contains(pattern),
                "'{pattern}' must be in DEFAULT_IGNORE_GLOBS"
            );
        }
    }

    // T21 — parse_config_without_index_section_uses_defaults
    //
    // A config TOML with no [index] section must parse successfully and
    // produce IndexConfig::default() for the index field. Validates the
    // R3 additivity invariant: existing configs without [index] do not break.
    #[test]
    fn parse_config_without_index_section_uses_defaults() {
        let toml_str = r#"
[fs]
max_depth = 4
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert!(
            cfg.index.ignore_globs.is_empty(),
            "ignore_globs should default to empty"
        );
        assert_eq!(
            cfg.index.sync_mode,
            SyncMode::Live,
            "sync_mode should default to Live"
        );
        assert!(
            cfg.index.camera_transform.is_none(),
            "camera_transform should default to None"
        );
        assert!(
            cfg.index.node_positions.is_none(),
            "node_positions should default to None"
        );
    }

    // T22 — parse_config_with_partial_index_section
    //
    // A config TOML with [index] containing only ignore_globs must parse
    // without error; the absent sync_mode must default to Live.
    #[test]
    fn parse_config_with_partial_index_section() {
        let toml_str = r#"
[index]
ignore_globs = ["*.bak"]
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert_eq!(
            cfg.index.ignore_globs,
            vec!["*.bak".to_string()],
            "ignore_globs should contain the specified pattern"
        );
        assert_eq!(
            cfg.index.sync_mode,
            SyncMode::Live,
            "absent sync_mode should default to Live"
        );
    }

    // T23 — parse_config_with_unknown_sync_mode_variant
    //
    // A sync_mode string that no current variant matches must parse without
    // error and produce SyncMode::Unknown (the #[serde(other)] catch-all).
    // Validates forward-compat: a newer Rift's config read by older Rift
    // does not panic or error on unrecognized variants.
    #[test]
    fn parse_config_with_unknown_sync_mode_variant() {
        let toml_str = r#"
[index]
sync_mode = "future_variant_xyz"
"#;
        let cfg: RiftConfig =
            toml::from_str(toml_str).expect("parse must not error on unknown variant");
        assert_eq!(
            cfg.index.sync_mode,
            SyncMode::Unknown,
            "unrecognized sync_mode string must map to SyncMode::Unknown"
        );
    }

    // T24 — index_config_default_round_trips
    //
    // IndexConfig::default() must survive a TOML serialise → deserialise round
    // trip with all fields structurally equal. Validates that the new [index]
    // section's serde annotations are correct and SyncMode::Live serializes
    // as expected (lowercase string).
    #[test]
    fn index_config_default_round_trips() {
        let original = IndexConfig::default();
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: IndexConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(
            back.sync_mode,
            SyncMode::Live,
            "sync_mode must round-trip as Live"
        );
        assert!(
            back.ignore_globs.is_empty(),
            "ignore_globs must round-trip as empty"
        );
        assert!(
            back.camera_transform.is_none(),
            "camera_transform must round-trip as None"
        );
        assert!(
            back.node_positions.is_none(),
            "node_positions must round-trip as None"
        );
    }

    // T25 — parse_config_without_session_section_uses_defaults
    //
    // A config TOML with no [session] section must parse successfully and
    // produce SessionConfig::default(). Validates the additive-versioning
    // invariant: existing configs without [session] do not break.
    #[test]
    fn parse_config_without_session_section_uses_defaults() {
        let toml_str = r#"
[fs]
max_depth = 4
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert!(
            cfg.session.enabled,
            "session.enabled should default to true"
        );
        assert_eq!(
            cfg.session.retention_days, 7,
            "session.retention_days should default to 7"
        );
        assert_eq!(
            cfg.session.max_file_size_mb, 100,
            "session.max_file_size_mb should default to 100"
        );
    }

    // T26 — session_config_round_trips
    #[test]
    fn session_config_round_trips() {
        let original = SessionConfig::default();
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: SessionConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back.enabled, original.enabled);
        assert_eq!(back.retention_days, original.retention_days);
        assert_eq!(back.max_file_size_mb, original.max_file_size_mb);
    }

    // T27 — parse_config_without_notif_filters_section_uses_defaults
    #[test]
    fn parse_config_without_notif_filters_section_uses_defaults() {
        let toml_str = r#"
[fs]
max_depth = 4
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert_eq!(
            cfg.notif_filters.default_threshold,
            SeverityLevel::Info,
            "default_threshold should default to Info"
        );
        assert!(
            cfg.notif_filters.per_tab.is_empty(),
            "per_tab should default to empty"
        );
    }

    // T28 — notif_filter_config_round_trips
    #[test]
    fn notif_filter_config_round_trips() {
        let mut original = NotifFilterConfig {
            default_threshold: SeverityLevel::Warn,
            ..Default::default()
        };
        original
            .per_tab
            .insert("bustail".to_string(), SeverityLevel::Debug);
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: NotifFilterConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back.default_threshold, SeverityLevel::Warn);
        assert_eq!(back.per_tab.get("bustail"), Some(&SeverityLevel::Debug));
    }

    // T29 — severity_level_unknown_variant_forward_compat
    #[test]
    fn severity_level_unknown_variant_forward_compat() {
        let toml_str = r#"
[notif_filters]
default_threshold = "future_level_xyz"
"#;
        let cfg: RiftConfig =
            toml::from_str(toml_str).expect("parse must not error on unknown variant");
        assert_eq!(cfg.notif_filters.default_threshold, SeverityLevel::Unknown);
    }

    // T30 — parse_config_without_tree_section_uses_defaults
    //
    // A config TOML with no [tree] section must parse successfully and
    // produce TreeConfig::default(). Validates the additive-versioning
    // invariant: existing configs without [tree] do not break.
    #[test]
    fn parse_config_without_tree_section_uses_defaults() {
        let toml_str = r#"
[fs]
max_depth = 4
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert!(
            !cfg.tree.heatmap_enabled,
            "tree.heatmap_enabled should default to false"
        );
        assert_eq!(
            cfg.tree.heatmap_window_minutes, 15,
            "tree.heatmap_window_minutes should default to 15"
        );
    }

    // T31a — parse_config_without_alerts_section_uses_defaults
    #[test]
    fn parse_config_without_alerts_section_uses_defaults() {
        let toml_str = r#"
[fs]
max_depth = 4
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert!(
            cfg.alerts.rules.is_empty(),
            "alerts.rules should default to empty"
        );
    }

    // T31b — alerts_config_round_trips
    #[test]
    fn alerts_config_round_trips() {
        let original = AlertsConfig {
            rules: vec![AlertRule {
                id: "test-001".to_string(),
                tab_id: "errors".to_string(),
                severity: SeverityLevel::Error,
                threshold: 3,
                window_secs: 10,
                action: AlertAction::Flash,
                enabled: true,
            }],
        };
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: AlertsConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back.rules.len(), 1);
        assert_eq!(back.rules[0].id, "test-001");
        assert_eq!(back.rules[0].tab_id, "errors");
        assert_eq!(back.rules[0].threshold, 3);
        assert_eq!(back.rules[0].action, AlertAction::Flash);
    }

    // T31 — tree_config_round_trips
    #[test]
    fn tree_config_round_trips() {
        let original = TreeConfig {
            heatmap_enabled: true,
            heatmap_window_minutes: 60,
        };
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: TreeConfig = toml::from_str(&toml_str).expect("deserialize");
        assert!(back.heatmap_enabled);
        assert_eq!(back.heatmap_window_minutes, 60);
    }

    // T32 — parse_config_without_ensemble_section_uses_defaults
    //
    // Existing configs without [ensemble] must parse and produce
    // EnsembleConfig::default(). Zero impact on single-model users.
    #[test]
    fn parse_config_without_ensemble_section_uses_defaults() {
        let toml_str = r#"
[fs]
max_depth = 4
"#;
        let cfg: RiftConfig = toml::from_str(toml_str).expect("parse should succeed");
        assert!(
            !cfg.ensemble.enabled,
            "ensemble.enabled should default to false"
        );
        assert_eq!(
            cfg.ensemble.active_profile,
            RoutingProfile::Manual,
            "active_profile should default to Manual"
        );
        assert!(
            cfg.ensemble.default_model.is_empty(),
            "default_model should default to empty"
        );
        assert!(
            cfg.ensemble.models.is_empty(),
            "models should default to empty"
        );
    }

    // T33 — ensemble_config_round_trips
    #[test]
    fn ensemble_config_round_trips() {
        let original = EnsembleConfig {
            enabled: true,
            active_profile: RoutingProfile::Balanced,
            default_model: "local-gemma".to_string(),
            models: vec![ModelConfig {
                id: "local-gemma".to_string(),
                enabled: true,
                display_name: "Gemma 4 27B".to_string(),
                provider: ProviderType::LlamaServer,
                model_identifier: "gemma-4-27b-it-Q4_K_M.gguf".to_string(),
                hosting: HostingMode::Local {
                    process_config: LlamaServerConfig {
                        model_path: PathBuf::from("C:\\Models\\gemma-4-27b-it-Q4_K_M.gguf"),
                        flash_attention: true,
                        ctx_size: 32768,
                        cache_type_k: KvCacheType::Q8_0,
                        cache_type_v: KvCacheType::Q8_0,
                        n_gpu_layers: 99,
                        cpu_moe: false,
                        n_cpu_moe: None,
                        cache_ram: None,
                        threads: None,
                        parallel: 1,
                        port: 8081,
                        cuda_visible_devices: Some("0".to_string()),
                        auto_start: true,
                        auto_restart: false,
                        extra_flags: vec![],
                    },
                },
                endpoint: "http://127.0.0.1:8081".to_string(),
                api_key_ref: None,
                color: "--model-local".to_string(),
                short_id: "LOC".to_string(),
                capabilities: ModelCapabilities {
                    max_context_tokens: 32768,
                    supports_streaming: true,
                    supports_tool_use: false,
                    cost_per_1m_input: 0.0,
                    cost_per_1m_output: 0.0,
                    strength_tags: vec!["fast".to_string(), "private".to_string()],
                },
            }],
            classifier_model_id: None,
        };
        let toml_str = toml::to_string_pretty(&original).expect("serialize");
        let back: EnsembleConfig = toml::from_str(&toml_str).expect("deserialize");
        assert!(back.enabled);
        assert_eq!(back.active_profile, RoutingProfile::Balanced);
        assert_eq!(back.default_model, "local-gemma");
        assert_eq!(back.models.len(), 1);
        assert_eq!(back.models[0].id, "local-gemma");
        assert_eq!(back.models[0].provider, ProviderType::LlamaServer);
        assert_eq!(back.models[0].short_id, "LOC");
    }

    // T34 — kv_cache_type_round_trips
    #[test]
    fn kv_cache_type_round_trips() {
        for variant in [
            KvCacheType::F32,
            KvCacheType::F16,
            KvCacheType::BF16,
            KvCacheType::Q8_0,
            KvCacheType::Q4_0,
            KvCacheType::Q4_1,
            KvCacheType::IQ4_NL,
            KvCacheType::Q5_0,
            KvCacheType::Q5_1,
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let back: KvCacheType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant, "round-trip failed for {variant:?}");
        }
    }

    // T35 — routing_profile_round_trips
    #[test]
    fn routing_profile_round_trips() {
        for variant in [
            RoutingProfile::Manual,
            RoutingProfile::CostOptimized,
            RoutingProfile::QualityFirst,
            RoutingProfile::Balanced,
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let back: RoutingProfile = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant, "round-trip failed for {variant:?}");
        }
    }

    // T36 — hosting_mode_cloud_round_trips
    #[test]
    fn hosting_mode_cloud_round_trips() {
        let original = HostingMode::Cloud;
        let json = serde_json::to_string(&original).expect("serialize");
        assert!(json.contains("\"mode\":\"cloud\""));
        let back: HostingMode = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(back, HostingMode::Cloud));
    }

    // T37 — hosting_mode_remote_round_trips
    #[test]
    fn hosting_mode_remote_round_trips() {
        let original = HostingMode::Remote {
            health_check_interval_secs: 30,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let back: HostingMode = serde_json::from_str(&json).expect("deserialize");
        match back {
            HostingMode::Remote {
                health_check_interval_secs,
            } => assert_eq!(health_check_interval_secs, 30),
            _ => panic!("expected Remote variant"),
        }
    }

    // T38 — llama_server_config_defaults
    #[test]
    fn llama_server_config_defaults() {
        let cfg = LlamaServerConfig::default();
        assert!(cfg.flash_attention);
        assert_eq!(cfg.ctx_size, 32768);
        assert_eq!(cfg.cache_type_k, KvCacheType::Q8_0);
        assert_eq!(cfg.cache_type_v, KvCacheType::Q8_0);
        assert_eq!(cfg.n_gpu_layers, 99);
        assert!(cfg.threads.is_none());
        assert_eq!(cfg.parallel, 1);
        assert!(!cfg.auto_start);
    }

    // T39 — kv_cache_type_as_flag
    #[test]
    fn kv_cache_type_as_flag() {
        assert_eq!(KvCacheType::Q8_0.as_flag(), "q8_0");
        assert_eq!(KvCacheType::F16.as_flag(), "f16");
        assert_eq!(KvCacheType::BF16.as_flag(), "bf16");
        assert_eq!(KvCacheType::IQ4_NL.as_flag(), "iq4_nl");
    }
}
