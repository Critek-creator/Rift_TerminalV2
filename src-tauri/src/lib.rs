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

use rift_bus::{Category, Envelope, IpcServer, RiftBus, SubscribeFilter};
use rift_core::pty::{PtyControl, PtyDims, PtySession};
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

    let (mut output, control) = PtySession::spawn(dims).map_err(|e| e.to_string())?;
    let registry = app.state::<PtyRegistry>().inner().clone();
    let id = registry.insert(control);

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
fn pty_write(state: State<'_, PtyRegistry>, id: u32, bytes: Vec<u8>) -> Result<(), String> {
    state
        .get(id)
        .ok_or_else(|| format!("pty session {id} not found"))?
        .write(&bytes)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_resize(state: State<'_, PtyRegistry>, id: u32, rows: u16, cols: u16) -> Result<(), String> {
    state
        .get(id)
        .ok_or_else(|| format!("pty session {id} not found"))?
        .resize(PtyDims {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_kill(state: State<'_, PtyRegistry>, id: u32) -> Result<(), String> {
    state
        .get(id)
        .ok_or_else(|| format!("pty session {id} not found"))?
        .kill()
        .map_err(|e| e.to_string())
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
        Some(raw) => SubscribeFilter::Category(parse_category(raw)?),
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

/// Frontend → bus. Construct an [`Envelope`] and publish it.
#[tauri::command]
fn bus_publish(
    bus: State<'_, RiftBus>,
    category: String,
    kind: String,
    payload: Option<serde_json::Value>,
) -> Result<(), String> {
    let cat = parse_category(&category)?;
    let mut env = Envelope::new(cat, kind);
    if let Some(p) = payload {
        env.payload = p;
    }
    bus.publish(env);
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
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyRegistry::default())
        .manage(BusSubscriptionRegistry::default())
        .setup(|app| {
            // Bus is always present; the IPC server is best-effort and may
            // fail to bind (e.g. if the socket name is taken). Frontend can
            // still subscribe through Tauri commands either way.
            let bus = RiftBus::default();
            app.manage(bus.clone());

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
        ])
        .run(tauri::generate_context!())
        .expect("rift: tauri runtime failed to start");
}
