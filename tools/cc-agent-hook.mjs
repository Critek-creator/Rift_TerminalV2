#!/usr/bin/env node
//
// CC Agent Hook — publishes agent lifecycle events to Rift's bus so the
// Agents notification tab populates automatically.
//
// Claude Code hook configuration (.claude/settings.json):
//
//   "hooks": {
//     "PreToolUse": [
//       {
//         "matcher": "Agent",
//         "hooks": [{
//           "type": "command",
//           "command": "node <repo>/tools/cc-agent-hook.mjs start"
//         }]
//       }
//     ],
//     "SubagentStop": [
//       {
//         "matcher": "",
//         "hooks": [{
//           "type": "command",
//           "command": "node <repo>/tools/cc-agent-hook.mjs end"
//         }]
//       }
//     ]
//   }
//
// Reads hook input from stdin (JSON). Writes an agent event to a JSONL
// file that the Rift agent-events translator tails, and also attempts
// a direct IPC publish via the rift CLI if available.
//
// ID correlation: PreToolUse and SubagentStop provide different input
// shapes. We normalize the agent ID via a registry file so start/end
// events always correlate — even if CC uses different field names.

import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const EVENT_FILE = join(tmpdir(), "rift-agent-events.jsonl");
const REGISTRY_FILE = join(tmpdir(), "rift-agent-registry.json");
const action = process.argv[2]; // "start" or "end"

const chunks = [];
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) {
  chunks.push(chunk);
}
const raw = chunks.join("").trim();
if (!raw) process.exit(0);

let input;
try {
  input = JSON.parse(raw);
} catch {
  process.exit(0);
}

// --- Registry: maps agent names/descriptions to stable IDs ---

function loadRegistry() {
  try {
    return JSON.parse(readFileSync(REGISTRY_FILE, "utf8"));
  } catch {
    return { agents: {}, counter: 0 };
  }
}

function saveRegistry(reg) {
  try {
    writeFileSync(REGISTRY_FILE, JSON.stringify(reg));
  } catch { /* best-effort */ }
}

function pruneStale(reg) {
  const cutoff = Date.now() - 3_600_000; // 1 hour
  for (const [key, entry] of Object.entries(reg.agents)) {
    if (entry.ts < cutoff) delete reg.agents[key];
  }
}

const ts = Date.now();
let event;

if (action === "start") {
  const toolInput = input.tool_input ?? input.input ?? {};
  const name = toolInput.description || toolInput.name || "subagent";
  const subagentType = toolInput.subagent_type ?? "fork";

  const reg = loadRegistry();
  pruneStale(reg);
  reg.counter = (reg.counter || 0) + 1;
  const id = `cc-agent-${reg.counter}`;

  // Register under multiple keys so SubagentStop can find it by
  // whichever field CC provides (agent_name, name, or description).
  const keys = [name, toolInput.name, toolInput.description].filter(Boolean);
  for (const k of keys) {
    reg.agents[k] = { id, ts };
  }
  saveRegistry(reg);

  event = {
    kind: "agent.start",
    ts,
    payload: { id, name, kind: subagentType, source: "claude-code" },
  };

} else if (action === "end") {
  const agentName = input.agent_name ?? input.name ?? input.description ?? "";

  const reg = loadRegistry();
  const entry = reg.agents[agentName];
  const id = entry?.id ?? `cc-agent-unknown-${ts}`;

  // Clean up all registry keys pointing to this ID.
  if (entry) {
    for (const [key, val] of Object.entries(reg.agents)) {
      if (val.id === id) delete reg.agents[key];
    }
    saveRegistry(reg);
  }

  event = {
    kind: "agent.end",
    ts,
    payload: {
      id,
      status: input.error ? "error" : "completed",
      message: input.error ?? input.summary ?? undefined,
    },
  };

} else {
  process.exit(0);
}

// Write to JSONL file for the translator to pick up.
try {
  appendFileSync(EVENT_FILE, JSON.stringify(event) + "\n");
} catch {
  // Non-fatal — the translator might not be running.
}

// Attempt direct IPC publish via rift CLI (best-effort).
try {
  execFileSync("rift", [
    "hook",
    "--category", "agent",
    event.kind,
    "--payload", JSON.stringify(event.payload),
    "--no-stdin",
  ], { timeout: 2000, stdio: "ignore" });
} catch {
  // rift CLI not in PATH or IPC not available — the JSONL file
  // is the fallback. Non-fatal.
}
