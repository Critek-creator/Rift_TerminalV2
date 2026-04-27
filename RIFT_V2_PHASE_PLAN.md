# Rift V2 — Phased Implementation Plan

*Generated: 2026-04-26 by `/aegis` (PLAN mode, TIER-SOLO)*
*Source-of-truth refs: `RIFT_V2_VISION.md` v0.5 (locked) + `rift-v2-mockup.html` + p006 v1 lessons*
*Status: DRAFT — pick a starting move; revise after §10.15 resolves*

---

## Anchor

- **Vision is locked** at v0.5 (2026-04-26). One open spec question remains: **§10.15 real-time update mechanism.**
- **Stack is locked** (§5): **Tauri 2 + Svelte 5 + xterm.js** (Rust backend, web frontend).
- **V1 = `C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/Rift_Terminal`** — shipped Phase 7 with MSI on 2026-04-24. Per CLAUDE.md it is a "cautionary museum exhibit" — *concepts* transfer (hook system, observability layer, IPC framing/replay), *code* does not. V1's wgpu+glyphon native stack is the wrong shape for V2's GUI requirement.
- **No source code, no build system, no tests** in this repo yet. Working tree currently holds: `RIFT_V2_VISION.md`, `rift-v2-mockup.html`, `CLAUDE.md`, this file.

---

## Build Sequencing — 9 phases

| Phase | Name                              | Gate(s)                       | Locked spec refs            |
|-------|-----------------------------------|-------------------------------|-----------------------------|
| 0     | Repo + Tauri scaffold             | none                          | §5, §8                      |
| 1     | Terminal foundation (PTY+xterm)   | none                          | §3, §5                      |
| 2     | Visual system (lanes/tags/CRT)    | none                          | §10.1, §10.2, §10.3         |
| 3     | Tab/Pane/Pop-out architecture     | none                          | §10.4–§10.10                |
| 4     | Integration Decoupling Protocol   | **§10.15 must close here**    | §9, §10.13–§10.14           |
| 5     | First integration: hooks tab      | depends on Phase 4            | §10.7, §10.8                |
| 6     | GUI Cockpit foundation (tree)     | mockups #2+#3 ✅              | §11                         |
| 7     | Aegis private translator module   | depends on Phase 4 + Phase 5  | §9 two-doc, §10.13          |
| 8     | Index integration (tab + graph)   | depends on Phase 6; **§10.18 graph-lib decision** | §10.12, §10.14, §10.18      |
| 9     | v1 ship: MSI + signing + runbook  | all above PASS                | §13 packaging               |

### Phase 0 — Repo + Tauri scaffold

**Out:** working `cargo tauri dev` cycle, empty webview, Aegis BV wired.
- `npm create tauri-app@latest` (template: Svelte 5 + TS).
- Workspace shape: `src-tauri/` (Rust), `src/` (Svelte), `static/`, `tauri.conf.json`.
- Pin Rust toolchain (`rust-toolchain.toml`) + Node version.
- Wire `/aegis --bv` and the Completeness PostToolUse hook for build discipline (§8).
- Write `Cargo.toml` workspace root with `rift-core` member placeholder.

### Phase 1 — Terminal foundation

**Out:** terminal that runs cmd/powershell/bash, keyboard works, resize works.
- Rust: `portable-pty 0.9` (transferrable from V1; **apply pty-exit-windows lesson** — ConPTY exit-watcher OS thread + AtomicBool alive flag).
- Tauri commands: `pty_start`, `pty_write`, `pty_resize`, `pty_kill`.
- Tauri events: `pty_output(bytes)`, `pty_exited(code)`.
- Frontend: xterm.js mounted in a Svelte component; `invoke`/`listen` bridge.
- Acceptance: type a command, see output, resize window, exit cleanly on Windows.

### Phase 2 — Visual system

