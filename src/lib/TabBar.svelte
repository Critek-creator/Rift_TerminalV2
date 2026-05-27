<script lang="ts">
  import { NOTIF_TAB_MIME } from './dragMime';
  import NotifGroupButton from './NotifGroupButton.svelte';
  import type { NotifGroupState } from './notifState.svelte';

  // §10.5 — tab strip; left group = session tabs, right group = notification
  // tab groups. Groups collapse 15 flat notification tabs into 3 category
  // dropdowns (System/Activity/Intel) to reduce strip clutter.

  import type { SplitNode } from './splitTypes';

  export type SessionTab = {
    id: number;
    title: string;
    projectPath: string | null;
    /** Recursive layout tree for split-pane support. Default is a single terminal leaf. */
    layout: SplitNode;
  };
  export type NotifTab = {
    id: string;
    title: string;
    icon: string;
    enabled: boolean;
    // §10.7 capability gate — tab renders only when its integration has
    // declared itself via at least one envelope on its category. Base set
    // (errors / hooks / commands) initializes detected=true; integration
    // tabs (aegis, index, …) initialize false and flip on first envelope.
    detected: boolean;
    // §10.9 badge counter — total envelopes received since this tab was
    // last viewed/promoted. Reset to 0 when activated or promoted.
    unreadCount: number;
    // §10.9 live border — timestamp of most recent envelope. Combined with
    // a 1s tick `tickNow` prop, a 3-second window drives the pulsing border.
    lastActivityTs: number | null;
  };

  export type ActiveSurface =
    | { kind: 'session'; id: number }
    | { kind: 'empty' };

  interface Props {
    sessions: SessionTab[];
    groupedNotifs: NotifGroupState[];
    active: ActiveSurface;
    promotedId: string | null;
    /** Updated every ~1s by App.svelte; drives the live-border decay window. */
    tickNow: number;
    onActivateSession: (id: number) => void;
    onActivateNotif: (id: string) => void;
    onCloseSession: (id: number) => void;
    onAddSession: (opts?: { pickProject?: boolean }) => void;
    onReorderSession: (srcId: number, dstId: number) => void;
    onRenameSession: (id: number, title: string) => void;
    onToggleNotif: (id: string) => void;
    onDemote: () => void;
    /** Phase 8.7h — open the notif tab manager popout. */
    onManageNotifs: () => void;
    /** Detach a notification tab into its own window. */
    onDetach: (id: string) => void;
    /** Set of currently detached tab IDs — visual state. */
    detachedIds: Set<string>;
    /** When true, multiple projects are open — show project name on tabs. */
    multiProject?: boolean;
    /** Cockpit right-pane collapsed state. */
    cockpitCollapsed?: boolean;
    /** Per-tab alert triggered state — drives flash animation. */
    alertTriggered?: Record<string, boolean>;
    /** Session IDs where all panes have exited — drives dead-tab indicator. */
    deadSessions?: Set<number>;
    /** Toggle cockpit right-pane visibility. */
    onToggleCockpit?: () => void;
  }

  let {
    sessions,
    groupedNotifs,
    active,
    promotedId,
    tickNow,
    onActivateSession,
    onActivateNotif,
    onCloseSession,
    onAddSession,
    onReorderSession,
    onRenameSession,
    onToggleNotif,
    onDemote,
    onManageNotifs,
    onDetach,
    detachedIds,
    multiProject = false,
    cockpitCollapsed = false,
    alertTriggered = {},
    deadSessions = new Set(),
    onToggleCockpit,
  }: Props = $props();

  function tabDisplayTitle(tab: SessionTab): string {
    if (!multiProject || !tab.projectPath) return tab.title;
    const name = tab.projectPath.split(/[\\/]/).at(-1) ?? '';
    return name ? `${tab.title} · ${name}` : tab.title;
  }

  // Drop-target highlight state — true while a drag is hovering the strip.
  let dropActive = $state(false);

  // Phase 6.6 regression-preventer (design call E):
  // Marker MIME type — defined in dragMime.ts so NotificationPane drag-back
  // dragstart can write the same constant (silent MIME mismatch on the
  // pane→tab demote path was the Phase 8.7 BV regression).

  // ----- session tab drag-to-reorder -----
  let sessionDragId = $state<number | null>(null);
  let sessionDropTarget = $state<number | null>(null);

  function onSessionDragStart(e: DragEvent, tab: SessionTab) {
    sessionDragId = tab.id;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', String(tab.id));
    }
  }
  function onSessionDragOver(e: DragEvent, tab: SessionTab) {
    if (sessionDragId === null || sessionDragId === tab.id) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    sessionDropTarget = tab.id;
  }
  function onSessionDragLeave(tabId: number) {
    if (sessionDropTarget === tabId) sessionDropTarget = null;
  }
  function onSessionDrop(e: DragEvent, tab: SessionTab) {
    e.preventDefault();
    if (sessionDragId !== null && sessionDragId !== tab.id) {
      onReorderSession(sessionDragId, tab.id);
    }
    sessionDragId = null;
    sessionDropTarget = null;
  }
  function onSessionDragEnd() {
    sessionDragId = null;
    sessionDropTarget = null;
  }

  // ----- session tab rename on double-click -----
  let editingTabId = $state<number | null>(null);
  let editValue = $state('');
  let editInputEl: HTMLInputElement | undefined = $state(undefined);

  function startRename(tab: SessionTab) {
    editingTabId = tab.id;
    editValue = tab.title;
    requestAnimationFrame(() => {
      editInputEl?.select();
    });
  }
  function commitRename() {
    if (editingTabId !== null && editValue.trim()) {
      onRenameSession(editingTabId, editValue.trim());
    }
    editingTabId = null;
  }
  function cancelRename() {
    editingTabId = null;
  }

  function isActiveSession(id: number) {
    return active.kind === 'session' && active.id === id;
  }

  // Strip-level drag handlers — pane→strip demote gesture.
  // With grouped tabs, per-tab drag is handled inside NotifGroupButton.
  // The strip still accepts drops for pane-source demote (NotificationPane /
  // AegisTabContent / IndexTabContent drag-handle writes '__promoted_pane__').
  function onStripDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dropActive = true;
  }
  function onStripDragEnter(e: DragEvent) {
    e.preventDefault();
    dropActive = true;
  }
  function onStripDragLeave() {
    dropActive = false;
  }
  function onStripDrop(e: DragEvent) {
    e.preventDefault();
    dropActive = false;
    if (!e.dataTransfer?.types.includes(NOTIF_TAB_MIME)) return;
    const payload = e.dataTransfer.getData(NOTIF_TAB_MIME);
    if (payload === '__promoted_pane__') {
      onDemote();
    }
  }
