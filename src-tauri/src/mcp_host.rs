//! MCP host-side subscriber (D-014 Phase A).
//!
//! Listens on `Category::Mcp / kind="mcp.request.*"` envelopes published by
//! the `rift-mcp` translator (which speaks JSON-RPC over stdio externally).
//! Each request is answered with a matching `mcp.response.*` envelope on
//! the same bus, keyed by `request_id` in the payload.
//!
//! Audit: every accepted invocation publishes `mcp.invoke` BEFORE running,
//! so denied or panicking calls are still surfaced in the audit trail.
//!
//! Off-by-default: `RiftConfig.mcp.enabled = false` short-circuits
//! [`spawn_mcp_host`] — the subscriber never starts and no `Category::Mcp`
//! traffic flows.
//!
//! Spec: `decisions/D-014_rift_mcp_v1_plan.md` (locked v1.1 — 2026-04-29).

use std::path::{Path, PathBuf};
use std::time::Duration;

use rift_bus::{
    build_tree, publish_error, read_text, save_config, write_text, Category, Envelope, McpConfig,
    RiftBus, ShellPref, SubscribeFilter,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::oneshot;

use crate::{git_status, todo_scan, CachedConfig, PtyRegistry};

/// Spawn the MCP host subscriber on the rift-bus runtime.
///
/// No-op when `cfg.enabled = false`. Otherwise:
///
/// 1. Confirms the on-disk token exists (generates one if not).
/// 2. Subscribes to `Category::Mcp`.
/// 3. Routes each `mcp.handshake` to a token-verification reply, and each
///    `mcp.request.*` to its tool handler — publishing `mcp.invoke` audit
///    envelopes BEFORE running.
///
/// `project_root` is captured for tools that need it (e.g. `git_status`).
/// `socket_name` is the live `rift-bus` IPC socket name — written to the
/// MCP discovery file so the standalone `rift-mcp` binary can find this
/// host without `--socket` or `$RIFT_SOCKET_NAME` plumbed through.
pub fn spawn_mcp_host(
    bus: RiftBus,
    cfg: McpConfig,
    project_root: PathBuf,
    socket_name: String,
    pty_registry: PtyRegistry,
    app_handle: AppHandle,
) {
    if !cfg.enabled {
        tracing::debug!("mcp_host: MCP disabled in config — subscriber not spawned");
        return;
    }

    // Ensure the token file exists. Errors are non-fatal but logged — the
    // host still starts, but every handshake will fail closed (the token
    // file is the source of truth used by the rift-mcp client).
    let token_present = match rift_bus::ensure_mcp_token() {
        Ok(_) => true,
        Err(e) => {
            tracing::error!("mcp_host: failed to ensure token: {e}");
            publish_error(&bus, "mcp_host.token", e.to_string(), None);
            false
        }
    };

    // Publish the live socket name so the standalone `rift-mcp` binary —
    // which Claude Code spawns with no env or args — can discover this
    // host. Cleared in the `RunEvent::ExitRequested` handler in lib.rs.
    if let Err(e) = rift_bus::save_mcp_socket(&socket_name) {
        tracing::error!("mcp_host: failed to write discovery file: {e}");
        publish_error(&bus, "mcp_host.discovery", e.to_string(), None);
    }

    let bus_for_task = bus.clone();
    let cfg_for_task = cfg;
    tauri::async_runtime::spawn(async move {
        run_subscriber(
            bus_for_task,
            cfg_for_task,
            project_root,
            token_present,
            pty_registry,
            app_handle,
        )
        .await;
    });
}

async fn run_subscriber(
    bus: RiftBus,
    cfg: McpConfig,
    project_root: PathBuf,
    token_present: bool,
    pty_registry: PtyRegistry,
    app_handle: AppHandle,
) {
    let (snapshot, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::Mcp));
    // Only one bus_tail stream can be active at a time (v1 limitation).
    // A new bus_tail request aborts the previous stream.
    let bus_tail_handle: tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>> =
        tokio::sync::Mutex::new(None);

    for env in snapshot {
        handle_envelope(
            &bus,
            &cfg,
            &project_root,
            token_present,
            &pty_registry,
            &app_handle,
            &env,
            &bus_tail_handle,
        )
        .await;
    }

    loop {
        match sub.recv().await {
            Ok(env) => {
                handle_envelope(
                    &bus,
                    &cfg,
                    &project_root,
                    token_present,
                    &pty_registry,
                    &app_handle,
                    &env,
                    &bus_tail_handle,
                )
                .await
            }
            Err(rift_bus::BusError::Lagged(n)) => {
                tracing::warn!("mcp_host: subscriber lagged by {n} envelopes");
            }
            Err(rift_bus::BusError::Closed) => break,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_envelope(
    bus: &RiftBus,
    cfg: &McpConfig,
    project_root: &Path,
    token_present: bool,
    pty_registry: &PtyRegistry,
    app_handle: &AppHandle,
    env: &Envelope,
    bus_tail_handle: &tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
) {
    // Ignore our own outbound envelopes (responses, audit, errors). The
    // subscriber sees everything in Category::Mcp, including what we publish.
    let kind = env.kind.as_str();
    if kind == "mcp.invoke"
        || kind.starts_with("mcp.response.")
        || kind == "mcp.handshake.ack"
        || kind == "mcp.handshake.deny"
    {
        return;
    }

    if kind == "mcp.handshake" {
        handle_handshake(bus, env, token_present);
        return;
    }

    if let Some(tool_name) = kind.strip_prefix("mcp.request.") {
        // Audit BEFORE running — denied or panicking calls still log.
        let request_id = env
            .payload
            .get("request_id")
            .cloned()
            .unwrap_or(Value::Null);
        publish_audit(
            bus,
            "mcp.invoke",
            json!({
                "tool": tool_name,
                "request_id": request_id.clone(),
                "ts": env.ts,
            }),
        );

        // Streaming tools (D-014 Phase A.1): spawn a subscribe-publish task
        // and return a start-ack synchronously. Subsequent envelopes flow
        // back as `mcp.notify.{tool}` rather than a single `mcp.response`.
        if tool_name == "bus_tail" {
            {
                let mut prev = bus_tail_handle.lock().await;
                if let Some(h) = prev.take() {
                    h.abort();
                }
            }
            let result = start_bus_tail(
                bus.clone(),
                request_id.clone(),
                &env.payload,
                bus_tail_handle,
            )
            .await;
            publish_response(bus, tool_name, request_id, result);
            return;
        }

        let result = dispatch_tool(
            bus,
            cfg,
            project_root,
            pty_registry,
            app_handle,
            tool_name,
            &env.payload,
        )
        .await;
        publish_response(bus, tool_name, request_id, result);
    }
}

fn handle_handshake(bus: &RiftBus, env: &Envelope, token_present: bool) {
    let provided = env
        .payload
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let request_id = env
        .payload
        .get("request_id")
        .cloned()
        .unwrap_or(Value::Null);

    if !token_present {
        publish_audit(
            bus,
            "mcp.handshake.deny",
            json!({ "request_id": request_id, "reason": "no token on disk" }),
        );
        return;
    }

    let on_disk = match rift_bus::load_mcp_token() {
        Ok(Some(t)) => t,
        _ => {
            publish_audit(
                bus,
                "mcp.handshake.deny",
                json!({ "request_id": request_id, "reason": "token unreadable" }),
            );
            return;
        }
    };

    if constant_time_eq(provided.as_bytes(), on_disk.as_bytes()) {
        publish_audit(
            bus,
            "mcp.handshake.ack",
            json!({ "request_id": request_id }),
        );
    } else {
        publish_audit(
            bus,
            "mcp.handshake.deny",
            json!({ "request_id": request_id, "reason": "token mismatch" }),
        );
    }
}

async fn dispatch_tool(
    bus: &RiftBus,
    cfg: &McpConfig,
    project_root: &Path,
    pty_registry: &PtyRegistry,
    app_handle: &AppHandle,
    tool: &str,
    payload: &Value,
) -> Result<Value, String> {
    match tool {
        // Phase A
        "bus_history" => tool_bus_history(bus, payload),
        "git_status" => tool_git_status(project_root),
        "aegis_state" => tool_aegis_state(bus),
        // Phase A.1 — bus_tail is intercepted before dispatch_tool — see handle_envelope.
        "bus_tail" => Err("bus_tail must route through start_bus_tail".into()),
        // Phase B — Tier 1 read tools (D-014 §3 Tier 1 catalog)
        "fs_read" => tool_fs_read(project_root, payload),
        "fs_tree" => tool_fs_tree(project_root, app_handle, payload),
        "todo_scan" => tool_todo_scan(project_root, app_handle),
        "pty_list" => tool_pty_list(pty_registry),
        "cockpit_state" => tool_cockpit_state(bus),
        "notif_tabs" => tool_notif_tabs(bus),
        // Phase C — Tier 2 inspection tools (D-014 §3, default-off)
        "dom_snapshot" => {
            if !cfg.allow_inspection {
                return Err("dom_snapshot requires mcp.allow_inspection = true".into());
            }
            tool_dom_snapshot(app_handle, payload).await
        }
        "screenshot" => {
            if !cfg.allow_inspection {
                return Err("screenshot requires mcp.allow_inspection = true".into());
            }
            tool_screenshot(app_handle, payload).await
        }
        "js_eval" => {
            if !cfg.allow_inspection {
                return Err("js_eval requires mcp.allow_inspection = true".into());
            }
            if !cfg.allow_js_eval {
                return Err("js_eval requires mcp.allow_js_eval = true".into());
            }
            tool_js_eval(app_handle, payload).await
        }
        // Phase D — Tier 3 mutating + read tools (D-014 §3)
        "pty_input" => {
            if !cfg.allow_mutations {
                return Err("pty_input requires mcp.allow_mutations = true".into());
            }
            tool_pty_input(pty_registry, payload)
        }
        "pty_read" => {
            if !cfg.allow_inspection {
                return Err("pty_read requires mcp.allow_inspection = true".into());
            }
            tool_pty_read(app_handle, payload).await
        }
        "bus_publish" => {
            if !cfg.allow_mutations {
                return Err("bus_publish requires mcp.allow_mutations = true".into());
            }
            tool_bus_publish(bus, payload)
        }
        "fs_write" => {
            if !cfg.allow_mutations {
                return Err("fs_write requires mcp.allow_mutations = true".into());
            }
            tool_fs_write(project_root, payload)
        }
        "git_action" => {
            if !cfg.allow_mutations {
                return Err("git_action requires mcp.allow_mutations = true".into());
            }
            tool_git_action(project_root, payload)
        }
        "simulate_click" => {
            if !cfg.allow_inspection {
                return Err("simulate_click requires mcp.allow_inspection = true".into());
            }
            if !cfg.allow_mutations {
                return Err("simulate_click requires mcp.allow_mutations = true".into());
            }
            tool_simulate_click(app_handle, payload).await
        }
        "simulate_drag" => {
            if !cfg.allow_inspection {
                return Err("simulate_drag requires mcp.allow_inspection = true".into());
            }
            if !cfg.allow_mutations {
                return Err("simulate_drag requires mcp.allow_mutations = true".into());
            }
            tool_simulate_drag(app_handle, payload).await
        }
        // Post-Phase D — diagnostic + runtime config tools
        "rift_diagnose" => tool_rift_diagnose(bus, pty_registry, app_handle),
        "rift_config_set" => {
            if !cfg.allow_mutations {
                return Err("rift_config_set requires mcp.allow_mutations = true".into());
            }
            tool_rift_config_set(bus, app_handle, payload)
        }
        other => Err(format!("unknown MCP tool: {other}")),
    }
}

/// Phase A.1 — start a streaming `bus_tail` subscription.
///
/// Parses optional `category` + `kind_prefix` filter args, spawns a task
/// that subscribes to the bus and publishes one `mcp.notify.bus_tail`
/// envelope per matching event, and returns a start-ack `Value` for the
/// initial `mcp.response.bus_tail`.
///
/// Per locked v1.0 design, cancellation is implicit — when the stdio
/// client (`rift-mcp`) disconnects, the bus subscription stays live but
/// notifications are dropped on the floor by the broadcast channel. The
/// task itself exits when the bus closes (process shutdown) or when the
/// subscribe channel returns `Closed`.
async fn start_bus_tail(
    bus: RiftBus,
    request_id: Value,
    payload: &Value,
    bus_tail_handle: &tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
) -> Result<Value, String> {
    // Parse filter args. Both optional; absent = match all.
    let category_filter = payload
        .get("category")
        .and_then(|v| v.as_str())
        .map(str::to_lowercase);
    let kind_prefix = payload
        .get("kind_prefix")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let filter = match category_filter.as_deref() {
        None => SubscribeFilter::All,
        Some(c) => match serde_json::from_value::<Category>(Value::String(c.to_owned())) {
            Ok(cat) => SubscribeFilter::Category(cat),
            Err(_) => return Err(format!("invalid category: {c}")),
        },
    };

    let bus_for_task = bus.clone();
    let request_id_for_task = request_id.clone();
    let kind_prefix_for_task = kind_prefix.clone();
    let handle = tauri::async_runtime::spawn(async move {
        run_bus_tail(
            bus_for_task,
            request_id_for_task,
            kind_prefix_for_task,
            filter,
        )
        .await;
    });
    {
        let mut prev = bus_tail_handle.lock().await;
        *prev = Some(handle);
    }

    Ok(json!({
        "stream_started": true,
        "request_id": request_id,
        "filter": {
            "category": category_filter,
            "kind_prefix": kind_prefix,
        },
    }))
}

/// Body of the streaming subscriber spawned by [`start_bus_tail`].
async fn run_bus_tail(
    bus: RiftBus,
    request_id: Value,
    kind_prefix: Option<String>,
    filter: SubscribeFilter,
) {
    // Skip the snapshot replay — `bus_tail` is "events from now on" by
    // design; clients use `bus_history` if they want backfill.
    let (_snapshot, mut sub) = bus.subscribe(filter);

    loop {
        match sub.recv().await {
            Ok(env) => {
                if let Some(prefix) = kind_prefix.as_deref() {
                    if !env.kind.starts_with(prefix) {
                        continue;
                    }
                }
                publish_notify(&bus, "bus_tail", &request_id, &env);
            }
            Err(rift_bus::BusError::Lagged(n)) => {
                tracing::warn!("mcp_host: bus_tail subscriber lagged by {n} envelopes");
                // Surface the lag to the client as a notification with no
                // envelope so the stream can resync if it cares.
                publish_notify_raw(
                    &bus,
                    "bus_tail",
                    json!({
                        "request_id": request_id,
                        "lagged": n,
                    }),
                );
            }
            Err(rift_bus::BusError::Closed) => break,
        }
    }
}

fn tool_bus_history(bus: &RiftBus, payload: &Value) -> Result<Value, String> {
    let category_filter = payload
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());
    let limit = payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(1000) as usize;

    let filter = match category_filter {
        None => SubscribeFilter::All,
        Some(c) => match serde_json::from_value::<Category>(Value::String(c.clone())) {
            Ok(cat) => SubscribeFilter::Category(cat),
            Err(_) => return Err(format!("invalid category: {c}")),
        },
    };

    let tail = bus.tail(&filter, limit);
    let count = tail.len();
    let envelopes: Vec<Value> = tail
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "envelopes": envelopes, "count": count }))
}

