mod capture;
mod cockpit_window;
mod git_status;
#[cfg(feature = "index")]
mod index_bridge;
mod llm_commands;
#[cfg(not(feature = "index"))]
mod index_bridge {
    #[tauri::command]
    pub fn index_list_nodes(
        _domain: Option<String>,
        _floor: Option<String>,
        _tags: Option<String>,
    ) -> Result<(), String> {
        Err("Index integration not available (built without 'index' feature)".into())
    }
    #[tauri::command]
    pub fn index_search_nodes(_query: String, _limit: Option<usize>) -> Result<(), String> {
        Err("Index integration not available (built without 'index' feature)".into())
    }
    #[tauri::command]
    pub fn index_get_node(_id: String) -> Result<(), String> {
        Err("Index integration not available (built without 'index' feature)".into())
    }
    #[tauri::command]
    pub fn index_get_connections(_id: String, _depth: Option<u32>) -> Result<(), String> {
        Err("Index integration not available (built without 'index' feature)".into())
    }
    #[tauri::command]
    pub fn index_get_stats() -> Result<(), String> {
        Err("Index integration not available (built without 'index' feature)".into())
    }
}
mod command_history;
mod file_preview;
mod health_collector;
mod integrations;
mod mcp_host;
mod notif_window;
mod profiles;
mod todo_scan;

// Rift Terminal v2 — Tauri host crate.
//
// Phase 1: PTY surface.
//   * `pty_start`  — spawn a default-shell PTY, return a session id, drain
//                    its byte stream onto a `tauri::ipc::Channel<Vec<u8>>`.
//   * `pty_write`  — write bytes to the session's stdin.
//   * `pty_resize` — resize the session's TTY.
//   * `pty_kill`   — terminate the session's child.
//
// Phase 4.3: IPC server.
//   * Manages a single `RiftBus` for the process.
//   * Spawns an `IpcServer` on a process-unique socket name during
//     `setup`. External translators (Aegis, MCP shims, future SSH-headless
//     mobile client) connect via that socket.
//   * `rift_bus_status` reports subscriber count + replay length + socket
//     name for diagnostics.
//
// Phase 6.7: Config + ProjectRoot + WatcherRegistry.
//   * `config_get`    — load RiftConfig (defaults on first launch).
//   * `config_save`   — persist RiftConfig atomically.
//   * `project_swap`  — swap the active project root + live watcher.
//   * `WatcherRegistry` — drop-safe single-watcher state; replaces the
//     previous `app.manage(watcher)` pattern.
//   * `ProjectRoot`   — Tauri-managed canonical project root path, read by
//     `fs_tree`, `fs_read_text`, and `fs_write_text`. Consolidates the
//     duplicate `env::current_dir()` calls flagged by Validator 6.2.
//
// Real-time mechanism per `decisions/§10.15_real-time_update_mechanism.md`:
// Tier 1 = `Channel<T>` for in-process high-throughput streams (PTY
// output) + Tauri `app.emit` for low-frequency notifications. Tier 2 =
// `RiftBus` + `IpcServer` for cross-process translator integration.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows `CREATE_NO_WINDOW` — see the matching constant in
/// `crates/rift-bus/src/translators/status.rs::CREATE_NO_WINDOW` and
/// `crates/rift-bus/src/translators/status.rs::git_cmd` for the full rationale.
/// Applied to every `Command::spawn` site reachable from this crate.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A spawned [`tauri::async_runtime::JoinHandle`] wrapped in an `Option` so
/// it can be taken and aborted exactly once.
type DrainHandle = tauri::async_runtime::JoinHandle<()>;

#[cfg(feature = "aegis-private")]
use rift_aegis::probe as aegis_probe;

use rift_bus::translators::llm_process::ProcessManager;
use rift_bus::{
    build_tree, load_config, prepare_lane_prelude, publish_command, publish_error, read_text,
    save_config, spawn_fs_watcher, spawn_session_logger, spawn_status_translator,
    spawn_vault_walker, write_text, Category, CommandBuffer, Envelope, FsWatcher, IpcServer, Lane,
    LaneClassifier, RiftBus, RiftConfig, SentinelEvent, ShellPref, SubscribeFilter, TreeNode,
    FS_TREE_DEFAULT_MAX_DEPTH,
};
use rift_core::process::is_claude_descendant;
use rift_core::pty::{PtyControl, PtyDims, PtyOptions, PtySession};
use rift_core::shell::{resolve_auto_shell, resolve_custom_shell, resolve_named_shell};
use serde::Serialize;
use serde_json::json;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tokio::sync::{oneshot, Notify};

/// App-wide shutdown notifier. Spawned long-lived translator tasks
/// (`spawn_status_translator` for now; future translators on the same pattern)
/// `tokio::select!` against this notify so they exit promptly when the main
/// window closes — without the signal, the 5-second status tick keeps spawning
/// `git.exe` after window close, painting visible terminal flashes until the
/// process is force-killed via Task Manager (the symptom this module-level
/// constant exists to fix; see `CREATE_NO_WINDOW` above).
#[derive(Clone, Default)]
struct ShutdownNotify(Arc<Notify>);

impl ShutdownNotify {
    fn handle(&self) -> Arc<Notify> {
        self.0.clone()
    }

    fn signal(&self) {
        self.0.notify_waiters();
    }
}

/// Signal from the frontend that the first terminal has mounted and started
/// its PTY. The vault walker awaits this instead of a fixed sleep, eliminating
/// the timing race that caused blank terminals when WebView2 setup was slow.
#[derive(Clone, Default)]
struct TerminalReady(Arc<Notify>);

// ---------------------------------------------------------------------------
// PTY registry (Phase 1)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PtyRegistryInner {
    sessions: Mutex<HashMap<u32, PtyControl>>,
    /// Outer drain task handles keyed by session id. Aborted on kill or
    /// natural exit so no orphaned tasks outlive their PTY session.
    drain_handles: Mutex<HashMap<u32, DrainHandle>>,
    next_id: AtomicU32,
}

/// Tracks live PTY sessions. Cheap to clone (`Arc` inside).
#[derive(Clone, Default)]
pub(crate) struct PtyRegistry(Arc<PtyRegistryInner>);

impl PtyRegistry {
    fn insert(&self, control: PtyControl) -> u32 {
        let id = self.0.next_id.fetch_add(1, Ordering::SeqCst);
        self.0.sessions.lock().insert(id, control);
        id
    }

    /// Store the outer drain task handle alongside the session.
    ///
    /// Must be called once, after [`insert`], with the handle returned by
    /// `tauri::async_runtime::spawn`. Overwrites any existing handle (guards
    /// against double-registration; in practice never occurs because sessions
    /// have unique ids).
    fn insert_drain(&self, id: u32, handle: DrainHandle) {
        self.0.drain_handles.lock().insert(id, handle);
    }

    fn get(&self, id: u32) -> Option<PtyControl> {
        self.0.sessions.lock().get(&id).cloned()
    }

    /// Remove the session and abort its drain task (if still running).
    fn remove(&self, id: u32) {
        self.0.sessions.lock().remove(&id);
        if let Some(handle) = self.0.drain_handles.lock().remove(&id) {
            handle.abort();
        }
    }

    /// Snapshot of currently-tracked sessions: `(id, alive)` pairs sorted by
    /// id ascending. Used by the MCP `pty_list` tool (D-014 Phase B).
    pub(crate) fn list(&self) -> Vec<(u32, bool)> {
        let guard = self.0.sessions.lock();
        let mut entries: Vec<(u32, bool)> = guard
            .iter()
            .map(|(id, ctl)| (*id, ctl.is_alive()))
            .collect();
        entries.sort_by_key(|(id, _)| *id);
        entries
    }

    /// Phase 8.7q.4 — kill every tracked session + abort every drain handle.
    /// Used by the WebView reload-cleanup path (`rift_reset_for_reload`)
    /// to close orphan PTY sessions whose `Channel<Vec<u8>>` callback ids
    /// died with the prior page load. Without this, Rust drain tasks keep
    /// pumping bytes into dead callbacks and Tauri spams the console with
    /// "Couldn't find callback id" warnings.
    ///
    /// Returns the number of sessions killed.
    fn kill_all(&self) -> usize {
        // Collect ids first so we can release the lock before invoking
        // `remove`, which re-acquires it.
        let ids: Vec<u32> = self.0.sessions.lock().keys().copied().collect();
        let count = ids.len();
        for id in ids {
            // Best-effort — kill failures are non-fatal (session may be
            // dead already); we just need the drain handle aborted.
            if let Some(ctl) = self.get(id) {
                let _ = ctl.kill();
            }
            self.remove(id); // aborts drain handle + removes from map
        }
        count
    }
}

// ---------------------------------------------------------------------------
// Command buffer registry (Phase 5.4 — commands translator)
// ---------------------------------------------------------------------------

/// Tracks per-session [`CommandBuffer`] instances. Keyed by the same session
/// id used in [`PtyRegistry`]. std Mutex — no `.await` crosses the lock.
#[derive(Default)]
struct CommandBufferRegistry {
    buffers: Mutex<HashMap<u32, CommandBuffer>>,
}

impl CommandBufferRegistry {
    fn insert(&self, id: u32) {
        self.buffers.lock().insert(id, CommandBuffer::new());
    }

    /// Feed bytes into the session's buffer. Returns completed
    /// `(command_string, raw_len)` pairs; empty if the session is not found.
    fn feed(&self, id: u32, bytes: &[u8]) -> Vec<(String, usize)> {
        let mut guard = self.buffers.lock();
        match guard.get_mut(&id) {
            Some(buf) => buf.feed(bytes),
            None => Vec::new(),
        }
    }

    fn remove(&self, id: u32) {
        self.buffers.lock().remove(&id);
    }
}

// ---------------------------------------------------------------------------
// Bus + IPC state (Phase 4.3)
// ---------------------------------------------------------------------------

const IPC_SOCKET_PREFIX: &str = "rift-v2";

