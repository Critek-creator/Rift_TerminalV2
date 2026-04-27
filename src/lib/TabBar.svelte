<script lang="ts">
  // §10.5 — tab strip; left group = session tabs, right group = notification
  // tabs. Phase 3 ships click-to-switch + +/× for sessions + per-tab toggle
  // for notifications (§10.6). Drag-promote-to-pane lands a later phase.

  export type SessionTab = { id: number; title: string };
  export type NotifTab = {
    id: string;
    title: string;
    icon: string;
    enabled: boolean;
    badge?: { text: string; alert?: boolean };
  };

  export type ActiveSurface =
    | { kind: 'session'; id: number }
    | { kind: 'notification'; id: string }
    | { kind: 'empty' };

  interface Props {
    sessions: SessionTab[];
    notifs: NotifTab[];
    active: ActiveSurface;
    onActivateSession: (id: number) => void;
    onActivateNotif: (id: string) => void;
    onCloseSession: (id: number) => void;
    onAddSession: () => void;
    onToggleNotif: (id: string) => void;
  }

  let {
    sessions,
    notifs,
    active,
    onActivateSession,
    onActivateNotif,
    onCloseSession,
    onAddSession,
    onToggleNotif,
  }: Props = $props();

  function isActiveSession(id: number) {
    return active.kind === 'session' && active.id === id;
  }
  function isActiveNotif(id: string) {
    return active.kind === 'notification' && active.id === id;
  }
</script>

<nav class="tabbar">
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
    {#each notifs as tab (tab.id)}
      <button
        type="button"
        class="tab notif"
        class:active={isActiveNotif(tab.id)}
        class:disabled={!tab.enabled}
        aria-current={isActiveNotif(tab.id) ? 'page' : 'false'}
        onclick={() => tab.enabled && onActivateNotif(tab.id)}
        oncontextmenu={(e) => { e.preventDefault(); onToggleNotif(tab.id); }}
        title={tab.enabled ? 'click to open · right-click to disable' : 'right-click to enable'}
      >
        <span class="icon">{tab.icon}</span>
        <span>{tab.title}</span>
        {#if tab.badge && tab.enabled}
          <span class="badge" class:alert={tab.badge.alert}>{tab.badge.text}</span>
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
  }
  .badge.alert {
    background: var(--term-red);
    color: var(--term-white);
    animation: pulse 2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50%      { opacity: 0.6; }
  }
</style>
