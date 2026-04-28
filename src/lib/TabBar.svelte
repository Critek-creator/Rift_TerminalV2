<script lang="ts">
  // §10.5 — tab strip; left group = session tabs, right group = notification
  // tabs. Phase 3 ships click-to-switch + +/× for sessions + per-tab toggle
  // for notifications (§10.6). Phase 3.5a adds drag-promote: drag a notif
  // tab off the strip → App promotes it to a fixed-width right-side pane;
  // drag the pane handle back onto the strip → demote.

  export type SessionTab = { id: number; title: string };
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
    onAddSession: () => void;
    onToggleNotif: (id: string) => void;
    onPromote: (id: string) => void;
    onDemote: () => void;
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
  }: Props = $props();

  // §10.9 — "Amber border animates around a tab when something is live/active
  // inside it." A tab is "live" if any envelope arrived in the last 3 seconds.
  const LIVE_WINDOW_MS = 3000;
  function isLive(tab: NotifTab): boolean {
    return tab.lastActivityTs !== null && (tickNow - tab.lastActivityTs) < LIVE_WINDOW_MS;
  }
  // Visible notifs are detected ones — capability-gate filter (§10.7).
  // Disabled tabs still render (struck-through) so the user can re-enable
  // them via right-click; only undetected tabs are completely hidden.
  const visibleNotifs = $derived(notifs.filter((n) => n.detected));

  // Drop-target highlight state — true while a drag is hovering the strip.
  let dropActive = $state(false);

  // Phase 6.6 regression-preventer (design call E):
  // Marker MIME type written by onNotifDragStart so onStripDrop can distinguish
  // a notif-tab drag from a tree-node drag (or any other foreign drop source).
  const NOTIF_TAB_MIME = 'application/x-rift-notif-tab';

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
    // Promoted tab in strip = no-op; user interacts with the side pane instead.
    if (isPromoted(tab.id)) return;
    onActivateNotif(tab.id);
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
    // Phase 6.6: only act when the drop carries a notif-tab payload.
    // Drops from foreign sources (e.g. tree-node paths) are acknowledged
    // (preventDefault already ran for visual coherence) but otherwise ignored.
    if (!e.dataTransfer?.types.includes(NOTIF_TAB_MIME)) return;

    // Only demote on strip-drop when the dragged tab was already promoted
    // BEFORE this gesture started. Otherwise dragging an unpromoted tab and
    // releasing within the strip's bounds would promote-then-demote in the
    // same gesture, making drag-to-promote silently no-op (the original bug
    // captured as pr003 `tabbar-drag-promote-demote-self-cancel-on-strip-drop`).
    if (draggedFromPromoted) {
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
  <div class="group">
    {#each sessions as tab (tab.id)}
      <button
        type="button"
        class="tab session"
        class:active={isActiveSession(tab.id)}
        aria-current={isActiveSession(tab.id) ? 'page' : 'false'}
        onclick={() => onActivateSession(tab.id)}
      >
        <span class="icon">▶</span>
        <span>{tab.title}</span>
        <span
          role="button"
          tabindex="0"
          class="close"
          aria-label="close tab"
          onclick={(e) => { e.stopPropagation(); onCloseSession(tab.id); }}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              e.stopPropagation();
              onCloseSession(tab.id);
            }
          }}
        >×</span>
      </button>
    {/each}
    <button type="button" class="add" aria-label="new tab" onclick={onAddSession}>+</button>
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
        class:live={isLive(tab) && tab.enabled}
        aria-current={isActiveNotif(tab.id) ? 'page' : 'false'}
        draggable={tab.enabled}
        onclick={() => onNotifClick(tab)}
        ondragstart={(e) => onNotifDragStart(e, tab)}
        oncontextmenu={(e) => { e.preventDefault(); onToggleNotif(tab.id); }}
        title={tab.enabled
          ? (isPromoted(tab.id)
              ? 'promoted to side pane · drag pane handle back to dock'
              : 'click to open · drag to promote · right-click to disable')
          : 'right-click to enable'}
      >
        <span class="icon">{isPromoted(tab.id) ? '↗' : tab.icon}</span>
        <span>{tab.title}</span>
        {#if tab.unreadCount > 0 && tab.enabled}
          <span class="badge" aria-label="{tab.unreadCount} unread events">
            {tab.unreadCount > 99 ? '99+' : tab.unreadCount}
          </span>
        {/if}
      </button>
    {/each}
  </div>
</nav>

<style>
  .tabbar {
    height: 36px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: stretch;
    flex-shrink: 0;
  }
  .tabbar.drop-active {
    box-shadow: inset 0 0 0 1px var(--amber-bright);
  }
  .group { display: flex; align-items: stretch; }
  .group.right {
    margin-left: auto;
    border-left: 1px solid var(--border-subtle);
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    height: 100%;
    border: none;
    border-right: 1px solid var(--border-subtle);
    background: transparent;
    color: var(--amber-dim);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    position: relative;
    transition: color 0.12s, background 0.12s;
    user-select: none;
  }
  .tab:hover {
    color: var(--amber-warm);
    background: var(--bg-hover);
  }
  .tab.active {
    color: var(--amber-primary);
    background: var(--bg-base);
    text-shadow: var(--glow-amber);
  }
  .tab.active::before {
    content: '';
    position: absolute;
    inset: 0 0 auto 0;
    height: 2px;
    background: var(--amber-bright);
    box-shadow: 0 0 6px var(--amber-bright);
  }
  .tab.notif { cursor: grab; }
  .tab.notif:active { cursor: grabbing; }
  .tab.notif.disabled {
    color: var(--amber-faint);
    text-decoration: line-through;
    cursor: pointer;
    opacity: 0.55;
  }
  .tab.notif.disabled:hover {
    color: var(--amber-dim);
    opacity: 0.85;
  }
  .tab.notif.promoted {
    opacity: 0.55;
    cursor: default;
  }
  .tab.notif.promoted .icon {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    opacity: 1;
  }
  .tab.notif.promoted-cyan .icon { color: var(--term-cyan); }
  .tab.notif.promoted-red .icon { color: var(--term-red); }

  .icon { font-size: 11px; opacity: 0.85; }

  .close {
    margin-left: 4px;
    color: var(--amber-faint);
    font-size: 12px;
    width: 14px;
    height: 14px;
    line-height: 14px;
    text-align: center;
    cursor: pointer;
  }
  .close:hover { color: var(--term-red); }

  .add {
    width: 36px;
    background: transparent;
    border: none;
    border-right: 1px solid var(--border-subtle);
    color: var(--amber-dim);
    cursor: pointer;
    font-size: 14px;
    font-family: inherit;
  }
  .add:hover { color: var(--amber-primary); background: var(--bg-hover); }

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
  }

  /* §10.9 — live-active animated amber border. The pulse runs on the bottom
     border (sits under the existing 2px amber active-state border without
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
  /* When the tab is also active, the existing ::before solid bar is the
     primary signal; the live ::after still pulses underneath but shifts to
     an outer glow halo so the two cues don't fight visually. */
  .tab.notif.live.active::after {
    inset: 0;
    background: transparent;
    border: 1px solid var(--amber-bright);
    box-shadow: 0 0 10px rgba(245, 158, 11, 0.55);
  }
  @keyframes notif-live-pulse {
    0%, 100% { opacity: 1;   box-shadow: 0 0 6px  var(--amber-bright); }
    50%      { opacity: 0.55; box-shadow: 0 0 12px var(--amber-bright); }
  }
</style>