**Out:** mockup parity (compare side-by-side with `rift-v2-mockup.html`).
- Lane classifier (Rust): structured-output annotation (which lane each line belongs to). For raw PTY without source attribution, default off-white (user input echo) / amber (prompt) per lane table §10.1.
- Tag prefix component (Svelte): bordered uppercase boxes (`CLAUDE`, `AGENT`, `HOOK`, `AEGIS`, `OK`, `WARN`, `ERR`, `SYS`).
- Status line component: 2 rows, color-block backgrounds, **SKILL segment is non-negotiable** (§10.2).
- CRT aesthetic: CSS scanlines + vignette overlays.
- JetBrains Mono via fontsource or local font file.
- Acceptance: open repo, alt-tab to mockup HTML, verify pixel-level visual match.

### Phase 3 — Tab/Pane/Pop-out architecture

**Out:** tab strip, drag-to-promote, pop-out modal, default tab set.
- `Tab` Svelte component (persistent surfaces).
- `Pane` (split inside tab OR promoted notification tab — only **one** promoted at a time per §10.5).
- `Popout` (ephemeral, e.g. rule editor).
- Drag-tab-out → promotes to pane. Drag-pane-back → returns to tab. Same gesture handles GUI window detach in Phase 6.
- Default tab set per §10.7: **Errors / Hooks / Commands / one open slot.**
- Tab anatomy (§10.4 / §10.8): 4 modular sections per tab — status header / live activity / recent events / persistent state.
- Section catalog (§10.10) — extensible, self-discovering. v1 ships hardcoded; integration registration deferred to Phase 4+.
- Per-tab independent toggle (§10.6).

### Phase 4 — Integration Decoupling Protocol

**🚧 GATE: §10.15 real-time update mechanism MUST resolve before this phase.** Recommend `/aegis --research` dispatch with the spec, the V1 IPC pattern (UDS + named pipe + framing + replay), and Tauri's IPC primitives as inputs.

**Out:** Rift Integration Protocol v1 spec doc + reference implementation in Rust + a green test that a fake translator module can subscribe to events and invoke a control endpoint.
- Define internal event/state JSON schema (Rust types + JSON-schema export).
- Three capability classes (§9): **event subscription / control endpoints / data enrichment.**
- `translators/` directory — every external system enters through a module here. **No direct Claude/Aegis/MCP calls outside translators** (§9 build-time enforcement).
- Feature detection at runtime: bare Rift renders anonymous activity; integration presence lights up enrichment.
- **Critical**: keep this protocol public-facing. Aegis's translator goes in Phase 7 as a private module, not here.

### Phase 5 — Hooks tab (first integration)

**Out:** real Claude Code hook events flowing into the Hooks tab.
- A built-in `hook_translator` module that subscribes to Claude Code's hook event surface and emits Rift internal events.
- Hooks tab renders cyan-lane events with `HOOK` tag prefix.
- Section catalog populated for hooks: live activity strip + recent log + state panel.
- Acceptance: trigger a Claude Code hook in the embedded session, see it land in the Hooks tab.

### Phase 6 — GUI Cockpit foundation

**Prerequisites:** Mockup #2 (GUI alone) ✅ and Mockup #3 (terminal+GUI integrated) ✅ — both shipped 2026-04-27 (rework: graph = Abyssal Index, tree = node-based filesystem). **§10.18 graph-lib decision moved to Phase 8** (graph = Index surface; Phase 6's tree is hierarchical-SVG, not free-form). No graph-lib spike required for Phase 6.