/// Holds the IPC server handle alongside its socket name, so commands can
/// report the name and a future shutdown command can pull the server out.
/// `server` is intentionally held — dropping the [`IpcServer`] would
/// silently let the accept loop continue but lose the shutdown handle.
struct BusIpcState {
    socket_name: String,
    #[allow(dead_code)] // held to keep the IpcServer alive for the process lifetime
    server: Mutex<Option<IpcServer>>,
}

// ---------------------------------------------------------------------------
// Bus subscription registry (Phase 5)
// ---------------------------------------------------------------------------

/// Tracks frontend bus subscriptions. Each entry holds a one-shot sender
/// for cooperative cancellation AND the drain task's `JoinHandle` for
/// unconditional abort on unsubscribe.
#[derive(Default)]
struct BusSubscriptionRegistry {
    next_id: AtomicU64,
    subs: Mutex<HashMap<u64, (oneshot::Sender<()>, DrainHandle)>>,
}

impl BusSubscriptionRegistry {
    /// Unsubscribe: send the cooperative stop signal AND abort the task handle.
    ///
    /// Abort is a safety net — if the task is blocked or the close_rx side
    /// was already dropped, `abort()` guarantees the task stops.
    fn remove(&self, id: u64) {
        if let Some((tx, handle)) = self.subs.lock().remove(&id) {
            let _ = tx.send(());
            handle.abort();
        }
    }

    /// Phase 8.7q.4 — drain every WebView-Channel-based bus subscription.
    /// Same rationale as [`PtyRegistry::kill_all`]: after a hard-refresh,
    /// every `Channel<Envelope>` handed to `bus_subscribe` is orphaned
    /// (the JS callback id is gone) but Rust's drain task keeps pushing
    /// envelopes through it, generating "Couldn't find callback id"
    /// console flood. Wiped on every `rift_reset_for_reload` invocation.
    ///
    /// Note: in-process Rust subscribers are NOT in this registry — they
    /// hold their own `Subscription` handles via `bus.subscribe()` and
    /// are unaffected. Only WebView-side Channel subscriptions are dropped.
    ///
    /// Returns the number of subscriptions cleaned up.
    fn remove_all(&self) -> usize {
        // Drain into a vec under the lock to avoid the lock+abort race.
        let drained: Vec<(oneshot::Sender<()>, DrainHandle)> = {
            let mut guard = self.subs.lock();
            guard.drain().map(|(_, v)| v).collect()
        };
        let count = drained.len();
        for (tx, handle) in drained {
            let _ = tx.send(());
            handle.abort();
        }
        count
    }
}

// ---------------------------------------------------------------------------
// WatcherRegistry (Phase 6.7)
//
// Replaces the old `app.manage(watcher)` pattern. Holds at most one live
// FsWatcher; `replace` drops the old watcher cleanly before installing the
// new one (pr003 gotcha: `command-buffer-leak-on-natural-pty-exit` applied
// to watchers — drop fully before mounting new).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct WatcherRegistry {
    current: Mutex<Option<FsWatcher>>,
}

impl WatcherRegistry {
    /// Install `w` as the active watcher.  Drops the previous watcher first
    /// (which closes its notify OS thread and dispatcher thread cleanly).
    /// Returns the previous watcher, if any.
    fn replace(&self, w: FsWatcher) -> Option<FsWatcher> {
        self.current.lock().replace(w)
    }

    /// Drop the current watcher, leaving the registry empty.
    fn clear(&self) {
        *self.current.lock() = None;
    }
}

// ---------------------------------------------------------------------------
// ProjectRoot (Phase 6.7)
//
// Tauri-managed canonical project root. Initialized from current_dir() at
// startup; mutated by project_swap. fs_tree / fs_read_text / fs_write_text
// read from here instead of calling current_dir() themselves — consolidates
// the duplicate env::current_dir() calls flagged by Validator 6.2.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ProjectRoot {
    pub(crate) path: Arc<Mutex<PathBuf>>,
}

impl ProjectRoot {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(Mutex::new(path)),
        }
    }

    pub(crate) fn get(&self) -> PathBuf {
        self.path.lock().clone()
    }

    fn set(&self, path: PathBuf) {
        *self.path.lock() = path;
    }
}

// ---------------------------------------------------------------------------
// CachedConfig (M9 audit fix)
//
// Avoids re-reading config from disk on every Tauri command that needs it.
// Initialized from load_config() at startup; invalidated by config_save.
// ---------------------------------------------------------------------------

pub(crate) struct CachedConfig {
    inner: Mutex<RiftConfig>,
}

impl CachedConfig {
    fn new(cfg: RiftConfig) -> Self {
        Self {
            inner: Mutex::new(cfg),
        }
    }

    fn get(&self) -> RiftConfig {
        self.inner.lock().clone()
    }

    fn set(&self, cfg: RiftConfig) {
        *self.inner.lock() = cfg;
    }
}

// ---------------------------------------------------------------------------
// ProjectTreeCache (P-02)
//
// LRU cache of per-project TreeNode snapshots. Avoids a full read_dir walk
// when switching back to a recently visited project. The frontend remains
// source of truth for its live-mutated copy; this cache only accelerates
// the initial snapshot served by fs_tree.
// ---------------------------------------------------------------------------

const TREE_CACHE_MAX: usize = 5;

struct CachedTree {
    root: TreeNode,
    built_at: Instant,
}

pub(crate) struct ProjectTreeCache {
    trees: Mutex<HashMap<PathBuf, CachedTree>>,
}

impl ProjectTreeCache {
    fn new() -> Self {
        Self {
            trees: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, path: &PathBuf) -> Option<TreeNode> {
        self.trees.lock().get(path).map(|ct| ct.root.clone())
    }

    fn put(&self, path: PathBuf, root: TreeNode) {
        let mut trees = self.trees.lock();
        if trees.len() >= TREE_CACHE_MAX && !trees.contains_key(&path) {
            if let Some(oldest) = trees
                .iter()
                .min_by_key(|(_, v)| v.built_at)
                .map(|(k, _)| k.clone())
            {
                trees.remove(&oldest);
            }
        }
        trees.insert(
            path,
            CachedTree {
                root,
                built_at: Instant::now(),
            },
        );
    }

    fn clear(&self) {
        self.trees.lock().clear();
    }
}

/// L2: Map a `Category::Hook` bus envelope to a `SentinelEvent`.
///
/// Heuristic: envelope kinds ending in `.started` or starting with `Pre`
/// map to HookStart; `.finished`/`Post` map to HookEnd. Single-shot
/// events (no clear start/end) default to HookStart (the next
/// PROMPT_START from L1 will naturally clear the Hook lane).
fn envelope_to_hook_event(env: &Envelope) -> SentinelEvent {
    let kind = &env.kind;
    let name = env
        .payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(kind)
        .to_owned();
    if kind.ends_with(".finished") || kind.starts_with("Post") {
        SentinelEvent::HookEnd { name }
    } else {
        SentinelEvent::HookStart { name }
    }
}

/// L2: Map a `Category::Aegis` bus envelope to a `SentinelEvent`.
///
/// Aegis envelopes with kind ending in `.end` or `.finished` map to
/// AegisEnd; everything else to AegisStart.
fn envelope_to_aegis_event(env: &Envelope) -> SentinelEvent {
    let kind = &env.kind;
    if kind.ends_with(".end") || kind.ends_with(".finished") {
        SentinelEvent::AegisEnd
    } else {
        SentinelEvent::AegisStart
    }
}

fn parse_category(raw: &str) -> Result<Category, String> {
    serde_json::from_value::<Category>(serde_json::Value::String(raw.to_lowercase()))
        .map_err(|e| format!("invalid category {raw:?}: {e}"))
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
struct PtyExitedEvent {
    id: u32,
    code: u32,
}

#[derive(Serialize)]
struct BusStatus {
    socket_name: String,
    subscribers: usize,
    replay_len: usize,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn phase0_check() -> &'static str {
    "rift phase 4.3 · backend reachable"
}

/// Resolve the user's [`ShellPref`] from `RiftConfig.terminal` to a concrete
/// `(PathBuf, Vec<String>)`. Falls back to [`resolve_auto_shell`] when:
///   * pref is `Auto` (the default)
///   * pref is `Unknown` (forward-compat catch-all from a newer config)
///   * a named shell (`pwsh`, `bash`, ...) isn't found on `PATH`
///   * a `Custom` path doesn't point at an existing file
///
/// The fallback is silent — the user can verify which shell launched by
/// inspecting the spawned process. A future enhancement (D-018-adjacent)
/// can publish a `terminal.shell_resolved` envelope so the UI surfaces it.
fn resolve_shell_from_pref(pref: &ShellPref) -> (std::path::PathBuf, Vec<String>) {
    let named = |n: &str| -> Option<(std::path::PathBuf, Vec<String>)> { resolve_named_shell(n) };
    match pref {
        ShellPref::Auto | ShellPref::Unknown => resolve_auto_shell(),
        ShellPref::Pwsh => named("pwsh").unwrap_or_else(resolve_auto_shell),
        ShellPref::Powershell => named("powershell").unwrap_or_else(resolve_auto_shell),
        ShellPref::Cmd => named("cmd").unwrap_or_else(resolve_auto_shell),
        ShellPref::Bash => named("bash").unwrap_or_else(resolve_auto_shell),
        ShellPref::Zsh => named("zsh").unwrap_or_else(resolve_auto_shell),
        ShellPref::Sh => named("sh").unwrap_or_else(resolve_auto_shell),
        ShellPref::Custom(path) => resolve_custom_shell(path).unwrap_or_else(resolve_auto_shell),
        // ShellPref is #[non_exhaustive]; future variants degrade to Auto
        // until the resolver learns them.
        _ => resolve_auto_shell(),
    }
}

#[tauri::command]
async fn pty_start(
    app: AppHandle,
    cached_config: State<'_, CachedConfig>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
) -> Result<u32, String> {
    let dims = PtyDims {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    };

    // Inject RIFT_SOCKET_NAME so `rift hook ...` from inside the spawned
    // shell finds the running instance without manual setup. Skipped when
    // the IPC server isn't up yet — the CLI surfaces a helpful error
    // pointing at --socket / $RIFT_SOCKET_NAME in that case.
    let mut opts = PtyOptions::new(dims);
    // Resolve the configured shell preference. Defaults (Auto) walk
    // pwsh > powershell > %COMSPEC% > cmd on Windows / $SHELL > zsh > bash > sh
    // on Unix — fixing the cmd.exe-only history-key UX surfaced in the
    // 2026-04-29 audit.
    let cfg_for_shell = cached_config.get();
    let (shell_path, mut shell_args) = resolve_shell_from_pref(&cfg_for_shell.terminal.shell);
    // D-018: Lane classification prelude injection. When lanes_enabled,
    // write the shell-specific prelude to a temp file and modify the
    // spawn args so the shell sources it at startup (transparent to user).
    let lanes_enabled = cfg_for_shell.terminal.lanes_enabled;
    if lanes_enabled {
        if let Some(injection) = prepare_lane_prelude(&shell_path) {
            if !injection.shell_args.is_empty() {
                shell_args = injection.shell_args;
            }
            for (k, v) in injection.extra_env {
                opts = opts.with_env(k, v);
            }
        }
    }
    opts = opts.with_shell(shell_path, shell_args);
    if let Some(ipc) = app.try_state::<BusIpcState>() {
        opts = opts.with_env("RIFT_SOCKET_NAME", ipc.socket_name.clone());
    } else {
        tracing::warn!("pty_start: IPC server not ready yet — RIFT_SOCKET_NAME omitted; hooks from this PTY session won't publish to the bus until next pty_start");
    }
    let effective_cwd = match cwd {
        Some(ref p) => dunce::canonicalize(p).map_err(|e| format!("invalid cwd: {e}"))?,
        None => app
            .try_state::<ProjectRoot>()
            .map(|r| r.inner().get())
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            }),
    };
    opts = opts.with_cwd(effective_cwd);
    let (mut output, control) = PtySession::spawn_with_options(opts).map_err(|e| {
        let msg = e.to_string();
        publish_error(
            app.state::<RiftBus>().inner(),
            "tauri.command.pty_start",
            &msg,
            None,
        );
        msg
    })?;
    let registry = app.state::<PtyRegistry>().inner().clone();
    let id = registry.insert(control);
    // Register a fresh CommandBuffer for this session (commands translator).
    app.state::<CommandBufferRegistry>().insert(id);

