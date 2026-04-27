<script lang="ts">
  import TitleBar from './lib/TitleBar.svelte';
  import TabBar, {
    type SessionTab,
    type NotifTab,
    type ActiveSurface,
  } from './lib/TabBar.svelte';
  import Terminal from './lib/Terminal.svelte';
  import NotificationPane from './lib/NotificationPane.svelte';
  import StatusLine from './lib/StatusLine.svelte';
  import type { Category } from './lib/bus';

  // Tab id → bus category. `undefined` = no integration registered yet,
  // so the pane stays in placeholder mode until a translator lights it up.
  const CATEGORY_BY_NOTIF: Record<string, Category | undefined> = {
    hooks: 'hook',
    errors: undefined,    // wires to Category::System filtered to error kinds in a future phase
    commands: undefined,  // wires when a command translator exists
  };

  // ----- session tabs -----
  let nextSessionId = 1;
  let sessions = $state<SessionTab[]>([{ id: 0, title: 'rift' }]);

  // ----- notification tabs (default set per §10.7) -----
  let notifs = $state<NotifTab[]>([
    { id: 'errors',   title: 'errors',   icon: '⚡', enabled: true },
    { id: 'hooks',    title: 'hooks',    icon: '⚓', enabled: true },
    { id: 'commands', title: 'commands', icon: '⌘', enabled: true },
  ]);

  // ----- which surface is in the main pane -----
  let active = $state<ActiveSurface>({ kind: 'session', id: 0 });

  // ----- handlers -----
  function activateSession(id: number) {
    active = { kind: 'session', id };
  }
  function activateNotif(id: string) {
    active = { kind: 'notification', id };
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
    if (active.kind === 'notification' && active.id === id) {
      const enabled = notifs.find((n) => n.id === id)?.enabled;
      if (!enabled) {
        const fallback = sessions.at(0);
        active = fallback ? { kind: 'session', id: fallback.id } : { kind: 'empty' };
      }
    }
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
</script>

<div class="app-shell">
  <TitleBar />
  <TabBar
    {sessions}
    {notifs}
    {active}
    onActivateSession={activateSession}
    onActivateNotif={activateNotif}
    onCloseSession={closeSession}
    onAddSession={addSession}
    onToggleNotif={toggleNotif}
  />

  <main class="main">
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
         subscription rather than reusing one tied to the previous tab. -->
    {#if activeNotifTab}
      {#key activeNotifTab.id}
        <div class="surface visible">
          <NotificationPane
            title={activeNotifTab.title}
            icon={activeNotifTab.icon}
            accent={notifAccent(activeNotifTab.id)}
            categoryFilter={CATEGORY_BY_NOTIF[activeNotifTab.id]}
          />
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
  </main>

  <StatusLine
    dir="~/AA/Rift_TerminalV2"
    model="opus-4.7"
    repo="rift-v2"
    git="main · 0↑ · 0M"
  />
</div>

<style>
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    position: relative;
  }

  .surface {
    display: none;
    flex: 1;
    flex-direction: column;
    min-height: 0;
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
</style>