**Out:** node-based filesystem tree with live activity rendering + detachable cockpit window + scope-bound in-cockpit viewer.
- Filesystem watcher (Rust, `notify` crate, behind §9 translator boundary at `crates/rift-bus/src/translators/fs.rs`) → emits `Category::Fs` envelopes (read/write/create/delete/rename) with ignore-globs default `.git/** node_modules/** target/** dist/** *.log`.
- Tree model: file-as-node, type icons, hierarchical filesystem mirror (per 2026-04-27 mockup rework — circles for files, soft-square dirs, L-shaped edges; same glow vocabulary as graph).
- Activity visualization: glow-on-touch, decay, pin (click), background (click-again/shift-click) — frontend Svelte 5 rune store, decay loop via rAF; configurable `decay_ms` in `rift-config.toml`.
- Hierarchical bubble-up (§11): collapsed dir aggregates max child glow + pinned-presence indicator; expanded dir hides aggregate, shows children individually.
- Detachable cockpit window: `WebviewWindowBuilder` per r004; `cockpit_detach`/`cockpit_reattach` Tauri commands; drag handle on cockpit divider; bus subscription per-window-label per §10.15.
- Drag-node-into-terminal (reuses Phase 3.5a drag infra).
- Project swap menu (reuses popouts.svelte.ts from Phase 3.5b).
- Friction-reduction in-cockpit viewer: **scope-bounded per §11** — Shiki WASM for syntax highlighting in v1 (TextMate grammars; tree-sitter migration deferred to v1.1 — spec wording "tree-sitter or equivalent" covers Shiki). Quick edit/save. **OUT OF SCOPE: multi-file refactor, debug tooling, extensions.**
- Subphases (locked 2026-04-27 via `/aegis --plan phase 6`): 6.0 spec patch (this commit) → 6.1 fs translator + Category::Fs → 6.2 tree renderer + activity store → 6.3 hierarchical bubble-up → tranche-1 ship; then re-plan 6.4 detachable window → 6.5 viewer (Shiki) → 6.6 drag-into-terminal → 6.7 project swap.

### Phase 7 — Aegis private translator

**Out:** the Aegis ↔ Rift module (**private optional feature-gated path dep (NOT a workspace member — workspace members fail public build when the path is gitignored)**, NOT public). Architecture LOCKED 2026-04-27 via `/aegis --plan phase 7`.

- Lives outside `translators/` public set as `crates/rift-aegis/` (private optional feature-gated path dep excluded from public CI + `.gitignore` on public branches; `tools/check-translator-boundary.sh` extended to verify the path is gitignored on public-branch pushes).
- Loads conditionally at runtime: Aegis-presence probe at startup (a) checks `~/.claude/skills/aegis/SKILL.md` existence and (b) `linkme`/`inventory` self-registration if compiled in → emits `aegis.detected` envelope. Probe runs on a separate tokio task to avoid blocking Tauri `setup()`.
- Aegis tab populated from three sources: (c1) startup snapshot — parse `~/.claude/skills/aegis/SKILL.md` HTML-comment version + scan `~/.claude/settings.json` hooks → `aegis.context` envelope; (c2) live tail — `notify`-watched `~/.claude/aegis.log` → `aegis.invocation` envelope per appended line; (c3) lazy load — `~/.claude/anti-claude-lessons.md` read on tab focus only (too large for snapshot).
- SKILL segment in status line: `~/.claude/scripts/aegis-log.mjs` UserPromptSubmit hook extended to spool per-session `.aegis/session/<project>/skill.json`; rift-aegis tails the spool → emits `aegis.session.skill_loaded` envelope. Status line also gains live ctx % / session % / week % via the same envelope (closes the Phase 2 acceptance gap noted at `src/lib/StatusLine.svelte:6`).
- Sentinel: **NOT IMPLEMENTED in v1** — no Sentinel crate, no source file, no Aegis-side spec yet. Agents tab renders capability-driven empty-state card "Sentinel: integration not loaded" per §10.7 pattern. Sentinel implementation deferred to post-v1 as `D-010` in `DEFERRED.md`.
- §10.17 (agent tab grouping/filtering) → standalone `/aegis --think` brainstorm beat at end of phase (subphase 7.6); doc-only output.
- Subphases (locked 2026-04-27 via `/aegis --plan phase 7`): 7.0 spec patch (this commit) → 7.1 rift-aegis private optional path dep + load detection → tranche-3 fan-out: 7.2 Aegis tab + AegisTabContent → 7.3 quick-action buttons → 7.4 live SKILL status line → 7.5 Sentinel placeholder card. **Tranche-3 ships at end of 7.5.** Then 7.6 §10.17 brainstorm beat (separate, no BV).

