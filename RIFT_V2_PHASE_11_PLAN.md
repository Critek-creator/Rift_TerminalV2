# Rift V2 — Phase 11: Beta → v1.0 Release

*Generated: 2026-05-21 by `/aegis --plan` (TIER-SOLO). Follows v0.2.0 free open beta ship.*
*Source-of-truth refs: `RIFT_V2_VISION.md` v0.6 (locked) + `RIFT_V2_PHASE_PLAN.md` (Phases 0-10) + `docs/RELEASE_RUNBOOK.md`*
*Status: PLANNED — awaiting beta soak period + pricing decision*

---

## Goal

Turn Rift from a free, unsigned beta into a paid product on the Abyssal Arts website. The product itself is mature (43K LOC, 10 build phases shipped, 170+ tests, three-platform CI). What's missing is the commercial wrapper: trust signals (signing), purchase flow, legal minimum, and the polish that separates "developer tool" from "product."

---

## Prerequisites (before any sprint starts)

- [ ] v0.2.0 release verified on all 3 platforms (auto-updater confirmed working)
- [ ] Patreon page created (early signal on willingness-to-pay)
- [ ] At least 2 weeks of beta soak time (crash dumps, GitHub Issues, organic feedback)
- [ ] Pricing decision made (see Pricing section below)

---

## Phase 11.1 — Trust Layer (2-3 sessions + procurement lead time)

**Goal:** Eliminate SmartScreen/Gatekeeper warnings that kill first impressions.

### 11.1a — Apple Developer Enrollment

**External (Garth does manually):**
- [ ] Enroll at developer.apple.com ($99/year)
- [ ] Generate Developer ID Application certificate
- [ ] Create app-specific password for notarization (appleid.apple.com > Security > App-Specific Passwords)
- [ ] Note Team ID (10-char identifier)

**Build (1 session):**
- Add GitHub Secrets: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_TEAM_ID`, `APPLE_ID`, `APPLE_ID_PASSWORD`
- Update `.github/workflows/release.yml` — add Apple cert decode step + env vars to `tauri-action`
- Verify: tag a test release, download .dmg, double-click opens without Gatekeeper warning

### 11.1b — Windows Code Signing

**External:**
- [ ] Procure OV code-signing cert (Sectigo/DigiCert, ~$200-400/year)

**Build (0.5 session):**
- Set GitHub Secret: `WINDOWS_CERTIFICATE_BASE64`
- Release workflow already has the conditional signing stub — activates automatically
- Verify: tag a test release, download .exe, SmartScreen does not warn

### 11.1c — Verified Signed Auto-Updater

- After signing is live, cut a test release on each platform
- Verify the full signed update flow: old version detects → downloads → installs → restarts
- Document any platform quirks in `docs/RELEASE_RUNBOOK.md`

**External blockers:** Apple enrollment (1-2 days), cert procurement (1-5 business days). Start these immediately — they're calendar-time, not build-time.

---

## Phase 11.2 — Payment Infrastructure (2-3 sessions)

**Goal:** Someone can pay money and receive a download.

### 11.2a — Platform Selection

| Platform | Pros | Cons |
|----------|------|------|
| **Gumroad** | Zero setup, handles payments + delivery, good for indie | 10% fee, limited customization |
| **Paddle** | Merchant of record (handles global tax), professional | More setup, 5%+50c fee |
| **Stripe** | Full control, lowest fees (2.9%+30c) | You handle tax, delivery, customer support |

**Decision needed from Garth** before this phase starts. Recommendation: Gumroad for v1.0 (simplest path to revenue). Migrate to Paddle if volume justifies the tax/VAT automation.

### 11.2b — License Key System

**Option A — No license key (recommended for v1.0).** Sell on Gumroad/Paddle, buyer gets a download link. No DRM, no activation. Honor system. Works for indie dev tools where piracy isn't a real threat.

**Option B — Simple key validation (v1.x if needed).** Generate a license key on purchase, validate against Gumroad's license key API or a Cloudflare Worker. Show "Licensed to: [name]" in Settings. Adds one session of work.

Recommendation: Start with Option A. Add keys later only if piracy becomes a measurable problem (it won't at this scale).

### 11.2c — Website Integration (1 session)

- Add a "Buy" / "Download" page to abyssal-arts.com
- Pricing display with platform-specific download buttons (Windows / macOS / Linux)
- Link from Rift's Settings > About to the purchase page
- Update Getting Started page with purchase link

---

## Phase 11.3 — Legal Minimum (1 session + template)

**Goal:** Cover the legal basics for selling software.

### 11.3a — EULA / License Agreement

- Use a standard software EULA template
- Key terms: perpetual license, single user, no redistribution, no warranty, limitation of liability
- Display during install (NSIS supports license page) or on first launch
- Store as `docs/EULA.md` in repo, render on website

### 11.3b — Privacy Policy

- Simple page: "We don't collect data. Crash dumps stay on your machine. Auto-updater checks GitHub for new versions."
- Host on website
- Link from Settings

### 11.3c — Refund Policy

- If using Gumroad/Paddle: they handle refund mechanics by default
- State the policy on the purchase page: "30-day money-back, no questions asked"

---

## Phase 11.4 — Release Polish (3-4 sessions)

**Goal:** Ship paid-quality, not beta-quality.

### 11.4a — Self-Testing Sprint (ongoing during beta soak)

Use Rift as your daily terminal for 2+ weeks. Note every friction point:
- Shell compatibility (PowerShell, bash, zsh, fish)
- Long-running process behavior (npm install, cargo build, docker compose)
- Multi-tab workflows
- Cockpit usefulness — are the notification tabs actually helpful?
- Window management (resize, detach, restore, multi-monitor)

### 11.4b — Fix Beta Feedback (1-2 sessions)

- Triage GitHub Issues from beta period
- Fix the top 3-5 user-reported issues
- Close or defer the rest with clear labels

### 11.4c — E2E Smoke Tests (1 session, optional)

Only if beta reveals regression-class bugs that unit tests can't catch.
- 3-5 Playwright smoke tests: app launches, terminal accepts input, notification tab renders events
- Runs in CI as a post-build step
- vitest.config.ts already documents the Playwright deferral rationale — override only with evidence

### 11.4d — Performance Pass (1 session)

- Profile startup time — target < 2s to usable terminal
- Profile memory usage — baseline + after 1 hour of use
- Fix any outliers (memory leaks in bus subscriptions, unbounded event logs)
- Rust: `cargo build --release` timing, binary size audit

---

## Phase 11.5 — v1.0 Ship (1-2 sessions)

**Goal:** Cut the release and announce it.

### 11.5a — Version Bump + Changelog

- Bump to `1.0.0` across all files (Cargo.toml, tauri.conf.json, package.json — lesson: `version-bump-before-tag`)
- Write `CHANGELOG.md` covering v0.1.0 → v0.2.0 → v1.0.0
- Remove "BETA" badge from Settings (change to version number only)
- Update CLAUDE.md project status

### 11.5b — Release

- Follow `docs/RELEASE_RUNBOOK.md`
- All 3 platforms build with signed installers
- Publish the GitHub release (not draft)
- Update website: purchase page live, download buttons point to release

### 11.5c — Announce

- Patreon post thanking beta supporters
- Reddit: r/commandline, r/rust, r/webdev, r/devtools
- Hacker News (Show HN)
- Social media as applicable

---

## Sequence + Dependencies

```
Week 1-2:   Beta soak (self-test daily, collect feedback)
            Start Apple enrollment + cert procurement (async, external)
            Set up Patreon page
            Make pricing decision

