<script lang="ts">
  // IndexGraph.svelte — Vault Observatory (list-based vault browser)
  //
  // Structured grouped list of Abyssal Index vaults with stats strip,
  // horizontal recent cards, kind-colored category headers, 2-line vault
  // rows, and visual connection chips in the detail panel.
  //
  // Data flow:
  //   vault_walker (Rust) → Category::Index/vault.update → liveNodeMap
  //   vault_walker (Rust) → Category::Index/walk.complete → walkComplete flag
  //
  // Drag-to-terminal preserved: mousedown on vault row → ghost → drop on terminal.

  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { subscribe } from './bus';
  import { RIFT_VAULT_DROP_EVENT, type RiftVaultDropDetail } from './dragMime';
  import { crossRefHighlight } from './crossRefHighlight.svelte';
  import { enrichmentStore } from './enrichmentStore.svelte';
  import IndexContentBrowser from './IndexContentBrowser.svelte';

  type ViewMode = 'vaults' | 'content';
  let viewMode = $state<ViewMode>('vaults');

  type VaultKind = 'p' | 'pr' | 'r' | 's' | 'lore' | 'agt' | 'h';
  type NodeState = 'active' | 'recent' | 'ambient' | 'background';

  const RECENT_WINDOW_MS = 60 * 60 * 1000;

  const KIND_GLYPH: Record<VaultKind, string> = {
    p: '◐', pr: '§', r: '✦', s: '⚙', lore: '✧', agt: '⚝', h: '⏱',
  };

  const KIND_LABEL: Record<VaultKind, string> = {
    p: 'PROJECTS', pr: 'PRACTICES', r: 'RESEARCH',
    s: 'SKILLS', lore: 'LORE', agt: 'AGENTS', h: 'HISTORY',
  };

  const CATEGORY_ORDER: VaultKind[] = ['p', 'r', 'pr', 's', 'lore', 'agt', 'h'];

  interface VaultNode {
    id: string;
    kind: VaultKind;
    label: string;
    shortLabel?: string;
    displayName?: string;
    updatedMs?: number;
    path?: string;
    crossRefs?: string[];
    parentId?: string | null;
  }

  interface VaultLink {
    source: string;
    target: string;
    parent?: boolean;
  }

  interface VaultUpdatePayload {
    vault_id: string;
    parent_vault_id?: string | null;
    path: string;
    change_kind: 'created' | 'modified' | 'deleted';
    name?: string;
    short_label?: string | null;
    updated_ms?: number | null;
    cross_refs?: string[];
  }

  // ---------------------------------------------------------------------------
  // Static fixture fallback
  // ---------------------------------------------------------------------------

  const STATIC_NODES: VaultNode[] = [
    { id: 'p001',  kind: 'p',  label: 'p001 — Abyssal Arts Main' },
    { id: 'p002',  kind: 'p',  label: 'p002 — AIDE (Abyssal IDE)' },
    { id: 'p003',  kind: 'p',  label: 'p003 — AIDE v2' },
    { id: 'p004',  kind: 'p',  label: 'p004 — Abyssal Indexer' },
    { id: 'p005',  kind: 'p',  label: 'p005 — Abyssal Insight' },
    { id: 'p006',  kind: 'p',  label: 'p006 — Rift Terminal V2' },
    { id: 'pr001', kind: 'pr', label: 'pr001 — Global Practices' },
    { id: 'pr003', kind: 'pr', label: 'pr003 — Lessons/Gotchas' },
    { id: 'pr004', kind: 'pr', label: 'pr004 — Agent Protocols' },
    { id: 'r004',  kind: 'r',  label: 'r004 — Tauri+Svelte Research' },
  ];

  // ---------------------------------------------------------------------------
  // Live data state
  // ---------------------------------------------------------------------------

  let liveNodeMap = $state<Map<string, VaultNode>>(new Map());
  let walkComplete = $state(false);

  const activeNodes = $derived.by<VaultNode[]>(() => {
    if (liveNodeMap.size > 0) {
      return Array.from(liveNodeMap.values()).filter(
        (n) => !n.id.includes('archive') && !(n.path && n.path.includes('/archive/'))
      );
    }
    if (walkComplete) return STATIC_NODES.map((n) => ({ ...n }));
    return [];
  });

  const KNOWN_EDGES: [string, string][] = [
    ['p001', 'r001'], ['p002', 'r001'], ['p003', 'r004'],
    ['p006', 'r006'], ['p004', 'r003'], ['p009', 'r001'],
    ['pr001', 'pr003'], ['pr001', 'pr004'], ['pr002', 'pr001'],
    ['p001', 'pr001'], ['p002', 'pr001'], ['p003', 'pr001'],
    ['p004', 'pr001'], ['p005', 'pr001'], ['p006', 'pr001'],
    ['p007', 'pr001'], ['p008', 'pr001'], ['p009', 'pr001'],
    ['p010', 'pr001'],
    ['s001', 'pr001'], ['s001', 'pr003'],
    ['s011', 'p006'],
    ['r004', 'p003'], ['r004', 'p006'],
  ];

  const activeEdges = $derived.by<VaultLink[]>(() => {
    const nodeIds = new Set(activeNodes.map(n => n.id));
    const edges: VaultLink[] = [];
    const seen = new Set<string>();

    if (liveNodeMap.size > 0) {
      for (const [id, node] of liveNodeMap) {
        for (const ref of node.crossRefs ?? []) {
          if (liveNodeMap.has(ref)) {
            const key = [id, ref].sort().join('-');
            if (!seen.has(key)) { seen.add(key); edges.push({ source: id, target: ref }); }
          }
        }
        if (node.parentId && liveNodeMap.has(node.parentId)) {
          edges.push({ source: id, target: node.parentId, parent: true });
        }
      }
    }

    for (const [src, tgt] of KNOWN_EDGES) {
      if (nodeIds.has(src) && nodeIds.has(tgt)) {
        const key = [src, tgt].sort().join('-');
        if (!seen.has(key)) { seen.add(key); edges.push({ source: src, target: tgt }); }
      }
    }

    return edges;
  });

  // ---------------------------------------------------------------------------
  // UI state
  // ---------------------------------------------------------------------------

  let searchQuery = $state('');
  let collapsedKinds = $state<Set<VaultKind>>(new Set());
  let selectedId = $state<string | null>(null);
  let hoveredId = $state<string | null>(null);
  let searchInput = $state<HTMLInputElement | undefined>(undefined);
  let activeKindFilter = $state<VaultKind | null>(null);

  function toggleKind(kind: VaultKind): void {
    const next = new Set(collapsedKinds);
    if (next.has(kind)) next.delete(kind); else next.add(kind);
    collapsedKinds = next;
  }

  function toggleKindFilter(kind: VaultKind): void {
    activeKindFilter = activeKindFilter === kind ? null : kind;
  }

  // ---------------------------------------------------------------------------
  // Derived: categorized + filtered
  // ---------------------------------------------------------------------------

  function inferKind(vaultId: string): VaultKind {
    if (vaultId.startsWith('pr'))   return 'pr';
    if (vaultId.startsWith('lore')) return 'lore';
    if (vaultId.startsWith('agt'))  return 'agt';
    if (vaultId.startsWith('p'))    return 'p';
    if (vaultId.startsWith('r'))    return 'r';
    if (vaultId.startsWith('s'))    return 's';
    if (vaultId.startsWith('h'))    return 'h';
    return 'p';
  }

  function nodeState(node: VaultNode): NodeState {
    if (!node.updatedMs) return 'background';
    const age = Date.now() - node.updatedMs;
    if (age < 5 * 60 * 1000) return 'active';
    if (age < RECENT_WINDOW_MS) return 'recent';
    if (age < 24 * 60 * 60 * 1000) return 'ambient';
    return 'background';
  }

  function connectionsFor(id: string): string[] {
    const refs = new Set<string>();
    for (const e of activeEdges) {
      if (e.source === id) refs.add(e.target);
      if (e.target === id) refs.add(e.source);
    }
    return Array.from(refs);
  }

  interface CategoryGroup {
    kind: VaultKind;
    label: string;
    glyph: string;
    vaults: VaultNode[];
    collapsed: boolean;
  }

  const RECENTS_LIMIT = 6;
  const RECENTS_WINDOW_MS = 24 * 60 * 60 * 1000;

  const recentVaults = $derived.by<VaultNode[]>(() => {
    return activeNodes
      .filter((n) => n.updatedMs && (Date.now() - n.updatedMs) < RECENTS_WINDOW_MS)
      .sort((a, b) => (b.updatedMs ?? 0) - (a.updatedMs ?? 0))
      .slice(0, RECENTS_LIMIT);
  });

  function matchesSearch(n: VaultNode, q: string): boolean {
    return n.id.toLowerCase().includes(q) ||
      n.label.toLowerCase().includes(q) ||
      (n.displayName ?? '').toLowerCase().includes(q) ||
      (n.shortLabel ?? '').toLowerCase().includes(q);
  }

  const categories = $derived.by<CategoryGroup[]>(() => {
    const q = searchQuery.toLowerCase().trim();
    const kindsToShow = activeKindFilter ? [activeKindFilter] : CATEGORY_ORDER;
    return kindsToShow
      .map((kind) => {
        let vaults = activeNodes.filter((n) => n.kind === kind);
        if (q) {
          vaults = vaults.filter((n) => matchesSearch(n, q));
        }
        return {
          kind,
          label: KIND_LABEL[kind],
          glyph: KIND_GLYPH[kind],
          vaults,
          collapsed: collapsedKinds.has(kind),
        };
      })
      .filter((g) => g.vaults.length > 0);
  });

  const kindCounts = $derived.by<Record<VaultKind, number>>(() => {
    const counts: Record<string, number> = {};
    for (const n of activeNodes) {
      counts[n.kind] = (counts[n.kind] ?? 0) + 1;
    }
    return counts as Record<VaultKind, number>;
  });

  const totalCount = $derived(activeNodes.length);
  const totalEdgeCount = $derived(activeEdges.length);
  const activeCategoryCount = $derived(
    CATEGORY_ORDER.filter((k) => (kindCounts[k] ?? 0) > 0).length
  );

  const hoveredConnections = $derived.by<Set<string>>(() => {
    if (!hoveredId) return new Set();
    return new Set(connectionsFor(hoveredId));
  });

  const treeHighlightedVaultIds = $derived.by<Set<string>>(() => {
    const treePath = crossRefHighlight.hoveredTreePath;
    if (!treePath) return new Set();
    const entries = enrichmentStore.get(treePath);
    if (!entries) return new Set();
    return new Set(entries.map((e) => e.vault_id));
  });

  // ---------------------------------------------------------------------------
  // Bus subscription
  // ---------------------------------------------------------------------------

  onMount(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    function onKeyDown(e: KeyboardEvent): void {
      if (e.key === '/' && document.activeElement?.tagName !== 'INPUT') {
        e.preventDefault();
        searchInput?.focus();
      }
    }
    document.addEventListener('keydown', onKeyDown);

    const pendingUpdates = new Map<string, VaultNode>();
    const pendingDeletes = new Set<string>();
    let flushTimer: number | null = null;
    const FLUSH_QUIET_MS = 150;

    function flushPendingVaults(): void {
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
        flushTimer = null;
      }
      if (pendingUpdates.size === 0 && pendingDeletes.size === 0) return;
      const next = new Map(liveNodeMap);
      for (const id of pendingDeletes) next.delete(id);
      for (const [id, node] of pendingUpdates) next.set(id, node);
      pendingDeletes.clear();
      pendingUpdates.clear();
      liveNodeMap = next;
    }

    function scheduleFlush(): void {
      if (flushTimer !== null) window.clearTimeout(flushTimer);
      flushTimer = window.setTimeout(() => {
        flushTimer = null;
        flushPendingVaults();
      }, FLUSH_QUIET_MS);
    }

    void (async () => {
      try {
        const u = await subscribe({ category: 'index' }, (env) => {
          if (env.kind === 'vault.update') {
            const p = env.payload as VaultUpdatePayload;
            if (p.change_kind === 'deleted') {
              pendingUpdates.delete(p.vault_id);
              pendingDeletes.add(p.vault_id);
            } else {
              const node: VaultNode = {
                id: p.vault_id,
                kind: inferKind(p.vault_id),
                label: p.name ? `${p.vault_id} — ${p.name}` : p.vault_id,
                displayName: p.name ?? undefined,
                shortLabel: p.short_label ?? undefined,
                updatedMs: p.updated_ms ?? undefined,
                crossRefs: p.cross_refs ?? [],
                parentId: p.parent_vault_id ?? undefined,
                path: p.path,
              };
              pendingDeletes.delete(p.vault_id);
              pendingUpdates.set(p.vault_id, node);
            }
            scheduleFlush();
          } else if (env.kind === 'walk.complete') {
            flushPendingVaults();
            walkComplete = true;
          }
        });
        if (cancelled) { void u().catch(() => {}); }
        else { unsub = u; }

        if (!cancelled) {
          invoke('vault_rescan').catch((err: unknown) => {
            console.warn('[IndexGraph] vault_rescan failed:', err);
          });
        }
      } catch (err) {
        console.warn('[IndexGraph] Category::Index subscribe failed:', err);
        walkComplete = true;
      }
    })();

    return () => {
      cancelled = true;
      document.removeEventListener('keydown', onKeyDown);
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
        flushTimer = null;
      }
      pendingUpdates.clear();
      pendingDeletes.clear();
      if (dragActive || dragNode) {
        document.removeEventListener('mousemove', onDocMouseMove);
        document.removeEventListener('mouseup', onDocMouseUp);
        dragActive = false;
        dragNode = null;
      }
      void (async () => { await unsub?.(); })();
    };
  });

  // ---------------------------------------------------------------------------
  // Drag-to-terminal (manual gesture — WebView2 SVG drag limitation)
  // ---------------------------------------------------------------------------

  let ghostVisible = $state(false);
  let ghostX = $state(0);
  let ghostY = $state(0);
  let ghostLabel = $state('');
  let ghostKind = $state<VaultKind>('p');
  let draggingId = $state<string | null>(null);

  const DRAG_THRESHOLD_PX = 5;
  let dragNode: VaultNode | null = null;
  let dragStartX = 0;
  let dragStartY = 0;
  let dragActive = false;

  function onRowMouseDown(e: MouseEvent, node: VaultNode): void {
    if (e.button !== 0) return;
    dragNode = node;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    dragActive = false;
    document.addEventListener('mousemove', onDocMouseMove);
    document.addEventListener('mouseup', onDocMouseUp);
  }

  function onDocMouseMove(e: MouseEvent): void {
    if (!dragNode) return;
    const dx = e.clientX - dragStartX;
    const dy = e.clientY - dragStartY;
    if (!dragActive && Math.abs(dx) + Math.abs(dy) < DRAG_THRESHOLD_PX) return;
    if (!dragActive) {
      dragActive = true;
      draggingId = dragNode.id;
      ghostLabel = dragNode.shortLabel || dragNode.displayName || dragNode.id;
      ghostKind = dragNode.kind;
    }
    ghostX = e.clientX + 12;
    ghostY = e.clientY - 8;
    ghostVisible = true;
  }

  function onDocMouseUp(e: MouseEvent): void {
    document.removeEventListener('mousemove', onDocMouseMove);
    document.removeEventListener('mouseup', onDocMouseUp);
    if (dragActive && dragNode?.path) {
      const target = document.elementFromPoint(e.clientX, e.clientY);
      const terminal = target?.closest('.terminal-host');
      if (terminal) {
        terminal.dispatchEvent(new CustomEvent<RiftVaultDropDetail>(
          RIFT_VAULT_DROP_EVENT,
          { detail: { path: dragNode.path }, bubbles: true },
        ));
      }
    }
    dragNode = null;
    dragActive = false;
    draggingId = null;
    ghostVisible = false;
  }

  function formatAge(ms?: number): string {
    if (!ms) return '';
    const age = Date.now() - ms;
    const sec = Math.floor(age / 1000);
    if (sec < 60) return 'now';
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}m`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h`;
    const d = Math.floor(hr / 24);
    return `${d}d`;
  }