fn tool_git_status(project_root: &Path) -> Result<Value, String> {
    let snap = git_status::snapshot(project_root)?;
    serde_json::to_value(snap).map_err(|e| e.to_string())
}

fn tool_aegis_state(bus: &RiftBus) -> Result<Value, String> {
    let (snapshot, _sub) = bus.subscribe(SubscribeFilter::Category(Category::Aegis));
    let last = snapshot
        .into_iter()
        .rev()
        .find(|e| e.kind == "aegis.session.skill_loaded");
    match last {
        Some(env) => Ok(env.payload),
        None => Ok(Value::Null),
    }
}

// ---------------------------------------------------------------------------
// Phase B — Tier 1 read tools (D-014 plan §3 Tier 1)
// ---------------------------------------------------------------------------

/// `fs_read` — read a project-relative text file.
///
/// Args: `{ "path": "<relative-to-project-root>" }`. Mirrors the public
/// `fs_read_text` Tauri command's path-validation discipline (root is the
/// current `ProjectRoot`, NOT the launch cwd — post-swap reads land in
/// the swapped project).
fn tool_fs_read(project_root: &Path, payload: &Value) -> Result<Value, String> {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs_read: missing required 'path' argument".to_owned())?;
    let text = read_text(project_root, path).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path, "content": text }))
}

