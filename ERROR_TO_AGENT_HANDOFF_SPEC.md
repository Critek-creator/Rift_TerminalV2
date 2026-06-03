# Error → Agent Handoff — Feature Spec

*Status: PROPOSED (candidate, not locked) · drafted 2026-06-03 · target: Rift v1.x post-release*
*Provenance: prompted by Microsoft's `intelligent-terminal` v0.1 (a Windows Terminal fork with native ACP-agent integration). Its "Error Detection" feature is the direct analogue. This spec adapts the idea to Rift's §9 architecture instead of copying it.*

> ⚠️ **PARTIALLY SUPERSEDED — see §9 "Adversarial Review Corrections (2026-06-03)" at the bottom.**
> A 4-agent deep review verified the foundations (§9 boundary, additive protocol) but **falsified** several infrastructure claims in §4/§7 (the `command_history` correlation path, the `llm_prompt` entry point, the `RIFT_EVENT_MIME` fix path, and "scrollback between CMD_START/CMD_END markers"). It also found **2 blockers**. Read §9 before implementing — §4/§7 below are the original draft and are wrong in the places §9 lists.

---

## 1. One-line

When a shell command exits non-zero, give the user a one-keystroke affordance that packages the failure context and hands it to whatever agent is present (Aegis, a Claude translator, or — bare install — a local grunt model via the router) for an explanation or a proposed fix.

## 2. Why this one

This is the highest effort-to-payoff item surfaced when comparing Rift against MS `intelligent-terminal` and zellij, because **the detection half already ships.** The competitor builds the whole pipeline; Rift only needs the last ~20% — the surfacing affordance, the context packaging, and a §9-clean invocation path. It also closes a known "started-but-not-closed" loop: Rift already *detects* command failure and *renders* it, but does nothing actionable with it.

## 3. What already exists (do not rebuild)

| Capability | Where | Note |
|---|---|---|
| Per-command exit code | `crates/rift-bus/src/translators/lane.rs` → `SentinelEvent::CmdEnd { exit_code }`, parsed from shell-prelude OSC (`extract_param(params, "exit")`) | `exit_code == 0` clears; non-zero is the failure signal |
| `command.completed` bus event | `src-tauri/src/lib.rs:~755` publishes `Envelope::new(Category::Pty, "command.completed")` with `exit_code` + duration on `cls.take_cmd_end()` | Already fires on EVERY command end |
| Per-command scrollback badge | shipped 2026-05-31 (exit/duration xterm decoration) | The badge is the natural anchor for the new affordance |
| Failed-command history | `src-tauri/src/command_history.rs:169` (`rec.exit_code.map(|c| c != 0)`) + `exit_code: Option<i32>` on the record | Command text + cwd already retained |
| §9 control-endpoint action registry | `action.declare` / `action.invoke` / `action.result` (capability-class-2), generic `ControlActions` surface, shipped 2026-05-31 | The §9-correct invocation mechanism — already built |
| Notif-event → terminal inject | `RIFT_EVENT_MIME` + active-terminal registry (drag/click), shipped 2026-05-31 | Context-injection plumbing exists |
| LLM router + `rift llm` gateway | `crates/rift-router`, `llm.route`/`llm.response`, grunt/partner tiers | Bare-install fallback path for "explain this error" with no agent present |
| Errors notif tab + clustering | `translators/errors.rs` (`Category::System`/`kind="error"`) + error-clustering toggle | NOTE: this tab is for Rift's *own* Tauri-command errors, **not** shell-command failures. Keep them distinct (see §6). |

## 4. The gap (what to build)

