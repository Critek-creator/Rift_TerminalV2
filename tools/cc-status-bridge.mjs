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

// Parse new fields for enrichment
let effortLevel = "";
let thinkingEnabled = null;
try {
  const parsed = JSON.parse(input);
  effortLevel = parsed?.effort?.level || "";
  thinkingEnabled = parsed?.thinking?.enabled ?? null;
} catch { /* best-effort */ }

// Tee to temp file for Rift's status translator
try {
  writeFileSync(STATUS_FILE, input);
} catch {
  // Non-fatal — Rift just won't get CC data this tick
}

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