/// `fs_tree` — static snapshot of the project filesystem subtree.
///
/// Args: `{ "max_depth"?: number }`. Defaults to the same constant the
/// Tauri command uses (`FS_TREE_DEFAULT_MAX_DEPTH = 6`). Ignore globs
/// come from the persisted [`RiftConfig`] — same source the watcher uses.
fn tool_fs_tree(
    project_root: &Path,
    app_handle: &AppHandle,
    payload: &Value,
) -> Result<Value, String> {
    let max_depth = payload
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(64) as u32)
        .unwrap_or(rift_bus::FS_TREE_DEFAULT_MAX_DEPTH);

    let ignore_patterns: Vec<String> = app_handle.state::<CachedConfig>().get().fs.ignore_globs;

    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &ignore_patterns {
        let glob = globset::Glob::new(pattern)
            .map_err(|e| format!("fs_tree: invalid ignore glob '{pattern}': {e}"))?;
        builder.add(glob);
    }
    let ignore_globs = builder
        .build()
        .map_err(|e| format!("fs_tree: failed to build GlobSet: {e}"))?;

    let tree = build_tree(project_root, max_depth, &ignore_globs).map_err(|e| e.to_string())?;
    serde_json::to_value(tree).map_err(|e| e.to_string())
}

/// `todo_scan` — scan project source for TODO/FIXME/XXX markers.
///
/// Same payload shape as the TODO notif tab. Ignore globs are loaded from
/// [`RiftConfig`] (matching the public `todo_scan_command`).
fn tool_todo_scan(project_root: &Path, app_handle: &AppHandle) -> Result<Value, String> {
    let ignore_patterns: Vec<String> = app_handle.state::<CachedConfig>().get().fs.ignore_globs;

    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &ignore_patterns {
        let glob = globset::Glob::new(pattern)
            .map_err(|e| format!("todo_scan: invalid ignore glob '{pattern}': {e}"))?;
        builder.add(glob);
    }
    let ignore_globs = builder
        .build()
        .map_err(|e| format!("todo_scan: failed to build GlobSet: {e}"))?;

    let entries = todo_scan::scan_todos(project_root, &ignore_globs);
    let count = entries.len();
    let entries_json = serde_json::to_value(entries).map_err(|e| e.to_string())?;
    Ok(json!({ "entries": entries_json, "count": count }))
}

