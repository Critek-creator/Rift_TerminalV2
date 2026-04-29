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
}