</script>

<div class="index-browser">
  <!-- Header: mode toggle + count + search -->
  <div class="browser-header">
    <div class="mode-toggle">
      <button type="button"
        class="mode-btn"
        class:active={viewMode === 'vaults'}
        onclick={() => { viewMode = 'vaults'; }}
      >VAULTS</button>
      <button type="button"
        class="mode-btn"
        class:active={viewMode === 'content'}
        onclick={() => { viewMode = 'content'; }}
      >CONTENT</button>
    </div>
    {#if viewMode === 'vaults'}
      <span class="browser-count">{totalCount}</span>
      <input
        class="browser-search"
        type="text"
        placeholder="filter vaults... ( / )"
        aria-label="filter vault nodes"
        bind:value={searchQuery}
        bind:this={searchInput}
      />
    {/if}
  </div>

  {#if viewMode === 'content'}
    <IndexContentBrowser />
  {:else}
    <!-- Stats strip -->
    <div class="stats-strip">
      <div class="stat-block">
        <span class="stat-value">{totalCount}</span>
        <span class="stat-label">VAULTS</span>
      </div>
      <div class="stat-divider"></div>
      <div class="stat-block">
        <span class="stat-value">{totalEdgeCount}</span>
        <span class="stat-label">LINKS</span>
      </div>
      <div class="stat-divider"></div>
      <div class="stat-block">
        <span class="stat-value">{activeCategoryCount}</span>
        <span class="stat-label">KINDS</span>
      </div>
    </div>

    <!-- Kind filter chips -->
    {#if activeNodes.length > 0 || walkComplete}
      <div class="kind-chips">
        {#each CATEGORY_ORDER as kind (kind)}
          {@const count = kindCounts[kind] ?? 0}
          {#if count > 0}
            <button
              type="button"
              class="kind-chip kind-chip-{kind}"
              class:active={activeKindFilter === kind}
              onclick={() => toggleKindFilter(kind)}
              title="{KIND_LABEL[kind]} ({count})"
            >
              <span class="chip-glyph">{KIND_GLYPH[kind]}</span>
              <span class="chip-label">{KIND_LABEL[kind]}</span>
              <span class="chip-count">{count}</span>
            </button>
          {/if}
        {/each}
        {#if activeKindFilter}
          <button
            type="button"
            class="kind-chip kind-chip-clear"
            onclick={() => { activeKindFilter = null; }}
            title="Clear filter"
          >ALL</button>
        {/if}
      </div>
    {/if}

    <!-- Vault list -->
    <div class="browser-body">
      {#if activeNodes.length === 0 && !walkComplete}
        <div class="browser-loading empty-state">
          <span class="loading-glyph empty-state-icon">◆</span>
          <span class="empty-state-text">scanning vaults...</span>
        </div>
      {:else if categories.length === 0 && searchQuery}
        <div class="browser-empty empty-state">
          <span class="empty-state-icon">◇</span>
          <span class="empty-state-text">no vaults match "{searchQuery}"</span>
          <span class="empty-state-hint">try a shorter term or clear the filter</span>
        </div>
      {:else}
        <!-- Recents strip -->
        {#if recentVaults.length > 0 && !searchQuery && !activeKindFilter}
          <div class="recents-strip">
            <div class="recents-label">
              <span class="recents-glyph">◆</span>
              RECENT
            </div>
            <div class="recents-scroll">
              {#each recentVaults as vault (vault.id)}
                {@const state = nodeState(vault)}
                <button
                  type="button"
                  class="recent-card"
                  class:selected={selectedId === vault.id}
                  onmouseenter={() => { hoveredId = vault.id; crossRefHighlight.hoveredVaultId = vault.id; }}
                  onmouseleave={() => { if (hoveredId === vault.id) hoveredId = null; crossRefHighlight.hoveredVaultId = null; }}
                  onclick={() => { selectedId = selectedId === vault.id ? null : vault.id; }}
                  onmousedown={(e) => onRowMouseDown(e, vault)}
                >
                  <div class="rc-accent kind-bg-{vault.kind}"></div>
                  <div class="rc-content">
                    <div class="rc-top">
                      <span class="state-dot state-{state}"></span>
                      <span class="rc-glyph kind-{vault.kind}">{KIND_GLYPH[vault.kind]}</span>
                      <span class="rc-id">{vault.id}</span>
                    </div>
                    <div class="rc-name">{vault.displayName || vault.shortLabel || ''}</div>
                    <div class="rc-age">{formatAge(vault.updatedMs)}</div>
                  </div>
                </button>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Categories -->
        {#each categories as group (group.kind)}
          <button
            type="button"
            class="category-header cat-{group.kind}"
            onclick={() => toggleKind(group.kind)}
            aria-expanded={!group.collapsed}
          >
            <span class="cat-accent kind-bg-{group.kind}"></span>
            <span class="category-chevron" class:collapsed={group.collapsed}>&#9662;</span>
            <span class="category-glyph kind-{group.kind}">{group.glyph}</span>
            <span class="category-label">{group.label}</span>
            <span class="category-count">{group.vaults.length}</span>
          </button>

          {#if !group.collapsed}
            <div class="category-body">
              {#each group.vaults as vault (vault.id)}
                {@const state = nodeState(vault)}
                {@const conns = connectionsFor(vault.id)}
                {@const isHighlighted = hoveredId === vault.id || hoveredConnections.has(vault.id)}
                {@const isTreeHighlighted = treeHighlightedVaultIds.has(vault.id)}
                {@const isSelected = selectedId === vault.id}
                <button
                  type="button"
                  class="vault-row"
                  class:highlighted={isHighlighted}
                  class:tree-highlighted={isTreeHighlighted}
                  class:selected={isSelected}
                  class:dragging={draggingId === vault.id}
                  onmouseenter={() => { hoveredId = vault.id; crossRefHighlight.hoveredVaultId = vault.id; }}
                  onmouseleave={() => { if (hoveredId === vault.id) hoveredId = null; crossRefHighlight.hoveredVaultId = null; }}
                  onclick={() => { selectedId = selectedId === vault.id ? null : vault.id; }}
                  onmousedown={(e) => onRowMouseDown(e, vault)}
                >
                  <span class="vault-kind-bar kind-bg-{vault.kind}"></span>
                  <div class="vault-inner">
                    <div class="vault-main">
                      <span class="state-dot state-{state}"></span>
                      <span class="vault-glyph kind-{vault.kind}">{KIND_GLYPH[vault.kind]}</span>
                      <span class="vault-id">{vault.id}</span>
                      <span class="vault-name">{vault.displayName || vault.shortLabel || ''}</span>
                    </div>
                    <div class="vault-meta">
                      {#if vault.updatedMs}
                        <span class="vault-age">{formatAge(vault.updatedMs)}</span>
                      {/if}
                      {#if conns.length > 0}
                        <span class="vault-conn-badge" title={conns.join(', ')}>&#10231; {conns.length}</span>
                      {/if}
                    </div>
                  </div>
                </button>

                <!-- Expanded detail panel -->
                {#if isSelected}
                  <div class="vault-detail">
                    {#if vault.path}
                      <div class="detail-row">
                        <span class="detail-label">PATH</span>
                        <span class="detail-value">{vault.path}</span>
                      </div>
                    {/if}
                    {#if vault.updatedMs}
                      <div class="detail-row">
                        <span class="detail-label">MODIFIED</span>
                        <span class="detail-value">{new Date(vault.updatedMs).toLocaleString()}</span>
                      </div>
                    {/if}
                    {#if conns.length > 0}
                      <div class="detail-connections">
                        <span class="detail-conn-label">CONNECTIONS</span>
                        <div class="detail-conn-chips">
                          {#each conns as ref}
                            <button
                              type="button"
                              class="conn-chip kind-{inferKind(ref)}"
                              onclick={(e) => { e.stopPropagation(); selectedId = ref; }}
                              title={ref}
                            >
                              <span class="conn-chip-glyph">{KIND_GLYPH[inferKind(ref)]}</span>
                              <span>{ref}</span>
                            </button>
                          {/each}
                        </div>
                      </div>
                    {/if}
                  </div>
                {/if}
              {/each}
            </div>
          {/if}
        {/each}
      {/if}
    </div>
  {/if}
</div>

<!-- Drag ghost -->
{#if ghostVisible}
  <div
    class="drag-ghost kind-{ghostKind}"
    style="left: {ghostX}px; top: {ghostY}px;"
  >
    <span class="drag-ghost-glyph">{KIND_GLYPH[ghostKind]}</span>
    <span>{ghostLabel}</span>
  </div>
{/if}

<style>
  .index-browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-base);
    font-family: var(--font-family);
    font-size: var(--text-base);
    color: var(--amber-warm);
    user-select: none;
  }

  /* ========== Header ========== */
  .browser-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    background: var(--bg-surface);
    box-shadow: var(--sep-glow);
    flex-shrink: 0;
  }
  .mode-toggle { display: flex; }
  .mode-btn {
    padding: 3px var(--space-md);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    background: var(--bg-elevated);
    color: var(--amber-faint);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    cursor: pointer;
    transition: all var(--duration-base);
  }
  .mode-btn:first-child { border-top-right-radius: 0; border-bottom-right-radius: 0; border-right: none; }
  .mode-btn:last-child  { border-top-left-radius: 0; border-bottom-left-radius: 0; }
  .mode-btn.active {
    color: var(--term-cyan);
    border-color: var(--term-cyan);
    background: rgba(74, 212, 212, 0.1);
    box-shadow: 0 0 6px rgba(74, 212, 212, 0.15);
  }
  .mode-btn:hover:not(.active) {
    color: var(--amber-dim);
    border-color: var(--amber-dim);
    background: var(--bg-hover);
  }
  .mode-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .browser-count {
    font-size: var(--text-2xs);
    font-weight: 700;
    color: var(--bg-base);
    background: var(--amber-dim);
    padding: 2px 7px;
    min-width: 20px;
    text-align: center;
    border-radius: 10px;
  }
  .browser-search {
    flex: 1;
    min-width: 0;
    height: var(--control-sm);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    padding: 0 var(--space-md);
    outline: 2px solid transparent;
    transition: border-color var(--duration-med), box-shadow var(--duration-med);
  }
  .browser-search::placeholder { color: var(--amber-faint); font-style: italic; }
  .browser-search:focus {
    border-color: var(--amber-dim);
    box-shadow: 0 0 0 1px rgba(255, 168, 38, 0.15), inset 0 1px 3px rgba(0, 0, 0, 0.2);
  }

  /* ========== Stats Strip ========== */
  .stats-strip {
    display: flex;
    align-items: stretch;
    flex-shrink: 0;
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
  }
  .stat-block {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: var(--space-8) var(--space-md);
  }
  .stat-divider {
    width: 1px;
    background: var(--border-subtle);
    align-self: stretch;
    margin: var(--space-sm) 0;
  }
  .stat-value {
    font-size: 18px;
    font-weight: 800;
    color: var(--amber-bright);
    text-shadow: 0 0 12px rgba(255, 200, 64, 0.3);
    line-height: 1;
  }
  .stat-label {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.16em;
    color: var(--amber-faint);
    margin-top: 2px;
  }

  /* ========== Kind filter chips ========== */
  .kind-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-12) var(--space-8);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .kind-chip {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 3px var(--space-8);
    border: 1px solid var(--border-subtle);
    border-radius: 12px;
    background: var(--bg-elevated);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    cursor: pointer;
    transition: all var(--duration-base);
  }
  .kind-chip:hover {
    border-color: var(--amber-dim);
    background: var(--bg-hover);
    color: var(--amber-warm);
  }
  .kind-chip:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .kind-chip.active { border-color: currentColor; box-shadow: 0 0 6px rgba(255, 168, 38, 0.15); }
  .kind-chip-p.active       { color: var(--amber-bright); background: rgba(255, 200, 64, 0.1); }
  .kind-chip-pr.active      { color: var(--amber-warm);   background: rgba(240, 160, 48, 0.1); }
  .kind-chip-r.active       { color: var(--term-cyan);    background: rgba(111, 224, 224, 0.1); }
  .kind-chip-s.active       { color: var(--term-blue);    background: rgba(108, 182, 255, 0.1); }
  .kind-chip-lore.active    { color: var(--term-purple);  background: rgba(197, 143, 255, 0.1); }
  .kind-chip-agt.active     { color: var(--term-green);   background: rgba(79, 232, 85, 0.1); }
  .kind-chip-h.active       { color: var(--amber-faint);  background: rgba(168, 120, 48, 0.1); }
  .kind-chip-clear {
    color: var(--amber-faint);
    border-style: dashed;
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
  }
  .kind-chip-clear:hover { color: var(--amber-warm); border-style: solid; }
  .chip-glyph { font-size: var(--text-xs); }
  .chip-label { text-transform: uppercase; }
  .chip-count { font-size: var(--text-2xs); color: var(--amber-faint); font-weight: 400; }
  .kind-chip.active .chip-count { color: inherit; opacity: 0.7; }

  /* ========== Recents Strip ========== */
  .recents-strip {
    flex-shrink: 0;
    box-shadow: var(--sep-depth);
  }
  .recents-label {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-12);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
  }
  .recents-glyph {
    font-size: var(--text-xs);
    text-shadow: var(--glow-amber);
  }
  .recents-scroll {
    display: flex;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    overflow-x: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--amber-faint) transparent;
    background: linear-gradient(to bottom, rgba(255, 200, 64, 0.05), transparent);
  }
  .recents-scroll::-webkit-scrollbar { height: 4px; }
  .recents-scroll::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .recent-card {
    flex-shrink: 0;
    display: flex;
    min-width: 100px;
    max-width: 140px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    font-family: inherit;
    cursor: pointer;
    transition: all var(--duration-med);
    overflow: hidden;
  }
  .recent-card:hover {
    border-color: var(--amber-dim);
    background: var(--bg-hover);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  }
  .recent-card:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .recent-card.selected {
    border-color: var(--amber-bright);
    box-shadow: 0 0 8px rgba(255, 200, 64, 0.2);
  }
  .rc-accent { width: 3px; flex-shrink: 0; }
  .rc-content {
    padding: var(--space-sm) var(--space-8);
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .rc-top { display: flex; align-items: center; gap: var(--space-xs); }
  .rc-glyph { font-size: var(--text-xs); }
  .rc-id { font-size: var(--text-xs); font-weight: 700; color: var(--amber-primary); }
  .rc-name {
    font-size: var(--text-2xs);
    color: var(--amber-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rc-age { font-size: var(--text-2xs); color: var(--amber-faint); font-style: italic; }

  /* ========== Scrollable body ========== */
  .browser-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-color: var(--amber-faint) transparent;
    scrollbar-width: thin;
  }
  .browser-body::-webkit-scrollbar { width: 6px; }
  .browser-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* .browser-loading and .browser-empty extend .empty-state (global) for layout/color;
     local overrides only: loading-glyph animation. */
  .loading-glyph {
    font-size: var(--text-xl);
    font-style: normal;
    animation: pulse-glyph 1.6s ease-in-out infinite;
  }
  @keyframes pulse-glyph {
    0%, 100% { opacity: 0.3; text-shadow: none; }
    50%      { opacity: 1;   text-shadow: var(--glow-amber); }
  }

  /* ========== Category header ========== */
  .category-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    width: 100%;
    padding: var(--space-8) var(--space-12) var(--space-8) var(--space-14);
    background: var(--bg-surface);
    border: none;
    box-shadow: var(--sep-depth);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    cursor: pointer;
    transition: color var(--duration-base), background var(--duration-base);
    text-align: left;
    position: relative;
    overflow: hidden;
  }
  .category-header:hover { color: var(--amber-warm); }
  .category-header:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .cat-accent {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 3px;
  }
  .cat-p    { background: linear-gradient(to right, rgba(255, 200, 64, 0.06), var(--bg-surface) 60%); }
  .cat-pr   { background: linear-gradient(to right, rgba(255, 168, 38, 0.06), var(--bg-surface) 60%); }
  .cat-r    { background: linear-gradient(to right, rgba(111, 224, 224, 0.06), var(--bg-surface) 60%); }
  .cat-s    { background: linear-gradient(to right, rgba(108, 182, 255, 0.06), var(--bg-surface) 60%); }
  .cat-lore { background: linear-gradient(to right, rgba(197, 143, 255, 0.06), var(--bg-surface) 60%); }
  .cat-agt  { background: linear-gradient(to right, rgba(79, 232, 85, 0.06), var(--bg-surface) 60%); }
  .cat-h    { background: linear-gradient(to right, rgba(168, 120, 48, 0.06), var(--bg-surface) 60%); }
  .cat-p:hover    { background: linear-gradient(to right, rgba(255, 200, 64, 0.10), var(--bg-hover) 60%); }
  .cat-pr:hover   { background: linear-gradient(to right, rgba(255, 168, 38, 0.10), var(--bg-hover) 60%); }
  .cat-r:hover    { background: linear-gradient(to right, rgba(111, 224, 224, 0.10), var(--bg-hover) 60%); }
  .cat-s:hover    { background: linear-gradient(to right, rgba(108, 182, 255, 0.10), var(--bg-hover) 60%); }
  .cat-lore:hover { background: linear-gradient(to right, rgba(197, 143, 255, 0.10), var(--bg-hover) 60%); }
  .cat-agt:hover  { background: linear-gradient(to right, rgba(79, 232, 85, 0.10), var(--bg-hover) 60%); }
  .cat-h:hover    { background: linear-gradient(to right, rgba(168, 120, 48, 0.10), var(--bg-hover) 60%); }

  .category-chevron {
    font-size: var(--text-xs);
    transition: transform var(--duration-med) var(--ease-out);
    display: inline-block;
    width: 12px;
    text-align: center;
  }
  .category-chevron.collapsed { transform: rotate(-90deg); }
  .category-glyph { font-size: var(--text-sm); }
  .category-label { flex: 1; }
  .category-count { font-size: var(--text-2xs); color: var(--amber-faint); font-weight: 400; }
  .category-body { box-shadow: var(--sep-depth); }

  /* ========== Vault row (2-line) ========== */
  .vault-row {
    display: flex;
    width: 100%;
    padding: 0;
    background: transparent;
    border: none;
    color: var(--amber-warm);
    font-family: inherit;
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast), color var(--duration-fast);
    position: relative;
  }
  .vault-row:hover { background: var(--bg-hover); }
  .vault-row:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .vault-row:hover .vault-kind-bar { opacity: 1; }
  .vault-kind-bar {
    width: 2px;
    flex-shrink: 0;
    opacity: 0.4;
    transition: opacity var(--duration-med);
  }
  .vault-inner {
    flex: 1;
    min-width: 0;
    padding: 5px var(--space-12) 5px var(--space-8);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .vault-main {
    display: flex;
    align-items: center;
    gap: var(--space-8);
  }
  .vault-meta {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding-left: 34px;
  }
  .vault-row.highlighted {
    background: rgba(255, 168, 38, 0.12);
    animation: xref-flash 0.3s ease-out;
  }
  .vault-row.highlighted .vault-kind-bar { opacity: 1; }
  .vault-row.tree-highlighted {
    background: rgba(74, 212, 212, 0.1);
    animation: xref-cyan-flash 0.3s ease-out;
  }
  @keyframes xref-flash {
    from { background: rgba(255, 168, 38, 0.25); }
    to   { background: rgba(255, 168, 38, 0.12); }
  }
  @keyframes xref-cyan-flash {
    from { background: rgba(74, 212, 212, 0.22); }
    to   { background: rgba(74, 212, 212, 0.1); }
  }
  .vault-row.selected { background: var(--bg-hover); }
  .vault-row.selected .vault-kind-bar { opacity: 1; }
  .vault-row.selected .vault-id { color: var(--amber-bright); }
  .vault-row.dragging { opacity: 0.4; }

  /* State dot */
  .state-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .state-active {
    background: var(--amber-bright);
    box-shadow: 0 0 6px var(--amber-bright);
    animation: dot-pulse 2s ease-in-out infinite;
  }
  @keyframes dot-pulse {
    0%, 100% { box-shadow: 0 0 4px var(--amber-bright); }
    50%      { box-shadow: 0 0 10px var(--amber-bright), 0 0 16px rgba(255, 200, 64, 0.3); }
  }
  .state-recent    { background: var(--amber-warm); }
  .state-ambient   { background: var(--amber-faint); }
  .state-background { background: var(--border-subtle); }

  /* Vault row elements */
  .vault-glyph {
    font-size: var(--text-xs);
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }
  .vault-id {
    font-weight: 600;
    color: var(--amber-primary);
    min-width: 48px;
    flex-shrink: 0;
    font-size: var(--text-sm);
    transition: color var(--duration-base);
  }
  .vault-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--amber-dim);
    font-size: var(--text-xs);
  }
  .vault-age {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    font-style: italic;
    flex-shrink: 0;
  }
  .vault-conn-badge {
    font-size: var(--text-2xs);
    color: var(--amber-dim);
    flex-shrink: 0;
    padding: 1px var(--space-sm);
    border: 1px solid var(--border-subtle);
    letter-spacing: 0.04em;
    transition: border-color var(--duration-base);
  }
  .vault-row:hover .vault-conn-badge { border-color: var(--amber-dim); }

  /* Kind colors */
  .kind-p    { color: var(--amber-bright); }
  .kind-pr   { color: var(--amber-warm); }
  .kind-r    { color: var(--term-cyan); }
  .kind-s    { color: var(--term-blue); }
  .kind-lore { color: var(--term-purple); }
  .kind-agt  { color: var(--term-green); }
  .kind-h    { color: var(--amber-faint); }

  /* Kind background bars */
  .kind-bg-p    { background: var(--amber-bright); }
  .kind-bg-pr   { background: var(--amber-warm); }
  .kind-bg-r    { background: var(--term-cyan); }
  .kind-bg-s    { background: var(--term-blue); }
  .kind-bg-lore { background: var(--term-purple); }
  .kind-bg-agt  { background: var(--term-green); }
  .kind-bg-h    { background: var(--amber-faint); }

  /* ========== Vault detail panel ========== */
  .vault-detail {
    padding: var(--space-sm) var(--space-md) var(--space-md) var(--space-lg);
    background: var(--bg-surface);
    border-left: 2px solid var(--amber-dim);
    margin-left: 2px;
  }
  .detail-row {
    display: flex;
    gap: var(--space-8);
    padding: 2px 0;
    align-items: baseline;
  }
  .detail-label {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    min-width: 60px;
    flex-shrink: 0;
  }
  .detail-value {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .detail-connections {
    margin-top: var(--space-sm);
    padding-top: var(--space-sm);
    border-top: 1px solid var(--border-subtle);
  }
  .detail-conn-label {
    display: block;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    margin-bottom: var(--space-sm);
  }
  .detail-conn-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .conn-chip {
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 2px var(--space-8);
    border: 1px solid currentColor;
    background: transparent;
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    cursor: pointer;
    transition: background var(--duration-base), box-shadow var(--duration-base);
  }
  .conn-chip:hover {
    background: rgba(255, 168, 38, 0.1);
    box-shadow: 0 0 6px rgba(255, 168, 38, 0.15);
  }
  .conn-chip:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .conn-chip-glyph { font-size: var(--text-2xs); }

  /* ========== Drag ghost ========== */
  .drag-ghost {
    position: fixed;
    pointer-events: none;
    z-index: 5000;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md);
    background: var(--bg-elevated);
    border: 1px solid var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    color: var(--amber-bright);
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.5);
    opacity: 0.92;
  }
  .drag-ghost-glyph { font-size: var(--text-base); }
</style>