/// `pty_list` — currently-tracked PTY sessions: `[{ id, alive }]`.
///
/// Phase B v1.0 returns id + liveness. Per-session dimensions are not
/// surfaced here yet; can be added when the registry tracks them
/// alongside [`PtyControl`].
fn tool_pty_list(pty_registry: &PtyRegistry) -> Result<Value, String> {
    let entries: Vec<Value> = pty_registry
        .list()
        .into_iter()
        .map(|(id, alive)| json!({ "id": id, "alive": alive }))
        .collect();
    let count = entries.len();
    Ok(json!({ "sessions": entries, "count": count }))
}

/// `cockpit_state` — last `Category::System / kind="cockpit.state"`
/// snapshot envelope. Producer is `cockpit_window` — the module
/// publishes one envelope at startup and on every detach/reattach. If
/// no producer envelope is in the bus replay buffer, returns
/// `{ detached: false }` (sensible default — bare install isn't
/// detached).
fn tool_cockpit_state(bus: &RiftBus) -> Result<Value, String> {
    let (snapshot, _sub) = bus.subscribe(SubscribeFilter::Category(Category::System));
    let last = snapshot
        .into_iter()
        .rev()
        .find(|e| e.kind == "cockpit.state");
    match last {
        Some(env) => Ok(env.payload),
        None => Ok(json!({ "detached": false })),
    }
}

/// `notif_tabs` — last `Category::System / kind="notif.tabs"` snapshot
/// envelope. Producer is `App.svelte` — it republishes whenever the
/// catalog changes (toggle / reorder / capability detection). If no
/// producer envelope has landed yet (bare boot, before App mounts),
/// returns an empty list rather than erroring.
fn tool_notif_tabs(bus: &RiftBus) -> Result<Value, String> {
    let (snapshot, _sub) = bus.subscribe(SubscribeFilter::Category(Category::System));
    let last = snapshot.into_iter().rev().find(|e| e.kind == "notif.tabs");
    match last {
        Some(env) => Ok(env.payload),
        None => Ok(json!({ "tabs": [] })),
    }
}

// ---------------------------------------------------------------------------
// Phase C — Tier 2 inspection tools (D-014 §3, default-off)
// ---------------------------------------------------------------------------

fn resolve_window_label(payload: &Value) -> &str {
    payload
        .get("window")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
}

