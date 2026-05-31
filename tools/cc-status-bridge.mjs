#!/usr/bin/env node
//
// CC StatusJSON bridge — tees Claude Code's statusLine JSON to a temp file
// so Rift's status translator can read it, then forwards to ccstatusline
// for rendering.
//
// Claude Code settings usage:
//   "statusLine": {
//     "type": "command",
//     "command": "node <absolute-path>/tools/cc-status-bridge.mjs"
//   }
//
// Data flow:
//   CC StatusJSON (stdin) → write %TEMP%/rift-cc-status.json → pipe to ccstatusline → stdout

import { writeFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const STATUS_FILE = join(tmpdir(), "rift-cc-status.json");

const chunks = [];
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) {
  chunks.push(chunk);
}
const input = chunks.join("");

if (!input.trim()) {
  process.exit(0);
}

// Tee to temp file for Rift's status translator. This is the ONLY thing Rift
// reads — its GUI status line (StatusLine.svelte ← status.rs ← this file) is
// fully independent of whatever we print to stdout below.
try {
  writeFileSync(STATUS_FILE, input);
} catch {
  // Non-fatal — Rift just won't get CC data this tick
}

// ── Rift-aware rendering ────────────────────────────────────────────────────
// When this command runs inside a Rift PTY, Rift injects $RIFT_SOCKET_NAME
// (src-tauri/src/lib.rs — pty_start). Inside Rift, the GUI status line already
// renders all of this data, so printing an in-terminal status bar here just
// duplicates it. We stay SILENT (tee-only) so Rift owns the single status line,
// while CC stays fully installed and the data keeps flowing to the GUI.
//
// Outside Rift (a plain terminal CC session), $RIFT_SOCKET_NAME is absent and
// we fall through to the normal ccstatusline rendering below — so CC users who
// run elsewhere keep their in-terminal status bar unchanged.
//
// To force one behaviour regardless of host: set RIFT_STATUSLINE=silent (always
// tee-only) or RIFT_STATUSLINE=render (always render the in-terminal bar).
const mode = process.env.RIFT_STATUSLINE;
const insideRift = process.env.RIFT_SOCKET_NAME !== undefined;
const silent = mode === "silent" || (mode !== "render" && insideRift);
if (silent) {
  process.exit(0);
}

// Parse new fields for enrichment (only needed for the in-terminal rendering).
let effortLevel = "";
let thinkingEnabled = null;
try {
  const parsed = JSON.parse(input);
  effortLevel = parsed?.effort?.level || "";
  thinkingEnabled = parsed?.thinking?.enabled ?? null;
} catch { /* best-effort */ }

// Forward to ccstatusline for CC's own status bar rendering.
// Try bunx first (faster), fall back to npx.
const runners = [
  { cmd: "bunx", args: ["-y", "ccstatusline@latest"] },
  { cmd: "npx", args: ["-y", "ccstatusline@latest"] },
];

let statusOutput = "";
for (const { cmd, args } of runners) {
  try {
    statusOutput = execFileSync(cmd, args, {
      input,
      encoding: "utf8",
      stdio: ["pipe", "pipe", "ignore"],
      timeout: 10_000,
      windowsHide: true,
    });
    break;
  } catch {
    // Try next runner
  }
}

// Append effort + thinking segments if ccstatusline didn't surface them
const segments = [];
if (effortLevel && !statusOutput.includes(effortLevel)) {
  segments.push(`effort:${effortLevel}`);
}
if (thinkingEnabled !== null && !statusOutput.includes("thinking")) {
  segments.push(thinkingEnabled ? "thinking:on" : "thinking:off");
}

if (segments.length > 0 && statusOutput.trim()) {
  statusOutput = statusOutput.trimEnd() + " | " + segments.join(" | ") + "\n";
} else if (segments.length > 0) {
  statusOutput = segments.join(" | ") + "\n";
}

if (statusOutput) {
  process.stdout.write(statusOutput);
}
process.exit(0);
