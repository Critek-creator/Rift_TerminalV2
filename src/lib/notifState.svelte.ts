// notifState.svelte.ts — A-01 extraction step 2 (renamed from notifManager to avoid casing conflict with NotifManager.svelte)
//
// Owns notification tab state, promoted pane, toggle/reorder/persist logic,
// detach-to-window state, badge acknowledgment, and notif.tabs bus publish.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { NotifTab } from './TabBar.svelte';
import { publish, type Category } from './bus';
import { parseSeverity, resolveThreshold, type SeverityLevel } from './notifFilter';
import type { RiftConfig as RiftConfigType } from './riftConfig';
import { sectionCatalog, NOTIF_GROUPS, type GroupDescriptor } from './sectionCatalog.svelte';
import { popouts } from './popouts.svelte';

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

const CATEGORY_BY_NOTIF: Record<string, Category | undefined> = $derived.by(() => {
  const result: Record<string, Category | undefined> = {};
  for (const desc of sectionCatalog.allTabs) {
    if (desc.category) result[desc.id] = desc.category;
  }
  return result;
});

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
  llm: 'llm-activity',
};

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

let notifFilterDefault = $state<SeverityLevel>('info');
let notifFilterPerTab = $state<Record<string, SeverityLevel>>({});

let promoted = $state<string | null>(null);

let detachedIds = $state<Set<string>>(new Set());

// ---------------------------------------------------------------------------
// Derived
// ---------------------------------------------------------------------------

const promotedTab = $derived.by(() => {
  if (promoted === null) return undefined;
  return notifs.find((n) => n.id === promoted);
});

export interface NotifGroupState extends GroupDescriptor {
  tabs: NotifTab[];
  aggregateBadge: number;
}

const groupedNotifs = $derived.by((): NotifGroupState[] => {
  const result: NotifGroupState[] = NOTIF_GROUPS.map((g) => ({
    ...g,
    tabs: [],
    aggregateBadge: 0,
  }));
  const groupMap = new Map(result.map((g) => [g.id, g]));

  for (const tab of notifs) {
    if (!tab.detected || !tab.enabled) continue;
    const desc = sectionCatalog.get(tab.id);
    if (!desc?.group) continue;
    const group = groupMap.get(desc.group);
    if (group) {
      group.tabs.push(tab);
      group.aggregateBadge += tab.unreadCount;
    }
  }

  return result.filter((g) => g.tabs.length > 0);
});

// ---------------------------------------------------------------------------
// Filter helpers
// ---------------------------------------------------------------------------

function thresholdFor(tabId: string): SeverityLevel {
  return resolveThreshold(tabId, notifFilterDefault, notifFilterPerTab);
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

// ---------------------------------------------------------------------------
// Tab management
// ---------------------------------------------------------------------------

function activateNotif(id: string) {
  const wasPromoted = promoted === id;
  promoted = wasPromoted ? null : id;
  if (!wasPromoted) ackUnread(id);
}

function toggleNotif(id: string) {
  notifs = notifs.map((n) => (n.id === id ? { ...n, enabled: !n.enabled } : n));
  const enabled = notifs.find((n) => n.id === id)?.enabled;
  if (!enabled && promoted === id) {
    promoted = null;
  }
  persistNotifOrder();
}

function resetNotifs() {
  notifs = notifs.map((n) => ({ ...n, enabled: true }));
  persistNotifOrder();
}

function ackUnread(id: string) {
  notifs = notifs.map((n) => (n.id === id && n.unreadCount > 0 ? { ...n, unreadCount: 0 } : n));
}

function promoteTab(id: string) {
  promoted = id;
  ackUnread(id);
  persistWorkspace();
}

function demoteTab() {
  promoted = null;
  persistWorkspace();
}

function notifAccent(id: string): 'amber' | 'cyan' | 'purple' | 'red' {
  if (id === 'hooks') return 'cyan';
  if (id === 'errors') return 'red';
  return 'amber';
}

// ---------------------------------------------------------------------------
// Reorder + persistence
// ---------------------------------------------------------------------------

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
    for (const tab of idToTab.values()) reordered.push(tab);
    if (reordered.length === notifs.length) {
      notifs = reordered;
    }
  } catch {
    // Corrupt JSON — ignore and use defaults.
  }
}

// ---------------------------------------------------------------------------
// Notif manager popout
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Detach-to-window
// ---------------------------------------------------------------------------

async function detachNotif(id: string) {
  if (detachedIds.has(id)) return;
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

async function recoverDetachState() {
  try {
    const ids = await invoke<string[]>('notif_detach_status');
    if (ids.length > 0) detachedIds = new Set(ids);
  } catch {
    // best-effort
  }
}

// ---------------------------------------------------------------------------
// Effects (module-level)
// ---------------------------------------------------------------------------

$effect.root(() => {
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

  // D-014 Phase B — publish `notif.tabs` snapshot to the bus whenever the
  // catalog changes. Debounced 500ms to avoid flooding during burst activity.
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
    const timer = setTimeout(() => {
      void publish('system', 'notif.tabs', { tabs }).catch((err) => {
        console.warn('[App] notif.tabs publish failed:', err);
      });
    }, 500);
    return () => clearTimeout(timer);
  });

  return () => {};
});

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

export const notifManager = {
  get notifs(): NotifTab[] { return notifs; },
  set notifs(v: NotifTab[]) { notifs = v; },
  get groupedNotifs(): NotifGroupState[] { return groupedNotifs; },
  get promoted(): string | null { return promoted; },
  set promoted(v: string | null) { promoted = v; },
  get promotedTab(): NotifTab | undefined { return promotedTab; },
  get detachedIds(): Set<string> { return detachedIds; },
  get CATEGORY_BY_NOTIF(): Record<string, Category | undefined> { return CATEGORY_BY_NOTIF; },
  get NOTIF_BY_CATEGORY(): Record<string, string> { return NOTIF_BY_CATEGORY; },
  thresholdFor,
  loadNotifFilters,
  activateNotif,
  toggleNotif,
  resetNotifs,
  ackUnread,
  promoteTab,
  demoteTab,
  notifAccent,
  reorderNotif,
  persistNotifOrder,
  openNotifManager,
  detachNotif,
  recoverDetachState,
  applyPersistedNotifOrder,
  applyPersistedWorkspace,
};
