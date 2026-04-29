---
description: Snapshot of the connected Rift instance — git state, aegis state, and last 50 bus envelopes.
allowed-tools:
  - "mcp__plugin_rift-mcp_rift__bus_history"
  - "mcp__plugin_rift-mcp_rift__git_status"
  - "mcp__plugin_rift-mcp_rift__aegis_state"
---

Pull a quick situational snapshot of the running Rift instance:

1. Call `mcp__plugin_rift-mcp_rift__git_status` for branch, ahead/behind,
   staged/modified/untracked counts.
2. Call `mcp__plugin_rift-mcp_rift__aegis_state` for the last
   `aegis.session.skill_loaded` snapshot (skill version + lessons-file path).
3. Call `mcp__plugin_rift-mcp_rift__bus_history` with `limit: 50` to get
   the most recent envelope flow.

Summarise in three short sections:

- **git** — branch, dirty/clean, ahead/behind
- **aegis** — skill version (or "not loaded") + lessons path
- **recent activity** — counts per `Category` from the bus_history result
  with the most recent kind in each category

Keep it terse. The user wants a glance, not an essay.
