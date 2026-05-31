//! `rift llm` — one-shot LLM access through the running Rift host.
//!
//! Lets standalone scripts (report-to-html.py, the grunt MCP, …) use Rift's
//! local-model gateway when Rift is running, instead of POSTing raw to a
//! llama-server port. **§9-clean:** this talks to the host over the existing
//! MCP socket — the same `mcp.handshake` → `mcp.request.{tool}` →
//! `mcp.response.{tool}` protocol that rift-mcp's `HostBridge` uses — and the
//! host routes the work through `rift-router` → the llm translator. No HTTP is
//! made here. We implement a lean ONE-SHOT client (connect, handshake, one
//! request, read until the matching response) rather than reusing the full
//! async router, because a CLI invocation issues exactly one call and exits.
//!
//! Silent-fallback contract (mirrors `rift hook`): when Rift isn't running the
//! command exits non-zero with a one-line stderr note so callers fall back to
//! their own path (Ollama / direct port / deterministic). `RIFT_LLM_DEBUG=1`
//! surfaces the underlying error in full.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use rift_bus::{Category, Envelope, IpcClient};
use serde_json::{json, Value};
use tokio::time::timeout;

/// Handshake exchange ceiling.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-call ceiling — local models (cold load, long generations) can be slow.
const CALL_TIMEOUT: Duration = Duration::from_secs(180);

/// A best-effort-unique correlation id without pulling in the `uuid` crate:
/// process id + wall-clock nanos is unique enough for the two frames (handshake
/// + request) a single CLI invocation sends on a fresh connection.
fn request_id(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("rift-cli-{tag}-{}-{nanos}", std::process::id())
}

/// Resolve the socket SILENTLY — `None` when Rift isn't running (no `--socket`,
/// no `$RIFT_SOCKET_NAME`, no discovery file). Unlike `resolve_socket` this
/// never errors, so the caller controls the quiet-fallback message.
fn resolve_socket_quiet(arg: Option<&str>) -> Option<String> {
    if let Some(s) = arg {
        return Some(s.to_owned());
    }
    if let Ok(s) = std::env::var(crate::SOCKET_ENV_VAR) {
        return Some(s);
    }
    rift_bus::load_mcp_socket().ok().flatten()
}

