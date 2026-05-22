# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Status

**v0.2.1 (2026-05-21): performance (notif.tabs debounce, bus.tail()), stability (parking_lot::Mutex migration — 8 crash paths eliminated), MCP server `instructions` for Claude Rift-awareness, a11y/polish (ShortcutOverlay, focus-visible, StatusLine overflow). v0.2.0 free open beta (2026-05-21). Beta launch sprint: component test infrastructure (@testing-library/svelte, 22 new tests), welcome overlay (5-step first-run guide), getting-started docs, macOS CI/release workflows, crash dump infrastructure (Rust panic hook + JS error capture + Settings viewer), release runbook, BETA badge + Patreon links.** Phase 8 (Index integration tab + graph view) CLOSED — all subphases 8.0-8.7 shipped; D-019 remade IndexGraph as vault browser list (D3 deps removed). Post-v1 work active: D-014 MCP tool surface (20 tools, Phases A-D complete), D-018 live lane classification (closed C-023). Remaining deferrals: D-010 (Sentinel, post-v1 by design), D-012 (StatusLine — SESSION/SKILL/EFFORT now live; CTX/USE/WEEK upstream-blocked on CC hook schema), D-020 (heatmap — closed 2026-05-16, implemented). Audit status: 40/50 findings resolved, 5 accepted as v1-adequate, 0 post-v1 remaining. Vision and architecture locked at v0.6 (2026-04-27); §10.15 real-time-update mechanism LOCKED via `decisions/§10.15_real-time_update_mechanism.md`; §10.17 agent tab grouping resolved via `decisions/§10.17_agent_tab_grouping_filtering.md`. Workspace has four Rust crates (`crates/rift-bus` — protocol/transport/translators, `crates/rift-cli` — `rift hook`/`rift status` CLI, `crates/rift-core` — PTY abstraction, `crates/rift-aegis` — private optional feature-gated path dep) + `crates/rift-mcp` (MCP translator), the `src-tauri/` Tauri shell, and the Svelte 5 frontend under `src/`. §9 Integration Decoupling is CI-enforced. All 3 mockups (terminal alone / GUI alone detached / integrated cockpit) live; see `DEFERRED.md` closed-deferrals section for ship history.

## Build / Test / Lint

CI commands (mirror `.github/workflows/ci.yml`):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
npm run check
bash tools/check-translator-boundary.sh
```

Run `npm run tauri:dev` to spawn the dev environment (Tauri shell + Vite at `http://localhost:1420`). The boundary check enforces §9 — direct external-system primitives (`tokio::net::`, `reqwest::`, `claude_*`, `mcp_*`) outside `crates/rift-bus/src/translators/` (or the bus's internal `ipc.rs`) fail CI. `bash tools/check-translator-boundary.sh --help` for usage; `--test` mode confirms the check catches violations end-to-end.

## Source-of-Truth Files

- `RIFT_V2_VISION.md` — locked vision and architecture. Read this before any design or implementation decision; section numbers cited below refer to it.
- `rift-v2-mockup.html` — visual source of truth for terminal aesthetic, lane colors, tag style, tabs, status line, and active-state density (§10.3). Reference the mockup, not memory of a conversation. Open it in a browser to see the active state.

## What Rift v2 Is (and Isn't)

