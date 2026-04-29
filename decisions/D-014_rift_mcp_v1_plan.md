# D-014 — Rift MCP server (v1.x design plan) — **LOCKED 2026-04-29**

*Plan opened 2026-04-29 in response to user `/aegis --full --plan` request.
Companion to DEFERRED.md D-014. **Locked 2026-04-29** with the v1.1 answers
captured in §11 below. Phase A is now the active build.*

---

## 1. Goal

Expose Rift's running state and control surface as a standard
[Model Context Protocol](https://modelcontextprotocol.io/) server so that
external Claude Code sessions, MCP-aware IDEs, and automation harnesses can
**observe**, **introspect**, and **drive** a live Rift instance through the
same protocol Rift already uses to talk to the rest of its own ecosystem.

The catalysing event was the 2026-04-29 Phase 8.7 BV-regression: roughly
**10% of weekly tokens** were spent guessing at IndexGraph + secondary-window
issues that a programmatic-control surface would have collapsed to a
single Playwright-style verification. The MCP surface is meant to make
that class of debug loop trivial — for the user *and* for Aegis itself.

### Non-goals (v1)

- **Replace Tauri's webview internals.** MCP tools wrap existing surfaces;
  they do not bypass them.
- **Be the editor automation surface for arbitrary projects.** The scope is
  Rift itself — its bus, its UI, its PTY sessions, its config.
- **Public-internet accessible.** localhost-only, opt-in, off by default.
- **Replace the IpcServer.** rift-cli's hook publishing path stays intact.

---

## 2. Architecture

### 2a. Process model

**Standalone binary `crates/rift-mcp/`** (mirrors `crates/rift-cli/` exactly).

| Choice                        | Pros                                                      | Cons                                       |
|-------------------------------|-----------------------------------------------------------|--------------------------------------------|
| **Standalone binary** *(rec)* | Clean §9 boundary; ships independently; no Tauri lifecycle entanglement; can be packaged + distributed separately for headless deployments. | Adds a binary to the workspace; requires the Rift host to be running for any tool to succeed. |
| In-process Tauri module       | Direct access to webview / windows / PTY registry         | Violates §9 spirit; entangles MCP lifetime with the GUI; harder to test in isolation. |
| Sidecar process spawned by Rift | Lifetime managed; easy install                           | Just option 1 with extra spawning glue — no real win. |

**Recommendation:** standalone binary. The host process manages its own
lifetime; the MCP binary connects to the host via the existing
`IpcServer` socket (named pipe on Windows, UDS on Unix), exactly the way
`rift-cli` already does.

This means: every MCP tool that observes or mutates state does so via
**bus envelopes** — never by reaching into Rift internals. §9 holds.

### 2b. Transport (MCP-side)

**Stdio JSON-RPC** is the MCP standard. Ship that first.

`rift-mcp` listens on stdin (newline-delimited JSON-RPC requests) and
writes responses + notifications to stdout. Errors and tracing go to stderr.
This matches every MCP client today (Claude Code's `mcpServers` block,
Anthropic's MCP inspector, etc.) without adapter code.

A WebSocket transport (for browser-side automation like
`claude-in-chrome`) is **deferred** — adding it later is additive and the
stdio path already covers Claude Code's automation case.

### 2c. MCP protocol implementation

**Decision (locked 2026-04-29 after marketplace survey):** use
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) — Anthropic's
official Rust SDK — directly. Optionally crib boilerplate from
[`mcp-rs-template`](https://github.com/linux-china/mcp-rs-template), a
community Rust scaffold (not a Claude Code plugin — just a git template
with a pre-wired `Cargo.toml` + `main.rs` + `tool!` macro setup).

**Why not the `mcp-server-dev` plugin path?** Anthropic's official
`mcp-server-dev` Claude Code plugin (installed locally, invocable via
`/mcp-server-dev:build-mcp-server`) is **TypeScript-biased**. Its
five-phase interrogation skill is excellent for deciding the
*deployment model* and *tool-design pattern*, but if you pick Rust as
the implementation language it hands you off to the raw `rmcp` docs
without code generation. Worth running the skill once for the design
checklist (it surfaces auth/elicitation/widget choices we'd otherwise
miss), then implementing in Rust by hand.

**What the survey found:**

| Tool                      | Use for                                     | Verdict |
|---------------------------|---------------------------------------------|---------|
| `mcp-server-dev` plugin   | Pre-build interrogation: deployment model, tool pattern, auth flow, UI choice | Run it once for the design checklist; ignore its code-output path |
| `rmcp` SDK                | Cargo dep; `#[tool]` proc-macros; stdio + HTTP transport | **Primary** |
| `mcp-rs-template` (community) | Boilerplate `Cargo.toml`, `main.rs`, tool macro skeleton | Reference / cherry-pick — not a dependency |
| `mcp-builder`, `mcp-anything` (community plugins) | Various design/generation utilities | Unverified relevance to Rust+stdio path; skip for v1 |

**Fallback:** if `rmcp` has compatibility issues with our `tokio` /
`serde_json` versions (the workspace pins `tokio = "1"`, `serde_json = "1"`),
hand-roll over `tokio::io::{stdin, stdout}` + `serde_json` line-delimited
messages. Roughly 200 LOC. The MCP wire format is plain JSON-RPC 2.0;
the schema is documented at [modelcontextprotocol.io/spec](https://modelcontextprotocol.io/specification).

### 2d. Host-side: how Rift fulfills MCP tool calls

For each MCP tool the user invokes, `rift-mcp` translates:

```
client → MCP json-rpc → rift-mcp → bus envelope (publish) → Rift host
                                                                ↓
client ← MCP json-rpc ← rift-mcp ← bus envelope (subscribe) ← (response)
```

The host needs new **request/response envelope kinds** for tools that have
no existing analog (DOM snapshot, screenshot, JS eval). Two convention
options:

1. **Single category `Category::Mcp`** — every MCP-driven request/response
   lives here, kinds `mcp.request.*` and `mcp.response.*`.
2. **Reuse existing categories** — DOM snapshot under `Category::System`,
   PTY input under `Category::Pty`, etc.

**Recommendation:** option 1 — new dedicated `Category::Mcp` variant. Keeps
the surface explicit so audit logs (`bus tail`, `errors` tab) can filter
MCP traffic at a glance, and makes the "off by default" guarantee
trivially verifiable (no Mcp envelopes in flight = MCP is off).

The host gets a new in-process subscriber that listens for
`Category::Mcp / kind="mcp.request.*"` envelopes, runs them against the
appropriate Tauri-internal API, and replies with `mcp.response.*` carrying
the same `request_id`. This subscriber lives in `src-tauri/src/mcp_host.rs`.

### 2e. Authentication

- **Token model.** On first launch with MCP enabled, Rift generates a
  random per-install token, displays it in the Settings popout, and
  serialises it to `~/.rift/mcp_token` (chmod 600). MCP clients pass the
  token via env var `RIFT_MCP_TOKEN` or a `--token` CLI flag.
- **Token check.** `rift-mcp` includes the token in its first envelope
  to the host (`Category::Mcp / kind="mcp.handshake"`). Host verifies
  against the saved token and either accepts or closes the IPC connection.
- **Audit log.** Every MCP tool invocation publishes a
  `Category::Mcp / kind="mcp.invoke"` envelope BEFORE running the tool
  (so denied calls are also logged). Audit is a first-class observable
  surface, not a debug log.
- **Off by default.** `RiftConfig.mcp.enabled = false`. Settings popout
  flips it. Until enabled, the host doesn't subscribe to `Category::Mcp`
  at all and `rift-mcp` exits early if the IPC handshake fails.

### 2f. §9 boundary preserved

The MCP server is, structurally, a **translator**. It speaks MCP outside,
speaks bus envelopes inside. The translator-boundary check
(`tools/check-translator-boundary.sh`) gets a new exemption for
`crates/rift-mcp/`, the same way `crates/rift-bus/src/translators/` is
exempted today. Hand-rolled JSON-RPC vs `rmcp` makes no difference to the
boundary check — only the destination of network/IPC primitives matters.

---

## 3. Tool surface

Tools split into three risk tiers. Each tier gets its own settings toggle
so power users can ramp gradually.

### Tier 1 — Read-only (default-on once MCP is enabled)

| Tool                  | Wraps                                       | Effect |
|-----------------------|---------------------------------------------|--------|
| `bus_history`         | `RiftBus::subscribe(SubscribeFilter)` replay | Returns recent envelopes (paginated). |
| `bus_tail`            | `RiftBus::subscribe` (live)                  | Streams envelopes as MCP notifications. Long-running. |
| `git_status`          | `git_status_command`                         | Same payload as the Git tab. |
| `fs_read`             | `fs_read_text`                               | Read-only file fetch. |
| `fs_tree`             | `fs_tree`                                    | Static project tree snapshot. |
| `aegis_state`         | Subscribed `aegis.session.skill_loaded` snapshot | Last known skill version + lessons-file path. |
| `notif_tabs`          | App.svelte notifs derivation                 | Returns the visible notif-tab catalog. |
| `pty_list`            | PtyRegistry read                             | Active session ids, dimensions, status. |
| `cockpit_state`       | `cockpit_status` + window position           | Detached / docked + saved coords. |
| `todo_scan`           | `todo_scan_command`                          | Same payload as the TODO tab. |

These wrap existing Tauri commands or bus-replay mechanics. Marginal new
code per tool: ~20 LOC each (envelope round-trip wrapper + MCP tool
declaration).

### Tier 2 — DOM / screenshot / JS eval (default-off)

| Tool                  | Effect | Risk |
|-----------------------|--------|------|
| `dom_snapshot(window?)` | Returns the accessibility tree of the main or cockpit-detached window. | Low — read-only. Used for "what is the user seeing". |
| `screenshot(window?)`   | Returns a PNG of the named window. | Low — could leak sensitive on-screen text but disclosure only to the localhost client that already authed. |
| `js_eval(window?, code)` | Evaluates JS in the named webview, returns the value. | **High** — full UI access. Needs explicit per-call confirmation? Or per-session toggle. |

These need new in-process Tauri APIs (no existing wrapper). The host-side
subscriber for `Category::Mcp` runs them via Tauri's
`WebviewWindow::eval` and a screenshot helper using
`webview.with_webview` on Windows / `Webkit2GTK` snapshot on Linux.

**Decision needed on `js_eval`:** ship in v1.0 (gated behind a separate
toggle) or defer to v1.1 — see Open Questions §6.

### Tier 3 — Mutating tools (default-off, additional confirm)

| Tool                  | Wraps                  | Risk |
|-----------------------|-------------------------|------|
| `bus_publish`         | `bus_publish` Tauri cmd | Low — already a documented capability of any in-process script. |
| `pty_input(id, bytes)`| `pty_write`             | High — types into the user's terminal. |
| `simulate_click`      | New Tauri webview API   | High. |
| `simulate_drag`       | New Tauri webview API   | High. |
| `fs_write`            | `fs_write_text`         | High — modifies user files. |
| `git_action`          | `git_action_command`    | High — fetch/pull/push/commit. |

These tools must require **two affirmative steps**:
1. MCP enabled in Settings.
2. "Allow mutating tools" toggle explicitly flipped (separate from MCP
   enabled).

A future v1.x can add a per-call confirmation popout (`agent.cancel`-style
dispatch — Rift surfaces a popout, MCP call blocks until user clicks
Allow/Deny). Skip in v1.0.

---

## 4. Phasing

Each phase ships independently. Earlier phases are valuable on their own.

### Phase A — Scaffold + auth + bus-read tools

- New `crates/rift-mcp/` crate with `rmcp` dependency.
- `mcp_host.rs` in `src-tauri/` subscribing to `Category::Mcp / mcp.request.*`.
- Token generation + storage in `RiftConfig.mcp`.
- Settings popout adds an MCP section: enable toggle, token display,
  audit log link.
- New `Category::Mcp` variant added to the `Category` enum (additive —
  bus.ts mirrors).
- Boundary-check exemption for `crates/rift-mcp/`.
- Tier 1 tools: `bus_history`, `bus_tail`, `git_status`, `aegis_state`.
- E2E test: `rift-mcp` connects, fetches `bus_history`, receives at
  least one envelope.

**Estimated effort:** medium. ~1-2 days.

### Phase B — Tier 1 completion

- `fs_read`, `fs_tree`, `notif_tabs`, `pty_list`, `cockpit_state`, `todo_scan`.
- All read-only; each is a 20-30 LOC wrapper over an existing command.

**Estimated effort:** small. ~half day.

### Phase C — DOM snapshot + screenshot

- `dom_snapshot(window?)` via Tauri's accessibility APIs.
- `screenshot(window?)` via platform-specific webview snapshot.
- Defaults off; settings flag `RiftConfig.mcp.allow_inspection = false`.

**Estimated effort:** medium. Platform-specific code per OS. ~1-2 days.

### Phase D — Mutating tools

- `bus_publish`, `pty_input`, `fs_write`, `git_action`.
- Defaults off; settings flag `RiftConfig.mcp.allow_mutations = false`.
- Audit envelope on every invocation.

**Estimated effort:** small (existing wrappers) + thoughtful UX for the
audit surface. ~half day plus settings work.

### Phase E — JS eval + simulated input + per-call confirm

- `js_eval`, `simulate_click`, `simulate_drag`.
- Per-call confirmation popout (optional — toggle via settings).
- Audit log surface as a notif tab? Or just lean on bus tail filtered
  to `Category::Mcp`?

**Estimated effort:** medium. Per-call confirm UX is the bulk of it.

### Phase F (optional) — WebSocket transport

- Add WS listener inside `rift-mcp` for browser-based clients
  (`claude-in-chrome`, web automation harnesses).
- Same auth model; same tool surface.

**Estimated effort:** small. ~half day.

---

## 5. Risks and mitigations

| Risk                                                         | Severity | Mitigation                              |
|--------------------------------------------------------------|----------|-----------------------------------------|
| Token leakage exposes full UI/PTY control                    | High     | Localhost-only socket; chmod 600 on token file; rotation button in Settings; audit log. |
| Tool churn breaks downstream automation                      | Med      | Version tools individually; never remove without deprecation cycle; tool capability list returned by `initialize` is the contract. |
| `rmcp` SDK version churn vs our tokio/serde versions         | Low      | Vendor or fork if pinning becomes painful; fallback to hand-rolled JSON-RPC. |
| MCP host subscriber slow under heavy bus traffic blocks tools | Med      | Use bounded channels with backpressure (same pattern as `IpcServer` per-connection); drop oldest mcp.request.* on overflow with an error response. |
| Surface enables Aegis to drive Rift, which then loops back   | High     | Audit log makes loops detectable; per-tool invocation cap; user-facing kill switch in Settings popout. |
| Performance: long-poll subscribe holds a tokio task per client | Low      | Acceptable; clients are 1-2 in practice. Bound max concurrent subscriptions (16). |
| Cross-platform webview screenshot is fiddly                  | Med      | Phase C is its own effort; defer to v1.1 if Phase A-B ship cleanly first. |

---

## 6. Open questions

These need user signoff before Phase A starts.

**Recommended pre-signoff step:** run `/mcp-server-dev:build-mcp-server`
once with Rift's case (local Tauri app, ~10-15 tools tier 1, token auth,
maybe widget confirms in v1.x). The skill's deployment-model
recommendation + tool-design pattern call is worth comparing against
this doc's recommendations before locking. Treat its output as a second
opinion, not a replacement — it doesn't know about §9 boundary
discipline or our existing IpcServer.


1. **Standalone `rift-mcp` binary or in-process Tauri module?** Recommended
   standalone for §9 cleanliness, but it adds a binary to the workspace
   and needs to be packaged. Acceptable tradeoff?

2. **Stdio first, WS later** — confirm? Or do you want WS in v1 because
   the browser-automation case (`claude-in-chrome`) is the bigger driver
   for you?

3. **`js_eval` in v1.0 or v1.1?** Phase C ships DOM + screenshot; `js_eval`
   could ride along (gated by a separate toggle) or wait until v1.1.

4. **Per-call confirmation popouts (Phase E) or just audit-after-the-fact?**
   Per-call is high-friction but high-safety. Audit-only is the opposite.
   For agent-automation runs, audit-only is friendlier; for "Aegis driving
   Rift unattended" audit-only is scarier.

5. **Token storage path** — `~/.rift/mcp_token` (new dir) or under
   the existing `~/.config/rift/` (Linux) / `%APPDATA%\rift\` (Win) path
   that `RiftConfig` already uses? Recommend the latter for one-place
   config hygiene.

6. **MCP server name / branding.** `rift-mcp` is the obvious binary name.
   The MCP `serverInfo.name` field can be "Rift" or "Rift Terminal".
   Cosmetic, but it's what shows up in Claude Code's connection list.

7. **Initial tool catalog freeze.** Should Tier 1 ship with everything
   listed in §3, or pare to 3-4 (`bus_history`, `bus_tail`, `git_status`,
   `aegis_state`) to lock the protocol shape before adding more?

---

## 7. What I'd recommend

If you greenlight, the smallest interesting MVP is **Phase A only**:
scaffold + auth + 4 read-only tools (`bus_history`, `bus_tail`,
`git_status`, `aegis_state`). That's ~1-2 days of work and gives you a
working MCP server you can register with Claude Code and immediately use
to introspect a running Rift session. Phase B-F can ship in any order
after that based on what proves most useful in practice.

The biggest open call is whether to ship Phase E's per-call confirms.
Recommend **no** for v1.0 — start with audit-only, add confirms later if
the audit log shows usage patterns you want to gate. Friction is the
killer of automation.

---

## 8. Build / test strategy

- **Unit tests** in `crates/rift-mcp` for tool argument parsing + envelope
  round-trip shape.
- **Integration test** that spawns a Rift host (headless mode if we add
  one, otherwise the existing `tauri::test::mock_app` shape) + spawns
  `rift-mcp`, then drives an `initialize` + `tools/list` + a couple of
  Tier 1 tool calls over stdio. Asserts the JSON-RPC responses.
- **Manual verification**: register `rift-mcp` in Claude Code's
  `mcpServers` config, run a real Claude Code session against a running
  Rift, confirm the tools appear in the model's available tools list.
- **CI gate addition**: extend the 9-gate matrix with a 10th: `cargo
  build -p rift-mcp --locked` + `cargo test -p rift-mcp --locked`.
- **Boundary check addition**: `crates/rift-mcp/` exempted alongside
  `crates/rift-bus/src/translators/` from the no-direct-external-IO rule.

---

## 9. What lands in the repo for each phase

Phase A artifacts:
- `crates/rift-mcp/{Cargo.toml,src/main.rs,src/lib.rs,src/tools/{mod.rs,bus.rs,git.rs,aegis.rs}}`
- `src-tauri/src/mcp_host.rs` + lib.rs `.invoke_handler(...)` wiring
- `crates/rift-bus/src/envelope.rs` — `Category::Mcp` variant + tests
- `src/lib/bus.ts` — mirror the new category
- `src/lib/SettingsPanel.svelte` — new `Mcp` section (toggle, token,
  copy-to-clipboard button)
- `RiftConfig.mcp.{enabled, allow_inspection, allow_mutations, token_path}`
- `tools/check-translator-boundary.sh` — exemption for `crates/rift-mcp/`
- `decisions/D-014_rift_mcp_v1_plan.md` — this doc, updated with locked
  decisions after user signoff
- `RELEASING.md` §5 pre-flight gets `cargo build -p rift-mcp` added
- `CHANGELOG.md` Unreleased section grows MCP-related entries
- `DEFERRED.md` D-014 closes via a new C-021 entry citing this plan as
  the canonical decision doc

The plan itself stays in the repo even after D-014 closes — it's the
audit trail for why the v1 surface looks the way it does.

---

## 10. Status

**LOCKED 2026-04-29.** All 7 open questions in §6 answered in §11 below.
Phase A is the active build (scaffold + auth + 4 read-only Tier 1 tools).

---

## 11. Locked decisions (v1.1 — 2026-04-29)

User signoff on the 7 open questions from §6:

1. **Process model: standalone `rift-mcp` binary.** New `crates/rift-mcp/`
   crate added to the workspace. Connects to the running Rift host via the
   existing `IpcServer` socket (named pipe on Windows, UDS on Unix), same
   as `rift-cli`. §9 boundary preserved — `rift-mcp` is a translator, no
   webview reach-in.

2. **Transport: stdio first, WebSocket in v1.x (Phase F).** Phase A ships
   stdio JSON-RPC only. WebSocket transport is Phase F (additive, same
   auth, same tool surface). Browser-automation cases (`claude-in-chrome`)
   are served by Phase F when it lands.

3. **`js_eval` ships in v1.0 (Phase C).** Gated behind a separate settings
   toggle (`RiftConfig.mcp.allow_js_eval`, default `false`) alongside
   `allow_inspection` for DOM/screenshot. User accepts the risk profile
   ("audit-only is fine"). Phase C now covers DOM + screenshot + js_eval
   together.

4. **Audit-only — no per-call confirmation popouts.** Every MCP tool
   invocation publishes `Category::Mcp / kind="mcp.invoke"` BEFORE running
   (so denied calls also log). The audit log is the safety surface.
   Per-call confirms are NOT shipped in v1.x — friction kills automation.
   (Phase E reduces to "js_eval + simulated input + audit-log notif tab".)

5. **Token storage path: existing `RiftConfig` directory.** Token is a
   sibling file alongside `config.toml`, NOT a new `~/.rift/` directory:

   - Windows: `%APPDATA%\com.abyssal.rift\config\mcp_token`
   - macOS:   `~/Library/Application Support/com.abyssal.rift/mcp_token`
   - Linux:   `$XDG_CONFIG_HOME/rift/mcp_token` (or `~/.config/rift/mcp_token`)

   Resolved via the same `directories::ProjectDirs::from("com", "abyssal",
   "rift")` call that `rift-bus::config` already uses. One-place hygiene.
   Permissions: `chmod 600` on Unix; default ACL inherits on Windows
   (no extra restriction beyond the per-user `%APPDATA%` directory).

6. **Branding.** Binary name: `rift-mcp`. MCP `serverInfo.name`: `Rift`.
   Cosmetic but pins it.

7. **Initial tool catalog frozen at 4 read-only tools.** Phase A ships
   exactly these to lock the protocol shape before adding more:

   - `bus_history` — paginated replay of recent envelopes
   - `bus_tail` — live envelope stream as MCP notifications
   - `git_status` — Git tab payload
   - `aegis_state` — last `aegis.session.skill_loaded` snapshot

   Phase B fills in the remaining Tier 1 tools (`fs_read`, `fs_tree`,
   `notif_tabs`, `pty_list`, `cockpit_state`, `todo_scan`) once the
   protocol shape has held under real usage.

### Phase reshuffle from §4 (post-lock)

Phase plan is unchanged structurally; the v1.1 lock just means:

- **Phase A**: 4 tools (was: 4 — same).
- **Phase C**: now bundles DOM + screenshot + `js_eval` (was: DOM +
  screenshot only; `js_eval` was Phase E).
- **Phase E**: now covers simulated input + audit-log notif tab UX (was:
  `js_eval` + simulated input + per-call confirms — confirms dropped).
- **Phase F**: WebSocket transport (unchanged).

### Settings toggles (locked names)

```toml
[mcp]
enabled = false           # master switch
allow_inspection = false  # DOM + screenshot
allow_js_eval = false     # JS eval (separate from inspection)
allow_mutations = false   # bus_publish, pty_input, fs_write, git_action
                          # (Phase D)
# token_path is computed from the platform config dir; not user-editable
```

`allow_js_eval` is a NEW toggle relative to the original §3 schema —
splits the v1 inspection surface into "read-only inspection" (DOM,
screenshot) vs. "JS execution" (`js_eval`) so users can opt into the
former without the latter.
