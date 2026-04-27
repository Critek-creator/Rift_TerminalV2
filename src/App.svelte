<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import TitleBar from './lib/TitleBar.svelte';
  import TabBar, {
    type SessionTab,
    type NotifTab,
    type ActiveSurface,
  } from './lib/TabBar.svelte';
  import Terminal from './lib/Terminal.svelte';
  import NotificationPane from './lib/NotificationPane.svelte';
  import AegisTabContent from './lib/AegisTabContent.svelte';
  import IndexTabContent from './lib/IndexTabContent.svelte';
  import StatusLine from './lib/StatusLine.svelte';
  import Popout from './lib/Popout.svelte';
  import { popouts } from './lib/popouts.svelte';
  import Tree from './lib/Tree.svelte';
  import { subscribe, type Category } from './lib/bus';

  // Tab id → bus category. `undefined` = no integration registered yet,
  // so the pane stays in placeholder mode until a translator lights it up.
  const CATEGORY_BY_NOTIF: Record<string, Category | undefined> = {
    hooks: 'hook',
    errors: 'system',     // v1: all Category::System envelopes are errors (kind="error" is the only System kind emitted); kind-filter is a future enhancement
    commands: 'pty',      // v1: all Category::Pty envelopes; only kind emitted is "command.submitted"; kind-filter deferred
    aegis: 'aegis',       // Phase 7.2 — Aegis integration tab (private translator, feature-gated)
    index: 'index',       // Phase 8.2 — Index integration tab (vault-walker source wires in Phase 8.5)
  };

  // ----- session tabs -----
  let nextSessionId = 1;
  let sessions = $state<SessionTab[]>([{ id: 0, title: 'rift' }]);

  // ----- notification tabs (default set per §10.7) -----
  let notifs = $state<NotifTab[]>([
    { id: 'errors',   title: 'errors',   icon: '⚡', enabled: true },
    { id: 'hooks',    title: 'hooks',    icon: '⚓', enabled: true },
    { id: 'commands', title: 'commands', icon: '⌘', enabled: true },
    { id: 'aegis',    title: 'aegis',    icon: '◉', enabled: true },
    { id: 'index',    title: 'index',    icon: '◈', enabled: true },
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

  // Phase 6.2 — cockpit right pane header data.
  // Tree.svelte computes these internally and pushes them up via $bindable props.
  let nodeCount = $state(0);
  let watchedPathLabel = $state('…');

  // Phase 6.4 — cockpit detach state.
  // `cockpitDetached` drives whether the cockpit-right renders the Tree or the
  // placeholder card. Polled once on mount for reload-recovery (design E),
  // then kept current via `cockpit_detached` / `cockpit_reattached` events.
  let cockpitDetached = $state(false);

  // Phase 7.4 — live SKILL segment.
  // Subscribes at App level (not inside AegisTabContent) so the status-line
  // SKILL segment updates regardless of which tab is active.
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: $effect cleanup is SYNC;
  // async unsubscribe wrapped in IIFE.
  let aegisSkillName = $state('');

  let _skillUnsub: (() => Promise<void>) | undefined;

  $effect(() => {
    void (async () => {
      try {
        _skillUnsub = await subscribe({ category: 'aegis' }, (env) => {
          if (env.kind === 'aegis.session.skill_loaded') {
            const p = env.payload as { skill_name?: string; skill_version?: string | null };
            aegisSkillName = p.skill_name ?? '';
          }
        });
      } catch (err) {
        console.warn('[App] skill_loaded subscribe failed:', err);
      }
    })();

    return () => {
      void (async () => {
        await _skillUnsub?.();
      })();
    };
  });

  onMount(() => {
    // Svelte 5's onMount requires a sync callback that optionally returns a
    // cleanup. Async init runs in an IIFE; cleanup captures unlisten handles
    // via mutable refs the IIFE populates.
    let unlistenDetached: (() => void) | undefined;
    let unlistenReattached: (() => void) | undefined;

    void (async () => {
      try {
        cockpitDetached = await invoke<boolean>('cockpit_status');
      } catch (err) {
        console.warn('[App] cockpit_status failed:', err);
      }

      unlistenDetached = await listen('cockpit_detached', () => {
        cockpitDetached = true;
      });
      unlistenReattached = await listen('cockpit_reattached', () => {
        cockpitDetached = false;
      });
    })();

    return () => {
      unlistenDetached?.();
      unlistenReattached?.();
    };
  });

  async function reattachCockpit(): Promise<void> {
    try {
      await invoke('cockpit_reattach');
    } catch (err) {
      console.error('[App] cockpit_reattach failed:', err);
    }
  }
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

  <!-- Phase 6.2 — always-on cockpit: left = terminal surface, right = file tree -->
  <main class="cockpit">
    <!-- Left half: existing terminal / notification / empty surfaces + optional promoted pane -->
    <div class="cockpit-left" class:split={promoted !== null}>
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
             subscription rather than reusing one tied to the previous tab.
             Phase 7.2: aegis tab routes to AegisTabContent; all others
             continue to use the generic NotificationPane. -->
        {#if activeNotifTab}
          {#key activeNotifTab.id}
            <div class="surface visible">
              {#if activeNotifTab.id === 'aegis'}
                <AegisTabContent />
              {:else if activeNotifTab.id === 'index'}
                <IndexTabContent />
              {:else}
                <NotificationPane
                  title={activeNotifTab.title}
                  icon={activeNotifTab.icon}
                  accent={notifAccent(activeNotifTab.id)}
                  categoryFilter={CATEGORY_BY_NOTIF[activeNotifTab.id]}
                />
              {/if}
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
           Independent NotificationPane / AegisTabContent instance (not driven by `active`).
           Re-keyed on the promoted id so the subscription resets cleanly
           when one promoted tab replaces another. -->
      {#if promotedTab}
        {#key promotedTab.id}
          <aside class="promoted-pane">
            {#if promotedTab.id === 'aegis'}
              <AegisTabContent onDragBack={demoteTab} />
            {:else if promotedTab.id === 'index'}
              <IndexTabContent onDragBack={demoteTab} />
            {:else}
              <NotificationPane
                title={promotedTab.title}
                icon={promotedTab.icon}
                accent={notifAccent(promotedTab.id)}
                categoryFilter={CATEGORY_BY_NOTIF[promotedTab.id]}
                onDragBack={demoteTab}
              />
            {/if}
          </aside>
        {/key}
      {/if}
    </div>

    <!-- Right half: filesystem tree (Phase 6.4 — hidden when cockpit is detached) -->
    <div class="cockpit-right">
      {#if !cockpitDetached}
        <div class="pane-header">
          <span>FILE TREE</span>
          <span class="meta">{nodeCount} files · {watchedPathLabel}</span>
        </div>
        <div class="tree-body">
          <Tree bind:nodeCount bind:watchedPathLabel />
        </div>
      {:else}
        <div class="detached-placeholder">
          <div class="detached-card">
            <div class="detached-glyph">↗</div>
            <div class="detached-title">cockpit detached</div>
            <div class="detached-hint">flying on a second display</div>
            <button type="button" class="reattach-btn" onclick={reattachCockpit}>
              ↙ reattach
            </button>
          </div>
        </div>
      {/if}
    </div>
  </main>

  <StatusLine
    dir="~/AA/Rift_TerminalV2"
    model="opus-4.7"
    repo="rift-v2"
    git="main · 0↑ · 0M"
    skill={aegisSkillName || '--'}
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
  /* Phase 6.2 — cockpit outer shell: terminal left, file tree right. */
  .cockpit {
    flex: 1;
    display: flex;
    flex-direction: row;
    min-height: 0;
    background: var(--bg-base);
    position: relative;
    overflow: hidden;
  }

  /* Left half — holds the terminal/notif/empty area + optional promoted pane. */
  .cockpit-left {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-right: 1px solid var(--border-subtle);
  }
  /* When a notif tab is promoted, the left column switches to row layout so
     the promoted pane sits alongside the main terminal area. */
  .cockpit-left.split {
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

  /* Right half — filesystem tree pane. */
  .cockpit-right {
    flex: 0 0 38%;
    min-width: 360px;
    max-width: 520px;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
  }

  /* Pane header — FILE TREE title + meta. Shared vocabulary with NotificationPane. */
  .pane-header {
    height: 24px;
    padding: 0 10px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    flex-shrink: 0;
    user-select: none;
  }
  .pane-header .meta {
    color: var(--amber-faint);
    font-weight: 400;
    font-size: 9px;
    letter-spacing: 0.04em;
  }

  /* Tree body scrolls; Tree.svelte owns the SVG. */
  .tree-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0;
  }
  .tree-body::-webkit-scrollbar { width: 5px; }
  .tree-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

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

  /* Phase 6.4 — cockpit detached placeholder */
  .detached-placeholder {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--amber-faint);
  }

  .detached-card {
    text-align: center;
    user-select: none;
    padding: 32px;
  }

  .detached-glyph {
    font-size: 48px;
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-strong);
    margin-bottom: 16px;
  }

  .detached-title {
    color: var(--amber-warm);
    font-size: 14px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    margin-bottom: 8px;
  }

  .detached-hint {
    color: var(--amber-dim);
    font-size: 11px;
    margin-bottom: 20px;
    font-style: italic;
  }

  .reattach-btn {
    background: transparent;
    border: 1px solid var(--amber-dim);
    color: var(--amber-dim);
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    letter-spacing: 0.1em;
    padding: 4px 12px;
    cursor: pointer;
  }

  .reattach-btn:hover {
    color: var(--amber-primary);
    border-color: var(--amber-primary);
    text-shadow: var(--glow-amber);
  }
</style>
