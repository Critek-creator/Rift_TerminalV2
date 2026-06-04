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
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex as ParkingMutex;
use rift_bus::{
    build_tree, publish_error, read_text, save_config, write_text, Category, Envelope,
    ErrorHandoffMode, McpConfig, RiftBus, ShellPref, SubscribeFilter,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::{oneshot, Mutex as TokioMutex};

use crate::{git_status, todo_scan, CachedConfig, PtyRegistry};

/// Serializes inspection tools (screenshot, js_eval, dom_snapshot) so they
/// never race each other or overlap with user interaction on the webview.
static INSPECTION_LOCK: once_cell::sync::Lazy<TokioMutex<()>> =
    once_cell::sync::Lazy::new(|| TokioMutex::new(()));

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

    // Defence-in-depth: tool dispatch is gated on a successful handshake.
    // Without this, any local process with bus access could invoke tools
    // before authenticating via the token handshake.
    let handshake_complete = Arc::new(ParkingMutex::new(false));

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
            handshake_complete,
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
    handshake_complete: Arc<ParkingMutex<bool>>,
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
            &handshake_complete,
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
                    &handshake_complete,
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
    handshake_complete: &Arc<ParkingMutex<bool>>,
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
        handle_handshake(bus, env, token_present, handshake_complete);
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

        // Defence-in-depth: reject tool invocations before handshake.
        if !*handshake_complete.lock() {
            tracing::warn!("mcp_host: rejecting {tool_name} — handshake not completed");
            publish_response(
                bus,
                tool_name,
                request_id,
                Err("MCP handshake required before tool invocation".into()),
            );
            return;
        }

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

fn handle_handshake(
    bus: &RiftBus,
    env: &Envelope,
    token_present: bool,
    handshake_complete: &Arc<ParkingMutex<bool>>,
) {
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
        *handshake_complete.lock() = true;
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

/// `session_compact` — force a one-shot compaction of the newest session log
/// (summarize the older prefix → sidecar; audit `.jsonl` untouched). Mutating
/// (writes the sidecar) → the caller gates it on `allow_mutations`. Uses the
/// shared summarizer factory (currently-serving local model; `Err` if none).
async fn tool_session_compact(app_handle: &AppHandle) -> Result<Value, String> {
    let cfg = rift_bus::config::load_config().map_err(|e| format!("config load: {e}"))?;
    let pm = app_handle
        .state::<std::sync::Arc<rift_bus::translators::llm_process::ProcessManager>>()
        .inner()
        .clone();
    let summarizer = crate::llm_commands::build_summarizer_factory(pm);
    let (id, summary) = rift_bus::compact_now(&cfg.session, &summarizer).await?;
    Ok(json!({
        "session_id": id,
        "summary_chars": summary.len(),
        "summary": summary,
    }))
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
            let _guard = INSPECTION_LOCK.lock().await;
            tool_dom_snapshot(app_handle, payload).await
        }
        "screenshot" => {
            if !cfg.allow_inspection {
                return Err("screenshot requires mcp.allow_inspection = true".into());
            }
            let _guard = INSPECTION_LOCK.lock().await;
            tool_screenshot(app_handle, payload).await
        }
        "js_eval" => {
            if !cfg.allow_inspection {
                return Err("js_eval requires mcp.allow_inspection = true".into());
            }
            if !cfg.allow_js_eval {
                return Err("js_eval requires mcp.allow_js_eval = true".into());
            }
            let _guard = INSPECTION_LOCK.lock().await;
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
        "session_compact" => {
            if !cfg.allow_mutations {
                return Err("session_compact requires mcp.allow_mutations = true".into());
            }
            tool_session_compact(app_handle).await
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
        // Ensemble Router — LLM tools
        "llm_models" => tool_llm_models(),
        "llm_switch" => tool_llm_switch(bus, payload),
        "llm_health" => tool_llm_health(payload).await,
        "llm_prompt" => tool_llm_prompt(bus, app_handle, payload).await,
        "llm_ensemble" => tool_llm_ensemble(bus, payload).await,
        // Single forced tool call by a local model (Path C spike). Reuses the
        // full dispatch context to run ONE read-only tool through dispatch_tool.
        "llm_tool_call" => {
            run_local_tool_call(bus, cfg, project_root, pty_registry, app_handle, payload).await
        }
        // Process management — local llama-server lifecycle
        "llm_process_start" => {
            if !cfg.allow_mutations {
                return Err("llm_process_start requires mcp.allow_mutations = true".into());
            }
            tool_llm_process_start(app_handle, payload)
        }
        "llm_process_stop" => {
            if !cfg.allow_mutations {
                return Err("llm_process_stop requires mcp.allow_mutations = true".into());
            }
            tool_llm_process_stop(app_handle, payload)
        }
        "llm_model_apply_config" => {
            if !cfg.allow_mutations {
                return Err("llm_model_apply_config requires mcp.allow_mutations = true".into());
            }
            tool_llm_apply_config(app_handle, payload)
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

    // Same overflow guard as `screenshot`: a large DOM serialized inline blows
    // the MCP result token budget. Return the HTML inline when it's small,
    // otherwise spill it to a stable per-window temp file and return the path
    // (callers Read it). The `inline` flag lets callers branch deterministically.
    const INLINE_MAX: usize = 30_000;
    let html_str = html.as_str().unwrap_or_default();
    if html_str.len() <= INLINE_MAX {
        Ok(json!({ "window": window, "inline": true, "html": html_str }))
    } else {
        let path = std::env::temp_dir().join(format!("rift-dom-{window}.html"));
        std::fs::write(&path, html_str)
            .map_err(|e| format!("failed to write DOM snapshot to {}: {e}", path.display()))?;
        Ok(json!({
            "window": window,
            "inline": false,
            "path": path.to_string_lossy(),
            "bytes": html_str.len(),
        }))
    }
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

    // Run GDI capture on a blocking thread so it never contends with the
    // async runtime, and catch_unwind inside capture_window_png absorbs
    // any HWND-went-stale panic.
    let hwnd_val = hwnd.0 as isize;
    let png_bytes =
        tokio::task::spawn_blocking(move || crate::capture::capture_window_png(hwnd_val))
            .await
            .map_err(|e| format!("capture task panicked: {e}"))?
            .map_err(|e| format!("capture failed: {e}"))?;

    // Write the PNG to a stable temp path (one file per window) and return the
    // path rather than a multi-megabyte base64 blob — a 1.4 MB base64 string
    // overflowed the MCP result token budget, forcing callers to decode-to-file
    // by hand. INSPECTION_LOCK serializes inspection tools, so the fixed
    // per-window filename has no write race. Callers Read the path to view it.
    let (width, height) = png_dimensions(&png_bytes).unwrap_or((0, 0));
    let path = std::env::temp_dir().join(format!("rift-screenshot-{window_label}.png"));
    std::fs::write(&path, &png_bytes)
        .map_err(|e| format!("failed to write screenshot to {}: {e}", path.display()))?;

    Ok(json!({
        "window": window_label,
        "path": path.to_string_lossy(),
        "bytes": png_bytes.len(),
        "width": width,
        "height": height,
    }))
}

/// Parse width/height from a PNG's IHDR chunk: 8-byte signature, then a
/// length+type header, then IHDR data — width and height as big-endian u32 at
/// byte offsets 16 and 20. Returns None if the buffer isn't a PNG.
#[cfg(windows)]
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
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

    // eval_js wraps {js} in `(async function() { {js} })()`, so whatever we
    // pass must `return` the value the caller wants. Rather than parse JS in
    // Rust to locate the "last expression" — the old heuristic silently dropped
    // the value whenever the code ended in `;`/newline, and a nested `return`
    // in a callback disabled it entirely — hand the raw code to `eval` and
    // return its COMPLETION VALUE, the same "last expression wins" semantics a
    // browser console gives. `eval("const x=1; x;")` is `1`, so declarations,
    // trailing semicolons and nested returns all just work; `await` resolves the
    // value when the final expression is a Promise (e.g. an async IIFE).
    //
    // Indirect `(0,eval)` runs in global scope, so user code can't see our
    // wrapper locals and its `let`/`const` don't leak. If the caller wrote a
    // top-level `return` (the old contract) eval throws "Illegal return
    // statement"; we catch that one case and re-run the code as a function body
    // so `return X` keeps working. Top-level `await` is unsupported — wrap async
    // work in `(async () => { ... })()`.
    let code_literal =
        serde_json::to_string(code).map_err(|e| format!("js_eval: failed to encode code: {e}"))?;
    let wrapper = format!(
        r#"const __c = {code_literal};
try {{ return await (0, eval)(__c); }}
catch (__e) {{
  if (__e instanceof SyntaxError && /return/i.test(__e.message)) {{
    return await (0, eval)('(async function(){{' + __c + '\n}})()');
  }}
  throw __e;
}}"#
    );

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
            "color_palette": terminal.color_palette,
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

    // color_palette: string
    if let Some(v) = payload.get("color_palette").and_then(|v| v.as_str()) {
        let v = v.trim();
        if v.is_empty() {
            return Err("rift_config_set: color_palette cannot be empty".into());
        }
        cfg.terminal.color_palette = v.to_string();
        changed = true;
    }

    // custom_palette: Record<string, string>
    if let Some(v) = payload.get("custom_palette").and_then(|v| v.as_object()) {
        let mut map = std::collections::HashMap::new();
        for (k, val) in v {
            if let Some(s) = val.as_str() {
                map.insert(k.clone(), s.to_string());
            }
        }
        cfg.terminal.custom_palette = map;
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

    // error_handoff_mode: string → ErrorHandoffMode (Phase 5 / R2)
    if let Some(v) = payload.get("error_handoff_mode").and_then(|v| v.as_str()) {
        let mode = match v {
            "off" => ErrorHandoffMode::Off,
            "detect" => ErrorHandoffMode::Detect,
            "assist" => ErrorHandoffMode::Assist,
            other => {
                return Err(format!(
                "rift_config_set: unknown error_handoff_mode '{other}'. Valid: off, detect, assist"
            ))
            }
        };
        cfg.error_handoff.mode = mode;
        changed = true;
    }

    if !changed {
        return Err(
            "rift_config_set: no recognized fields provided. Supply at least one of: font_size, font_family, line_height, scrollback, lanes_enabled, shell, error_handoff_mode".into(),
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
            "color_palette": terminal.color_palette,
            "shell": format!("{:?}", terminal.shell),
        },
    }))
}

/// Escape a string for safe interpolation into a JavaScript single-quoted literal.
/// Handles backslash, quotes, backticks, newlines, carriage returns, null bytes,
/// and Unicode directional override / isolate characters (visual spoofing defence).
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
            // Strip Unicode bidi override / mark / isolate characters that
            // could cause visual spoofing in error messages or eval output.
            '\u{200E}' // LEFT-TO-RIGHT MARK
            | '\u{200F}' // RIGHT-TO-LEFT MARK
            | '\u{202E}' // RIGHT-TO-LEFT OVERRIDE
            | '\u{2066}' // LEFT-TO-RIGHT ISOLATE
            | '\u{2067}' // RIGHT-TO-LEFT ISOLATE
            | '\u{2068}' // FIRST STRONG ISOLATE
            | '\u{2069}' // POP DIRECTIONAL ISOLATE
            => {} // stripped — intentionally produces no output
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

/// `llm_ensemble` — send the same prompt to two models in parallel via MCP.
///
/// Non-streaming (MCP is non-interactive). Dispatches `provider.complete()`
/// for both models via `tokio::join!`, then optionally runs a critique step
/// (model B reviews model A's output with a review system prompt).
///
/// Publishes `llm.ensemble.start` and `llm.ensemble.complete` bus events.
/// Returns JSON with both model results + optional critique + total_cost_usd.
async fn tool_llm_ensemble(bus: &RiftBus, payload: &Value) -> Result<Value, String> {
    use rift_bus::translators::llm::{CompletionRequest, LlmProvider, Message, Role};

    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or("llm_ensemble: missing required argument: prompt")?;

    let model_a_override = payload.get("model_a").and_then(|v| v.as_str());
    let model_b_override = payload.get("model_b").and_then(|v| v.as_str());
    let run_critique = payload
        .get("critique")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let router = rift_router::RouterService::new(config.ensemble.clone());

    let parsed = rift_router::parse_model_tag(prompt);
    let clean_prompt = &parsed.clean_prompt;
    let task_type = rift_router::classifier::classify(clean_prompt);

    // Resolve the two model IDs — explicit args take priority, otherwise
    // pick two diverse models via the same logic the Tauri command uses.
    let (id_a, id_b) = match (model_a_override, model_b_override) {
        (Some(a), Some(b)) => (a.to_owned(), b.to_owned()),
        _ => {
            // Inline of pick_two_models (private to llm_commands) —
            // prefer different providers for maximum perspective diversity.
            let models = router.models();
            if models.len() < 2 {
                return Err("llm_ensemble requires at least 2 configured models".into());
            }
            let tag = rift_router::profiles::task_type_tag(&task_type);
            let mut ranked: Vec<&rift_bus::config::ModelConfig> = models.iter().collect();
            ranked.sort_by(|a, b| {
                let a_match = a.capabilities.strength_tags.iter().any(|t| t == tag);
                let b_match = b.capabilities.strength_tags.iter().any(|t| t == tag);
                b_match.cmp(&a_match).then_with(|| {
                    b.capabilities
                        .cost_per_1m_input
                        .partial_cmp(&a.capabilities.cost_per_1m_input)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
            let first = ranked[0].id.clone();
            let second = ranked[1..]
                .iter()
                .find(|m| m.provider != ranked[0].provider)
                .unwrap_or(&ranked[1])
                .id
                .clone();
            (first, second)
        }
    };

    let model_a = router
        .find_model(&id_a)
        .map_err(|e| format!("{e}"))?
        .clone();
    let model_b = router
        .find_model(&id_b)
        .map_err(|e| format!("{e}"))?
        .clone();

    // Publish ensemble start
    if let Ok(env) = Envelope::new(Category::Llm, "llm.ensemble.start").with_payload(&json!({
        "model_a": id_a,
        "model_b": id_b,
        "task_type": format!("{task_type:?}"),
        "source": "mcp",
    })) {
        bus.publish(env);
    }

    // Build completion requests — identical prompt, no streaming
    let make_request = |p: &str| CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: p.to_owned(),
        }],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: None,
        provider_options: None,
    };

    let provider_a: Box<dyn LlmProvider> = crate::llm_commands::create_provider(&model_a)?;
    let provider_b: Box<dyn LlmProvider> = crate::llm_commands::create_provider(&model_b)?;

    let req_a = make_request(clean_prompt);
    let req_b = make_request(clean_prompt);

    // Dispatch in parallel
    let (res_a, res_b) = tokio::join!(provider_a.complete(req_a), provider_b.complete(req_b),);

    // Compute cost helper
    let cost_for = |model: &rift_bus::config::ModelConfig, tokens_in: u64, tokens_out: u64| {
        (tokens_in as f64 / 1_000_000.0) * model.capabilities.cost_per_1m_input
            + (tokens_out as f64 / 1_000_000.0) * model.capabilities.cost_per_1m_output
    };

    let result_a = match res_a {
        Ok(r) => {
            let cost = cost_for(&model_a, r.tokens_in, r.tokens_out);
            json!({
                "model_id": model_a.id,
                "model_short_id": model_a.short_id,
                "content": r.content,
                "tokens_in": r.tokens_in,
                "tokens_out": r.tokens_out,
                "latency_ms": r.latency_ms,
                "cost_usd": cost,
                "error": null,
            })
        }
        Err(e) => json!({
            "model_id": model_a.id,
            "model_short_id": model_a.short_id,
            "content": null,
            "tokens_in": 0,
            "tokens_out": 0,
            "latency_ms": 0,
            "cost_usd": 0.0,
            "error": e.to_string(),
        }),
    };

    let result_b = match res_b {
        Ok(r) => {
            let cost = cost_for(&model_b, r.tokens_in, r.tokens_out);
            json!({
                "model_id": model_b.id,
                "model_short_id": model_b.short_id,
                "content": r.content,
                "tokens_in": r.tokens_in,
                "tokens_out": r.tokens_out,
                "latency_ms": r.latency_ms,
                "cost_usd": cost,
                "error": null,
            })
        }
        Err(e) => json!({
            "model_id": model_b.id,
            "model_short_id": model_b.short_id,
            "content": null,
            "tokens_in": 0,
            "tokens_out": 0,
            "latency_ms": 0,
            "cost_usd": 0.0,
            "error": e.to_string(),
        }),
    };

    let cost_a = result_a["cost_usd"].as_f64().unwrap_or(0.0);
    let cost_b = result_b["cost_usd"].as_f64().unwrap_or(0.0);
    let total_cost = cost_a + cost_b;

    // Optional critique — model B reviews model A's output
    let content_a = result_a["content"].as_str().unwrap_or("").to_owned();
    let critique_text = if run_critique
        && result_a["error"].is_null()
        && result_b["error"].is_null()
        && !content_a.is_empty()
    {
        let critique_prompt = format!(
            "The user asked: \"{clean_prompt}\"\n\n\
             Another AI model responded:\n\n---\n{content_a}\n---\n\n\
             Critique this response. Identify strengths, weaknesses, factual errors, \
             and missed nuances. Be specific and constructive."
        );
        let critic: Box<dyn LlmProvider> = crate::llm_commands::create_provider(&model_b)?;
        let critique_req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: critique_prompt,
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: Some(
                "You are a critical reviewer. Analyze the given AI response for accuracy, \
                 completeness, and quality. Be concise but thorough."
                    .to_owned(),
            ),
            provider_options: None,
        };
        match critic.complete(critique_req).await {
            Ok(r) => Some(r.content),
            Err(e) => Some(format!("Critique failed: {e}")),
        }
    } else {
        None
    };

    // Publish ensemble complete
    if let Ok(env) = Envelope::new(Category::Llm, "llm.ensemble.complete").with_payload(&json!({
        "model_a": id_a,
        "model_b": id_b,
        "total_cost_usd": total_cost,
        "has_critique": critique_text.is_some(),
        "source": "mcp",
    })) {
        bus.publish(env);
    }

    Ok(json!({
        "results": [result_a, result_b],
        "task_type": format!("{task_type:?}"),
        "critique": critique_text,
        "total_cost_usd": total_cost,
    }))
}

/// `llm_process_start` — spawn a local llama-server for a configured model.
///
/// Requires `mcp.allow_mutations = true`. The model must be configured with
/// `HostingMode::Local { process_config }`. Uses the app-managed shared
/// `Arc<ProcessManager>` so started processes are visible to subsequent
/// `llm_process_stop` calls within the same Rift session.
///
/// Bus events (`llm.process.start`) are published by [`ProcessManager::start`]
/// internally — no double-publish here.
fn tool_llm_process_start(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    use rift_bus::config::HostingMode;
    use rift_bus::translators::llm_process::ProcessManager;
    use std::sync::Arc;

    let model_id = payload
        .get("model_id")
        .and_then(|v| v.as_str())
        .ok_or("llm_process_start: missing required argument: model_id")?;

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let model = config
        .ensemble
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("llm_process_start: model not found: {model_id}"))?;

    let mut process_config = match &model.hosting {
        HostingMode::Local { process_config } => process_config.clone(),
        HostingMode::Cloud => {
            return Err(format!(
                "llm_process_start: model '{model_id}' is a cloud model — only local models can be started"
            ))
        }
        HostingMode::Remote { .. } => {
            return Err(format!(
                "llm_process_start: model '{model_id}' is a remote model — only local models can be started"
            ))
        }
        // Forward-compat: a hosting mode written by a newer Rift build that this
        // build deserialized to `HostingMode::Unknown` — can't be started.
        _ => {
            return Err(format!(
                "llm_process_start: model '{model_id}' has an unrecognized hosting mode — only local models can be started"
            ))
        }
    };
    // Explicit start ⟹ keep it alive. A manually-started server should self-heal
    // on crash rather than stay dead until the next manual start (the failure mode
    // behind the observed gpt-oss-20b "instability"). The 3×/60s cap in
    // check_for_crashes still guards against OOM restart-loops.
    process_config.auto_restart = true;

    let manager: tauri::State<'_, Arc<ProcessManager>> = app_handle
        .try_state()
        .ok_or("llm_process_start: ProcessManager not available (internal error)")?;

    let pid = manager
        .start(model_id, &process_config)
        .map_err(|e| format!("llm_process_start: {e}"))?;

    Ok(json!({
        "model_id": model_id,
        "pid": pid,
        "port": process_config.port,
    }))
}

/// `llm_process_stop` — stop a running local llama-server process.
///
/// Requires `mcp.allow_mutations = true`. Uses the app-managed shared
/// `Arc<ProcessManager>` — must be the same instance that called `start`.
/// [`ProcessManager::stop`] publishes `llm.process.stop` internally.
fn tool_llm_process_stop(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    use rift_bus::translators::llm_process::ProcessManager;
    use std::sync::Arc;

    let model_id = payload
        .get("model_id")
        .and_then(|v| v.as_str())
        .ok_or("llm_process_stop: missing required argument: model_id")?;

    let manager: tauri::State<'_, Arc<ProcessManager>> = app_handle
        .try_state()
        .ok_or("llm_process_stop: ProcessManager not available (internal error)")?;

    manager
        .stop(model_id)
        .map_err(|e| format!("llm_process_stop: {e}"))?;

    Ok(json!({
        "model_id": model_id,
        "stopped": true,
    }))
}

/// `llm_model_apply_config` — apply a model's persisted config to its running
/// server, restarting it only if the launch args drifted (the stale-server
/// fix from Phase 0). Requires `mcp.allow_mutations = true`. Reads the on-disk
/// config (so the caller must save edits first), then reconciles the running
/// process. Returns `outcome`: "restarted" | "unchanged" | "not_running".
fn tool_llm_apply_config(app_handle: &AppHandle, payload: &Value) -> Result<Value, String> {
    use rift_bus::config::HostingMode;
    use rift_bus::translators::llm_process::{ApplyOutcome, ProcessManager};
    use std::sync::Arc;

    let model_id = payload
        .get("model_id")
        .and_then(|v| v.as_str())
        .ok_or("llm_model_apply_config: missing required argument: model_id")?;

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let model = config
        .ensemble
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("llm_model_apply_config: model not found: {model_id}"))?;
    let process_config = match &model.hosting {
        HostingMode::Local { process_config } => process_config.clone(),
        _ => {
            return Err(format!(
                "llm_model_apply_config: model '{model_id}' is not a local model"
            ))
        }
    };

    let manager: tauri::State<'_, Arc<ProcessManager>> = app_handle
        .try_state()
        .ok_or("llm_model_apply_config: ProcessManager not available (internal error)")?;

    let outcome = manager
        .apply_config(model_id, &process_config)
        .map_err(|e| format!("llm_model_apply_config: {e}"))?;

    let outcome_str = match outcome {
        ApplyOutcome::Restarted => "restarted",
        ApplyOutcome::Unchanged => "unchanged",
        ApplyOutcome::NotRunning => "not_running",
    };

    Ok(json!({
        "model_id": model_id,
        "outcome": outcome_str,
    }))
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

// ---------------------------------------------------------------------------
// Ensemble Router LLM tools
// ---------------------------------------------------------------------------

/// `llm_models` — list all configured models with status.
fn tool_llm_models() -> Result<Value, String> {
    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let models: Vec<Value> = config
        .ensemble
        .models
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "display_name": m.display_name,
                "provider": format!("{:?}", m.provider),
                "model_identifier": m.model_identifier,
                "hosting_mode": match &m.hosting {
                    rift_bus::config::HostingMode::Cloud => "cloud",
                    rift_bus::config::HostingMode::Local { .. } => "local",
                    rift_bus::config::HostingMode::Remote { .. } => "remote",
                    // Forward-compat: a mode this build doesn't recognize.
                    _ => "unknown",
                },
                "endpoint": m.endpoint,
                "short_id": m.short_id,
                "color": m.color,
                "capabilities": {
                    "max_context_tokens": m.capabilities.max_context_tokens,
                    "supports_streaming": m.capabilities.supports_streaming,
                    "supports_tool_use": m.capabilities.supports_tool_use,
                    "cost_per_1m_input": m.capabilities.cost_per_1m_input,
                    "cost_per_1m_output": m.capabilities.cost_per_1m_output,
                    "strength_tags": m.capabilities.strength_tags,
                },
            })
        })
        .collect();

    Ok(json!({
        "enabled": config.ensemble.enabled,
        "active_profile": format!("{:?}", config.ensemble.active_profile),
        "default_model": config.ensemble.default_model,
        "models": models,
    }))
}

