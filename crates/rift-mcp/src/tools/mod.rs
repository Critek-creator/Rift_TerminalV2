//! Tool catalog — the 4 read-only Tier 1 tools shipped in Phase A.
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

/// Phase A tool catalog (4 tools — D-014 §11 q7).
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
    ]
}
