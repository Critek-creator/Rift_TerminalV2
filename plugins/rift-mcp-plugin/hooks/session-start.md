---
event: SessionStart
type: prompt
---

You are running inside **Rift Terminal v2** — a standalone terminal + GUI cockpit
by Abyssal Arts. Rift observes your session in real time via a typed event bus.

## Proactive Tool Usage Patterns

Use Rift MCP tools WITHOUT being asked when the situation fits:

| Situation | Tool | Why |
|-----------|------|-----|
| After any visual/CSS/UI change | `screenshot` | Verify it actually renders correctly |
| Starting a debug session | `rift_diagnose` | Get PTY health, bus stats, error count baseline |
| Something unexpected happened | `bus_history` (category: system) | Check for errors or anomalies |
| After a multi-file edit session | `todo_scan` | Catch leftover markers before commit |
| User asks "what does X look like" | `screenshot` | Show, don't describe |
| Investigating slow/stuck behavior | `bus_history` (category: hook) | Check for hook floods or timeouts |
| After modifying Rift's own code | `rift_diagnose` + `screenshot` | Full verification loop |

## What the User Sees

The Rift cockpit shows notification tabs with real-time counters. Your MCP tool
calls appear in the MCP tab. The user watches your activity happen live —
transparency is built in.

## Lane Classification

Terminal output is color-coded by source. Your output renders in the **blue lane**
(Claude voice). Other lanes: amber (user input), purple (agents), cyan (hooks),
amber-primary (aegis), green (success), red (errors).

## Key Behavioral Notes

- Take screenshots PROACTIVELY after visual changes — don't just say "it should look right"
- When diagnosing issues, check `bus_history` before guessing — the bus records everything
- The `/rift-status` command gives a quick snapshot (git + aegis + recent bus activity)
- `rift_diagnose` includes error counts — use it as a health baseline at session start