/// `llm_switch` — set the active model (publishes a bus event).
fn tool_llm_switch(bus: &RiftBus, payload: &Value) -> Result<Value, String> {
    let model_id = payload
        .get("model_id")
        .and_then(|v| v.as_str())
        .ok_or("missing required argument: model_id")?;

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let model = config
        .ensemble
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("model not found: {model_id}"))?;

    if let Ok(env) = Envelope::new(Category::Llm, "llm.switch").with_payload(&json!({
        "model_id": model_id,
        "display_name": model.display_name,
        "short_id": model.short_id,
    })) {
        bus.publish(env);
    }

    Ok(json!({
        "switched_to": model_id,
        "display_name": model.display_name,
        "short_id": model.short_id,
    }))
}

/// `llm_health` — run health check on one or all models.
async fn tool_llm_health(payload: &Value) -> Result<Value, String> {
    use rift_bus::translators::llm::LlmProvider;
    use rift_bus::translators::llm_server::LlamaServerProvider;

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let target_id = payload.get("model_id").and_then(|v| v.as_str());

    let models: Vec<_> = config
        .ensemble
        .models
        .iter()
        .filter(|m| target_id.is_none() || target_id == Some(m.id.as_str()))
        .collect();

    let mut results = Vec::new();
    for m in &models {
        let provider: Box<dyn LlmProvider> = match &m.hosting {
            rift_bus::config::HostingMode::Local { process_config } => {
                let ep = format!("http://127.0.0.1:{}", process_config.port);
                Box::new(LlamaServerProvider::new(ep, &m.model_identifier))
            }
            rift_bus::config::HostingMode::Remote { .. } => {
                Box::new(LlamaServerProvider::new(&m.endpoint, &m.model_identifier))
            }
            rift_bus::config::HostingMode::Cloud => {
                results.push(json!({
                    "model_id": m.id,
                    "status": "skipped",
                    "reason": "cloud health check not implemented — use API dashboard",
                }));
                continue;
            }
            // Forward-compat: a hosting mode this build doesn't recognize
            // (deserialized to `HostingMode::Unknown`) — nothing to health-check.
            _ => {
                results.push(json!({
                    "model_id": m.id,
                    "status": "skipped",
                    "reason": "unrecognized hosting mode — written by a newer Rift build",
                }));
                continue;
            }
        };

        let status = provider.health_check().await;
        results.push(json!({
            "model_id": m.id,
            "display_name": m.display_name,
            "status": serde_json::to_value(&status).unwrap_or(json!("unknown")),
        }));
    }

    Ok(json!({ "results": results }))
}