async fn eval_js(app_handle: &AppHandle, window_label: &str, js: &str) -> Result<Value, String> {
    let webview = app_handle
        .get_webview_window(window_label)
        .ok_or_else(|| format!("window '{window_label}' not found"))?;

    let callback_id = format!("mcp-eval-{}", uuid::Uuid::new_v4());
    let (tx, rx) = oneshot::channel::<String>();

    let tx = parking_lot::Mutex::new(Some(tx));
    let cb_id_for_listener = callback_id.clone();
    app_handle.once(&cb_id_for_listener, move |event| {
        if let Some(sender) = tx.lock().take() {
            let _ = sender.send(event.payload().to_string());
        }
    });

    let escaped_cb = callback_id.replace('\'', "\\'");
    let eval_script = format!(
        r#"(async function() {{
            const __cbId = '{escaped_cb}';
            const __internals = window.__TAURI_INTERNALS__;
            if (!__internals || !__internals.invoke) {{
                console.error('[rift-mcp] __TAURI_INTERNALS__ unavailable');
                return;
            }}
            const __emit = (evt, payload) => __internals.invoke('plugin:event|emit', {{ event: evt, payload }});
            try {{
                const __result = await (async function() {{ {js} }})();
                const __serialized = (typeof __result === 'string') ? __result : JSON.stringify(__result);
                await __emit(__cbId, {{ ok: true, result: __serialized }});
            }} catch (e) {{
                await __emit(__cbId, {{ ok: false, error: e.message || String(e) }});
            }}
        }})()"#
    );

    webview
        .eval(&eval_script)
        .map_err(|e| format!("eval dispatch failed: {e}"))?;

    let result = tokio::time::timeout(Duration::from_secs(10), rx).await;

    match result {
        Ok(Ok(raw)) => {
            let parsed: Value =
                serde_json::from_str(&raw).map_err(|e| format!("parse eval result: {e}"))?;
            if parsed.get("ok") == Some(&Value::Bool(true)) {
                let inner = parsed
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("null");
                Ok(serde_json::from_str(inner).unwrap_or(Value::String(inner.to_owned())))
            } else {
                Err(parsed
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown JS error")
                    .to_owned())
            }
        }
        Ok(Err(_)) => Err("eval callback channel closed unexpectedly".into()),
        Err(_) => Err("js_eval timed out after 10s".into()),
    }
}

async fn tool_dom_snapshot(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);
    let html = eval_js(
        app_handle,
        window,
        "return document.documentElement.outerHTML;",
    )
    .await?;
    Ok(json!({ "window": window, "html": html }))
}

#[cfg(windows)]
async fn tool_screenshot(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window_label = resolve_window_label(payload);
    let webview = app_handle
        .get_webview_window(window_label)
        .ok_or_else(|| format!("window '{window_label}' not found"))?;

    let hwnd = webview
        .hwnd()
        .map_err(|e| format!("failed to get HWND: {e}"))?;

    let png_bytes = crate::capture::capture_window_png(hwnd.0 as isize)
        .map_err(|e| format!("capture failed: {e}"))?;

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(json!({ "window": window_label, "png_base64": b64 }))
}

#[cfg(not(windows))]
async fn tool_screenshot(_app_handle: &AppHandle, _payload: &Value) -> Result<Value, String> {
    Err("screenshot is only supported on Windows".into())
}

async fn tool_js_eval(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);
    let code = payload
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "js_eval: missing required 'code' argument".to_owned())?;

    // eval_js wraps {js} in `(async function() { {js} })()`, so the code
    // runs as a function body. Bare expressions don't return a value from a
    // function body — only explicit `return` does. Auto-return the last
    // expression so callers get "last expression value" semantics without
    // needing to write `return` manually.
    let trimmed = code.trim();
    let wrapper = if trimmed.contains("return ") || trimmed.contains("return\n") {
        trimmed.to_string()
    } else if let Some(last_sep) = trimmed.rfind([';', '\n']) {
        let (head, tail) = trimmed.split_at(last_sep + 1);
        let tail = tail.trim();
        if tail.is_empty() {
            trimmed.to_string()
        } else {
            format!("{head} return {tail}")
        }
    } else {
        format!("return ({trimmed})")
    };

    let result = eval_js(app_handle, window, &wrapper).await?;
    Ok(json!({ "window": window, "result": result }))
}

// ---------------------------------------------------------------------------
// Phase D — Tier 3 mutating + read tools (D-014 §3)
// ---------------------------------------------------------------------------

/// `pty_input` — write text into a PTY session.
fn tool_pty_input(pty_registry: &PtyRegistry, payload: &Value) -> Result<Value, String> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "pty_input: missing required 'id' argument".to_owned())? as u32;
    let data = payload
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "pty_input: missing required 'data' argument".to_owned())?;

    let control = pty_registry
        .get(id)
        .ok_or_else(|| format!("pty_input: no PTY session with id {id}"))?;

    if !control.is_alive() {
        return Err(format!("pty_input: PTY session {id} is not alive"));
    }

    let bytes = data.as_bytes();
    control
        .write(bytes)
        .map_err(|e| format!("pty_input: write failed: {e}"))?;

    Ok(json!({ "ok": true, "id": id, "bytes_written": bytes.len() }))
}

