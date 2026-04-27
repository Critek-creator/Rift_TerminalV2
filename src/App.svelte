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
  import Popout from './lib/Popout.svelte';
  import { popouts } from './lib/popouts.svelte';
  import type { Category } from './lib/bus';

  // Tab id → bus category. `undefined` = no integration registered yet,
  // so the pane stays in placeholder mode until a translator lights it up.
  const CATEGORY_BY_NOTIF: Record<string, Category | undefined> = {
    hooks: 'hook',
    errors: 'system',     // v1: all Category::System envelopes are errors (kind="error" is the only System kind emitted); kind-filter is a future enhancement
    commands: 'pty',      // v1: all Category::Pty envelopes; only kind emitted is "command.submitted"; kind-filter deferred
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

  // ----- promoted notification tab (Phase 3.5a) -----
  // Holds the id of the single promoted tab, or null when no pane is docked.
  // Max-1 promotion is enforced structurally — string|null can hold one id.
  let promoted = $state<string | null>(null);

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
    const enabled = notifs.find((n) => n.id === id)?.enabled;
    if (!enabled) {
      // Disabled tabs shouldn't render anywhere — pull them from main and side.
      if (active.kind === 'notification' && active.id === id) {
        const fallback = sessions.at(0);
        active = fallback ? { kind: 'session', id: fallback.id } : { kind: 'empty' };
      }
      if (promoted === id) {
        promoted = null;
      }
    }
  }

  // Phase 3.5a — promote a notif tab to the right-side pane.
  // Reassigning `promoted` enforces max-1 (a 2nd promote auto-replaces the 1st).
  function promoteTab(id: string) {
    promoted = id;
    // The promoted tab now lives in the side pane, not the main area —
    // if it was the active main surface, recompute active.
    if (active.kind === 'notification' && active.id === id) {
      const fallback = sessions.at(0);
      active = fallback ? { kind: 'session', id: fallback.id } : { kind: 'empty' };
    }
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
</script>

<div class="app-shell">
  <TitleBar />
  <TabBar
    {sessions}
    {notifs}
    {active}
    promotedId={promoted}
    onActivateSession={activateSession}
    onActivateNotif={activateNotif}
    onCloseSession={closeSession}
    onAddSession={addSession}
    onToggleNotif={toggleNotif}
    onPromote={promoteTab}
    onDemote={demoteTab}
  />

  <main class="main" class:split={promoted !== null}>
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
    </div>

    <!-- Phase 3.5a — promoted notification side-pane.
         Independent NotificationPane instance (not driven by `active`).
         Re-keyed on the promoted id so the subscription resets cleanly
         when one promoted tab replaces another. -->
    {#if promotedTab}
      {#key promotedTab.id}
        <aside class="promoted-pane">
          <NotificationPane
            title={promotedTab.title}
            icon={promotedTab.icon}
            accent={notifAccent(promotedTab.id)}
            categoryFilter={CATEGORY_BY_NOTIF[promotedTab.id]}
            onDragBack={demoteTab}
          />
        </aside>
      {/key}
    {/if}
  </main>

  <StatusLine
    dir="~/AA/Rift_TerminalV2"
    model="opus-4.7"
    repo="rift-v2"
    git="main · 0↑ · 0M"
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
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    position: relative;
  }
  /* When a notif tab is promoted, switch to a 2-column layout:
     main-left holds the existing session/notif/empty surfaces (flex 1),
     promoted-pane is a fixed-width 420px aside on the right. */
  .main.split {
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
    border-left: 1px solid var(--border-subtle);
    background: var(--bg-base);
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
