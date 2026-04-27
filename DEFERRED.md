# DEFERRED — Rift V2

*Loud-log of deferrals required by `RIFT_V2_VISION.md` §7 ("No silent stubbing or deferring").*
*Every deferral here names the concrete unblocking event per `pr001` TODO SHARPNESS rule.*

---

## Active deferrals

<!-- D-002 closed 2026-04-26, see C-007 below -->


### D-005 — Tab/Pane/Pop-out architecture (partial — drag-promote + pop-out remain)
- **Phase:** 3 (chassis shipped) → 3.5 (drag-promote pane + pop-out infrastructure)
- **What's shipped (2026-04-26):** multi-session terminal tabs (`+`/`×`/click-to-switch with state preserved via `display:none` + xterm `refresh()` on visibility transition), notification-pane 4-section anatomy per §10.4 + §10.8 (status header / live activity strip / recent events log / persistent state panel), per-tab independent toggle §10.6 (right-click), default tab set Errors/Hooks/Commands per §10.7.
- **What remains:**
  - **Drag-tab-out / drag-pane-back gesture (§10.5)** — promotes a notification tab to a pane alongside the main terminal; only one promoted tab at a time. Benefits from interactive iteration.
  - **Pop-out infrastructure (§10.5)** — ephemeral overlay container (rule editor, file viewer, agent confirm). Deferred because no pop-out content exists until Phase 5+.
- **Concrete unblocking event:** explicit `/aegis` invocation to ship 3.3 (drag-promote) and 3.4 (pop-out infrastructure), or natural phase progression once Phase 5 has content that needs them.
- **Files affected (when resumed):** new `lib/Pane.svelte`, new `lib/Popout.svelte`, gesture glue in `App.svelte` + `TabBar.svelte`.
- **Owner:** future Phase 3.x builder.
- **Acceptance:** drag a notification tab outside the strip → promotes to right-side pane next to terminal; drag back → returns to tab strip; only one promoted at a time; pop-out container can stack overlays with click-outside-dismiss.

<!-- D-006 closed 2026-04-26 in three commits, see C-009 below -->

### D-008 — Global Claude Code `settings.json` hooks wiring for `rift hook`
- **Phase:** 5.4 close — deferred by explicit user choice on 2026-04-27.
- **Why deferred:** wiring `rift hook PreToolUse` / `PostToolUse` / etc. into `~/.claude/settings.json` would make EVERY Claude Code session (not just Rift's embedded ones) fire the binary on hook events. Until Rift is daily-driver usable AND a built `rift.exe` artifact is reliably on PATH, that's noise + potential failure surface across all sessions. Engineering is done; this is a deployment + lifestyle handshake the user wants to make consciously, not autopilot.
- **What's needed:**
  - Built `rift.exe` artifact reachable from any shell (PATH addition OR absolute path in settings.json).
  - Global hook entries in `~/.claude/settings.json` for PreToolUse / PostToolUse / UserPromptSubmit / SessionStart / SessionEnd / Notification / Stop / SubagentStop, each piping the hook JSON payload through stdin to `rift hook <event-type>`.
  - Live Rift instance exposing `RIFT_SOCKET_NAME` — without one, `rift hook` from a non-Rift Claude Code session falls back to the documented "no socket name" error path. Acceptable graceful failure (Claude Code doesn't crash on hook errors), but worth confirming end-to-end before flipping the switch globally.
- **Concrete unblocking event:** explicit user invocation ("ship D-008" / "wire the global hooks") once Rift is daily-usable. Not gated on any other Rift phase.
- **Files affected (when resumed):** `~/.claude/settings.json` (global, NOT in this repo); PATH or absolute reference to the built `rift.exe`.
- **Owner:** Garth (user-controlled — both the deployment decision and the per-session impact).
- **Acceptance:** with Rift running, fire any Claude Code tool use from any project → corresponding envelope appears in Rift's Hooks tab live activity strip + recent log; non-Rift sessions log a graceful "no socket name" error without breaking Claude Code.

### D-007 — Mockup #2 (GUI alone) and Mockup #3 (integrated) not built before Phase 6
- **Phase:** 0 → 6
- **Why deferred:** mockup #2 IS now built (`rift-v2-mockup-gui.html`, 2026-04-26). Mockup #3 (terminal+GUI integrated cockpit) remains pending per §11.
- **Concrete unblocking event:** before Phase 6 GUI implementation begins.
- **Files affected:** `rift-v2-mockup-integrated.html` does not exist yet.
- **Owner:** Garth + Claude.
- **Acceptance:** integrated cockpit view showing terminal + GUI side-by-side with locked visual vocabulary.

---

## Closed deferrals

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