/// `pty_read` — read the current visible terminal buffer content.
///
/// Uses `eval_js` to read from the xterm.js instance exposed on
/// `window.__RIFT_TERM__` by Terminal.svelte.
async fn tool_pty_read(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let lines_requested = payload
        .get("lines")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(5000) as usize);

    let js = format!(
        r#"
        const term = window.__RIFT_TERM__;
        if (!term) return {{ error: 'no terminal instance' }};
        const buf = term.buffer.active;
        const totalRows = buf.length;
        const viewportRows = term.rows;
        const cols = term.cols;
        const cursorRow = buf.cursorY;
        const cursorCol = buf.cursorX;
        const requested = {lines_arg};
        const count = requested > 0 ? Math.min(requested, totalRows) : viewportRows;
        const startLine = Math.max(0, totalRows - count);
        const lines = [];
        for (let i = startLine; i < totalRows; i++) {{
            const line = buf.getLine(i);
            lines.push(line ? line.translateToString(true) : '');
        }}
        return {{ lines, cursor_row: cursorRow, cursor_col: cursorCol, rows: viewportRows, cols, total_lines: totalRows }};
        "#,
        lines_arg = lines_requested.unwrap_or(0),
    );

    let wrapper = format!("return (function() {{ {js} }})();");
    let window = resolve_window_label(payload);
    let result = eval_js(app_handle, window, &wrapper).await?;

    if result.get("error").is_some() {
        return Err(result["error"].as_str().unwrap_or("unknown").to_owned());
    }

    Ok(result)
}

/// `bus_publish` — publish an envelope to the Rift bus.
fn tool_bus_publish(bus: &RiftBus, payload: &Value) -> Result<Value, String> {
    let category_str = payload
        .get("category")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "bus_publish: missing required 'category' argument".to_owned())?;
    let kind = payload
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "bus_publish: missing required 'kind' argument".to_owned())?;
    let envelope_payload = payload.get("payload").cloned().unwrap_or(json!({}));

    let category: Category = serde_json::from_value(Value::String(category_str.to_owned()))
        .map_err(|_| format!("bus_publish: invalid category: {category_str}"))?;

    let env = Envelope::new(category, kind)
        .with_payload(&envelope_payload)
        .map_err(|e| format!("bus_publish: envelope creation failed: {e}"))?;

    bus.publish(env);
    Ok(json!({ "ok": true, "category": category_str, "kind": kind }))
}

/// `fs_write` — write text content to a project-relative file.
fn tool_fs_write(project_root: &Path, payload: &Value) -> Result<Value, String> {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs_write: missing required 'path' argument".to_owned())?;
    let content = payload
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs_write: missing required 'content' argument".to_owned())?;

    write_text(project_root, path, content).map_err(|e| format!("fs_write: {e}"))?;
    Ok(json!({ "ok": true, "path": path, "bytes_written": content.len() }))
}

/// `git_action` — run a git mutating action in the project root.
fn tool_git_action(project_root: &Path, payload: &Value) -> Result<Value, String> {
    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "git_action: missing required 'action' argument".to_owned())?;
    let message = payload.get("message").and_then(|v| v.as_str());

    let result = git_status::run_action(project_root, action, message)
        .map_err(|e| format!("git_action: {e}"))?;
    serde_json::to_value(result).map_err(|e| format!("git_action: serialize failed: {e}"))
}

// ---------------------------------------------------------------------------
// Post-Phase D — diagnostic + runtime config tools
// ---------------------------------------------------------------------------

/// `rift_diagnose` — return Rift terminal health metrics for diagnosis.
///
/// Read-only. No permission gate required. Gathers version, active PTY
/// sessions, bus subscriber/replay counts, recent system-error count,
/// and current terminal + MCP config sections.
fn tool_rift_diagnose(
    bus: &RiftBus,
    pty_registry: &PtyRegistry,
    app_handle: &AppHandle,
) -> Result<Value, String> {
    // PTY sessions snapshot.
    let pty_entries: Vec<Value> = pty_registry
        .list()
        .into_iter()
        .map(|(id, alive)| json!({ "id": id, "alive": alive }))
        .collect();
    let pty_count = pty_entries.len();

    // Bus metrics.
    let bus_subscribers = bus.subscriber_count();
    let bus_replay_len = bus.replay_len();

    // Count recent system-category envelopes whose kind contains "error".
    let (system_snapshot, _sub) = bus.subscribe(SubscribeFilter::Category(Category::System));
    let recent_errors = system_snapshot
        .iter()
        .filter(|e| e.kind.contains("error"))
        .count();

    // Terminal + MCP config from CachedConfig.
    let cfg = app_handle.state::<CachedConfig>().get();
    let terminal = &cfg.terminal;
    let mcp = &cfg.mcp;

    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pty_sessions": pty_entries,
        "pty_count": pty_count,
        "bus_subscribers": bus_subscribers,
        "bus_replay_len": bus_replay_len,
        "recent_errors": recent_errors,
        "terminal_config": {
            "font_size": terminal.font_size,
            "font_family": terminal.font_family,
            "line_height": terminal.line_height,
            "scrollback": terminal.scrollback,
            "lanes_enabled": terminal.lanes_enabled,
            "shell": format!("{:?}", terminal.shell),
        },
        "mcp_config": {
            "enabled": mcp.enabled,
            "allow_inspection": mcp.allow_inspection,
            "allow_mutations": mcp.allow_mutations,
        },
    }))
}

