// sectionCatalog.svelte.ts — D-021 (§10.16)
//
// Typed section registry for the notification tab system. Each tab
// declares its section anatomy via a TabDescriptor; integrations can
// register new tabs at runtime without touching App.svelte.
//
// The 4 canonical section types per §10.4:
//   1. status-header  — title, icon, event count, last-seen
//   2. live-strip     — horizontally-scrolling recent envelopes (<4s)
//   3. recent-log     — scrollable event log (last 100)
//   4. state-panel    — persistent state (custom per tab)
//
// All 11 built-in tabs are pre-registered on module load. Future
// integrations call `sectionCatalog.register()` to add tabs dynamically.

import type { Category } from './bus';

/** The 4 canonical section types from §10.4 + extensible custom type. */
export type SectionType =
  | 'status-header'
  | 'live-strip'
  | 'recent-log'
  | 'state-panel'
  | `custom:${string}`;

/** Describes one section within a tab's anatomy. */
export interface SectionDescriptor {
  type: SectionType;
  /** Render order within the tab (lower = higher). */
  order: number;
  /** Whether this section is visible by default (can be toggled by user). */
  visible: boolean;
}

/** Full descriptor for a notification tab. */
export interface TabDescriptor {
  id: string;
  title: string;
  icon: string;
  /** Bus category this tab subscribes to. Undefined = no bus subscription
   *  (tab uses dedicated Tauri commands or raw firehose). */
  category?: Category;
  /** Whether the tab is visible in the strip before its integration
   *  declares itself via an envelope. Base tabs = true, integration = false. */
  detectedByDefault: boolean;
  /** Ordered section anatomy for this tab. */
  sections: SectionDescriptor[];
  /** Source of the registration — 'builtin' for first-party, integration
   *  name for third-party. Enables unregister-by-source cleanup. */
  source: string;
}

// §10.4 standard 4-section anatomy — reused by all tabs.
const STANDARD_SECTIONS: SectionDescriptor[] = [
  { type: 'status-header', order: 0, visible: true },
  { type: 'live-strip',    order: 1, visible: true },
  { type: 'recent-log',    order: 2, visible: true },
  { type: 'state-panel',   order: 3, visible: true },
];

// ---------------------------------------------------------------------------
// Built-in tab descriptors (pre-registered on module load)
// ---------------------------------------------------------------------------

const BUILTIN_TABS: TabDescriptor[] = [
  {
    id: 'errors', title: 'errors', icon: '⚡',
    category: 'system', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'hooks', title: 'hooks', icon: '⚓',
    category: 'hook', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'commands', title: 'commands', icon: '⌘',
    category: 'pty', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'aegis', title: 'aegis', icon: '◉',
    category: 'aegis', detectedByDefault: false,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'index', title: 'index', icon: '◈',
    category: 'index', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'bustail', title: 'bus tail', icon: '⌁',
    category: undefined, detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'todo', title: 'todo', icon: '⊜',
    category: undefined, detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'git', title: 'git', icon: '⎇',
    category: undefined, detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'agents', title: 'agents', icon: '◊',
    category: 'agent', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'sentinel', title: 'sentinel', icon: '⊘',
    category: 'sentinel', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'filesystem', title: 'files', icon: '⊞',
    category: 'fs', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'mcp', title: 'mcp', icon: '⬡',
    category: 'mcp', detectedByDefault: false,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'sessions', title: 'sessions', icon: '⏱',
    category: undefined, detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'cmd-intelligence', title: 'analytics', icon: '◇',
    category: undefined, detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
  {
    id: 'health', title: 'health', icon: '⊕',
    category: 'system', detectedByDefault: true,
    sections: [...STANDARD_SECTIONS],
    source: 'builtin',
  },
];

// ---------------------------------------------------------------------------
// Reactive registry state
// ---------------------------------------------------------------------------

let registry = $state(new Map<string, TabDescriptor>(
  BUILTIN_TABS.map((tab) => [tab.id, tab] as const),
));

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export const sectionCatalog = {
  /**
   * Register a new tab. Overwrites any existing tab with the same id.
   * Returns a cleanup function that unregisters the tab.
   */
  register(desc: TabDescriptor): () => void {
    registry = new Map(registry).set(desc.id, desc);
    return () => {
      const next = new Map(registry);
      next.delete(desc.id);
      registry = next;
    };
  },

  /** Unregister a tab by id. No-op if not found. */
  unregister(id: string): void {
    if (!registry.has(id)) return;
    const next = new Map(registry);
    next.delete(id);
    registry = next;
  },

  /** Unregister all tabs from a given source (e.g., when an integration disconnects). */
  unregisterBySource(source: string): void {
    const next = new Map(registry);
    let changed = false;
    for (const [id, desc] of next) {
      if (desc.source === source) {
        next.delete(id);
        changed = true;
      }
    }
    if (changed) registry = next;
  },

  /** Get a tab descriptor by id. */
  get(id: string): TabDescriptor | undefined {
    return registry.get(id);
  },

  /** Check if a tab is registered. */
  has(id: string): boolean {
    return registry.has(id);
  },

  /** Reactive getter — all registered tabs in insertion order. */
  get allTabs(): TabDescriptor[] {
    return Array.from(registry.values());
  },

  /** Reactive getter — the raw registry Map (for derived bindings). */
  get registry(): Map<string, TabDescriptor> {
    return registry;
  },

  /** Build the category→tabId reverse map from the current registry. */
  get categoryMap(): Map<Category, string> {
    const map = new Map<Category, string>();
    for (const desc of registry.values()) {
      if (desc.category) map.set(desc.category, desc.id);
    }
    return map;
  },

  /** Get the standard 4-section anatomy (for tabs that use the default). */
  get standardSections(): SectionDescriptor[] {
    return [...STANDARD_SECTIONS];
  },
};