/// Read-only tools surfaced to the local model in [`run_local_tool_call`].
///
/// Deliberately excludes ALL mutating tools (fs_write, git_action, pty_input,
/// bus_publish, rift_config_set, simulate_*), the heavy inspection tools, and
/// every `llm_*` tool. The last exclusion is the recursion guard — the local
/// agent must never be able to call itself or another model. The spike's safety
/// claim rests on this allowlist + the post-call name re-check below.
const LOCAL_TOOL_ALLOWLIST: &[&str] = &[
    "fs_read",
    "fs_tree",
    "todo_scan",
    "git_status",
    "bus_history",
];

/// Build the OpenAI `tools` array for the curated read-only subset.
fn local_tool_schemas() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "fs_read",
                "description": "Read a UTF-8 text file from the project, relative to the project root.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Project-relative file path, e.g. \"src/lib.rs\"." }
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "fs_tree",
                "description": "List the project file tree (honors ignore globs).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "max_depth": { "type": "integer", "description": "Max directory depth (default 6).", "minimum": 1, "maximum": 64 }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "todo_scan",
                "description": "Scan project source for TODO / FIXME / XXX markers.",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "Current git branch, staged/unstaged changes, and recent commits.",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "bus_history",
                "description": "Replay recent Rift event-bus envelopes (errors, hooks, commands, fs, mcp, etc.).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "category": { "type": "string", "description": "Optional category filter (pty, hook, agent, fs, index, aegis, status, system, mcp)." },
                        "limit": { "type": "integer", "description": "Max envelopes (default 100, max 1000).", "minimum": 1, "maximum": 1000 }
                    }
                }
            }
        }
    ])
}