/// `rift_config_set` — update terminal configuration at runtime.
///
/// Requires `mcp.allow_mutations = true`. Applies only the fields present
/// in the payload (partial update). Validates ranges and persists to disk.
fn tool_rift_config_set(
    bus: &RiftBus,
    app_handle: &AppHandle,
    payload: &Value,
) -> Result<Value, String> {
    let cached = app_handle.state::<CachedConfig>();
    let mut cfg = cached.get();

    // Track whether any field was actually updated.
    let mut changed = false;

    // font_size: 8..=48
    if let Some(v) = payload.get("font_size").and_then(|v| v.as_u64()) {
        let v = v as u16;
        if !(8..=48).contains(&v) {
            return Err(format!("rift_config_set: font_size {v} out of range 8-48"));
        }
        cfg.terminal.font_size = v;
        changed = true;
    }

    // font_family: CSS font-family string
    if let Some(v) = payload.get("font_family").and_then(|v| v.as_str()) {
        let v = v.trim();
        if v.is_empty() {
            return Err("rift_config_set: font_family cannot be empty".into());
        }
        cfg.terminal.font_family = v.to_string();
        changed = true;
    }

    // line_height: 1.0..=2.5
    if let Some(v) = payload.get("line_height").and_then(|v| v.as_f64()) {
        let v = v as f32;
        if !(1.0..=2.5).contains(&v) {
            return Err(format!(
                "rift_config_set: line_height {v} out of range 1.0-2.5"
            ));
        }
        cfg.terminal.line_height = v;
        changed = true;
    }

    // scrollback: 100..=100_000
    if let Some(v) = payload.get("scrollback").and_then(|v| v.as_u64()) {
        let v = v as u32;
        if !(100..=100_000).contains(&v) {
            return Err(format!(
                "rift_config_set: scrollback {v} out of range 100-100000"
            ));
        }
        cfg.terminal.scrollback = v;
        changed = true;
    }

    // lanes_enabled: bool
    if let Some(v) = payload.get("lanes_enabled").and_then(|v| v.as_bool()) {
        cfg.terminal.lanes_enabled = v;
        changed = true;
    }

    // shell: string → ShellPref
    if let Some(v) = payload.get("shell").and_then(|v| v.as_str()) {
        let pref = match v {
            "auto" => ShellPref::Auto,
            "pwsh" => ShellPref::Pwsh,
            "powershell" => ShellPref::Powershell,
            "cmd" => ShellPref::Cmd,
            "bash" => ShellPref::Bash,
            "zsh" => ShellPref::Zsh,
            "sh" => ShellPref::Sh,
            other => {
                return Err(format!(
                    "rift_config_set: unknown shell '{other}'. Valid: auto, pwsh, powershell, cmd, bash, zsh, sh"
                ))
            }
        };
        cfg.terminal.shell = pref;
        changed = true;
    }

    if !changed {
        return Err(
            "rift_config_set: no recognized fields provided. Supply at least one of: font_size, font_family, line_height, scrollback, lanes_enabled, shell".into(),
        );
    }

    // Persist to disk.
    save_config(&cfg).map_err(|e| {
        let msg = format!("rift_config_set: save failed: {e}");
        publish_error(bus, "mcp.rift_config_set", &msg, None);
        msg
    })?;

    // Update the in-memory cache.
    cached.set(cfg.clone());

    // Publish config.changed so the frontend can react without a page reload.
    if let Ok(env) =
        Envelope::new(Category::System, "config.changed").with_payload(&json!({ "source": "mcp" }))
    {
        bus.publish(env);
    }

    let terminal = &cfg.terminal;
    Ok(json!({
        "ok": true,
        "terminal_config": {
            "font_size": terminal.font_size,
            "font_family": terminal.font_family,
            "line_height": terminal.line_height,
            "scrollback": terminal.scrollback,
            "lanes_enabled": terminal.lanes_enabled,
            "shell": format!("{:?}", terminal.shell),
        },
    }))
}

/// Escape a string for safe interpolation into a JavaScript single-quoted literal.
/// Handles backslash, quotes, backticks, newlines, carriage returns, and null bytes.
fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '`' => out.push_str("\\`"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out
}

/// `simulate_click` — dispatch a synthetic click event at coordinates or a CSS selector.
async fn tool_simulate_click(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);

    let target_js = if let Some(selector) = payload.get("selector").and_then(|v| v.as_str()) {
        let escaped = escape_js_string(selector);
        format!(
            "const el = document.querySelector('{escaped}'); \
             if (!el) return {{ error: 'selector not found: {escaped}' }}; \
             const r = el.getBoundingClientRect(); \
             const x = r.left + r.width / 2; \
             const y = r.top + r.height / 2;"
        )
    } else {
        let x = payload
            .get("x")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "simulate_click: need 'selector' or 'x'+'y' coordinates".to_owned())?;
        let y = payload
            .get("y")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "simulate_click: need 'y' coordinate".to_owned())?;
        format!("const x = {x}; const y = {y}; const el = document.elementFromPoint(x, y);")
    };

    let js = format!(
        r#"
        {target_js}
        if (el) {{
            el.dispatchEvent(new MouseEvent('mousedown', {{ clientX: x, clientY: y, bubbles: true }}));
            el.dispatchEvent(new MouseEvent('mouseup', {{ clientX: x, clientY: y, bubbles: true }}));
            el.dispatchEvent(new MouseEvent('click', {{ clientX: x, clientY: y, bubbles: true }}));
        }}
        return {{ ok: true, x, y, tag: el ? el.tagName : null }};
        "#
    );

    let wrapper = format!("return (function() {{ {js} }})();");
    let result = eval_js(app_handle, window, &wrapper).await?;

    if result.get("error").is_some() {
        return Err(result["error"].as_str().unwrap_or("unknown").to_owned());
    }
    Ok(result)
}

