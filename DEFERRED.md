# DEFERRED — Rift V2

*Loud-log of deferrals required by `RIFT_V2_VISION.md` §7 ("No silent stubbing or deferring").*
*Every deferral here names the concrete unblocking event per `pr001` TODO SHARPNESS rule.*

---

## Active deferrals

<!-- D-002 closed 2026-04-26, see C-007 below -->

<!-- D-005 fully closed 2026-04-27: drag-promote half (C-010) + pop-out chassis (C-011) -->

<!-- D-006 closed 2026-04-26 in three commits, see C-009 below -->

<!-- D-008 closed 2026-04-27, see C-012 below -->

<!-- D-007 closed 2026-04-27, see C-013 below -->

<!-- D-011 closed 2026-04-27, see C-014 below -->

### D-012 — StatusLine live-value plumbing — PARTIALLY CLOSED 2026-04-28 (DIR/GIT/REPO live; CTX/SESSION/WEEK/MODEL still blocked)

**Closed sub-tranche (2026-04-28):** DIR / GIT / REPO segments are now live via `crates/rift-bus/src/translators/status.rs`. The translator polls every 5 s, publishes `Category::Status / kind="usage"` with `{ dir, git, repo, ts }`, and `App.svelte` subscribes + wires `StatusLine` props reactively.

**Still open (upstream-blocked):**
- `CTX%` / `SESSION%` / `SESSION USE%` / `WEEK%` → Claude Code must emit a hook event with usage-payload schema (token counts + window size + week-rolling totals). No such hook exists in the upstream Claude Code surface as of 2026-04-28. Tracking: confirm via `/changelog-check` quarterly until the hook lands.
- `MODEL` → emitted by Claude Code as part of session-init hook; same upstream-blocked path as CTX% / SESSION%.
- **Unblocking event:** Claude Code usage hook lands with token-count payload; then wire a cc-translator or extend `status.rs` to subscribe + republish as additional `Category::Status` fields. Update `StatusLine.svelte` subscription handler to read `ctx`, `sessionUse`, `week`, `model` from the envelope payload.

### D-013 — Updater plugin Rust integration — PARTIALLY CLOSED 2026-04-28 (Rust plugin wired + dep added; frontend session-start check + GitHub Secret + active-flip still open)

**Closed sub-tranche (2026-04-28):** Rust-side plugin integration shipped. Workspace + crate Cargo.toml declare `tauri-plugin-updater = "2"` (resolved to v2.10.1). `src-tauri/src/lib.rs` wires `.plugin(tauri_plugin_updater::Builder::new().build())` into the `tauri::Builder`. `src-tauri/capabilities/default.json` grants `updater:default`. `tauri.conf.json` pubkey field carries a real minisign public key (no longer the `PLACEHOLDER_PUBKEY_RUN_TAURI_SIGNER_GENERATE` sentinel). `package.json` declares `@tauri-apps/plugin-updater@^2.0.0` (dep only — no JS call sites yet). Full preflight green: fmt / clippy / build / test (84 workspace tests + 14 rift-aegis private-module tests) / `npm run check` (207 files, 0 errors) / §9 boundary check / `cargo build -p rift --features aegis --locked` / `cargo clippy -p rift --features aegis --locked`.

**Still open (must land before flipping `plugins.updater.active` to `true`):**
- **Frontend session-start check** — `src/` has zero call sites for `@tauri-apps/plugin-updater`. Need an `App.svelte` (or equivalent) `onMount` that calls `check()` from the plugin and surfaces an `update.available` UX (likely a notification-tab card per §10.7 capability-driven empty-state pattern).
- **GitHub Secret `TAURI_SIGNING_PRIVATE_KEY`** — out-of-band user task. Per `RELEASING.md` §1c, the private half of the keypair (printed by `tauri signer generate -w ~/.tauri/rift.key` when the public half above was generated) must land as a repo secret before `release.yml` can sign release artifacts.
- **Vitest 2 → 4 major-version bump** rode along in this batch (`devDependencies.vitest` `^2.1.0` → `^4.1.5`). No frontend test suite exists yet (C5 shipped infra only), so no regression surface to verify — but if Phase 8 lands frontend tests, validate the bump didn't change config-file syntax.
- **Active-flip** — `plugins.updater.active` stays `false` until the frontend check + GitHub Secret are both in place.

### D-017 — Viewer edit-mode syntax highlighting (post-v1 ask, opened 2026-04-29)

- Read mode uses Shiki for full syntax highlighting (`Viewer.svelte:330`). Edit mode is a plain `<textarea>` (`Viewer.svelte:313-319`) — no highlighting while typing.
- Plain `<textarea>` cannot have inline syntax highlighting; it requires a real code editor (CodeMirror 6, Monaco, or a Shiki-overlay-on-contenteditable hack).
- **Cost**: medium-large.
  - **CodeMirror 6** (~150 KB gzipped): import `@codemirror/view`, `@codemirror/state`, `@codemirror/language`, `@codemirror/legacy-modes`. Wire the CM6 EditorView into the viewer body when `mode === 'edit'`. Map fs_read_text/write_text to the editor doc. ~1-2 hr including theme integration with the amber palette.
  - **Monaco**: heavier (~3 MB), gives full IDE feel (Intellisense, multi-cursor) but overkill for the §11 "friction-reduction-only" editor scope.
  - **DIY contenteditable + Shiki re-render on input**: lightest dep but laggy on large files.
