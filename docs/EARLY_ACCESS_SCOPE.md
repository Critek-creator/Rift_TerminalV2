# Rift Terminal V2 — Early Access Scope

*Generated: 2026-05-21 | Target: $29 one-time early access | Platform: Windows (MSI)*

---

## 1. Current Capability Inventory (what's already built)

Rift V2 is at v0.1.10 with a significant amount of shipping code:

### Terminal Core (fully functional)
- PTY via portable-pty — cmd/powershell/bash, keyboard, resize, exit handling
- xterm.js terminal surface with ANSI 16-color palette
- Color-coded output lanes (CLAUDE/AGENT/HOOK/AEGIS/OK/WARN/ERR/SYS tags)
- Terminal search (Ctrl+Shift+F, SearchAddon)
- CRT aesthetic: scanlines, vignette, amber-on-black, JetBrains Mono
- Status line (2 rows): DIR, MODEL, CTX%, SESSION%, SKILL, GIT, REPO, EFFORT

### GUI Cockpit (functional)
- Tab/Pane/Pop-out architecture with drag-to-promote and detachable windows
- Filesystem tree with live activity visualization (glow-on-touch, decay, pin)
- Hierarchical bubble-up for directory activity
- Detachable cockpit window for multi-monitor
- In-cockpit Shiki syntax-highlighted file viewer (quick edit/save)
- Drag-node-into-terminal context injection
- Project swap menu

### Integration Layer (fully wired)
- §9 Integration Decoupling Protocol — CI-enforced translator boundary
- 4 Rust crates: rift-bus (protocol/transport), rift-cli, rift-core (PTY), rift-mcp
- 20 MCP tools (bus_history, bus_tail, git_status, aegis_state, pty_input, pty_read, fs_read, fs_tree, fs_write, todo_scan, etc.)
- Hooks tab — live Claude Code hook events
- Aegis tab — aegis.log live tail, session snapshots, quick-action buttons
- Agents tab — violation rendering from Sentinel translator
- Index tab — vault browser list (Abyssal Index integration)
- Errors/Commands/FS/MCP notification tabs
- Session replay viewer (bus event .jsonl playback)
- Bus event sparklines (60s circular buffer visualization)
- Live lane classification (OSC 6973 parser + shell preludes)

### Developer Experience
- Command palette (Ctrl+K)
- Keyboard shortcut overlay (Ctrl+?)
- Tab workspace persistence (localStorage)
- Notification filter rules engine (per-tab severity thresholds)
- Session event persistence (.jsonl logging)
- Settings panel with config-driven hot-reload
- Resizable promoted notification pane
- File tree color coding by extension type
- Cross-component hover highlighting
- Bookmarks panel

### Infrastructure (ready for release)
- CI pipeline: fmt + clippy + build + test + svelte-check + translator-boundary
- GitHub Actions release workflow (release.yml) — triggers on tag push
- Tauri auto-updater wired (tauri-plugin-updater, signing keypair generated)
- RELEASING.md runbook documenting full process
- Unsigned MSI supported (code-signing optional)
- License key validation API already live on abyssal-arts.com
- Stripe checkout already wired on the website for Rift purchase

---

## 2. Minimum Shippable Beta Feature Set

The core question: **what would make a Claude Code user pay $29?**

### MUST HAVE (already built — verify stability)
- **Reliable terminal** — PTY start/write/resize/kill works without crashes
- **Lane-classified output** — the visual differentiation of Claude/Agent/Hook/Error output is the #1 value prop. This is what makes Rift more than "another terminal"
- **Status line** — session context at a glance (model, context %, skill loaded, git branch)
- **Notification tabs** — Hooks, Errors, Agents — the observability surface Claude Code lacks
- **Filesystem activity tree** — see what Claude is touching in real-time
- **CRT aesthetic** — the brand. Not a feature, but it's what makes first impressions

### MUST HAVE (needs verification/hardening)
- **First-launch experience** — user installs MSI, opens Rift, sees terminal ready. No setup wizard required for basic use. Claude Code should work if already installed.
- **Graceful degradation** — if Aegis/Index/Sentinel aren't present, UI shows clean empty states (already designed per §10.7 capability-driven defaults)
- **Auto-updater working** — early access users need to get patches without re-downloading

### NICE TO HAVE (already built, ship if stable)
- MCP tool surface (20 tools — lets Claude Code read Rift state)
- Command palette + shortcuts overlay
- Detachable cockpit window
- Session replay viewer
- Bus event sparklines

### EXPLICITLY "COMING SOON" (acceptable to advertise)
- Sentinel real-time enforcement (D-010, post-v1)
- Linux/macOS builds (Tauri supports them; just needs CI matrix expansion)
- StatusLine live context/usage % (upstream-blocked on CC hook schema)
- Ecosystem mesh integrations (Brain Dump capture → Rift, Bridge, Grunt dispatch)

---

## 3. What to Cut/Defer