### Phase 8 — Index integration

**Out:** Index tab + Index graph view (two views of same data, §10.12). **§10.18 graph-library decision (Cytoscape / D3 / Sigma) must close before code starts** — render same fixture in all 3 libs, decide on perf + interaction quality.
- Translator module subscribes to Index update events.
- Tab view = list/tree. Graph view = node-edge free-form layout (the Abyssal Index vault network, per 2026-04-27 mockup #3 rework). Pan/zoom required.

### Phase 9 — v1 ship

**Out:** signed MSI installer + GitHub release + runbook.
- Apply V1 lessons: `cargo-wix` invoked from package dir (not workspace root), workspace-relative paths in `.wxs`, conditional code-signing in CI.
- Release runbook documents the full Phase 0→9 build verification flow.

---

## Anti-Patterns to actively guard against

Per CLAUDE.md §7 + RIFT_V2_VISION §7:
- **No wrapper architecture.** This is V1's original sin. V2 is standalone.
- **No silent stubbing or deferring.** If something is deferred, log it loudly in `DEFERRED.md`, never bury in `// for now`.
- **No floating text.** Every UI element belongs to a tab/pane/pop-out.
- **No shortcuts on lane classification or section catalog** — these are load-bearing visuals.
- **No direct external-system calls outside translator modules** (§9 build-time enforcement).

---

## Open spec items — must-resolve list

| Ref     | Item                                          | Action                            | Gates phase |
|---------|-----------------------------------------------|-----------------------------------|-------------|
| §10.15  | Real-time update mechanism                    | `/aegis --research`               | Phase 4     |
| §10.16  | Section catalog brainstorm                    | `/aegis --think` during Phase 5   | Phase 5     |
| §10.17  | Agent tab grouping/filtering                  | RESOLVED 2026-04-27 — see `decisions/§10.17_agent_tab_grouping_filtering.md` | Phase 7 ✓   |
| §10.18  | GUI rendering tech (Cytoscape/D3/Sigma)       | spike + `/aegis --crit`           | Phase 8     |

---

## Lessons that transfer from V1 (p006 vault)

- `pty-exit-windows` — ConPTY exit-watcher 250ms poll + Arc<AtomicBool> alive flag. Apply Phase 1.
- `pre-publish-before-start-ipc-server` — eliminate subscribe-vs-publish race. Apply Phase 4.
- `serialize-deserialize-asymmetry-bidirectional-protocol` — round-trip tests mandatory for any new envelope. Apply Phase 4.
- `cargo-wix-workspace-member-light-path-resolution` — invoke from package dir or use workspace-relative `.wxs` paths. Apply Phase 9.
- `envelope-version-additive-categories-no-bump` — adding new event categories is additive; only schema breaks bump version. Apply Phase 4.
- `coordinator-surgical-recovery` — mid-edit interrupt protocol. Apply throughout build.

## Lessons that DO NOT transfer

- V1's wgpu+glyphon native renderer stack — Tauri webview replaces it.
- V1's tmux + capture-pane testing surface — `/aegis --verify` Tauri profile would be needed (not yet built; consider creation under the 5-gate protocol when Phase 2 lands).

---

## Recommended starting move

**Phase 0 + close §10.15 in parallel.** Phase 0 has no spec dependencies; §10.15 research can run independently and will be done by the time Phase 0 + Phase 1 + Phase 2 wrap.

Concrete next-message options:
- `[A]` Start Phase 0: scaffold the Tauri+Svelte project with `npm create tauri-app@latest` and wire workspace.
- `[B]` Resolve §10.15 first: `/aegis --research real-time update mechanism Tauri 2 IPC vs websocket vs event bus`.
- `[C]` Build Mockup #2 (GUI alone) — closes Phase 6 prerequisite ahead of time.
- `[D]` Revise this plan before any code (changes you want?).
