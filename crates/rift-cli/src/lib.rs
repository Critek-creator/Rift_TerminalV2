//! rift-cli — public-facing CLI surface for the Rift Integration Protocol.
//!
//! The binary in `src/main.rs` is a thin shim over [`run`]. The lib API
//! (publishing, status) is exposed so integration tests can drive the
//! CLI without spawning a subprocess.

use std::io::{self, IsTerminal, Read};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use rift_bus::{Category, Envelope, IpcClient};

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
    /// Publish a hook event to the running Rift instance.
    ///
    /// Reads stdin (when piped) as the payload — JSON if it parses,
    /// otherwise wrapped as `{ "stdin": "<text>" }`. `--payload` overrides
    /// stdin. `--no-stdin` skips stdin even when one is connected.
    Hook {
        /// Event kind (e.g. "PreToolUse", "PostToolUse", "UserPromptSubmit").
        kind: String,

        #[arg(long, value_name = "JSON")]
        payload: Option<String>,

        #[arg(long)]
        no_stdin: bool,
    },

    /// Connect to the running instance and print the resolved socket
    /// name. Useful as a smoke test from inside a Rift shell.
    Status,
}

/// Resolve the socket name from `--socket` arg or `$RIFT_SOCKET_NAME`.
pub fn resolve_socket(arg: Option<&str>) -> Result<String> {
    if let Some(s) = arg {
        return Ok(s.to_owned());
    }
    std::env::var(SOCKET_ENV_VAR).map_err(|_| {
        anyhow!(
            "no socket name. Pass --socket <name> or set ${SOCKET_ENV_VAR}.\n\
             The running Rift instance prints the socket name on boot \
             (typical value: rift-v2-<pid>.sock); shells launched from \
             inside Rift inherit ${SOCKET_ENV_VAR} automatically."
        )
    })
}

/// Open a connection to a running Rift instance and publish a single
/// hook envelope. Lifted to a free function so integration tests can
/// drive it directly without spawning a subprocess.
pub async fn publish_hook(socket: &str, kind: &str, payload: serde_json::Value) -> Result<()> {
    let mut client = IpcClient::connect(socket)
        .await
        .with_context(|| format!("connect to {socket}"))?;
    let mut env = Envelope::new(Category::Hook, kind);
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
    let socket = resolve_socket(cli.socket.as_deref())?;
    match cli.cmd {
        Cmd::Hook {
            kind,
            payload,
            no_stdin,
        } => {
            let payload = read_payload(payload.as_deref(), no_stdin)?;
            publish_hook(&socket, &kind, payload).await
        }
        Cmd::Status => {
            // Connecting establishes the round-trip; on success we know
            // the IPC server is up and reachable.
            let _client = IpcClient::connect(&socket)
                .await
                .with_context(|| format!("connect to {socket}"))?;
            println!("rift: connected to {socket}");
            Ok(())
        }
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
        // SAFETY: tests run in a single process; there is no other test
        // mutating this var. Restore after to avoid leaking into other
        // tests.
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
        let prev = std::env::var(SOCKET_ENV_VAR).ok();
        std::env::remove_var(SOCKET_ENV_VAR);
        let err = resolve_socket(None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("--socket"), "got: {msg}");
        assert!(msg.contains(SOCKET_ENV_VAR), "got: {msg}");
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
                payload,
                no_stdin,
            } => {
                assert_eq!(kind, "PreToolUse");
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
}