- **Why deferred**: §11 explicitly bounds the in-cockpit editor to "spot something in the graph, fix it, return to flow" — multi-file refactoring + IDE features are out of scope. Plain textarea is consistent with that scope but loses the syntax cue. Worth scoping a v1.x decision: do users want syntax-highlighted editing badly enough to take a 150KB dep + a code-editor abstraction surface? If yes, CodeMirror 6 is the right size. Currently undecided.
- **Unblocking event**: user signals "I want syntax in edit mode badly enough to take CodeMirror 6 as a dep" → wire CM6 + theme.

### D-016 — StatusLine EFFORT segment data source (opened 2026-04-29)

- The EFFORT segment was added to StatusLine row 1 as part of Phase 8.7g.2 alongside the SKILL segment, both in the AMBER family per the new category palette. SKILL is live via `aegis.session.skill_loaded`; EFFORT is currently rendered as `'—'` because no envelope publishes the current Aegis effort level.
- The data exists conceptually — Aegis's pr001 EFFORT LEVELS section maps tasks to low / medium / high / xhigh / max, and `/aegis` mode dispatches calculate it per-invocation. It just isn't surfaced over the bus yet.
- **Cost**: small, frontend-only on the consumer side.
  - Producer (Aegis-side, gitignored crates/rift-aegis): publish `Category::Aegis / kind="aegis.session.effort"` whenever `/aegis` resolves a tier (or as a derived state from the current dispatch). Same envelope shape as `aegis.session.skill_loaded`. Out of scope of public CI; lives behind the `aegis` feature gate.
  - Consumer (frontend, public): subscribe alongside the existing `aegis.session.skill_loaded` listener in App.svelte; bind the value to `<StatusLine effort={…} />`. ~10 lines.
- **Why deferred**: the producer is in the private rift-aegis crate, which lives outside the public-CI build (D-011 close). The frontend consumer can land independently but has nothing to show until the producer publishes. Skip until either (a) the public Aegis stub gets a deterministic mock effort value for development OR (b) the user wants to wire it in their private build.
- **Unblocking event**: rift-aegis publishes an `aegis.session.effort` envelope on dispatch. Then App.svelte adds one subscribe + one bind.

### D-015 — IndexGraph sub-door rendering (post-v1 ask, opened 2026-04-29)

- User-requested: render nested sub-doors (e.g., `pr003/agentic-workflow.md`, `pr003/agentic-workflow/base.md`) as nodes linked to their parent vault. Currently the IndexGraph only renders top-level vaults; sub-doors exist on disk and are visible to `integrity-check.ps1` (SUB-OK / SUB-SUB-OK lines) but are invisible to both the vault-walker translator and the frontend.
- **Cost**: BOTH translator-change AND frontend-change. Surfaced 2026-04-29 by a dedicated scout pass.
  - **Translator** — `crates/rift-bus/src/translators/vault_walker.rs:684-735` boot walk uses `std::fs::read_dir(&vaults_dir)` non-recursively. Sub-directories are not traversed. Either (a) switch to the `walkdir` crate for cross-platform recursive traversal or (b) implement manual recursion with explicit depth limit. Either way: emit one `Category::Index / kind="vault.update"` envelope per `.md` file at every depth.
  - **Schema** — `index.rs:83-94` `VaultUpdatePayload` has `vault_id`, `path`, `change_kind` (and rich variant adds `name`, `cross_refs`). No parent linkage. Add `parent_vault_id: Option<String>` (None for top-level) so the frontend can wire edges without parsing slashes.
  - **Frontend** — `src/lib/IndexGraph.svelte` subscription block (lines ~239) currently treats `vault_id` as a flat identifier. Generalize to: every distinct `vault_id` becomes a node; if `parent_vault_id` is present, add an edge from child → parent. Hierarchical IDs (e.g., `pr003.agentic-workflow.base`) need to be syntactically valid `vault_id` strings — coordinate with how integrity-check + manifest builder already produce them.
- **Why deferred**: spec change (add a new field to a load-bearing payload) + crate dep change (potentially adding `walkdir`) + visual-density implications (the radial layout will need re-tuning when node count multiplies). Wants its own decision pass + plan, not an inline fix during BV-regression cleanup.
- **Unblocking event**: post-v1 plan beat that decides:
  1. Recursion strategy (walkdir vs hand-rolled) + max-depth policy
  2. Whether sub-doors are first-class nodes (full visual treatment) or rendered differently (smaller, dimmer, or expand-on-click)
  3. Layout strategy (still radial-by-kind, or local-cluster around parent)
  4. Whether to ship behind a `rift.ui.show_sub_doors` config flag (preserves the simpler default for users with ~40 vaults; opt-in for power users with deep nesting)

### D-014 — Rift MCP server (post-v1 ask, opened 2026-04-29)

