<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getPtyId } from './lib/terminalInject';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window';
  import { check, type Update } from '@tauri-apps/plugin-updater';
  import TitleBar from './lib/TitleBar.svelte';
  import TabBar from './lib/TabBar.svelte';
  import TerminalGrid from './lib/TerminalGrid.svelte';
  import NotificationPane from './lib/NotificationPane.svelte';
  import AegisTabContent from './lib/AegisTabContent.svelte';
  import IndexTabContent from './lib/IndexTabContent.svelte';
  import BusTailTabContent from './lib/BusTailTabContent.svelte';
  import TodoTabContent from './lib/TodoTabContent.svelte';
  import GitTabContent from './lib/GitTabContent.svelte';
  import AgentsTabContent from './lib/AgentsTabContent.svelte';
  import LlmActivityTabContent from './lib/LlmActivityTabContent.svelte';
  import FsTabContent from './lib/FsTabContent.svelte';
  import McpTabContent from './lib/McpTabContent.svelte';
  import SentinelTabContent from './lib/SentinelTabContent.svelte';
  import SessionsTabContent from './lib/SessionsTabContent.svelte';
  import HealthTabContent from './lib/HealthTabContent.svelte';
  import IntegrationInspector from './lib/IntegrationInspector.svelte';
  import FeaturePipelineTabContent from './lib/FeaturePipelineTabContent.svelte';
  import StatusLine from './lib/StatusLine.svelte';
  import ModeHintBar from './lib/ModeHintBar.svelte';
  import FailuresPanel from './lib/FailuresPanel.svelte';
  import Popout from './lib/Popout.svelte';
  import Tree from './lib/Tree.svelte';
  import IndexGraph from './lib/IndexGraph.svelte';
  import Splitter from './lib/Splitter.svelte';
  import { subscribe, signalBusReady, publish } from './lib/bus';
  import { actionRegistry } from './lib/actionRegistry.svelte';
  import { enrichmentRegistry } from './lib/enrichmentRegistry.svelte';
  import { actionProviders } from './lib/actionProviders.svelte';
  import { errorHandoffProvider } from './lib/errorHandoffProvider.svelte';
  import { commandFailureStore } from './lib/commandFailureStore.svelte';
  import { popouts } from './lib/popouts.svelte';
  import { enrichmentStore } from './lib/enrichmentStore.svelte';
  import type { RiftConfig as RiftConfigType, StatusLineConfig, AlertRule } from './lib/riftConfig';
  import { SparklineBuffer } from './lib/SparklineBuffer';
  import { invalidateTerminalSettingsCache } from './lib/terminalConfigCache';
  import { evaluateRule, triggerAction } from './lib/alertRules';
  import { CorrelationIndex } from './lib/correlationIndex';
  import CommandPalette from './lib/CommandPalette.svelte';
  import ShortcutOverlay from './lib/ShortcutOverlay.svelte';
  import WelcomeOverlay from './lib/WelcomeOverlay.svelte';
  import CommandIntelligencePanel from './lib/CommandIntelligencePanel.svelte';
  import { notifPriority } from './lib/notifPriority.svelte';
  import { sessionManager as sm } from './lib/sessionManager.svelte';
  import { notifManager as nm } from './lib/notifState.svelte';
  import { shouldShow } from './lib/notifFilter';
  import { llmModels, loadFromConfig as loadLlmFromConfig } from './lib/llmModels.svelte';

  // ----- session state (extracted to sessionManager.svelte.ts) -----
  // ----- notification state (extracted to notifManager.svelte.ts) -----

  async function loadAppearanceConfig() {
    try {
      const cfg = await invoke<RiftConfigType>('config_get');
      statuslineConfig = cfg?.statusline;
      const family = cfg?.terminal?.font_family;
      if (family) {
        document.documentElement.style.setProperty('--font-family', family);
      }
    } catch (err) {
      console.warn('Failed to load appearance config:', err);
    }
  }

  // ----- correlation index -----
  const correlationIndex = new CorrelationIndex();

  // ----- alert rules -----
  let alertRules = $state<AlertRule[]>([]);
  const alertBuffers = new Map<string, SparklineBuffer>();
  let alertTriggered = $state<Record<string, boolean>>({});

  function getAlertBuffer(tabId: string): SparklineBuffer {
    let buf = alertBuffers.get(tabId);
    if (!buf) {
      buf = new SparklineBuffer();
      alertBuffers.set(tabId, buf);
    }
    return buf;
  }

  async function loadAlertRules() {
    try {
      const cfg = await invoke<RiftConfigType>('config_get');
      alertRules = cfg?.alerts?.rules ?? [];
    } catch (err) {
      console.warn('Failed to load alert rules:', err);
    }
  }

  // Populate the Ensemble Router model store at boot. Without this the store
  // stays empty until the user opens model settings (which is the only other
  // caller of loadFromConfig), so the router prompt / model indicator show no
  // models and auto-started local servers appear unconfigured.
  async function loadLlmConfig() {
    try {
      const cfg = await invoke<RiftConfigType>('config_get');
      loadLlmFromConfig(cfg?.ensemble);
    } catch (err) {
      console.warn('Failed to load LLM ensemble config:', err);
    }
  }

  // ----- command palette -----
  let paletteOpen = $state(false);
  let paletteInitialQuery = $state('');

  // ----- shortcut overlay -----
  let shortcutsOpen = $state(false);

  // ----- command-failures issues list (Phase 5 / R1.5) -----
  let failuresOpen = $state(false);

  // ----- welcome overlay (first-run experience) -----
  let welcomeOpen = $state(false);

  async function dismissWelcome() {
    welcomeOpen = false;
    try {
      const cfg = await invoke<RiftConfigType>('config_get');
      await invoke('config_save', { cfg: { ...cfg, first_run_completed: true } });
    } catch (err) {
      console.warn('[App] failed to save first_run_completed:', err);
    }
  }

  /** Re-open welcome overlay from Settings "Show Welcome Guide" button. */
  function showWelcomeGuide() {
    welcomeOpen = true;
  }

  // Expose to SettingsPanel via window event (cleaned up in onMount return).

  // Phase 6.2 — cockpit right pane header data.
  // Tree.svelte computes these internally and pushes them up via $bindable props.
  let nodeCount = $state(0);
  let watchedPathLabel = $state('…');

  // Phase 6.4 — cockpit detach state.
  // `cockpitDetached` drives whether the cockpit-right renders the Tree or the
  // placeholder card. Polled once on mount for reload-recovery (design E),
  // then kept current via `cockpit_detached` / `cockpit_reattached` events.
  let cockpitDetached = $state(false);

  // Phase 8.7e — resizable pane sizes. Splitter.svelte hydrates these from
  // localStorage on mount and persists on drag-end. Defaults match the prior
  // CSS flex-basis values so existing users see no immediate layout change.
  let cockpitRightWidth = $state(420); // px — was flex: 0 0 38% (~420 of 1100)
  let promotedPaneWidth = $state(420); // px — promoted notif pane, splitter-resizable
  let graphHeightPct = $state(55);     // percent — was flex: 0 0 55%

  // Cockpit collapse toggle — hides the right pane (graph + tree) to give
  // the terminal full width. Persisted to localStorage. Ctrl+B toggles.
  const COCKPIT_COLLAPSED_KEY = 'rift.cockpit.collapsed';
  let cockpitCollapsed = $state(
    (() => { try { return localStorage.getItem(COCKPIT_COLLAPSED_KEY) === 'true'; } catch { return false; } })()
  );

  function toggleCockpit() {
    cockpitCollapsed = !cockpitCollapsed;
    try { localStorage.setItem(COCKPIT_COLLAPSED_KEY, String(cockpitCollapsed)); } catch {}
  }

  // Phase 8.7g — main window position + size persistence.
  // tauri.conf.json hardcodes width:1100/height:700 at every launch, so
  // resizing the window during a session is forgotten on restart. Saving
  // the outer rect to localStorage on move/resize and restoring on mount
  // mirrors the pattern CockpitDetached.svelte already uses for the
  // detached cockpit window.
  const MAIN_POS_KEY = 'rift.main.window_pos';
  interface SavedMainPos { x: number; y: number; width: number; height: number; }
  // Corruption floor for window-geometry restore/persist. The main window's
  // configured minimum is 720×480 logical (tauri.conf.json), so any saved
  // PHYSICAL outerSize below 600×400 can only come from a broken launch
  // (0×0 / tiny outerSize) — never a legitimate resize. Restoring such a value
  // is the root cause of the "launches as a tiny window" bug (instrumentation
  // proved Rust reveal hands off a healthy 1650×1050; the frontend setSize
  // below then shrank it from corrupt localStorage). Reject sub-floor sizes on
  // BOTH restore and persist so the corruption can neither take hold nor
  // re-perpetuate. See memory reference_rift_webview_launch_minimized.
  const MAIN_MIN_W = 600;
  const MAIN_MIN_H = 400;

  // D-013 — Updater plugin frontend wiring (session-start check).
  // On launch we ask plugin-updater to fetch latest.json from the GitHub
  // releases endpoint configured in tauri.conf.json. If a newer version
  // is signed-and-published, we surface a thin banner with Install/Later.
  // The check is silent on failure (offline, GitHub down, no release yet) —
  // we don't bother the user with check-time errors, only with availability.
  let availableUpdate = $state<Update | null>(null);
  let updateInstalling = $state(false);
  let updateError = $state<string | null>(null);

  // §10.9 live-border tick — drives TabBar.svelte's `isLive` derived. The
  // 1-second cadence is fast enough to feel responsive without burning CPU;
  // the live window itself is 3s in TabBar.
  let tickNow = $state(Date.now());

  // Batched badge accumulator — coalesces rapid bus envelopes into one
  // Svelte reactivity update per animation frame. Under multi-session
  // traffic (3+ concurrent Claude Code sessions), the master subscriber
  // receives 10+ envelopes/sec. Without batching, each triggers
  // nm.notifs = nm.notifs.map(...) → full reactive graph propagation.
  // Batching reduces this to ≤1 mutation per frame (~60Hz).
  const UNREAD_CAP = 999;
  const _pendingBadges = new Map<string, { lastActivityTs: number; unreadDelta: number }>();
  let _badgeFlushId = 0;

  function _flushBadges() {
    _badgeFlushId = 0;
    if (_pendingBadges.size === 0) return;
    nm.notifs = nm.notifs.map((n) => {
      const p = _pendingBadges.get(n.id);
      if (!p) return n;
      const inView = nm.promoted === n.id;
      return {
        ...n,
        detected: true,
        lastActivityTs: p.lastActivityTs,
        unreadCount: n.enabled
          ? (inView ? 0 : Math.min(n.unreadCount + p.unreadDelta, UNREAD_CAP))
          : n.unreadCount,
      };
    });
    _pendingBadges.clear();
  }

  // §10.7 capability-gate + §10.9 badge counter + live border —
  // master envelope subscription lives at App level so it works regardless
  // of which tab is currently mounted (panes only mount when active).
  // Per-envelope effect: flip detected=true, bump unreadCount (unless tab
  // is currently in view), update lastActivityTs.
  // pr003 svelte5-async-cleanup-via-sync-shell-iife pattern + cancelled-flag
  // mount-race guard match the SKILL subscription below.
  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({}, (env) => {
          const id = nm.NOTIF_BY_CATEGORY[env.category];
          if (!id) return;
          // Pty has many kinds in the long run; today only "command.submitted"
          // is published, but kind-filter here makes the policy explicit so
          // future Category::Pty kinds (e.g. resize, exit) don't accidentally
          // count toward the Commands tab badge.
          if (env.category === 'pty' && env.kind !== 'command.submitted') return;
          // Phase 8.7q.4 — skip the `system/notif.tabs` snapshot publish
          // (broadcast by the debounced $effect below whenever `notifs`
          // changes). It is a state-mirror, not an activity event.
          // Without this guard:
          //   $effect publishes notif.tabs → master subscriber bumps
          //   errors.unreadCount → notifs changes → $effect re-runs
          //   → publishes again ⟶ infinite loop @ ~100Hz, generating
          //   ~10k orphan-callback "[TAURI] Couldn't find callback id"
          //   warnings/sec on dead subscribers.
          if (env.category === 'system' && env.kind === 'notif.tabs') return;
          const now = Date.now();
          const target = nm.notifs.find((n) => n.id === id);
          if (!target) return;
          const inView = nm.promoted === id;
          // H-1 fix (notif-filter audit 2026-05-31): gate the unread *count* by
          // the tab's severity threshold so the badge agrees with the filtered
          // events list (which already calls shouldShow). Detection (§10.7) and
          // the live-border pulse stay severity-independent — a tab still
          // appears and pulses for any activity; only the count honors the
          // threshold. `bustail` resolves to a 'debug' threshold so it is
          // unaffected (counts everything, matching its firehose role).
          const passesFilter = shouldShow(env.kind, nm.thresholdFor(id));
          // Once a tab hits UNREAD_CAP, further increments are clamped away
          // anyway — skip accumulation to avoid unnecessary flush cycles.
          if (target.detected && !inView && target.unreadCount >= UNREAD_CAP) {
            // Still update lastActivityTs for the live-border pulse.
            const existing = _pendingBadges.get(id);
            if (existing) { existing.lastActivityTs = now; }
            else { _pendingBadges.set(id, { lastActivityTs: now, unreadDelta: 0 }); }
          } else {
            const delta = (target.enabled && !inView && passesFilter) ? 1 : 0;
            const existing = _pendingBadges.get(id);
            if (existing) {
              existing.lastActivityTs = now;
              existing.unreadDelta += delta;
            } else {
              _pendingBadges.set(id, { lastActivityTs: now, unreadDelta: delta });
            }
          }
          if (!_badgeFlushId) {
            _badgeFlushId = requestAnimationFrame(_flushBadges);
          }

          // Adaptive notification priority: seed baseline from all envelopes.
          if (notifPriority.isEnabled()) {
            notifPriority.recordInteraction(env.kind, 'ignore');
          }

          // Correlation: index every envelope that carries a correlation_id.
          correlationIndex.index(env);

          // Alert rules: record event in per-tab buffer and evaluate.
          const buf = getAlertBuffer(id);
          buf.record();
          for (const rule of alertRules) {
            if (rule.tab_id !== id) continue;
            if (evaluateRule(rule, buf, env.kind)) {
              const result = triggerAction(rule.action);
              if (result.flash) {
                alertTriggered = { ...alertTriggered, [id]: true };
                setTimeout(() => {
                  alertTriggered = { ...alertTriggered, [id]: false };
                }, 2000);
              }
              if (result.promote && nm.promoted !== id) {
                nm.promoted = id;
              }
              break;
            }
          }
        });
        if (cancelled) {
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] master notif subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      if (_badgeFlushId) { cancelAnimationFrame(_badgeFlushId); _badgeFlushId = 0; }
      _pendingBadges.clear();
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // D-012 — live StatusLine segments.
  // Subscribes to Category::Status at App level so the StatusLine updates
  // regardless of which tab is active. Same svelte5-async-cleanup-via-sync-
  // shell-iife + cancelled-flag pattern as the SKILL subscription below.
  // Global env segments — computed from the single project root, app-wide.
  // Sourced from the BASE status envelope (the one with no session_id).
  let statusDir = $state('');
  let statusGit = $state('');
  let statusRepoBasename = $state('');
  let statusSession = $state(''); // session clock — ticked by the 1s timer below
  let statuslineConfig = $state<StatusLineConfig | undefined>(undefined);

  // Per-pane process-tree resource sample (StatusLine CPU/RAM). Sampled at 1Hz
  // for the FOCUSED pane via `sample_pane_resources`; null = unavailable
  // (no PTY started, or sampling failed) → segments hide.
  interface PaneResources {
    cpu_pct: number;
    rss_kb: number;
    foreground_cmd: string;
    pid_count: number;
  }
  let paneResources = $state<PaneResources | null>(null);
  // Plain (non-reactive) guard so overlapping 1Hz invokes don't pile up.
  let resourceSampleInFlight = false;

  function formatRam(kb: number): string {
    if (kb <= 0) return '';
    const mb = kb / 1024;
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${Math.round(mb)} MB`;
  }

  const statusCpu = $derived(
    paneResources && paneResources.pid_count > 0
      ? `${paneResources.cpu_pct.toFixed(1)}%`
      : '',
  );
  const statusRam = $derived(paneResources ? formatRam(paneResources.rss_kb) : '');

  // Phase 2 / N4 — ambient indicators for the ModeHintBar. Reuse the existing
  // notif unread state (no new bus subscriptions): each indicator lights with a
  // count when its tab has unacked activity, and dims once promoted (ackUnread
  // zeroes the count). The mode-hint line's context comes from sm.active.kind.
  const ambientCount = (id: string): number =>
    nm.notifs.find((n) => n.id === id)?.unreadCount ?? 0;
  const errorAmbient = $derived(ambientCount('errors'));
  const agentAmbient = $derived(ambientCount('agents'));
  const mcpAmbient = $derived(ambientCount('mcp'));

  // Per-session Claude Code status. Each Rift PTY runs its own CC session whose
  // statusLine bridge tees to rift-cc-status-<sessionId>.json; status.rs emits
  // one usage envelope per session carrying `session_id`. We key by that id and
  // the StatusLine renders the FOCUSED pane's CC data — fixing the prior bug
  // where one shared temp file meant the status line showed whichever terminal
  // wrote last ("reads from any and every terminal").
  interface CcStatus {
    model?: string;
    ctx_pct?: number;
    session_use_pct?: number;
    week_pct?: number;
    github_owner?: string;
    github_repo?: string;
    skill?: string;
    effort?: string;
    thinking?: string;
  }
  // Reassigned (not mutated) on each update so the deriveds below recompute.
  let ccBySession = $state<Record<number, CcStatus>>({});

  const ccFocused = $derived(ccBySession[sm.focusedSessionId]);
  const statusModel = $derived(ccFocused?.model ?? '');
  const statusCtx = $derived(ccFocused?.ctx_pct != null ? `${ccFocused.ctx_pct}%` : '');
  const statusSessionUse = $derived(
    ccFocused?.session_use_pct != null ? `${ccFocused.session_use_pct}%` : '',
  );
  const statusWeek = $derived(ccFocused?.week_pct != null ? `${ccFocused.week_pct}%` : '');
  const statusThinking = $derived(ccFocused?.thinking ?? '');
  const ccSkill = $derived(ccFocused?.skill ?? '');
  const ccEffort = $derived(ccFocused?.effort ?? '');

  // REPO precedence: focused session's CC-sourced github owner/name (per-session)
  // → active pane path basename (instant on focus switch, no poll wait) → global
  // repo basename from the base envelope.
  const statusRepo = $derived.by(() => {
    if (ccFocused?.github_owner && ccFocused?.github_repo) {
      return `${ccFocused.github_owner}/${ccFocused.github_repo}`;
    }
    const path = sm.activeProjectPath;
    if (path) {
      const parts = path.replace(/\\/g, '/').split('/');
      const name = parts[parts.length - 1];
      if (name) return name;
    }
    return statusRepoBasename;
  });

  // Session elapsed timer. Stored in sessionStorage (NOT localStorage) so it
  // resets to zero on each fresh app launch — a new webview context starts with
  // empty sessionStorage — while still surviving in-app reloads / Vite HMR,
  // which keep the same context. localStorage persisted across launches, so the
  // clock accumulated indefinitely (observed at 86h) instead of tracking the
  // current session.
  const SESS_START_KEY = 'rift:sessionStartMs';
  const sessionStartMs = Number(sessionStorage.getItem(SESS_START_KEY)) || Date.now();
  sessionStorage.setItem(SESS_START_KEY, String(sessionStartMs));

  // Unified 1-second timer — handles session clock, live-border tick,
  // timeline now, and sparkline buffer ticks. Merging avoids two competing
  // setIntervals and lets each sub-concern skip work when idle.
  $effect(() => {
    const LIVE_WINDOW_MS = 4000; // slightly wider than TabBar's 3s to avoid edge-case flicker
    const tick = () => {
      const now = Date.now();
      // Session clock — always updates (cheap string format).
      const elapsed = Math.floor((now - sessionStartMs) / 1000);
      const h = Math.floor(elapsed / 3600);
      const m = Math.floor((elapsed % 3600) / 60);
      const s = elapsed % 60;
      statusSession = h > 0
        ? `${h}h ${String(m).padStart(2, '0')}m`
        : `${m}m ${String(s).padStart(2, '0')}s`;

      // Live-border tick — only update tickNow when at least one tab has
      // recent activity, otherwise the TabBar deriveds are already stable.
      const hasRecentActivity = nm.notifs.some(
        (n) => n.lastActivityTs !== null && (now - n.lastActivityTs) < LIVE_WINDOW_MS,
      );
      if (hasRecentActivity) tickNow = now;

      // Sparkline buffer ticks — always cheap (just shifts a fixed-size ring).
      for (const buf of alertBuffers.values()) buf.tick();

      // Per-pane resource sample for the focused pane's StatusLine CPU/RAM.
      // Guarded so a slow sample never lets overlapping invokes pile up.
      const ptyId = getPtyId(sm.focusedSessionId);
      if (ptyId === undefined) {
        paneResources = null;
      } else if (!resourceSampleInFlight) {
        resourceSampleInFlight = true;
        invoke<PaneResources>('sample_pane_resources', { ptyId })
          .then((snap) => { paneResources = snap; })
          .catch(() => { paneResources = null; })
          .finally(() => { resourceSampleInFlight = false; });
      }
    };
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({ category: 'status' }, (env) => {
          if (env.kind !== 'usage') return;
          const p = env.payload as {
            session_id?: number;
            dir?: string; git?: string; repo?: string;
            model?: string; ctx_pct?: number; session_use_pct?: number; week_pct?: number;
            github_owner?: string; github_repo?: string;
            skill?: string; effort?: string; thinking?: string;
          };
          if (p.session_id !== undefined) {
            // Per-session CC envelope — store under its session id. Reassign the
            // record (not mutate) so the ccFocused derived recomputes.
            ccBySession = {
              ...ccBySession,
              [p.session_id]: {
                model: p.model,
                ctx_pct: p.ctx_pct,
                session_use_pct: p.session_use_pct,
                week_pct: p.week_pct,
                github_owner: p.github_owner,
                github_repo: p.github_repo,
                skill: p.skill,
                effort: p.effort,
                thinking: p.thinking,
              },
            };
          } else {
            // Base envelope — global env segments (DIR / GIT / REPO basename).
            if (p.dir  !== undefined) statusDir = p.dir;
            if (p.git  !== undefined) statusGit = p.git;
            if (p.repo !== undefined) statusRepoBasename = p.repo;
          }
        });
        if (cancelled) {
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] status subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // Live local-LLM process status — keep the model-card status dots in sync
  // with auto-start, MCP-initiated starts, our own start/stop, and crashes.
  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      // Seed from whatever is already running (e.g. boot auto-start).
      void llmModels.syncRunning();
      try {
        const u = await subscribe({ category: 'llm' }, (env) => {
          if (
            env.kind === 'llm.process.start' ||
            env.kind === 'llm.process.stop' ||
            env.kind === 'llm.process.crash'
          ) {
            const p = env.payload as { model_id?: string };
            if (p.model_id) llmModels.applyProcessEvent(env.kind, p.model_id);
          }
        });
        if (cancelled) {
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] llm process subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // Phase 7.4 — live SKILL segment.
  // D-016 (closed) — live EFFORT segment. Both subscriptions ride the same
  // Category::Aegis stream; one $effect, one wire-tap dispatch, two state
  // bindings. Subscribes at App level (not inside AegisTabContent) so
  // status-line segments update regardless of which tab is active.
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: $effect cleanup is SYNC;
  // async unsubscribe wrapped in IIFE. Mount-race guarded via `cancelled`
  // flag — if cleanup fires before subscribe resolves, the resolved
  // unsubscribe is invoked immediately so the subscription doesn't leak.
  let aegisSkillName = $state('');
  let aegisEffort = $state('');

  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({ category: 'aegis' }, (env) => {
          if (env.kind === 'aegis.session.skill_loaded') {
            const p = env.payload as { skill_name?: string; skill_version?: string | null };
            aegisSkillName = p.skill_name ?? '';
          } else if (env.kind === 'aegis.session.effort') {
            // Producer (private rift-aegis crate, feature-gated) publishes
            // a tier label per /aegis dispatch — `low` / `medium` / `high`
            // / `xhigh` / `max`. Public-CI builds without the `aegis`
            // feature never see this envelope; segment stays at '—'.
            const p = env.payload as { effort?: string | null };
            aegisEffort = p.effort ?? '';
          }
        });
        if (cancelled) {
          // Cleanup already ran while subscribe was in flight — clean up now.
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] aegis-session subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // Category::Index lifecycle subscription (vault.update + walk.complete).
  // Enrichment ATTACH now flows through the generic enrichmentRegistry
  // (Category::System, kind="enrichment.attach") which the vault-walker
  // dogfoods — see enrichmentRegistry.svelte.ts. This subscription keeps the
  // Index-specific deletion + load-complete signals. Same
  // svelte5-async-cleanup-via-sync-shell-iife + cancelled-flag pattern as the
  // status and skill_loaded subscriptions above.
  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({ category: 'index' }, (env) => {
          if (env.kind === 'vault.update') {
            const p = env.payload as { change_kind: string; vault_id: string };
            if (p.change_kind === 'deleted') {
              enrichmentStore.removeByVaultId(p.vault_id);
            }
            // Other change_kinds (e.g. "updated") are no-ops here;
            // consumed by other surfaces in later phases.
          } else if (env.kind === 'walk.complete') {
            enrichmentStore.loaded = true;
          }
          // All other Category::Index kinds fall through — no-op.
        });
        if (cancelled) {
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] index enrichment subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      void (async () => {
        await unsub?.();
      })();
    };
  });

  onMount(() => {
    // Reveal the main window now that the app has mounted + rendered. The
    // window starts hidden (tauri.conf `visible: false`) to avoid the
    // blank/minimized-flash race during WebView2 init; this is the fast path
    // that shows it the instant the UI is ready. Fire-and-forget — a backend
    // fallback timer reveals it regardless if this never lands.
    invoke('main_window_ready').catch(() => {});

    // JS crash log — capture unhandled errors for beta issue reporting.
    const CRASH_KEY = 'rift:crash_log';
    const MAX_CRASHES = 10;
    function logJsCrash(msg: string, source?: string) {
      try {
        const existing = JSON.parse(localStorage.getItem(CRASH_KEY) || '[]') as Array<unknown>;
        existing.push({ ts: Date.now(), msg, source });
        if (existing.length > MAX_CRASHES) existing.splice(0, existing.length - MAX_CRASHES);
        localStorage.setItem(CRASH_KEY, JSON.stringify(existing));
      } catch { /* localStorage quota — non-fatal */ }
    }
    window.onerror = (_msg, source, line, col, err) => {
      logJsCrash(err?.message ?? String(_msg), `${source}:${line}:${col}`);
    };
    window.onunhandledrejection = (e) => {
      logJsCrash(String(e.reason), 'unhandled-promise');
    };

    // Phase 8.7q.4 — drain orphan async resources from the prior page load.
    // On every mount (including HMR reloads), kill all Rust-side PTY drains
    // and bus subscriptions whose JS callbacks died with the prior page.
    // bus.ts gates subscribe() behind signalBusReady(), so no subscription
    // can fire until this reset completes — eliminating the race where
    // $effects create subs before reset kills orphans.
    // NOTE: resetBusReady() is NOT called here — $effect subscribe calls
    // are already awaiting the module-scope promise. Replacing it would
    // leave them hanging on a dead promise forever.
    void (async () => {
      try {
        await invoke('rift_reset_for_reload');
      } catch (err) {
        console.warn('[App] rift_reset_for_reload failed:', err);
      }
      signalBusReady();
      // §9 control-endpoint registry + in-process provider (candidate 568).
      // Await the registry's subscribe BEFORE the provider declares, so the
      // declaration is delivered live (the bus replay buffer is a backstop, not
      // the primary path). Sequenced so ordering never depends on IPC latency.
      await actionRegistry.start();
      await actionProviders.start();
      // Phase 5 / R1 — local-only error→agent explain provider. Started after
      // the registry so its action.result publishes are delivered live, and it
      // is listening before any failure can declare/invoke an explain action.
      await errorHandoffProvider.start();
      // Phase 5 / R1.5 — persistent command-failure log feeding the status-line
      // issues list. Subscribes to the command.failed stream (R0 capture).
      await commandFailureStore.start();
      // §9 data-enrichment registry (class 3) — generic enrichment.attach
      // channel the vault-walker now dogfoods. Started before the walker can
      // emit so attaches are delivered live (replay buffer as backstop).
      await enrichmentRegistry.start();
      const root = await invoke<string>('project_root_get');
      sm.initialProjectRoot = root;
      if (sm.sessions[0] && sm.sessions[0].projectPath === null) {
        sm.sessions = sm.sessions.map((s, i) => i === 0 ? { ...s, projectPath: root } : s);
      }
    })();

    // Phase 8.7j — restore persisted notif tab order before any subscription
    // fires. Pure synchronous read of localStorage; ordering must settle
    // before TabBar renders to avoid a flicker.
    nm.applyPersistedNotifOrder();
    nm.applyPersistedWorkspace();

    // Svelte 5's onMount requires a sync callback that optionally returns a
    // cleanup. Async init runs in an IIFE; cleanup captures unlisten handles
    // via mutable refs the IIFE populates.
    let unlistenDetached: (() => void) | undefined;
    let unlistenReattached: (() => void) | undefined;
    let unlistenMoved: (() => void) | undefined;
    let unlistenResized: (() => void) | undefined;

    window.addEventListener('rift:show-welcome', showWelcomeGuide);

    // Load notification filter thresholds from config on mount.
    void nm.loadNotifFilters();

    // First-run welcome check — show overlay if config.first_run_completed is false.
    void (async () => {
      try {
        const cfg = await invoke<RiftConfigType>('config_get');
        if (!cfg?.first_run_completed) welcomeOpen = true;
      } catch { /* non-fatal — skip welcome on config read failure */ }
    })();

    // Load font_family + statusline config on mount.
    void loadAppearanceConfig();

    // Load alert rules from config on mount.
    void loadAlertRules();

    // Load Ensemble Router models from config on mount.
    void loadLlmConfig();

    // Re-read filters + appearance + alerts + LLM models when Settings saves
    // (rift:config-changed broadcast).
    const onConfigChanged = () => {
      void nm.loadNotifFilters();
      void loadAppearanceConfig();
      void loadAlertRules();
      void loadLlmConfig();
      invalidateTerminalSettingsCache();
    };
    window.addEventListener('rift:config-changed', onConfigChanged);

    void (async () => {
      try {
        cockpitDetached = await invoke<boolean>('cockpit_status');
      } catch (err) {
        console.warn('[App] cockpit_status failed:', err);
      }

      // Recover notification tab detach state on reload.
      await nm.recoverDetachState();

      unlistenDetached = await listen('cockpit_detached', () => {
        cockpitDetached = true;
      });
      unlistenReattached = await listen('cockpit_reattached', () => {
        cockpitDetached = false;
      });

      // D-013 — session-start update check. Silent on failure; only surfaces
      // when an update is genuinely available so users aren't pestered.
      try {
        const update = await check();
        if (update) {
          availableUpdate = update;
        }
      } catch (err) {
        console.warn('[App] update check failed:', err);
      }

      // Phase 8.7g — main window position+size restore + tracking.
      // Run AFTER the cockpit_status / update check so a slow remote update
      // probe doesn't delay the visual settle. Failures are non-fatal —
      // the window just stays at the tauri.conf.json defaults.
      try {
        const appWindow = getCurrentWindow();
        const raw = localStorage.getItem(MAIN_POS_KEY);
        if (raw) {
          try {
            const pos = JSON.parse(raw) as SavedMainPos;
            const sane =
              typeof pos.x === 'number' && typeof pos.y === 'number' &&
              typeof pos.width === 'number' && typeof pos.height === 'number' &&
              pos.width >= MAIN_MIN_W && pos.height >= MAIN_MIN_H;
            if (sane) {
              // Off-screen guard: if saved position is beyond visible monitors,
              // center the window instead of restoring to invisible coordinates.
              // screen.availWidth/Height cover the primary monitor; multi-monitor
              // may still clip but at least the primary is guaranteed.
              const onScreen =
                pos.x >= -pos.width / 2 &&
                pos.y >= -pos.height / 2 &&
                pos.x < (window.screen?.availWidth ?? 9999) &&
                pos.y < (window.screen?.availHeight ?? 9999);
              if (onScreen) {
                await appWindow.setPosition(new PhysicalPosition(pos.x, pos.y));
              } else {
                await appWindow.center();
              }
              await appWindow.setSize(new PhysicalSize(pos.width, pos.height));
            } else {
              // Corrupt geometry — most often a sub-floor (tiny / 0×0) size
              // persisted by an earlier broken launch. Drop it so the healthy
              // Rust-revealed default stands; the immediate re-save below then
              // overwrites it with a sane rect (self-healing on this launch).
              try { localStorage.removeItem(MAIN_POS_KEY); } catch { /* ignore */ }
            }
          } catch {
            try { localStorage.removeItem(MAIN_POS_KEY); } catch { /* ignore */ }
          }
        }
        // WebView2 restores its own cached minimized/maximized state AFTER
        // Rust setup() runs unminimize(). Re-assert here so the JS-side
        // restore always wins the race.
        await appWindow.unminimize();
        await appWindow.show();
        await appWindow.setFocus();

        // Floor-guarded persist: never write a sub-minimum (broken-launch)
        // rect to localStorage, or it would re-seed the "tiny window" bug on
        // the next restore. Centralized so all three save sites share the guard.
        const persistRect = (x: number, y: number, width: number, height: number) => {
          if (width < MAIN_MIN_W || height < MAIN_MIN_H) return;
          try {
            localStorage.setItem(MAIN_POS_KEY, JSON.stringify({ x, y, width, height }));
          } catch { /* quota / private */ }
        };
        // Save current rect immediately so a crash before any move/resize
        // still records a reasonable last-known position.
        const [pos0, size0] = await Promise.all([
          appWindow.outerPosition(),
          appWindow.outerSize(),
        ]);
        persistRect(pos0.x, pos0.y, size0.width, size0.height);
        // Subscribe to move + resize so subsequent changes persist live.
        unlistenMoved = await appWindow.onMoved(({ payload: pos }) => {
          appWindow.outerSize().then((size) => {
            persistRect(pos.x, pos.y, size.width, size.height);
          }).catch(() => { /* ignore */ });
        });
        unlistenResized = await appWindow.onResized(({ payload: size }) => {
          appWindow.outerPosition().then((pos) => {
            persistRect(pos.x, pos.y, size.width, size.height);
          }).catch(() => { /* ignore */ });
        });
      } catch (err) {
        console.warn('[App] window position persistence failed:', err);
      }
    })();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === 'k') {
        e.preventDefault();
        paletteInitialQuery = '';
        paletteOpen = !paletteOpen;
        return;
      }
      // Ctrl+T — open a new session tab. The empty-state hint advertises this;
      // wire it so the promise is real. !shiftKey keeps Ctrl+Shift+T free.
      if (e.ctrlKey && !e.shiftKey && (e.key === 't' || e.key === 'T')) {
        e.preventDefault();
        sm.addSession();
        return;
      }
      if (e.ctrlKey && e.shiftKey && e.key === 'M') {
        e.preventDefault();
        paletteInitialQuery = 'model';
        paletteOpen = true;
        return;
      }
      // Ctrl+Shift+P — open the unified command palette in Claude-command mode
      // (the leading "/" filters to /commands). The palette absorbed the old
      // slash launcher, so Ctrl+K and Ctrl+Shift+P open one surface.
      if (e.ctrlKey && e.shiftKey && (e.key === 'P' || e.key === 'p')) {
        e.preventDefault();
        paletteInitialQuery = '/';
        paletteOpen = true;
        return;
      }
      // Ctrl+Shift+K — drop a session marker (candidate 49d). Ctrl+M is
      // Carriage Return in a terminal, so the candidate's suggested Ctrl+M
      // would shadow Enter; Ctrl+Shift+K is terminal-safe. The marker is a
      // `status`/`session.marker` envelope — the session logger persists every
      // envelope, so it lands in the active session's .jsonl and surfaces as a
      // clickable point on the replay timeline (SessionsTabContent).
      if (e.ctrlKey && e.shiftKey && (e.key === 'K' || e.key === 'k')) {
        e.preventDefault();
        const now = new Date();
        const label = `Mark @ ${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}`;
        void publish('status', 'session.marker', {
          marker_id: `mk-${now.getTime()}`,
          label,
          note: null,
        }).catch((err) => console.warn('[App] session marker publish failed:', err));
        return;
      }
      if (e.ctrlKey && e.key === 'b') {
        e.preventDefault();
        toggleCockpit();
      }
      if (e.ctrlKey && e.shiftKey && e.key === '?') {
        e.preventDefault();
        shortcutsOpen = !shortcutsOpen;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'L' || e.key === 'l')) {
        e.preventDefault();
        popouts.summon({ content: { kind: 'llm-ensemble' }, width: 'min(1100px, 95vw)' });
      }
      // Split-pane shortcuts — only when a session tab is active.
      if (sm.active.kind === 'session') {
        // Ctrl+Shift+E — split focused pane horizontally (top / bottom)
        if (e.ctrlKey && e.shiftKey && (e.key === 'E' || e.key === 'e')) {
          e.preventDefault();
          sm.handleSplit(sm.focusedSessionId, 'hsplit');
        }
        // Ctrl+Shift+D — split focused pane vertically (left | right)
        if (e.ctrlKey && e.shiftKey && (e.key === 'D' || e.key === 'd')) {
          e.preventDefault();
          sm.handleSplit(sm.focusedSessionId, 'vsplit');
        }
        // Ctrl+Shift+W — close the focused pane
        if (e.ctrlKey && e.shiftKey && (e.key === 'W' || e.key === 'w')) {
          e.preventDefault();
          sm.handleClosePane(sm.focusedSessionId);
        }
      }
    };
    // Visible TitleBar affordances (⌘K / ?) dispatch these so the chrome can
    // open the palette + shortcuts overlay without prop-threading through it.
    const onOpenPalette = () => { paletteInitialQuery = ''; paletteOpen = true; };
    const onOpenCommands = () => { paletteInitialQuery = '/'; paletteOpen = true; };
    const onOpenShortcuts = () => { shortcutsOpen = true; };
    const onOpenFailures = () => { failuresOpen = true; };
    const onOpenModelSwap = () => { paletteInitialQuery = 'model'; paletteOpen = true; };
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('rift:open-palette', onOpenPalette);
    window.addEventListener('rift:open-commands', onOpenCommands);
    window.addEventListener('rift:open-shortcuts', onOpenShortcuts);
    window.addEventListener('rift:open-failures', onOpenFailures);
    window.addEventListener('rift:open-model-swap', onOpenModelSwap);

    return () => {
      unlistenDetached?.();
      unlistenReattached?.();
      unlistenMoved?.();
      unlistenResized?.();
      window.removeEventListener('rift:config-changed', onConfigChanged);
      window.removeEventListener('rift:show-welcome', showWelcomeGuide);
      window.removeEventListener('rift:open-palette', onOpenPalette);
      window.removeEventListener('rift:open-commands', onOpenCommands);
      window.removeEventListener('rift:open-shortcuts', onOpenShortcuts);
      window.removeEventListener('rift:open-failures', onOpenFailures);
      window.removeEventListener('rift:open-model-swap', onOpenModelSwap);
      window.removeEventListener('keydown', onKeyDown);
    };
  });

  async function installUpdate(): Promise<void> {
    if (!availableUpdate || updateInstalling) return;
    updateInstalling = true;
    updateError = null;
    try {
      await availableUpdate.downloadAndInstall();
      // The Windows installer typically restarts the app itself. If it
      // doesn't, the user can manually relaunch — the new version takes
      // effect on next start. We deliberately skip plugin-process here
      // (one fewer dep, one fewer capability surface) for v1.
    } catch (err) {
      updateError = String(err);
      updateInstalling = false;
    }
  }

  function dismissUpdate(): void {
    availableUpdate = null;
    updateError = null;
  }

</script>

<div class="app-shell">
  <TitleBar />

  {#if availableUpdate}
    <!-- D-013 — Update-available banner. Slim amber strip between TitleBar
         and TabBar. Visible only when latest.json reports a newer signed
         release than the running build. -->
    <aside class="update-banner" class:installing={updateInstalling} class:error={updateError !== null}>
      {#if updateError}
        <span class="update-glyph">◇</span>
        <span class="update-text">Update install failed: {updateError}</span>
        <button type="button" class="update-btn" onclick={dismissUpdate}>Dismiss</button>
      {:else}
        <span class="update-glyph">↗</span>
        <span class="update-text">
          Update available — v{availableUpdate.version}
          {#if availableUpdate.body}<span class="update-body">· {availableUpdate.body.slice(0, 80)}{availableUpdate.body.length > 80 ? '…' : ''}</span>{/if}
        </span>
        <button type="button" class="update-btn update-btn-primary" disabled={updateInstalling} onclick={installUpdate}>
          {updateInstalling ? 'Installing…' : 'Install'}
        </button>
        <button type="button" class="update-btn" disabled={updateInstalling} onclick={dismissUpdate}>Later</button>
      {/if}
    </aside>
  {/if}

  <TabBar
    sessions={sm.sessions}
    groupedNotifs={nm.groupedNotifs}
    active={sm.active}
    {tickNow}
    promotedId={nm.promoted}
    onActivateSession={sm.activateSession}
    onActivateNotif={nm.activateNotif}
    onCloseSession={sm.closeSession}
    onAddSession={sm.addSession}
    onReorderSession={sm.reorderSession}
    onRenameSession={sm.renameSession}
    onToggleNotif={nm.toggleNotif}
    onDemote={nm.demoteTab}
    onManageNotifs={nm.openNotifManager}
    onDetach={nm.detachNotif}
    detachedIds={nm.detachedIds}
    multiProject={sm.multiProject}
    {cockpitCollapsed}
    {alertTriggered}
    deadSessions={sm.deadSessions}
    onToggleCockpit={toggleCockpit}
  />

  <!-- Phase 6.2 — always-on cockpit: left = terminal surface, right = file tree -->
  <main class="cockpit">
    <!-- Left half: existing terminal / notification / empty surfaces + optional promoted pane -->
    <div class="cockpit-left" class:split={nm.promoted !== null} class:full-width={cockpitCollapsed || cockpitDetached}>
      <div class="main-left">
        <!-- session terminals — keep all alive, hide inactive ones to preserve scrollback.
             Each session renders its layout tree via TerminalGrid so split panes work
             without requiring any backend changes. Only the active session is visible;
             inactive ones are display:none to preserve scrollback while unmounted. -->
        {#each sm.sessions as s (s.id)}
          <div
            class="surface"
            class:visible={sm.active.kind === 'session' && sm.active.id === s.id}
          >
            <TerminalGrid
              node={s.layout}
              projectPath={s.projectPath}
              bind:focusedId={sm.focusedSessionId}
              onSplit={sm.handleSplit}
              onClose={sm.handleClosePane}
              onFocus={(id) => { sm.focusedSessionId = id; }}
              onPtyExited={sm.markPaneExited}
            />
          </div>
        {/each}


        <!-- empty state — no tabs open -->
        {#if sm.active.kind === 'empty'}
          <div class="surface visible empty-state">
            <div class="empty-card">
              <div class="empty-glyph">◆</div>
              <div class="empty-title">No terminal open</div>
              <div class="empty-hint">Press <kbd>Ctrl+T</kbd> or click <kbd>+</kbd> to open a new shell</div>
            </div>
          </div>
        {/if}
      </div>

      <!-- Phase 3.5a — promoted notification side-pane.
           Independent NotificationPane / AegisTabContent instance (not driven by `active`).
           Re-keyed on the promoted id so the subscription resets cleanly
           when one promoted tab replaces another. -->
      {#if nm.promotedTab}
        <Splitter
          direction="vertical"
          storageKey="rift.promoted.width_px"
          unit="px"
          bind:size={promotedPaneWidth}
          min={280}
          max={800}
          onDblClick={() => (promotedPaneWidth = 420)}
        />
        {#key nm.promotedTab.id}
          <aside class="promoted-pane" style="flex: 0 0 {promotedPaneWidth}px;">
            {#if nm.promotedTab.id === 'aegis'}
              <AegisTabContent severityThreshold={nm.thresholdFor('aegis')} onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'index'}
              <IndexTabContent onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'bustail'}
              <BusTailTabContent severityThreshold={nm.thresholdFor('bustail')} onDragBack={nm.demoteTab} {correlationIndex} />
            {:else if nm.promotedTab.id === 'todo'}
              <TodoTabContent onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'git'}
              <GitTabContent onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'agents'}
              <AgentsTabContent severityThreshold={nm.thresholdFor('agents')} onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'filesystem'}
              <FsTabContent severityThreshold={nm.thresholdFor('filesystem')} onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'mcp'}
              <McpTabContent severityThreshold={nm.thresholdFor('mcp')} onDragBack={nm.demoteTab} {correlationIndex} />
            {:else if nm.promotedTab.id === 'llm-activity'}
              <LlmActivityTabContent severityThreshold={nm.thresholdFor('llm-activity')} onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'sentinel'}
              <SentinelTabContent severityThreshold={nm.thresholdFor('sentinel')} onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'health'}
              <HealthTabContent severityThreshold={nm.thresholdFor('health')} onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'sessions'}
              <SessionsTabContent onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'cmd-intelligence'}
              <CommandIntelligencePanel
                project={sm.sessions.find(s => s.id === (sm.active.kind === 'session' ? sm.active.id : 0))?.projectPath?.split(/[\\/]/).pop() ?? null}
                cwd={sm.sessions.find(s => s.id === (sm.active.kind === 'session' ? sm.active.id : 0))?.projectPath ?? null}
                onDragBack={nm.demoteTab}
              />
            {:else if nm.promotedTab.id === 'integrations'}
              <IntegrationInspector onDragBack={nm.demoteTab} />
            {:else if nm.promotedTab.id === 'feature-pipeline'}
              <FeaturePipelineTabContent onDragBack={nm.demoteTab} />
            {:else}
              <NotificationPane
                title={nm.promotedTab.title}
                icon={nm.promotedTab.icon}
                accent={nm.notifAccent(nm.promotedTab.id)}
                categoryFilter={nm.CATEGORY_BY_NOTIF[nm.promotedTab.id]}
                severityThreshold={nm.thresholdFor(nm.promotedTab.id)}
                onDragBack={nm.demoteTab}
              />
            {/if}
          </aside>
        {/key}
      {/if}
    </div>

    <!-- Right half: IndexGraph (top 55%) + File Tree (bottom 45%).
         Phase 8.7d: when cockpit is detached, the entire right pane is
         removed from the layout so the terminal expands to full width.
         Reattach is handled by the cockpit window's own DOCK button (or its
         X — both intercepted to .hide()). The TitleBar's chip swaps to a
         ↙ DOCK button while detached, so the user always has a path back. -->
    {#if !cockpitDetached && !cockpitCollapsed}
      <!-- Phase 8.7e — vertical splitter between terminal (cockpit-left) and
           cockpit-right (graph + tree). Drag to resize terminal/cockpit
           widths; double-click to reset to default 420px. Width persists. -->
      <Splitter
        direction="vertical"
        storageKey="rift.cockpit.right_width_px"
        unit="px"
        bind:size={cockpitRightWidth}
        min={280}
        max={800}
        onDblClick={() => (cockpitRightWidth = 420)}
      />

      <div class="cockpit-right" style="flex: 0 0 {cockpitRightWidth}px;">
        <!-- Phase 8.4 — Graph pane (top slot). IndexGraph.svelte renders width:100%/height:100%
             inside .graph-pane; border-bottom is the horizontal divider per mockup #3. -->
        <div class="graph-pane" style="flex: 0 0 {graphHeightPct}%;">
          <div class="pane-header">
            <span>INDEX</span>
            <span class="meta">vault index · live</span>
          </div>
          <div class="graph-body">
            <IndexGraph />
          </div>
        </div>

        <!-- Phase 8.7e — horizontal splitter between graph and tree.
             Drag to resize their height ratio; double-click resets to 55/45. -->
        <Splitter
          direction="horizontal"
          storageKey="rift.cockpit.graph_height_pct"
          unit="percent"
          bind:size={graphHeightPct}
          min={20}
          max={80}
          onDblClick={() => (graphHeightPct = 55)}
        />

        <!-- Tree pane (bottom slot) — all existing Tree props preserved unchanged -->
        <div class="tree-pane">
          <div class="pane-header">
            <span>FILE TREE</span>
            <span class="meta">{nodeCount} files · {watchedPathLabel}</span>
          </div>
          <div class="tree-body">
            <Tree bind:nodeCount bind:watchedPathLabel />
          </div>
        </div>
      </div>
    {/if}
  </main>

  <!-- Phase 2 / N4 — ambient status chrome. Mode-hint line (context keybinds)
       on the left, agent/error/mcp ambient indicators on the right. Sits as a
       thin band directly above the two-row StatusLine. -->
  <ModeHintBar
    activeKind={sm.active.kind === 'session' ? 'session' : 'empty'}
    {cockpitCollapsed}
    errorCount={errorAmbient}
    agentCount={agentAmbient}
    mcpCount={mcpAmbient}
    onPromote={nm.activateNotif}
  />

  <StatusLine
    dir={statusDir || '—'}
    model={statusModel || '—'}
    thinking={statusThinking || '—'}
    ctx={statusCtx || '—'}
    session={statusSession || '—'}
    repo={statusRepo || '—'}
    git={statusGit || '—'}
    skill={aegisSkillName || ccSkill || '—'}
    effort={aegisEffort || ccEffort || '—'}
    sessionUse={statusSessionUse || '—'}
    week={statusWeek || '—'}
    cpu={statusCpu}
    ram={statusRam}
    visibility={statuslineConfig}
  />

  <!-- Phase 3.5b — pop-out stack (§10.5). Renders one overlay per entry
       in the global `popouts` store; only the topmost responds to Esc /
       backdrop click. Chassis-only in 3.5b — no production summon calls
       yet; first consumer (rule editor / file viewer / agent confirm)
       lands in Phase 5+. -->
  {#each popouts.entries as entry, i (entry.id)}
    <Popout
      {entry}
      isTop={i === popouts.entries.length - 1}
      stackIndex={i}
    />
  {/each}

  {#if paletteOpen}
    <CommandPalette
      onclose={() => { paletteOpen = false; paletteInitialQuery = ''; }}
      onActivateNotif={nm.activateNotif}
      onNewTab={sm.addSession}
      onToggleCockpit={toggleCockpit}
      initialQuery={paletteInitialQuery}
    />
  {/if}

  {#if shortcutsOpen}
    <ShortcutOverlay onclose={() => { shortcutsOpen = false; }} />
  {/if}

  {#if failuresOpen}
    <FailuresPanel onclose={() => { failuresOpen = false; }} />
  {/if}

  {#if welcomeOpen}
    <WelcomeOverlay ondismiss={dismissWelcome} />
  {/if}

  {#if sm.pendingCloseId !== null}
    <div class="close-confirm-backdrop" role="presentation" onclick={sm.cancelClose}>
      <!-- svelte-ignore a11y_click_events_have_key_events -- alertdialog; keyboard handled by buttons inside -->
      <div
        class="close-confirm-dialog"
        onclick={(e) => e.stopPropagation()}
        role="alertdialog"
        tabindex="-1"
        aria-modal="true"
        aria-label="Confirm close tab"
      >
        <p>This tab has a running process. Close anyway?</p>
        <div class="close-confirm-actions">
          <button type="button" class="rift-btn" onclick={sm.cancelClose}>Cancel</button>
          <button type="button" class="rift-btn danger" onclick={sm.confirmClose}>Close</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  /* Phase 6.2 — cockpit outer shell: terminal left, file tree right. */
  .cockpit {
    flex: 1;
    display: flex;
    flex-direction: row;
    min-height: 0;
    /* Vantablack seam — the gutter/splitter between the terminal and the
       cockpit sidebar sits on the same black void as the terminal, so the
       matte sidebar reads as furniture set against pure black glass. */
    background: var(--term-bg);
    position: relative;
    overflow: hidden;
  }

  .cockpit-left {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-right: none;
  }
  .cockpit-left.full-width {
    border-right: none;
  }
  /* When a notif tab is promoted, the left column switches to row layout so
     the promoted pane sits alongside the main terminal area. */
  .cockpit-left.split {
    flex-direction: row;
  }

  .main-left {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }

  .promoted-pane {
    /* flex-basis set inline via promotedPaneWidth state (splitter-controlled) */
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
    /* Matte panel + grain, separated from the vantablack terminal by a
       visible warm border (not just an inset shadow). */
    border-left: 1px solid var(--border-active);
    box-shadow: var(--depth-inset);
    background-color: var(--bg-panel);
    background-image: var(--grain);
    transition: box-shadow var(--duration-med) var(--ease-out);
  }
  .promoted-pane:focus-within {
    box-shadow: var(--depth-inset), var(--glow-active-inset);
  }

  /* Right half — graph + tree stacked column (Phase 8.4).
     flex-basis is set inline by App.svelte via cockpitRightWidth state
     (Phase 8.7e — Splitter-controlled, persisted to localStorage). */
  .cockpit-right {
    display: flex;
    flex-direction: column;
    /* Matte sidebar fill + grain. A warm left border draws the hard seam
       between the vantablack terminal and the chrome. */
    background-color: var(--bg-panel);
    background-image: var(--grain);
    border-left: 1px solid var(--border-active);
    min-width: 0;
  }

  .graph-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    /* Transparent — shares the cockpit-right matte fill + grain as one
       continuous surface. The amber divider below separates it from the tree. */
    background: transparent;
    border-bottom: 2px solid var(--amber-faint);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
    transition: box-shadow var(--duration-med) var(--ease-out);
  }
  /* Active-panel amber glow — INDEX lights up when you're working in it. */
  .graph-pane:focus-within {
    box-shadow: var(--glow-active-inset), 0 2px 8px rgba(0, 0, 0, 0.5);
  }

  /* Graph body — fills the remaining height below graph pane-header.
     IndexGraph SVG renders at width:100%/height:100% inside this container. */
  .graph-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  /* Tree pane (bottom slot, 45% of cockpit-right height).
     flex: 1 1 45% matches mockup #3 .gui-tree canonical sizing.
     Phase 8.7g.5 — bg-base matches graph-pane + terminal so the cockpit
     reads as a single visual surface instead of having a panel-tinted
     tree zone (user feedback: "different background than terminal/nodes").
     Phase 8.7q.3 — fixed second-declaration override that was silently
     reintroducing the bg-panel tint and re-creating the original mismatch. */
  .tree-pane {
    flex: 1 1 45%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    /* Transparent — shares the cockpit-right matte fill + grain. */
    background: transparent;
    transition: box-shadow var(--duration-med) var(--ease-out);
  }
  /* Active-panel amber glow — FILE TREE lights up when focused. */
  .tree-pane:focus-within {
    box-shadow: var(--glow-active-inset);
  }

  .pane-header {
    height: var(--space-2xl);
    padding: 0 var(--space-14);
    /* Raised header — elevated matte tier + grain + a faint amber lit edge,
       so the header sits clearly above the panel fill below it. */
    background-color: var(--bg-elevated);
    background-image:
      linear-gradient(180deg, rgba(255, 200, 64, 0.06), rgba(255, 200, 64, 0.015)),
      var(--grain);
    border-bottom: 1px solid var(--amber-faint);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--amber-bright);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    flex-shrink: 0;
    user-select: none;
  }
  .pane-header .meta {
    color: var(--amber-dim);
    font-weight: 400;
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
  }

  /* Tree body scrolls; Tree.svelte owns the SVG. */
  .tree-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-xs) 0;
  }
  .tree-body::-webkit-scrollbar { width: 5px; }
  .tree-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .surface {
    display: none;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    /* Phase 8.7q.3 — see .promoted-pane comment. */
    min-width: 0;
  }
  .surface.visible { display: flex; }

  .empty-state {
    align-items: center;
    justify-content: center;
    color: var(--amber-faint);
  }
  .empty-card {
    text-align: center;
    user-select: none;
    padding: var(--space-2xl);
  }
  .empty-glyph {
    font-size: var(--text-display);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-strong);
    margin-bottom: var(--space-lg);
  }
  .empty-title {
    color: var(--amber-warm);
    font-size: var(--text-lg);
    letter-spacing: 0.18em;
    text-transform: uppercase;
    margin-bottom: var(--space-8);
  }
  .empty-hint {
    color: var(--amber-dim);
    font-size: var(--text-sm);
  }
  .empty-hint kbd {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-primary);
    padding: 1px var(--space-sm);
    font-family: inherit;
    font-size: var(--text-xs);
  }

  /* D-013 — Update-available banner. Slim row, amber-bordered, sits
     between TitleBar (32px) and TabBar (36px). 28px height keeps it
     compact; only renders when availableUpdate is truthy. */
  .update-banner {
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: 0 var(--space-lg);
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--amber-primary);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    user-select: none;
  }
  .update-banner.installing {
    border-bottom-color: var(--amber-bright);
  }
  .update-banner.error {
    border-bottom-color: var(--term-red);
    color: var(--term-red);
  }
  .update-glyph {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber);
    font-size: var(--text-md);
  }
  .update-text {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .update-body {
    color: var(--amber-faint);
  }
  .update-btn {
    background: transparent;
    border: 1px solid var(--amber-dim);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: var(--text-xs);
    letter-spacing: 0.08em;
    padding: 3px var(--space-md);
    cursor: pointer;
    border-radius: var(--radius-md, 4px);
    transition: color 0.12s, border-color 0.12s;
  }
  .update-btn:not(:disabled):hover {
    color: var(--amber-primary);
    border-color: var(--amber-primary);
  }
  .update-btn-primary {
    color: var(--amber-warm);
    border-color: var(--amber-primary);
  }
  .update-btn-primary:not(:disabled):hover {
    color: var(--amber-bright);
    border-color: var(--amber-bright);
    text-shadow: var(--glow-amber);
  }
  .update-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* U-02: close confirmation dialog */
  .close-confirm-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .close-confirm-dialog {
    background: var(--bg-base, #1a1610);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: var(--radius-md, 6px);
    padding: var(--space-xl) var(--space-24);
    min-width: 300px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }
  .close-confirm-dialog p {
    color: var(--term-white, #e8e4d8);
    font-size: 14px;
    margin: 0 0 var(--space-lg) 0;
  }
  .close-confirm-actions {
    display: flex;
    gap: var(--space-8);
    justify-content: flex-end;
  }
  .close-confirm-actions .rift-btn.danger {
    border-color: var(--term-red, #ff4848);
    color: var(--term-red, #ff4848);
  }
  .close-confirm-actions .rift-btn.danger:hover {
    background: var(--bg-red-tint-hover);
  }
</style>
