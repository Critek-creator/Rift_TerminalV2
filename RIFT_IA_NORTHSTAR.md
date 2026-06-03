# Rift Information Architecture — North-Star + Consolidation Plan

*Status: PROPOSED · drafted 2026-06-03 · supersedes the ad-hoc feature-accretion model*
*Inputs: 7-agent research+audit sweep (3 external IA research, 4 internal audits). This document is the synthesis.*

---

## 0. The diagnosis (one line)

**Rift's internals are sound; its access layer rotted.** Every internal audit independently found the same shape: the *capability* shipped, but the way to **reach** it, **join** it, or **surface** it did not. This is not a rewrite — it's a consolidation of access, navigation, and data-joining on top of a solid core.

Evidence of the pattern:
- **Cockpit:** the tree shipped its full activity-physics vision; the graph's *data model* is intact — only the renderer was swapped for a list (D-019). The capability is there; the surface isn't.
- **Settings:** the save path is architecturally correct (atomic TOML + cache + bus broadcast); ~15 settings simply have no UI and there's no search.
- **Navigation:** a command palette *and* a slash launcher both exist; both are invisible and overlapping.
- **Data:** 20 stores hold real data; nothing shares a correlation key, so nothing joins.

---

## 1. North-Star principles (convergent across all research)

When Warp, Wave, Windows Terminal, zellij, and VS Code independently agree, it's a spine, not a preference.

### N1 — One universal entry point (command palette as primary nav)
*Named the #1 fix by all three research streams.* Name-first navigation means a user never has to know **where** a feature lives or memorize a keybind — they type its name. Every action, setting, surface, and session is reachable from one searchable box. This is the antidote to "keybinds for important menus" **and** "messy settings" simultaneously.
**Rift reality:** already 80% built — `Ctrl+K` palette + `Ctrl+Shift+P` slash launcher exist but are invisible, overlapping, and not a canonical registry.

### N2 — Fixed named zones (the layout IS the IA)
Every category of content gets exactly **one** home, enforced architecturally (VS Code workbench: activity bar / sidebar / panel / editor / status bar — Wave: every surface is a tile). A new feature must *claim a zone*; it cannot float as an orphan. This is the structural cure for "data everywhere."
**Rift reality:** 36 surfaces with ad-hoc, undeclared zone relationships.

### N3 — Atomic addressable units (blocks, not scroll)
Warp's blocks: every command+output is a named, selectable, copyable, bookmarkable unit, with a sticky command header so you always know what you're looking at. Output stops being an undifferentiated scroll and becomes navigable objects.
**Rift reality:** raw xterm scrollback; per-command badges exist but carry no identity/context.

