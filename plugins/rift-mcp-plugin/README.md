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

1. **A built `rift-mcp` binary on PATH.** Build from this repo:
   ```sh
   cargo build --release -p rift-mcp
   ```
   Then put `target/release/rift-mcp` on your shell PATH.

2. **A running Rift host with MCP enabled.** Open Rift → Settings popout
   → toggle "enable MCP server". A token is generated and persisted at:
   - Windows: `%APPDATA%\com.abyssal.rift\config\mcp_token`
   - macOS:   `~/Library/Application Support/com.abyssal.rift/mcp_token`
   - Linux:   `$XDG_CONFIG_HOME/rift/mcp_token`

3. **Token in the environment.** Either:
   - Click "reveal token" + "copy" in Settings, then
     `export RIFT_MCP_TOKEN=<paste>` before launching Claude Code, or
   - Let `rift-mcp` discover the token file automatically (default — no env
     var needed when running on the same machine as the host).

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

- **`/mcp` shows the server but no tools** — the host bridge in
  `src-tauri/src/mcp_host.rs` only subscribes when `RiftConfig.mcp.enabled
  = true`. Re-check the Settings toggle.
- **"No MCP token available"** — the binary couldn't find a token. Pass
  `--token` explicitly, set `RIFT_MCP_TOKEN`, or enable MCP in Rift's
  Settings to generate one.
- **Token mismatch / handshake denied** — regenerate the token in Settings,
  update `RIFT_MCP_TOKEN`, and restart Claude Code.

## Audit trail

Every MCP tool invocation publishes a `Category::Mcp / kind="mcp.invoke"`
envelope on the Rift bus BEFORE running. To watch it live:

- Open Rift's "bus tail" notif tab and filter to `Category::Mcp`, or
- From CLI: `rift tail --category mcp`

Audit-only by design — there are no per-call confirmation popouts in v1.x
(D-014 §11 q4).
