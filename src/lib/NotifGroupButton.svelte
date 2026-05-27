<script lang="ts">
  import type { NotifTab } from './TabBar.svelte';
  import type { NotifGroupState } from './notifState.svelte';

  interface Props {
    group: NotifGroupState;
    promotedId: string | null;
    tickNow: number;
    detachedIds: Set<string>;
    alertTriggered: Record<string, boolean>;
    onActivateNotif: (id: string) => void;
    onToggleNotif: (id: string) => void;
    onDetach: (id: string) => void;
  }

  let {
    group,
    promotedId,
    tickNow,
    detachedIds,
    alertTriggered = {},
    onActivateNotif,
    onToggleNotif,
    onDetach,
  }: Props = $props();

  let open = $state(false);
  let groupEl: HTMLDivElement | undefined = $state(undefined);

  const LIVE_WINDOW_MS = 3000;

  const hasLive = $derived(
    group.tabs.some((t) => t.lastActivityTs !== null && (tickNow - t.lastActivityTs) < LIVE_WINDOW_MS)
  );

  const promotedTab = $derived(
    group.tabs.find((t) => t.id === promotedId)
  );

  function isLive(tab: NotifTab): boolean {
    return tab.lastActivityTs !== null && (tickNow - tab.lastActivityTs) < LIVE_WINDOW_MS;
  }

  function toggle() {
    open = !open;
  }

  function handleTabClick(tab: NotifTab) {
    onActivateNotif(tab.id);
    open = false;
  }

  function handleTabContextMenu(e: MouseEvent, tab: NotifTab) {
    e.preventDefault();
    onToggleNotif(tab.id);
  }

  function handleWindowClick(e: MouseEvent) {
    if (open && groupEl && !groupEl.contains(e.target as Node)) {
      open = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) {
      open = false;
    }
  }
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleKeydown} />

