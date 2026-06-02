//! `rift session` — session-management commands through the running Rift host.
//!
//! Mirrors the `llm` module: each subcommand is one `mcp.request.{tool}`
//! round-trip over the IPC socket (reusing [`crate::llm::host_call`]). The
//! host executes it in `src-tauri/src/mcp_host.rs`.

use anyhow::Result;
use serde_json::json;

/// `rift session` subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum SessionCmd {
    /// Force a one-shot compaction of the active session log: summarize its
    /// older prefix into a sidecar `<id>.summary.json` (the append-only
    /// `.jsonl` audit log is left untouched). Works even when idle
    /// auto-compaction is off. Requires `mcp.allow_mutations = true`.
    Compact {
        /// Print the full JSON result instead of a one-line summary.
        #[arg(long)]
        json: bool,
    },
}

/// Dispatch a `rift session` subcommand.
pub async fn run(socket_arg: Option<&str>, cmd: SessionCmd) -> Result<()> {
    match cmd {
        SessionCmd::Compact { json: as_json } => {
            let result = crate::llm::host_call(socket_arg, "session_compact", json!({})).await?;
            if as_json {
                println!("{result}");
            } else {
                let id = result
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let chars = result
                    .get("summary_chars")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                println!("compacted session {id} — {chars} summary chars");
                if let Some(s) = result.get("summary").and_then(|v| v.as_str()) {
                    println!("\n{s}");
                }
            }
            Ok(())
        }
    }
}
