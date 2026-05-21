<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window';
  import { check, type Update } from '@tauri-apps/plugin-updater';
  import TitleBar from './lib/TitleBar.svelte';
  import TabBar, {
    type SessionTab,
    type NotifTab,
    type ActiveSurface,
  } from './lib/TabBar.svelte';
  import Terminal from './lib/Terminal.svelte';
  import NotificationPane from './lib/NotificationPane.svelte';
  import AegisTabContent from './lib/AegisTabContent.svelte';
  import IndexTabContent from './lib/IndexTabContent.svelte';
  import BusTailTabContent from './lib/BusTailTabContent.svelte';
  import TodoTabContent from './lib/TodoTabContent.svelte';
  import GitTabContent from './lib/GitTabContent.svelte';
  import AgentsTabContent from './lib/AgentsTabContent.svelte';
  import FsTabContent from './lib/FsTabContent.svelte';
  import McpTabContent from './lib/McpTabContent.svelte';
  import SentinelTabContent from './lib/SentinelTabContent.svelte';
  import SessionsTabContent from './lib/SessionsTabContent.svelte';
  import StatusLine from './lib/StatusLine.svelte';
  import Popout from './lib/Popout.svelte';
  import { popouts } from './lib/popouts.svelte';
  import Tree from './lib/Tree.svelte';
  import IndexGraph from './lib/IndexGraph.svelte';
  import Splitter from './lib/Splitter.svelte';
  import { subscribe, publish, signalBusReady, type Category } from './lib/bus';
  import { enrichmentStore } from './lib/enrichmentStore.svelte';
  import { parseSeverity, type SeverityLevel } from './lib/notifFilter';
  import type { RiftConfig as RiftConfigType, StatusLineConfig, AlertRule } from './lib/riftConfig';
  import { SparklineBuffer } from './lib/SparklineBuffer';
  import { evaluateRule, triggerAction } from './lib/alertRules';
  import { CorrelationIndex } from './lib/correlationIndex';
  import { sectionCatalog } from './lib/sectionCatalog.svelte';
  import CommandPalette from './lib/CommandPalette.svelte';
  import ShortcutOverlay from './lib/ShortcutOverlay.svelte';
  import WelcomeOverlay from './lib/WelcomeOverlay.svelte';

  // D-021: Tab→category mapping is now derived from the section catalog.
  // `sectionCatalog.categoryMap` builds the reverse map dynamically.
  // Legacy accessor for code that still references CATEGORY_BY_NOTIF:
  const CATEGORY_BY_NOTIF: Record<string, Category | undefined> = $derived.by(() => {
    const result: Record<string, Category | undefined> = {};
    for (const desc of sectionCatalog.allTabs) {
      if (desc.category) result[desc.id] = desc.category;
    }
    return result;
  });

  // ----- session tabs -----
  let nextSessionId = 1;
  let initialProjectRoot = $state<string | null>(null);
  let sessions = $state<SessionTab[]>([{ id: 0, title: 'rift', projectPath: null }]);

  // ----- notification tabs (§10.7 + §10.16 catalog-driven) -----
  // D-021: Tab strip is initialized from the section catalog registry.
  // Capability-gating (§10.7): `detectedByDefault` in the catalog drives
  // the initial `detected` flag. Runtime detection flips on first envelope.
  // The catalog is the source of truth for which tabs exist and their metadata;
  // the `notifs` array holds runtime state (unreadCount, lastActivityTs, detected).
  let notifs = $state<NotifTab[]>(
    sectionCatalog.allTabs.map((desc) => ({
      id: desc.id,
      title: desc.title,
      icon: desc.icon,
      enabled: true,
      detected: desc.detectedByDefault,
      unreadCount: 0,
      lastActivityTs: null,
    }))
  );

  // ----- notification filter thresholds -----
  // Loaded from RiftConfig.notif_filters on mount + rift:config-changed.
  // BusTail defaults to 'debug' (firehose by design) unless explicitly overridden.
  let notifFilterDefault = $state<SeverityLevel>('info');
  let notifFilterPerTab = $state<Record<string, SeverityLevel>>({});

  function thresholdFor(tabId: string): SeverityLevel {
    if (tabId in notifFilterPerTab) return notifFilterPerTab[tabId];
    if (tabId === 'bustail') return 'debug';
    return notifFilterDefault;
  }

  async function loadNotifFilters() {
    try {
      const cfg = await invoke<RiftConfigType>('config_get');
      const nf = cfg?.notif_filters;
      notifFilterDefault = parseSeverity(nf?.default_threshold);
      const pt: Record<string, SeverityLevel> = {};
      if (nf?.per_tab) {
        for (const [k, v] of Object.entries(nf.per_tab)) {
          pt[k] = parseSeverity(v);
        }
      }
      notifFilterPerTab = pt;
    } catch (err) {
      console.warn('Failed to load notification filters:', err);
    }
  }

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

  // Reverse map for envelope routing — Category → notif tab id. Used by the
  // master subscription below to attribute incoming envelopes to the right
  // tab. The forward map (notif id → Category) is in CATEGORY_BY_NOTIF.
  const NOTIF_BY_CATEGORY: Record<string, string> = {
    hook: 'hooks',
    system: 'errors',
    pty: 'commands',
    aegis: 'aegis',
    index: 'index',
    agent: 'agents',
    fs: 'filesystem',
    mcp: 'mcp',
    sentinel: 'sentinel',
  };

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

  // ----- command palette -----
  let paletteOpen = $state(false);

  // ----- shortcut overlay -----
  let shortcutsOpen = $state(false);

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

  // Expose to SettingsPanel via window event.
  if (typeof window !== 'undefined') {
    window.addEventListener('rift:show-welcome', showWelcomeGuide);
  }

  // ----- which surface is in the main pane -----
  let active = $state<ActiveSurface>({ kind: 'session', id: 0 });

  // ----- promoted notification tab (Phase 3.5a) -----
  // Holds the id of the single promoted tab, or null when no pane is docked.
  // Max-1 promotion is enforced structurally — string|null can hold one id.
  let promoted = $state<string | null>(null);

  // ----- handlers -----
  function activateSession(id: number) {
    active = { kind: 'session', id };
  }
  function activateNotif(id: string) {
    // Notif tabs NEVER replace the terminal/session surface in the main slot.
    // Click toggles promotion alongside the main session surface — the notif's
    // content lives in `aside.promoted-pane` next to the terminal, never in
    // place of it.
    //
    // Detach-to-separate-window is now wired via notif_window.rs (pool of 4
    // pre-built hidden Tauri windows). The pop-out button and drag-out-of-
    // window gesture call `detachNotif(id)` which invokes `notif_detach`.
    //
    // The `active` state machine never becomes `notification` — notif tabs
    // promote to a side pane via `promoted`, never replace the main surface.
    const wasPromoted = promoted === id;
    promoted = wasPromoted ? null : id;
    // §10.9 ack — promoting a tab into the side pane counts as viewing it.
    if (!wasPromoted) ackUnread(id);
  }
  function addSession(opts?: { pickProject?: boolean }) {
    if (opts?.pickProject) {
      openProjectPickerForNewTab();
      return;
    }
    const id = nextSessionId++;
    const activeSession = sessions.find(
      (s) => active.kind === 'session' && s.id === active.id,
    );
    const inheritedPath = activeSession?.projectPath ?? initialProjectRoot;
    sessions = [...sessions, { id, title: `shell ${id + 1}`, projectPath: inheritedPath }];
    active = { kind: 'session', id };
  }

  function openProjectInNewTab(path: string) {
    const id = nextSessionId++;
    sessions = [...sessions, { id, title: `shell ${id + 1}`, projectPath: path }];
    active = { kind: 'session', id };
  }

  function openProjectPickerForNewTab() {
    popouts.summon({
      content: {
        kind: 'project-picker',
        onSelect: openProjectInNewTab,
      },
    });
  }
  function closeSession(id: number) {
    sessions = sessions.filter((s) => s.id !== id);
    if (active.kind === 'session' && active.id === id) {
      // Activate the rightmost remaining session, or fall through to empty.
      const last = sessions.at(-1);
      active = last ? { kind: 'session', id: last.id } : { kind: 'empty' };
    }
  }
  // ----- project-per-tab: active project follows active session -----
  const activeProjectPath = $derived.by(() => {
    const a = active;
    if (a.kind !== 'session') return initialProjectRoot;
    const s = sessions.find((s) => s.id === a.id);
    return s?.projectPath ?? initialProjectRoot;
  });

  const multiProject = $derived(
    new Set(sessions.map((s) => s.projectPath)).size > 1,
  );


  let lastSwappedPath: string | null = null;
  let swapTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const path = activeProjectPath;
    if (!path || path === lastSwappedPath) return;
    lastSwappedPath = path;
    // Debounce: let Terminal.onMount's pty_start complete before project_swap
    // restarts the fs watcher and floods the bus. 100ms is enough for the
    // Terminal to call pty_start; project_swap's watcher restart then happens
    // after the PTY is already draining output. Also collapses rapid tab switches.
    clearTimeout(swapTimer);
    swapTimer = setTimeout(() => {
      invoke('project_swap', { path }).catch((err: unknown) =>
        console.warn('[rift] tab-switch project_swap failed:', err),
      );
    }, 100);
  });

  function toggleNotif(id: string) {
    notifs = notifs.map((n) => (n.id === id ? { ...n, enabled: !n.enabled } : n));
    const enabled = notifs.find((n) => n.id === id)?.enabled;
    if (!enabled && promoted === id) {
      promoted = null;
    }
    persistNotifOrder();
  }

  // Phase 8.7h — reset all notif tabs to enabled. Capability-driven
  // detected:false flags stay as-is; reset only flips enabled.
  function resetNotifs() {
    notifs = notifs.map((n) => ({ ...n, enabled: true }));
    persistNotifOrder();
  }

  // Phase 8.7j — drag-to-reorder + localStorage-persisted order. The strip
  // sends `(srcId, dstId)`; we splice src to dst's index. Order survives
  // reloads via `rift.notifs.order` (an array of tab ids). New tabs added
  // in future builds append at the end so order updates are forward-safe.
  const NOTIF_ORDER_KEY = 'rift.notifs.order';
  const WORKSPACE_KEY = 'rift.workspace';
  function reorderNotif(srcId: string, dstId: string) {
    if (srcId === dstId) return;
    const srcIdx = notifs.findIndex((n) => n.id === srcId);
    const dstIdx = notifs.findIndex((n) => n.id === dstId);
    if (srcIdx < 0 || dstIdx < 0) return;
    const next = notifs.slice();
    const [moved] = next.splice(srcIdx, 1);
    next.splice(dstIdx, 0, moved);
    notifs = next;
    persistNotifOrder();
  }
  function persistNotifOrder() {
    try {
      const order = notifs.map((n) => ({ id: n.id, enabled: n.enabled }));
      localStorage.setItem(NOTIF_ORDER_KEY, JSON.stringify(order));
    } catch {
      // localStorage unavailable (private mode etc.) — silent best-effort.
    }
  }
  function persistWorkspace() {
    try {
      localStorage.setItem(WORKSPACE_KEY, JSON.stringify({
        promoted: promoted,
      }));
    } catch {}
  }
  function applyPersistedWorkspace() {
    try {
      const raw = localStorage.getItem(WORKSPACE_KEY);
      if (!raw) return;
      const ws = JSON.parse(raw);
      if (typeof ws.promoted === 'string' && notifs.some((n) => n.id === ws.promoted && n.enabled)) {
        promoted = ws.promoted;
      }
    } catch {}
  }
  function applyPersistedNotifOrder() {
    try {
      const raw = localStorage.getItem(NOTIF_ORDER_KEY);
      if (!raw) return;
      const order = JSON.parse(raw) as unknown;
      if (!Array.isArray(order)) return;
      // Build a map of persisted enabled state. Supports both formats:
      // old: ["errors", "hooks", ...] — enabled state not saved
      // new: [{id: "errors", enabled: true}, ...] — enabled state saved
      const enabledMap = new Map<string, boolean>();
      const orderedIds: string[] = [];
      for (const entry of order) {
        if (typeof entry === 'string') {
          orderedIds.push(entry);
        } else if (entry && typeof entry === 'object' && typeof (entry as {id?: unknown}).id === 'string') {
          const e = entry as { id: string; enabled?: boolean };
          orderedIds.push(e.id);
          if (typeof e.enabled === 'boolean') enabledMap.set(e.id, e.enabled);
        }
      }
      const idToTab = new Map(notifs.map((n) => [n.id, n]));
      const reordered: typeof notifs = [];
      for (const id of orderedIds) {
        const tab = idToTab.get(id);
        if (tab) {
          const persisted = enabledMap.get(id);
          reordered.push(persisted !== undefined ? { ...tab, enabled: persisted } : tab);
          idToTab.delete(id);
        }
      }
      // Append any tabs not present in the saved order (new builds).
      for (const tab of idToTab.values()) reordered.push(tab);
      if (reordered.length === notifs.length) {
        notifs = reordered;
      }
    } catch {
      // Corrupt JSON — ignore and use defaults.
    }
  }

  // Phase 8.7h — open the notif manager popout. Passes a getTabs getter
  // so the popout sees fresh state on every render (notifs reassigns
  // immutably on each toggle).
  function openNotifManager() {
    popouts.summon({
      content: {
        kind: 'notif-manager',
        getTabs: () => notifs.map((n) => ({
          id: n.id,
          title: n.title,
          icon: n.icon,
          enabled: n.enabled,
          detected: n.detected,
        })),
        onToggle: toggleNotif,
        onReset: resetNotifs,
      },
      width: 'min(560px, 80vw)',
    });
  }

  // §10.9 — viewing or promoting a notif tab acknowledges its badge.
  // Called from activateNotif (above) and promoteTab (below).
  function ackUnread(id: string) {
    notifs = notifs.map((n) => (n.id === id && n.unreadCount > 0 ? { ...n, unreadCount: 0 } : n));
  }

  // Phase 3.5a — promote a notif tab to the right-side pane.
  // Reassigning `promoted` enforces max-1 (a 2nd promote auto-replaces the 1st).
  // `active` is never `notification` (notif tabs only ever live in `promoted`),
  // so the prior fallback-recompute branch was unreachable and has been removed.
  function promoteTab(id: string) {
    promoted = id;
    ackUnread(id); // §10.9 — promotion = view, clear the badge
    persistWorkspace();
  }
  function demoteTab() {
    promoted = null;
    // active stays as-is — user clicks the tab in the strip to view it again.
    persistWorkspace();
  }

  function notifAccent(id: string): 'amber' | 'cyan' | 'purple' | 'red' {
    if (id === 'hooks') return 'cyan';
    if (id === 'errors') return 'red';
    return 'amber';
  }
  // The promoted tab's data — looked up fresh from `notifs` so its
  // enabled/title/icon stay reactive with toggles.
  const promotedTab = $derived.by(() => {
    if (promoted === null) return undefined;
    return notifs.find((n) => n.id === promoted);
  });

  // Notification tab detach-to-window state.
  // Tracks which tabs are currently hosted in their own Tauri windows.
  let detachedIds = $state<Set<string>>(new Set());

  async function detachNotif(id: string) {
    if (detachedIds.has(id)) return;
    // If promoted, demote first — can't be both promoted and detached.
    if (promoted === id) promoted = null;

    const tab = notifs.find((n) => n.id === id);
    if (!tab) return;

    try {
      await invoke('notif_detach', {
        args: {
          tabId: id,
          category: CATEGORY_BY_NOTIF[id] ?? '',
          title: tab.title,
          icon: tab.icon,
          severityThreshold: thresholdFor(id),
        },
      });
      detachedIds = new Set([...detachedIds, id]);
    } catch (err) {
      console.warn('[App] notif_detach failed:', err);
    }
  }

  // Dock events arrive from Rust when the user clicks DOCK or closes the window.
  $effect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const u = await listen<{ tabId: string }>('notif_docked', (event) => {
        const next = new Set(detachedIds);
        next.delete(event.payload.tabId);
        detachedIds = next;
      });
      if (cancelled) {
        u();
      } else {
        unlisten = u;
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  });

  // Recover detach state on mount (reload-recovery).
  async function recoverDetachState() {
    try {
      const ids = await invoke<string[]>('notif_detach_status');
      if (ids.length > 0) detachedIds = new Set(ids);
    } catch {
      // best-effort
    }
  }

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

  $effect(() => {
    const t = setInterval(() => {
      tickNow = Date.now();
      for (const buf of alertBuffers.values()) buf.tick();
    }, 1000);
    return () => clearInterval(t);
  });

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
          const id = NOTIF_BY_CATEGORY[env.category];
          if (!id) return;
          // Pty has many kinds in the long run; today only "command.submitted"
          // is published, but kind-filter here makes the policy explicit so
          // future Category::Pty kinds (e.g. resize, exit) don't accidentally
          // count toward the Commands tab badge.
          if (env.category === 'pty' && env.kind !== 'command.submitted') return;
          // Phase 8.7q.4 — skip the `system/notif.tabs` snapshot publish
          // (broadcast by the $effect at ~line 452 whenever `notifs`
          // changes). It is a state-mirror, not an activity event.
          // Without this guard:
          //   $effect publishes notif.tabs → master subscriber bumps
          //   errors.unreadCount → notifs changes → $effect re-runs
          //   → publishes again ⟶ infinite loop @ ~100Hz, generating
          //   ~10k orphan-callback "[TAURI] Couldn't find callback id"
          //   warnings/sec on dead subscribers.
          if (env.category === 'system' && env.kind === 'notif.tabs') return;
          const now = Date.now();
          notifs = notifs.map((n) => {
            if (n.id !== id) return n;
            const inView = promoted === id;
            return {
              ...n,
              detected: true,
              lastActivityTs: now,
              unreadCount: inView ? 0 : n.unreadCount + 1,
            };
          });

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
              if (result.promote && promoted !== id) {
                promoted = id;
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
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // D-012 — live StatusLine segments.
  // Subscribes to Category::Status at App level so the StatusLine updates
  // regardless of which tab is active. Same svelte5-async-cleanup-via-sync-
  // shell-iife + cancelled-flag pattern as the SKILL subscription below.
  let statusDir = $state('');
  let statusGit = $state('');
  let statusRepo = $state('');
  let statusModel = $state('');
  let statusCtx = $state('');
  let statusSessionUse = $state('');
  let statusWeek = $state('');
  let statusSession = $state('');
  let ccSkill = $state('');
  let ccEffort = $state('');
  let statuslineConfig = $state<StatusLineConfig | undefined>(undefined);

  // Session elapsed timer — 1s tick for smooth clock display.
  const sessionStartMs = Date.now();
  $effect(() => {
    const tick = () => {
      const elapsed = Math.floor((Date.now() - sessionStartMs) / 1000);
      const h = Math.floor(elapsed / 3600);
      const m = Math.floor((elapsed % 3600) / 60);
      const s = elapsed % 60;
      statusSession = h > 0
        ? `${h}h ${String(m).padStart(2, '0')}m`
        : `${m}m ${String(s).padStart(2, '0')}s`;
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
          if (env.kind === 'usage') {
            const p = env.payload as {
              dir?: string; git?: string; repo?: string;
              model?: string; ctx_pct?: number; session_use_pct?: number; week_pct?: number;
              github_owner?: string; github_repo?: string;
              skill?: string; effort?: string;
            };
            if (p.dir  !== undefined) statusDir  = p.dir;
            if (p.git  !== undefined) statusGit  = p.git;
            if (p.github_owner && p.github_repo) {
              statusRepo = `${p.github_owner}/${p.github_repo}`;
            } else if (p.repo !== undefined) {
              statusRepo = p.repo;
            }
            if (p.model !== undefined) statusModel = p.model;
            if (p.ctx_pct !== undefined) statusCtx = `${p.ctx_pct}%`;
            if (p.session_use_pct !== undefined) statusSessionUse = `${p.session_use_pct}%`;
            if (p.week_pct !== undefined) statusWeek = `${p.week_pct}%`;
            if (p.skill !== undefined) ccSkill = p.skill;
            if (p.effort !== undefined) ccEffort = p.effort;
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

  // D-014 Phase B — publish `notif.tabs` snapshot to the bus whenever the
  // catalog changes (toggle / reorder / detected flip). MCP `notif_tabs`
  // tool reads the latest snapshot from the replay buffer; without this
  // producer the tool returns an empty list. Strips presentation-only
  // fields the bus consumer doesn't need (icon glyph stays — useful for
  // tool callers that surface tab affordances).
  $effect(() => {
    const tabs = notifs.map((n) => ({
      id: n.id,
      title: n.title,
      icon: n.icon,
      enabled: n.enabled,
      detected: n.detected,
      unread_count: n.unreadCount,
      last_activity_ts: n.lastActivityTs,
    }));
    void publish('system', 'notif.tabs', { tabs }).catch((err) => {
      console.warn('[App] notif.tabs publish failed:', err);
    });
  });

  // Phase 8.6.1 — Category::Index enrichment subscription.
  // Populates enrichmentStore so Tree.svelte (8.6.2) can join enrichment data
  // onto fs_path nodes. Same svelte5-async-cleanup-via-sync-shell-iife +
  // cancelled-flag pattern as the status and skill_loaded subscriptions above.
  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({ category: 'index' }, (env) => {
          if (env.kind === 'enrichment') {
            const p = env.payload as {
              fs_path: string;
              vault_id: string;
              vault_kind: string;
              tags: string[];
            };
            enrichmentStore.ingest(p);
          } else if (env.kind === 'vault.update') {
            const p = env.payload as { change_kind: string; vault_id: string };
            if (p.change_kind === 'deleted') {
              enrichmentStore.removeByVaultId(p.vault_id);
            }
            // Other change_kinds (e.g. "updated") are no-ops in 8.6.1;
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
      const root = await invoke<string>('project_root_get');
      initialProjectRoot = root;
      if (sessions[0] && sessions[0].projectPath === null) {
        sessions[0].projectPath = root;
      }
    })();

    // Phase 8.7j — restore persisted notif tab order before any subscription
    // fires. Pure synchronous read of localStorage; ordering must settle
    // before TabBar renders to avoid a flicker.
    applyPersistedNotifOrder();
    applyPersistedWorkspace();

    // Svelte 5's onMount requires a sync callback that optionally returns a
    // cleanup. Async init runs in an IIFE; cleanup captures unlisten handles
    // via mutable refs the IIFE populates.
    let unlistenDetached: (() => void) | undefined;
    let unlistenReattached: (() => void) | undefined;

    // Load notification filter thresholds from config on mount.
    void loadNotifFilters();

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

    // Re-read filters + appearance + alerts when Settings saves (rift:config-changed broadcast).
    const onConfigChanged = () => {
      void loadNotifFilters();
      void loadAppearanceConfig();
      void loadAlertRules();
    };
    window.addEventListener('rift:config-changed', onConfigChanged);

    void (async () => {
      try {
        cockpitDetached = await invoke<boolean>('cockpit_status');
      } catch (err) {
        console.warn('[App] cockpit_status failed:', err);
      }

      // Recover notification tab detach state on reload.
      await recoverDetachState();

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
            if (
              typeof pos.x === 'number' && typeof pos.y === 'number' &&
              typeof pos.width === 'number' && typeof pos.height === 'number'
            ) {
              await appWindow.setPosition(new PhysicalPosition(pos.x, pos.y));
              await appWindow.setSize(new PhysicalSize(pos.width, pos.height));
            }
          } catch {
            try { localStorage.removeItem(MAIN_POS_KEY); } catch { /* ignore */ }
          }
        }
        // Save current rect immediately so a crash before any move/resize
        // still records a reasonable last-known position.
        const [pos0, size0] = await Promise.all([
          appWindow.outerPosition(),
          appWindow.outerSize(),
        ]);
        try {
          localStorage.setItem(MAIN_POS_KEY, JSON.stringify({
            x: pos0.x, y: pos0.y, width: size0.width, height: size0.height,
          }));
        } catch { /* quota / private */ }
        // Subscribe to move + resize so subsequent changes persist live.
        appWindow.onMoved(({ payload: pos }) => {
          appWindow.outerSize().then((size) => {
            try {
              localStorage.setItem(MAIN_POS_KEY, JSON.stringify({
                x: pos.x, y: pos.y, width: size.width, height: size.height,
              }));
            } catch { /* ignore */ }
          }).catch(() => { /* ignore */ });
        });
        appWindow.onResized(({ payload: size }) => {
          appWindow.outerPosition().then((pos) => {
            try {
              localStorage.setItem(MAIN_POS_KEY, JSON.stringify({
                x: pos.x, y: pos.y, width: size.width, height: size.height,
              }));
            } catch { /* ignore */ }
          }).catch(() => { /* ignore */ });
        });
      } catch (err) {
        console.warn('[App] window position persistence failed:', err);
      }
    })();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === 'k') {
        e.preventDefault();
        paletteOpen = !paletteOpen;
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
    };
    window.addEventListener('keydown', onKeyDown);

    return () => {
      unlistenDetached?.();
      unlistenReattached?.();
      window.removeEventListener('rift:config-changed', onConfigChanged);
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
    {sessions}
    {notifs}
    {active}
    {tickNow}
    promotedId={promoted}
    onActivateSession={activateSession}
    onActivateNotif={activateNotif}
    onCloseSession={closeSession}
    onAddSession={addSession}
    onToggleNotif={toggleNotif}
    onPromote={promoteTab}
    onDemote={demoteTab}
    onManageNotifs={openNotifManager}
    onReorderNotif={reorderNotif}
    onDetach={detachNotif}
    {detachedIds}
    {multiProject}
    {cockpitCollapsed}
    {alertTriggered}
    onToggleCockpit={toggleCockpit}
  />

  <!-- Phase 6.2 — always-on cockpit: left = terminal surface, right = file tree -->
  <main class="cockpit">
    <!-- Left half: existing terminal / notification / empty surfaces + optional promoted pane -->
    <div class="cockpit-left" class:split={promoted !== null} class:full-width={cockpitCollapsed || cockpitDetached}>
      <div class="main-left">
        <!-- session terminals — keep all alive, hide inactive ones to preserve scrollback -->
        {#each sessions as s (s.id)}
          <div
            class="surface"
            class:visible={active.kind === 'session' && active.id === s.id}
          >
            <Terminal
              visible={active.kind === 'session' && active.id === s.id}
              projectPath={s.projectPath}
            />
          </div>
        {/each}


        <!-- empty state — no tabs open -->
        {#if active.kind === 'empty'}
          <div class="surface visible empty-state">
            <div class="empty-card">
              <div class="empty-glyph">◆</div>
              <div class="empty-title">no surface active</div>
              <div class="empty-hint">click <kbd>+</kbd> to open a new shell</div>
            </div>
          </div>
        {/if}
      </div>

      <!-- Phase 3.5a — promoted notification side-pane.
           Independent NotificationPane / AegisTabContent instance (not driven by `active`).
           Re-keyed on the promoted id so the subscription resets cleanly
           when one promoted tab replaces another. -->
      {#if promotedTab}
        <Splitter
          direction="vertical"
          storageKey="rift.promoted.width_px"
          unit="px"
          bind:size={promotedPaneWidth}
          min={280}
          max={800}
          onDblClick={() => (promotedPaneWidth = 420)}
        />
        {#key promotedTab.id}
          <aside class="promoted-pane" style="flex: 0 0 {promotedPaneWidth}px;">
            {#if promotedTab.id === 'aegis'}
              <AegisTabContent severityThreshold={thresholdFor('aegis')} onDragBack={demoteTab} />
            {:else if promotedTab.id === 'index'}
              <IndexTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'bustail'}
              <BusTailTabContent severityThreshold={thresholdFor('bustail')} onDragBack={demoteTab} {correlationIndex} />
            {:else if promotedTab.id === 'todo'}
              <TodoTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'git'}
              <GitTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'agents'}
              <AgentsTabContent severityThreshold={thresholdFor('agents')} onDragBack={demoteTab} />
            {:else if promotedTab.id === 'filesystem'}
              <FsTabContent severityThreshold={thresholdFor('filesystem')} onDragBack={demoteTab} />
            {:else if promotedTab.id === 'mcp'}
              <McpTabContent severityThreshold={thresholdFor('mcp')} onDragBack={demoteTab} {correlationIndex} />
            {:else if promotedTab.id === 'sentinel'}
              <SentinelTabContent severityThreshold={thresholdFor('sentinel')} onDragBack={demoteTab} />
            {:else if promotedTab.id === 'sessions'}
              <SessionsTabContent onDragBack={demoteTab} />
            {:else}
              <NotificationPane
                title={promotedTab.title}
                icon={promotedTab.icon}
                accent={notifAccent(promotedTab.id)}
                categoryFilter={CATEGORY_BY_NOTIF[promotedTab.id]}
                severityThreshold={thresholdFor(promotedTab.id)}
                onDragBack={demoteTab}
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
            <span class="meta">vault graph · fixture</span>
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

  <StatusLine
    dir={statusDir || '—'}
    model={statusModel || '—'}
    ctx={statusCtx || '—'}
    session={statusSession || '—'}
    repo={statusRepo || '—'}
    git={statusGit || '—'}
    skill={aegisSkillName || ccSkill || '—'}
    effort={aegisEffort || ccEffort || '—'}
    sessionUse={statusSessionUse || '—'}
    week={statusWeek || '—'}
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
      onclose={() => { paletteOpen = false; }}
      onActivateNotif={activateNotif}
    />
  {/if}

  {#if shortcutsOpen}
    <ShortcutOverlay onclose={() => { shortcutsOpen = false; }} />
  {/if}

  {#if welcomeOpen}
    <WelcomeOverlay ondismiss={dismissWelcome} />
  {/if}
</div>

<style>
  /* Phase 6.2 — cockpit outer shell: terminal left, file tree right. */
  .cockpit {
    flex: 1;
    display: flex;
    flex-direction: row;
    min-height: 0;
    background: var(--bg-base);
    position: relative;
    overflow: hidden;
  }

  .cockpit-left {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-right: 2px solid var(--amber-faint);
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
    border-left: none;
    box-shadow: var(--depth-inset);
    background: var(--bg-base);
  }

  /* Right half — graph + tree stacked column (Phase 8.4).
     flex-basis is set inline by App.svelte via cockpitRightWidth state
     (Phase 8.7e — Splitter-controlled, persisted to localStorage). */
  .cockpit-right {
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    min-width: 0;
  }

  .graph-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-base);
    border-bottom: 2px solid var(--amber-faint);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
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
    background: var(--bg-base);
  }

  .pane-header {
    height: 32px;
    padding: 0 14px;
    background: rgba(255, 168, 38, 0.05);
    border-bottom: 1px solid var(--amber-faint);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--amber-bright);
    font-family: var(--font-family);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.16em;
    flex-shrink: 0;
    user-select: none;
  }
  .pane-header .meta {
    color: var(--amber-dim);
    font-weight: 400;
    font-size: 9px;
    letter-spacing: 0.04em;
  }

  /* Tree body scrolls; Tree.svelte owns the SVG. */
  .tree-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0;
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
    padding: 32px;
  }
  .empty-glyph {
    font-size: 48px;
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-strong);
    margin-bottom: 16px;
  }
  .empty-title {
    color: var(--amber-warm);
    font-size: 14px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    margin-bottom: 8px;
  }
  .empty-hint {
    color: var(--amber-dim);
    font-size: 11px;
  }
  .empty-hint kbd {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-primary);
    padding: 1px 6px;
    font-family: inherit;
    font-size: 10px;
  }

  /* D-013 — Update-available banner. Slim row, amber-bordered, sits
     between TitleBar (32px) and TabBar (36px). 28px height keeps it
     compact; only renders when availableUpdate is truthy. */
  .update-banner {
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--amber-primary);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: 11px;
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
    font-size: 13px;
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
    font-size: 10px;
    letter-spacing: 0.08em;
    padding: 3px 10px;
    cursor: pointer;
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
</style>