- User-requested capability: a Rift-side MCP server that lets external Claude Code sessions (or any MCP-aware client) directly connect, control, screenshot, and test the running Rift instance. Originated during Phase 8.7 BV-regression diagnostic when the user observed the absence of a programmatic-control surface cost ~10% of weekly token budget on the IndexGraph node-drag guess loop — Playwright connecting to localhost:1420 worked for inspecting Vite-served frontend code but couldn't reach Tauri-only APIs (`invoke`, `listen`, native menus, secondary windows) and couldn't execute drag gestures inside WebView2.
- v1.x scope sketch (not committed):
  - **Capabilities**: snapshot DOM (or accessibility tree), screenshot main + cockpit windows, evaluate JS in either webview, simulate input (click, type, drag-drop incl. native), read/write the bus envelope stream, query Aegis log + skill state, drive PTY input/output, navigate the file tree.
  - **Transport**: stdio MCP server inside `src-tauri/` exposing the above as MCP tools. Mirrors the §9 translator-boundary discipline — the MCP server is an *external interface* and must speak the existing protocol (Category, Envelope) where it observes Rift state, not reach into internals directly.
  - **Security**: opt-in via a Rift settings flag; off by default; bound to localhost; per-session token. Critical because this surface gives full UI/PTY control of the dev's terminal.
- **Why it's deferred from v1**: v1 scope locked at standalone terminal + GUI cockpit (§1, §11). Adding an MCP surface is a separate translator + multi-tool API design that wants its own decision doc and capability-discovery shape (parallels §9 Integration Decoupling). Not blocking v1 ship.
- **Value beyond Rift**: outlives Rift v1 — same MCP would let other Claude sessions drive any future Abyssal app embedded in Rift, automate UX testing of the cockpit, and let Aegis run end-to-end checks on its own host without manual user steps. Generalizes to "Abyssal-app MCP" pattern.
- **Unblocking event**: v1 ships → user spec for MCP surface (which capabilities to expose first, transport choice, auth model) → translator-style implementation in `crates/rift-bus/src/translators/mcp.rs` (or sibling crate). Track via `/changelog-check` if Tauri ships an official MCP plugin first.
- **Scope guardrails (when picked up)**: do NOT bypass the bus protocol — the MCP server is a *consumer/producer of envelopes*, not a back-channel into PTY/state. This preserves the §9 translator-boundary that makes Rift testable in the first place.

### D-010 — Sentinel implementation (active 2026-04-27, opened by Phase 7.0 architecture lock)
- Spec §10.11 names Sentinel as the source-of-truth for agent misbehavior detection (stuck / runaway / unauthorized edits); Rift is the display layer.
- Sentinel does NOT yet exist in the workspace — no crate, no source file, no Aegis-side spec defining the event surface. Greenfield post-v1 work.
- v1 scope: Phase 7.5 ships an empty Agents-tab placeholder card "Sentinel: integration not loaded" per §10.7 capability-driven empty-state pattern. No detection logic, no event subscription — pure visual stub that lights up when a future Sentinel translator self-registers.
- **Unblocking event**: (a) Sentinel architecture spec lands as a separate planning beat post-v1, AND (b) a Sentinel-side implementation produces detectable misbehavior events on a documented schema. Then Rift's Agents tab subscribes to `sentinel.*` envelopes and renders them alongside existing Aegis-derived `agent.*` events.
- Created during Phase 7.0 architecture lock (this commit). No code change required to open this deferral — pure spec deferral. Phase 7.5 will write the placeholder card and reference this entry inline.
- Phase 7.5 placeholder card landed in `src/lib/NotificationPane.svelte` (persistent-state section, bottom of the state-panel footer).

---

## Closed deferrals

### C-014 — D-011 public-CI fresh-clone build (closed 2026-04-27)

