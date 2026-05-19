// notif_window.rs — Notification tab detach-to-window.
//
// Manages a pool of pre-built hidden Tauri webview windows that host
// detached notification tab content. Each window loads notif-detached.html
// and is configured at runtime via a `notif_configure` event.
//
// Architecture mirrors cockpit_window.rs (Phase 8.7d show/hide model):
// windows are pre-built at setup() to avoid the wry#1418 __TAURI_INTERNALS__
// injection race. This module handles show/hide + slot allocation.

use std::sync::Mutex;

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
    /// Each slot maps to a window label `notif-detached-{i}`.
    /// None = free, Some(tab_id) = occupied by the named notification tab.
    slots: Mutex<[Option<String>; POOL_SIZE]>,
    /// One-shot guard per slot — register close-requested listeners exactly
    /// once per app lifetime (mirrors cockpit_window.rs pattern).
    listeners_attached: Mutex<[bool; POOL_SIZE]>,
}

impl Default for NotifWindowState {
    fn default() -> Self {
        Self {
            slots: Mutex::new([const { None }; POOL_SIZE]),
            listeners_attached: Mutex::new([false; POOL_SIZE]),
        }
    }
}

// ---------------------------------------------------------------------------
// Event names
// ---------------------------------------------------------------------------

const EVENT_CONFIGURE: &str = "notif_configure";
const EVENT_RESET: &str = "notif_reset";
const EVENT_DETACHED: &str = "notif_detached";
const EVENT_DOCKED: &str = "notif_docked";

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConfigurePayload {
    tab_id: String,
    category: String,
    title: String,
    icon: String,
    severity_threshold: String,
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

fn emit_to_window(app: &AppHandle, label: &str, event: &str, payload: impl Serialize + Clone) {
    if let Err(e) = app.emit_filter(
        event,
        payload,
        |target| matches!(target, EventTarget::WebviewWindow { label: l } if l == label),
    ) {
        tracing::warn!("notif_window: failed to emit '{event}' to {label}: {e}");
    }
}

fn publish_notif_window_state(bus: &RiftBus, tab_id: &str, detached: bool) {
    let payload = json!({ "tab_id": tab_id, "detached": detached });
    if let Ok(env) = Envelope::new(Category::System, "notif.window.state").with_payload(&payload) {
        bus.publish(env);
    }
}

// Lock ordering: `slots` before `listeners_attached`. Never hold both.
fn dock_slot(app: &AppHandle, state: &NotifWindowState, slot: usize) {
    // Hold lock across take + hide to prevent a concurrent notif_detach
    // from grabbing the slot between take() and hide().
    let (tab_id, label) = {
        let mut slots = state.slots.lock().unwrap();
        let tab_id = slots[slot].take();
        let label = window_label(slot);
        // win.hide() is sync — safe to call under the lock.
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.hide();
        }
        (tab_id, label)
    };

    emit_to_window(app, &label, EVENT_RESET, ());

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
        let mut slots = state.slots.lock().unwrap();

        // Check if already detached.
        if slots.iter().any(|s| s.as_deref() == Some(&args.tab_id)) {
            return Err(format!(
                "notif_detach: tab '{}' is already detached",
                args.tab_id
            ));
        }

        // Find a free slot.
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
        let mut slots = state.slots.lock().unwrap();
        slots[slot] = None;
        let msg =
            format!("notif_detach: window '{label}' not found — setup-time build may have failed");
        publish_error(&bus, "tauri.command.notif_detach", &msg, None);
        msg
    })?;

    // Register close-requested handler once per slot per app lifetime.
    {
        let mut attached = state.listeners_attached.lock().unwrap();
        if !attached[slot] {
            attached[slot] = true;
            let app_for_close = app.clone();
            let slot_for_close = slot;
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Some(s) = app_for_close.try_state::<NotifWindowState>() {
                        dock_slot(&app_for_close, s.inner(), slot_for_close);
                    }
                }
            });
        }
    }

    // Configure the window with tab details.
    let payload = ConfigurePayload {
        tab_id: args.tab_id.clone(),
        category: args.category,
        title: args.title,
        icon: args.icon,
        severity_threshold: args.severity_threshold,
    };
    emit_to_window(&app, &label, EVENT_CONFIGURE, payload);

    // Show + focus.
    win.show().map_err(|e| {
        let mut slots = state.slots.lock().unwrap();
        slots[slot] = None;
        let msg = format!("notif_detach: failed to show window '{label}': {e}");
        publish_error(&bus, "tauri.command.notif_detach", &msg, None);
        msg
    })?;
    let _ = win.set_focus();

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

#[tauri::command]
pub fn notif_dock(
    app: AppHandle,
    state: State<'_, NotifWindowState>,
    tab_id: String,
) -> Result<(), String> {
    // Find-and-clear atomically under one lock acquisition to avoid
    // TOCTOU between lookup and dock_slot.
    let slot = {
        let mut slots = state.slots.lock().unwrap();
        let pos = slots
            .iter()
            .position(|s| s.as_deref() == Some(&tab_id))
            .ok_or_else(|| format!("notif_dock: tab '{tab_id}' is not detached"))?;
        slots[pos] = None;
        let label = window_label(pos);
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.hide();
        }
        (pos, label)
    };

    emit_to_window(&app, &slot.1, EVENT_RESET, ());
    emit_to_main(
        &app,
        EVENT_DOCKED,
        DetachEvent {
            tab_id: tab_id.clone(),
        },
    );
    if let Some(bus) = app.try_state::<RiftBus>() {
        publish_notif_window_state(bus.inner(), &tab_id, false);
    }

    Ok(())
}

#[tauri::command]
pub fn notif_detach_status(state: State<'_, NotifWindowState>) -> Vec<String> {
    let slots = state.slots.lock().unwrap();
    slots.iter().filter_map(|s| s.clone()).collect()
}