/// `llm_tool_call` — single forced tool call by a LOCAL model (spike, Path C).
///
/// Offers the local model a curated READ-ONLY tool subset, runs ONE tool call
/// through [`dispatch_tool`], then inlines the result into a follow-up turn so
/// the model answers the original prompt. Bounded to one tool execution — this
/// function is the permanent home of the (currently length-1) agentic loop; the
/// multi-step loop, human-confirm gate, and mutating tools are later phases.
///
/// Safety:
/// - Only LOCAL/Remote (llama-server-backed) models with `supports_tool_use`
///   are accepted (basic gate; Step 4 hardens with auto-swap).
/// - The offered subset excludes every `llm_*` tool (recursion guard), and the
///   returned tool name is re-checked against [`LOCAL_TOOL_ALLOWLIST`] before
///   dispatch — a hallucinated mutating-tool name can never execute.
///
/// Runtime verification pends the Step 6 reliability probe against a live
/// tool-capable llama-server (`--jinja`). This step delivers a compiling,
/// clippy-clean executor; it is UNVERIFIED at runtime until then.
async fn run_local_tool_call(
    bus: &RiftBus,
    cfg: &McpConfig,
    project_root: &Path,
    pty_registry: &PtyRegistry,
    app_handle: &AppHandle,
    payload: &Value,
) -> Result<Value, String> {
    use rift_bus::config::HostingMode;
    use rift_bus::translators::llm::{CompletionRequest, LlmProvider, Message, Role};

    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or("llm_tool_call: missing required argument: prompt")?;
    let model_id_override = payload.get("model_id").and_then(|v| v.as_str());

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let mut router = rift_router::RouterService::new(config.ensemble.clone());
    if let Some(pm) =
        app_handle.try_state::<Arc<rift_bus::translators::llm_process::ProcessManager>>()
    {
        router.sync_local_availability(&pm.live_models());
    }

    let decision = router
        .route(prompt, model_id_override)
        .map_err(|e| format!("{e}"))?;
    let model = router
        .find_model(&decision.model_id)
        .map_err(|e| format!("{e}"))?
        .clone();

    // Capability gate (basic — Step 4 hardens with auto-swap). Tool calling only
    // works for llama-server-backed models with a tool-calling chat template.
    if matches!(model.hosting, HostingMode::Cloud) {
        return Err(format!(
            "llm_tool_call: model '{}' is a cloud model; the spike supports local/remote llama-server tool calling only.",
            model.id
        ));
    }
    if !model.capabilities.supports_tool_use {
        return Err(format!(
            "llm_tool_call: model '{}' does not advertise tool-use support. Switch to a tool-capable local model (e.g. qwen2.5-coder-14b).",
            model.id
        ));
    }

    let provider: Box<dyn LlmProvider> = crate::llm_commands::create_provider(&model)?;

    // Reasoning models (gpt-oss-20b, qwen3) emit tool calls into their thinking
    // channel unless it's disabled — the calls then never reach `tool_calls` and
    // the loop sees a plain text turn. Default thinking OFF for the tool-selection
    // turn; `enable_thinking: true` in the payload overrides (e.g. for probing).
    let enable_thinking = payload
        .get("enable_thinking")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let thinking_kwargs = json!({ "enable_thinking": enable_thinking });

    // --- Turn 1: offer the read-only tools; let the model pick one ----------
    let req1 = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: prompt.to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: Some(
            "You may call ONE read-only tool to gather information, then answer. \
             Prefer a tool when the question is about this project's files, TODOs, \
             git state, or recent activity. Otherwise answer directly."
                .to_string(),
        ),
        provider_options: Some(json!({
            "tools": local_tool_schemas(),
            "tool_choice": "auto",
            "chat_template_kwargs": thinking_kwargs.clone(),
        })),
    };

    let resp1 = provider.complete(req1).await.map_err(|e| format!("{e}"))?;

    // No tool call → the model answered directly. Return as-is.
    let call = match resp1.tool_calls.as_ref().and_then(|c| c.first()) {
        Some(c) => c.clone(),
        None => {
            return Ok(json!({
                "answer": resp1.content,
                "tool_used": Value::Null,
                "model_id": model.id,
                "turn1_stop_reason": format!("{:?}", resp1.stop_reason),
            }));
        }
    };

    // Recursion / safety guard: only the curated read-only allowlist may run,
    // even if the model hallucinated a tool name outside what we offered.
    if !LOCAL_TOOL_ALLOWLIST.contains(&call.name.as_str()) {
        return Err(format!(
            "llm_tool_call: model requested disallowed tool '{}' (allowed: {:?})",
            call.name, LOCAL_TOOL_ALLOWLIST
        ));
    }

    // Observability: announce the call BEFORE running it (mirrors mcp.invoke).
    let mut call_env = Envelope::new(Category::Llm, "llm.tool.call");
    call_env.payload = json!({
        "model_id": model.id,
        "tool": call.name,
        "arguments": call.arguments,
        "source": "mcp",
        "tier": "grunt",
    });
    bus.publish(call_env);

    // Execute the single tool call through the existing in-process dispatch.
    // `Box::pin` breaks the type-level async-recursion cycle (dispatch_tool ->
    // run_local_tool_call -> dispatch_tool); the LOCAL_TOOL_ALLOWLIST guard
    // above already prevents the cycle from ever occurring at runtime.
    let tool_result = Box::pin(dispatch_tool(
        bus,
        cfg,
        project_root,
        pty_registry,
        app_handle,
        &call.name,
        &call.arguments,
    ))
    .await;

    let (result_value, result_text) = match &tool_result {
        Ok(v) => (v.clone(), serde_json::to_string(v).unwrap_or_default()),
        Err(e) => (json!({ "error": e }), format!("(tool error: {e})")),
    };

    let mut result_env = Envelope::new(Category::Llm, "llm.tool.result");
    result_env.payload = json!({
        "model_id": model.id,
        "tool": call.name,
        "ok": tool_result.is_ok(),
        "source": "mcp",
        "tier": "grunt",
    });
    bus.publish(result_env);

    // --- Turn 2: inline the result as plain text, ask for the answer --------
    // The single-call spike inlines the tool result rather than using structured
    // Role::Tool messages; that refactor lands with the multi-step loop phase.
    let req2 = CompletionRequest {
        messages: vec![
            Message {
                role: Role::User,
                content: prompt.to_string(),
            },
            Message {
                role: Role::Assistant,
                content: format!("I'll use the {} tool to find out.", call.name),
            },
            Message {
                role: Role::User,
                content: format!(
                    "Result of `{}`:\n{}\n\nUsing this, answer concisely: {}",
                    call.name, result_text, prompt
                ),
            },
        ],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: None,
        // No tools on the summarizing turn; keep thinking disabled so reasoning
        // models don't spend their token budget thinking and return empty.
        provider_options: Some(json!({ "chat_template_kwargs": thinking_kwargs })),
    };

    let resp2 = provider.complete(req2).await.map_err(|e| format!("{e}"))?;

    Ok(json!({
        "answer": resp2.content,
        "tool_used": {
            "tool": call.name,
            "arguments": call.arguments,
            "result": result_value,
        },
        "model_id": model.id,
        "turn1_stop_reason": format!("{:?}", resp1.stop_reason),
    }))
}