Rift v2 is a **standalone terminal + GUI graph cockpit** — one product, two surfaces. It is **not a wrapper** around another terminal (that was v1's "original sin"). The GUI is **in scope for v1**, not deferred (§1, §11). Internal build phases the terminal first then the GUI on top, but v1 ships as both.

v1 lives on disk as a "cautionary museum exhibit" — do not import or reuse code from it. Only the *concepts* of the hook system and agent observability layer transfer (§4).

## Stack (Locked)

**Tauri** — Rust backend + web frontend (§5).
- Rust handles terminal emulation, hooks, integration protocol, real-time event feed.
- Webview hosts xterm.js for the terminal surface and a vault browser list for the GUI cockpit (§10.18 resolved: D3 chosen then D-019 replaced with list-based layout; D3 deps removed).
- CSS owns the Abyssal Arts aesthetic. Reopen the stack decision only on a concrete blocker.

## Load-Bearing Architecture

### Integration Decoupling Principle (§9)

**Rift core must never speak directly to Claude Code, Aegis, MCP servers, or any external system.** All external interaction goes through translator modules that map between the external interface and Rift's internal event/state protocol. Two enforcement reasons:

1. Claude Code's hook/skill/MCP surface is evolving fast — hardcoded assumptions become maintenance crises.
2. Aegis is proprietary and must remain decoupled. Rift core ships standalone; Aegis (or any agent system) plugs in by speaking the protocol.

The protocol supports three capability classes: **event subscription**, **control endpoints** (declared actions Rift can invoke), and **data enrichment** (metadata attached to filesystem nodes/events). Feature detection happens at runtime — no "integration not found" errors; the UI looks complete with whatever's there.

A "just a quick direct call" outside a designated translator module is a violation. Sentinel and (when present) Aegis should both flag this at build time.

### Sentinel ↔ Rift split (§10.11)

Sentinel is the **source of truth** for agent misbehavior detection (stuck, runaway, unauthorized edits). Rift is the **display layer** — it surfaces Sentinel's events through the Agents tab. Do not duplicate Sentinel's detection logic inside Rift.

### GUI foundation = filesystem activity, not agent activity (§11)

The graph cockpit's foundation is filesystem activity (reads/writes/creates/deletes) — always-on, present in every install. Agent attribution and Index enrichment are **layers on top**, provided by integrations. Bare Rift renders anonymous filesystem activity; integrations make it richer. This produces the layered value model: bare → +attribution → +enrichment → full cockpit.

## Visual System (Locked)

### Color-coded lanes (§10.1)

Every output line is routed to one of these lanes. Tag prefixes are small bordered uppercase boxes (`CLAUDE`, `AGENT`, `HOOK`, `AEGIS`, `OK`, `WARN`, `ERR`, `SYS`). Border color matches lane.

| Lane | Hex | CSS var | Use |
|---|---|---|---|
| Amber bright | `#FFC840` | `--amber-bright` | Prompt / cursor |
| Off-white | `#E8E4D8` | `--term-white` | User input |
| Blue | `#6CB6FF` | `--term-blue` | Claude voice |
| Purple | `#C58FFF` | `--term-purple` | Agents |
| Cyan | `#6FE0E0` | `--term-cyan` | Hooks |
| Amber primary | `#FFA826` | `--amber-primary` | Aegis |
| Terminal green | `#4FE855` | `--term-green` | Success |
| Terminal red | `#FF4848` | `--term-red` | Errors / warnings |
| Faint amber italic | `#A87830` | `--amber-faint` | Meta / timestamps |

Aesthetic: matte black textured background, amber/terminal-tone accents, JetBrains Mono, scanlines + CRT vignette as brand fingerprint. Match `rift-v2-mockup.html` — do not improvise.

### Surface taxonomy (§10.5)

- **Tab** = persistent surface (sessions, notification tabs).
- **Pane** = split inside a tab, OR a notification tab promoted to live alongside the main terminal. Only one tab can be promoted at a time in v1.
- **Pop-out** = ephemeral (rule editor, file viewer, agent cancel confirm).

Drag-tab-out promotes to pane; drag-pane-back returns to tab. Same gesture (one level up) handles GUI window detach for multi-monitor (§11).

### Tab anatomy (§10.4, §10.8)

Every notification tab has the same internal anatomy: status header / live activity strip / recent events log / persistent state panel. Tabs support 4 modular sections drawn from an extensible self-discovering catalog (§10.10) — new event types added by integrations register new section types automatically.

### Default tab set is capability-driven (§10.7)

Bare install: Errors / Hooks / Commands / one open slot. Integrations add tabs (Aegis, Agents, Index, …) when present. No "missing integration" warnings.

### Status line (§10.2)

Two rows, all values bold, budget-style values as percentages, color-block backgrounds with dark text. Row 1: `DIR / MODEL / CTX% / SESSION% / SKILL`. Row 2: `GIT / REPO / SESSION USE% / WEEK%`.

The `SKILL` segment is a deliberate addition — always-on confirmation that the expected skill (typically `aegis`) is loaded.

## Anti-Patterns — The "Fired" List (§7)

These are enforceable build-time rules, not suggestions.

- **No wrapper architecture, ever.** This is v1's original sin.
- **No shortcuts.** Spec says a thing → the thing gets built. Watch for shortcut-signal phrases like `// for now`, stub returns, mock data. The mockup demonstrates the expected detection behavior on line 492.
- **No silent stubbing or deferring.** If something gets deferred, it gets logged loudly in a dedicated file, not buried in a code comment.
- **No floating text.** Every UI element belongs to a tab, pane, or pop-out.
- **No telling the user when the session should end** — and no guessing at problems. Research, scout, read the code/docs.
- **If documentation is missing from vault, add it** — don't work around the gap.
- **Don't ask the user to do something Claude can do** — run the commands.
- **Pre-task and post-task Aegis audits stay on full** during build (§8). Every session ends with "did we build what the spec said, or did we quietly amputate something?"

## Editor Scope Bound (§11)

The in-cockpit editor exists for **friction reduction only** — spot something in the graph, fix it, return to flow. In scope: full syntax highlighting (tree-sitter or equivalent) across all languages, quick edit/save. **Out of scope: multi-file refactoring, debug tooling, extensions, anything that competes with a real editor.** This boundary must hold during build.

## Out of Scope for v1

- Native Android/tablet client (post-v1, §13 — companion SSH-headless client; architecture must support it without refactoring).
- Anything not named in §2–3, §10, or §11.