/// `simulate_drag` — dispatch synthetic mousedown→mousemove→mouseup between two points.
async fn tool_simulate_drag(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);

    let from_js = if let Some(selector) = payload.get("from_selector").and_then(|v| v.as_str()) {
        let escaped = escape_js_string(selector);
        format!(
            "const fromEl = document.querySelector('{escaped}'); \
                 if (!fromEl) return {{ error: 'from_selector not found: {escaped}' }}; \
                 const fr = fromEl.getBoundingClientRect(); \
                 const fx = fr.left + fr.width / 2; \
                 const fy = fr.top + fr.height / 2;"
        )
    } else {
        let fx = payload
            .get("from_x")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "simulate_drag: need 'from_selector' or 'from_x'+'from_y'".to_owned())?;
        let fy = payload
            .get("from_y")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "simulate_drag: need 'from_y'".to_owned())?;
        format!(
            "const fx = {fx}; const fy = {fy}; const fromEl = document.elementFromPoint(fx, fy);"
        )
    };

    let to_js = if let Some(selector) = payload.get("to_selector").and_then(|v| v.as_str()) {
        let escaped = escape_js_string(selector);
        format!(
            "const toEl = document.querySelector('{escaped}'); \
             if (!toEl) return {{ error: 'to_selector not found: {escaped}' }}; \
             const tr = toEl.getBoundingClientRect(); \
             const tx = tr.left + tr.width / 2; \
             const ty = tr.top + tr.height / 2;"
        )
    } else {
        let tx = payload
            .get("to_x")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "simulate_drag: need 'to_selector' or 'to_x'+'to_y'".to_owned())?;
        let ty = payload
            .get("to_y")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "simulate_drag: need 'to_y'".to_owned())?;
        format!("const tx = {tx}; const ty = {ty};")
    };

    let js = format!(
        r#"
        {from_js}
        {to_js}
        if (fromEl) {{
            fromEl.dispatchEvent(new MouseEvent('mousedown', {{ clientX: fx, clientY: fy, bubbles: true }}));
            const steps = 5;
            for (let i = 1; i <= steps; i++) {{
                const mx = fx + (tx - fx) * (i / steps);
                const my = fy + (ty - fy) * (i / steps);
                document.dispatchEvent(new MouseEvent('mousemove', {{ clientX: mx, clientY: my, bubbles: true }}));
            }}
            const dropTarget = document.elementFromPoint(tx, ty);
            if (dropTarget) {{
                dropTarget.dispatchEvent(new MouseEvent('mouseup', {{ clientX: tx, clientY: ty, bubbles: true }}));
            }}
        }}
        return {{ ok: true, from: {{ x: fx, y: fy }}, to: {{ x: tx, y: ty }} }};
        "#
    );

    let wrapper = format!("return (function() {{ {js} }})();");
    let result = eval_js(app_handle, window, &wrapper).await?;

    if result.get("error").is_some() {
        return Err(result["error"].as_str().unwrap_or("unknown").to_owned());
    }
    Ok(result)
}

fn publish_response(bus: &RiftBus, tool: &str, request_id: Value, result: Result<Value, String>) {
    let kind = format!("mcp.response.{tool}");
    let corr_id = request_id.as_str().map(|s| format!("mcp-{s}"));
    let payload = match result {
        Ok(v) => json!({ "request_id": request_id, "ok": true, "result": v }),
        Err(e) => json!({ "request_id": request_id, "ok": false, "error": e }),
    };
    if let Ok(mut env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        env.correlation_id = corr_id;
        bus.publish(env);
    }
}

fn publish_audit(bus: &RiftBus, kind: &str, payload: Value) {
    let corr_id = payload
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(|s| format!("mcp-{s}"));
    if let Ok(mut env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        env.correlation_id = corr_id;
        bus.publish(env);
    }
}

/// Publish a streaming-tool notification — `mcp.notify.{tool}` envelope
/// carrying `{ request_id, envelope }`. The rift-mcp translator's
/// notification forwarder converts these to JSON-RPC notifications on
/// stdout (method `notifications/rift/{tool}`).
fn publish_notify(bus: &RiftBus, tool: &str, request_id: &Value, envelope: &Envelope) {
    let kind = format!("mcp.notify.{tool}");
    let inner = serde_json::to_value(envelope).unwrap_or(Value::Null);
    let payload = json!({
        "request_id": request_id.clone(),
        "envelope": inner,
    });
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

/// Lower-level notify helper — emit a `mcp.notify.{tool}` with a custom
/// payload (e.g. lag sentinel). Use [`publish_notify`] for the standard
/// envelope-carrying form.
fn publish_notify_raw(bus: &RiftBus, tool: &str, payload: Value) {
    let kind = format!("mcp.notify.{tool}");
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

/// Constant-time byte comparison to defeat timing-based token leakage.
/// The XOR loop always runs for `min(a.len(), b.len())` iterations regardless
/// of where the first mismatch occurs, and length inequality is folded into
/// the accumulator rather than early-returned.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() != b.len()) as u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// (Audit publishes use `publish_audit` directly — no extension trait needed.)