1. **Surface affordance** — when `command.completed` carries `exit_code != 0`, render a "fix" / "explain" control on the existing scrollback badge, and (optionally) a small lit indicator in the status line / a dedicated affordance. One click or one hotkey triggers the handoff.
2. **Context packaging** — assemble a structured failure context: `{ command, exit_code, cwd, duration_ms, scrollback_tail }` (last N lines of the failed command's output, bounded). Source = `command_history` record + ring-buffer scrollback. Pure Rift core; no external calls.
3. **§9-clean invocation** — emit `action.invoke` for a well-known action id (e.g. `error.explain` / `error.fix`) carrying the packaged context. **Rift core never calls Claude/Aegis directly** (CI-enforced by `tools/check-translator-boundary.sh`). Whoever declared the action handles it:
   - Aegis present → the private Aegis translator declares `error.*` and answers.
   - A Claude/ACP translator present → it declares and answers.
   - **Nothing declared** → Rift's own router translator answers with a local grunt model via the `rift llm` gateway, so bare Rift still does "explain this error." This is Rift's edge over `intelligent-terminal`, which *requires* an external ACP agent.
4. **Result rendering** — consume `action.result` and show the explanation/fix in a pop-out or the Agent/Errors surface, with an apply-the-fix path that reuses `RIFT_EVENT_MIME` inject (propose command → user confirms → injected into the active terminal; never auto-run without confirmation).

## 5. Configuration (mirror MS, but token-conservative defaults)

Three modes, default the safest:
- `off` — no affordance.
- `detect` *(default)* — light the affordance on failure; never spend tokens until the user clicks. (Default chosen deliberately: don't auto-burn tokens on every red exit code.)
- `assist` — on failure, auto-invoke `error.explain` (detection + explanation, still no auto-run of any fix).

Auto-running a proposed fix is explicitly **out of scope** — fixes are always propose-then-confirm.

## 6. Boundaries / non-goals

- Do **not** fold shell-command failures into the existing Errors tab (that tab = Rift's internal Tauri errors via `errors.rs`). Shell failures stay on the `Category::Pty` / `command.completed` path. Mixing them pollutes both surfaces.
- Do **not** add a direct LLM call anywhere outside `crates/rift-bus/src/translators/` — §9 boundary, CI-enforced.
- Do **not** auto-execute fixes.
- Not an ACP implementation. ACP-agent support is a separate, larger bet; this spec only needs the existing `action.*` registry, which any future ACP translator can satisfy.

## 7. Suggested build phases

- **P0 — Surface + package (core + frontend):** detect `exit_code != 0` from `command.completed`, render the badge affordance, package context. No agent call yet — wire the affordance to a stub `action.invoke`. Verifiable in isolation.
- **P1 — Router fallback translator:** implement the bare-install answer path (declare `error.explain`, route to local grunt via `rift llm`, render `action.result`). This makes the feature work with zero external integrations.
- **P2 — Config modes** (`off`/`detect`/`assist`) + status-line indicator wiring.
- **P3 — Propose-fix path:** `error.fix` returns a candidate command; render with confirm → inject via `RIFT_EVENT_MIME`.
- **P4 (deferred):** richer Aegis-side `error.*` handler (lives in the private translator, not here).

## 8. Open questions

- Scrollback tail bound — fixed line count vs byte cap vs "since last prompt"? (Lean: since the failed command's CMD_START marker, capped.)
- Should repeated identical failures cluster (reuse the error-clustering toggle's logic) before offering a handoff, to avoid affordance spam in a retry loop?
- Status-line real estate — is there a free segment, or does this live only on the badge + a transient toast?

---

*Drafted against p006 vault state v1.3.0 (2026-06-01). Symbols cited are real as of that commit; re-verify before implementing. Feed to `/idea-to-plan` or `/aegis --plan` to turn P0–P3 into an implementation plan.*

---

## 9. Adversarial Review Corrections (2026-06-03)

Four parallel red-team agents (opus×3 + sonnet×1) verified every empirical claim against the live source. Net verdict: **NEEDS-REVISION — do not build §7 P0 as written.** Foundations sound; affordance/capture/wiring layer needs a rebuild; 2 blockers + 1 privacy-critical fallback fix.

### Confirmed sound (keep)
- **§9 boundary holds.** All network I/O is isolated in `crates/rift-bus/src/translators/llm_*.rs`; `rift-router` and the command layer make no external calls (`rift-router/src/lib.rs:4-6` self-documents §9-compliance). A frontend provider calling an existing LLM route adds **zero** new Rust external-call sites → `check-translator-boundary.sh` stays green.
- **Protocol extension is purely additive.** Bus payload is untyped end-to-end (`unknown` in TS, opaque `serde_json::Value` in Rust); **no** Rust code parses the `action.*` protocol; `deny_unknown_fields` appears nowhere; versioning doctrine (`envelope.rs:6-11`, `correlation_id`/`parent_id` precedent) blesses optional additive fields. `params` on `action.invoke` + `proposed_command` on `action.result` ship clean, **no version bump**.
- **`command.completed` + session routing sound.** Payload `{session_id, exit_code, duration_ms}` (`lib.rs:755-769`); each `Terminal.svelte` self-filters by `session_id == paneId` (`Terminal.svelte:974-979`). (`duration_ms` can be null — handle as Option.)
- **P2 config serde-default protects old config files** (`config.rs` `#[serde(default)]` on `RiftConfig` + all sub-structs).

### Falsified claims (the §4/§7 draft is wrong here)
1. **`llm_prompt` is NOT a Tauri command** — it's an MCP host tool (`mcp_host.rs:439`). The frontend must call **`llm_complete`/`llm_stream`** (`lib.rs:2641-2642`).
2. **`command_history` correlation by `session_id` is a fiction.** `CommandRecord` has **no `session_id` field** and the history API is write-only append + aggregate stats — there is **no query-by-record** command (`command_history.rs:11-23`, Tauri cmds at `lib.rs:2637-2639`). Command text actually arrives on the **`command.submitted`** bus event; cwd from **`cwd.changed`** (`lib.rs:776`). → Need a **per-session frontend cache** (last submitted command + cwd), not a history lookup.
3. **"Scrollback between CMD_START and CMD_END markers" is unimplementable** — only a CMD_END marker is created (`Terminal.svelte:207`, `registerMarker(0)` at completion). There is no CMD_START marker. → Capture start-row (`buffer.active.baseY + cursorY`) at `command.submitted` time into the same cache; read back from there to the end marker. Fallback: "last N lines before end marker."
4. **`RIFT_EVENT_MIME` is not the fix-injection path** — it's a drag-MIME marker (`dragMime.ts:51`). Real injection is `term.paste(text + ' ')` (`Terminal.svelte:172-174`), newline-suppressed (so "never auto-run" holds) but with **no confirm/preview UI** — that must be built.
5. **Context packaging cannot live "in core" from the badge** — `addCommandBadge(exitCode, durationMs)` carries no command text/stderr (`Terminal.svelte:207`). Packaging is frontend, sourced from the caches above.

### 🔴 Blockers (fix before building)
- **B1 — `action_id`-keyed `pending`/`results` collide** (flagged independently by 2 reviewers). The shared registry keys state by `action_id` (`actionRegistry.svelte.ts:144-146`, `resultFor(actionId)`). Many concurrent/retried failures reusing one `rift.error.explain` id stomp each other — last writer wins. Fix: per-failure unique action ids (`rift.error.explain::<pane>::<seq>`) **+ `action.revoke`** after result, OR invocation-scoped result keying. Either touches code the existing `rift.llm.reset-ledger` consumer depends on → regression-test it.
- **B2 — hard dependency on `lanes_enabled = true`.** `command.completed` is only emitted when the shell prelude injects CMD_END sentinels, which only happens when `lanes_enabled` (prelude header `pwsh.ps1:4`). With lanes off, the whole feature is silently inert regardless of `error_handoff.mode`. Fix: gate/disable the Settings toggle when lanes are off + surface the dependency.

### ⚠️ Privacy-critical: grunt fallback can leak off-machine
Auto-routing **excludes non-resident local models** (`rift-router/src/lib.rs:72-88,164,188-189`): with no grunt model loaded, `llm_complete` either errors (`AllModelsUnavailable`) or **escalates the fallback chain to a cloud model** — sending the shell command + stderr + cwd to Anthropic/Google, defeating the "free/private local" premise. Fix: pin grunt-tier **local-only**; on "no model loaded" → **degrade gracefully (show raw error / offer to load a model), never escalate to cloud.**

### Other required revisions
- **P0 "stub invoke" is not verifiable in isolation** — a stub that never publishes `action.result` leaves the action `pending` **forever** (no timeout in the registry). → **Merge P0+P1** (or have the stub echo an immediate `action.result`).
- **No badge/pop-out result surface exists** — `action.result` only renders inside `ControlActions.svelte` (two tab mounts). A dedicated badge-anchored render surface must be built; it can read `resultFor()` from anywhere.
- **Chicken-and-egg invoke** — `actionRegistry.invoke()` needs a pre-declared `DeclaredAction`; there is no `invokeById`. Declare-first (reactively on failure) or add an accessor.
- **Retry-loop spam + no dismiss** — `addCommandBadge` is unconditional, no dedup/clustering, and the registry has no per-result dismiss. Cluster identical consecutive failures; add an acknowledge path. (Clustering is a P0 design concern, not deferrable.)
- **Accessibility** — the badge is a non-interactive `<span>` with `pointer-events: none`, and the decoration auto-disposes when the line scrolls out of scrollback (ephemeral target). Making it a control needs role/aria/keyboard-focus + a decision on the scroll-out-vanish behavior.

### Open decisions (Garth's call — see chat)
- **D1 — where does "explain" logic live?** Frontend provider in shippable Rift (bare Rift can explain via local model) vs integration-only (Aegis/translator declares `error.*`; bare Rift only surfaces the affordance + hands off). §9's two-document split arguably wants agent logic on the integration side.
- **D2 — fallback policy:** local-only-or-degrade (recommended; privacy) vs allow cloud escalation.

*Review artifacts: 4 agent reports, 2026-06-03. Revised phase plan in §10 supersedes §7.*

---

## 10. Revised Phase Plan (supersedes §7) — decisions locked 2026-06-03

**D1 = frontend provider in shippable Rift** (bare Rift explains via a local model). **D2 = grunt-tier local-only; degrade, never escalate to cloud.**

### R0 — Capture foundation *(frontend only, no Rust, no UI)*
Build the context source the original plan assumed existed.
- In `Terminal.svelte`, add a per-session cache: on `command.submitted` store `{ command, cwd (latest cwd.changed/effectiveCwd), startRow = buffer.active.baseY + cursorY, ts }` (bounded ring).
- On `command.completed`, pair `exit_code`/`duration_ms` with the cached entry → `FailureContext { command, cwd, exit_code, duration_ms, startRow, endRow = marker.line }`.
- Scrollback tail: read `startRow..endRow` from `buffer.active` (cap ~200 lines / N KB).
- **Verify in isolation:** assert assembled `FailureContext` is correct via a debug log / `bus_history`. No agent, no button yet. *(This is the genuinely-isolatable P0, unlike the old stub-invoke.)*

### R1 — Affordance + provider + result surface *(merged — old P0+P1)*
- **Affordance:** when `exit_code != 0` && `mode != off`, make the CMD_END badge interactive — `role=button`, `aria-label`, keyboard focus, `pointer-events` on. Decide scroll-out-vanish behavior. **Cluster identical consecutive failures** (dedup by command+exit within a window) to kill retry-loop spam.
- **Registry (fixes B1):** per-failure unique action id `rift.error.explain::<pane>::<seq>` — `action.declare` on failure, `actionRegistry.invoke(action, params=FailureContext)`, `action.revoke` after result. Avoids re-architecting the shared `action_id`-keyed state; **regression-test `rift.llm.reset-ledger` is unaffected.**
- **Provider (D1):** new frontend provider fulfills the action — builds a prompt from `params`, calls **`llm_complete`** (not `llm_prompt`) with an explicit **local/grunt `model_id` pin**. **D2:** if no local model resident → `action.result {status:'error', message:'no local model loaded — raw error shown; load a model to explain'}` + offer to load; **never** route to cloud.
- **Result surface:** badge-anchored pop-out reading the invocation's result; **dismiss/acknowledge** control. (No surface exists today — `ControlActions` is tab-bound.)
- Merging means the provider always answers (even to degrade) → no stuck `pending`.

### R2 — Config modes
- `error_handoff: { mode: off|detect|assist }` on `RiftConfig` (`#[serde(default)]`; mode enum with `#[serde(other)] Unknown` → `detect`).
- `rift_config_set`: **3 touch-points** — new `error_handoff_mode` arm + the "recognized fields" error string + the MCP tool JSONSchema.
- `SettingsPanel` toggle via `config_save` + `broadcastConfigChanged()` (window event); badge logic reads `mode` live on **both** `rift:config-changed` (window) **and** bus `config.changed` (MCP-driven).
- **Fixes B2:** disable the toggle + show a dependency note when `lanes_enabled` is off.
- `assist` = auto-invoke explain on failure (still no fix auto-run).

### R3 — Propose-then-confirm fix
- `rift.error.fix::<pane>::<seq>` action; `action.result` gains optional `proposed_command` (additive, confirmed safe).
- **Build a confirm/preview UI** (none exists). On confirm → `term.paste(command)` **without newline** (existing injector satisfies "never auto-run"). `RIFT_EVENT_MIME` is *not* the mechanism.
- Fix generation local-only, same degrade rule as D2.

### R4 — Deferred
- Richer Aegis-side `error.*` handler lives in the **private** rift-aegis translator (out of this repo). When present, the tiered precedence (integration answers, else local) can be added — but D1 ships the local path first.

### Verification (every phase)
`cargo fmt --check` · `cargo clippy --workspace -- -D warnings` · `cargo test --workspace` · `npm run check` · `vitest` · `bash tools/check-translator-boundary.sh` · live: failing command with lanes **on** → affordance + correct `FailureContext` (`bus_history`); **no-model-loaded → degrade, confirm nothing leaves the box**; R3 → confirm-before-run.

### Revised file map
- `src/lib/Terminal.svelte` — capture cache, interactive a11y badge, config-reactive gating, result-pop-out mount (R0/R1/R2)
- `src/lib/errorHandoff.ts` *(new)* — `FailureContext` assembly + scrollback read (R0)
- `src/lib/errorHandoffProvider.svelte.ts` *(new)* — per-failure declare/invoke/revoke + `llm_complete` local-pin + degrade (R1/R3)
- `src/lib/actionRegistry.svelte.ts` — optional `params` arg on `invoke()` (additive) (R1)
- `src/lib/ErrorResultPopout.svelte` *(new)* — badge-anchored result + dismiss (R1)
- `crates/rift-bus/src/config.rs` — `error_handoff` section (R2)
- `src-tauri/src/mcp_host.rs` — `rift_config_set` arm + JSONSchema (R2)
- `src/lib/SettingsPanel.svelte` — toggle + `lanes_enabled` gate (R2)

*Effort read: larger than the original "last 20%." R0 small; R1 substantial (registry wiring + provider + new pop-out + a11y); R2 medium; R3 medium. Detection is real; capture + affordance + result-surface are mostly greenfield.*
