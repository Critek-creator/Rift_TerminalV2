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

use rift_bus::{publish_error, Category, Envelope, RiftBus};
use serde_json::json;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State};

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
    /// One-shot guard for cockpit_detach to register the close-requested
    /// listener exactly once per app lifetime (Phase 8.7d show/hide model).
    pub listeners_attached: AtomicBool,
}

impl Default for CockpitWindowState {
    fn default() -> Self {
        Self {
            is_detached: AtomicBool::new(false),
            listeners_attached: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Label + event name constants
// ---------------------------------------------------------------------------

const WINDOW_LABEL: &str = "cockpit-detached";
const EVENT_DETACHED: &str = "cockpit_detached";
const EVENT_REATTACHED: &str = "cockpit_reattached";

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

pub fn build_cockpit_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("cockpit-detached.html".into()),
    )
    .title("Rift — Cockpit")
    .inner_size(720.0, 900.0)
    .min_inner_size(420.0, 600.0)
    .decorations(false)
    .resizable(true)
    .visible(false);

    #[cfg(windows)]
    {
        builder = builder.drag_and_drop(false);
    }

    builder
        .build()
        .map_err(|e| format!("cockpit_window: failed to build '{WINDOW_LABEL}': {e}"))
}

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

/// Show the detached cockpit window, building it on-demand if needed.
///
/// The window is created on first detach (not at startup) to avoid the
/// ~1.7 GB WebView2 process cost when the user hasn't detached. Once
/// built, it persists for the session — subsequent detach/reattach cycles
/// use show/hide on the same window.
#[tauri::command]
pub fn cockpit_detach(app: AppHandle, state: State<'_, CockpitWindowState>) -> Result<(), String> {
    // Atomic double-detach guard: only proceed if we flip false → true.
    state
        .is_detached
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| String::from("cockpit_detach: cockpit window is already detached"))?;

    let bus = app.state::<rift_bus::RiftBus>().inner().clone();

    let win = match app.get_webview_window(WINDOW_LABEL) {
        Some(w) => w,
        None => build_cockpit_window(&app).inspect_err(|e| {
            state.is_detached.store(false, Ordering::SeqCst);
            publish_error(&bus, "tauri.command.cockpit_detach", e, None);
        })?,
    };

    // First-detach-only setup: force-reload + register close interceptor.
    // Hidden WebView2 windows defer JS execution, so the initial page load
    // never ran Svelte. Reload forces fresh module execution — same pattern
    // as notif_detach. Only needed once; subsequent show/hide cycles reuse
    // the now-initialized JS context.
    if !state.listeners_attached.swap(true, Ordering::SeqCst) {
        let _ = win.eval("location.reload()");

        let app_for_close = app.clone();
        win.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(w) = app_for_close.get_webview_window(WINDOW_LABEL) {
                    let _ = w.hide();
                }
                if let Some(s) = app_for_close.try_state::<CockpitWindowState>() {
                    s.is_detached.store(false, Ordering::SeqCst);
                }
                if let Some(b) = app_for_close.try_state::<RiftBus>() {
                    publish_cockpit_state(b.inner(), false);
                }
                emit_to_main(&app_for_close, EVENT_REATTACHED);
            }
        });
    }

    // Show + focus the pre-built window. show() is idempotent if already shown.
    win.show().map_err(|e| {
        state.is_detached.store(false, Ordering::SeqCst);
        let msg = format!("cockpit_detach: failed to show window: {e}");
        publish_error(&bus, "tauri.command.cockpit_detach", &msg, None);
        msg
    })?;
    let _ = win.set_focus();

    // Notify main that the cockpit is now detached.
    emit_to_main(&app, EVENT_DETACHED);
    publish_cockpit_state(&bus, true);

    Ok(())
}

/// Hide the cockpit window and reattach the tree to the main window.
///
/// Phase 8.7d: hide instead of destroy. The cockpit window persists for the
/// app lifetime so its already-injected Tauri runtime survives across detach
/// cycles. The close-requested listener registered in cockpit_detach handles
/// the X-button path the same way (prevent_close + hide).
#[tauri::command]
pub fn cockpit_reattach(
    app: AppHandle,
    state: State<'_, CockpitWindowState>,
) -> Result<(), String> {
    let bus = app.state::<rift_bus::RiftBus>().inner().clone();

    match app.get_webview_window(WINDOW_LABEL) {
        Some(win) => {
            win.hide().map_err(|e| {
                let msg = format!("cockpit_reattach: failed to hide window: {e}");
                publish_error(&bus, "tauri.command.cockpit_reattach", &msg, None);
                msg
            })?;
            state.is_detached.store(false, Ordering::SeqCst);
            emit_to_main(&app, EVENT_REATTACHED);
            publish_cockpit_state(&bus, false);
        }
        None => {
            // Window missing entirely (setup-time build failed). Recover state
            // so the UI doesn't get stuck in "detached" mode.
            state.is_detached.store(false, Ordering::SeqCst);
            emit_to_main(&app, EVENT_REATTACHED);
            publish_cockpit_state(&bus, false);
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

/// Publish a `Category::System / kind="cockpit.state"` envelope so MCP
/// consumers (`cockpit_state` tool, D-014 Phase B) can see the current
/// detach state without holding a reference to [`CockpitWindowState`].
///
/// Should be called whenever the detach flag flips, plus once at startup
/// from `setup()` so the bus replay buffer always carries a snapshot.
pub fn publish_cockpit_state(bus: &RiftBus, detached: bool) {
    let payload = json!({ "detached": detached });
    if let Ok(env) = Envelope::new(Category::System, "cockpit.state").with_payload(&payload) {
        bus.publish(env);
    }
}
