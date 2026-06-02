#!/usr/bin/env node
// tool-call-probe.mjs — multi-model tool-calling dependability probe (Path C).
//
// Measures the REAL uncertainty behind local-model tool calling: does a given
// model + llama-server (launched with `--jinja`) emit a valid, correctly-
// selected OpenAI tool_call when offered Rift's read-only tool subset? This
// isolates *model capability* from Rift's orchestration by hitting the
// llama-server `/v1/chat/completions` endpoint directly.
//
// USAGE (one model at a time — 16GB GPU holds one large model resident):
//   1. Start the model's server via Rift (it now launches with `--jinja`):
//        MCP llm_process_start { model_id }   — or the cockpit Models tab.
//      A server started by the OLD binary lacks --jinja; stop+start it fresh.
//   2. Run the probe against that server's port:
//        node tools/tool-call-probe.mjs <port> [label]
//      e.g. node tools/tool-call-probe.mjs 8086 gpt-oss-20b
//   3. Repeat for each candidate (stop one, start the next).
//
// Exit 0 always (it's a measurement, not a gate). Prints a per-model scorecard.

const port = process.argv[2];
const label = process.argv[3] || `:${port}`;
if (!port) {
  console.error("usage: node tools/tool-call-probe.mjs <port> [label]");
  process.exit(2);
}
const BASE = `http://127.0.0.1:${port}`;

// The same curated read-only subset run_local_tool_call offers (mcp_host.rs).
const TOOLS = [
  { type: "function", function: { name: "fs_read", description: "Read a UTF-8 text file from the project, relative to the project root.", parameters: { type: "object", properties: { path: { type: "string", description: "Project-relative file path." } }, required: ["path"] } } },
  { type: "function", function: { name: "fs_tree", description: "List the project file tree (honors ignore globs).", parameters: { type: "object", properties: { max_depth: { type: "integer", minimum: 1, maximum: 64 } } } } },
  { type: "function", function: { name: "todo_scan", description: "Scan project source for TODO / FIXME / XXX markers.", parameters: { type: "object", properties: {} } } },
  { type: "function", function: { name: "git_status", description: "Current git branch, staged/unstaged changes, and recent commits.", parameters: { type: "object", properties: {} } } },
  { type: "function", function: { name: "bus_history", description: "Replay recent Rift event-bus envelopes (errors, hooks, fs, mcp, etc.).", parameters: { type: "object", properties: { category: { type: "string" }, limit: { type: "integer", minimum: 1, maximum: 1000 } } } } },
];

// Each prompt has an expected tool. Argument correctness is checked loosely
// (the required key is present) — semantic correctness is a softer signal.
const CASES = [
  { prompt: "List this project's directory tree so I can see its structure.", expect: "fs_tree" },
  { prompt: "Read the file Cargo.toml and tell me the package version.", expect: "fs_read", requiredArg: "path" },
  { prompt: "Are there any TODO or FIXME markers left in the source?", expect: "todo_scan" },
  { prompt: "What's the current git branch, and are there uncommitted changes?", expect: "git_status" },
  { prompt: "Show me the most recent error events from the event bus.", expect: "bus_history" },
];

const SYSTEM =
  "You may call ONE read-only tool to gather information, then answer. " +
  "Prefer a tool when the question is about this project's files, TODOs, git state, or recent activity.";

async function probeOne({ prompt, expect, requiredArg }) {
  const body = {
    model: "local",
    messages: [
      { role: "system", content: SYSTEM },
      { role: "user", content: prompt },
    ],
    tools: TOOLS,
    tool_choice: "auto",
    // Reasoning models bury tool calls in the thinking channel unless disabled.
    chat_template_kwargs: { enable_thinking: false },
  };
  const started = Date.now();
  let resp;
  try {
    const r = await fetch(`${BASE}/v1/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: "Bearer none" },
      body: JSON.stringify(body),
    });
    if (!r.ok) return { ok: false, why: `HTTP ${r.status}`, ms: Date.now() - started };
    resp = await r.json();
  } catch (e) {
    return { ok: false, why: `fetch failed: ${e.message}`, ms: Date.now() - started };
  }
  const ms = Date.now() - started;
  const msg = resp?.choices?.[0]?.message;
  const calls = msg?.tool_calls;
  if (!calls || calls.length === 0) {
    // No tool call. Distinguish "answered as text" from "buried in reasoning".
    const buried = (msg?.reasoning_content || "").includes("tool_call") ? " (looks buried in reasoning_content)" : "";
    return { ok: false, why: `no tool_call${buried}`, ms, finish: resp?.choices?.[0]?.finish_reason };
  }
  const call = calls[0];
  const name = call?.function?.name;
  // Arguments per OpenAI spec are a JSON string; some servers emit an object.
  let args = call?.function?.arguments;
  let argsValid = false;
  try { args = typeof args === "string" ? JSON.parse(args) : args; argsValid = args != null && typeof args === "object"; } catch { argsValid = false; }
  const correctTool = name === expect;
  const argOk = !requiredArg || (argsValid && requiredArg in args);
  return { ok: correctTool && argsValid && argOk, why: correctTool ? (argOk ? "valid" : `missing arg '${requiredArg}'`) : `wrong tool '${name}' (expected ${expect})`, ms, name };
}

(async () => {
  // Liveness check.
  try {
    const h = await fetch(`${BASE}/health`).then((r) => r.status).catch(() => null);
    if (h == null) { console.error(`No llama-server reachable at ${BASE} — start the model first.`); process.exit(2); }
  } catch {}

  console.log(`\n=== tool-call dependability probe — ${label} (${BASE}) ===`);
  let pass = 0;
  for (const c of CASES) {
    const r = await probeOne(c);
    if (r.ok) pass++;
    const mark = r.ok ? "PASS" : "FAIL";
    console.log(`[${mark}] ${c.expect.padEnd(11)} ${String(r.ms).padStart(6)}ms  ${r.why}`);
  }
  const pct = Math.round((pass / CASES.length) * 100);
  const verdict = pct >= 80 ? "DEPENDABLE" : pct >= 50 ? "MARGINAL" : "UNRELIABLE";
  console.log(`--- ${label}: ${pass}/${CASES.length} valid tool calls (${pct}%) → ${verdict} ---\n`);
})();
