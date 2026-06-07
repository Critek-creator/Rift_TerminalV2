//! rift-cli — public-facing CLI surface for the Rift Integration Protocol.
//!
//! The binary in `src/main.rs` is a thin shim over [`run`]. The lib API
//! (publishing, status) is exposed so integration tests can drive the
//! CLI without spawning a subprocess.

use std::io::{self, IsTerminal, Read};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use rift_bus::{Category, Envelope, IpcClient};

mod chat;
mod llm;
mod session;

/// Environment variable consulted when `--socket` is omitted. Set by
/// the running Rift instance when it spawns child shells, so commands
/// like `rift hook ...` from inside a Rift terminal "just work."
pub const SOCKET_ENV_VAR: &str = "RIFT_SOCKET_NAME";

#[derive(Parser, Debug)]
#[command(
    name = "rift",
    version,
    about = "Rift Terminal CLI — speak the Rift Integration Protocol"
)]
pub struct Cli {
    /// Override the IPC socket name. Default reads `$RIFT_SOCKET_NAME`.
    #[arg(long, global = true)]
    pub socket: Option<String>,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Publish an event to the running Rift instance.
    ///
    /// Reads stdin (when piped) as the payload — JSON if it parses,
    /// otherwise wrapped as `{ "stdin": "<text>" }`. `--payload` overrides
    /// stdin. `--no-stdin` skips stdin even when one is connected.
    Hook {
        /// Event kind (e.g. "PreToolUse", "agent.start", "agent.end").
        kind: String,

        /// Bus category (default: "hook"). Use "agent" to publish agent
        /// lifecycle events that the Agents tab receives.
        #[arg(long, default_value = "hook")]
        category: String,

        #[arg(long, value_name = "JSON")]
        payload: Option<String>,

        #[arg(long)]
        no_stdin: bool,
    },

    /// Connect to the running instance and print the resolved socket
    /// name. Useful as a smoke test from inside a Rift shell.
    Status,

    /// Use a local model through the running Rift host — prompt, list, switch,
    /// or health-check. Routed via rift-router (the §9-clean gateway). Exits
    /// non-zero when Rift isn't running so callers fall back. See `llm` module.
    Llm {
        #[command(subcommand)]
        sub: llm::LlmCmd,
    },

    /// Session management through the running Rift host — e.g. compact the
    /// active session log on demand.
    Session {
        #[command(subcommand)]
        sub: session::SessionCmd,
    },

    /// Interactive multi-turn chat with a local model, routed through Rift.
    ///
    /// A local lifeline: when the cloud assistant is rate-limited or offline,
    /// `rift chat` still answers (degraded but functional). Exits cleanly if
    /// Rift isn't running. See the `chat` module.
    Chat {
        /// Force a specific model id (else the router/profile chooses).
        #[arg(long)]
        model: Option<String>,
        /// System prompt / behavioral rules for the whole session (inline).
        #[arg(long)]
        system: Option<String>,
        /// Read the session rules from a file. Overrides the auto-discovered
        /// `.rift/rules.md` (project) and `~/.config/rift/rules.md` (global).
        #[arg(long)]
        system_file: Option<std::path::PathBuf>,
        /// Ground the session in the Abyssal Index: FTS5-select entries
        /// matching this topic and inject them as context at startup. Needs an
        /// `index`-enabled Rift build; degrades to no grounding otherwise.
        #[arg(long)]
        ground: Option<String>,
        /// Max tokens to generate per turn.
        #[arg(long)]
        max_tokens: Option<u32>,
    },
}

/// Resolve the socket name from `--socket` arg, `$RIFT_SOCKET_NAME`, or
/// the on-disk discovery file written by the running Rift host.
pub fn resolve_socket(arg: Option<&str>) -> Result<String> {
    if let Some(s) = arg {
        return Ok(s.to_owned());
    }
    if let Ok(s) = std::env::var(SOCKET_ENV_VAR) {
        return Ok(s);
    }
    if let Ok(Some(s)) = rift_bus::load_mcp_socket() {
        return Ok(s);
    }
    Err(anyhow!(
        "no socket name. Pass --socket <name> or set ${SOCKET_ENV_VAR}.\n\
         The running Rift instance writes its socket name to the discovery \
         file at {}; if Rift is running, this should resolve automatically.",
        rift_bus::mcp_socket_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<config dir unavailable>".to_owned())
    ))
}