<div class="notif-group" class:open bind:this={groupEl}>
  <button
    type="button"
    class="group-btn"
    class:has-promoted={!!promotedTab}
    class:has-live={hasLive}
    data-accent={group.accent}
    onclick={toggle}
    title={promotedTab
      ? `${group.title}: ${promotedTab.title} open · click to browse`
      : `${group.title} · click to browse tabs`}
  >
    <span class="icon">{group.icon}</span>
    <span class="label">{promotedTab ? promotedTab.title : group.title}</span>
    {#if group.aggregateBadge > 0}
      <span class="badge" data-accent={group.accent}>
        {group.aggregateBadge > 999 ? '999+' : group.aggregateBadge}
      </span>
    {/if}
    <span class="chevron">{open ? '▴' : '▾'}</span>
  </button>

  {#if open}
    <div class="dropdown" role="menu">
      {#each group.tabs as tab (tab.id)}
        <button
          type="button"
          class="dropdown-tab"
          class:promoted={tab.id === promotedId}
          class:detached={detachedIds.has(tab.id)}
          class:live={isLive(tab)}
          data-accent={tab.id}
          role="menuitem"
          onclick={() => handleTabClick(tab)}
          oncontextmenu={(e) => handleTabContextMenu(e, tab)}
          title={detachedIds.has(tab.id)
            ? 'detached to own window'
            : tab.id === promotedId
              ? 'click to close side pane'
              : 'click to open · right-click to hide'}
        >
          <span class="tab-icon">
            {detachedIds.has(tab.id) ? '⬡' : tab.id === promotedId ? '↗' : tab.icon}
          </span>
          <span class="tab-name">{tab.title}</span>
          {#if tab.unreadCount > 0}
            <span
              class="tab-badge"
              class:alert-flash={alertTriggered[tab.id]}
              data-accent={tab.id}
            >
              {tab.unreadCount > 99 ? '99+' : tab.unreadCount}
            </span>
          {/if}
          {#if !detachedIds.has(tab.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <span
              role="button"
              tabindex="-1"
              class="popout-btn"
              onclick={(e) => { e.stopPropagation(); onDetach(tab.id); }}
              title="pop out to own window"
            >⧉</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .notif-group {
    position: relative;
    display: flex;
    align-items: stretch;
  }

  .group-btn {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: 0 var(--space-md);
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
    transition: color 0.15s, background 0.15s, border-color 0.15s;
    user-select: none;
    white-space: nowrap;
  }
  .group-btn:hover {
    color: var(--amber-bright);
    background: var(--bg-hover);
    border-color: var(--amber-faint);
  }
  .group-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .group-btn.has-promoted {
    color: var(--amber-bright);
    background: var(--bg-base);
    border-color: var(--border-active);
    box-shadow: inset 0 2px 8px rgba(255, 200, 64, 0.06);
  }
  .group-btn.has-promoted::before {
    content: '';
    position: absolute;
    inset: -1px 0 auto 0;
    height: 3px;
    border-radius: var(--space-xs) var(--space-xs) 0 0;
    background: var(--amber-bright);
    box-shadow: 0 0 12px rgba(255, 200, 64, 0.6);
  }

  /* Live pulse on group when any child tab has recent activity */
  .group-btn.has-live::after {
    content: '';
    position: absolute;
    inset: auto 0 0 0;
    height: 2px;
    background: var(--amber-bright);
    box-shadow: 0 0 6px var(--amber-bright);
    animation: group-live-pulse 1.4s ease-in-out infinite;
    pointer-events: none;
  }
  @keyframes group-live-pulse {
    0%, 100% { opacity: 1;    box-shadow: 0 0 6px  var(--amber-bright); }
    50%      { opacity: 0.55; box-shadow: 0 0 12px var(--amber-bright); }
  }

  .icon { font-size: var(--text-sm); opacity: 0.85; }
  .group-btn.has-promoted .icon { opacity: 1; }

  .label { font-size: var(--text-sm); }

  .chevron {
    font-size: 8px;
    opacity: 0.5;
    margin-left: 2px;
  }

  .badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    padding: 1px 5px;
    margin-left: 2px;
    min-width: var(--space-lg);
    text-align: center;
    letter-spacing: 0.04em;
    line-height: var(--space-14);
    border-radius: var(--space-8);
    background: var(--amber-bright);
    color: var(--bg-base);
    box-shadow: 0 0 6px rgba(255, 200, 64, 0.4);
  }
  /* Group-level badge accent colors */
  .badge[data-accent="errors"] { background: var(--term-red); box-shadow: 0 0 6px rgba(255, 72, 72, 0.4); }
  .badge[data-accent="agents"] { background: var(--term-purple); box-shadow: 0 0 6px rgba(197, 143, 255, 0.4); }

  /* ---- Dropdown ---- */
  .dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 100;
    min-width: 200px;
    max-width: 280px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-active);
    border-radius: 0 0 var(--radius-md, 4px) var(--radius-md, 4px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5), 0 2px 8px rgba(0, 0, 0, 0.3);
    padding: var(--space-xs) 0;
    display: flex;
    flex-direction: column;
  }

  .dropdown-tab {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-md);
    background: transparent;
    border: none;
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    transition: color 0.12s, background 0.12s;
    text-align: left;
    position: relative;
  }
  .dropdown-tab:hover {
    color: var(--amber-bright);
    background: var(--bg-hover);
  }
  .dropdown-tab.promoted {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.06);
  }
  .dropdown-tab.promoted::before {
    content: '';
    position: absolute;
    left: 0;
    top: 2px;
    bottom: 2px;
    width: 3px;
    background: var(--amber-bright);
    border-radius: 0 2px 2px 0;
  }
  .dropdown-tab.detached {
    opacity: 0.5;
  }
  .dropdown-tab.detached .tab-icon {
    color: var(--term-blue);
    opacity: 1;
  }

  /* Live indicator on individual dropdown tabs */
  .dropdown-tab.live::after {
    content: '';
    position: absolute;
    right: var(--space-xs);
    top: 50%;
    transform: translateY(-50%);
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--amber-bright);
    box-shadow: 0 0 4px var(--amber-bright);
    animation: group-live-pulse 1.4s ease-in-out infinite;
  }

  .tab-icon {
    font-size: var(--text-sm);
    opacity: 0.85;
    flex-shrink: 0;
    width: 16px;
    text-align: center;
  }
  .dropdown-tab.promoted .tab-icon {
    color: var(--amber-bright);
    opacity: 1;
  }

  .tab-name { flex: 1; }

  .tab-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    padding: 1px 5px;
    min-width: var(--space-lg);
    text-align: center;
    letter-spacing: 0.04em;
    line-height: var(--space-14);
    border-radius: var(--space-8);
    background: var(--amber-bright);
    color: var(--bg-base);
    box-shadow: 0 0 4px rgba(255, 200, 64, 0.3);
  }
  .tab-badge.alert-flash {
    background: var(--term-red);
    box-shadow: 0 0 8px rgba(255, 72, 72, 0.6);
    animation: alert-flash-anim 0.5s ease-in-out 4;
  }
  @keyframes alert-flash-anim {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(1.3); }
  }

  /* Per-tab accent badge colors in dropdown */
  .tab-badge[data-accent="hooks"] { background: var(--term-cyan); box-shadow: 0 0 4px rgba(111, 224, 224, 0.3); }
  .tab-badge[data-accent="errors"] { background: var(--term-red); box-shadow: 0 0 4px rgba(255, 72, 72, 0.3); }
  .tab-badge[data-accent="agents"] { background: var(--term-purple); box-shadow: 0 0 4px rgba(197, 143, 255, 0.3); }
  .tab-badge[data-accent="git"] { background: var(--term-green); box-shadow: 0 0 4px rgba(79, 232, 85, 0.3); }
  .tab-badge[data-accent="sessions"],
  .tab-badge[data-accent="cmd-intelligence"] { background: var(--term-blue); box-shadow: 0 0 4px rgba(108, 182, 255, 0.3); }
  .tab-badge[data-accent="mcp"] { background: var(--term-cyan); box-shadow: 0 0 4px rgba(111, 224, 224, 0.3); }

  .popout-btn {
    display: none;
    margin-left: 2px;
    padding: 0 3px;
    height: var(--space-lg);
    line-height: var(--space-lg);
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-family: inherit;
    cursor: pointer;
    border-radius: 2px;
    flex-shrink: 0;
  }
  .dropdown-tab:hover .popout-btn { display: inline-flex; }
  .popout-btn:hover {
    color: var(--term-blue);
    border-color: var(--term-blue);
    background: rgba(108, 182, 255, 0.08);
  }
</style>
