# rift-mcp-plugin

Claude Code plugin that wires Claude into a running Rift terminal via the
[D-014](../../decisions/D-014_rift_mcp_v1_plan.md) MCP server.

## What it does

Registers the `rift-mcp` binary as a stdio MCP server in Claude Code. Once
connected, Claude can call Rift's tool surface to introspect a live session.

### Phase A tool surface (4 read-only tools)

| Tool | Effect |
|------|--------|
| `bus_history`  | Replay recent envelopes from the Rift bus (paginated). |
| `bus_tail`     | Stream live envelopes as MCP notifications. |
| `git_status`   | Same payload as the Git notif tab. |
| `aegis_state`  | Last `aegis.session.skill_loaded` snapshot. |

Phase B–F add filesystem, PTY, cockpit, DOM/screenshot/`js_eval`,
mutation, and WebSocket-transport tools. See `decisions/D-014_rift_mcp_v1_plan.md`.

## Prerequisites

1. **A built `rift-mcp` binary on PATH.** From this repo:
   ```sh
   cargo install --path crates/rift-mcp --force
   ```
   This drops `rift-mcp` into `~/.cargo/bin`, which should already be on PATH
   for any Rust toolchain install. Alternative: `cargo build --release -p rift-mcp`
   then copy `target/release/rift-mcp` to a directory on PATH manually.

2. **A running Rift host with MCP enabled.** Open Rift → Settings popout
   → toggle "enable MCP server" → restart Rift. On boot the host writes two
   sibling files to the platform config directory:
   - `mcp_token`   — auth token, owner-readable, never logged.
   - `mcp_socket`  — current IPC socket name (changes per launch). Auto-cleared on Rift exit.

   Paths:
   - Windows: `%APPDATA%\com.abyssal.rift\config\`
   - macOS:   `~/Library/Application Support/com.abyssal.rift/`
   - Linux:   `$XDG_CONFIG_HOME/rift/`

   No env vars needed — the binary reads both files at startup. Claude Code can
   spawn `rift-mcp` with no args, no env, no plumbing.

## Installation

Tell Claude Code about the plugin directory:

```sh
claude plugin install ./plugins/rift-mcp-plugin
```

Verify the server is registered:

```sh
claude /mcp
```

You should see `rift` in the connected-servers list once Rift is running.

## Tool naming

Claude Code namespaces MCP tools as
`mcp__plugin_rift-mcp_rift__<tool_name>`. To pre-allow specific Rift tools
in a custom command:

```yaml
---
allowed-tools:
  - "mcp__plugin_rift-mcp_rift__bus_history"
  - "mcp__plugin_rift-mcp_rift__git_status"
  - "mcp__plugin_rift-mcp_rift__aegis_state"
---
```

Avoid wildcards (`mcp__plugin_rift-mcp_rift__*`) for security — the catalog
grows in later phases to include mutating tools.

## Troubleshooting

- **`× failed` in `/mcp`** — most common cause is no Rift host running, or
  MCP not enabled in Settings. Run `rift-mcp` directly to see the actual
  error on stderr; it will name the exact discovery file path it looked for.
- **`/mcp` shows the server but no tools** — the host bridge in
  `src-tauri/src/mcp_host.rs` only subscribes when `RiftConfig.mcp.enabled
  = true`. Re-check the Settings toggle, then **restart Rift** (the host
  spawn happens at app boot, so toggling at runtime won't take effect until
  next launch).
- **"No MCP token available"** — the binary couldn't find a token. Enable
  MCP in Rift's Settings to generate one (or pass `--token` / set
  `RIFT_MCP_TOKEN` for ad-hoc testing).
- **"no Rift host found"** — Rift is not running, or the discovery file
  (`mcp_socket`) was never written. Start Rift, confirm MCP is enabled in
  Settings, and confirm the file exists in the platform config dir.
- **Token mismatch / handshake denied** — regenerate the token in Settings
  and restart Claude Code so the plugin re-reads it from disk.

## Audit trail

Every MCP tool invocation publishes a `Category::Mcp / kind="mcp.invoke"`
envelope on the Rift bus BEFORE running. To watch it live:

- Open Rift's "bus tail" notif tab and filter to `Category::Mcp`, or
- From CLI: `rift tail --category mcp`

Audit-only by design — there are no per-call confirmation popouts in v1.x
(D-014 §11 q4).
