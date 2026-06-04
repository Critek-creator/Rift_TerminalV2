<script lang="ts">
  // Index integration tab — §10.8 four-section notification anatomy.
  //
  // Data sources (Phase 8.1 index.rs translator):
  //   vault.update  — vault file created/modified/deleted; payload: vault_id, path, change_kind
  //   enrichment    — filesystem path receives vault metadata; payload: fs_path, vault_id, vault_kind, tags
  //
  // Capability-driven empty state (§10.7, Phase 8.2):
  //   When no Index source has published any envelope yet (true for all installs
  //   until Phase 8.5 wires the vault-walker source), the tab renders the
  //   "Index: integration not loaded" card. The moment the first Index envelope
  //   arrives, `hasIndexData` flips and the 4-section live layout takes over.
  //
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: the cleanup returned from
  // $effect must be SYNC. Async teardown (bus unsubscribe) is wrapped in IIFE.

  import { invoke } from '@tauri-apps/api/core';
  import { subscribe, type Envelope } from './bus';
  import IndexTabRenderer from './IndexTabRenderer.svelte';
  import { NOTIF_TAB_MIME } from './dragMime';

  interface Props {
    /** Drag-back handle for promoted-pane mode (Phase 3.5a). */
    onDragBack?: () => void;
  }

  let { onDragBack }: Props = $props();

  const RECENT_LOG_LIMIT = 100;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let entries = $state<Envelope[]>([]);
  let lastTickTs = $state<number>(Date.now());

  // §10.14: search/filter bar — filters recent events by kind or payload text.
  let searchTerm = $state('');

  // Vault-root quick-link error state (mirrors AegisTabContent quick-action pattern).
  let vaultRootError = $state<string | null>(null);
  let vaultRootTimer: ReturnType<typeof setTimeout> | undefined;

  // ---------------------------------------------------------------------------
  // Derived views (§10.8 sections)
  // ---------------------------------------------------------------------------

  const hasIndexData = $derived(entries.length > 0);

  // Section 2 — live activity strip: events within the trailing 4-second window.
  const liveEntries = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_ACTIVITY_WINDOW_MS;
    return entries.filter((e) => e.ts >= cutoff);
  });

  // Section 3 — recent events log: last N, newest first.
  const recentEntries = $derived(entries.slice(-RECENT_LOG_LIMIT).reverse());

  // Counts for status header and persistent panel.
  const totalCount = $derived(entries.length);
  const vaultUpdateCount = $derived(entries.filter((e) => e.kind === 'vault.update').length);
  const enrichmentCount = $derived(entries.filter((e) => e.kind === 'enrichment').length);

  // Vault-kind category summary for Section 4 persistent state panel.
  // Groups enrichment envelopes by vault_kind ("p", "pr", "r", "s", "lore", "agt", "h").
  const vaultKindCounts = $derived.by(() => {
    const counts: Record<string, number> = {};
    for (const e of entries) {
      if (e.kind === 'enrichment') {
        const p = e.payload as Record<string, unknown>;
        const vk = typeof p.vault_kind === 'string' ? p.vault_kind : 'unknown';
        counts[vk] = (counts[vk] ?? 0) + 1;
      }
    }
    return counts;
  });

  const lastSeenLabel = $derived.by(() => {
    if (entries.length === 0) return '—';
    const last = entries[entries.length - 1];
    const ageMs = Math.max(0, lastTickTs - last.ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  });

  // Section 3 — search-filtered recent events (§10.14 search bar).
  const filteredRecentEntries = $derived.by(() => {
    const term = searchTerm.trim().toLowerCase();
    if (!term) return recentEntries;
    return recentEntries.filter((e) => {
      if (e.kind.toLowerCase().includes(term)) return true;
      if (e.payload) {
        const p = e.payload as Record<string, unknown>;
        for (const v of Object.values(p)) {
          if (typeof v === 'string' && v.toLowerCase().includes(term)) return true;
        }
      }
      return false;
    });
  });

  // Section 1 — status header label.
  const statusLabel = $derived.by(() => {
    if (!hasIndexData) return 'Index · waiting for vault source…';
    return `Index · ${vaultUpdateCount} vault update${vaultUpdateCount === 1 ? '' : 's'} · ${enrichmentCount} enrichment${enrichmentCount === 1 ? '' : 's'}`;
  });

  // ---------------------------------------------------------------------------
  // Envelope handler
  // ---------------------------------------------------------------------------

  function handleEnvelope(env: Envelope) {
    entries = [...entries, env];
    if (entries.length > RECENT_LOG_LIMIT * 2) {
      entries = entries.slice(-RECENT_LOG_LIMIT);
    }
    lastTickTs = Date.now();
  }

  // ---------------------------------------------------------------------------
  // Bus subscription + tick timer (Svelte 5 runes)
  // pr003 svelte5-async-cleanup-via-sync-shell-iife
  // ---------------------------------------------------------------------------

  let connected = $state(false);
  let connectError = $state<string | null>(null);
  let tickTimer: ReturnType<typeof setInterval> | undefined;
  let unsubscribeFn: (() => Promise<void>) | undefined;

  $effect(() => {
    // Mount-race guard (NotificationPane reference pattern): if the component
    // unmounts before `subscribe()` resolves, the sync cleanup below runs
    // while `unsubscribeFn` is still undefined — the in-flight handle would
    // leak. We flip `cancelled` in cleanup and unsubscribe on resolution.
    let cancelled = false;
    // Async setup inside IIFE — cleanup returned from $effect must be sync.
    void (async () => {
      try {
        const u = await subscribe({ category: 'index' }, handleEnvelope);
        if (cancelled) {
          void u().catch(() => {});
          return;
        }
        unsubscribeFn = u;
        connected = true;
      } catch (err) {
        console.error('[IndexTabContent] bus subscribe failed', err);
        connectError = err instanceof Error ? err.message : String(err);
      }
    })();

    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
    }, 1000);

    // Sync cleanup.
    return () => {
      cancelled = true;
      if (tickTimer) clearInterval(tickTimer);
      // Async teardown in IIFE (pr003 svelte5-async-cleanup-via-sync-shell-iife).
      void (async () => {
        await unsubscribeFn?.();
      })();
    };
  });

  // ---------------------------------------------------------------------------
  // ---------------------------------------------------------------------------
  // §10.14: '/' shortcut — focus search input when Index pane is active and
  // focus is NOT already inside a text input/textarea.
  // ---------------------------------------------------------------------------

  let searchInputEl = $state<HTMLInputElement | null>(null);

  function onPaneKeydown(e: KeyboardEvent) {
    if (e.key !== '/') return;
    const tag = (e.target as HTMLElement | null)?.tagName?.toLowerCase();
    if (tag === 'input' || tag === 'textarea') return;
    if (!searchInputEl) return;
    e.preventDefault();
    searchInputEl.focus();
  }

  // ---------------------------------------------------------------------------
  // §10.14: vault-root quick link — open ~/.claude/abyssal-index/ in OS
  // file manager. Uses the same pattern as AegisTabContent quick-actions.
  // ---------------------------------------------------------------------------

  function clearVaultRootErrorAfterDelay() {
    if (vaultRootTimer) clearTimeout(vaultRootTimer);
    vaultRootTimer = setTimeout(() => {
      vaultRootError = null;
      vaultRootTimer = undefined;
    }, 3000);
  }

  function openVaultRoot() {
    void (async () => {
      try {
        await invoke('index_open_vault_root');
      } catch (err) {
        vaultRootError = err instanceof Error ? err.message : String(err);
        clearVaultRootErrorAfterDelay();
      }
    })();
  }

  // ---------------------------------------------------------------------------
  // Drag-handle (Phase 3.5a promoted-pane)
  // ---------------------------------------------------------------------------

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Marker MIME — TabBar.onStripDrop filters by NOTIF_TAB_MIME presence
      // and rejects drags missing it (silent demote failure if omitted).
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }

  // Keyboard equivalent of the drag-to-dock handle (matches AegisTabContent et al.).
  function onHandleDragKeydown(e: KeyboardEvent) {
    if ((e.key === 'Enter' || e.key === ' ') && onDragBack) {
      e.preventDefault();
      onDragBack();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- Pane-level shortcut scope: `/` focuses the Index search input (see
     onPaneKeydown). Not an interactive widget — it's a keyboard-shortcut host. -->
<section class="pane" data-accent="cyan" onkeydown={onPaneKeydown}>
  {#if onDragBack}
    <div
      class="drag-handle"
      role="button"
      tabindex="0"
      draggable={true}
      ondragstart={onHandleDragStart}
      onkeydown={onHandleDragKeydown}
      title="drag back to tab strip to dock"
      aria-label="Index pane — drag or press Enter to dock"
    >
      <span class="handle-glyph" style="color: var(--amber-warm); font-size: 14px">⬢</span>
      <span class="handle-title">INDEX</span>
      <button
        type="button"
        class="dock-btn"
        draggable={false}
        onclick={(e) => { e.stopPropagation(); onDragBack?.(); }}
        title="Return to tab strip"
        aria-label="Dock pane back to tab strip"
      >↩ dock</button>
    </div>
  {/if}

  {#if connectError}
    <div class="connect-error">{connectError}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else if !hasIndexData}
    <!-- Capability-driven empty state (§10.7, Phase 8.2).
         Renders until Phase 8.5 vault-walker publishes the first Index envelope. -->
    <div class="empty-state-wrap">
      <div class="index-card">
        <div class="index-card-heading">
          <span class="index-card-icon">◈</span>
          INDEX
        </div>
        <div class="index-card-status">integration not loaded</div>
        <div class="index-card-subtitle">
          Vault network will appear when the Index source begins publishing.
          The vault-walker source wires automatically in Phase 8.5.
        </div>
      </div>
    </div>
  {:else}
    <!-- 4-section live-data layout (§10.8).
         Populated the moment the first Index envelope arrives. -->

    <!-- Section 1: Status header -->
    <header class="status">
      <span class="title"><span class="icon">◈</span>{statusLabel}</span>
      <span class="spacer"></span>
      <span class="state">
        {totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
      </span>
    </header>

    <!-- §10.14: search/filter bar + vault-root quick link -->
    <div class="search-bar">
      <input
        class="search-input"
        type="text"
        placeholder="filter events…"
        bind:value={searchTerm}
        bind:this={searchInputEl}
        aria-label="Filter index events"
      />
      <button type="button"
        class="rift-btn rift-btn--sm vault-root-btn"
        onclick={openVaultRoot}
        title="Open vault root (~/.claude/abyssal-index/) in file manager"
      >VAULT ROOT ↗</button>
    </div>
    {#if vaultRootError}
      <div class="vault-root-error">{vaultRootError}</div>
    {/if}

    <!-- Section 2: Live activity strip -->
    <div class="strip">
      <span class="strip-label">LIVE</span>
      {#if liveEntries.length === 0}
        <span class="strip-empty">(no in-flight events)</span>
      {:else}
        <div class="strip-events">
          {#each liveEntries as e, i (e.ts + ':' + e.kind + ':' + i)}
            <span class="strip-event">{e.kind}</span>
          {/each}
        </div>
      {/if}
    </div>

    <!-- Section 3: Recent events log (filtered by search bar) -->
    <div class="log">
      <div class="log-header">RECENT EVENTS{searchTerm.trim() ? ` (${filteredRecentEntries.length}/${recentEntries.length})` : ''}</div>
      <div class="log-body" aria-live="polite">
        {#if recentEntries.length === 0}
          <div class="empty-state">
            <span class="empty-state-icon">⬡</span>
            <span class="empty-state-text">Index is synced</span>
            <span class="empty-state-hint">vault activity events will stream in as files change</span>
          </div>
        {:else if filteredRecentEntries.length === 0}
          <div class="empty-state">
            <span class="empty-state-icon">◇</span>
            <span class="empty-state-text">No matches for "{searchTerm.trim()}"</span>
            <span class="empty-state-hint">try a broader search or check spelling</span>
          </div>
        {:else}
          {#each filteredRecentEntries as e, i (e.ts + ':' + e.kind + ':' + i)}
            <IndexTabRenderer
              entry={{
                ts: e.ts,
                category: e.category as string,
                kind: e.kind,
                payload: (e.payload as Record<string, unknown>) ?? {},
              }}
            />
          {/each}
        {/if}
      </div>
    </div>

    <!-- Section 4: Persistent state panel -->
    <footer class="state-panel">
      <div class="state-header">PERSISTENT STATE</div>
      <div class="state-body">
        <div class="row k-row">
          <span class="k">total events</span>
          <span class="v">{totalCount}</span>
        </div>
        <div class="row k-row">
          <span class="k">vault updates</span>
          <span class="v">{vaultUpdateCount}</span>
        </div>
        <div class="row k-row">
          <span class="k">enrichments</span>
          <span class="v">{enrichmentCount}</span>
        </div>
        <div class="row k-row">
          <span class="k">last seen</span>
          <span class="v">{lastSeenLabel}</span>
        </div>

        <!-- Vault-kind breakdown — enrichment events grouped by vault_kind -->
        {#if Object.keys(vaultKindCounts).length > 0}
          <div class="vault-kinds">
            <span class="vk-label">VAULT CATEGORIES</span>
            <div class="vk-tags">
              {#each Object.entries(vaultKindCounts).sort(([, a], [, b]) => b - a) as [kind, count] (kind)}
                <span class="vk-tag" title={kind}>{kind} <span class="vk-count">{count}</span></span>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    </footer>
  {/if}
</section>

<style>
  .connecting-state {
    color: var(--amber-faint);
    padding: var(--space-lg) var(--space-14);
    font-style: italic;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
  }
  .connect-error {
    color: var(--term-red);
    padding: var(--space-8) var(--space-14);
    font-style: italic;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
    opacity: 0.9;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    color: var(--term-cyan, #6FE0E0);
    font-family: var(--font-family);
    font-size: var(--text-base);
    --accent: var(--term-cyan, #6FE0E0);
  }

  /* Phase 3.5a drag handle */
  .drag-handle {
    height: var(--control-sm);
    padding: 0 var(--space-12);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    gap: var(--space-md);
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: var(--type-label-size);
    letter-spacing: var(--type-label-spacing);
    font-weight: var(--type-label-weight);
    transition: background var(--duration-base) ease-out;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -2px; }
  .drag-handle .handle-glyph {
    color: var(--accent);
    font-size: var(--text-base);
  }
  .drag-handle .handle-title {
    color: var(--accent);
    text-transform: uppercase;
  }

  /* Capability-driven empty state (§10.7) */
  .empty-state-wrap {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-24);
  }
  .index-card {
    padding: var(--space-2xl) var(--space-lg);
    background: transparent;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-8);
    max-width: 320px;
    text-align: center;
  }
  .index-card-heading {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    color: var(--accent);
    font-size: var(--type-body-size);
    font-weight: var(--type-body-weight);
    letter-spacing: var(--type-body-spacing);
  }
  .index-card-icon {
    font-size: var(--text-md);
    opacity: 0.85;
  }
  .index-card-status {
    color: var(--amber-dim);
    font-size: var(--type-caption-size);
    font-style: italic;
    letter-spacing: var(--type-caption-spacing);
  }
  .index-card-subtitle {
    color: var(--amber-faint);
    font-size: var(--type-caption-size);
    font-weight: 400;
    letter-spacing: var(--type-caption-spacing);
    line-height: 1.5;
  }

  /* Section 1: Status header */
  .status {
    height: 36px;
    padding: 0 var(--space-lg);
    background: var(--bg-elevated);
    box-shadow: var(--sep-glow);
    display: flex;
    align-items: center;
    gap: var(--space-14);
    color: var(--accent);
    flex-shrink: 0;
  }
  .status .title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--accent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; font-size: var(--text-lg); }
  .status .spacer { flex: 1; }
  .status .state {
    font-size: var(--type-caption-size);
    font-weight: var(--type-caption-weight);
    letter-spacing: var(--type-caption-spacing);
    color: var(--amber-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Section 2: Live activity strip */
  .strip {
    height: var(--control-sm);
    padding: 0 var(--space-14);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    gap: var(--space-14);
    background: linear-gradient(to bottom, rgba(111, 224, 224, 0.06), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
    flex-shrink: 0;
  }
  .strip-label { color: var(--accent); font-weight: 700; }
  .strip-empty { color: var(--amber-dim); font-size: var(--type-caption-size); font-style: italic; letter-spacing: var(--type-caption-spacing); }
  .strip-events { display: flex; gap: var(--space-sm); flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px var(--space-sm);
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(111, 224, 224, 0.06);
    flex-shrink: 0;
  }

  /* Section 3: Recent events log */
  .log {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .log-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    box-shadow: var(--sep-depth);
    background: var(--bg-surface);
    flex-shrink: 0;
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-md) var(--space-lg);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    box-shadow: var(--depth-inset);
    line-height: 1.5;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* Section 4: Persistent state panel */
  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 220px;
    overflow-y: auto;
    box-shadow: 0 -2px 6px rgba(0, 0, 0, 0.45);
  }
  .state-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    box-shadow: var(--sep-depth);
  }
  .state-body {
    padding: var(--space-md) var(--space-lg) var(--space-14);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .row.k-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }

  /* Vault-kind breakdown tags */
  .vault-kinds {
    margin-top: var(--space-8);
    padding-top: var(--space-8);
  }
  .vk-label {
    display: block;
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    margin-bottom: var(--space-sm);
  }
  .vk-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .vk-tag {
    padding: 1px var(--space-sm);
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    background: rgba(111, 224, 224, 0.06);
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .vk-count {
    color: var(--amber-warm);
    font-weight: 700;
  }

  /* §10.14: search/filter bar + vault-root quick link */
  .search-bar {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-14);
    box-shadow: var(--sep-depth);
    background: var(--bg-surface);
    flex-shrink: 0;
  }
  .search-input {
    flex: 1;
    background: var(--bg-base);
    border: 1px solid var(--amber-faint);
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    padding: 3px var(--space-8);
    letter-spacing: 0.04em;
    outline: 2px solid transparent;
    transition: border-color var(--duration-base) ease-out;
  }
  .search-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .search-input:focus {
    border-color: var(--accent);
  }
  .vault-root-btn {
    border-color: var(--accent);
    color: var(--accent);
    background: rgba(111, 224, 224, 0.06);
    white-space: nowrap;
  }
  .vault-root-btn:hover {
    background: rgba(111, 224, 224, 0.14);
    border-color: var(--accent);
    color: var(--accent);
  }
  .vault-root-error {
    padding: 2px var(--space-14);
    color: var(--term-red);
    font-size: var(--text-2xs);
    font-style: italic;
    letter-spacing: 0.04em;
    word-break: break-all;
  }
</style>
