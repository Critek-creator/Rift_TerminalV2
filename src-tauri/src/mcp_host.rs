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
    build_tree, load_config, publish_error, read_text, write_text, Category, Envelope, McpConfig,
    RiftBus, SubscribeFilter,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::oneshot;

use crate::{git_status, todo_scan, PtyRegistry};

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

    for env in snapshot {
        handle_envelope(
            &bus,
            &cfg,
            &project_root,
            token_present,
            &pty_registry,
            &app_handle,
            &env,
        );
    }

    loop {
        match sub.recv().await {
            Ok(env) => handle_envelope(
                &bus,
                &cfg,
                &project_root,
                token_present,
                &pty_registry,
                &app_handle,
                &env,
            ),
            Err(rift_bus::BusError::Lagged(n)) => {
                tracing::warn!("mcp_host: subscriber lagged by {n} envelopes");
            }
            Err(rift_bus::BusError::Closed) => break,
        }
    }
}

fn handle_envelope(
    bus: &RiftBus,
    cfg: &McpConfig,
    project_root: &Path,
    token_present: bool,
    pty_registry: &PtyRegistry,
    app_handle: &AppHandle,
    env: &Envelope,
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
            let result = start_bus_tail(bus.clone(), request_id.clone(), &env.payload);
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
        );
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

fn dispatch_tool(
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
        "fs_tree" => tool_fs_tree(project_root, payload),
        "todo_scan" => tool_todo_scan(project_root),
        "pty_list" => tool_pty_list(pty_registry),
        "cockpit_state" => tool_cockpit_state(bus),
        "notif_tabs" => tool_notif_tabs(bus),
        // Phase C — Tier 2 inspection tools (D-014 §3, default-off)
        "dom_snapshot" => {
            if !cfg.allow_inspection {
                return Err("dom_snapshot requires mcp.allow_inspection = true".into());
            }
            tool_dom_snapshot(app_handle, payload)
        }
        "screenshot" => {
            if !cfg.allow_inspection {
                return Err("screenshot requires mcp.allow_inspection = true".into());
            }
            tool_screenshot(app_handle, payload)
        }
        "js_eval" => {
            if !cfg.allow_inspection {
                return Err("js_eval requires mcp.allow_inspection = true".into());
            }
            if !cfg.allow_js_eval {
                return Err("js_eval requires mcp.allow_js_eval = true".into());
            }
            tool_js_eval(app_handle, payload)
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
            tool_pty_read(app_handle, payload)
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
            tool_simulate_click(app_handle, payload)
        }
        "simulate_drag" => {
            if !cfg.allow_inspection {
                return Err("simulate_drag requires mcp.allow_inspection = true".into());
            }
            if !cfg.allow_mutations {
                return Err("simulate_drag requires mcp.allow_mutations = true".into());
            }
            tool_simulate_drag(app_handle, payload)
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
fn start_bus_tail(bus: RiftBus, request_id: Value, payload: &Value) -> Result<Value, String> {
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
    tauri::async_runtime::spawn(async move {
        run_bus_tail(
            bus_for_task,
            request_id_for_task,
            kind_prefix_for_task,
            filter,
        )
        .await;
    });

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

    let (snapshot, _sub) = bus.subscribe(filter);
    let take_from = snapshot.len().saturating_sub(limit);
    let envelopes: Vec<Value> = snapshot
        .into_iter()
        .skip(take_from)
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    let count = envelopes.len();
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
fn tool_fs_tree(project_root: &Path, payload: &Value) -> Result<Value, String> {
    let max_depth = payload
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(64) as u32)
        .unwrap_or(rift_bus::FS_TREE_DEFAULT_MAX_DEPTH);

    let ignore_patterns: Vec<String> =
        load_config()
            .map(|cfg| cfg.fs.ignore_globs)
            .unwrap_or_else(|_| {
                rift_bus::DEFAULT_IGNORE_GLOBS
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            });

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
fn tool_todo_scan(project_root: &Path) -> Result<Value, String> {
    let ignore_patterns: Vec<String> =
        load_config()
            .map(|cfg| cfg.fs.ignore_globs)
            .unwrap_or_else(|_| {
                rift_bus::DEFAULT_IGNORE_GLOBS
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            });

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

fn eval_js_blocking(app_handle: &AppHandle, window_label: &str, js: &str) -> Result<Value, String> {
    let webview = app_handle
        .get_webview_window(window_label)
        .ok_or_else(|| format!("window '{window_label}' not found"))?;

    let callback_id = format!("mcp-eval-{}", uuid::Uuid::new_v4());
    let (tx, rx) = oneshot::channel::<String>();

    let tx = std::sync::Mutex::new(Some(tx));
    let cb_id_for_listener = callback_id.clone();
    app_handle.once(&cb_id_for_listener, move |event| {
        if let Some(sender) = tx.lock().unwrap().take() {
            let _ = sender.send(event.payload().to_string());
        }
    });

    let escaped_cb = callback_id.replace('\'', "\\'");
    let eval_script = format!(
        r#"(async function() {{
            const __cbId = '{escaped_cb}';
            try {{
                const __result = await (async function() {{ {js} }})();
                const __serialized = (typeof __result === 'string') ? __result : JSON.stringify(__result);
                window.__TAURI__.event.emit(__cbId, JSON.stringify({{ ok: true, result: __serialized }}));
            }} catch (e) {{
                window.__TAURI__.event.emit(__cbId, JSON.stringify({{ ok: false, error: e.message || String(e) }}));
            }}
        }})()"#
    );

    webview
        .eval(&eval_script)
        .map_err(|e| format!("eval dispatch failed: {e}"))?;

    let result = tauri::async_runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(10), rx).await
    });

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

