//! Tool catalog — Phase A (4) + Phase B (6) + Phase C (3) + Phase D (7) + diagnostic (2) = 22 tools.
//!
//! Each tool's `inputSchema` is a JSON Schema object understood by MCP
//! clients. Per-tool semantics live host-side in `src-tauri/src/mcp_host.rs`;
//! this crate is a translator that ships requests across the IPC bus and
//! returns the host's response.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// MCP `tools/call` parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool name (`bus_history`, `bus_tail`, …).
    pub name: String,
    /// Free-form arguments per the tool's `inputSchema`.
    #[serde(default)]
    pub arguments: Value,
}

/// Static descriptor for an MCP tool.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// MCP tool name as exposed to clients.
    pub name: &'static str,
    /// One-line description shown in `tools/list`.
    pub description: &'static str,
    /// JSON Schema describing the `arguments` shape.
    pub input_schema: Value,
}

/// Phase A + B + C + D tool catalog (22 tools — D-014 §3 Tier 1 + Tier 2 + Tier 3 + diagnostic).
pub fn tool_catalog() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bus_history",
            description: "Replay recent envelopes from the Rift bus (paginated).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Optional category filter (pty, hook, agent, fs, index, aegis, status, system, mcp).",
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1000,
                        "default": 100,
                    },
                },
            }),
        },
        ToolSpec {
            name: "bus_tail",
            description: "Stream live Rift bus envelopes as JSON-RPC notifications (method `notifications/rift/bus_tail`). Returns `{stream_started: true, request_id, filter}` synchronously; subsequent envelopes flow as notifications until the client disconnects.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Optional category filter (pty, hook, agent, fs, index, aegis, status, system, mcp).",
                    },
                    "kind_prefix": {
                        "type": "string",
                        "description": "Optional kind-prefix filter applied client-side in the host task. E.g. `aegis.session.` to only stream Aegis session events.",
                    },
                },
            }),
        },
        ToolSpec {
            name: "git_status",
            description: "Return the same payload as Rift's Git tab.",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        ToolSpec {
            name: "aegis_state",
            description: "Return the last `aegis.session.skill_loaded` snapshot (skill version + lessons-file path).",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        // ----- Phase B — Tier 1 read tools (D-014 §3) -----
        ToolSpec {
            name: "fs_read",
            description: "Read a project-relative text file. Path is validated against the current ProjectRoot — symlink + parent traversals out of the project tree are rejected. Files larger than 16 MiB return an error.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Project-relative path, e.g. `src/lib.rs`. Forward slashes accepted on all platforms.",
                    },
                },
                "required": ["path"],
            }),
        },
        ToolSpec {
            name: "fs_tree",
            description: "Static snapshot of the project filesystem subtree. Same shape as Rift's GUI Tree pane. Honors RiftConfig.fs.ignore_globs.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "max_depth": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 64,
                        "description": "Walk depth cap. Default 6 (matches the GUI Tree default).",
                    },
                },
            }),
        },
        ToolSpec {
            name: "todo_scan",
            description: "Scan project source for TODO / FIXME / XXX markers. Same payload as Rift's TODO notif tab.",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        ToolSpec {
            name: "pty_list",
            description: "Currently-tracked PTY sessions. Returns `{sessions: [{id, alive}], count}`. Per-session dimensions are not surfaced in v1.",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        ToolSpec {
            name: "cockpit_state",
            description: "Last `cockpit.state` snapshot — whether the GUI cockpit is detached from the main window. Defaults to `{detached: false}` if no snapshot has been published yet.",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        ToolSpec {
            name: "notif_tabs",
            description: "Last `notif.tabs` snapshot — the visible notification-tab catalog (id, title, enabled, detected, unread count). Empty `{tabs: []}` if App.svelte hasn't published yet.",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        // ----- Phase C — Tier 2 inspection tools (D-014 §3, default-off) -----
        ToolSpec {
            name: "dom_snapshot",
            description: "Return the full HTML of the named Rift webview window. Requires `mcp.allow_inspection = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["main", "cockpit"],
                        "description": "Which window to snapshot. Default: main.",
                    },
                },
            }),
        },
        ToolSpec {
            name: "screenshot",
            description: "Capture the named Rift webview window as a base64-encoded PNG. Requires `mcp.allow_inspection = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["main", "cockpit"],
                        "description": "Which window to capture. Default: main.",
                    },
                },
            }),
        },
        ToolSpec {
            name: "js_eval",
            description: "Evaluate JavaScript in the named Rift webview and return the result. Requires BOTH `mcp.allow_inspection = true` AND `mcp.allow_js_eval = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["main", "cockpit"],
                        "description": "Which window to evaluate in. Default: main.",
                    },
                    "code": {
                        "type": "string",
                        "description": "JavaScript code to evaluate. The last expression's value is returned as JSON.",
                    },
                },
                "required": ["code"],
            }),
        },
        // ----- Phase D — Tier 3 mutating + read tools (D-014 §3) -----
        ToolSpec {
            name: "pty_input",
            description: "Type text into a PTY session. Requires `mcp.allow_mutations = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "PTY session ID (from `pty_list`).",
                    },
                    "data": {
                        "type": "string",
                        "description": "Text to write to the PTY. Use \\r for Enter, \\x03 for Ctrl+C, etc.",
                    },
                },
                "required": ["id", "data"],
            }),
        },
        ToolSpec {
            name: "pty_read",
            description: "Read the current visible content of the terminal buffer. Returns the last N lines of terminal output. Requires `mcp.allow_inspection = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "lines": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "description": "Number of lines to read from the bottom of the buffer. Default: visible rows.",
                    },
                },
            }),
        },
        ToolSpec {
            name: "bus_publish",
            description: "Publish an envelope to the Rift bus. Requires `mcp.allow_mutations = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Envelope category (hook, agent, fs, index, aegis, status, system).",
                    },
                    "kind": {
                        "type": "string",
                        "description": "Envelope kind string, e.g. `hook.result`.",
                    },
                    "payload": {
                        "type": "object",
                        "description": "JSON payload to attach to the envelope.",
                    },
                },
                "required": ["category", "kind"],
            }),
        },
        ToolSpec {
            name: "fs_write",
            description: "Write text content to a project-relative file. Path is validated against the current ProjectRoot — traversals out of the project tree are rejected. Only writes to existing files in v1. Requires `mcp.allow_mutations = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Project-relative path, e.g. `src/lib.rs`. Forward slashes accepted on all platforms.",
                    },
                    "content": {
                        "type": "string",
                        "description": "Full file content to write.",
                    },
                },
                "required": ["path", "content"],
            }),
        },
        ToolSpec {
            name: "git_action",
            description: "Run a git mutating action in the project root. Supported actions: `fetch`, `pull`, `push`, `commit-all`. `commit-all` requires the `message` argument. Requires `mcp.allow_mutations = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["fetch", "pull", "push", "commit-all"],
                        "description": "Git action to perform.",
                    },
                    "message": {
                        "type": "string",
                        "description": "Commit message (required for `commit-all`, ignored otherwise).",
                    },
                },
                "required": ["action"],
            }),
        },
        ToolSpec {
            name: "simulate_click",
            description: "Simulate a mouse click at the given coordinates in a Rift webview window. Requires BOTH `mcp.allow_inspection = true` AND `mcp.allow_mutations = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["main", "cockpit"],
                        "description": "Which window to click in. Default: main.",
                    },
                    "x": {
                        "type": "number",
                        "description": "X coordinate (pixels from left edge of viewport).",
                    },
                    "y": {
                        "type": "number",
                        "description": "Y coordinate (pixels from top edge of viewport).",
                    },
                    "selector": {
                        "type": "string",
                        "description": "CSS selector to click instead of coordinates. If provided, x/y are ignored.",
                    },
                },
            }),
        },
        ToolSpec {
            name: "simulate_drag",
            description: "Simulate a mouse drag from one point to another in a Rift webview window. Requires BOTH `mcp.allow_inspection = true` AND `mcp.allow_mutations = true` in config.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["main", "cockpit"],
                        "description": "Which window to drag in. Default: main.",
                    },
                    "from_x": {
                        "type": "number",
                        "description": "Start X coordinate.",
                    },
                    "from_y": {
                        "type": "number",
                        "description": "Start Y coordinate.",
                    },
                    "to_x": {
                        "type": "number",
                        "description": "End X coordinate.",
                    },
                    "to_y": {
                        "type": "number",
                        "description": "End Y coordinate.",
                    },
                    "from_selector": {
                        "type": "string",
                        "description": "CSS selector for the drag source. If provided, from_x/from_y are ignored.",
                    },
                    "to_selector": {
                        "type": "string",
                        "description": "CSS selector for the drop target. If provided, to_x/to_y are ignored.",
                    },
                },
            }),
        },
        // ----- Post-Phase D — diagnostic + runtime config tools -----
        ToolSpec {
            name: "rift_diagnose",
            description: "Return Rift terminal health metrics: version, active PTY sessions, bus subscriber count, recent error count, lane classifier state, and terminal config.",
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        ToolSpec {
            name: "rift_config_set",
            description: "Update Rift terminal configuration at runtime. Changes are persisted to disk. Requires allow_mutations permission. Supports: font_size (8-48), line_height (1.0-2.5), scrollback (100-100000), lanes_enabled (bool), shell (auto/pwsh/cmd/bash/zsh).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "font_size": {
                        "type": "integer",
                        "minimum": 8,
                        "maximum": 48,
                        "description": "Terminal font size in pixels",
                    },
                    "line_height": {
                        "type": "number",
                        "minimum": 1.0,
                        "maximum": 2.5,
                        "description": "Terminal line height multiplier",
                    },
                    "scrollback": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 100000,
                        "description": "Terminal scrollback buffer lines",
                    },
                    "lanes_enabled": {
                        "type": "boolean",
                        "description": "Enable/disable lane classification coloring",
                    },
                    "shell": {
                        "type": "string",
                        "enum": ["auto", "pwsh", "powershell", "cmd", "bash", "zsh", "sh"],
                        "description": "Preferred shell for new terminal sessions",
                    },
                },
                "additionalProperties": false,
            }),
        },
    ]
}
