// cockpit_window.rs — Phase 6.4: detachable cockpit right-pane (GUI half).
//
// Manages the lifecycle of an optional second Tauri webview window that hosts
// the filesystem tree (CockpitDetached.svelte). The main window retains the
// terminal; the detached window is GUI-only per §11 design.
//
// State choice: `AtomicBool` rather than `Mutex<Option<()>>`.
// Rationale: the only state transition is a boolean toggle (attached ↔ detached).
// AtomicBool requires no lock, never deadlocks if a thread panics mid-hold,
// and the compare-exchange idiom gives us the double-detach guard atomically
// without an extra Mutex wrapping. The actual WebviewWindow handle is NOT
// stored here — Tauri's internal window registry (accessed via
// `app.get_webview_window`) is the source of truth for the window object;
// we track the *logical* state here to gate commands quickly.

use std::sync::atomic::{AtomicBool, Ordering};

use rift_bus::publish_error;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewUrl, WebviewWindowBuilder};

// ---------------------------------------------------------------------------
// Managed state
// ---------------------------------------------------------------------------

/// Tracks whether the cockpit detached window is currently open.
///
/// Invariant: `is_detached` is `true` if and only if a window with label
/// `"cockpit-detached"` exists in the Tauri window registry AND we successfully
/// set the flag during `cockpit_detach`. The `WindowEvent::Destroyed` callback
/// resets the flag on every close path (button, X, OS close), preventing leaks.
pub struct CockpitWindowState {
    pub is_detached: AtomicBool,
}

impl Default for CockpitWindowState {
    fn default() -> Self {
        Self {
            is_detached: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Label + event name constants
// ---------------------------------------------------------------------------

const WINDOW_LABEL: &str = "cockpit-detached";
const EVENT_DETACHED: &str = "cockpit_detached";
const EVENT_REATTACHED: &str = "cockpit_reattached";

// Default window size per §11: "graph on a second display while terminal lives
// on the primary" — compact enough for a portrait side-monitor, wide enough for
// the tree layout.
const DEFAULT_WIDTH: f64 = 480.0;
const DEFAULT_HEIGHT: f64 = 800.0;

// ---------------------------------------------------------------------------
// Helper: emit an event to the main window only
// ---------------------------------------------------------------------------

fn emit_to_main(app: &AppHandle, event: &str) {
    if let Err(e) = app.emit_filter(
        event,
        (),
        |target| matches!(target, EventTarget::WebviewWindow { label } if label == "main"),
    ) {
        // Best-effort; log but don't block — the window-close path must not panic.
        tracing::warn!("cockpit_window: failed to emit '{event}' to main: {e}");
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Open the detached cockpit window.
///
/// Guards against double-detach atomically: uses `compare_exchange` to set
/// `is_detached = true` only if it was `false`. Returns `Err` immediately if
/// already detached, without touching the window registry.
///
/// On success:
/// 1. Builds a new WebviewWindow at `cockpit-detached.html`.
/// 2. Registers a `WindowEvent::Destroyed` listener — the single source of
///    truth for cleanup on ALL close paths (DOCK button, X, programmatic
///    destroy). This satisfies the pr003 "registry leak on natural close"
///    gotcha.
/// 3. Emits `cockpit_detached` to main so `App.svelte` hides the in-pane tree.
#[tauri::command]
pub fn cockpit_detach(app: AppHandle, state: State<'_, CockpitWindowState>) -> Result<(), String> {
    // Atomic double-detach guard: only proceed if we flip false → true.
    state
        .is_detached
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| String::from("cockpit_detach: cockpit window is already detached"))?;

    let bus = app.state::<rift_bus::RiftBus>().inner().clone();

    let win = WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        WebviewUrl::App("cockpit-detached.html".into()),
    )
    .title("Rift — Cockpit")
    .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
    .decorations(false) // We draw our own titlebar (CockpitDetached.svelte).
    .resizable(true)
    .build()
    .map_err(|e| {
        // Build failed — roll back the flag so a subsequent call can retry.
        state.is_detached.store(false, Ordering::SeqCst);
        let msg = format!("cockpit_detach: failed to open window: {e}");
        publish_error(&bus, "tauri.command.cockpit_detach", &msg, None);
        msg
    })?;

    // Register the destroyed handler — fires whether the user hits X,
    // the OS closes the window, or `cockpit_reattach` calls `.destroy()`.
    // This is the SINGLE source of truth for all reattach paths (design D).
    //
    // We move `app_for_close` into the closure. `AppHandle` is `'static` and
    // `Clone`, so the closure satisfies `'static`. We get the managed state
    // from the cloned handle inside the closure rather than borrowing `app`
    // (which is local and would not live long enough for `'static`).
    let app_for_close = app.clone();
    win.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            // Resolve state from the cloned handle — it is `'static`.
            if let Some(s) = app_for_close.try_state::<CockpitWindowState>() {
                s.is_detached.store(false, Ordering::SeqCst);
            }
            emit_to_main(&app_for_close, EVENT_REATTACHED);
        }
    });

    // Notify main that the cockpit is now detached.
    emit_to_main(&app, EVENT_DETACHED);

    Ok(())
}

/// Close the detached cockpit window and reattach the tree to the main window.
///
/// Calls `.destroy()` on the window if it exists, which triggers
/// `WindowEvent::Destroyed` → the handler above clears `is_detached` and
/// emits `cockpit_reattached`. If the window no longer exists (edge case:
/// user X'd it in the same millisecond), we fall back to clearing state
/// manually so the main window doesn't get stuck in "detached" UI mode.
#[tauri::command]
pub fn cockpit_reattach(
    app: AppHandle,
    state: State<'_, CockpitWindowState>,
) -> Result<(), String> {
    let bus = app.state::<rift_bus::RiftBus>().inner().clone();

    match app.get_webview_window(WINDOW_LABEL) {
        Some(win) => {
            // Destroy triggers WindowEvent::Destroyed, which clears state +
            // emits cockpit_reattached. Don't emit here to avoid a duplicate.
            win.destroy().map_err(|e| {
                let msg = format!("cockpit_reattach: failed to destroy window: {e}");
                publish_error(&bus, "tauri.command.cockpit_reattach", &msg, None);
                msg
            })?;
        }
        None => {
            // Window already gone (race: user X'd before button fired).
            // Ensure state + main-window event are consistent.
            state.is_detached.store(false, Ordering::SeqCst);
            emit_to_main(&app, EVENT_REATTACHED);
        }
    }

    Ok(())
}

/// Return whether the cockpit is currently detached.
///
/// Used by `App.svelte` on mount to recover correct UI state after a Tauri
/// reload mid-detach (design E).
#[tauri::command]
pub fn cockpit_status(state: State<'_, CockpitWindowState>) -> bool {
    state.is_detached.load(Ordering::SeqCst)
}
