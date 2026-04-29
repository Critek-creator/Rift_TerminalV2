# Changelog

All notable changes to Rift Terminal are documented here. The format is
loosely [Keep a Changelog](https://keepachangelog.com/), the project follows
[Semantic Versioning](https://semver.org/) once v1.0 ships.

---

## [Unreleased]

Pre-v1.0 work-in-progress. Items move into a numbered release section once tagged.

### D-014 Phase A — Rift MCP server (scaffold)

- New `crates/rift-mcp/` workspace member — standalone stdio JSON-RPC
  binary that exposes a running Rift host to MCP-aware clients (Claude
  Code, automation harnesses). Binary name: `rift-mcp`. Server name:
  `Rift`. Hand-rolled JSON-RPC over `tokio::io::{stdin,stdout}`; can swap
  to `rmcp` SDK in a follow-up if version-pin compatibility holds.
  See `decisions/D-014_rift_mcp_v1_plan.md` for the locked design.
- `Category::Mcp` envelope variant added to `rift-bus` (additive — no
  schema-version bump per the additive-categories rule). Mirrored in
  `src/lib/bus.ts`.
- `src-tauri/src/mcp_host.rs` — in-process subscriber that listens for
  `mcp.request.*` envelopes, runs the handler, publishes
  `mcp.response.*`. Audit envelopes (`mcp.invoke`) published BEFORE
  every call so denied/panicking calls also log. Off by default —
  no subscription occurs unless `RiftConfig.mcp.enabled = true`.
- `RiftConfig.mcp` config section: `enabled`, `allow_inspection`,
  `allow_js_eval`, `allow_mutations` (all default `false`). Token at
  `<config_dir>/mcp_token` (chmod 600 on Unix) — sibling to `config.toml`,
  not a separate `~/.rift/` directory.
- Settings popout — new "MCP server" section: enable toggle, three
  capability sub-toggles, token reveal/copy/regenerate, token-path readout.
- Tauri commands: `mcp_status`, `mcp_token_get`, `mcp_token_regenerate`.
- Phase A tool catalog (4 tools): `bus_history`, `bus_tail` (streaming —
  Phase A.1), `git_status`, `aegis_state`.
- Translator-boundary check (`tools/check-translator-boundary.sh`)
  exempts `crates/rift-mcp/**/*.rs` — the crate is a translator by §9
  definition.
- CI: explicit `cargo build/test -p rift-mcp` step (10th gate);
  `RELEASING.md` pre-flight checklist updated to match.
- Claude Code plugin scaffold under `plugins/rift-mcp-plugin/` with
  `.mcp.json` registering `rift-mcp` as a stdio server, README +
  `/rift-status` example command.
- `getrandom = "0.2"` added as a workspace dep for token generation
  (CSPRNG: `/dev/urandom` on Unix, `BCryptGenRandom` on Windows).

---

## [0.1.0] — first packaged drop (Phase 9)

The first release that ships as a signed MSI / AppImage. Encompasses Phases
0 → 8 of `RIFT_V2_PHASE_PLAN.md` plus the Phase 8.7 polish/feedback bundle.
Everything below is what shipped between the Phase 0 scaffold and the v1
tag.

### Standalone terminal + cockpit (§1, §11)

- Tauri 2 host + Svelte 5 (runes) frontend + Rust workspace with four
  crates: `rift-bus` (protocol/transport/translators), `rift-core` (PTY
  abstraction), `rift-cli` (`rift hook` / `rift status` external entry),
  `rift-aegis` (private optional feature-gated path dep).
- xterm.js terminal with PortablePty PTY, attachCustomKeyEventHandler for
  Ctrl+C/V semantics that don't break SIGINT, font-load gating before
  `open()` to avoid the blank-terminal race.
- Right-side cockpit with IndexGraph (top) + Tree (bottom), splittered,
  amber/cockpit aesthetic locked to the `rift-v2-mockup.html` reference.
- Standalone GUI cockpit can detach into a separate window for multi-
  monitor use. Show/hide architecture (not destroy/recreate) so the
  detach gesture is instant after the first open.
- Three mockups all live: terminal alone, GUI alone detached, integrated.

### Visual system (§10.1 / §10.2 / §10.4)

- Color-coded lanes: amber-bright prompt, off-white user input, blue
  Claude voice, purple agents, cyan hooks, amber-primary Aegis, terminal
  green / red, faint-amber meta italic.
- Tag prefix system (CLAUDE / AGENT / HOOK / AEGIS / OK / WARN / ERR / SYS)
  with bordered uppercase boxes matched to lane colour.
- StatusLine: two rows, category-coloured segments, JetBrains Mono
  monospace throughout. DIR / GIT / REPO are live; CTX / SESSION / WEEK /
  MODEL render placeholders pending Claude Code's usage-payload hook
  (D-012, upstream-blocked).
- Scanlines + CRT vignette as brand fingerprint.
- Global scrollbar palette (amber-faint thumb / amber-dim hover) so the
  terminal aesthetic survives every scrollable surface.

### Tab / pane / pop-out architecture (§10.5–§10.10)

- `Tab` = persistent surface, `Pane` = split inside a tab or promoted
  notif, `Popout` = ephemeral overlay. Drag-tab-out promotes; drag-pane-
  back demotes. Same gesture (one level up) detaches the cockpit window.
- Drag-to-reorder for notif tabs with localStorage persistence
  (`rift.notifs.order`). Right-click hides a tab; the `⋯` manager popout
  is the discoverable path back.
- 4-section notif anatomy on every tab: status header / live activity
  strip / recent events log / persistent state panel.

### Notification tabs

- **errors** — `Category::System` envelopes (kind=`error`).
- **hooks** — `Category::Hook` envelopes from the rift-cli hook surface.
- **commands** — `Category::Pty` `command.submitted` envelopes.
- **aegis** — `AegisTabContent.svelte`. Subscribes to `aegis.context` +
  `aegis.invocation`. Quick-action buttons: open lessons / open settings.
  Capability-gated (lights up when rift-aegis startup probe publishes).
- **index** — `IndexTabContent.svelte` for vault telemetry. Capability-
  gated.
- **bus tail** — Phase 8.7i firehose view of every category for dev
  visibility. Pause / clear / per-category mute.
- **todo** — Phase 8.7i project-wide TODO/FIXME/XXX/HACK scraper backed
  by `todo_scan_command` (Rust). 1000-result cap, 1 MiB/file cap, depth
  16, honors fs_tree ignore-globs. localStorage-persisted dismissal with
  show-done toggle.
- **git** — Phase 8.7i `git_status_command` (porcelain v1 + last commit).
  Branch / ahead-behind / staged / modified / untracked / last commit,
  5s poll. Phase 8.7j added Fetch / Pull / Push / Commit actions; Commit
  opens an inline message form.
- **agents** — Phase 8.7k `AgentsTabContent.svelte` display layer for
  `Category::Agent`. Tracks live agents via `agent.start` / `.activity`
  / `.end` envelopes. × cancel publishes `agent.cancel` for translators
  to fulfill (§9 control endpoint pattern). Capability-gated empty
  state documents the protocol contract.

### Pop-outs (§10.5)

- Resizable + draggable card chrome with backdrop dismiss + Esc handling
  per top-most-only.
- Project Picker — Browse button (`@tauri-apps/plugin-dialog`) + manual
  text path entry.
- Notif Tab Manager — checkbox list with reset, makes the right-click
  toggle gesture discoverable per §10.7.
- Viewer — Shiki syntax highlighting, soft-wrap, edit/save round-trip
  via `fs_read_text` / `fs_write_text`. Edit-mode plain textarea
  intentional per §11 friction-reduction scope (CodeMirror highlighting
  deferred via D-017).

### Integration Decoupling (§9)

- Build-time enforcement: `tools/check-translator-boundary.sh` fails CI
  if external-system primitives (`tokio::net::`, `reqwest::`, `claude_*`,
  `mcp_*`) appear outside `crates/rift-bus/src/translators/`.
- Translators today: `commands.rs`, `errors.rs`, `fs.rs`, `index.rs`,
  `status.rs`, `vault_walker.rs` plus the private rift-aegis stubs.
- Capability classes: event subscription, control endpoints, data
  enrichment. Feature-detection at runtime — no "integration not found"
  errors; UI looks complete with whatever's there.

### Vault enrichment (Phase 8.6)

- vault-walker `repo:`-match enrichment: vaults whose `repo:` field
  canonicalizes to the active project root emit a `Category::Index /
  kind="enrichment"` envelope after their `vault.update`. Tree.svelte
  renders the indicator inline with the matching tree node.
- Telegraphic-frontmatter parser fixed in 8.5b after audit revealed the
  prior parser was a no-op on 100% of production vaults.

### Auto-update (§13 / D-013, closed C-018)

- `tauri-plugin-updater` v2 wired in Rust + capability + frontend.
- App `check()`s on session start; surfaces an update banner with
  Install / Later / Dismiss + error states.
- Updater bundle signed via `TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` GitHub Secrets.
- Endpoint: `releases/latest/download/latest.json` on the GitHub repo.

### Build / release infrastructure

- 9-gate CI matrix on `windows-latest` + `ubuntu-latest`: fmt, clippy,
  build, test, npm check, §9 boundary check, aegis-feature build,
  aegis-private-modules tests, aegis-feature clippy.
- `release.yml` triggers on `v*` tags, builds installers via
  `tauri-action@v0`, signs the updater bundle, uploads as a draft
  release with `latest.json`.
- Public-clone fresh-build path verified — minimal-stub + cfg-gated
  private modules pattern (D-011 / C-014).

### Known limitations / deferred to post-v1

- **D-010** — Sentinel itself (agent misbehavior detection). Agents tab
  is the display layer waiting on it.
- **D-012** — `CTX% / SESSION% / WEEK% / MODEL` placeholders. Waits on
  Claude Code's usage-payload hook.
- **D-014** — Rift MCP server.
- **D-015** — IndexGraph sub-door rendering.
- **D-016** — StatusLine `EFFORT` segment data wiring.
- **D-017** — Viewer edit-mode syntax highlighting via CodeMirror 6.

See `DEFERRED.md` for the full deferral log including unblocking events.

---

[0.1.0]: https://github.com/Critek-creator/Rift_TerminalV2/releases/tag/v0.1.0
