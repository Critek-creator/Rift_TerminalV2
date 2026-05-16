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

  // Capability detector: flips true the moment the first Index envelope arrives.
  // TODO(8.5): this gate becomes always-true once vault-walker publishes continuously.
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

  let tickTimer: ReturnType<typeof setInterval> | undefined;
  let unsubscribeFn: (() => Promise<void>) | undefined;

  $effect(() => {
    // Async setup inside IIFE — cleanup returned from $effect must be sync.
    void (async () => {
      try {
        unsubscribeFn = await subscribe({ category: 'index' }, handleEnvelope);
      } catch (err) {
        console.error('[IndexTabContent] bus subscribe failed', err);
      }
    })();

    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
    }, 1000);

    // Sync cleanup.
    return () => {
      if (tickTimer) clearInterval(tickTimer);
      // Async teardown in IIFE (pr003 svelte5-async-cleanup-via-sync-shell-iife).
      void (async () => {
        await unsubscribeFn?.();
      })();
    };
  });

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
</script>

<section class="pane" data-accent="cyan">
  {#if onDragBack}
    <div
      class="drag-handle"
      role="button"
      tabindex="0"
      draggable={true}
      ondragstart={onHandleDragStart}
      title="drag back to tab strip to dock"
    >
      <span class="handle-glyph">↙</span>
      <span class="handle-title">INDEX</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if !hasIndexData}
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
        aria-label="Filter index events"
      />
      <button
        class="vault-root-btn"
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
      <div class="log-body">
        {#if recentEntries.length === 0}
          <div class="empty">subscribed to <span class="cat">index</span> — no events received yet</div>
        {:else if filteredRecentEntries.length === 0}
          <div class="empty">no events matching "<span class="cat">{searchTerm.trim()}</span>"</div>
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
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    color: var(--term-cyan, #4ad4d4);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    --accent: var(--term-cyan, #4ad4d4);
  }

  /* Phase 3.5a drag handle */
  .drag-handle {
    height: 26px;
    padding: 0 12px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: 10px;
    letter-spacing: 0.1em;
    font-weight: 700;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--accent);
    font-size: 12px;
  }
  .drag-handle .handle-title {
    color: var(--accent);
    text-transform: uppercase;
  }
  .drag-handle .handle-hint {
    margin-left: auto;
    color: var(--amber-faint);
    font-style: italic;
    font-weight: 400;
    letter-spacing: 0.04em;
  }

  /* Capability-driven empty state (§10.7) */
  .empty-state-wrap {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .index-card {
    padding: 16px 18px;
    border: 1px dashed rgba(74, 212, 212, 0.3);
    background: var(--bg-panel, rgba(0, 0, 0, 0.3));
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 320px;
    opacity: 0.75;
  }
  .index-card-heading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--accent);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  .index-card-icon {
    font-size: 13px;
    opacity: 0.85;
  }
  .index-card-status {
    color: var(--amber-faint, #5a4410);
    font-size: 10px;
    font-style: italic;
    letter-spacing: 0.04em;
  }
  .index-card-subtitle {
    color: var(--amber-faint, #5a4410);
    font-size: 9px;
    font-weight: 400;
    letter-spacing: 0.03em;
    line-height: 1.5;
    opacity: 0.85;
  }

  /* Section 1: Status header */
  .status {
    height: 30px;
    padding: 0 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    display: flex;
    align-items: center;
    gap: 14px;
    color: var(--accent);
    font-size: 11px;
    letter-spacing: 0.1em;
    font-weight: 700;
    flex-shrink: 0;
  }
  .status .title {
    color: var(--accent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .spacer { flex: 1; }
  .status .state {
    color: var(--amber-dim);
    font-weight: 400;
    letter-spacing: 0.04em;
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Section 2: Live activity strip */
  .strip {
    height: 26px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
    display: flex;
    align-items: center;
    gap: 14px;
    background: linear-gradient(to bottom, rgba(74, 212, 212, 0.04), transparent);
    color: var(--amber-dim);
    font-size: 10px;
    letter-spacing: 0.1em;
    overflow: hidden;
    flex-shrink: 0;
  }
  .strip-label { color: var(--accent); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events { display: flex; gap: 6px; flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px 6px;
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(74, 212, 212, 0.04);
    flex-shrink: 0;
  }

  /* Section 3: Recent events log */
  .log {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-bottom: 1px solid var(--border-subtle);
  }
  .log-header {
    padding: 6px 14px;
    color: var(--amber-warm);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    flex-shrink: 0;
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 14px;
    color: var(--amber-warm);
    font-size: 11px;
    box-shadow: var(--depth-inset);
    line-height: 1.5;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }
  .empty {
    color: var(--amber-faint);
    font-style: italic;
  }
  .empty .cat {
    color: var(--accent);
    font-style: normal;
    font-weight: 600;
  }

  /* Section 4: Persistent state panel */
  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 220px;
    overflow-y: auto;
    border-top: 1px solid var(--border-subtle);
    box-shadow: var(--depth-lift), var(--depth-edge-light);
  }
  .state-header {
    padding: 6px 14px;
    color: var(--amber-warm);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    box-shadow: var(--depth-edge-light);
    border-bottom: 1px solid var(--border-subtle);
  }
  .state-body {
    padding: 8px 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row.k-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 10px;
    letter-spacing: 0.04em;
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }

  /* Vault-kind breakdown tags */
  .vault-kinds {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border-subtle);
  }
  .vk-label {
    display: block;
    color: var(--amber-warm);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    margin-bottom: 6px;
  }
  .vk-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .vk-tag {
    padding: 1px 6px;
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.06em;
    background: rgba(74, 212, 212, 0.06);
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .vk-count {
    color: var(--amber-warm);
    font-weight: 700;
  }

  /* §10.14: search/filter bar + vault-root quick link */
  .search-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 14px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    flex-shrink: 0;
  }
  .search-input {
    flex: 1;
    background: var(--bg-primary, var(--bg-base));
    border: 1px solid var(--amber-faint);
    color: var(--term-white);
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    padding: 3px 8px;
    letter-spacing: 0.04em;
    outline: none;
  }
  .search-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .search-input:focus {
    border-color: var(--accent);
  }
  .vault-root-btn {
    padding: 2px 8px;
    border: 1px solid var(--accent);
    color: var(--accent);
    background: rgba(74, 212, 212, 0.06);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.06em;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.1s;
  }
  .vault-root-btn:hover {
    background: rgba(74, 212, 212, 0.14);
  }
  .vault-root-btn:active {
    background: rgba(74, 212, 212, 0.22);
  }
  .vault-root-error {
    padding: 2px 14px;
    color: var(--term-red);
    font-size: 9px;
    font-style: italic;
    letter-spacing: 0.04em;
    word-break: break-all;
  }
</style>
