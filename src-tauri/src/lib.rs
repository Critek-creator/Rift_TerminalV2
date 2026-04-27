mod cockpit_window;

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
// Real-time mechanism per `decisions/§10.15_real-time_update_mechanism.md`:
// Tier 1 = `Channel<T>` for in-process high-throughput streams (PTY
// output) + Tauri `app.emit` for low-frequency notifications. Tier 2 =
// `RiftBus` + `IpcServer` for cross-process translator integration.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rift_bus::{
    build_tree, publish_command, publish_error, read_text, spawn_fs_watcher, write_text, Category,
    CommandBuffer, Envelope, IpcServer, RiftBus, SubscribeFilter, TreeNode,
    FS_TREE_DEFAULT_MAX_DEPTH,
};
use rift_core::pty::{PtyControl, PtyDims, PtyOptions, PtySession};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

// ---------------------------------------------------------------------------
// PTY registry (Phase 1)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PtyRegistryInner {
    sessions: Mutex<HashMap<u32, PtyControl>>,
    next_id: AtomicU32,
}

/// Tracks live PTY sessions. Cheap to clone (`Arc` inside).
#[derive(Clone, Default)]
struct PtyRegistry(Arc<PtyRegistryInner>);

impl PtyRegistry {
    fn insert(&self, control: PtyControl) -> u32 {
        let id = self.0.next_id.fetch_add(1, Ordering::SeqCst);
        self.0
            .sessions
            .lock()
            .expect("pty registry poisoned")
            .insert(id, control);
        id
    }

    fn get(&self, id: u32) -> Option<PtyControl> {
        self.0
            .sessions
            .lock()
            .expect("pty registry poisoned")
            .get(&id)
            .cloned()
    }

    fn remove(&self, id: u32) {
        self.0
            .sessions
            .lock()
            .expect("pty registry poisoned")
            .remove(&id);
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
        self.buffers
            .lock()
            .expect("command buffer registry poisoned")
            .insert(id, CommandBuffer::new());
    }

    /// Feed bytes into the session's buffer. Returns completed
    /// `(command_string, raw_len)` pairs; empty if the session is not found.
    fn feed(&self, id: u32, bytes: &[u8]) -> Vec<(String, usize)> {
        let mut guard = self
            .buffers
            .lock()
            .expect("command buffer registry poisoned");
        match guard.get_mut(&id) {
            Some(buf) => buf.feed(bytes),
            None => Vec::new(),
        }
    }