/// Open a connection to a running Rift instance and publish a single
/// envelope. Lifted to a free function so integration tests can drive
/// it directly without spawning a subprocess.
pub async fn publish_hook(socket: &str, kind: &str, payload: serde_json::Value) -> Result<()> {
    publish_event(socket, Category::Hook, kind, payload).await
}

/// Publish an envelope with an explicit category.
pub async fn publish_event(
    socket: &str,
    category: Category,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    let mut client = IpcClient::connect(socket)
        .await
        .with_context(|| format!("connect to {socket}"))?;
    let mut env = Envelope::new(category, kind);
    env.payload = payload;
    client.send(&env).await.context("publish envelope")?;
    Ok(())
}

/// Top-level entry point used by the binary. Parses argv and dispatches.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    execute(cli).await
}

/// Run a pre-parsed CLI invocation. Useful for tests that construct a
/// `Cli` programmatically.
pub async fn execute(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Hook {
            kind,
            category,
            payload,
            no_stdin,
        } => {
            let payload = read_payload(payload.as_deref(), no_stdin)?;
            let cat = parse_category(&category);
            // Hooks are best-effort telemetry. When Rift isn't running, the
            // socket can't be resolved (discovery file is removed on exit /
            // pruned when stale) or the connect fails — and Claude Code fires
            // ~16 hook event types on every tool call and prompt. Propagating
            // the error here floods the terminal with connection failures the
            // moment Rift is closed. Swallow it and exit cleanly; set
            // RIFT_HOOK_DEBUG=1 to surface the underlying error for diagnostics.
            if let Err(err) = publish_resolved(cli.socket.as_deref(), cat, &kind, payload).await {
                if std::env::var_os("RIFT_HOOK_DEBUG").is_some() {
                    eprintln!("rift hook: suppressed (Rift not reachable): {err:#}");
                }
            }
            Ok(())
        }
        Cmd::Status => {
            // Status is the diagnostic smoke test — it is *supposed* to report
            // connection failures loudly, so it resolves and connects directly.
            let socket = resolve_socket(cli.socket.as_deref())?;
            let _client = IpcClient::connect(&socket)
                .await
                .with_context(|| format!("connect to {socket}"))?;
            println!("rift: connected to {socket}");
            Ok(())
        }
        Cmd::Llm { sub } => llm::run(cli.socket.as_deref(), sub).await,
        Cmd::Session { sub } => session::run(cli.socket.as_deref(), sub).await,
        Cmd::Chat {
            model,
            system,
            system_file,
            ground,
            max_tokens,
        } => {
            chat::run_chat(
                cli.socket.as_deref(),
                chat::ChatOptions {
                    model,
                    system,
                    system_file,
                    ground,
                    max_tokens,
                },
            )
            .await
        }
    }
}

/// Resolve the socket then publish a single event. Factored out so the `Hook`
/// arm can treat the whole resolve→connect→send chain as one best-effort
/// operation that fails silently when Rift isn't running.
async fn publish_resolved(
    socket_arg: Option<&str>,
    category: Category,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    let socket = resolve_socket(socket_arg)?;
    publish_event(&socket, category, kind, payload).await
}

fn parse_category(s: &str) -> Category {
    match s {
        "pty" => Category::Pty,
        "hook" => Category::Hook,
        "agent" => Category::Agent,
        "fs" => Category::Fs,
        "index" => Category::Index,
        "aegis" => Category::Aegis,
        "status" => Category::Status,
        "system" => Category::System,
        "mcp" => Category::Mcp,
        "sentinel" => Category::Sentinel,
        "llm" => Category::Llm,
        _ => Category::Hook,
    }
}