    // L3: capture the root child PID for process-name detection.
    let l3_root_pid = if lanes_enabled {
        app.state::<PtyRegistry>()
            .get(id)
            .and_then(|ctl| ctl.child_pid())
    } else {
        None
    };

    let drain_app = app.clone();
    // L2: clone the bus for hook/aegis subscription inside the drain task.
    let drain_bus = if lanes_enabled {
        Some(app.state::<RiftBus>().inner().clone())
    } else {
        None
    };
    // Store the outer drain handle in the registry so pty_kill (and the
    // natural-exit path inside the inner watcher) can abort it on cleanup.
    // The inner exit_rx watcher is structured as a nested spawn whose
    // lifetime is bounded by the outer drain task: when the outer task is
    // aborted the inner task's owning future is cancelled too, because it
    // is awaited only inside the outer task's async block. No separate
    // handle is needed for the inner watcher.
    let drain_handle = tauri::async_runtime::spawn(async move {
        let exit_rx = output.take_exit();
        if let Some(exit_rx) = exit_rx {
            let watcher_app = drain_app.clone();
            let watcher_registry = registry.clone();
            tauri::async_runtime::spawn(async move {
                let code = exit_rx.await.unwrap_or(u32::MAX);
                let _ = watcher_app.emit("pty_exited", PtyExitedEvent { id, code });
                // remove() also aborts the outer drain handle, but by the time
                // this inner task fires the outer task is already finishing
                // naturally (output.recv() returned None) — abort is a no-op
                // on a completed handle, so this is always safe.
                watcher_registry.remove(id);
                watcher_app.state::<CommandBufferRegistry>().remove(id);
            });
        }

        // D-018: Lane classifier sits in the output path when enabled.
        // Strips OSC 6973 sentinels; lane state tracked for bus/gutter
        // (Approach D — no ANSI color injection into the byte stream).
        let mut lane_classifier = if lanes_enabled {
            Some(LaneClassifier::new())
        } else {
            None
        };
        // Track last-published lane so we only emit bus events on transitions.
        let mut last_lane = Lane::Sys;
        let lane_bus = drain_bus.clone();

        // L2: Subscribe to Hook + Aegis bus events for direct lane injection.
        // When a hook/aegis event arrives on the bus, we inject the lane
        // transition directly into the classifier (no PTY round-trip needed).
        let mut hook_sub = drain_bus.as_ref().map(|bus| {
            let (_snap, sub) = bus.subscribe(SubscribeFilter::Category(Category::Hook));
            sub
        });
        let mut aegis_sub = drain_bus.as_ref().map(|bus| {
            let (_snap, sub) = bus.subscribe(SubscribeFilter::Category(Category::Aegis));
            sub
        });

        loop {
            tokio::select! {
                chunk_opt = output.recv() => {
                    let Some(chunk) = chunk_opt else { break };
                    let out = match lane_classifier {
                        Some(ref mut cls) => {
                            let transformed = cls.transform(&chunk);
                            // L3: On CMD_START, sample process tree for claude.
                            // inject_event updates lane state (for bus/notif
                            // consumers); no ANSI bytes sent to terminal
                            // (Approach D — stop fighting shell SGR).
                            if cls.take_cmd_start_flag() {
                                if let Some(pid) = l3_root_pid {
                                    if is_claude_descendant(pid) {
                                        cls.inject_event(SentinelEvent::ClaudeStart);
                                    }
                                }
                            }
                            // Publish lane.changed on transitions so the
                            // frontend gutter can update in real time.
                            let cur = cls.current_lane();
                            if cur != last_lane {
                                last_lane = cur;
                                if let Some(ref bus) = lane_bus {
                                    let mut env = Envelope::new(Category::Pty, "lane.changed");
                                    env.payload = json!({
                                        "lane": cur.to_string(),
                                        "session_id": id,
                                    });
                                    bus.publish(env);
                                }
                            }
                            transformed
                        }
                        None => chunk,
                    };
                    if drain_app.emit_to("main", "pty-chunk", serde_json::json!({ "id": id, "bytes": out })).is_err() { break; }
                }
                // L2 hook events: inject HookStart/HookEnd lane transitions.
                hook_result = async {
                    match hook_sub {
                        Some(ref mut sub) => sub.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Ok(env) = hook_result {
                        if let Some(ref mut cls) = lane_classifier {
                            let event = envelope_to_hook_event(&env);
                            cls.inject_event(event);
                            let cur = cls.current_lane();
                            if cur != last_lane {
                                last_lane = cur;
                                if let Some(ref bus) = lane_bus {
                                    let mut lenv = Envelope::new(Category::Pty, "lane.changed");
                                    lenv.payload = json!({
                                        "lane": cur.to_string(),
                                        "session_id": id,
                                    });
                                    bus.publish(lenv);
                                }
                            }
                        }
                    }
                }
                // L2 aegis events: inject AegisStart/AegisEnd lane transitions.
                aegis_result = async {
                    match aegis_sub {
                        Some(ref mut sub) => sub.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Ok(env) = aegis_result {
                        if let Some(ref mut cls) = lane_classifier {
                            let event = envelope_to_aegis_event(&env);
                            cls.inject_event(event);
                            let cur = cls.current_lane();
                            if cur != last_lane {
                                last_lane = cur;
                                if let Some(ref bus) = lane_bus {
                                    let mut lenv = Envelope::new(Category::Pty, "lane.changed");
                                    lenv.payload = json!({
                                        "lane": cur.to_string(),
                                        "session_id": id,
                                    });
                                    bus.publish(lenv);
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    // Register the drain handle alongside the session so pty_kill can abort it.
    app.state::<PtyRegistry>().insert_drain(id, drain_handle);

    Ok(id)
}

#[tauri::command]
fn pty_write(
    state: State<'_, PtyRegistry>,
    bus: State<'_, RiftBus>,
    cmd_bufs: State<'_, CommandBufferRegistry>,
    id: u32,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let control = state.get(id).ok_or_else(|| {
        let msg = format!("pty session {id} not found");
        publish_error(bus.inner(), "tauri.command.pty_write", &msg, None);
        msg
    })?;
    control.write(&bytes).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.pty_write", &msg, None);
        msg
    })?;
    // Write succeeded — feed bytes into the command buffer and publish each
    // completed command line. Failed writes (above) do NOT emit here (§3c).
    for (cmd, raw_len) in cmd_bufs.feed(id, &bytes) {
        publish_command(bus.inner(), id, cmd, raw_len);
    }
    Ok(())
}

#[tauri::command]
fn pty_resize(
    state: State<'_, PtyRegistry>,
    bus: State<'_, RiftBus>,
    id: u32,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let control = state.get(id).ok_or_else(|| {
        let msg = format!("pty session {id} not found");
        publish_error(bus.inner(), "tauri.command.pty_resize", &msg, None);
        msg
    })?;
    control
        .resize(PtyDims {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| {
            let msg = e.to_string();
            publish_error(bus.inner(), "tauri.command.pty_resize", &msg, None);
            msg
        })
}

#[tauri::command]
fn pty_kill(
    state: State<'_, PtyRegistry>,
    bus: State<'_, RiftBus>,
    cmd_bufs: State<'_, CommandBufferRegistry>,
    id: u32,
) -> Result<(), String> {
    let control = state.get(id).ok_or_else(|| {
        let msg = format!("pty session {id} not found");
        publish_error(bus.inner(), "tauri.command.pty_kill", &msg, None);
        msg
    })?;
    control.kill().map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.pty_kill", &msg, None);
        msg
    })?;
    // Remove the session + abort the drain task (in case the PTY output
    // stream is still blocked or the channel receiver hung up).
    state.remove(id);
    // Clean up the command buffer for this session.
    cmd_bufs.remove(id);
    Ok(())
}

#[tauri::command]
fn rift_bus_status(app: AppHandle, bus: State<'_, RiftBus>) -> BusStatus {
    let socket_name = app
        .try_state::<BusIpcState>()
        .map(|s| s.socket_name.clone())
        .unwrap_or_else(|| String::from("(ipc server not started)"));
    BusStatus {
        socket_name,
        subscribers: bus.subscriber_count(),
        replay_len: bus.replay_len(),
    }
}

/// Frontend → bus. Subscribe to a single category (or all categories when
/// `category` is `None`), stream the replay snapshot first, then live
/// envelopes. Returns a subscription id for [`bus_unsubscribe`].
#[tauri::command]
async fn bus_subscribe(
    app: AppHandle,
    category: Option<String>,
    on_envelope: Channel<Envelope>,
) -> Result<u64, String> {
    let bus = app.state::<RiftBus>().inner().clone();
    let registry = app.state::<BusSubscriptionRegistry>().inner();

    let filter = match category.as_deref() {
        None => SubscribeFilter::All,
        Some(raw) => SubscribeFilter::Category(parse_category(raw).inspect_err(|msg| {
            publish_error(
                app.state::<RiftBus>().inner(),
                "tauri.command.bus_subscribe",
                msg,
                None,
            );
        })?),
    };

    let (close_tx, mut close_rx) = oneshot::channel::<()>();

    // We must clone for `move` into the spawned task without holding any
    // `State<'_, _>` reference (which doesn't outlive this function).
    let registry_clone: BusSubscriptionRegistryHandle =
        BusSubscriptionRegistryHandle::new(app.clone());

    // Allocate the subscription id before spawning so the id is captured in
    // the task closure, while the handle is stored back into the registry
    // after spawn. The task cannot call registry.remove(id) before `insert`
    // completes because the spawned async block is not scheduled until after
    // the current await point (the spawn call) returns — which happens after
    // insert() below.
    let id = registry.next_id.fetch_add(1, Ordering::SeqCst);

    let drain_handle = tauri::async_runtime::spawn(async move {
        let (snapshot, mut sub) = bus.subscribe(filter);
        for env in snapshot {
            if on_envelope.send(env).is_err() {
                registry_clone.remove(id);
                return;
            }
        }
        loop {
            tokio::select! {
                _ = &mut close_rx => break,
                next = sub.recv() => {
                    match next {
                        Ok(env) => {
                            if on_envelope.send(env).is_err() { break; }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        registry_clone.remove(id);
    });

    // Store the close sender AND the drain handle together, keyed by id.
    {
        let mut guard = registry.subs.lock();
        guard.insert(id, (close_tx, drain_handle));
    }

    Ok(id)
}

#[tauri::command]
fn bus_unsubscribe(state: State<'_, BusSubscriptionRegistry>, id: u64) {
    state.remove(id);
}

/// Signal from Terminal.svelte that the first PTY has started and the
/// terminal canvas is live. Unblocks the vault walker (which waits on
/// `TerminalReady` instead of a fixed sleep).
#[tauri::command]
fn terminal_mounted(ready: State<'_, TerminalReady>) {
    // notify_one stores a permit if no one is waiting yet, so the vault
    // walker picks it up immediately when it starts polling — no 5s timeout.
    ready.0.notify_one();
}

/// Re-walk the Abyssal Index vault directory and re-publish all `vault.update`
/// + `walk.complete` envelopes. Called by IndexGraph.svelte on mount to handle
/// the case where boot-walk events have been evicted from the replay ring
/// buffer (e.g., after a page reload with heavy bus traffic between walks).
#[tauri::command]
fn vault_rescan(bus: State<'_, RiftBus>, project_root: State<'_, ProjectRoot>) {
    use rift_bus::translators::vault_walker::{publish_walk_complete, rescan_vaults};

    let vault_root = match directories::BaseDirs::new() {
        Some(b) => b.home_dir().join(".claude/abyssal-index"),
        None => {
            publish_walk_complete(bus.inner());
            return;
        }
    };
    if !vault_root.exists() {
        publish_walk_complete(bus.inner());
        return;
    }
    rescan_vaults(bus.inner(), &vault_root, Some(&project_root.get()));
}

/// Phase 8.7q.4 — page-load cleanup hook.
///
/// Called on every WebView mount (App.svelte `onMount`) to drain orphan
/// async resources whose JS-side callback ids died with the prior page.
///
/// Specifically:
///   * Every `Channel<Vec<u8>>` registered via [`pty_start`] is dead —
///     the PTY drain task on the Rust side keeps pumping bytes into a
///     callback id JS no longer knows about, producing the "[TAURI]
///     Couldn't find callback id" console flood (~5000/s on heavy
///     output). Killing all PTY sessions terminates those drain tasks.
///   * Every `Channel<Envelope>` registered via [`bus_subscribe`] has
///     the same fate — same flood, same fix.
///
/// In-process Rust subscribers (translators using `bus.subscribe()`
/// directly) are NOT affected — only the Tauri-Channel-backed ones,
/// which is what we want.
///
/// Returns a `(pty_killed, bus_unsubscribed)` count tuple for telemetry
/// (visible in the dev terminal's `[publish_error]` log if errors fire,
/// otherwise silent).
#[tauri::command]
fn rift_reset_for_reload(
    pty: State<'_, PtyRegistry>,
    bus_subs: State<'_, BusSubscriptionRegistry>,
) -> (usize, usize) {
    let pty_count = pty.kill_all();
    let bus_count = bus_subs.remove_all();
    (pty_count, bus_count)
}

/// Build a [`globset::GlobSet`] from a list of glob pattern strings.
///
/// Shared helper used by `fs_tree` and `todo_scan_command` — the glob
/// construction logic was previously copy-pasted between the two.
fn build_ignore_globs(patterns: &[String]) -> Result<globset::GlobSet, String> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob = globset::Glob::new(pattern)
            .map_err(|e| format!("invalid ignore glob '{pattern}': {e}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| format!("failed to build GlobSet: {e}"))
}

/// Return a static snapshot of the filesystem tree rooted at the current
/// project root (managed [`ProjectRoot`] state — Phase 6.7).
///
/// Globs are read from the current [`RiftConfig`]'s `fs.ignore_globs`.
/// Falling back to serde-default ignore globs on config-load failure ensures
/// the command always succeeds even if the config file is absent.
///
/// Async with `spawn_blocking` — filesystem traversal on large repos can
/// take 500ms-5s and must not block the Tauri command thread.
#[tauri::command]
async fn fs_tree(
    bus: State<'_, RiftBus>,
    project_root: State<'_, ProjectRoot>,
    cached_cfg: State<'_, CachedConfig>,
    tree_cache: State<'_, ProjectTreeCache>,
) -> Result<TreeNode, String> {
    let root = project_root.get();

    if let Some(cached) = tree_cache.get(&root) {
        return Ok(cached);
    }

    let bus_clone = bus.inner().clone();
    let ignore_patterns: Vec<String> = cached_cfg.get().fs.ignore_globs;
    let root_for_build = root.clone();

    let tree = tokio::task::spawn_blocking(move || {
        let ignore_globs = build_ignore_globs(&ignore_patterns).map_err(|e| {
            let msg = format!("fs_tree: {e}");
            publish_error(&bus_clone, "tauri.command.fs_tree", &msg, None);
            msg
        })?;

        build_tree(&root_for_build, FS_TREE_DEFAULT_MAX_DEPTH, &ignore_globs).map_err(|e| {
            let msg = e.to_string();
            publish_error(&bus_clone, "tauri.command.fs_tree", &msg, None);
            msg
        })
    })
    .await
    .map_err(|e| format!("fs_tree: task join error: {e}"))??;

    tree_cache.put(root, tree.clone());
    Ok(tree)
}

/// Frontend → bus. Construct an [`Envelope`] and publish it.
#[tauri::command]
fn bus_publish(
    bus: State<'_, RiftBus>,
    category: String,
    kind: String,
    payload: Option<serde_json::Value>,
) -> Result<(), String> {
    let cat = parse_category(&category).inspect_err(|msg| {
        publish_error(bus.inner(), "tauri.command.bus_publish", msg, None);
    })?;
    let mut env = Envelope::new(cat, kind);
    if let Some(p) = payload {
        env.payload = p;
    }
    bus.publish(env);
    Ok(())
}

/// Read the text of a project-relative file and return it to the frontend.
///
/// Uses the managed [`ProjectRoot`] as the base for path validation —
/// Phase 6.7 consolidation closes Validator lesson
/// `fs_read_write-still-reads-current_dir-after-project_swap`. After
/// `project_swap`, this command resolves paths against the swapped root.
#[tauri::command]
fn fs_read_text(
    bus: State<'_, RiftBus>,
    project_root: State<'_, ProjectRoot>,
    path: String,
) -> Result<String, String> {
    let root = project_root.get();
    read_text(&root, &path).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.fs_read_text", &msg, None);
        msg
    })
}

/// Write text content to an existing project-relative file.
///
/// Mirrors `fs_read_text`'s `ProjectRoot` discipline so post-swap writes
/// land in the swapped project, not the launch cwd.
#[tauri::command]
fn fs_write_text(
    bus: State<'_, RiftBus>,
    project_root: State<'_, ProjectRoot>,
    path: String,
    content: String,
) -> Result<(), String> {
    let root = project_root.get();
    write_text(&root, &path, &content).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.fs_write_text", &msg, None);
        msg
    })
}

// ---------------------------------------------------------------------------
// Phase 7.3 commands — aegis quick-actions (open files in OS default editor)
// §9 exemption: OS editor launch is a local utility, not an external integration.
// ---------------------------------------------------------------------------

/// Open `~/.claude/anti-claude-lessons.md` in the OS default editor.
///
/// Uses `std::process::Command` — no shell plugin required (std-process
/// is sufficient for these two hardcoded paths per Phase 7.3 spec).
/// Resolves `~/.claude/` via `directories::BaseDirs` (cross-platform,
/// already a workspace dep). Fails gracefully: returns `Err(String)` + logs
/// a `tracing::warn!` if the file is missing or the OS command exits nonzero.
#[tauri::command]
async fn aegis_open_lessons() -> Result<(), String> {
    open_in_os_editor(".claude/anti-claude-lessons.md")
}

/// Open `~/.claude/settings.json` in the OS default editor.
///
/// Same mechanics as `aegis_open_lessons` — see its doc for invariants.
#[tauri::command]
async fn aegis_open_settings() -> Result<(), String> {
    open_in_os_editor(".claude/settings.json")
}

/// Open `~/.claude/abyssal-index/` in the OS file manager.
///
/// Same mechanics as `aegis_open_lessons` — see its doc for invariants.
/// `open_in_os_editor` works for directories too: `cmd /C start` (Windows),
/// `open` (macOS), and `xdg-open` (Linux) all handle directory paths by
/// opening the platform file manager.
#[tauri::command]
async fn index_open_vault_root() -> Result<(), String> {
    open_in_os_editor(".claude/abyssal-index")
}

/// Open a URL in the user's default browser.
/// Only http:// and https:// URLs are allowed to prevent shell injection.
#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("open_url: only http:// and https:// URLs are allowed".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .status()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .status()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Phase 8.7i — TODO/FIXME/XXX/HACK scan over the active project root.
///
/// Best-effort scan with hard caps (1000 results, 1 MiB/file, depth 16)
/// so the command always returns promptly. Honors the same ignore-globs
/// as `fs_tree` so vendor / build dirs are skipped.
///
/// Async with `spawn_blocking` — full-project text scan must not block
/// the Tauri command thread.
#[tauri::command]
async fn todo_scan_command(
    bus: State<'_, RiftBus>,
    project_root: State<'_, ProjectRoot>,
    cached_config: State<'_, CachedConfig>,
) -> Result<Vec<todo_scan::TodoEntry>, String> {
    let root = project_root.get();
    let bus_clone = bus.inner().clone();
    let ignore_patterns: Vec<String> = cached_config.get().fs.ignore_globs;

    tokio::task::spawn_blocking(move || {
        let ignore_globs = build_ignore_globs(&ignore_patterns).map_err(|e| {
            let msg = format!("todo_scan: {e}");
            publish_error(&bus_clone, "tauri.command.todo_scan", &msg, None);
            msg
        })?;

        Ok(todo_scan::scan_todos(&root, &ignore_globs))
    })
    .await
    .map_err(|e| format!("todo_scan: task join error: {e}"))?
}

// ---------------------------------------------------------------------------
// MCP commands (D-014 Phase A)
//
// Settings UI reads/regenerates the auth token; backend exposes connection
// status (enabled? token present?) for the SettingsPanel readout.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct McpStatus {
    enabled: bool,
    token_present: bool,
    token_path: String,
}

/// Return current MCP enable state, whether a token is on disk, and the
/// token-file path so Settings can show "Token stored at <path>".
#[tauri::command]
fn mcp_status(cached_config: State<'_, CachedConfig>) -> Result<McpStatus, String> {
    let cfg = cached_config.get();
    let token_path = rift_bus::mcp_token_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("(unavailable: {e})"));
    let token_present = rift_bus::load_mcp_token()
        .map(|opt| opt.is_some())
        .unwrap_or(false);
    Ok(McpStatus {
        enabled: cfg.mcp.enabled,
        token_present,
        token_path,
    })
}

/// Return the current MCP token (creating one on first call if MCP is
/// enabled). Errors if the token cannot be persisted.
#[tauri::command]
fn mcp_token_get() -> Result<String, String> {
    rift_bus::ensure_mcp_token().map_err(|e| e.to_string())
}

/// Generate a fresh MCP token, replacing any existing one. Returns the new
/// token. Existing `rift-mcp` clients must be reconfigured with the new
/// value — there is no rotation grace period in v1.0.
#[tauri::command]
fn mcp_token_regenerate() -> Result<String, String> {
    let token = rift_bus::generate_mcp_token().map_err(|e| e.to_string())?;
    rift_bus::save_mcp_token(&token).map_err(|e| e.to_string())?;
    Ok(token)
}

/// Phase 8.7i — git status snapshot for the Git notif tab.
///
/// One-shot snapshot of branch, ahead/behind, staged/modified/untracked
/// lists, and the last commit. Returns `not_a_repo: true` when the project
/// root is not inside a git working tree so the frontend can render an
/// empty state instead of showing an error.
///
/// Async with `spawn_blocking` — runs 4+ sequential git subprocess
/// commands that can block for hundreds of milliseconds.
#[tauri::command]
async fn git_status_command(
    project_root: State<'_, ProjectRoot>,
) -> Result<git_status::GitStatus, String> {
    let root = project_root.get();
    tokio::task::spawn_blocking(move || git_status::snapshot(&root))
        .await
        .map_err(|e| format!("git_status: task join error: {e}"))?
}

/// Phase 8.7j — git mutating action: fetch / pull / push / commit-all.
/// Returns stdout/stderr/exit_code to the frontend so the UI can show
/// what happened. `commit-all` requires `message`.
///
/// Async with `spawn_blocking` — git network operations (fetch/pull/push)
/// can take seconds.
#[tauri::command]
async fn git_action_command(
    project_root: State<'_, ProjectRoot>,
    action: String,
    message: Option<String>,
) -> Result<git_status::GitActionResult, String> {
    let root = project_root.get();
    tokio::task::spawn_blocking(move || git_status::run_action(&root, &action, message.as_deref()))
        .await
        .map_err(|e| format!("git_action: task join error: {e}"))?
}

/// Async with `spawn_blocking` — reads .jsonl session files from disk.
#[tauri::command]
async fn list_sessions() -> Result<Vec<rift_bus::session_reader::SessionMeta>, String> {
    tokio::task::spawn_blocking(rift_bus::session_reader::list_sessions)
        .await
        .map_err(|e| format!("list_sessions: task join error: {e}"))?
}

/// Async with `spawn_blocking` — reads and parses a session .jsonl file.
#[tauri::command]
async fn load_session(session_id: String) -> Result<Vec<serde_json::Value>, String> {
    tokio::task::spawn_blocking(move || rift_bus::session_reader::load_session(&session_id))
        .await
        .map_err(|e| format!("load_session: task join error: {e}"))?
}

/// Async with `spawn_blocking` — reads and diffs two session files.
#[tauri::command]
async fn compare_sessions(
    baseline_id: String,
    compare_id: String,
) -> Result<rift_bus::session_compare::SessionDiff, String> {
    tokio::task::spawn_blocking(move || {
        rift_bus::session_compare::compare_sessions(&baseline_id, &compare_id)
    })
    .await
    .map_err(|e| format!("compare_sessions: task join error: {e}"))?
}

/// Shared helper: resolve `~/<rel_path>` and open it in the OS default editor.
///
/// Cross-platform dispatch:
///   - Windows  → `cmd /C start "" "<path>"`
///   - macOS    → `open "<path>"`
///   - Linux    → `xdg-open "<path>"`
///
// Maintenance note: if Aegis skill path changes, update this const.
const AEGIS_SKILL_RELATIVE: &str = ".claude/skills/aegis/SKILL.md";

/// Lightweight Aegis detection probe (non-gated — runs in every build).
///
/// Checks `~/.claude/skills/aegis/SKILL.md` existence and publishes
/// `aegis.detected` on the bus so the frontend can show the Aegis tab.
/// The private `rift-aegis` translator (feature-gated) adds snapshot +
/// log tail on top of this when compiled in.
async fn aegis_detect_lightweight(bus: RiftBus) {
    let path = match directories::BaseDirs::new() {
        Some(b) => b.home_dir().join(AEGIS_SKILL_RELATIVE),
        None => {
            tracing::warn!("aegis_detect: could not resolve home directory");
            return;
        }
    };
    let exists = tokio::fs::metadata(&path).await.is_ok();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let payload = serde_json::json!({
        "skill_path": path.to_string_lossy(),
        "detected_at_ms": ts,
        "exists": exists,
    });

    if let Ok(env) = Envelope::new(Category::Aegis, "aegis.detected").with_payload(&payload) {
        bus.publish(env);
        tracing::info!(
            "aegis_detect: SKILL.md {} at '{}'",
            if exists { "found" } else { "not found" },
            path.display()
        );
    }
}

/// Returns `Err` (with a descriptive message) if:
///   - `BaseDirs::new()` fails (very rare: no home dir).
///   - The resolved file does not exist on disk.
///   - The spawned process exits with a non-zero status.
///
/// Security: `rel_to_home` is always a hardcoded string from Tauri command
/// handlers (e.g. `"anti-claude-lessons.md"`, `"settings.json"`) — never
/// user-controlled input. The `cmd /C start` invocation is safe because the
/// path is resolved from `BaseDirs::home_dir()` and verified to exist.
fn open_in_os_editor(rel_to_home: &str) -> Result<(), String> {
    use directories::BaseDirs;

    let base = BaseDirs::new().ok_or_else(|| {
        let msg = "aegis_open: could not resolve home directory".to_string();
        tracing::warn!("{msg}");
        msg
    })?;

    let target = base.home_dir().join(rel_to_home);

    if !target.exists() {
        let msg = format!("aegis_open: file not found: {}", target.display());
        tracing::warn!("{msg}");
        return Err(msg);
    }

    let path_str = target.to_string_lossy().into_owned();

    #[cfg(target_os = "windows")]
    let status = {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", "", &path_str]);
        // Suppress the cmd.exe console flash on the file-open path. Without
        // this the `aegis_open_lessons` / `aegis_open_settings` commands
        // briefly paint a console window each time the user clicks them.
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.status()
    };

    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&path_str).status();

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let status = std::process::Command::new("xdg-open")
        .arg(&path_str)
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => {
            let msg = format!(
                "aegis_open: OS editor command exited with status {} for {}",
                s,
                target.display()
            );
            tracing::warn!("{msg}");
            Err(msg)
        }
        Err(e) => {
            let msg = format!(
                "aegis_open: failed to spawn OS editor for {}: {e}",
                target.display()
            );
            tracing::warn!("{msg}");
            Err(msg)
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 6.7 commands — config + project_swap
// ---------------------------------------------------------------------------

/// Return the current [`RiftConfig`] (loads from disk; returns defaults on
/// first launch or missing file).
#[tauri::command]
fn config_get(cached_config: State<'_, CachedConfig>) -> Result<RiftConfig, String> {
    Ok(cached_config.get())
}

/// Persist `cfg` to the platform config directory (atomic write).
#[tauri::command]
fn config_save(
    bus: State<'_, RiftBus>,
    cached: State<'_, CachedConfig>,
    tree_cache: State<'_, ProjectTreeCache>,
    cfg: RiftConfig,
) -> Result<(), String> {
    save_config(&cfg).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.config_save", &msg, None);
        msg
    })?;
    tree_cache.clear();
    cached.set(cfg);
    Ok(())
}

/// Swap the active project to `path`.
///
#[tauri::command]
fn project_root_get(project_root: State<'_, ProjectRoot>) -> String {
    project_root.get().to_string_lossy().into_owned()
}

/// Sequence (Design F):
///  1. Canonicalize `path` via `dunce` (pr003 gotcha).
///  2. Verify it is a directory.
///  3. Drop the old watcher (WatcherRegistry::clear).
///  4. Spawn a new watcher on `path`.
///  5. On watcher failure → publish error, return Err (config NOT saved — rollback).
///  6. Update ProjectRoot managed state.
///  7. Upsert + sort + cap ProjectEntry in RiftConfig; save atomically.
///  8. Publish `Category::System / kind="project.changed"` envelope so
///     Tree.svelte re-fetches the tree and clears treeActivity.
#[tauri::command]
fn project_swap(
    bus: State<'_, RiftBus>,
    watcher_reg: State<'_, WatcherRegistry>,
    project_root: State<'_, ProjectRoot>,
    cached_config: State<'_, CachedConfig>,
    path: String,
) -> Result<(), String> {
    // Step 1: Canonicalize via dunce (avoids Windows \\?\ UNC prefix issues).
    let canon = dunce::canonicalize(&path)
        .map_err(|e| format!("project_swap: canonicalize failed: {e}"))?;

    // Step 2: Verify the path is a directory.
    if !canon.metadata().map(|m| m.is_dir()).unwrap_or(false) {
        return Err(format!(
            "project_swap: '{}' is not a directory",
            canon.display()
        ));
    }

    // Read config from cache (audit fix T2 — avoids stale disk read).
    let mut cfg = cached_config.get();
    let ignore_globs = cfg.fs.ignore_globs.clone();

    // Step 3: Drop the old watcher cleanly BEFORE spawning the new one.
    // (pr003 `command-buffer-leak-on-natural-pty-exit` applied to watchers.)
    watcher_reg.clear();

    // Step 4: Spawn the new watcher.
    let bus_clone = bus.inner().clone();
    match spawn_fs_watcher(bus_clone.clone(), canon.clone(), ignore_globs) {
        Ok(new_watcher) => {
            watcher_reg.replace(new_watcher);
        }
        Err(e) => {
            // Step 5: Watcher spawn failed — publish error, return Err without
            // saving config (rollback semantics: old watcher is already gone but
            // at least config doesn't record a broken project).
            let msg = format!("project_swap: watcher spawn failed: {e}");
            publish_error(bus.inner(), "tauri.command.project_swap", &msg, None);
            return Err(msg);
        }
    }

    // Step 6: Update the managed ProjectRoot.
    project_root.set(canon.clone());

    // Step 7: Upsert the ProjectEntry, sort descending, cap at 10 (LRU).
    let name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| canon.to_string_lossy().into_owned());

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Update existing entry or insert a new one.
    if let Some(existing) = cfg.projects.iter_mut().find(|e| e.path == canon) {
        existing.last_used_ms = now_ms;
        existing.name = name;
    } else {
        cfg.projects.push(rift_bus::ProjectEntry {
            name,
            path: canon.clone(),
            last_used_ms: now_ms,
        });
    }

    // Sort by last_used_ms descending, cap to 10 (LRU eviction).
    cfg.projects
        .sort_by_key(|e| std::cmp::Reverse(e.last_used_ms));
    cfg.projects.truncate(10);

    // Update the in-memory config cache immediately so config_get callers
    // (e.g. ProjectPicker) see the new project list without waiting for disk.
    cached_config.set(cfg.clone());

    // Step 8: Publish project.changed so Tree.svelte re-fetches + clears activity.
    let path_str = canon.to_string_lossy().to_string();
    let mut env = Envelope::new(Category::System, "project.changed");
    env.payload = json!({
        "path": path_str,
        "name": canon
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone()),
    });
    bus.publish(env);

    let bus_bg = bus.inner().clone();
    let canon_bg = canon;
    std::thread::spawn(move || {
        if let Err(e) = save_config(&cfg) {
            tracing::warn!("project_swap: config save failed (non-fatal): {e}");
            publish_error(
                &bus_bg,
                "tauri.command.project_swap.config_save",
                e.to_string(),
                None,
            );
        }
        rift_bus::translators::status::publish_status_snapshot(&bus_bg, &canon_bg);
    });

    Ok(())
}

/// Tiny owned handle so spawned tasks can call `remove` without holding
/// a Tauri `State<'_, _>` reference across `await` points.
struct BusSubscriptionRegistryHandle {
    app: AppHandle,
}
impl BusSubscriptionRegistryHandle {
    fn new(app: AppHandle) -> Self {
        Self { app }
    }
    fn remove(&self, id: u64) {
        if let Some(reg) = self.app.try_state::<BusSubscriptionRegistry>() {
            reg.remove(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Startup cleanup
// ---------------------------------------------------------------------------

/// Kill orphaned WebView2 crashpad-handler processes from previous Rift sessions.
///
/// WebView2 spawns a crashpad-handler per browser-main process. When Rift exits
/// (or is killed during dev), the handler often outlives its parent and stays
/// resident. Over days of development this accumulates dozens of zombie processes.
/// Since this runs *before* Tauri creates any windows, every existing
/// `com.abyssal.rift` crashpad-handler is guaranteed stale.
#[cfg(windows)]
fn cleanup_orphaned_webview2() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let script = r#"Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'com\.abyssal\.rift' -and $_.CommandLine -match 'crashpad-handler' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"#;

    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(windows))]
fn cleanup_orphaned_webview2() {}

// ---------------------------------------------------------------------------
// Crash dump infrastructure + FFI panic guard
// ---------------------------------------------------------------------------

/// Wrap a Tauri invoke handler in `catch_unwind` so panics in IPC command
/// handlers don't unwind through the WebView2 COM FFI boundary (which is
/// `extern "system"` and triggers undefined behavior / instant process death).
fn guarded_invoke_handler<R, F>(inner: F) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync
where
    R: tauri::Runtime,
    F: Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync,
{
    move |invoke| match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner(invoke))) {
        Ok(handled) => handled,
        Err(panic_payload) => {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic_payload.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("(unknown panic)");
            tracing::error!("IPC command panic caught by FFI guard: {msg}");
            true
        }
    }
}

/// Write a crash/panic dump to the platform crash directory.
/// Used by both the global panic hook (captures real backtrace at panic site)
/// and the IPC `catch_unwind` guard (captures post-unwind backtrace).
fn write_crash_dump(message: &str, location: &str, source: &str) {
    let crash_dir = directories::BaseDirs::new()
        .map(|d| d.data_dir().join("com.abyssal.rift").join("crashes"))
        .unwrap_or_else(|| std::path::PathBuf::from("crashes"));
    let _ = std::fs::create_dir_all(&crash_dir);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = crash_dir.join(format!("crash-{ts}.txt"));

    let version = env!("CARGO_PKG_VERSION");
    let body = format!(
        "RIFT CRASH DUMP\n\
         ===============\n\
         version:  {version}\n\
         os:       {os}\n\
         arch:     {arch}\n\
         time:     {ts}\n\
         location: {location}\n\
         message:  {message}\n\
         source:   {source}\n\
         \n\
         backtrace:\n\
         {bt:?}\n",
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        bt = std::backtrace::Backtrace::force_capture(),
    );

    let _ = std::fs::write(&path, &body);

    // Rotate: keep only the 20 most recent crash files.
    if let Ok(entries) = std::fs::read_dir(&crash_dir) {
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
            })
            .collect();
        if files.len() > 20 {
            files.sort_by_key(|e| e.file_name());
            for old in &files[..files.len() - 20] {
                let _ = std::fs::remove_file(old.path());
            }
        }
    }

    eprintln!("Rift {source} — dump saved to {}", path.display());
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("(unknown)");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "(unknown location)".into());

        write_crash_dump(payload, &location, "panic_hook");
    }));

    cleanup_orphaned_webview2();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(PtyRegistry::default())
        .manage(BusSubscriptionRegistry::default())
        .manage(CommandBufferRegistry::default())
        .manage(cockpit_window::CockpitWindowState::default())
        .manage(notif_window::NotifWindowState::default())
        .manage(ShutdownNotify::default())
        .manage(TerminalReady::default())
        .manage(profiles::ProfileStore::default())
        .manage(command_history::CommandHistoryStore::default())
        .setup(|app| {
            // Bus is always present; the IPC server is best-effort and may
            // fail to bind (e.g. if the socket name is taken). Frontend can
            // still subscribe through Tauri commands either way.
            let bus = RiftBus::default();
            app.manage(bus.clone());

            // --- Phase 6.7: Config-driven root + WatcherRegistry ---
            //
            // Compute the canonical project root via dunce (pr003 gotcha:
            // windows-canonicalize-unc-prefix-vs-notify-callback-paths).
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let fs_root = dunce::canonicalize(&cwd).unwrap_or(cwd);

            // Load config for ignore globs. Fall back to defaults on first launch.
            let cfg = load_config().unwrap_or_default();
            let fs_ignore_globs = cfg.fs.ignore_globs.clone();

            // Register cached config (M9: avoids disk re-read per command call).
            app.manage(CachedConfig::new(cfg.clone()));
            app.manage(ProjectTreeCache::new());

            // Register the canonical project root.
            app.manage(ProjectRoot::new(fs_root.clone()));

            // Register the WatcherRegistry; spawn the initial watcher.
            let watcher_reg = WatcherRegistry::default();
            match spawn_fs_watcher(bus.clone(), fs_root, fs_ignore_globs) {
                Ok(watcher) => {
                    watcher_reg.replace(watcher);
                }
                Err(e) => {
                    let msg = e.to_string();
                    tracing::error!("rift-fs-watcher failed to start: {msg}");
                    publish_error(&bus, "tauri.setup.fs_watcher", &msg, None);
                    // Do NOT fail setup — Rift ships even without the watcher,
                    // mirroring the IpcServer best-effort pattern.
                }
            }
            app.manage(watcher_reg);

            // --- Phase 8.5: Vault-walker (runtime-gated on integrations.index_enabled) ---
            //
            // Resolve ~/.claude/abyssal-index/ via directories::BaseDirs.
            // Skip the spawn with a tracing::warn if the directory does not
            // exist (clean dev clones may not have the Abyssal Index).
            if cfg.integrations.index_enabled {
                let vault_root_opt =
                    directories::BaseDirs::new().map(|b| b.home_dir().join(".claude/abyssal-index"));

                match vault_root_opt {
                    None => {
                        tracing::warn!(
                            "vault_walker: could not resolve home directory — walker skipped"
                        );
                    }
                    Some(vault_root) => {
                        if vault_root.exists() {
                            // Phase 7.1 pattern: spawn on a separate tokio task.
                            // spawn_vault_walker is an async fn — wrap in
                            // tauri::async_runtime::spawn (mirrors the aegis probe).
                            //
                            // 250ms boot delay (2026-04-28 audit fix): vault.update
                            // envelope flood from the boot walk competes with the
                            // frontend cockpit's initial flex-layout pass, causing
                            // Terminal.svelte's host to never reach a measurable
                            // size before deferredFit's retry budget exhausts → PTY
                            // launches with bogus rows/cols → blank initial terminal.
                            // Audit-3 (git archeology) HIGH confidence pinpointed
                            // this as the regression introduced by bv-a (commit
                            // b35c915). Audit-1 (Rust PTY) recommended the boot
                            // delay as the cleanest fix. 250ms gives Terminal's
                            // onMount + deferredFit + pty_start round-trip time to
                            // win the layout race before envelope churn begins.
                            let bus_for_walker = bus.clone();
                            // Capture project_root BEFORE the async move block;
                            // mirrors the status_root capture pattern below (~line 1004).
                            // ProjectRoot::get() returns PathBuf (not Option); wrap in Some
                            // to satisfy spawn_vault_walker's Option<PathBuf> parameter.
                            let project_root_for_walker =
                                Some(app.state::<ProjectRoot>().inner().get());
                            let terminal_notify =
                                app.state::<TerminalReady>().inner().0.clone();
                            tauri::async_runtime::spawn(async move {
                                // Wait for the frontend's first Terminal to signal
                                // it has mounted + started its PTY, OR 5s timeout
                                // (covers edge cases where no terminal tab opens).
                                // This replaces the fragile 250ms/750ms fixed sleep
                                // that broke whenever setup overhead changed.
                                tokio::select! {
                                    _ = terminal_notify.notified() => {},
                                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                                        tracing::info!("vault_walker: terminal_mounted signal not received within 5s — proceeding anyway");
                                    },
                                }
                                spawn_vault_walker(
                                    bus_for_walker,
                                    vault_root,
                                    project_root_for_walker,
                                )
                                .await;
                            });
                        } else {
                            tracing::warn!(
                                "vault_walker: '{}' does not exist — walker skipped (normal on fresh clones)",
                                vault_root.display()
                            );
                        }
                    }
                }
            }

            let socket_name = format!("{IPC_SOCKET_PREFIX}-{}.sock", std::process::id());
            let bus_for_ipc = bus;
            let app_handle = app.handle().clone();
            let socket_name_for_task = socket_name.clone();

            // D-012 unblocked slice — clone bus + capture live root reference
            // before bus_for_ipc is moved into the IPC spawn closure below.
            let status_bus = bus_for_ipc.clone();
            let status_root = app.state::<ProjectRoot>().inner().clone();

            // Phase 7.1 — Aegis detection (two-layer).
            //
            // Layer 1 (non-gated): lightweight file-existence probe that runs
            // in every build. Publishes `aegis.detected` so the Aegis tab
            // appears whenever `~/.claude/skills/aegis/SKILL.md` exists.
            //
            // Layer 2 (feature-gated): the private rift-aegis translator that
            // additionally publishes a startup snapshot and spawns aegis.log
            // live tail. Only compiled with `--features aegis-private`.
            let aegis_detect_bus = bus_for_ipc.clone();
            #[cfg(feature = "aegis-private")]
            let aegis_bus = bus_for_ipc.clone();

            tauri::async_runtime::spawn(async move {
                match IpcServer::start(bus_for_ipc, &socket_name_for_task).await {
                    Ok(server) => {
                        tracing::info!(
                            "rift-bus IPC server listening on '{}'",
                            server.local_name()
                        );
                        app_handle.manage(BusIpcState {
                            socket_name: socket_name_for_task,
                            server: Mutex::new(Some(server)),
                        });
                    }
                    Err(e) => {
                        tracing::error!("rift-bus IPC server failed to start: {e}");
                    }
                }
            });

            // Layer 1 — runtime-gated detection probe.
            if cfg.integrations.aegis_enabled {
                tauri::async_runtime::spawn(async move {
                    aegis_detect_lightweight(aegis_detect_bus).await;
                });

                // Layer 2 — private translator (snapshot + log tail).
                #[cfg(feature = "aegis-private")]
                tauri::async_runtime::spawn(async move {
                    aegis_probe(aegis_bus).await;
                });
            }

            // D-012 unblocked slice — spawn status translator (DIR / GIT / REPO).
            // Publishes Category::Status / kind="usage" every 5 s with
            // { dir, git, repo, ts } derived from the current project root.
            // CTX / SESSION / WEEK / MODEL remain em-dash placeholders until the
            // Claude Code usage hook lands (still upstream-blocked, see DEFERRED.md D-012).
            // spawn_status_translator is async — Tauri 2 owns the runtime; wrap
            // in tauri::async_runtime::spawn (matches vault_walker pattern). A
            // bare tokio::spawn inside the translator would panic with
            // "no reactor running" since rift-bus runs on the Tauri main thread.
            // Shutdown handle for the status translator's `tokio::select!` —
            // signalled in the `RunEvent::ExitRequested` handler at the very
            // bottom of `run()` so the translator exits promptly when the
            // main window closes (otherwise the 5-second tick keeps firing
            // `git.exe`, painting visible flashes until Task Manager kill).
            let status_shutdown = app.state::<ShutdownNotify>().handle();
            tauri::async_runtime::spawn(async move {
                // 250ms boot delay — same rationale as the vault-walker spawn
                // above (cockpit-layout race; audit-1 / audit-3 2026-04-28).
                // First status envelope publish is then ~250ms after Terminal
                // mount, well past the cockpit's initial flex settle.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                spawn_status_translator(status_bus, status_root.path.clone(), status_shutdown).await;
            });

            // Session logger — persists every bus envelope to a .jsonl file on disk.
            // No boot delay needed (no visual output, no layout race).
            {
                let session_bus = app.state::<RiftBus>().inner().clone();
                let session_cfg = cfg.session.clone();
                let session_shutdown = app.state::<ShutdownNotify>().handle();
                tauri::async_runtime::spawn(async move {
                    spawn_session_logger(session_bus, session_cfg, session_shutdown).await;
                });
            }

            // S-3 — Sentinel translator (watches ~/.sentinel/events.jsonl).
            // Publishes sentinel.* envelopes on Category::Agent so the Agents
            // tab can display violations. No-op if ~/.sentinel/ doesn't exist.
            {
                let sentinel_bus = app.state::<RiftBus>().inner().clone();
                let sentinel_shutdown = app.state::<ShutdownNotify>().handle();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    rift_bus::spawn_sentinel_translator(sentinel_bus, sentinel_shutdown).await;
                });
            }

            // Agent translator — tails %TEMP%/rift-agent-events.jsonl written
            // by the CC agent hook (tools/cc-agent-hook.mjs). Also receives
            // events from `rift hook --category agent` CLI invocations via IPC.
            {
                let agent_bus = app.state::<RiftBus>().inner().clone();
                let agent_shutdown = app.state::<ShutdownNotify>().handle();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    rift_bus::translators::agent::spawn_agent_translator(agent_bus, agent_shutdown).await;
                });
            }

            // Portfolio health collector — periodic background sweep that
            // gathers vault staleness, sentinel violations, and git status
            // for every Abyssal Arts project and publishes it as a
            // `health.portfolio` envelope on Category::System.
            // §9 compliant: uses std::fs + std::process::Command only.
            {
                let health_bus = app.state::<RiftBus>().inner().clone();
                let health_shutdown = app.state::<ShutdownNotify>().handle();
                tauri::async_runtime::spawn(async move {
                    health_collector::spawn_health_collector(health_bus, health_shutdown).await;
                });
            }

            // Shared ProcessManager for MCP llm_process_start / llm_process_stop tools.
            // Must be managed before spawn_mcp_host so app_handle.try_state() resolves
            // inside the tool handlers. llama-server path is resolved lazily per-call
            // via ProcessManager::detect_llama_server(); the manager itself only needs
            // the bus for publishing lifecycle events.
            {
                let pm_bus = app.state::<RiftBus>().inner().clone();
                let llama_path = ProcessManager::detect_llama_server()
                    .unwrap_or_else(|| std::path::PathBuf::from("llama-server"));
                app.manage(std::sync::Arc::new(ProcessManager::new(llama_path, pm_bus)));
            }

            // D-014 Phase A — MCP host subscriber (off by default).
            //
            // Reads RiftConfig.mcp; if disabled, the spawn is a no-op and no
            // Category::Mcp traffic flows. When enabled, the host subscribes
            // to mcp.request.* envelopes published by the rift-mcp translator
            // and answers with mcp.response.* on the same bus.
            //
            // Spec: decisions/D-014_rift_mcp_v1_plan.md (locked v1.1).
            {
                let mcp_bus = app.state::<RiftBus>().inner().clone();
                let mcp_cfg = cfg.mcp.clone();
                let mcp_root = app.state::<ProjectRoot>().inner().get();
                let mcp_socket = socket_name.clone();
                let mcp_pty = app.state::<PtyRegistry>().inner().clone();
                let mcp_app = app.handle().clone();
                mcp_host::spawn_mcp_host(mcp_bus, mcp_cfg, mcp_root, mcp_socket, mcp_pty, mcp_app);
            }

            // D-014 Phase B — seed `cockpit.state` snapshot so the
            // `cockpit_state` MCP tool returns a useful value even before
            // the user has detached for the first time.
            cockpit_window::publish_cockpit_state(app.state::<RiftBus>().inner(), false);

            // Cockpit-detached window is now built on-demand by
            // cockpit_window::cockpit_detach to avoid the ~1.7 GB WebView2
            // process cost at startup when the user hasn't detached yet.

            // Pre-build PRE_BUILT notification windows at startup to avoid
            // the wry#1418 __TAURI_INTERNALS__ race on the most common case.
            // Additional windows (up to POOL_SIZE) are created on-demand by
            // notif_detach and destroyed on dock to reclaim memory.
            for i in 0..notif_window::PRE_BUILT {
                let label = notif_window::window_label(i);
                #[allow(unused_mut)]
                let mut notif_builder = tauri::WebviewWindowBuilder::new(
                    app,
                    &label,
                    tauri::WebviewUrl::App("notif-detached.html".into()),
                )
                .title("Rift — Notification")
                .inner_size(500.0, 650.0)
                .min_inner_size(360.0, 400.0)
                .decorations(false)
                .resizable(true)
                .visible(false);

                #[cfg(windows)]
                {
                    notif_builder = notif_builder.drag_and_drop(false);
                }

                if let Err(e) = notif_builder.build() {
                    tracing::warn!("notif_window: failed to pre-build '{label}': {e}");
                }
            }

            // Force-center and unminimize the main window to override stale
            // WebView2 cached state. EBWebView persists window geometry AND
            // minimized/maximized state across runs — a previous minimized
            // session restores as minimized even after show().
            if let Some(main_win) = app.get_webview_window("main") {
                let _ = main_win.unminimize();
                let _ = main_win.center();
                let _ = main_win.show();
                let _ = main_win.set_focus();
            }

            Ok(())
        })
        .invoke_handler(guarded_invoke_handler(tauri::generate_handler![
            phase0_check,
            pty_start,
            pty_write,
            pty_resize,
            pty_kill,
            rift_bus_status,
            bus_subscribe,
            bus_unsubscribe,
            bus_publish,
            rift_reset_for_reload,
            terminal_mounted,
            vault_rescan,
            fs_tree,
            fs_read_text,
            fs_write_text,
            config_get,
            config_save,
            project_root_get,
            project_swap,
            cockpit_window::cockpit_detach,
            cockpit_window::cockpit_reattach,
            cockpit_window::cockpit_status,
            notif_window::notif_detach,
            notif_window::notif_dock,
            notif_window::notif_get_config,
            notif_window::notif_detach_status,
            aegis_open_lessons,
            aegis_open_settings,
            open_url,
            index_open_vault_root,
            todo_scan_command,
            git_status_command,
            git_action_command,
            mcp_status,
            mcp_token_get,
            mcp_token_regenerate,
            list_sessions,
            load_session,
            compare_sessions,
            index_bridge::index_list_nodes,
            index_bridge::index_search_nodes,
            index_bridge::index_get_node,
            index_bridge::index_get_connections,
            index_bridge::index_get_stats,
            profiles::profile_list,
            profiles::profile_save,
            profiles::profile_load,
            profiles::profile_delete,
            file_preview::file_preview,
            command_history::command_history_record,
            command_history::command_stats,
            command_history::command_suggestions,
            integrations::integration_detect,
            llm_commands::llm_complete,
            llm_commands::llm_stream,
            llm_commands::llm_ensemble,
            llm_commands::llm_key_store,
            llm_commands::llm_key_delete,
        ]))
        .build(tauri::generate_context!())
        .expect("rift: tauri runtime failed to start")
        .run(|app_handle, event| {
            // Stop long-lived translator tasks the moment Tauri begins
            // tearing the app down. Without this the `spawn_status_translator`
            // 5-second loop continues spawning `git.exe` children after the
            // last window closes, which (a) on Windows paints visible
            // terminal flashes until the process fully exits and (b) can
            // hold the process alive long enough that the user must kill
            // it via Task Manager. `notify_waiters` wakes every task
            // currently `.notified().await`-ing on the same `Notify` and
            // they break out of their loops on the next poll.
            //
            // `ExitRequested` fires once, before the runtime begins
            // dropping resources — exactly the right moment to signal.
            // We do NOT call `api.prevent_exit()`; default exit is desired.
            if let RunEvent::ExitRequested { .. } = event {
                if let Some(notify) = app_handle.try_state::<ShutdownNotify>() {
                    notify.signal();
                }
                // Kill all live PTY sessions so child shell processes don't
                // survive as zombies after the window closes. The frontend's
                // onDestroy calls pty_kill per-session, but during app
                // teardown Svelte cleanup may not complete before the
                // runtime drops resources.
                if let Some(registry) = app_handle.try_state::<PtyRegistry>() {
                    let killed = registry.kill_all();
                    if killed > 0 {
                        tracing::info!("rift: killed {killed} PTY session(s) on exit");
                    }
                }
                // Clear the MCP discovery file so a stopped Rift can't
                // masquerade as live to a freshly-spawned `rift-mcp` client
                // started by Claude Code (see crates/rift-bus/src/config.rs).
                if let Err(e) = rift_bus::clear_mcp_socket() {
                    tracing::warn!("rift: failed to clear mcp_socket on exit: {e}");
                }
                // Destroy any live notification pop-out windows so they
                // don't hold the process alive after main closes.
                notif_window::destroy_all(app_handle);
                // The cockpit window uses prevent_close() + hide() for fast
                // re-detach, which means it survives the main window closing
                // and keeps the process alive. Destroy it explicitly here.
                if let Some(w) = app_handle.get_webview_window("cockpit-detached") {
                    let _ = w.destroy();
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tokio::time::Duration;

    /// Verify that `BusSubscriptionRegistry::remove` aborts the stored drain task.
    ///
    /// Proof strategy (Pattern B — AtomicBool):
    ///   A shared `ran_to_completion` flag starts `false`. The spawned task
    ///   yields control immediately (allowing abort to fire), then sleeps 500ms,
    ///   then sets the flag `true`. If `remove()` calls `handle.abort()`, the task
    ///   is cancelled before the sleep completes and the flag stays `false`.
    ///   After remove + a 50ms pause we assert `false`; after a further 600ms
    ///   (longer than the task's own 500ms sleep) we re-assert `false`. If abort
    ///   were omitted the task would complete, the flag would become `true`, and
    ///   the first assert would catch it.
    #[tokio::test]
    async fn bus_subscription_remove_aborts_drain_task() {
        let registry = BusSubscriptionRegistry::default();

        let (close_tx, _close_rx) = oneshot::channel::<()>();

        let ran_to_completion = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran_to_completion);

        // Task yields immediately so abort can fire, then sleeps 500ms before
        // setting the flag. Abort must prevent reaching the store().
        let handle = tauri::async_runtime::spawn(async move {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            flag.store(true, Ordering::SeqCst);
        });

        let id = registry.next_id.fetch_add(1, Ordering::SeqCst);
        registry.subs.lock().insert(id, (close_tx, handle));

        // remove() must call handle.abort() — that is what we are proving.
        registry.remove(id);

        // Give tokio a moment to process the abort signal.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // If abort fired, the task never reached flag.store(true). Fail here
        // means abort() was not called (or did not work).
        assert!(
            !ran_to_completion.load(Ordering::SeqCst),
            "task must NOT have run to completion: abort() should have fired before \
             the 500ms sleep completed"
        );

        // Wait longer than the task's own sleep (500ms) to be certain: if the
        // task were still alive it would have set the flag by now.
        tokio::time::sleep(Duration::from_millis(600)).await;

        assert!(
            !ran_to_completion.load(Ordering::SeqCst),
            "task still must not have completed after 650ms: abort() prevents the \
             task body from ever reaching flag.store(true)"
        );
    }

    /// Verify that `PtyRegistry::remove` aborts the associated drain handle.
    ///
    /// Proof strategy (Pattern B — AtomicBool):
    ///   Same as `bus_subscription_remove_aborts_drain_task`. A `ran_to_completion`
    ///   flag starts `false`; the task yields then sleeps 500ms then sets it `true`.
    ///   `remove()` must call `handle.abort()` before the sleep completes.
    ///   The test asserts the flag stays `false` at 50ms and again at 650ms.
    #[tokio::test]
    async fn pty_registry_remove_aborts_drain_handle() {
        let registry = PtyRegistry::default();

        // We don't have a real PtyControl here, so we can't call insert().
        // Insert directly into the inner map to test the drain-abort path.
        let id = 42u32;

        let ran_to_completion = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran_to_completion);

        // Task yields immediately so abort can fire, then sleeps 500ms before
        // setting the flag. Abort must prevent reaching the store().
        let handle = tauri::async_runtime::spawn(async move {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            flag.store(true, Ordering::SeqCst);
        });

        registry.insert_drain(id, handle);

        // Confirm the handle is registered before we trigger remove.
        assert!(
            registry.0.drain_handles.lock().contains_key(&id),
            "handle must be registered before remove"
        );

        // remove() must call handle.abort() — that is what we are proving.
        registry.remove(id);

        // Give tokio a moment to process the abort signal.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // If abort fired, the task never reached flag.store(true). Fail here
        // means abort() was not called (or did not work).
        assert!(
            !ran_to_completion.load(Ordering::SeqCst),
            "task must NOT have run to completion: abort() should have fired before \
             the 500ms sleep completed"
        );

        // Wait longer than the task's own sleep to be doubly certain.
        tokio::time::sleep(Duration::from_millis(600)).await;

        assert!(
            !ran_to_completion.load(Ordering::SeqCst),
            "task still must not have completed after 650ms: abort() prevents the \
             task body from ever reaching flag.store(true)"
        );
    }
}
