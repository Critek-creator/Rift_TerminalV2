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
  import StatusLine from './lib/StatusLine.svelte';
  import Popout from './lib/Popout.svelte';
  import { popouts } from './lib/popouts.svelte';
  import Tree from './lib/Tree.svelte';
  import IndexGraph from './lib/IndexGraph.svelte';
  import Splitter from './lib/Splitter.svelte';
  import { subscribe, publish, signalBusReady, type Category } from './lib/bus';
  import { enrichmentStore } from './lib/enrichmentStore.svelte';

  // Tab id → bus category. `undefined` = no integration registered yet,
  // so the pane stays in placeholder mode until a translator lights it up.
  const CATEGORY_BY_NOTIF: Record<string, Category | undefined> = {
    hooks: 'hook',
    errors: 'system',     // v1: all Category::System envelopes are errors (kind="error" is the only System kind emitted); kind-filter is a future enhancement
    commands: 'pty',      // v1: all Category::Pty envelopes; only kind emitted is "command.submitted"; kind-filter deferred
    aegis: 'aegis',       // Phase 7.2 — Aegis integration tab (private translator, feature-gated)
    index: 'index',       // Phase 8.2 — Index integration tab (vault-walker source wires in Phase 8.5)
    agents: 'agent',      // Phase 8.7k — Agents tracker (§10.11 display layer; Sentinel detection deferred via D-010)
  };

  // ----- session tabs -----
  let nextSessionId = 1;
  let sessions = $state<SessionTab[]>([{ id: 0, title: 'rift' }]);

  // ----- notification tabs (default set per §10.7) -----
  // Capability-gating (§10.7): base set (errors / hooks / commands) is
  // always present; integration tabs (aegis, index, …) appear only after
  // the integration declares itself via an envelope on its category.
  // - `aegis` → flips on `aegis.detected` from rift-aegis startup probe (Phase 7.1).
  // - `index` → CURRENTLY initialized detected=true because the Index translator
  //   is wired (Phase 8.1) but doesn't emit a startup-ready envelope yet.
  //   TODO(8.5): tighten to envelope-driven once vault-walker publishes — then
  //   change this back to `detected: false` and let the master subscription flip it.
  // unreadCount + lastActivityTs drive the §10.9 badge counter and live border.
  let notifs = $state<NotifTab[]>([
    { id: 'errors',   title: 'errors',   icon: '⚡', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    { id: 'hooks',    title: 'hooks',    icon: '⚓', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    { id: 'commands', title: 'commands', icon: '⌘', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    { id: 'aegis',    title: 'aegis',    icon: '◉', enabled: true, detected: false, unreadCount: 0, lastActivityTs: null },
    { id: 'index',    title: 'index',    icon: '◈', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    // Phase 8.7i — bus tail (firehose dev visibility). detected:true since
    // it's a Rift-internal surface, not capability-gated on any translator.
    { id: 'bustail',  title: 'bus tail', icon: '⌁', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    // Phase 8.7i — TODO scraper across project source. detected:true since
    // it's a pure frontend feature backed by a synchronous Tauri command.
    { id: 'todo',     title: 'todo',     icon: '⊜', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    // Phase 8.7i — git status (branch, ahead/behind, working tree).
    // Shells out to `git`; renders not-a-repo empty state when applicable.
    { id: 'git',      title: 'git',      icon: '⎇', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    // Phase 8.7k — Agents tracker. Capability-gated: detected:false until
    // any Category::Agent envelope arrives (master sub flips it). Cancel
    // button in the tab publishes agent.cancel back to the bus per §9 — a
    // translator (Aegis today, Sentinel post-D-010) is what fulfills it.
    { id: 'agents',   title: 'agents',   icon: '◊', enabled: true, detected: false, unreadCount: 0, lastActivityTs: null },
  ]);

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
  };

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
    // Per user spec correction (drift from earlier Phase 3.5a implementation):
    // notif tabs NEVER replace the terminal/session surface in the main slot.
    // Click toggles promotion alongside the main session surface — the notif's
    // content lives in `aside.promoted-pane` next to the terminal, never in
    // place of it.
    //
    // To detach a notif tab into a SEPARATE webview window (drag-out for
    // multi-monitor observation, paralleling Phase 6.4 cockpit_detach), is
    // a v1.x feature ask — not yet wired. For v1, click is the only path to
    // attach/detach a notif side-pane; the side pane's drag-back handle (or
    // another click on the same tab) is the path to demote.
    //
    // The `active` state machine never becomes `notification` — notif tabs
    // promote to a side pane via `promoted`, never replace the main surface.
    // If a future "notif fullscreen" gesture is needed, re-introduce a
    // dedicated state-machine variant with explicit semantics; do NOT revive
    // the previous unreachable branches that this commit removed.
    const wasPromoted = promoted === id;
    promoted = wasPromoted ? null : id;
    // §10.9 ack — promoting a tab into the side pane counts as viewing it.
    if (!wasPromoted) ackUnread(id);
  }
  function addSession() {
    const id = nextSessionId++;
    sessions = [...sessions, { id, title: `shell ${id + 1}` }];
    active = { kind: 'session', id };
  }
  function closeSession(id: number) {
    sessions = sessions.filter((s) => s.id !== id);
    if (active.kind === 'session' && active.id === id) {
      // Activate the rightmost remaining session, or fall through to empty.
      const last = sessions.at(-1);
      active = last ? { kind: 'session', id: last.id } : { kind: 'empty' };
    }
  }
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
  }
  function demoteTab() {
    promoted = null;
    // active stays as-is — user clicks the tab in the strip to view it again.
  }

  function notifAccent(id: string): 'amber' | 'cyan' | 'purple' | 'red' {
    if (id === 'hooks') return 'cyan';
    if (id === 'errors') return 'red';
    return 'amber';
  }
  const activeNotifTab = $derived.by(() => {
    const a = active;
    if (a.kind !== 'notification') return undefined;
    return notifs.find((n) => n.id === a.id);
  });
  // The promoted tab's data — looked up fresh from `notifs` so its
  // enabled/title/icon stay reactive with toggles.
  const promotedTab = $derived.by(() => {
    if (promoted === null) return undefined;
    return notifs.find((n) => n.id === promoted);
  });

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
  let graphHeightPct = $state(55);     // percent — was flex: 0 0 55%

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
    const t = setInterval(() => { tickNow = Date.now(); }, 1000);
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

  // D-012 unblocked slice — live DIR / GIT / REPO segments.
  // Subscribes to Category::Status at App level so the StatusLine updates
  // regardless of which tab is active. Same svelte5-async-cleanup-via-sync-
  // shell-iife + cancelled-flag pattern as the SKILL subscription below.
  let statusDir = $state('');
  let statusGit = $state('');
  let statusRepo = $state('');

  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({ category: 'status' }, (env) => {
          if (env.kind === 'usage') {
            const p = env.payload as { dir?: string; git?: string; repo?: string };
            if (p.dir  !== undefined) statusDir  = p.dir;
            if (p.git  !== undefined) statusGit  = p.git;
            if (p.repo !== undefined) statusRepo = p.repo;
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
    })();

    // Phase 8.7j — restore persisted notif tab order before any subscription
    // fires. Pure synchronous read of localStorage; ordering must settle
    // before TabBar renders to avoid a flicker.
    applyPersistedNotifOrder();

    // Svelte 5's onMount requires a sync callback that optionally returns a
    // cleanup. Async init runs in an IIFE; cleanup captures unlisten handles
    // via mutable refs the IIFE populates.
    let unlistenDetached: (() => void) | undefined;
    let unlistenReattached: (() => void) | undefined;

    void (async () => {
      try {
        cockpitDetached = await invoke<boolean>('cockpit_status');
      } catch (err) {
        console.warn('[App] cockpit_status failed:', err);
      }

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

    return () => {
      unlistenDetached?.();
      unlistenReattached?.();
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
  />

  <!-- Phase 6.2 — always-on cockpit: left = terminal surface, right = file tree -->
  <main class="cockpit">
    <!-- Left half: existing terminal / notification / empty surfaces + optional promoted pane -->
    <div class="cockpit-left" class:split={promoted !== null}>
      <div class="main-left">
        <!-- session terminals — keep all alive, hide inactive ones to preserve scrollback -->
        {#each sessions as s (s.id)}
          <div
            class="surface"
            class:visible={active.kind === 'session' && active.id === s.id}
          >
            <Terminal visible={active.kind === 'session' && active.id === s.id} />
          </div>
        {/each}

        <!-- notification pane — only mount when a notif tab is active.
             Re-key on the tab id so switching tabs gives the pane a fresh
             subscription rather than reusing one tied to the previous tab.
             Phase 7.2: aegis tab routes to AegisTabContent; all others
             continue to use the generic NotificationPane. -->
        {#if activeNotifTab}
          {#key activeNotifTab.id}
            <div class="surface visible">
              {#if activeNotifTab.id === 'aegis'}
                <AegisTabContent />
              {:else if activeNotifTab.id === 'index'}
                <IndexTabContent />
              {:else if activeNotifTab.id === 'bustail'}
                <BusTailTabContent />
              {:else if activeNotifTab.id === 'todo'}
                <TodoTabContent />
              {:else if activeNotifTab.id === 'git'}
                <GitTabContent />
              {:else if activeNotifTab.id === 'agents'}
                <AgentsTabContent />
              {:else}
                <NotificationPane
                  title={activeNotifTab.title}
                  icon={activeNotifTab.icon}
                  accent={notifAccent(activeNotifTab.id)}
                  categoryFilter={CATEGORY_BY_NOTIF[activeNotifTab.id]}
                />
              {/if}
            </div>
          {/key}
        {/if}

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
        {#key promotedTab.id}
          <aside class="promoted-pane">
            {#if promotedTab.id === 'aegis'}
              <AegisTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'index'}
              <IndexTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'bustail'}
              <BusTailTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'todo'}
              <TodoTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'git'}
              <GitTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'agents'}
              <AgentsTabContent onDragBack={demoteTab} />
            {:else}
              <NotificationPane
                title={promotedTab.title}
                icon={promotedTab.icon}
                accent={notifAccent(promotedTab.id)}
                categoryFilter={CATEGORY_BY_NOTIF[promotedTab.id]}
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
    {#if !cockpitDetached}
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
    repo={statusRepo || '—'}
    git={statusGit || '—'}
    skill={aegisSkillName || '—'}
    effort={aegisEffort || '—'}
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

  /* Left half — holds the terminal/notif/empty area + optional promoted pane. */
  .cockpit-left {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-right: 1px solid var(--border-subtle);
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
    flex: 0 0 420px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* Phase 8.7q.3 — min-width: 0 + overflow: hidden enforce the 420px
       basis as a hard cap. Without these, intrinsic min-content of the
       inner pane (long unbroken JSON in a notif row) overrides flex-basis
       and pushes .main-left (the terminal) out of the viewport. */
    min-width: 0;
    overflow: hidden;
    border-left: 1px solid var(--border-subtle);
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

  /* Phase 8.4 — Graph pane (top slot). flex-basis set inline via
     graphHeightPct state (Phase 8.7e Splitter-controlled). */
  .graph-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-base);
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

  /* Pane header — FILE TREE / INDEX title + meta. Shared vocabulary with NotificationPane. */
  .pane-header {
    height: 24px;
    padding: 0 10px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    flex-shrink: 0;
    user-select: none;
  }
  .pane-header .meta {
    color: var(--amber-faint);
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
    font-family: 'JetBrains Mono', monospace;
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
