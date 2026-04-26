---
title: Rift Terminal v2 — Vision
version: 0.5 (window architecture + mockup plan locked)
status: Vision locked, architecture mostly locked, one technical item open
date: 2026-04-26
author: Garth (Critek)
project: Abyssal Arts
supersedes: Rift Terminal v1 (kept as cautionary exhibit)
---

# Rift Terminal v2 — Vision

> "I live in the terminal and it is meant to be the main focus of the whole."

## 1. What Rift v2 IS

A **standalone** terminal — not a wrapper, not a frame around another terminal. Full ownership of the canvas. Built for someone who lives in the terminal and wants to *enjoy* being there. Claude-first, Aegis-integrated, Abyssal Arts aesthetic, professional and high-quality throughout.

**Rift is a cockpit, not a terminal.** The complete vision is terminal + GUI graph cockpit together — two surfaces of one experience. The GUI is not an add-on or a "later" feature; it is the reason Rift exists. Every other terminal shows text. Rift shows the command line *plus a live visualization of the system being worked on*.

**Shipping framing:** v1 = the complete vision (terminal + GUI). Internal build happens in phases (terminal foundation first, GUI layer built on top), but what ships is Rift-as-intended, not "Rift minus the interesting part." Deferring the GUI would be the biggest possible violation of section 7's anti-patterns.

## 2. What Rift v2 LOOKS LIKE

**Aesthetic:** Same visual language as Brain Dump. Abyssal Arts brand — matte black textured background, amber/terminal-tone accents, JetBrains Mono, on-brand throughout.

**Layout:**
- Proper tabs — easy click to add, easy click to close sessions
- Notification tabs anchored far right (separate visual zone from session tabs)
- Status line at bottom — ccstatusline-style, important info at a glance
- Color-coded text for legibility and low friction: see what matters, what's important, what went wrong, what Claude is doing
- No floating text. Every element has a place — either a tab, a pane, or a pop-out panel

**Feel:** Solid. Reliable. Professional. Low friction. Enjoyable to inhabit.

## 3. Core Behaviors That Make It Rift (Not Just A Terminal)

- **Standalone terminal** — owns rendering, typography, prompt, cursor, the whole experience. No leeching.
- **Color-coded legibility system** — information hierarchy is visual. Errors, Claude output, hook activity, agent status each have their lane.
- **Agent visibility & tracking** — see what agents are doing in real time. Ability to cancel if they run off.
- **Integration-ready, not integration-coupled** — Rift core works standalone and lights up additional capabilities when integrations are loaded. Aegis (when present) and the Abyssal Index (when present) plug in via the Rift Integration Protocol; they are not hardcoded into the core. See section 9.
- **Status line** — ccstatusline-inspired. High-value info at a glance.
- **Easy copy/paste** — left and right click functionality. Standard behavior that other terminals fumble, handled correctly.
- **Session control is the user's** — no auto-ending, no guessing problems, no "should we stop here" nudges.

## 4. What Transfers From v1

- **Hook system** — the genuinely valuable core. Salvageable.
- **Agent observability layer** — the logic, not the UI. Transfers.
- **Knowledge gained** — what the agent-terminal workflow actually wants to be. v1's job was to teach this; job done.

v1 itself stays on disk, surrounded in red tape, as a cautionary museum exhibit: "Unacceptable work example — this crap will get you fired."

## 5. Stack Decision

**LOCKED: Tauri** (Rust backend + web frontend).

Reasoning:
- GUI requirement (section 10) effectively forces it. Native Rust TUI cannot host a pannable graph with clickable nodes, drag-to-terminal, and a custom syntax-aware file viewer without reinventing a browser engine.
- Tauri webview gives the GUI layer natively; mature graph libs (D3, Cytoscape, Sigma, etc.) live in web.
- Rust backend handles terminal emulation, hooks, Aegis, Index, and the real-time event feed to the GUI.
- xterm.js is mature and solves the "terminal emulator is hard" problem for the terminal surface.
- Shared stack direction with Brain Dump PC (already heading to Tauri) — one ecosystem, shared tooling.
- CSS gives total aesthetic control for the Abyssal Arts look.

Decision date: 2026-04-24. Reopen only if a concrete blocker emerges.

## 6. Explicitly Out Of Scope For v1

- **Native Android/tablet client** — on the broader Abyssal Arts roadmap, tracked separately.
- **Anything not named in sections 2–3 or 10** — scope-creep firewall.