Resolved via the **minimal-stub + cfg-gated private modules** pattern (option b variant from D-011's three options). Verified end-to-end with a public-clone simulation (move `detect.rs` + `snapshot.rs` aside, run `cargo build --workspace --locked` → exit 0; restore + run `cargo build -p rift --features aegis --locked` → exit 0).

**Mechanism (4 surgical edits + 1 boundary-check update)**:
- `crates/rift-aegis/Cargo.toml` (now TRACKED): added `[features] private_modules = []` empty-feature section. Deps unchanged (rift-bus path + tokio + serde + serde_json + tracing + directories + notify) so the private impl can compile when the feature flag is on; on public CI those deps go unresolved (rift-aegis itself is the optional path dep).
- `crates/rift-aegis/src/lib.rs` (now TRACKED): every `pub mod` and `pub use` line wrapped in `#[cfg(feature = "private_modules")]`. Public stub compiles to empty; private dev with the feature on activates the modules. Cargo only resolves `pub mod detect;` to a file lookup when the cfg is active, so `detect.rs` does NOT need to exist on public clones.
- `src-tauri/Cargo.toml`: `aegis` feature flipped from `["dep:rift-aegis"]` to `["dep:rift-aegis", "rift-aegis/private_modules"]`. Feature unification: `cargo build -p rift --features aegis` propagates `private_modules` to rift-aegis automatically.
- `.gitignore`: pattern flipped from `/crates/rift-aegis/` (full-dir ignore) to `crates/rift-aegis/src/*` + `!crates/rift-aegis/src/lib.rs` (ignore private impl files; track Cargo.toml + lib.rs). Also keeps `crates/rift-aegis/target/` ignored.
- `tools/check-translator-boundary.sh`: `check_rift_aegis_gitignored` (Phase 7.1, full-dir-must-be-ignored invariant) replaced with `check_rift_aegis_private_files_ignored` (the new invariant — only `lib.rs` may be tracked among `src/*.rs`; any other tracked `.rs` under `src/` fails the boundary check). Self-test (`--test` mode) still runs cleanly.

**Verification (all 9 canonical gates green on the fixed state)**:
1. fmt; 2. clippy workspace; 3. build workspace --locked (PUBLIC CLONE SIM — `detect.rs` + `snapshot.rs` moved aside; rift-aegis compiles to empty); 4. test workspace --locked; 5. npm check; 6. boundary (rewired). 7. build -p rift --features aegis --locked (PRIVATE DEV SIM — files restored; rift-aegis activates `private_modules`); 8. test -p rift-aegis --features private_modules --locked → 14 tests pass; 9. clippy -p rift --features aegis.

**Updated CI gate 8 wording (BV mode going forward)**: tests for rift-aegis now require the `--features private_modules` flag (or `-p rift --features aegis` propagation). The previous form `cargo test -p rift-aegis --locked` returns "0 tests" because the modules are cfg-gated. Future BV briefs should specify `cargo test -p rift-aegis --features private_modules --locked`.

**Trade-offs vs the other two options**:
- (a) submodule / sibling cargo project: rejected for git-remote complexity.
- (c) CI-time stub injection: rejected — would have left rust-analyzer broken on public clones (no tracked Cargo.toml means `cargo metadata` fails locally too, even outside CI).
- The chosen (b) variant gets us: clean public-clone DX (rust-analyzer works), zero private-dev friction (no skip-worktree dance — private dev's gitignored `detect.rs` / `snapshot.rs` are silently absent from `git status`), single-source-of-truth Cargo.toml (private impl deps live in the tracked Cargo.toml; tested unused on public CI).

Now safe to push: `origin/main` will accept all 13 unpushed commits without turning CI red.

### C-013 — D-007 Mockup #3 integrated cockpit (closed 2026-04-27)
- New `rift-v2-mockup-integrated.html` (1042 lines) — the default attached cockpit experience per §11. Single window with shared titlebar (`◆ RIFT` + `COCKPIT — INTEGRATED` mode label + `↗ DETACH GUI` button mirroring mockup #2's RE-ATTACH) + horizontal split (terminal LEFT 62%, GUI RIGHT 38%) + full-width 2-row status line.
- Visual vocabulary inherits 100% from mockups #1 and #2 — same `:root` palette tokens, same scanlines + vignette, same JetBrains Mono, same lane colors / tag styles / line classes / tab anatomy / tree-row classes / project-swap + view-toggles chrome.
- Integration moment locked: terminal issues `claude "add an error boundary wrapper to NotificationPane.svelte per §10.4"`; mid-flow the GUI-right surface lights up — graph node for `NotificationPane.svelte` glows amber-bright with `CLAUDE` attribution label, sibling files (`App.svelte`, `bus.ts`) carry recent-decay state, edges trace `App.svelte → NotificationPane.svelte → bus.ts → RiftBus`, AND the file-tree row marks NotificationPane as ACTIVE with the same CLAUDE badge. One terminal action → two GUI surfaces light up on the same file. Readable in a single glance.
- Right column stacked layout: graph (~55% height, hand-placed SVG with 5-8 nodes — RiftBus, App.svelte, NotificationPane.svelte, Terminal.svelte, bus.ts, pty.rs) on top; 12-15 row file tree on bottom.
- 1px `var(--border-subtle)` vertical divider between cockpit-left and cockpit-right; resize handle deferred (out of scope for visual mockup).
- Mockup plan §11 now complete: #1 terminal-alone (rift-v2-mockup.html, ✓), #2 GUI-alone detached (rift-v2-mockup-gui.html, ✓), #3 integrated attached (rift-v2-mockup-integrated.html, ✓ — this entry).

### C-012 — D-008 global hooks wiring (closed 2026-04-27, Phase 5.7)
- **Binary install:** `cargo install --path crates/rift-cli --locked` puts release-optimized `rift.exe` at `C:\Users\Critek\.cargo\bin\rift.exe` (already on PATH for Rust dev). The cargo-build target/debug/rift.exe collision between rift-cli and src-tauri remains as a workspace-build warning but doesn't affect the installed binary — `cargo install` writes only the rift-cli bin to `~/.cargo/bin/`, separate from the local `target/`. No bin rename needed.
- **Smoke test (pre-wire):** `echo '{...}' | rift hook PreToolUse` with no `RIFT_SOCKET_NAME` set → exits 1 in <50ms with the documented "no socket name. Pass --socket <name> or set $RIFT_SOCKET_NAME" message. Graceful-failure path verified before any settings.json edit.
- **Settings.json wiring** (`~/.claude/settings.json` — global, NOT in this repo): added 8 hook-group entries (one per D-008 event) under existing `hooks` block. Each entry has no `matcher` (matches all tool/event invocations) and a single command `rift hook <EventName>`. Existing user hooks (edit-guard, ccstatusline, completeness-check, auto-fmt-rust, aegis-log, vault-autoindex, cache-heal, aegis-session-end, aegis-precompact) untouched — rift entries APPENDED last in each event's array so they fire after existing hooks.
- **Events wired (8):** PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, SessionEnd, Notification, Stop, SubagentStop. PreCompact (existing user hook) intentionally NOT wired — not in D-008's spec list. Notification/Stop/SubagentStop event arrays were created fresh; the others appended to existing arrays.
- **JSON validation post-edit:** `node -e "require(...)"` parses cleanly; enumeration confirms 9 event keys (8 wired + existing PreCompact) and a `rift hook <Event>` entry as the last hook of each wired event.
- **Hot-reload trap (per pr003):** Claude Code reads `settings.json` ONCE at session start. The 8 hooks won't fire in the session that authored them — they activate on the NEXT Claude Code session. Acceptable per design.
- **Per-tool-use latency:** rift.exe spawn + clap parse + env-check is fast (<50ms on no-socket fail-fast path). Acceptable per D-008's "graceful failure without breaking Claude Code" spec line.
- **Reversibility:** removing the 9 added entries (1 per wired event + 3 new event arrays) restores prior settings.json behavior. Diff is surgical and idempotent.
- **Acceptance (next session):** with Rift running, fire any Claude Code tool use → envelope appears in Rift's Hooks tab live activity strip; non-Rift sessions log a graceful "no socket name" error without breaking Claude Code. First runtime confirmation pending the next session start.
- **Sister deferral state:** D-005 closed (3.5a + 3.5b), D-006 closed, D-008 closed. Only D-007 (mockup #3 integrated terminal+GUI) remains active.

### C-011 — D-005 pop-out chassis (closed 2026-04-27, Phase 3.5b)
- Pop-out infrastructure (§10.5) shipped as a chassis — global rune-aware store + overlay shell + App-level stack render. No production consumer yet; first consumer (rule editor / file viewer / agent cancel confirm) lands in Phase 5+ once content exists.
- New `src/lib/popouts.svelte.ts` — singleton `PopoutStore` instance exported as `popouts`. Public API: `summon(opts) → id`, `dismiss(id)`, `dismissTop()`, `dismissAll()`. Private monotonic `#nextId`. `entries: PopoutEntry[]` is `$state<...>`-backed; mutations use immutable spread to match the rest of the codebase's `$state` pattern. File extension `.svelte.ts` is required so the Svelte 5 rune compiler processes `$state` (plain `.ts` would not work).
- Discriminated-union `PopoutContent` — `kind: 'text'` (title + body) and `kind: 'confirm'` (title + body + optional confirmLabel/cancelLabel/onConfirm/onCancel). Future kinds (component / snippet) deferred to Phase 5+ when there's a real consumer to validate the API shape against.
- New `src/lib/Popout.svelte` — overlay shell. Props: `entry: PopoutEntry`, `isTop: boolean`, `stackIndex: number`. Behavior: full-viewport `.backdrop` (rgba 0,0,0,0.7) wraps an amber-bordered `.card` with header (title + close-X) + body (text or confirm-with-actions). Click-outside dismiss only fires on the top overlay; card `e.stopPropagation()` prevents inner clicks from bubbling. Esc dismiss attached via `$effect`-managed `window.addEventListener('keydown', ...)` cleaned up on teardown — only the top + dismissible entry reacts. Non-dismissible entries (`dismissible: false`) hide the close-X and ignore Esc + backdrop; only programmatic `dismiss(id)` / `dismissAll()` close them. Confirm-kind buttons fire `entry.content.onConfirm/onCancel` then `popouts.dismiss(entry.id)`. Z-index = `1000 + stackIndex * 10` so each stacked overlay paints above the prior one without clashing with app chrome.
- `src/App.svelte` MOD — imports `Popout` + `popouts`; renders `{#each popouts.entries as entry, i (entry.id)}` at the end of `<div class="app-shell">` (after `StatusLine`), passing `isTop = (i === entries.length - 1)` and `stackIndex = i`. No production summon calls in 3.5b — chassis-only.
- Visual style: matte black backdrop, amber-bright card border, `var(--bg-elevated)` body, `var(--glow-amber-faint) + 0 8px 32px rgba(0,0,0,0.5)` shadow, JetBrains Mono inherit. Two CSS keyframes: `popout-fade-in` (120ms) for backdrop, `popout-card-in` (160ms cubic-bezier) for card. Card max-width 90vw / max-height 80vh; default width `min(640px, 80vw)`, overridable per-entry via `entry.width`.
- Files: `src/lib/popouts.svelte.ts` (NEW), `src/lib/Popout.svelte` (NEW), `src/App.svelte` (MOD), `DEFERRED.md` (MOD — flip D-005 to fully closed + this entry). Net 4 files.
- Verification: all 6 CI gates exit 0 — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace --locked`, `cargo test --workspace --locked` (46 tests preserved), `npm run check` (0 errors / 0 warnings), `bash tools/check-translator-boundary.sh`. Runtime exercise (real summon from a Phase 5+ consumer) deferred to that consumer's BV cycle — chassis only here.
- **Sister deferral state:** D-005 now FULLY CLOSED (drag-promote half + pop-out chassis both shipped). D-008 (global hooks wiring) remains DEFERRED by user choice.

### C-010 — D-005 drag-promote half (closed 2026-04-27, Phase 3.5a)
- Drag any notification tab off the tab strip → promotes it to a fixed-width 420 px right-side pane alongside the active session/empty surface. Drag the pane's drag-handle back onto the tab strip → demotes. HTML5 native drag-and-drop API; tab-strip nav is the demote drop target (`ondragover` preventDefault + `ondrop` → `onDemote`).
- Max 1 promoted at a time enforced structurally via `let promoted = $state<string | null>(null)` in `App.svelte`. Promoting a 2nd tab assigns over any prior value, auto-demoting the 1st. Toggling-disable on the promoted tab also auto-demotes (`toggleNotif` → if `promoted === id`, set `promoted = null`).
- Layout split: when `promoted != null`, `<main>` switches to `flex-direction: row` with `.main-left` (flex 1, columnar) + `.promoted-pane` (flex `0 0 420px`, `border-left`). `Terminal.svelte`'s existing ResizeObserver path catches the column→row transition and refits cols/rows.
- Promoted side-pane is a SECOND `NotificationPane` instance independent of the active-tab pane; both are wrapped in `{#key id}` blocks so swapping the promoted tab destroys the prior subscription cleanly via `onDestroy → unsubscribe` before mounting the new one (`drag-promote-rekey-on-swap` lesson).
- Promoted tab visual marker: `↗` glyph (lane-accent colored — cyan for hooks, red for errors, amber default), opacity 0.55, click-in-strip is no-op while promoted (demote only via drag-back).
- `NotificationPane` gained optional `onDragBack` prop that gates rendering of a small drag-handle bar above the existing status header. Handle's `dragstart` sets `dataTransfer.setData('text/plain', ...)` for cross-browser drag validity (`html5-dnd-setdata-required-for-validity` lesson); the strip's `ondrop` is what actually fires `onDemote`.
- Files: `src/App.svelte` (+108 / −41), `src/lib/TabBar.svelte` (+85 / −2), `src/lib/NotificationPane.svelte` (+67 / −1). Net 3 files, +260 / −44.
- Verification: all 6 CI gates exit 0 — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace --locked`, `cargo test --workspace --locked` (46 tests preserved), `npm run check` (107 files / 0 errors / 0 warnings), `bash tools/check-translator-boundary.sh`. Runtime acceptance (drag gestures + 2-column layout transition + Terminal refit) deferred to first `/aegis --verify` chain on this project (`.aegis/verify.toml` is on `from_bv = "autonomous"`).
- **Sister deferral state:** D-005 still ACTIVE for pop-out half (no consumer until Phase 5+).

### C-001 — §10.15 real-time update mechanism (closed 2026-04-26)
- Resolved by `decisions/§10.15_real-time_update_mechanism.md`. Two-tier architecture: `tauri::ipc::Channel<T>` + Tauri events for in-process; tokio broadcast + UDS/named-pipe IPC server (V1 pattern) for cross-process. Vision §10.15 to be patched to LOCKED in v0.6.

### C-002 — App icons (closed 2026-04-26)
- Generated full set from user-supplied `Icon.png` (1024×1024 RGB) via `npm run tauri icon Icon.png`. Wired into `src-tauri/tauri.conf.json` `bundle.icon` (Windows ICO, macOS ICNS, 32/128/128@2x PNGs). iOS / Android variants also produced and live under `src-tauri/icons/` for future mobile work (post-v1 §13).

### C-009 — D-006 Translator surface complete (closed 2026-04-26, Phase 5.5)
- **Errors translator** (commit `6bdcc5d`): `crates/rift-bus/src/translators/errors.rs` ships `publish(bus, source, message, context)` (fire-and-forget; logs internally on bus errors, does not propagate). Emits `Category::System` / `kind: "error"` with payload `{source, message, context}`. Re-exported as `rift_bus::publish_error`. Every `Result<_, String>`-returning Tauri command in `src-tauri/src/lib.rs` instrumented at every `Err` site (7 sites across 6 commands; `pty_write`/`pty_resize`/`pty_kill` signatures widened to take `State<'_, RiftBus>`). `src/App.svelte` `CATEGORY_BY_NOTIF.errors → 'system'`.
- **Commands translator** (commit `6bdcc5d`): `crates/rift-bus/src/translators/commands.rs` ships `CommandBuffer` (line-buffer state machine: handles `\r`/`\n`/`\r\n` as single boundary, partial-buffer carryover, lossy UTF-8 decode) + `publish(bus, session_id, command, raw_len)`. Emits `Category::Pty` / `kind: "command.submitted"` with payload `{session_id, command, raw_len}`. Re-exported as `rift_bus::publish_command`. `CommandBufferRegistry` managed Tauri state tracks per-session line buffers; `pty_start` inserts, `pty_write` feeds AFTER successful write only (failed writes already publish via the errors translator), `pty_kill` removes. `src/App.svelte` `CATEGORY_BY_NOTIF.commands → 'pty'`.
- **§9 build-time guard** (commit `76e2843`): `tools/check-translator-boundary.sh` greps every tracked `*.rs` under `crates/` + `src-tauri/src/` for forbidden external-system primitives (`tokio::net::`, `reqwest::`, `claude_(api|code|sdk|cli)::`, `mcp_(client|server|core)::`). Allowlist: `crates/rift-bus/src/translators/**/*.rs` (the boundary itself), `crates/rift-bus/src/ipc.rs` (bus's own internal transport — forward-defense; current impl uses the `interprocess` crate, but the allowlist preserves the bus's right to use raw `tokio::net::*` should the impl swap), `**/tests/**/*.rs` (test files allowed). `--test` mode injects a deliberate violation, asserts the script catches it, cleans up via `trap` (works on success, failure, OR signal). `--help` mode documents usage + pattern catalog + recommended fix. Exit 1 on any violation.
- **First CI workflow** (commit `76e2843`): `.github/workflows/ci.yml` runs on `ubuntu-latest` for push + PR on all branches. 12 steps: `actions/checkout@v4` → `actions/setup-node@v4` (node 20, npm cache) → `dtolnay/rust-toolchain@stable` → `Swatinem/rust-cache@v2` → apt install Tauri 2 Linux deps → `npm ci` → `cargo fmt --all --check` → `cargo clippy --workspace --all-targets -- -D warnings` → `cargo build --workspace --locked` → `cargo test --workspace --locked` → `npm run check` → `bash tools/check-translator-boundary.sh`. Single ubuntu runner in v1; Windows matrix and SHA-pinning of third-party actions remain deferred audit items.
- **Companion fix** (commit `ea96d9b`): `command-buffer-leak-on-natural-pty-exit` — surfaced by validator during the commands BV cycle. Exit-watcher in `pty_start` now removes from `CommandBufferRegistry` alongside `PtyRegistry` so buffer entries don't leak across natural-exit sessions (e.g., user types `exit` in the shell).
- **Tests:** workspace `cargo test --workspace --locked` 22 → 46 (+12 errors translator + +12 commands translator; existing 22 preserved). All 6 CI gates pass locally; boundary check exit 0 (default mode + `--test` mode). Validator independently probed `reqwest::`, `claude_api::`, `mcp_client::` patterns by injecting test violations: all three additional regex categories fire as expected (exit 1 with FORBIDDEN line per pattern).
- **Acceptance met (per the original D-006 acceptance):** errors-translator surfaces Tauri command Errs as `Category::System kind:"error"` envelopes visible in the Errors tab ✓; commands-translator surfaces submitted commands as `Category::Pty kind:"command.submitted"` ✓; CI fails on a deliberate `reqwest::Client::new()` outside `translators/` ✓ (proven end-to-end across all 4 forbidden-pattern categories).
- **Sister deferral state:** D-008 (global hooks wiring) remains DEFERRED by user choice — independent of D-006, no longer blocked by anything.

### C-008 — Hooks tab + bus producer/consumer chassis (closed 2026-04-26, Phase 5.1 + 5.2 + 5.3)
- `crates/rift-bus` exports re-used directly from `src-tauri`: `Category`, `Envelope`, `RiftBus`, `SubscribeFilter`. Three new Tauri commands wire the webview into the bus:
  - `bus_subscribe(category: Option<String>, on_envelope: Channel<Envelope>) -> u64` — returns a subscription id, drains the replay snapshot synchronously into the channel, then forwards live envelopes via a spawned task that selects on a one-shot teardown receiver.
  - `bus_unsubscribe(id: u64)` — fires the one-shot, drain task exits cleanly.
  - `bus_publish(category, kind, payload?)` — frontend-side producer. Used by the demo button; same call shape future translators will use in-process.
- `BusSubscriptionRegistry` (managed Tauri state, AtomicU64 + `Mutex<HashMap<id, oneshot::Sender<()>>>`) tracks live subscriptions for clean teardown. Drain tasks remove themselves on channel close, rx error, or teardown signal.
- `parse_category(raw)` uses `serde_json::from_value` so adding a `Category` variant lights up at the wire layer with no string-table maintenance — additive-versioning rule preserved end-to-end.
- New `src/lib/bus.ts` mirrors the Rust schema (`Category` union + `Envelope` interface) and exposes ergonomic `subscribe(opts, onEnvelope)` returning a teardown promise + `publish(category, kind, payload?)`.
- `NotificationPane.svelte` refactored: `categoryFilter` prop drives `bus_subscribe`. Four §10.8 sections populate from real envelopes — status header shows event count + relative `last seen`, live activity strip renders kinds in the trailing 4-second window with a per-second tick, recent log renders timestamped `kind` + `payload` rows with `display: grid` columns and hover highlighting, persistent state renders kind histogram (top 6) + counters. Demo button beside the meta segment publishes `${category}.demo.click` for end-to-end verification.
- `App.svelte` maps `tab.id → Category` via a small `CATEGORY_BY_NOTIF` table (`hooks → 'hook'`; others undefined until their translator design lands). The notification surface re-keys on `activeNotifTab.id` so switching tabs gives the pane a fresh subscription tied to that tab rather than reusing one bound to the previous tab.
- Verification: `cargo check -p rift` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` 22/22 PASS; `npm run check` 107 files / 0 errors / 0 warnings.

### C-007 — Tier-2 IPC server (closed 2026-04-26, Phase 4.3 + 4.3.b)
- `crates/rift-bus/src/ipc.rs` shipped: `IpcServer` + `IpcClient` over `interprocess` v2 (UDS on Unix, named pipe on Windows). Length-prefixed JSON frames (4-byte LE prefix + serde_json `Envelope`), `MAX_FRAME_BYTES = 16 MiB` malformed-peer guard.
- Per-connection lifecycle: drain replay snapshot synchronously on accept → fan out live envelopes via the bus's `SubscribeFilter::All`. Bidirectional — clients can also publish back through the same connection; their inbound frames are pushed onto the same bus.
- Backpressure: `BusError::Lagged(n)` from a per-connection writer closes the connection so the client reconnects and re-drains a fresh snapshot.
- Wired into Tauri: `setup` hook spawns `IpcServer` on a process-unique socket name (`rift-v2-<pid>.sock`); `BusIpcState` holds the server alive in Tauri-managed state for the process lifetime; `rift_bus_status` diagnostic command returns `{ socket_name, subscribers, replay_len }`.
- Tests: 17/17 rift-bus passing (added 4 IPC: replay-then-live, client→bus publish round-trip, frame-too-large rejection, shutdown invocation no-panic). One brittle Windows-only test removed and documented inline — graceful-shutdown semantics are an internal detail not a wire-protocol contract; the four contract-level tests cover the actual surface.
- Verification: `cargo test --workspace` → 22/22 PASS (17 rift-bus + 5 rift-core); `cargo clippy --workspace --all-targets -- -D warnings` clean.

### C-006 — Rift Integration Protocol bus core (closed 2026-04-26, Phase 4.1 + 4.2)
- `crates/rift-bus` lit up: `envelope::{Envelope, Category, CURRENT_VERSION}` + `bus::{RiftBus, SubscribeFilter, Subscription, BusError}`.
- `Envelope` is `#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]` with `version` (u16), `category` (Category enum, `serde(rename_all = "lowercase")`), `kind` (String), `ts` (u64 unix ms), `payload` (`serde_json::Value`). `with_payload<T: Serialize>` helper. Additive-categories rule is the documented invariant — adding a `Category` variant or a new `kind` does NOT bump `CURRENT_VERSION`.
- `RiftBus` is `Arc`-backed Clone, default capacities 1024 broadcast / 128 replay. `publish` writes to ring buffer first then broadcasts (so late subscribers see the snapshot even when the broadcast send returns Err for zero-subscribers). `subscribe(filter)` returns `(Vec<Envelope>, Subscription)` — the snapshot is the filtered ring buffer at subscribe time; the live `Subscription` re-applies the filter on each `recv` and surfaces `BusError::Lagged(n)` for backpressure recovery via re-subscribe.
- 12 unit tests green: serde round-trip per category, lowercase wire format, version stamp, publish-before-subscribe snapshot delivery, category filter exclusion, multi-category filter, custom-closure filter, ring-buffer drop-oldest at capacity, subscriber count tracking, zero-subscribers no-panic.
- Tier-2 IPC server (D-002) remains for when first cross-process translator lands.

### C-005 — Tab/Pane chassis (partial close 2026-04-26, Phase 3.1 + 3.2)
- Multi-session terminal tabs: `App.svelte` owns `sessions` + `notifs` + `active` state via Svelte 5 runes. Each session keeps its own `Terminal.svelte` instance alive; inactive ones go `display: none` so xterm preserves scrollback. `Terminal.svelte` accepts a `visible` prop and refreshes/re-fits on transition `false → true` to redraw bytes that arrived while hidden.
- `+` button → `addSession()` mints a new tab id, appends to list, activates it; `Terminal.svelte` mounts → `pty_start` fires.
- `×` close button → `closeSession()` filters from list; `Terminal.svelte`'s `onDestroy` invokes `pty_kill`. Closing the last tab routes to an empty-state card with a `+` hint.
- `NotificationPane.svelte` shipped with 4-section §10.8 anatomy (status header / live activity strip / recent events log / persistent state panel). Accent prop drives per-tab tinting (`hooks` cyan / `errors` red / others amber).
- Per-tab toggle §10.6: right-click any notification tab → toggles `enabled`; disabled tabs render struck-through, can't be clicked open, and auto-deactivate if currently shown.
- See D-005 for what remains (drag-promote pane + pop-out infrastructure).

### C-004 — Visual chassis (closed 2026-04-26, Phase 2)
- `src/styles.css` extended with global scanlines + radial vignette (`body::before`/`::after`) + textured `.app-shell` background gradient + lane CSS classes + tag prefix CSS classes per §10.1 + §10.3.
- New components: `TitleBar.svelte` (drag region + min/max/close window controls via `@tauri-apps/api/window`), `TabBar.svelte` (one active session tab + add-tab button + 3 default notification-tab placeholders per §10.7 disabled until Phase 3), `StatusLine.svelte` (2-row, color-block segments, all values bold per §10.2 — DIR/MODEL/CTX/SESSION/SKILL/GIT/REPO/SESSION USE/WEEK with prop-driven values).
- Live data plumbing for status-line values (ctx %, skill, session use, etc.) deferred to later phases — Phase 5 lights up `dir`/`repo`/`git` from a Rust helper; Phase 7 lights up `ctx`/`session`/`skill`/`session use`/`week` via the Aegis private translator.
- Acceptance: mockup parity with `rift-v2-mockup.html` for chassis; svelte-check 0 warnings; cargo clippy `-D warnings` clean.

### C-003 — xterm.js bound to real PTY (closed 2026-04-26, Phase 1)
- `crates/rift-core` shipped: `PtySession::spawn` returns `(PtyOutput, PtyControl)`. Reader OS thread → tokio mpsc; exit-watcher OS thread polls `child.try_wait()` every 250 ms and resolves a one-shot exit-code receiver, per V1 `pty-exit-windows` lesson.
- `src-tauri/src/lib.rs` exposes `pty_start` / `pty_write` / `pty_resize` / `pty_kill` Tauri commands; `pty_exited` event emitted with `{ id, code }` payload.
- `src/lib/Terminal.svelte` wires xterm to PTY via `tauri::ipc::Channel<Vec<u8>>` per §10.15 decision; ResizeObserver propagates layout changes to the PTY; `pty_kill` invoked on component destroy.
- 5/5 rift-core unit tests green; full-workspace clippy `-D warnings` clean; `npm run check` 0 errors / 0 warnings. End-to-end input→output round-trip on Windows ConPTY pending manual `npm run tauri:dev` acceptance.
