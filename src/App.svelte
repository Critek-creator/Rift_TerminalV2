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
  import IndexGraph from './lib/IndexGraph.svelte';
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
  // Capability-gating (§10.7): base set (errors / hooks / commands) is
  // always present; integration tabs (aegis, index, …) appear only after
  // the integration declares itself via an envelope on its category.
  // - `aegis` → flips on `aegis.detected` from rift-aegis startup probe (Phase 7.1).
  // - `index` → CURRENTLY initialized detected=true because the Index translator
  //   is wired (Phase 8.1) but doesn't emit a startup-ready envelope yet.
  //   TODO(8.5): tighten to envelope-driven once vault-walker publishes — then
  //   change this back to `detected: false` and let the master subscription flip it.
  // unreadCount + lastActivityTs drive the §10.9 badge counter and live border.
  let notifs = $state<NotifTab[]>([
    { id: 'errors',   title: 'errors',   icon: '⚡', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    { id: 'hooks',    title: 'hooks',    icon: '⚓', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    { id: 'commands', title: 'commands', icon: '⌘', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
    { id: 'aegis',    title: 'aegis',    icon: '◉', enabled: true, detected: false, unreadCount: 0, lastActivityTs: null },
    { id: 'index',    title: 'index',    icon: '◈', enabled: true, detected: true,  unreadCount: 0, lastActivityTs: null },
  ]);

  // Reverse map for envelope routing — Category → notif tab id. Used by the
  // master subscription below to attribute incoming envelopes to the right
  // tab. The forward map (notif id → Category) is in CATEGORY_BY_NOTIF.
  const NOTIF_BY_CATEGORY: Record<string, string> = {
    hook: 'hooks',
    system: 'errors',
    pty: 'commands',
    aegis: 'aegis',
    index: 'index',
  };

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
    // Per user spec correction (drift from earlier Phase 3.5a implementation):
    // notif tabs NEVER replace the terminal/session surface in the main slot.
    // Click toggles promotion alongside the main session surface — the notif's
    // content lives in `aside.promoted-pane` next to the terminal, never in
    // place of it.
    //
    // To detach a notif tab into a SEPARATE webview window (drag-out for
    // multi-monitor observation, paralleling Phase 6.4 cockpit_detach), is
    // a v1.x feature ask — not yet wired. For v1, click is the only path to
    // attach/detach a notif side-pane; the side pane's drag-back handle (or
    // another click on the same tab) is the path to demote.
    //
    // The `active` state machine never becomes `notification` — notif tabs
    // promote to a side pane via `promoted`, never replace the main surface.
    // If a future "notif fullscreen" gesture is needed, re-introduce a
    // dedicated state-machine variant with explicit semantics; do NOT revive
    // the previous unreachable branches that this commit removed.
    const wasPromoted = promoted === id;
    promoted = wasPromoted ? null : id;
    // §10.9 ack — promoting a tab into the side pane counts as viewing it.
    if (!wasPromoted) ackUnread(id);
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
    if (!enabled && promoted === id) {
      promoted = null;
    }
  }

  // §10.9 — viewing or promoting a notif tab acknowledges its badge.
  // Called from activateNotif (above) and promoteTab (below).
  function ackUnread(id: string) {
    notifs = notifs.map((n) => (n.id === id && n.unreadCount > 0 ? { ...n, unreadCount: 0 } : n));
  }

  // Phase 3.5a — promote a notif tab to the right-side pane.
  // Reassigning `promoted` enforces max-1 (a 2nd promote auto-replaces the 1st).
  // `active` is never `notification` (notif tabs only ever live in `promoted`),
  // so the prior fallback-recompute branch was unreachable and has been removed.
  function promoteTab(id: string) {
    promoted = id;
    ackUnread(id); // §10.9 — promotion = view, clear the badge
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

  // §10.9 live-border tick — drives TabBar.svelte's `isLive` derived. The
  // 1-second cadence is fast enough to feel responsive without burning CPU;
  // the live window itself is 3s in TabBar.
  let tickNow = $state(Date.now());

  $effect(() => {
    const t = setInterval(() => { tickNow = Date.now(); }, 1000);
    return () => clearInterval(t);
  });

  // §10.7 capability-gate + §10.9 badge counter + live border —
  // master envelope subscription lives at App level so it works regardless
  // of which tab is currently mounted (panes only mount when active).
  // Per-envelope effect: flip detected=true, bump unreadCount (unless tab
  // is currently in view), update lastActivityTs.
  // pr003 svelte5-async-cleanup-via-sync-shell-iife pattern + cancelled-flag
  // mount-race guard match the SKILL subscription below.
  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({}, (env) => {
          const id = NOTIF_BY_CATEGORY[env.category];
          if (!id) return;
          // Pty has many kinds in the long run; today only "command.submitted"
          // is published, but kind-filter here makes the policy explicit so
          // future Category::Pty kinds (e.g. resize, exit) don't accidentally
          // count toward the Commands tab badge.
          if (env.category === 'pty' && env.kind !== 'command.submitted') return;
          const now = Date.now();
          notifs = notifs.map((n) => {
            if (n.id !== id) return n;
            const inView = promoted === id;
            return {
              ...n,
              detected: true,
              lastActivityTs: now,
              unreadCount: inView ? 0 : n.unreadCount + 1,
            };
          });
        });
        if (cancelled) {
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] master notif subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // Phase 7.4 — live SKILL segment.
  // Subscribes at App level (not inside AegisTabContent) so the status-line
  // SKILL segment updates regardless of which tab is active.
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: $effect cleanup is SYNC;
  // async unsubscribe wrapped in IIFE. Mount-race guarded via `cancelled`
  // flag — if cleanup fires before subscribe resolves, the resolved
  // unsubscribe is invoked immediately so the subscription doesn't leak.
  let aegisSkillName = $state('');

  $effect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    void (async () => {
      try {
        const u = await subscribe({ category: 'aegis' }, (env) => {
          if (env.kind === 'aegis.session.skill_loaded') {
            const p = env.payload as { skill_name?: string; skill_version?: string | null };
            aegisSkillName = p.skill_name ?? '';
          }
        });
        if (cancelled) {
          // Cleanup already ran while subscribe was in flight — clean up now.
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[App] skill_loaded subscribe failed:', err);
      }
    })();

    return () => {
      cancelled = true;
      void (async () => {
        await unsub?.();
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
    {tickNow}
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

    <!-- Right half: IndexGraph (top 55%) + File Tree (bottom 45%) (Phase 8.4 — hidden when cockpit is detached) -->
    <div class="cockpit-right">
      {#if !cockpitDetached}
        <!-- Phase 8.4 — Graph pane (top slot). IndexGraph.svelte renders width:100%/height:100%
             inside .graph-pane; border-bottom is the horizontal divider per mockup #3. -->
        <div class="graph-pane">
          <div class="pane-header">
            <span>INDEX</span>
            <span class="meta">vault graph · fixture</span>
          </div>
          <div class="graph-body">
            <!-- TODO(8.5): wire IndexGraph to subscribe(Category::Index) for live vault-walker data -->
            <IndexGraph />
          </div>
        </div>

        <!-- Horizontal divider — 1px solid border-subtle, no resize affordance in v1 (deferred per p006) -->
        <div class="cockpit-h-divider"></div>

        <!-- Tree pane (bottom slot) — all existing Tree props preserved unchanged -->
        <div class="tree-pane">
          <div class="pane-header">
            <span>FILE TREE</span>
            <span class="meta">{nodeCount} files · {watchedPathLabel}</span>
          </div>
          <div class="tree-body">
            <Tree bind:nodeCount bind:watchedPathLabel />
          </div>
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

  /* Right half — graph + tree stacked column (Phase 8.4). */
  .cockpit-right {
    flex: 0 0 38%;
    min-width: 360px;
    max-width: 520px;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
  }

  /* Phase 8.4 — Graph pane (top slot, 55% of cockpit-right height).
     flex: 0 0 55% matches mockup #3 .gui-graph canonical sizing. */
  .graph-pane {
    flex: 0 0 55%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-base);
  }

  /* Graph body — fills the remaining height below graph pane-header.
     IndexGraph SVG renders at width:100%/height:100% inside this container. */
  .graph-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  /* Phase 8.4 — Horizontal divider between graph and tree panes.
     1px solid, no resize affordance in v1 (resize handle deferred per p006). */
  .cockpit-h-divider {
    flex: 0 0 1px;
    background: var(--border-subtle);
    width: 100%;
  }

  /* Tree pane (bottom slot, 45% of cockpit-right height).
     flex: 1 1 45% matches mockup #3 .gui-tree canonical sizing. */
  .tree-pane {
    flex: 1 1 45%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-panel);
  }

  /* Pane header — FILE TREE / INDEX title + meta. Shared vocabulary with NotificationPane. */
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
