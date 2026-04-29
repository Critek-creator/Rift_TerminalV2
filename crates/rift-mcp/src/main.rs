//! `rift-mcp` — Model Context Protocol server for a running Rift host.
//!
//! Speaks JSON-RPC 2.0 over stdio (newline-delimited frames per the MCP
//! spec). Translates each MCP request into a `Category::Mcp / mcp.request.*`
//! envelope, ships it across the IPC socket to the Rift host, and writes the
//! `mcp.response.*` reply back to stdout.
//!
//! Spec: `decisions/D-014_rift_mcp_v1_plan.md` (locked v1.1 — 2026-04-29).
//!
//! Phase A tool surface (4 read-only Tier 1 tools):
//!   * `bus_history`  — paginated replay of recent envelopes
//!   * `bus_tail`     — live envelope stream (long-running notifications)
//!   * `git_status`   — Git tab payload
//!   * `aegis_state`  — last `aegis.session.skill_loaded` snapshot

use anyhow::{Context, Result};
use clap::Parser;

use rift_mcp::{run_stdio, McpServerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "rift-mcp",
    about = "Rift MCP server (stdio JSON-RPC). Connects to a running Rift host via the bus IPC socket.",
    version
)]
struct Args {
    /// Override the IPC socket name. Defaults to the platform-standard
    /// Rift bus socket.
    #[arg(long)]
    socket: Option<String>,

    /// Auth token. Falls back to the `RIFT_MCP_TOKEN` env var, then to the
    /// on-disk token file (`mcp_token` next to `config.toml`). The host
    /// closes the connection on token mismatch.
    #[arg(long)]
    token: Option<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber_init();

    let args = Args::parse();

    let token = args
        .token
        .or_else(|| std::env::var("RIFT_MCP_TOKEN").ok())
        .or_else(|| rift_bus::load_mcp_token().ok().flatten())
        .context(
            "No MCP token available. Pass --token, set RIFT_MCP_TOKEN, or enable MCP in Rift's \
             Settings popout to generate one.",
        )?;

    let cfg = McpServerConfig {
        socket_name: args.socket,
        token,
    };

    // Surface the failure on stderr before bubbling — Claude Code displays
    // stderr in the MCP server status pane, so a mid-flight handshake
    // failure or missing-host condition becomes a useful message instead
    // of a silent "× failed".
    if let Err(e) = run_stdio(cfg).await {
        eprintln!("rift-mcp: {e:#}");
        return Err(e);
    }
    Ok(())
}

fn tracing_subscriber_init() {
    // No-op for v1.0 — `tracing` events without a subscriber become no-ops.
    // stdout is reserved for JSON-RPC; stderr is free for future logging.
}