fn tool_dom_snapshot(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);
    let html = eval_js_blocking(
        app_handle,
        window,
        "return document.documentElement.outerHTML;",
    )?;
    Ok(json!({ "window": window, "html": html }))
}

fn tool_screenshot(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);
    let result = eval_js_blocking(
        app_handle,
        window,
        r#"
            const canvas = document.createElement('canvas');
            const rect = document.documentElement.getBoundingClientRect();
            canvas.width = window.innerWidth;
            canvas.height = window.innerHeight;
            const ctx = canvas.getContext('2d');
            const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${canvas.width}" height="${canvas.height}">
                <foreignObject width="100%" height="100%">
                    <div xmlns="http://www.w3.org/1999/xhtml">${document.documentElement.outerHTML}</div>
                </foreignObject>
            </svg>`;
            const img = new Image();
            const blob = new Blob([svg], { type: 'image/svg+xml;charset=utf-8' });
            const url = URL.createObjectURL(blob);
            await new Promise((resolve, reject) => {
                img.onload = resolve;
                img.onerror = () => reject(new Error('SVG render failed — screenshot unavailable (cross-origin or tainted canvas)'));
                img.src = url;
            });
            ctx.drawImage(img, 0, 0);
            URL.revokeObjectURL(url);
            return canvas.toDataURL('image/png');
        "#,
    )?;
    let data_url = result.as_str().unwrap_or("");
    let base64 = data_url
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(data_url);
    Ok(json!({ "window": window, "png_base64": base64, "width": null, "height": null }))
}

fn tool_js_eval(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);
    let code = payload
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "js_eval: missing required 'code' argument".to_owned())?;
    let wrapper = format!("return (async function() {{ {code} }})();");
    let result = eval_js_blocking(app_handle, window, &wrapper)?;
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
/// Uses `eval_js_blocking` to read from the xterm.js instance exposed on
/// `window.__RIFT_TERM__` by Terminal.svelte.
fn tool_pty_read(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
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
    let result = eval_js_blocking(app_handle, "main", &wrapper)?;

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

/// `simulate_click` — dispatch a synthetic click event at coordinates or a CSS selector.
fn tool_simulate_click(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);

    let target_js = if let Some(selector) = payload.get("selector").and_then(|v| v.as_str()) {
        let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
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
    let result = eval_js_blocking(app_handle, window, &wrapper)?;

    if result.get("error").is_some() {
        return Err(result["error"].as_str().unwrap_or("unknown").to_owned());
    }
    Ok(result)
}

/// `simulate_drag` — dispatch synthetic mousedown→mousemove→mouseup between two points.
fn tool_simulate_drag(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    let window = resolve_window_label(payload);

    let from_js = if let Some(selector) = payload.get("from_selector").and_then(|v| v.as_str()) {
        let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
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
        let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
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
    let result = eval_js_blocking(app_handle, window, &wrapper)?;

    if result.get("error").is_some() {
        return Err(result["error"].as_str().unwrap_or("unknown").to_owned());
    }
    Ok(result)
}

fn publish_response(bus: &RiftBus, tool: &str, request_id: Value, result: Result<Value, String>) {
    let kind = format!("mcp.response.{tool}");
    let payload = match result {
        Ok(v) => json!({ "request_id": request_id, "ok": true, "result": v }),
        Err(e) => json!({ "request_id": request_id, "ok": false, "error": e }),
    };
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
        bus.publish(env);
    }
}

fn publish_audit(bus: &RiftBus, kind: &str, payload: Value) {
    if let Ok(env) = Envelope::new(Category::Mcp, kind).with_payload(&payload) {
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
/// Returns false on length mismatch without scanning further.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// (Audit publishes use `publish_audit` directly — no extension trait needed.)