/// Resolve the payload for a `Hook` invocation:
/// `--payload` (inline JSON) > stdin (parsed if JSON, else wrapped) > Null.
fn read_payload(inline: Option<&str>, no_stdin: bool) -> Result<serde_json::Value> {
    if let Some(json) = inline {
        return serde_json::from_str(json)
            .with_context(|| format!("--payload is not valid JSON: {json}"));
    }
    if no_stdin || io::stdin().is_terminal() {
        return Ok(serde_json::Value::Null);
    }
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).context("read stdin")?;
    if buf.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_str::<serde_json::Value>(&buf)
        .unwrap_or_else(|_| serde_json::json!({ "stdin": buf })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that mutate the SOCKET_ENV_VAR process-global env var.
    /// `cargo test` runs tests in parallel by default; without this lock the
    /// env-fallback and missing-error tests race on the same global state.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn read_payload_inline_json() {
        let v = read_payload(Some(r#"{"a":1}"#), false).unwrap();
        assert_eq!(v, serde_json::json!({ "a": 1 }));
    }

    #[test]
    fn read_payload_inline_invalid_errors() {
        let err = read_payload(Some("{not json"), false).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not valid JSON"), "got: {msg}");
    }

    #[test]
    fn read_payload_no_stdin_returns_null() {
        let v = read_payload(None, true).unwrap();
        assert_eq!(v, serde_json::Value::Null);
    }

    #[test]
    fn resolve_socket_arg_wins() {
        let s = resolve_socket(Some("override.sock")).unwrap();
        assert_eq!(s, "override.sock");
    }

    #[test]
    fn resolve_socket_env_fallback() {
        // ENV_LOCK serializes against resolve_socket_missing_helpful_error,
        // which manipulates the same SOCKET_ENV_VAR process-global env var.
        // Without this lock the two tests race under cargo's default parallel
        // execution (one removes mid-set of the other).
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var(SOCKET_ENV_VAR).ok();
        std::env::set_var(SOCKET_ENV_VAR, "from-env.sock");
        let s = resolve_socket(None).unwrap();
        assert_eq!(s, "from-env.sock");
        match prev {
            Some(p) => std::env::set_var(SOCKET_ENV_VAR, p),
            None => std::env::remove_var(SOCKET_ENV_VAR),
        }
    }

    #[test]
    fn resolve_socket_missing_helpful_error() {
        // See ENV_LOCK rationale on resolve_socket_env_fallback above.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var(SOCKET_ENV_VAR).ok();
        std::env::remove_var(SOCKET_ENV_VAR);
        // If the on-disk discovery file exists (Rift is running on this
        // machine), resolve_socket succeeds via that fallback — skip the
        // error assertion. On CI (no Rift), all three sources fail.
        match resolve_socket(None) {
            Err(err) => {
                let msg = format!("{err:#}");
                assert!(msg.contains("--socket"), "got: {msg}");
                assert!(msg.contains(SOCKET_ENV_VAR), "got: {msg}");
            }
            Ok(s) => {
                assert!(!s.is_empty(), "discovery file returned empty socket name");
            }
        }
        if let Some(p) = prev {
            std::env::set_var(SOCKET_ENV_VAR, p);
        }
    }

    /// Parser smoke test — the binary's surface area is small enough that
    /// pinning the option/positional shape catches accidental rename
    /// regressions.
    #[test]
    fn parse_hook_with_payload() {
        let cli = Cli::try_parse_from([
            "rift",
            "--socket",
            "x.sock",
            "hook",
            "PreToolUse",
            "--payload",
            r#"{"k":1}"#,
        ])
        .unwrap();
        assert_eq!(cli.socket.as_deref(), Some("x.sock"));
        match cli.cmd {
            Cmd::Hook {
                kind,
                category,
                payload,
                no_stdin,
            } => {
                assert_eq!(kind, "PreToolUse");
                assert_eq!(category, "hook");
                assert_eq!(payload.as_deref(), Some(r#"{"k":1}"#));
                assert!(!no_stdin);
            }
            _ => panic!("expected Hook"),
        }
    }

    #[test]
    fn parse_status() {
        let cli = Cli::try_parse_from(["rift", "status"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Status));
    }

    /// A `hook` invocation that cannot reach Rift must exit cleanly (Ok),
    /// not propagate a connection error. This is the anti-spam contract:
    /// Claude Code fires ~16 hook events per tool call, and erroring on each
    /// when Rift is closed floods the terminal. `--no-stdin` keeps the test
    /// from blocking on stdin; the bogus socket name guarantees connect fails
    /// fast (missing named pipe → immediate error, no retry).
    #[tokio::test]
    async fn hook_silent_when_rift_unreachable() {
        let cli = Cli::try_parse_from([
            "rift",
            "--socket",
            "rift-cli-test-nonexistent-socket",
            "hook",
            "PreToolUse",
            "--no-stdin",
        ])
        .unwrap();
        // Must be Ok despite no Rift listening on that socket.
        execute(cli).await.expect("hook must be a silent no-op");
    }

    /// `status`, by contrast, MUST surface the connection failure — it is the
    /// diagnostic smoke test. Bogus socket → connect fails → Err.
    #[tokio::test]
    async fn status_errors_when_rift_unreachable() {
        let cli = Cli::try_parse_from([
            "rift",
            "--socket",
            "rift-cli-test-nonexistent-socket",
            "status",
        ])
        .unwrap();
        assert!(
            execute(cli).await.is_err(),
            "status must report connection failure"
        );
    }
}