/// `llm_prompt` — send a prompt through the router and return the response.
///
/// Phase 2: Uses RouterService for model selection. Supports @model tags,
/// auto-routing profiles, and escalation on retryable failures.
async fn tool_llm_prompt(
    bus: &RiftBus,
    app_handle: &AppHandle,
    payload: &Value,
) -> Result<Value, String> {
    use rift_bus::translators::llm::{CompletionRequest, LlmProvider, Message, Role};

    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or("missing required argument: prompt")?;

    let config = rift_bus::config::load_config().map_err(|e| format!("{e}"))?;
    let mut router = rift_router::RouterService::new(config.ensemble.clone());

    // Resident-aware routing: tell the router which LOCAL servers are actually
    // running so auto-routing (and fallback chains) never target a stopped
    // llama-server — the connection-fail cascade on a one-resident-at-a-time
    // (VRAM-bound) host. Best-effort: if the ProcessManager isn't available we
    // route as before. Explicit @tag/model_id overrides bypass this filter.
    if let Some(pm) =
        app_handle.try_state::<Arc<rift_bus::translators::llm_process::ProcessManager>>()
    {
        router.sync_local_availability(&pm.live_models());
    }

    let model_id_override = payload.get("model_id").and_then(|v| v.as_str());

    // Dispatch tier (grunt|partner|system) — defaults to "partner". Threaded
    // into the llm.* bus events so the LLM activity tab can distinguish grunt
    // (local, free, volume) work from partner-tier calls — the observability
    // half of tiered dispatch.
    let tier = payload
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or("partner")
        .to_string();

    let decision = router
        .route(prompt, model_id_override)
        .map_err(|e| format!("{e}"))?;

    // Phase 2: refine the ambiguous `Other` bucket with the tiny classifier
    // (no-op unless one is configured; only fires on auto, non-overridden routes).
    let decision = crate::llm_commands::maybe_refine_with_classifier(
        &router,
        &config.ensemble,
        prompt,
        decision,
    )
    .await;

    // Publish routing decision
    let mut route_env = Envelope::new(Category::Llm, "llm.route");
    route_env.payload = json!({
        "model_id": decision.model_id,
        "task_type": decision.task_type,
        "profile": decision.profile,
        "reason": decision.reason,
        "was_overridden": decision.was_overridden,
        "source": "mcp",
        "tier": tier,
    });
    bus.publish(route_env);

    let parsed = rift_router::parse_model_tag(prompt);
    let clean_prompt = &parsed.clean_prompt;

    let system_prompt = payload
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(String::from);

    let max_tokens = payload
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    // Optional thinking control for thinking-capable local models (gemma, gpt-oss).
    // `enable_thinking: false` disables the reasoning channel (direct answers — no
    // empty output from budget exhaustion). Threaded via provider_options ->
    // ChatRequest.chat_template_kwargs (llama.cpp template extension). A raw
    // `chat_template_kwargs` object, if supplied, takes precedence over the bool.
    let provider_options: Option<Value> = {
        let kwargs = payload.get("chat_template_kwargs").cloned().or_else(|| {
            payload
                .get("enable_thinking")
                .and_then(|v| v.as_bool())
                .map(|b| json!({ "enable_thinking": b }))
        });
        kwargs.map(|k| json!({ "chat_template_kwargs": k }))
    };

    let mut current_model_id = decision.model_id.clone();
    let mut fallback_chain = decision.fallback_chain.clone();
    let mut escalated = false;

    loop {
        let model = router
            .find_model(&current_model_id)
            .map_err(|e| format!("{e}"))?
            .clone();

        let provider: Box<dyn LlmProvider> = crate::llm_commands::create_provider(&model)?;

        let request = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: clean_prompt.clone(),
            }],
            max_tokens,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: system_prompt.clone(),
            provider_options: provider_options.clone(),
        };

        match provider.complete(request).await {
            Ok(resp) => {
                let cost_in =
                    (resp.tokens_in as f64 / 1_000_000.0) * model.capabilities.cost_per_1m_input;
                let cost_out =
                    (resp.tokens_out as f64 / 1_000_000.0) * model.capabilities.cost_per_1m_output;
                let cost_usd = cost_in + cost_out;

                let mut resp_env = Envelope::new(Category::Llm, "llm.response");
                resp_env.payload = json!({
                    "model_id": model.id,
                    "tokens_in": resp.tokens_in,
                    "tokens_out": resp.tokens_out,
                    "latency_ms": resp.latency_ms,
                    "cost_usd": cost_usd,
                    "escalated": escalated,
                    "source": "mcp",
                    "tier": tier,
                });
                bus.publish(resp_env);

                return Ok(json!({
                    "content": resp.content,
                    "tokens_in": resp.tokens_in,
                    "tokens_out": resp.tokens_out,
                    "model_used": resp.model_used,
                    "latency_ms": resp.latency_ms,
                    "stop_reason": format!("{:?}", resp.stop_reason),
                    "task_type": format!("{:?}", decision.task_type),
                    "routing_reason": decision.reason,
                    "cost_usd": cost_usd,
                    "escalated": escalated,
                }));
            }
            Err(err) => {
                let retryable = err.is_retryable();

                let mut err_env = Envelope::new(Category::Llm, "llm.error");
                err_env.payload = json!({
                    "model_id": current_model_id,
                    "error": err.to_string(),
                    "retryable": retryable,
                    "source": "mcp",
                    "tier": tier,
                });
                bus.publish(err_env);

                if !retryable || fallback_chain.is_empty() {
                    return Err(format!("{err}"));
                }

                router.mark_unavailable(&current_model_id);
                if let Some(next) =
                    router.escalate(&current_model_id, &fallback_chain, clean_prompt)
                {
                    current_model_id = next.model_id;
                    fallback_chain = next.fallback_chain;
                    escalated = true;
                } else {
                    return Err(format!(
                        "all models failed — last error from {}: {err}",
                        current_model_id
                    ));
                }
            }
        }
    }
}
