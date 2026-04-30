//! Tool catalog — Phase A (4 tools) + Phase B (6 tools) read-only Tier 1.
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

/// Phase A + Phase B tool catalog (10 tools — D-014 §3 Tier 1 read tools).
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
    ]
}