</script>

<nav
  class="tabbar"
  class:drop-active={dropActive}
  ondragover={onStripDragOver}
  ondragenter={onStripDragEnter}
  ondragleave={onStripDragLeave}
  ondrop={onStripDrop}
>
  <div class="group" role="tablist">
    {#each sessions as tab (tab.id)}
      <div
        class="tab session"
        class:active={isActiveSession(tab.id)}
        class:dead={deadSessions.has(tab.id)}
        class:drag-source={sessionDragId === tab.id}
        class:reorder-target={sessionDropTarget === tab.id}
        role="tab"
        tabindex="0"
        draggable={true}
        aria-selected={isActiveSession(tab.id)}
        onclick={() => onActivateSession(tab.id)}
        ondblclick={() => startRename(tab)}
        ondragstart={(e) => onSessionDragStart(e, tab)}
        ondragover={(e) => onSessionDragOver(e, tab)}
        ondragleave={() => onSessionDragLeave(tab.id)}
        ondrop={(e) => onSessionDrop(e, tab)}
        ondragend={onSessionDragEnd}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onActivateSession(tab.id);
          }
          if (e.key === 'F2') {
            e.preventDefault();
            startRename(tab);
          }
        }}
        title="double-click to rename · drag to reorder"
      >
        <span class="icon">{deadSessions.has(tab.id) ? '■' : '▶'}</span>
        {#if editingTabId === tab.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            bind:this={editInputEl}
            bind:value={editValue}
            class="tab-rename-input"
            autofocus
            onclick={(e) => e.stopPropagation()}
            onblur={commitRename}
            onkeydown={(e) => {
              if (e.key === 'Enter') { e.preventDefault(); commitRename(); }
              if (e.key === 'Escape') { e.preventDefault(); cancelRename(); }
              e.stopPropagation();
            }}
          />
        {:else}
          <span>{tabDisplayTitle(tab)}</span>
        {/if}
        <button
          type="button"
          class="close"
          aria-label="close tab"
          onclick={(e) => { e.stopPropagation(); onCloseSession(tab.id); }}
        >×</button>
      </div>
    {/each}
    <button
      type="button"
      class="add"
      aria-label="new tab"
      title="New tab (Shift+click: pick project)"
      onclick={(e: MouseEvent) => onAddSession({ pickProject: e.shiftKey })}
    >+</button>
  </div>

  <div class="group right">
    {#each groupedNotifs as group (group.id)}
      <NotifGroupButton
        {group}
        {promotedId}
        {tickNow}
        {detachedIds}
        {alertTriggered}
        {onActivateNotif}
        {onToggleNotif}
        {onDetach}
      />
    {/each}
    <button
      type="button"
      class="manage"
      aria-label="manage notification tabs"
      onclick={onManageNotifs}
      title="manage notification tabs"
    >⋯</button>
    {#if onToggleCockpit}
      <button
        type="button"
        class="manage cockpit-toggle"
        class:collapsed={cockpitCollapsed}
        aria-label={cockpitCollapsed ? 'show cockpit' : 'hide cockpit'}
        onclick={onToggleCockpit}
        title={cockpitCollapsed ? 'show cockpit (Ctrl+B)' : 'hide cockpit (Ctrl+B)'}
      >{cockpitCollapsed ? '◧' : '◨'}</button>
    {/if}
  </div>
</nav>

<style>
  .tabbar {
    height: 40px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-active);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: stretch;
    flex-shrink: 0;
    padding: 0 2px;
    gap: 2px;
  }
  .tabbar.drop-active {
    box-shadow: inset 0 0 0 1px var(--amber-bright), 0 1px 6px rgba(0, 0, 0, 0.4);
  }
  .group { display: flex; align-items: stretch; gap: 2px; }
  .group.right {
    margin-left: auto;
    padding-left: var(--space-sm);
    border-left: 2px solid var(--border-active);
    box-shadow: inset 1px 0 8px rgba(0, 0, 0, 0.3);
  }

  .tab {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: 0 var(--space-lg);
    height: calc(100% - var(--space-xs));
    margin-top: var(--space-xs);
    border: 1px solid var(--border-subtle);
    border-bottom: none;
    border-radius: var(--space-xs) var(--space-xs) 0 0;
    background: var(--bg-elevated);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-base);
    font-weight: 500;
    cursor: pointer;
    position: relative;
    transition: color 0.15s, background 0.15s, box-shadow 0.15s,
                border-color 0.15s;
    user-select: none;
  }
  .tab:hover {
    color: var(--amber-bright);
    background: var(--bg-hover);
    border-color: var(--amber-faint);
  }
  .tab:hover::before {
    content: '';
    position: absolute;
    inset: 0 0 auto 0;
    height: 2px;
    border-radius: var(--space-xs) var(--space-xs) 0 0;
    background: var(--amber-dim);
    opacity: 0.7;
  }
  .tab:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .tab.active {
    color: var(--amber-bright);
    background: var(--bg-base);
    border-color: var(--border-active);
    box-shadow: inset 0 2px 8px rgba(255, 200, 64, 0.06),
                0 -1px 4px rgba(0, 0, 0, 0.3);
    text-shadow: var(--glow-amber-strong);
    z-index: 1;
  }
  .tab.active::before {
    content: '';
    position: absolute;
    inset: -1px 0 auto 0;
    height: 3px;
    border-radius: var(--space-xs) var(--space-xs) 0 0;
    background: var(--amber-bright);
    box-shadow: 0 0 12px rgba(255, 200, 64, 0.6), 0 0 4px rgba(255, 200, 64, 0.4);
  }
  .tab.active::after {
    content: '';
    position: absolute;
    left: 0; right: 0; bottom: -1px;
    height: 2px;
    background: var(--bg-base);
    pointer-events: none;
  }
  /* U-06: dead PTY tab indicator */
  .tab.session.dead { opacity: 0.5; }
  .tab.session.dead .icon { color: var(--term-red); }
  .tab.session.dead.active .icon { color: var(--term-red); opacity: 0.8; }

  /* Session tab drag-to-reorder */
  .tab.session { cursor: grab; }
  .tab.session:active { cursor: grabbing; }
  .tab.session.drag-source { opacity: 0.4; }
  .tab.session.reorder-target {
    box-shadow: inset 3px 0 0 var(--amber-bright);
    background: var(--bg-hover);
  }
  .tab-rename-input {
    background: var(--bg-base);
    color: var(--amber-bright);
    border: 1px solid var(--amber-primary);
    border-radius: 2px;
    font-family: inherit;
    font-size: var(--text-base);
    font-weight: 500;
    padding: 0 var(--space-xs);
    height: var(--space-xl);
    width: 100px;
    outline: 2px solid transparent;
    box-shadow: 0 0 6px rgba(255, 168, 38, 0.3);
  }
  .tab-rename-input::selection {
    background: rgba(255, 168, 38, 0.3);
  }
  .icon { font-size: var(--text-sm); opacity: 0.85; transition: opacity 0.12s; }
  .tab.active .icon { opacity: 1; color: var(--amber-bright); }

  /* Close button — 18×18 click target, smooth red transition */
  .close {
    margin-left: var(--space-xs);
    color: var(--amber-dim);
    font-size: var(--text-base);
    width: 18px;
    height: 18px;
    line-height: 18px;
    text-align: center;
    cursor: pointer;
    border-radius: 2px;
    transition: color 0.18s, background 0.18s;
    flex-shrink: 0;
  }
  .close:hover {
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.12);
  }

  .add {
    width: 34px;
    background: transparent;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--space-xs) var(--space-xs) 0 0;
    margin-top: var(--space-xs);
    color: var(--amber-warm);
    cursor: pointer;
    font-size: 15px;
    font-family: inherit;
    transition: color 0.12s, background 0.12s, text-shadow 0.12s,
                border-color 0.12s;
  }
  .add:hover {
    color: var(--amber-bright);
    background: var(--bg-hover);
    border-color: var(--border-subtle);
    text-shadow: 0 0 8px rgba(255, 200, 64, 0.5);
  }
  .add:active { transform: scale(0.92); }
  .add:focus-visible, .manage:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  /* Phase 8.7h — manage button (notif strip tail) */
  .manage {
    width: 30px;
    background: transparent;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--space-xs) var(--space-xs) 0 0;
    margin-top: var(--space-xs);
    margin-left: 2px;
    color: var(--amber-faint);
    cursor: pointer;
    font-size: var(--text-lg);
    font-family: inherit;
    transition: color 0.12s, background 0.12s, border-color 0.12s;
  }
  .manage:hover {
    color: var(--amber-bright);
    background: var(--bg-hover);
    border-color: var(--border-subtle);
  }
  .cockpit-toggle {
    border-left: 1px solid var(--border-subtle);
    margin-left: var(--space-xs);
    padding-left: 2px;
  }
  .cockpit-toggle.collapsed {
    color: var(--amber-dim);
  }

</style>