| Feature | Reason to defer |
|---------|----------------|
| Linux/macOS builds | CI matrix expansion; test surface triples. Ship Windows-only first, expand based on demand. |
| MCP write tools (aide.fs.*) | Default OFF already. Don't expose in early access — risk of data loss bugs. |
| Ecosystem mesh (Phase 10) | Depends on components that don't exist yet. |
| Release watcher agent (M3) | Nice-to-have automation, not user-facing. |
| Full Sentinel integration | Sentinel itself is v1.0.0 but Rift integration is post-v1. |
| IndexGraph free-form layout | Replaced with list view (D-019). Don't resurrect for beta. |

---

## 4. Stability Requirements

### Critical (blocking launch)
1. **PTY reliability on Windows** — ConPTY exit handling, no zombie processes, no terminal hangs. The v0.1.10 reaper thread + AtomicBool pattern needs stress testing under heavy Claude Code output.
2. **Memory under long sessions** — bus event accumulation, sparkline buffers, .jsonl session logs. Verify ring buffers are actually bounded. A user running Rift for 8 hours shouldn't see 2GB RAM.
3. **WebView2 stability** — known upstream issue (tauri-apps/tauri#6559) with WebGL context loss after sleep/tab-switch. WebGL renderer already reverted; verify DOM renderer is stable.
4. **Auto-updater end-to-end** — tag a test release, verify MSI updates cleanly. This is the #1 early-access requirement; broken updates = lost users.
5. **License key validation** — verify the website's /api/license/validate endpoint works with the Rift app. Flow: purchase → key generated → user enters key → app validates → works offline after first check.

### Important (should fix before launch)
6. **Crash reporting** — early access users will hit bugs. Need a way to get crash context. Tauri has panic hook capabilities; wire them to a local crash log at minimum.
7. **Settings persistence** — verify rift-config.toml survives updates, doesn't corrupt.
8. **First-run detection** — on first launch, show a brief "welcome" or "getting started" tooltip. Don't dump the user into a blank terminal with no guidance.

### Nice to have
9. **Performance profiling** — verify < 200ms terminal input latency under load.
10. **Accessibility** — v0.1.10 added ARIA landmarks. Verify screen reader basics work.

---

## 5. Launch Checklist

### Week 1: Stability Sprint
- [ ] Run full CI (fmt + clippy + build + test + check + boundary)
- [ ] Manual smoke test: install MSI, open Rift, run Claude Code session for 30 min
- [ ] Stress test: heavy output (cargo build of large project), verify no hangs
- [ ] Memory profiling: 2-hour session, check for leaks
- [ ] PTY edge cases: rapid resize, multiple terminals, kill mid-output
- [ ] Verify all notification tabs render without crashes when Claude Code runs
- [ ] Verify graceful degradation: no Aegis → clean empty states
- [ ] Test auto-updater: tag v0.2.0-beta.1, verify update from v0.1.x

### Week 2: Release Prep
- [ ] Write CHANGELOG for early access release
- [ ] Create GitHub release with RELEASING.md process (unsigned MSI acceptable)
- [ ] Wire license key check into Rift startup (or first-launch dialog)
- [ ] Verify Stripe purchase flow end-to-end: website → payment → key → email/dashboard
- [ ] Write early access landing page copy for abyssal-arts.com/products/rift
- [ ] Add "Early Access — Active Development" badge to the app's title bar
- [ ] Set up a feedback channel (GitHub Discussions, or simple /feedback form on website)
- [ ] Create a "known issues" section in the GitHub release notes

### Week 3: Soft Launch
- [ ] Tag release (v0.2.0-ea.1 or similar)
- [ ] Push release workflow → verify MSI artifact on GitHub Releases
- [ ] Test purchase flow with a real Stripe test-mode transaction
- [ ] Go live: switch Stripe to live mode, publish product page
- [ ] Announce in targeted communities (see marketing section below)

### Pre-launch one-time setup (if not already done)
- [ ] Tauri signing keypair generated (per RELEASING.md §1a) — **done**
- [ ] Public key wired into tauri.conf.json (§1b)
- [ ] GitHub Secrets: TAURI_SIGNING_PRIVATE_KEY + PASSWORD (§1c)
- [ ] Website /products/rift page exists with purchase button
- [ ] Stripe product configured for Rift ($29 one-time)

---

## 6. Pricing Validation

### Competitor landscape

| Product | Price | Model | Notes |
|---------|-------|-------|-------|
| **Warp** | Free (Teams $22/user/mo) | Freemium + team tier | AI-powered terminal, VC-funded, massive team |
| **Wave Terminal** | Free (open source) | OSS | Modern terminal, open source, no revenue model |
| **Tabby** | Free (open source) | OSS | Cross-platform, GPU-accelerated |
| **iTerm2** | Free | Donations | macOS only, decades of polish |
| **Alacritty** | Free (open source) | OSS | Minimal, GPU-accelerated |
| **Hyper** | Free (open source) | OSS | Electron-based |
| **Fig (now Amazon Q)** | Free | Acquired | Was $12/mo before acquisition |
| **Cursor** | $20/mo (pro) | Subscription | IDE, not terminal, but comparable "AI dev tool" |
| **Windsurf** | $15/mo | Subscription | Similar category |

### Analysis

The terminal market is dominated by free/OSS products. $29 one-time is actually reasonable IF:
1. You position Rift as a **Claude Code companion tool**, not a generic terminal
2. The value prop is **observability + cockpit**, not "better terminal emulation"
3. You target Claude Code power users who already pay $20/mo for Claude

### Pricing recommendation

**$29 one-time for early access is the right call.** Here's why:
- **It's less than 2 months of Claude subscription** — trivial for someone already invested in the Claude Code workflow
- **One-time > subscription** for a solo dev's first product. Subscriptions require ongoing value delivery and support infrastructure you don't have yet
- **"Early access" justifies the price** — users know they're buying a rough-but-real tool, and they're buying at a discount vs future v1.0 pricing
- **Consider: $29 early access → $49 at v1.0 launch.** This gives early buyers a deal and creates urgency

### Alternative: tiered pricing
- **$19 — Terminal only** (no cockpit, no integrations)
- **$29 — Full early access** (everything)
- This adds complexity you don't need yet. Ship one tier.

---

## 7. Risk Assessment

### HIGH RISK

| Risk | Impact | Mitigation |
|------|--------|------------|
| **PTY crashes on edge cases** | User loses terminal session, possible data loss | Stress test Week 1. ConPTY on Windows is notoriously finicky. The v1 lessons + reaper thread pattern should catch most, but long-session stability is unknown. |
| **WebView2 dependency** | Users without WebView2 can't run Rift. WebView2 is pre-installed on Windows 11 but may be missing on Windows 10 LTSC/Enterprise. | Tauri bundles a WebView2 bootstrapper. Verify it works on clean Win10 VM. |
| **Support burden** | Solo dev + early access users = every bug is a direct message to Garth | Set expectations clearly: "Early Access — expect bugs, report via GitHub Issues." Consider a GitHub Discussions Q&A board. Cap response commitment. |
| **Refund requests** | Users who expected a polished product | Clear "Early Access" labeling. Stripe allows refunds within 14 days — honor them without friction. |

### MEDIUM RISK

| Risk | Impact | Mitigation |
|------|--------|------------|
| **Auto-updater fails** | Users stuck on buggy version, can't get patches | Test thoroughly in Week 1. Provide manual download fallback on website. |
| **License key system bugs** | Users pay but can't activate | Test the full flow. Have a manual key-issue fallback (email). |
| **Performance on low-end hardware** | Rift is a Tauri app (Chromium + Rust) — memory footprint is real | Document minimum specs. Target 500MB RAM baseline. |
| **Claude Code API changes** | Hook schema evolves, Rift's translators break | §9 decoupling principle means only translator modules need updating, not core. Monitor CC changelog. |

### LOW RISK

| Risk | Impact | Mitigation |
|------|--------|------------|
| **Competition from free terminals** | Users choose free alternatives | Rift isn't competing with terminals — it's competing with "no observability tool at all." The cockpit is the moat. |
| **Niche too small** | Not enough Claude Code users who want this | $29 × 50 users = $1,450. The niche doesn't need to be big at this price point. Even 10-20 sales covers months of Claude subscription. |

---

## 8. Marketing Channels (Early Access)

### Primary targets (Claude Code users who would pay)
- **Claude Code Discord / community** — if Anthropic has official channels
- **r/ClaudeAI** — "I built a terminal cockpit for Claude Code sessions"
- **r/LocalLLaMA** — the power-user crowd; overlap with Claude Code users
- **Hacker News** — "Show HN: Rift — a terminal cockpit that shows you what Claude Code is doing"
- **Product Hunt** — one launch, done

### Messaging angle
Don't sell "a terminal." Sell **"finally see what Claude Code is doing."**

The pitch: "Claude Code runs in your terminal but you can't see what it's doing — which files it's touching, which hooks fired, whether Aegis loaded. Rift gives you a cockpit. Watch file activity in real-time, see errors before Claude reports them, observe agent behavior. Built by a Claude Code power user for Claude Code power users."

---

## 9. Summary — Fastest Path to $29 Sales

**Rift is closer to shippable than it looks.** Phases 0-8 are complete. The release workflow exists. The website's payment system is wired. The gap is:

1. **Stability verification** — 1 week of focused testing
2. **Auto-updater validation** — must work for early access
3. **License key integration** — wire the check into Rift's startup
4. **Landing page copy** — the website needs compelling product page
5. **A few Reddit/HN posts** — reach the audience

**Estimated timeline: 2-3 weeks from current state to first sale.**

The hardest part isn't technical — it's the same distribution challenge as Brain Dump. But the audience is different: Claude Code users are a self-selecting group of enthusiasts who are already spending $20/month on AI tooling. $29 for a cockpit tool is an easy sell to someone who lives in Claude Code daily.

**Revenue math at $29:**
- 10 sales = $290 (14+ months of Claude subscription)
- 50 sales = $1,450
- 100 sales = $2,900
- 500 sales = $14,500

Even the pessimistic scenario (10-20 early adopters) covers the Claude subscription for over a year.
