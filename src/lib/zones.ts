// zones.ts — Phase 2 / N2: the fixed zone model ("the layout IS the IA").
//
// North-star principle N2: every category of content gets exactly ONE home,
// enforced architecturally. A new feature must *claim a zone* — it cannot float
// as an orphan. Before this module the zones existed only implicitly in
// App.svelte's markup; nothing declared them, so "data everywhere" had no
// structural cure.
//
// This is the single source of truth. `zones.test.ts` enforces completeness in
// CI: an unassigned surface, an empty zone, or a duplicate id is a test failure
// rather than a silent orphan. When you add a top-level surface, add it to
// SURFACES with its zone — that IS the registration.

/** The six fixed zones. Their on-screen order top→bottom is roughly:
 *  chrome → (terminal | cockpit) → status, with overlay floating above all and
 *  notification sharing the terminal row as a promoted side-pane. */
export type ZoneId =
  | 'chrome' // window furniture: title bar + tab bar
  | 'terminal' // the shell surface(s) — the primary work area
  | 'cockpit' // right-hand observability column (graph + tree)
  | 'notification' // promoted side-pane + the notif-tab system
  | 'status' // bottom ambient chrome — mode hints + status line
  | 'overlay'; // transient surfaces over everything — palette, dialogs, popouts

export interface ZoneDescriptor {
  id: ZoneId;
  title: string;
  /** What kind of content is allowed to live here. */
  role: string;
  /** fixed = always present · toggle = user can collapse/detach ·
   *  transient = appears on demand, then dismisses. */
  persistence: 'fixed' | 'toggle' | 'transient';
}

export const ZONES: Record<ZoneId, ZoneDescriptor> = {
  chrome: {
    id: 'chrome',
    title: 'Chrome',
    role: 'Window furniture: identity, window controls, and tab navigation.',
    persistence: 'fixed',
  },
  terminal: {
    id: 'terminal',
    title: 'Terminal',
    role: 'The shell surface — the primary work area. One or more PTY panes.',
    persistence: 'fixed',
  },
  cockpit: {
    id: 'cockpit',
    title: 'Cockpit',
    role: 'Right-hand observability column: the Index graph and the file tree.',
    persistence: 'toggle',
  },
  notification: {
    id: 'notification',
    title: 'Notification',
    role: 'Categorized activity surfaces — the notif-tab system and its promoted side-pane.',
    persistence: 'toggle',
  },
  status: {
    id: 'status',
    title: 'Status',
    role: 'Bottom ambient chrome: context keybind hints and the live status line.',
    persistence: 'fixed',
  },
  overlay: {
    id: 'overlay',
    title: 'Overlay',
    role: 'Transient surfaces that float above everything and dismiss on Esc/backdrop.',
    persistence: 'transient',
  },
};

export interface SurfaceDescriptor {
  /** Stable surface id (kebab-case). */
  id: string;
  /** Human label. */
  title: string;
  /** The one zone this surface claims. */
  zone: ZoneId;
  /** The component (or `App.svelte` inline) that renders it — traceability. */
  component: string;
}

/**
 * Every top-level surface, each claiming exactly one zone. The notif-tab
 * *system* is one surface here (the promoted side-pane); the individual notif
 * tabs are registered in sectionCatalog and all live in the notification zone
 * by rule (see NOTIF_ZONE), so they are not re-listed surface-by-surface.
 */
export const SURFACES: SurfaceDescriptor[] = [
  // chrome
  { id: 'title-bar', title: 'Title bar', zone: 'chrome', component: 'TitleBar.svelte' },
  { id: 'tab-bar', title: 'Tab bar', zone: 'chrome', component: 'TabBar.svelte' },
  { id: 'update-banner', title: 'Update banner', zone: 'chrome', component: 'App.svelte (inline)' },

  // terminal
  { id: 'terminal-grid', title: 'Terminal grid', zone: 'terminal', component: 'TerminalGrid.svelte' },
  { id: 'empty-state', title: 'Empty state', zone: 'terminal', component: 'App.svelte (inline)' },
  { id: 'sticky-cmd-header', title: 'Sticky command header', zone: 'terminal', component: 'StickyCommandHeader.svelte' },

  // cockpit
  { id: 'cockpit-graph', title: 'Index graph', zone: 'cockpit', component: 'IndexGraph.svelte' },
  { id: 'cockpit-tree', title: 'File tree', zone: 'cockpit', component: 'Tree.svelte' },

  // notification
  { id: 'promoted-pane', title: 'Promoted notif pane', zone: 'notification', component: 'NotificationPane.svelte + *TabContent.svelte' },

  // status
  { id: 'mode-hint-bar', title: 'Mode-hint bar', zone: 'status', component: 'ModeHintBar.svelte' },
  { id: 'status-line', title: 'Status line', zone: 'status', component: 'StatusLine.svelte' },

  // overlay
  { id: 'command-palette', title: 'Command palette', zone: 'overlay', component: 'CommandPalette.svelte' },
  { id: 'shortcut-overlay', title: 'Shortcut overlay', zone: 'overlay', component: 'ShortcutOverlay.svelte' },
  { id: 'welcome-overlay', title: 'Welcome overlay', zone: 'overlay', component: 'WelcomeOverlay.svelte' },
  { id: 'popout-stack', title: 'Pop-out stack', zone: 'overlay', component: 'Popout.svelte' },
  { id: 'close-confirm', title: 'Close-tab confirm', zone: 'overlay', component: 'App.svelte (inline)' },
];

/** All notif tabs (sectionCatalog) belong to this zone. The notif-tab system is
 *  represented in SURFACES by the single `promoted-pane` surface; this constant
 *  is the rule that keeps every dynamically-registered tab inside one zone. */
export const NOTIF_ZONE: ZoneId = 'notification';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** The zone a surface claims, or undefined if the surface id is unknown. */
export function zoneFor(surfaceId: string): ZoneId | undefined {
  return SURFACES.find((s) => s.id === surfaceId)?.zone;
}

/** All surfaces that claim a given zone. */
export function surfacesIn(zone: ZoneId): SurfaceDescriptor[] {
  return SURFACES.filter((s) => s.zone === zone);
}

/** Zones with no surface assigned — should always be empty (enforced in CI). */
export function emptyZones(): ZoneId[] {
  return (Object.keys(ZONES) as ZoneId[]).filter((z) => surfacesIn(z).length === 0);
}