/// One-shot host call: connect → handshake → `mcp.request.{tool}` → await the
/// matching `mcp.response.{tool}`. Returns the host's `result` value.
async fn host_call(socket_arg: Option<&str>, tool: &str, args: Value) -> Result<Value> {
    let socket = resolve_socket_quiet(socket_arg)
        .ok_or_else(|| anyhow!("Rift host not running (no socket)"))?;
    let token = rift_bus::load_mcp_token()
        .ok()
        .flatten()
        .unwrap_or_default();

    let mut client = IpcClient::connect(&socket)
        .await
        .with_context(|| format!("connect to {socket}"))?;

    // --- Handshake (token auth) ---
    let hid = request_id("hs");
    let hs = Envelope::new(Category::Mcp, "mcp.handshake")
        .with_payload(&json!({ "request_id": hid, "token": token }))
        .map_err(|e| anyhow!("handshake build: {e}"))?;
    client.send(&hs).await.context("send handshake")?;
    loop {
        let env = timeout(HANDSHAKE_TIMEOUT, client.recv())
            .await
            .map_err(|_| anyhow!("handshake timeout"))??;
        if env.category != Category::Mcp {
            continue;
        }
        if env.payload.get("request_id").and_then(|v| v.as_str()) != Some(hid.as_str()) {
            continue;
        }
        match env.kind.as_str() {
            "mcp.handshake.ack" => break,
            "mcp.handshake.deny" => {
                let reason = env
                    .payload
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unspecified");
                return Err(anyhow!("handshake denied: {reason}"));
            }
            _ => continue,
        }
    }

    // --- Request (payload = { request_id, ...args }) ---
    let rid = request_id("rq");
    let mut payload = json!({ "request_id": rid });
    if let Some(obj) = args.as_object() {
        if let Some(map) = payload.as_object_mut() {
            for (k, v) in obj {
                if k != "request_id" {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
    }
    let req = Envelope::new(Category::Mcp, format!("mcp.request.{tool}"))
        .with_payload(&payload)
        .map_err(|e| anyhow!("request build: {e}"))?;
    client.send(&req).await.context("send request")?;

    let resp_kind = format!("mcp.response.{tool}");
    loop {
        let env = timeout(CALL_TIMEOUT, client.recv())
            .await
            .map_err(|_| anyhow!("timed out waiting for {resp_kind}"))??;
        if env.category != Category::Mcp || env.kind != resp_kind {
            continue;
        }
        if env.payload.get("request_id").and_then(|v| v.as_str()) != Some(rid.as_str()) {
            continue;
        }
        let ok = env
            .payload
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if ok {
            return env
                .payload
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow!("host returned ok with no 'result'"));
        }
        let msg = env
            .payload
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown host error");
        return Err(anyhow!("tool '{tool}' failed: {msg}"));
    }
}

/// `rift llm` subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum LlmCmd {
    /// Send a prompt to a local model through Rift's router. Prints the
    /// response content (or the full JSON result with `--json`).
    Prompt {
        /// Prompt text. If omitted, read from stdin.
        text: Option<String>,
        /// Force a specific model id (else the router/profile chooses).
        #[arg(long)]
        model: Option<String>,
        /// Optional system prompt.
        #[arg(long)]
        system: Option<String>,
        /// Dispatch tier hint (grunt|partner) for routing + observability.
        #[arg(long)]
        tier: Option<String>,
        /// Max tokens to generate.
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Print the full JSON result instead of just the content.
        #[arg(long)]
        json: bool,
    },
    /// List configured models (JSON).
    List,
    /// Set the active routing model.
    Switch {
        /// Model id to activate.
        model_id: String,
    },
    /// Health-check a model endpoint (JSON). Omit --model to check the active one.
    Health {
        #[arg(long)]
        model: Option<String>,
    },
}

/// Dispatch a `rift llm` subcommand. On failure prints a one-line stderr
/// sentinel (full error under `RIFT_LLM_DEBUG=1`) and returns a terse `Err` so
/// the binary exits non-zero and callers fall back.
pub async fn run(socket_arg: Option<&str>, cmd: LlmCmd) -> Result<()> {
    let result = dispatch(socket_arg, cmd).await;
    if let Err(err) = &result {
        if std::env::var_os("RIFT_LLM_DEBUG").is_some() {
            eprintln!("rift llm: {err:#}");
        } else {
            eprintln!("rift llm: unavailable (falling back)");
        }
        // Terse error → concise non-zero exit, no anyhow chain spam.
        return Err(anyhow!("rift llm unavailable"));
    }
    result
}

async fn dispatch(socket_arg: Option<&str>, cmd: LlmCmd) -> Result<()> {
    match cmd {
        LlmCmd::Prompt {
            text,
            model,
            system,
            tier,
            max_tokens,
            json,
        } => {
            let prompt = match text {
                Some(t) => t,
                None => read_stdin_prompt()?,
            };
            let mut args = json!({ "prompt": prompt });
            let map = args.as_object_mut().unwrap();
            if let Some(m) = model {
                map.insert("model_id".into(), json!(m));
            }
            if let Some(s) = system {
                map.insert("system_prompt".into(), json!(s));
            }
            if let Some(t) = tier {
                map.insert("tier".into(), json!(t));
            }
            if let Some(mt) = max_tokens {
                map.insert("max_tokens".into(), json!(mt));
            }
            let result = host_call(socket_arg, "llm_prompt", args).await?;
            if json {
                println!("{result}");
            } else {
                let content = result
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("response had no 'content' field"))?;
                println!("{content}");
            }
            Ok(())
        }
        LlmCmd::List => {
            let result = host_call(socket_arg, "llm_models", json!({})).await?;
            println!("{result}");
            Ok(())
        }
        LlmCmd::Switch { model_id } => {
            let result =
                host_call(socket_arg, "llm_switch", json!({ "model_id": model_id })).await?;
            println!("{result}");
            Ok(())
        }
        LlmCmd::Health { model } => {
            let mut args = json!({});
            if let Some(m) = model {
                args.as_object_mut()
                    .unwrap()
                    .insert("model_id".into(), json!(m));
            }
            let result = host_call(socket_arg, "llm_health", args).await?;
            println!("{result}");
            Ok(())
        }
    }
}

/// Read a prompt from stdin (for `rift llm prompt` with no positional text —
/// lets callers pipe a large prompt without argv length limits).
fn read_stdin_prompt() -> Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("read prompt from stdin")?;
    if buf.trim().is_empty() {
        return Err(anyhow!(
            "no prompt: pass text as an argument or pipe via stdin"
        ));
    }
    Ok(buf)
}