(Note: the GUI cockpit is **in scope for v1** per section 1. Internal build phases the terminal first, but v1 as a shipped milestone includes both surfaces. See section 11 for full GUI vision.)

## 7. Anti-Patterns — Things v1 Did That v2 MUST NOT Do

The "fired" list. These are enforceable build-time rules, not suggestions.

- **No shortcuts.** Spec says a thing, the thing gets built.
- **No deferring unless absolutely necessary** — and "necessary" requires justification, not convenience.
- **No rush to ship.** There is no door. There is no deadline. There is no PM. "Get it out the door" is not a valid reason for any decision.
- **No feature skip.** If it's in the spec, it's in the build.
- **Must stay professional and high quality** throughout. No 3rd-grade art project outcomes.
- **No floating text.** Every element has a place — tab, pane, or pop-out panel.
- **No telling the user when the session should end.**
- **No guessing at the problem.** Research. Scout. Read the code. Read the docs. Find the answer.
- **If documentation is missing from vault, add it.** Don't work around a gap — close it.
- **Don't ask the user to do something Claude can do itself.** Run the damn commands.
- **No wrapper architecture.** Ever. Again. (This is the original sin that produced v1.)

## 8. Build Process Requirements

Before v2 implementation starts, the build process itself must have these guard rails active:

- **Aegis watchdog on full.** Pre-task and post-task audits. The spec-drift that produced v1 must be caught before it produces v2.
- **Spec adherence verification** — every session ends with "did we build what the spec said, or did we quietly amputate something?"
- **No silent stubbing.** If something gets stubbed or deferred, it gets logged loudly in a dedicated file, not buried in code comments.

Without these, v2 becomes v1 with better paint.

## 9. Integration Decoupling Principle

Rift's core never speaks directly to external systems (Claude Code, Aegis, MCP servers, Anthropic APIs, etc.). All external interactions go through **translator modules** that map between the external interface and Rift's internal event/state protocol. External systems can change without touching the core, **and any specific integration is optional** — Rift core works standalone, integrations light up additional capabilities.

This is the same architectural pattern as VS Code extensions or LSP: capabilities are advertised by integrations, the host adapts.

### Why this matters

Two reasons:

1. **Claude Code is moving fast.** Hooks, skills, MCP, the whole surface is evolving on a timeline Rift does not control. Hardcoding assumptions about how Aegis fires, what hook events look like, how skills register, or what an MCP tool call returns turns every Claude Code update into a Rift maintenance crisis. The same trap Rift v1 fell into at a different scale.
2. **Aegis is proprietary and cannot ship coupled to Rift.** Rift must be a complete, sellable product *without* Aegis present. Aegis remains private; Rift core remains decoupled. Aegis (or any agent system) plugs in by speaking the protocol.

### Two-document architecture

The integration spec splits into two documents:

- **The Rift Integration Protocol** *(public, eventually)* — the contract any system must speak to plug into Rift. Defines event schema, capability declaration, control endpoint format, and data enrichment format.
- **The Aegis ↔ Rift Module** *(private)* — a translator that makes Aegis speak the protocol. Garth's private implementation. The Aegis moat stays intact.

