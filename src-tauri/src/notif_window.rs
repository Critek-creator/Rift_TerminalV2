// notif_window.rs — Notification tab detach-to-window.
//
// Hybrid architecture: windows are PRE-BUILT during setup() so the
// __TAURI_INTERNALS__ IPC bridge is injected. But hidden WebView2
// windows don't initialize JS, so notif_detach force-reloads the page
// before showing. The freshly-loaded Svelte component pulls its config
// via invoke('notif_get_config') — no event timing dependency.

use parking_lot::Mutex;

use rift_bus::{publish_error, Category, Envelope, RiftBus};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const POOL_SIZE: usize = 4;

pub fn window_label(slot: usize) -> String {
    format!("notif-detached-{slot}")
}

// ---------------------------------------------------------------------------
// Managed state
// ---------------------------------------------------------------------------

pub struct NotifWindowState {
    slots: Mutex<[Option<String>; POOL_SIZE]>,
    pending_configs: Mutex<[Option<ConfigurePayload>; POOL_SIZE]>,
}

impl Default for NotifWindowState {
    fn default() -> Self {
        Self {
            slots: Mutex::new([const { None }; POOL_SIZE]),
            pending_configs: Mutex::new([const { None }; POOL_SIZE]),
        }
    }
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

const EVENT_DETACHED: &str = "notif_detached";
const EVENT_DOCKED: &str = "notif_docked";

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurePayload {
    pub tab_id: String,
    pub category: String,
    pub title: String,
    pub icon: String,
    pub severity_threshold: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DetachEvent {
    tab_id: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn emit_to_main(app: &AppHandle, event: &str, payload: impl Serialize + Clone) {
    if let Err(e) = app.emit_filter(
        event,
        payload,
        |target| matches!(target, EventTarget::WebviewWindow { label } if label == "main"),
    ) {
        tracing::warn!("notif_window: failed to emit '{event}' to main: {e}");
    }
}

fn publish_notif_window_state(bus: &RiftBus, tab_id: &str, detached: bool) {
    let payload = json!({ "tab_id": tab_id, "detached": detached });
    if let Ok(env) = Envelope::new(Category::System, "notif.window.state").with_payload(&payload) {
        bus.publish(env);
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachArgs {
    tab_id: String,
    category: String,
    title: String,
    icon: String,
    severity_threshold: String,
}

#[tauri::command]
pub fn notif_detach(
    app: AppHandle,
    state: State<'_, NotifWindowState>,
    args: DetachArgs,
) -> Result<usize, String> {
    let slot = {
        let mut slots = state.slots.lock();

        if slots.iter().any(|s| s.as_deref() == Some(&args.tab_id)) {
            return Err(format!(
                "notif_detach: tab '{}' is already detached",
                args.tab_id
            ));
        }

        let free = slots.iter().position(|s| s.is_none());
        match free {
            Some(i) => {
                slots[i] = Some(args.tab_id.clone());
                i
            }
            None => {
                return Err(format!(
                    "notif_detach: all {POOL_SIZE} notification window slots are in use — \
                     dock one before detaching another"
                ));
            }
        }
    };

    let bus = app.state::<RiftBus>().inner().clone();
    let label = window_label(slot);

    let win = app.get_webview_window(&label).ok_or_else(|| {
        let mut slots = state.slots.lock();
        slots[slot] = None;
        let msg =
            format!("notif_detach: window '{label}' not found — setup-time build may have failed");
        publish_error(&bus, "tauri.command.notif_detach", &msg, None);
        msg
    })?;

    // Store config BEFORE reload — the window pulls it on mount.
    let payload = ConfigurePayload {
        tab_id: args.tab_id.clone(),
        category: args.category,
        title: args.title.clone(),
        icon: args.icon,
        severity_threshold: args.severity_threshold,
    };
    {
        let mut configs = state.pending_configs.lock();
        configs[slot] = Some(payload);
    }

    // Force-reload the page so JS initializes fresh in a now-visible context.
    // The __TAURI_INTERNALS__ IPC bridge persists across reloads within the
    // same webview — it was injected at setup() build time. Hidden WebView2
    // windows defer JS execution, so the initial page load never ran Svelte.
    // This reload triggers a fresh module execution.
    let _ = win.eval("location.reload()");

    // Show centered, then focus. Center first so the window is always
    // on-screen — the JS-side restoreSavedPosition can override later
    // if valid coordinates exist in localStorage.
    let _ = win.center();
    win.show().map_err(|e| {
        let mut slots = state.slots.lock();
        slots[slot] = None;
        let msg = format!("notif_detach: failed to show window '{label}': {e}");
        publish_error(&bus, "tauri.command.notif_detach", &msg, None);
        msg
    })?;
    let _ = win.set_focus();
    let _ = win.set_title(&format!("Rift — {}", args.title));

    // Register close-requested handler (idempotent — on_window_event
    // appends, but dock_slot is safe to call on an already-empty slot).
    let app_for_close = app.clone();
    let slot_for_close = slot;
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let main_alive = app_for_close
                .get_webview_window("main")
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false);
            if !main_alive {
                return;
            }
            api.prevent_close();
            if let Some(s) = app_for_close.try_state::<NotifWindowState>() {
                dock_slot(&app_for_close, s.inner(), slot_for_close);
            }
        }
    });

    // Notify main window.
    emit_to_main(
        &app,
        EVENT_DETACHED,
        DetachEvent {
            tab_id: args.tab_id.clone(),
        },
    );
    publish_notif_window_state(&bus, &args.tab_id, true);

    Ok(slot)
}

fn dock_slot(app: &AppHandle, state: &NotifWindowState, slot: usize) {
    let tab_id = {
        let mut slots = state.slots.lock();
        slots[slot].take()
    };

    {
        let mut configs = state.pending_configs.lock();
        configs[slot] = None;
    }

    let label = window_label(slot);
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.hide();
    }

    if let Some(tid) = &tab_id {
        emit_to_main(
            app,
            EVENT_DOCKED,
            DetachEvent {
                tab_id: tid.clone(),
            },
        );
        if let Some(bus) = app.try_state::<RiftBus>() {
            publish_notif_window_state(bus.inner(), tid, false);
        }
    }
}

#[tauri::command]
pub fn notif_dock(
    app: AppHandle,
    state: State<'_, NotifWindowState>,
    tab_id: String,
) -> Result<(), String> {
    let slot = {
        let slots = state.slots.lock();
        slots
            .iter()
            .position(|s| s.as_deref() == Some(&tab_id))
            .ok_or_else(|| format!("notif_dock: tab '{tab_id}' is not detached"))?
    };

    dock_slot(&app, state.inner(), slot);
    Ok(())
}

#[tauri::command]
pub fn notif_get_config(
    state: State<'_, NotifWindowState>,
    label: String,
) -> Option<ConfigurePayload> {
    let slot: usize = label
        .strip_prefix("notif-detached-")
        .and_then(|s| s.parse().ok())?;
    if slot >= POOL_SIZE {
        return None;
    }
    let configs = state.pending_configs.lock();
    configs[slot].clone()
}

#[tauri::command]
pub fn notif_detach_status(state: State<'_, NotifWindowState>) -> Vec<String> {
    let slots = state.slots.lock();
    slots.iter().filter_map(|s| s.clone()).collect()
}

/// Destroy all live notification windows. Called from the exit handler.
pub fn destroy_all(app: &AppHandle) {
    for i in 0..POOL_SIZE {
        let label = window_label(i);
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.destroy();
        }
    }
}
