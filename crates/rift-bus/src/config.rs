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
    ".git/**",
    "node_modules/**",
    "target/**",
    "dist/**",
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
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 7,
            max_file_size_mb: 100,
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
/// if the file is absent. A stale file from a crashed prior Rift is still
/// returned — the bridge's `IpcClient::connect` is the source of truth for
/// liveness.
pub fn load_mcp_socket() -> Result<Option<String>, ConfigError> {
    let path = mcp_socket_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// Atomically write the current host's socket name to the discovery file.
pub fn save_mcp_socket(socket_name: &str) -> Result<(), ConfigError> {
    let path = mcp_socket_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, socket_name)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remove the discovery file. No-op if absent. Called from the host's
/// `ExitRequested` handler so the next Rift launch starts with a clean
/// slate (and so a stopped host can't masquerade as live).
pub fn clear_mcp_socket() -> Result<(), ConfigError> {
    let path = mcp_socket_path()?;
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
    pub show_effort: bool,
    pub show_model: bool,
    pub show_ctx: bool,
    pub show_session_use: bool,
    pub show_week: bool,
    /// Optional per-segment color overrides (CSS color strings).
    /// Keys: "dir", "git", "repo", "session", "skill", "effort",
    /// "model", "ctx", "session_use", "week".
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
            show_effort: true,
            show_model: true,
            show_ctx: true,
            show_session_use: true,
            show_week: true,
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

    // T20 — default_ignore_globs_match_legacy_fs_module
    //
    // DEFAULT_IGNORE_GLOBS must contain exactly 5 patterns and each must be
    // one of the five patterns that were previously hardcoded in fs.rs.
    // This test catches drift: if someone adds a pattern here without
    // reviewing fs.rs (or vice versa now that fs.rs imports from here).
    #[test]
    fn default_ignore_globs_match_legacy_fs_module() {
        const EXPECTED: &[&str] = &[
            ".git/**",
            "node_modules/**",
            "target/**",
            "dist/**",
            "*.log",
        ];

        assert_eq!(
            DEFAULT_IGNORE_GLOBS.len(),
            5,
            "DEFAULT_IGNORE_GLOBS must have exactly 5 entries; got {}",
            DEFAULT_IGNORE_GLOBS.len()
        );

        for pattern in EXPECTED {
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
}