    fn remove(&self, id: u32) {
        self.buffers
            .lock()
            .expect("command buffer registry poisoned")
            .remove(&id);
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
/// that, when fired, asks the corresponding drain task to exit.
#[derive(Default)]
struct BusSubscriptionRegistry {
    next_id: AtomicU64,
    subs: Mutex<HashMap<u64, oneshot::Sender<()>>>,
}

impl BusSubscriptionRegistry {
    fn insert(&self, close_tx: oneshot::Sender<()>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.subs
            .lock()
            .expect("bus subscription registry poisoned")
            .insert(id, close_tx);
        id
    }

    fn remove(&self, id: u64) {
        if let Some(tx) = self
            .subs
            .lock()
            .expect("bus subscription registry poisoned")
            .remove(&id)
        {
            let _ = tx.send(());
        }
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

#[tauri::command]
async fn pty_start(
    app: AppHandle,
    rows: u16,
    cols: u16,
    on_chunk: Channel<Vec<u8>>,
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
    if let Some(ipc) = app.try_state::<BusIpcState>() {
        opts = opts.with_env("RIFT_SOCKET_NAME", ipc.socket_name.clone());
    }
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

    let drain_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let exit_rx = output.take_exit();
        if let Some(exit_rx) = exit_rx {
            let watcher_app = drain_app.clone();
            let watcher_registry = registry.clone();
            tauri::async_runtime::spawn(async move {
                let code = exit_rx.await.unwrap_or(u32::MAX);
                let _ = watcher_app.emit("pty_exited", PtyExitedEvent { id, code });
                watcher_registry.remove(id);
                watcher_app.state::<CommandBufferRegistry>().remove(id);
            });
        }

        while let Some(chunk) = output.recv().await {
            if on_chunk.send(chunk).is_err() {
                break;
            }
        }
    });

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
    let registry_for_task = app.state::<BusSubscriptionRegistry>().inner();

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
    let id = registry.insert(close_tx);

    // We must clone for `move` into the spawned task without holding any
    // `State<'_, _>` reference (which doesn't outlive this function).
    let registry_clone: BusSubscriptionRegistryHandle =
        BusSubscriptionRegistryHandle::new(app.clone());

    let _ = registry_for_task; // silence unused — same value as `registry`

    tauri::async_runtime::spawn(async move {
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

    Ok(id)
}

#[tauri::command]
fn bus_unsubscribe(state: State<'_, BusSubscriptionRegistry>, id: u64) {
    state.remove(id);
}

/// Return a static snapshot of the filesystem tree rooted at the process
/// working directory.
///
/// Uses the same root and ignore-glob set as the live watcher spawned during
/// [`run`] setup.
///
/// # Phase 6.7 unblock
/// When the project-config system ships, the root and globs will be driven by
/// config state rather than re-computed inline here.
#[tauri::command]
fn fs_tree(bus: State<'_, RiftBus>) -> Result<TreeNode, String> {
    // Mirror the watcher's root and ignore-globs (Phase 6.7 unblock: move to
    // config-driven storage so the command and watcher share the same values).
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let ignore_patterns: Vec<String> = [
        ".git/**",
        "node_modules/**",
        "target/**",
        "dist/**",
        "*.log",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &ignore_patterns {
        let glob = globset::Glob::new(pattern).map_err(|e| {
            let msg = format!("fs_tree: invalid ignore glob '{pattern}': {e}");
            publish_error(bus.inner(), "tauri.command.fs_tree", &msg, None);
            msg
        })?;
        builder.add(glob);
    }
    let ignore_globs = builder.build().map_err(|e| {
        let msg = format!("fs_tree: failed to build GlobSet: {e}");
        publish_error(bus.inner(), "tauri.command.fs_tree", &msg, None);
        msg
    })?;

    build_tree(&root, FS_TREE_DEFAULT_MAX_DEPTH, &ignore_globs).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.fs_tree", &msg, None);
        msg
    })
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
/// Path validation (via [`read_text`]) runs first — directory traversal,
/// ignored-glob paths, and files exceeding 8 MiB are all rejected before any
/// I/O. Failures are published as `Category::System / kind="error"` envelopes
/// for diagnostic visibility (mirrors the `fs_tree` pattern).
#[tauri::command]
fn fs_read_text(bus: State<'_, RiftBus>, path: String) -> Result<String, String> {
    read_text(&path).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.fs_read_text", &msg, None);
        msg
    })
}

/// Write text content to an existing project-relative file.
///
/// Only writes to files that already exist (v1 constraint — see `write_text`
/// doc for the racy-lstat note). Path validation runs first. Failures are
/// published as `Category::System / kind="error"` envelopes.
#[tauri::command]
fn fs_write_text(bus: State<'_, RiftBus>, path: String, content: String) -> Result<(), String> {
    write_text(&path, &content).map_err(|e| {
        let msg = e.to_string();
        publish_error(bus.inner(), "tauri.command.fs_write_text", &msg, None);
        msg
    })
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
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyRegistry::default())
        .manage(BusSubscriptionRegistry::default())
        .manage(CommandBufferRegistry::default())
        .manage(cockpit_window::CockpitWindowState::default())
        .setup(|app| {
            // Bus is always present; the IPC server is best-effort and may
            // fail to bind (e.g. if the socket name is taken). Frontend can
            // still subscribe through Tauri commands either way.
            let bus = RiftBus::default();
            app.manage(bus.clone());

            // --- Filesystem watcher (Phase 6.1) ---
            // Phase 6.7 unblock: replace with config-driven root when the
            // project-config system ships.
            let fs_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            // Phase 6.7 unblock: move ignore globs to the project config system.
            let fs_ignore_globs: Vec<String> = [
                ".git/**",
                "node_modules/**",
                "target/**",
                "dist/**",
                "*.log",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            match spawn_fs_watcher(bus.clone(), fs_root, fs_ignore_globs) {
                Ok(watcher) => {
                    app.manage(watcher);
                }
                Err(e) => {
                    let msg = e.to_string();
                    tracing::error!("rift-fs-watcher failed to start: {msg}");
                    publish_error(&bus, "tauri.setup.fs_watcher", &msg, None);
                    // Do NOT fail setup — Rift ships even without the watcher,
                    // mirroring the IpcServer best-effort pattern.
                }
            }

            let socket_name = format!("{IPC_SOCKET_PREFIX}-{}.sock", std::process::id());
            let bus_for_ipc = bus;
            let app_handle = app.handle().clone();
            let socket_name_for_task = socket_name.clone();

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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            phase0_check,
            pty_start,
            pty_write,
            pty_resize,
            pty_kill,
            rift_bus_status,
            bus_subscribe,
            bus_unsubscribe,
            bus_publish,
            fs_tree,
            fs_read_text,
            fs_write_text,
            cockpit_window::cockpit_detach,
            cockpit_window::cockpit_reattach,
            cockpit_window::cockpit_status,
        ])
        .run(tauri::generate_context!())
        .expect("rift: tauri runtime failed to start");
}