Rift itself never knows Aegis by name. Other systems (anyone's) can plug in by speaking the protocol.

### Three capability classes

The protocol must support three distinct capability classes:

1. **Event subscription** — integrations emit events, Rift renders them.
2. **Control endpoints** — integrations declare what actions Rift can invoke (e.g. "pause this agent"). Rift renders UI based on declared capabilities. Read-only integrations simply don't declare any.
3. **Data enrichment** — integrations attach metadata to filesystem nodes / events (e.g. Index attaches vault relationships and semantic tags to filesystem nodes). Other systems can attach their own enrichment.

### Feature detection at runtime

Rift starts up, queries the environment for available integrations, renders UI accordingly. **No "integration not found" errors.** The UI looks complete with whatever's there.

### How it works in practice

- **Each integration is a translator module.** It speaks Rift's internal protocol on one side and the external interface on the other side. When the external system changes, the module updates — the core does not.
- **Same pattern for hooks, skills, MCP tools, Claude itself.**
- **New integrations are additive, not surgical.** Adding support for a new Claude Code feature is a new module, not a refactor.
- **The future mobile/SSH-headless surface is just another subscriber** to the internal protocol. It does not need to know anything about Aegis or hooks specifically.

### Build-time enforcement

This principle is load-bearing. It is easy to violate during build by writing "just a quick direct call" that grows roots. Sentinel and (when present) Aegis should both check for direct external-system calls outside of designated module locations. Violations get flagged.

### What this principle does NOT mean

- It does not mean abstracting everything to the point of paralysis. Modules should be thin and obvious, not over-engineered.
- It does not mean Rift cannot have opinions. The internal protocol can be opinionated. The point is the core doesn't know which external system produced an event — it just knows the event shape.

---

## 10. Planning Decisions (locked) + Open Questions (remaining)

### 10.1 Locked — Color-coding system

The lane assignment, established by the v0.2 mockup and confirmed in planning. All terminal output gets routed to one of these lanes, with a small bordered tag prefix for vertical scan-ability.

| Lane | Hex | Use |
|---|---|---|
| Amber bright | `#f59e0b` | Prompt / cursor (the user's anchor) |
| Off-white | `#d8d4c8` | User input |
| Blue | `#4a9eff` | Claude voice |
| Purple | `#b078e8` | Agents |
| Cyan | `#4ad4d4` | Hooks |
| Amber primary | `#D4890A` | Aegis |
| Terminal green | `#33CC33` | Success |
| Terminal red | `#CC3333` | Errors / warnings |
| Faint amber, italic | `#5a4410` | Meta / timestamps |

Tag prefixes are small bordered boxes containing short uppercase labels: `CLAUDE`, `AGENT`, `HOOK`, `AEGIS`, `OK`, `WARN`, `ERR`, `SYS`, etc. Border color matches the lane color.

### 10.2 Locked — Status line contents

Two rows, all values bold, all budget-style values as percentages, color-block backgrounds with dark text for maximum scan-ability.

- **Row 1:** `DIR` / `MODEL` / `CTX %` / `SESSION %` / `SKILL`
- **Row 2:** `GIT` / `REPO` / `SESSION USE %` / `WEEK %`

The `SKILL` segment is a deliberate addition — gives always-on confirmation that the expected skill (typically `aegis` when present) is actually loaded.

### 10.3 Locked — Visual mockup

`rift-v2-mockup.html` is the visual source of truth for terminal aesthetic, lane colors, tag style, tabs, status line, and active-state density. Future implementation work references the mockup, not memory of a conversation.

### 10.4 Locked — Notification tab pattern (Option C with elements of B)

All notification tabs follow a shared pattern: **persistent dashboard view of subsystem state, with a drill-down detail log inside it for digging into specific events.** Inline terminal output stays focused on the live work; tabs are where you go to inhabit a specific subsystem.

Each notification tab has the same internal anatomy:
1. **Status header** — current state at a glance (loaded / active / idle / alert)
2. **Live activity strip** — what's happening right now in this subsystem
3. **Recent events log** — scrollable, filterable, drill-down detail (the "B element" — full firehose available when you want it)
4. **Persistent state panel** — config, rules, agents-known, whatever's "always true" for that subsystem

This shared anatomy gives cognitive consistency across all tabs — you learn the layout once, it works everywhere.

### 10.5 Locked — Tab/pane/pop-out architecture

**Surface taxonomy:**
- **Tab** = persistent surface you might want to inhabit (sessions, notification tabs, etc.)
- **Pane** = split inside a tab when two views are needed simultaneously, OR a promoted tab living alongside the main terminal (see below)
- **Pop-out** = ephemeral surface for a specific action (rule editor, file viewer when clicking an Index entry, agent cancel confirmation). Appears when summoned, dismisses when done.

**Drag tab out to pane / drag pane back to tab.**
Any notification tab can be dragged out to become a side pane that lives alongside the main terminal — for better observation of what is going on while still being in the main terminal. Drag the pane back and it returns to being a tab when no longer needed.

**Constraint:** only one tab can be promoted to a pane at a time (in v1). Promoting a second tab while one is already promoted slides the first one back to being a tab. This keeps the terminal as the main focus of the whole — anything else and the cockpit becomes a sliver of terminal next to a wall of panes.

### 10.6 Locked — Per-tab independent toggle

Every notification tab is independently toggleable on/off. Defaults:
- **Desktop:** all available tabs default-on
- **Headless / mobile (future v2):** config-driven, defaults to terminal-plus-essentials only

This serves two purposes: lets the user customize their cockpit to taste, and makes the eventual mobile/SSH-headless surface practical without architectural refactoring.

### 10.7 Locked — Default tab set (capability-driven)

The default tab set flexes based on what's available at startup. Per the Integration Decoupling Principle (section 9), Rift queries the environment and renders accordingly.

**Bare install (no integrations loaded):**

- **Errors**
- **Hooks** *(generic terminal hooks, integration-agnostic)*
- **Commands**
- *One open slot*

This is the baseline "fresh install" experience. Already a useful product on its own.

**With integrations loaded, additional tabs appear:**

- **Aegis** *(when the Aegis integration module is present)*
- **Agents** *(when an agent system declaring agent events is present)*
- **Index** *(when the Abyssal Index integration module is present, and as a graph view in the GUI cockpit per section 11)*
- *Other tabs as future integrations declare them*

**Soft cap on tab count.** More tabs are optional, not gated — adding a fifth or sixth tab is a feature, not a workaround.

### 10.8 Locked — Sections per tab

Each notification tab supports **4 modular sections** (terminal-size constraint — 4 fits, more cramps).

- Sections are configurable: show, hide, resize, rearrange per tab.
- Sections are drawn from an extensible catalog (see 10.10).
- "What you see" follows what *that tab* is doing, not a fixed template.

### 10.9 Locked — Notification system

- **Amber notification badge** on each tab, counts up per event.
- **Triggering events** include: errors, failed commands, failed hooks, stuck agents, runaway agents, unauthorized file edits, misfires, "general trouble."
- **Amber border** animates around a tab when something is live/active inside it.
- **Badge persistence:** counter persists until acknowledged. Does *not* tick down when the triggering event scrolls out of the live view.

### 10.10 Locked — Section catalog model

Sections within tabs draw from an **extensible, self-discovering catalog**. New event types added by integrations register new section types automatically — Rift core does not need to know about every possible section type ahead of time.

**Initial categories named:**

- Hooks
- Aegis (full event surface, self-declared via the integration protocol)
- Errors
- Agents

**Future expansion (deferred — brainstorm pass needed):**

Placeholder list for the catalog brainstorm:
- Compile events
- Command activity
- MCP tool calls
- Token budget gauge
- Skill activation feed
- File context list
- Diff stream
- (Many more — Aegis alone brings a ton)

This is **not exhaustive**. A dedicated catalog brainstorm session is required to enumerate the full available section/event surface.

### 10.11 Locked — Agent misbehavior detection (Sentinel ↔ Rift split)

**Detection conditions.** The Agents tab watches for:
- **Stuck agent** — no tool call within configurable threshold
- **Runaway agent** — excessive activity beyond expected scope
- **Unauthorized file edits** — agent modifying files outside its permitted scope
- **General trouble** — extensible catch-all for misbehavior patterns

Threshold values (e.g., stuck-agent timeout) are configurable, potentially per-agent.

**Architectural split.**
- **Sentinel** is the **source of truth** for agent misbehavior detection. It detects and (where applicable) blocks.
- **Rift** is the **display layer**. It surfaces Sentinel's events through the Agents tab.
- This keeps the Integration Decoupling Principle (section 9) clean — Rift does not duplicate Sentinel's detection logic, and Sentinel speaks to Rift via the protocol like any other integration.

**Example agent roster (from Aegis when present, non-exhaustive):** builder, validator, watchdog, scout, synthesizer, and others. The Agents tab tracks whatever agents the loaded integration declares — Rift doesn't hardcode a roster.

### 10.12 Locked — Index as tab + graph (two views, same data)

When the Index integration is loaded, it gets its own notification tab AND lives in the GUI cockpit as the spatial graph view (with Index acting as a data-enrichment layer per section 11). These are two views of the same underlying data:

- **Tab view** = list/log/search interface — "what files have been touched, query the vault, see active scope"
- **Graph view** = spatial visualization with Index enrichment overlaid on filesystem activity — "see the system, watch nodes light up, drag-to-terminal"

Different cognitive modes for different tasks. Both useful, neither redundant.

### 10.13 Locked — Aegis integration (when present)

Aegis is one possible integration speaking the Rift Integration Protocol. When loaded, it lights up its own tab and contributes events to other tabs (Errors, Agents, Hooks).

Sub-decisions locked:
- Aegis hooks Rift subscribes to (when Aegis is present): session start, pre-edit, post-edit, pre-completion-claim, session-end framing.
- Overlap with Sentinel is reconciled by the Sentinel ↔ Rift split (10.11) — Sentinel is the detection source of truth, Aegis surfaces audit/lesson context, Rift displays both.
- Aegis tab persistent panel: loaded rule sources with paths/timestamps, active enforcement modes, lessons file summary, quick-action buttons.

### 10.14 Locked — Index integration (when present)

When the Index integration is loaded, it operates as a **data-enrichment layer** (capability class 3 in section 9): it attaches vault relationships and semantic tags to filesystem nodes the GUI is already rendering. The Index does not own the graph — the filesystem owns the graph; the Index annotates it.

Sub-decisions locked:
- Index tab persistent panel: project context scope, active vault sections, search bar, quick links to vault root.
- Inline emission rules: only meaningful events emit inline (queries, scope changes, out-of-scope reads). Routine reads stay quiet.
- Badge logic: no badge most of the time (Index is passive knowledge, not alerting); small indicator if a read is attempted outside loaded scope.

### 10.15 Open — Real-time update mechanism

How events actually flow from integrations to Rift core to the GUI. Likely involves websockets, IPC, or an event bus. Throughput, ordering guarantees, backpressure, and GUI subscription model all need decisions.

This is a technical/research question rather than pure design. **Best resolved against Claude Code's actual hook/event surface in a fresh session** rather than spec'd cold.

### 10.16 Deferred — Section catalog brainstorm pass

Full enumeration of trackable event types (compile, MCP, token budget, skill activation, diff stream, etc.). Held until a dedicated brainstorming session with focused energy.

### 10.17 Deferred — Agent tab grouping/filtering by agent type

With sizeable agent rosters, the Agents tab will likely need grouping or filtering by agent type to stay legible. Held until the catalog brainstorm pass — flagged here so it isn't lost.

### 10.18 Deferred to GUI planning session

Held until the GUI build phase begins:
- Rendering tech (Canvas / WebGL / native)
- Graph layout algorithm (force-directed / manual / hybrid)
- Node type taxonomy (refinements beyond "filesystem nodes + integration enrichments")
- Performance at vault scale
- State persistence (zoom, pan, opened files, layout)

## 11. GUI Cockpit — Vision (in scope for v1, phased build)

The GUI is not a separate product. It's the visual interface *to* the terminal and everything the terminal sees. **GUI must be fully integratable with the Rift terminal** — terminal and GUI are one experience with two surfaces.

**Build phasing:** terminal foundation first, GUI layer built on top. Both ship as v1. See section 1 for framing.

### Foundation — filesystem activity, not agent activity

The graph cockpit's foundation is **filesystem activity visualization** — reads, writes, creates, deletes, touches. This is **Rift core**, always-on, present in every install.

Index, Aegis, and any agent system are **data-enrichment and attribution layers attached to** the filesystem view, not replacements for it. Bare Rift renders filesystem activity by itself; integrations make it richer.

### Window architecture — detachable, default attached

GUI and terminal are **two surfaces of one experience**, with flexibility in how they're laid out.

- **Default: attached.** Single window, terminal and GUI side-by-side (or in another integrated layout). This is the cockpit experience for new users and single-screen setups.
- **Detachable.** User can pop the GUI out into its own window for multi-monitor workflows — graph on a second display while terminal lives on the primary. Drag back to re-attach.
- **Same conceptual pattern as tab → pane promotion** (section 10.5), one level up. Users already understand the gesture.

Architecturally trivial under Tauri (multi-window is well-trodden). State and event subscription remain shared across surfaces — detached doesn't mean disconnected.

This also answers the headless-mobile question elegantly (post-v1, see section 13): the SSH client is just another "detached" surface in the same conceptual model.

### Graph model

- **Mirrors the filesystem**, IDE-tree style. Familiar mental model.
- **Each file is a node** with custom imagery / icon based on file type.
- **Standard expand/collapse** on directories.
- **Drag-node-into-terminal** to load context into the terminal — friction-free context injection. UX moment that needs to feel *good*.
- **Easy project swap via menu** that points to a project directory — first-class action, not a config file edit.
- **Pan and zoom** — smooth navigation, easy zoom in/out, standard map-style interaction.

### Activity visualization model — decay, pin, background

Activity has three natural states. This applies the dial principle architecturally — every visual element has a "volume" that the user can either let the system manage (decay) or override (pin).

- **Ambient** — recent activity decays on its own; user glances, absorbs, moves on. Decay *is* the visual: the same glow that signals write/read/edit slowly fades over time. Activity and decay are one concept, not two.
- **Pinned** — clicked node stays lit; user's attention has parked there, system respects that until released.
- **Background** — quiet files/directories fade to neutral; don't compete for visual bandwidth.

**Click behavior (Option C — pin + dismiss with modifier).**

- **Click** = pin. Node locks lit, ignoring decay, until released.
- **Click again** (or shift-click) = release/dismiss. Visual goes to off immediately, no decay curve.

This gives both behaviors — parking attention on something you want to watch, *and* explicit acknowledge-and-dismiss — without forcing the user to choose one model. The interaction nuance needs to be discoverable in the UI (tooltip, status hint, or similar).

### Hierarchical bubble-up

Activity propagates up the tree:

- A directory **indicates activity inside it** when files within are being touched.
- **Expanding the directory** reveals exactly which files are active.
- Collapse behavior: parent shows aggregate activity state of its children.
- Result: ambient awareness at the collapsed level + full detail on demand, without forcing the user to choose globally between "see everything" and "see only what's active."

### Agent attribution (integration-provided)

The OS reports "file X was written," not "agent Y wrote file X." In a Claude Code session, all writes come from a single process. So a stock Rift install can show **what** is happening, but not **who** is doing it.

This is where agent integrations earn their keep. Aegis (or any agent system speaking the protocol) tags activity with its source agent. With attribution, the same activity in the graph is **colored by agent** rather than appearing anonymous.

### Layered value model

This produces a clean stack of value tiers, each one usefully complete:

- **Bare** *(no integrations)* — anonymous filesystem activity. Already more useful than any standard terminal.
- **+ Attribution** *(an agent integration loaded)* — same activity, now colored by which agent did what.
- **+ Enrichment** *(a data layer like Index loaded)* — nodes carry semantic metadata; the graph becomes a knowledge surface, not just an activity surface.
- **Full cockpit** *(attribution + enrichment + control endpoints)* — the experience Aegis + Index + Rift gives Garth personally.

This answers the implicit "why use Rift without Aegis" question: **because the bare filesystem cockpit view is already a meaningful product.**

### Built-in editor — explicit scope bound

Click-to-edit is supported with **full syntax highlighting** (tree-sitter or equivalent). Nodes are clickable; the file opens in a custom in-cockpit viewer with proper syntax per document type (markdown renders as markdown, code with proper highlighting, etc. — no "opens in external program" cop-out).

**This is not a primary editing environment.** The editor exists for one reason: **friction reduction.** When you spot something off in the graph, you can fix it without leaving Rift and breaking flow.

**In scope:**
- Syntax highlighting across full language coverage
- Quick edit → save → back to flow

**Out of scope:**
- Multi-file refactoring
- Debug tooling
- Extension ecosystem
- Anything that would compete with a real editor

If a user needs a real editor, they go to their real editor. **This boundary needs to hold during build to prevent scope creep.**

### Mockup plan (3 mockups total)

The visual design lands across three mockups, each exercising a different surface configuration:

1. **Terminal alone** — ✅ done. `rift-v2-mockup.html`. Visual source of truth for terminal aesthetic, lane colors, tag style, tabs, status line, active-state density.
2. **GUI alone (detached state)** — ⏳ pending. What the GUI looks like when popped out into its own window. Filesystem + Index enrichment value tier (the rich, full-detail view). Full graph, file tree, decay/pin/background activity states, glow-on-touch behavior, hierarchical bubble-up, custom file viewer pane, project swap menu.
3. **Terminal + GUI integrated (attached cockpit)** — ⏳ pending. The default attached cockpit experience — terminal and GUI side-by-side as one window. Shows how the two surfaces share screen real estate and work together.

The terminal mockup's visual vocabulary (palette, fonts, tags, scanlines, vignette) carries forward to the GUI mockups. Net-new visual work in the GUI mockups: node rendering, edge rendering, expand/collapse affordances, pan/zoom, viewer pane, project swap menu, decay/pin/background visual treatments.

### Why this matters

This is the feature that makes Rift a **cockpit**, not a terminal. Every other terminal shows you a command line. Rift shows you the command line *plus a live visualization of the system you're working on*. The agent visibility in the terminal (section 3) plus the graph visualization in the GUI turns "working with Claude" from "reading scrollback" into "watching the work happen spatially."

### GUI-specific open questions (deferred to GUI planning session)

Held until the GUI build phase begins — see section 10.18:
- Rendering tech (Canvas / WebGL / native)
- Graph layout algorithm (refinements beyond IDE-tree mirror — e.g. when free-graph view is needed)
- Performance at vault scale
- State persistence (zoom level, pan position, opened files, layout preferences across session restart)

### Scope discipline

GUI is **in scope for v1**. Internal build is phased (terminal foundation → GUI layer on top), but both ship together as the complete v1 product. Architecture decisions must actively support both from day one:

- Stack choice must support the GUI natively (Tauri is now effectively required — see section 5)
- Integration protocol must emit events the GUI can subscribe to
- File operations must be observable, not just happen
- The plugin/integration interface (section 9) is the primary extensibility surface
- Terminal and GUI share state; they are not independent apps communicating over a wire

## 12. Mid-Thought Road Trip Note

Additional items captured mid-rant; keep adding as they return:

- _(placeholder — add as remembered)_

## 13. Post-v1 / True v2 Bucket

Ideas that are genuinely *later* — not "deferred from v1" but "belongs to the next chapter." Kept here so they don't pollute v1 scope but don't get lost either.

- **Android phone + tablet SSH headless access to terminal.** Companion mobile client that connects to a running Rift instance over SSH. Headless — phone/tablet is a remote surface, not a standalone Rift. Lets you check on long-running agents, cancel runaway work, or glance at status from the couch/bed/outside. Fits naturally on top of the existing Rift architecture (terminal + hooks + agent visibility are already there; mobile just becomes another surface subscribing to them).

*Add more as they surface. Anything that starts with "oh and also" goes here, not in v1 sections.*

---

## Document History

- **v0.1 (2026-04-23)** — Vision captured during outdoor phone session while frustration with v1 was fresh. Will expand with planning pass.
- **v0.2 (2026-04-24)** — Planning pass. Locked: stack (Tauri), color-coding system, status line contents, mockup as visual source of truth, notification tab pattern (Option C with B elements), tab/pane/pop-out architecture with drag-out promotion, per-tab toggle, Index as both tab and graph view. Added Integration Decoupling Principle as load-bearing architectural section. Open: refined Aegis/Index integration sub-decisions, real-time update mechanism. Deferred to GUI planning session: rendering tech, graph layout, node taxonomy, performance, state persistence, editor depth.
- **v0.3 (2026-04-26)** — Tab anatomy & notification system locked: 4 modular sections per tab, amber notification badge with persistent counter, animated amber border for live activity, default tab set initially defined as Hooks/Aegis/Errors/Agents (later refined in v0.4). Section catalog model defined as extensible and self-discovering. Agent misbehavior detection split between Sentinel (source of truth) and Rift (display layer). Open carried forward: Aegis integration sub-decisions, Index integration sub-decisions, real-time update mechanism, section catalog brainstorm pass.
- **v0.4 (2026-04-26)** — Major integration architecture update: Rift core decoupled from any specific integration; Aegis remains private and optional. Two-document architecture defined (public Rift Integration Protocol + private Aegis ↔ Rift module). Three capability classes named (event subscription, control endpoints, data enrichment). Feature detection at runtime locked. Graph cockpit core locked: filesystem-as-foundation, IDE-tree mirror, file-as-node with type icons, decay-unless-clicked activity model with ambient/pinned/background states, hierarchical bubble-up, friction-reduction editor with explicit scope bound. Agent attribution defined as integration-provided value layer. Layered value model articulated (bare → attribution → enrichment → full cockpit). Default tab set without integrations refined to Errors/Hooks/Commands/open slot, with capability-driven additions when integrations load. Aegis and Index integration sub-decisions substantively locked. Numbering bug fixed (subsections under section 10 were labeled 9.x). Remaining open: real-time update mechanism. Deferred: section catalog brainstorm pass, agent tab grouping/filtering.
- **v0.5 (2026-04-26)** — Window architecture locked: detachable, default attached. GUI and terminal are two surfaces of one experience; default single-window attached layout, with multi-monitor users able to pop GUI into its own window and drag back to re-attach. Click behavior locked as Option C (click pins, click again or shift-click releases) — gives both pin-attention and dismiss behaviors without forcing a choice. Decay model clarified: decay *is* the visual, same glow as write/read/edit, fading over time — one concept, not two. Mockup plan locked at 3 total: terminal alone (done), GUI alone (pending), terminal + GUI integrated (pending).
