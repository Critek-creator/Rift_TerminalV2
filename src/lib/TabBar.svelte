<script lang="ts">
  import { NOTIF_TAB_MIME } from './dragMime';

  // §10.5 — tab strip; left group = session tabs, right group = notification
  // tabs. Phase 3 ships click-to-switch + +/× for sessions + per-tab toggle
  // for notifications (§10.6). Phase 3.5a adds drag-promote: drag a notif
  // tab off the strip → App promotes it to a fixed-width right-side pane;
  // drag the pane handle back onto the strip → demote.

  export type SessionTab = { id: number; title: string; projectPath: string | null };
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
    | { kind: 'notification'; id: string }
    | { kind: 'empty' };

  interface Props {
    sessions: SessionTab[];
    notifs: NotifTab[];
    active: ActiveSurface;
    promotedId: string | null;
    /** Updated every ~1s by App.svelte; drives the live-border decay window. */
    tickNow: number;
    onActivateSession: (id: number) => void;
    onActivateNotif: (id: string) => void;
    onCloseSession: (id: number) => void;
    onAddSession: (opts?: { pickProject?: boolean }) => void;
    onToggleNotif: (id: string) => void;
    onPromote: (id: string) => void;
    onDemote: () => void;
    /** Phase 8.7h — open the notif tab manager popout. */
    onManageNotifs: () => void;
    /** Phase 8.7j — reorder via drag-onto-other-tab. App splices `srcId`
     *  into `dstId`'s slot and persists the new order to localStorage. */
    onReorderNotif: (srcId: string, dstId: string) => void;
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
    /** Toggle cockpit right-pane visibility. */
    onToggleCockpit?: () => void;
  }

  let {
    sessions,
    notifs,
    active,
    promotedId,
    tickNow,
    onActivateSession,
    onActivateNotif,
    onCloseSession,
    onAddSession,
    onToggleNotif,
    onPromote,
    onDemote,
    onManageNotifs,
    onReorderNotif,
    onDetach,
    detachedIds,
    multiProject = false,
    cockpitCollapsed = false,
    alertTriggered = {},
    onToggleCockpit,
  }: Props = $props();

  function isDetached(id: string): boolean {
    return detachedIds.has(id);
  }

  function tabDisplayTitle(tab: SessionTab): string {
    if (!multiProject || !tab.projectPath) return tab.title;
    const name = tab.projectPath.split(/[\\/]/).at(-1) ?? '';
    return name ? `${tab.title} · ${name}` : tab.title;
  }

  // §10.9 — "Amber border animates around a tab when something is live/active
  // inside it." A tab is "live" if any envelope arrived in the last 3 seconds.
  const LIVE_WINDOW_MS = 3000;
  function isLive(tab: NotifTab): boolean {
    return tab.lastActivityTs !== null && (tickNow - tab.lastActivityTs) < LIVE_WINDOW_MS;
  }
  // Visible notifs are detected AND enabled.
  //   - undetected: capability-gate hide (§10.7) — integration hasn't loaded.
  //   - disabled:   user-hidden via right-click or NotifManager popout.
  // Re-enable through the `⋯` manager button at the right edge of the strip
  // (Phase 8.7j: prior behaviour was struck-through-but-still-rendered, which
  // didn't actually clear the strip when the user wanted to declutter).
  const visibleNotifs = $derived(notifs.filter((n) => n.detected && n.enabled));

  // Drop-target highlight state — true while a drag is hovering the strip.
  let dropActive = $state(false);

  // Phase 6.6 regression-preventer (design call E):
  // Marker MIME type — defined in dragMime.ts so NotificationPane drag-back
  // dragstart can write the same constant (silent MIME mismatch on the
  // pane→tab demote path was the Phase 8.7 BV regression).

  function isActiveSession(id: number) {
    return active.kind === 'session' && active.id === id;
  }
  function isActiveNotif(id: string) {
    return active.kind === 'notification' && active.id === id;
  }
  function isPromoted(id: string) {
    return promotedId === id;
  }

  function onNotifClick(tab: NotifTab) {
    if (!tab.enabled) return;
    onActivateNotif(tab.id);
  }

  // Phase 8.7j — drag-to-reorder. When the drop lands on another notif
  // tab (not the strip background), we treat the gesture as "reorder, not
  // promote" and undo the promote-on-dragstart. `reorderHoverId` drives
  // the per-tab insertion-line indicator.
  let reorderHoverId = $state<string | null>(null);
  let didReorder = false;
  let didDropInside = false;

  function onTabDragOver(e: DragEvent, tab: NotifTab) {
    if (!e.dataTransfer?.types.includes(NOTIF_TAB_MIME)) return;
    const payload = e.dataTransfer.getData(NOTIF_TAB_MIME);
    // Pane-source drags carry the sentinel — they're for demote, not reorder.
    if (payload === '__promoted_pane__') return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = 'move';
    reorderHoverId = tab.id;
  }
  function onTabDragLeave(tab: NotifTab) {
    if (reorderHoverId === tab.id) reorderHoverId = null;
  }
  function onTabDrop(e: DragEvent, tab: NotifTab) {
    if (!e.dataTransfer?.types.includes(NOTIF_TAB_MIME)) return;
    const srcId = e.dataTransfer.getData(NOTIF_TAB_MIME);
    if (!srcId || srcId === '__promoted_pane__' || srcId === tab.id) {
      reorderHoverId = null;
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    onReorderNotif(srcId, tab.id);
    // Reorder gesture supersedes the promote that fired on dragstart —
    // immediately demote so the tab settles back into the strip.
    if (isPromoted(srcId)) onDemote();
    didReorder = true;
    didDropInside = true;
    reorderHoverId = null;
  }

  // Tracks whether the tab being dragged was ALREADY promoted at dragstart time.
  // Used by onStripDrop to decide demote-vs-keep on drop-back-to-strip:
  //   - dragged-from-promoted + dropped-on-strip = explicit demote (user dragging
  //     the promoted tab back to the strip cancels promotion)
  //   - dragged-from-strip + dropped-on-strip = keep promoted (user started a
  //     drag-to-promote gesture but released within the strip; we still want
  //     the promote to stick rather than silently undo within the same gesture)
  // Without this state, the original code auto-demoted on any strip drop, which
  // made drag-to-promote appear broken — promote+demote happened in one gesture.
  let draggedFromPromoted = false;

  function onNotifDragStart(e: DragEvent, tab: NotifTab) {
    if (!tab.enabled) return;
    draggedFromPromoted = isPromoted(tab.id);
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Phase 6.6: marker MIME lets onStripDrop identify notif-tab drags and
      // ignore drops from other sources (e.g. tree-node drags).
      e.dataTransfer.setData(NOTIF_TAB_MIME, tab.id);
      e.dataTransfer.setData('text/plain', tab.id);
    }
    // Promote on dragstart only if the tab wasn't already promoted; dragging an
    // already-promoted tab from the strip is a demote gesture (resolved on drop).
    if (!draggedFromPromoted) onPromote(tab.id);
    didDropInside = false;
  }

  function onStripDragOver(e: DragEvent) {
    // preventDefault to mark the strip as a valid drop target.
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
    // Phase 8.7j — if a per-tab drop already handled this gesture as a
    // reorder, the strip-level drop must not also fire its demote/keep
    // logic (would otherwise clobber the reorder by demoting).
    if (didReorder) {
      didReorder = false;
      draggedFromPromoted = false;
      return;
    }
    // Phase 6.6: only act when the drop carries a notif-tab payload.
    // Drops from foreign sources (e.g. tree-node paths) are acknowledged
    // (preventDefault already ran for visual coherence) but otherwise ignored.
    if (!e.dataTransfer?.types.includes(NOTIF_TAB_MIME)) return;

    // Two valid demote paths:
    //   1. Tab-source drag from an already-promoted strip tab dropped back
    //      on the strip (draggedFromPromoted, set in onNotifDragStart).
    //   2. Pane-source drag from a NotificationPane / AegisTabContent /
    //      IndexTabContent drag-handle dropped on the strip — these write
    //      the sentinel value '__promoted_pane__' as their NOTIF_TAB_MIME
    //      payload (vs tab.id for tab-source drags). draggedFromPromoted is
    //      not set for pane-source drags because onNotifDragStart never ran.
    // The promote-then-demote self-cancel guard (pr003 `tabbar-drag-promote-
    // demote-self-cancel-on-strip-drop`) only applies to case 1 — pane-source
    // drags by definition come from an already-promoted tab.
    const payload = e.dataTransfer.getData(NOTIF_TAB_MIME);
    const isPaneSourceDrag = payload === '__promoted_pane__';
    didDropInside = true;
    if (isPaneSourceDrag || draggedFromPromoted) {
      onDemote();
    }
    draggedFromPromoted = false;
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
        role="tab"
        tabindex="0"
        aria-selected={isActiveSession(tab.id)}
        onclick={() => onActivateSession(tab.id)}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onActivateSession(tab.id);
          }
        }}
      >
        <span class="icon">▶</span>
        <span>{tabDisplayTitle(tab)}</span>
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
    {#each visibleNotifs as tab (tab.id)}
      <button
        type="button"
        class="tab notif"
        class:active={isActiveNotif(tab.id)}
        class:disabled={!tab.enabled}
        class:promoted={isPromoted(tab.id)}
        class:promoted-cyan={isPromoted(tab.id) && tab.id === 'hooks'}
        class:promoted-red={isPromoted(tab.id) && tab.id === 'errors'}
        class:detached={isDetached(tab.id)}
        class:live={isLive(tab) && tab.enabled}
        class:reorder-target={reorderHoverId === tab.id}
        aria-current={isActiveNotif(tab.id) ? 'page' : 'false'}
        draggable={tab.enabled && !isDetached(tab.id)}
        onclick={() => onNotifClick(tab)}
        ondragstart={(e) => onNotifDragStart(e, tab)}
        ondragend={(e) => {
          if (e.dataTransfer?.dropEffect === 'none' && !didDropInside && tab.enabled && !isDetached(tab.id)) {
            onDetach(tab.id);
          }
          didDropInside = false;
        }}
        ondragover={(e) => onTabDragOver(e, tab)}
        ondragleave={() => onTabDragLeave(tab)}
        ondrop={(e) => onTabDrop(e, tab)}
        oncontextmenu={(e) => { e.preventDefault(); onToggleNotif(tab.id); }}
        title={tab.enabled
          ? (isDetached(tab.id)
              ? 'detached to own window · click to focus'
              : isPromoted(tab.id)
                ? 'click to close side pane · drag to reorder · right-click to hide'
                : 'click to open · drag onto another tab to reorder · drag off strip to promote · right-click to hide')
          : 'right-click to enable'}
      >
        <span class="icon">{isDetached(tab.id) ? '⬡' : isPromoted(tab.id) ? '↗' : tab.icon}</span>
        <span>{tab.title}</span>
        {#if tab.unreadCount > 0 && tab.enabled}
          <span class="badge" class:alert-flash={alertTriggered[tab.id]} aria-label="{tab.unreadCount} unread events">
            {tab.unreadCount > 99 ? '99+' : tab.unreadCount}
          </span>
        {/if}
        {#if tab.enabled && !isDetached(tab.id)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <span
            role="button"
            tabindex="-1"
            class="popout-btn"
            aria-label="detach {tab.title} to own window"
            onclick={(e) => { e.stopPropagation(); onDetach(tab.id); }}
            title="pop out to own window"
          >⧉</span>
        {/if}
      </button>
    {/each}
    <!-- Phase 8.7h — notif manager trigger. Right-click on a tab still
         toggles enabled directly; this gear opens the discoverable
         management popout. -->
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
    padding-left: 6px;
    border-left: 2px solid var(--border-active);
    box-shadow: inset 1px 0 8px rgba(0, 0, 0, 0.3);
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 18px;
    height: calc(100% - 4px);
    margin-top: 4px;
    border: 1px solid var(--border-subtle);
    border-bottom: none;
    border-radius: 4px 4px 0 0;
    background: var(--bg-elevated);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 12px;
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
    border-radius: 4px 4px 0 0;
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
    border-radius: 4px 4px 0 0;
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
  /* Disabled tabs — visually distinct from hover, not just low opacity */
  .tab.notif.disabled {
    color: var(--amber-faint);
    text-decoration: line-through;
    cursor: pointer;
    opacity: 0.45;
    filter: saturate(0.4);
  }
  .tab.notif.disabled:hover {
    color: var(--amber-dim);
    opacity: 0.75;
    filter: saturate(0.7);
  }
  /* Suppress hover ::before tease on disabled tabs */
  .tab.notif.disabled:hover::before {
    display: none;
  }
  /* Promoted tabs get a subtle hover glow to signal click-to-close */
  .tab.notif.promoted:hover {
    opacity: 0.75;
    background: var(--bg-hover);
  }
  .tab.notif.promoted:hover::before {
    display: none;
  }
  .tab.notif { cursor: grab; }
  .tab.notif:active { cursor: grabbing; }
  /* Phase 8.7j — drop-target indicator */
  .tab.notif.reorder-target {
    box-shadow: inset 3px 0 0 var(--amber-bright);
    background: var(--bg-hover);
  }
  .tab.notif.promoted {
    opacity: 0.55;
    cursor: pointer;
  }
  .tab.notif.promoted .icon {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    opacity: 1;
  }
  .tab.notif.promoted-cyan .icon { color: var(--term-cyan); }
  .tab.notif.promoted-red .icon { color: var(--term-red); }

  .tab.notif.detached {
    opacity: 0.45;
    cursor: pointer;
  }
  .tab.notif.detached .icon {
    color: var(--term-blue);
    opacity: 1;
  }

  .popout-btn {
    display: none;
    margin-left: 2px;
    padding: 0 3px;
    height: 16px;
    line-height: 16px;
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-faint);
    font-size: 9px;
    font-family: inherit;
    cursor: pointer;
    border-radius: 2px;
    flex-shrink: 0;
  }
  .tab.notif:hover .popout-btn { display: inline-flex; }
  .popout-btn:hover {
    color: var(--term-blue);
    border-color: var(--term-blue);
    background: rgba(108, 182, 255, 0.08);
  }

  .icon { font-size: 11px; opacity: 0.85; transition: opacity 0.12s; }
  .tab.active .icon { opacity: 1; color: var(--amber-bright); }

  /* Close button — 18×18 click target, smooth red transition */
  .close {
    margin-left: 4px;
    color: var(--amber-faint);
    font-size: 12px;
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
    border-radius: 4px 4px 0 0;
    margin-top: 4px;
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
    border-radius: 4px 4px 0 0;
    margin-top: 4px;
    margin-left: 2px;
    color: var(--amber-faint);
    cursor: pointer;
    font-size: 14px;
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
    margin-left: 4px;
    padding-left: 2px;
  }
  .cockpit-toggle.collapsed {
    color: var(--amber-dim);
  }

  .badge {
    background: var(--amber-bright);
    color: var(--bg-base);
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    margin-left: 2px;
    min-width: 16px;
    text-align: center;
    letter-spacing: 0.04em;
    line-height: 14px;
    border-radius: 8px;
    box-shadow: 0 0 6px rgba(255, 200, 64, 0.4);
    animation: badge-pulse 2s ease-in-out infinite;
  }
  @keyframes badge-pulse {
    0%, 100% { opacity: 1; }
    50%       { opacity: 0.7; }
  }
  .badge.alert-flash {
    background: var(--term-red, #FF4848);
    box-shadow: 0 0 8px rgba(255, 72, 72, 0.6);
    animation: alert-flash-anim 0.5s ease-in-out 4;
  }
  @keyframes alert-flash-anim {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(1.3); }
  }

  /* §10.9 — live-active animated amber border. The pulse runs on the bottom
     border (sits under the existing 3px amber active-state border without
     conflicting) plus a soft outer glow. Disabled / promoted tabs do not
     pulse — the strip is for "look here" signaling, and a promoted tab is
     already living in its side pane. */
  .tab.notif.live::after {
    content: '';
    position: absolute;
    inset: auto 0 0 0;
    height: 2px;
    background: var(--amber-bright);
    box-shadow: 0 0 6px var(--amber-bright);
    animation: notif-live-pulse 1.4s ease-in-out infinite;
    pointer-events: none;
  }
  /* When the tab is also active, the ::after from .tab.active is already
     rendering a bottom gradient; override to the live-pulse variant. */
  .tab.notif.live.active::after {
    inset: auto 0 0 0;
    height: 2px;
    background: var(--amber-bright);
    border: none;
    box-shadow: 0 0 10px rgba(245, 158, 11, 0.55);
    animation: notif-live-pulse 1.4s ease-in-out infinite;
  }
  @keyframes notif-live-pulse {
    0%, 100% { opacity: 1;   box-shadow: 0 0 6px  var(--amber-bright); }
    50%      { opacity: 0.55; box-shadow: 0 0 12px var(--amber-bright); }
  }
</style>