### N4 — Ambient status-bar chrome (persistent, not a destination)
zellij's mode-gated hint line + intelligent-terminal's persistent agent/error status bar. State you need to *monitor* (current mode's keys, agent activity, error flags) lives in always-visible chrome — never a tab you navigate to. Discoverability becomes ambient.
**Rift reality:** has a StatusLine (model/ctx/git) but no mode-hint strip and no agent/error ambient surface; agent + error state are buried in cockpit tabs.

### N5 — A graphical surface earns its place by answering a question a list can't
A real cockpit encodes relationship density / flow / rate-of-change that a list destroys, and is interactive at the object level. The force-graph failed because it rendered *list data as a graph* (decoration). The fix is a **data question** ("which vaults are densely linked to what I'm working on *right now*"), then the right encoding, **reactive to live terminal activity** — a system-state mirror.
**Rift reality:** graph downgraded to list; data model + locked D3 blueprint (§10.18) already exist.

### N6 — One data layer with a shared correlation key
Stores must compose. A single key (session_id / pane_id) threaded through every store lets derived views join across them ("what happened in session X — commands + errors + cost + agent activity"). In-memory live views must be persisted and queryable, not evaporate on reload.
**Rift reality:** 20 stores, 3 non-composing tiers, **no shared key**; `CommandRecord` has no session_id; ledger/enrichment/clustering are in-memory-only and silently lose data.

### N7 — Settings: searchable + scoped + complete + palette-reachable
Search-first (type intent, not category), scopes (global/project), UI↔file duality with the file as source-of-truth, and **every** setting surfaced. Every setting is also a palette entry (auto-registration).
**Rift reality:** sound save path, but no search, ~15 hidden settings, a ghost INDEX tab, a 2,730-line monolith.

---

## 2. Current-state gap (per principle)

| Principle | Shipped & sound | Debt to close |
|---|---|---|
| N1 palette | `Ctrl+K` + `Ctrl+Shift+P` both work | invisible (no button), TWO overlapping surfaces, not a canonical auto-registry, no-op "Notification Manager" entry |
| N2 zones | tabs/panes/cockpit/popouts all render | no declared zone model; 36 surfaces, orphan tabs (`integrations`, `feature-pipeline` undocumented), hover-only controls |
| N3 blocks | per-command exit/duration badge | no command identity, no block model, no sticky header, no copy/bookmark on output |
| N4 status chrome | StatusLine (model/ctx/git/session) | no mode-hint strip, no agent/error ambient surface, keybinds undiscoverable |
| N5 cockpit graph | tree (full physics), graph data model, §10.18 D3 blueprint | graph renders as list; cockpit buried in narrow pane; not first-open |
| N6 data layer | session `.jsonl`, command_history, config, index — clean APIs each | no shared key; can't join; ledger/enrichment/clustering in-memory-only & lossy |
| N7 settings | atomic save path + bus propagation | no search; ~15 settings UI-less; ghost INDEX tab; notifications orphaned in GENERAL |

---

## 3. Consolidation plan (sequenced by leverage ÷ effort)

### Phase 0 — Truth & cleanup *(cheap, immediate trust repair)*
Fix the lies before adding anything. All small:
- Wire `Ctrl+T` (advertised in empty-state, does nothing — `App.svelte:1072` vs missing `onKeyDown` handler). **The first thing every new user tries.**
- Fix the no-op "Notification Manager" palette action (`CommandPalette.svelte` — calls `onclose()`, never summons).
- Reconcile `keybindings.ts` with reality: add the 5 undocumented real shortcuts (`Ctrl+Shift+L/M/E/D/W`), wire-or-remove the 2 ghosts (`Ctrl+D`, `Ctrl+Shift+N`), verify `/` Index-search.
- Delete the ghost `INDEX` settings tab (D-019 leftover).
- Set `aegis` tab `detectedByDefault: true` (or add an inactive placeholder).

### Phase 1 — Unify & surface the command palette *(N1 — highest leverage, ~80% built)*
- **Merge** the `Ctrl+K` palette and `Ctrl+Shift+P` slash launcher into ONE canonical command surface (keep one keybind, alias the other).
- **Auto-registration:** every action, setting, surface, and session becomes a palette entry from its definition (WT's "definition IS registration"). Settings become searchable *through the palette* — closes N7's discoverability hole without a settings rewrite.
- **Make it visible:** a `⌘K` chip / `/ CMD` button in the TitleBar + a `?` help affordance opening the shortcut overlay.
- Outcome: "keybinds for important menus" and "can't find settings" both dissolve here.

### Phase 2 — Declare zones + ambient status chrome *(N2 + N4)*
- Define the fixed zone model (terminal / cockpit / notification-panel / status bar) and make every surface claim exactly one. Document orphan tabs or fold them.
- Add the persistent **status-bar strip**: zellij-style mode-hint line (shows current-context keys inline) + an agent/error ambient indicator (lights on non-zero exit / active agent). This is also the **home for the error-handoff feature** (see §5).

### Phase 3 — Cockpit redemption *(N5)*
- Restore the D3 force graph (data model + `§10.18` blueprint already present) behind a `graph|list|content` view toggle (list survives as fallback).
- Give the graph a real question: highlight vaults semantically near the active project; make nodes react to live bus activity.
- Rebalance the cockpit layout so the visual surfaces are the hero; make the cockpit a first-open view.

### Phase 4 — Unified data layer *(N6 — deepest architectural value)*
- Thread a correlation key (`session_id`/`pane_id`) through `command_history` and every store.
- Promote in-memory singletons (LLM ledger, enrichment, error-clustering) to **persisted, queryable** stores; stop losing data on reload/unmount/replay-overflow.
- Add a cross-store query surface ("session X: commands + errors + cost + agents") — the join that's currently impossible.

### Phase 5 — Re-slot the Error → Agent Handoff *(the original feature, now on a real foundation)*
See `ERROR_TO_AGENT_HANDOFF_SPEC.md`. With Phases 0–4 done, **its blockers dissolve** (§5 below). It becomes ambient error chrome (N4) + a context payload from the correlated data layer (N6) + a local-only explain provider — not a badge-and-registry hack.

---

## 4. What is explicitly NOT changing (preserve the sound core)

- The `config_save` atomic-write + `CachedConfig` + `config.changed` bus path. Correct.
- The §9 translator boundary + CI guard. Load-bearing; all new external calls stay behind it.
- The tokio broadcast bus + envelope versioning (additive-fields rule). Correct.
- `Tree.svelte`'s activity physics (glow/decay/pin/bubble-up/drag). Already delivers the vision.
- The two-tier IPC (Channel<T> + bus). Locked, proven.

---

## 5. Why IA-first dissolves the error-handoff blockers

The 4-agent red-team of the original error-handoff plan found blockers that are **all symptoms of the missing IA** — and the foundation work removes them:

| Error-handoff blocker (red-team) | Dissolved by |
|---|---|
| `command.completed` carries no command text/context; `CommandRecord` has no session_id to join | **N6 / Phase 4** — correlation key + composed stores make the context a query, not a frontend cache hack |
| `action_id`-keyed registry state collides across panes/failures | **N4 / Phase 2** — ambient status-bar chrome owns error state; no per-failure registry-id juggling |
| No badge/pop-out result surface exists | **N2/N4** — the status-bar + zone model give it a declared home |
| Affordance keybind-gated, undiscoverable | **N1** — reachable by name in the palette |
| Fallback can silently escalate to cloud (privacy leak) | unchanged design decision: local-only-or-degrade (locked 2026-06-03) |

This is the case for the reset: the feature wasn't hard because error-handling is hard — it was hard because it had no foundation to stand on. Build the foundation, and the feature (and every future one) gets cheaper.

---

## 6. Open sequencing decision

Phases 0–1 are cheap, high-leverage, and low-risk — the recommended opening move (trust repair + the palette that fixes navigation AND settings discoverability at once). Phases 3 (cockpit) and 4 (data layer) are larger and independent; either can follow. Phase 5 (error-handoff) should come last, on the finished foundation.

*Synthesis of: 3 research reports (Warp/Wave, WT/zellij/intelligent-terminal, VS Code/desktop IA) + 4 internal audits (surface/nav, settings, data-stores, cockpit-gap), 2026-06-03. Re-verify symbols before implementing each phase.*