Week 3:     11.1a+b — Signing workflows (blocked on cert arrival)
            11.3 — Legal (EULA + privacy policy + refund)

Week 4:     11.2 — Payment infrastructure (blocked on pricing decision)
            11.1c — Verify signed auto-updater

Week 5:     11.4a+b — Polish pass (fix beta feedback)
            11.4d — Performance pass

Week 6:     11.5 — Ship v1.0
```

**Total: ~6 weeks calendar, ~10-12 sessions of active work.** Beta soak (weeks 1-2) and cert procurement happen in parallel with zero session cost.

---

## Pricing Decision (needed before Phase 11.2)

| Model | Price | Tradeoff |
|-------|-------|----------|
| One-time purchase | $15-25 | Simple, no recurring revenue, customers own it forever |
| Annual license | $10-15/year | Recurring revenue, must justify renewal with updates |
| Pay-what-you-want | $0-50 | Low friction, unpredictable revenue, Gumroad supports natively |

**Recommendation:** One-time $20, pay-what-you-want floor of $10. Maximizes early adopter goodwill. Include a "lifetime updates" promise. Raise the price when Phase 10 Ecosystem Mesh features ship.

---

## Costs Summary

| Item | Cost | Frequency |
|------|------|-----------|
| Apple Developer Program | $99 | Annual |
| Windows OV code-signing cert | $200-400 | Annual |
| Gumroad fees | 10% per sale | Per transaction |
| Total fixed annual | ~$300-500 | — |
| Break-even at $20/sale | 15-25 sales/year | — |

---

## What's Explicitly NOT in Phase 11

- **Phase 10 Ecosystem Mesh** (grunt dispatch, bridge translator) — post-v1 feature work
- **E2E test suite beyond smoke tests** — add when a regression class demands it
- **Native mobile client** — post-v1 per §6
- **Sentry crash reporting** — add if local dumps prove insufficient
- **Feature work** — v1.0 ships what v0.2.0 has, but signed and polished
- **License key DRM** — v1.0 uses honor system; add keys only if piracy is real

The product is done. Phase 11 is wrapping the gift.

---

*Phase 11 plan by Abyssal Aegis v2.10.0 — 2026-05-21*
*Pairs with: `RIFT_V2_PHASE_PLAN.md` (Phases 0-10), `docs/RELEASE_RUNBOOK.md`, `docs/getting-started.md`*
