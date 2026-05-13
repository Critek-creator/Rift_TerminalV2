<script lang="ts">
  // IndexGraph.svelte — Vault Browser (list-based replacement for radial graph)
  //
  // Structured grouped list of Abyssal Index vaults. Categories collapse/expand,
  // vaults show state dots + connection badges, search filters across all.
  //
  // Data flow (unchanged from graph era):
  //   vault_walker (Rust) → Category::Index/vault.update → liveNodeMap
  //   vault_walker (Rust) → Category::Index/walk.complete → walkComplete flag
  //
  // Drag-to-terminal preserved: mousedown on vault row → ghost → drop on terminal.

  import { onMount } from 'svelte';
  import { subscribe } from './bus';
  import { RIFT_VAULT_DROP_EVENT, type RiftVaultDropDetail } from './dragMime';

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
    if (liveNodeMap.size > 0) return Array.from(liveNodeMap.values());
    if (walkComplete) return STATIC_NODES.map((n) => ({ ...n }));
    return [];
  });

  const activeEdges = $derived.by<VaultLink[]>(() => {
    if (liveNodeMap.size === 0 && walkComplete) return [];
    const edges: VaultLink[] = [];
    for (const [id, node] of liveNodeMap) {
      for (const ref of node.crossRefs ?? []) {
        if (liveNodeMap.has(ref)) edges.push({ source: id, target: ref });
      }
      if (node.parentId && liveNodeMap.has(node.parentId)) {
        edges.push({ source: id, target: node.parentId, parent: true });
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

  function toggleKind(kind: VaultKind): void {
    const next = new Set(collapsedKinds);
    if (next.has(kind)) next.delete(kind); else next.add(kind);
    collapsedKinds = next;
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

  const categories = $derived.by<CategoryGroup[]>(() => {
    const q = searchQuery.toLowerCase().trim();
    return CATEGORY_ORDER
      .map((kind) => {
        let vaults = activeNodes.filter((n) => n.kind === kind);
        if (q) {
          vaults = vaults.filter((n) =>
            n.id.toLowerCase().includes(q) ||
            n.label.toLowerCase().includes(q) ||
            (n.displayName ?? '').toLowerCase().includes(q) ||
            (n.shortLabel ?? '').toLowerCase().includes(q)
          );
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

  const totalCount = $derived(activeNodes.length);

  const hoveredConnections = $derived.by<Set<string>>(() => {
    if (!hoveredId) return new Set();
    return new Set(connectionsFor(hoveredId));
  });

  // ---------------------------------------------------------------------------
  // Bus subscription (same debounce pattern as old graph)
  // ---------------------------------------------------------------------------

  onMount(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

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
      } catch (err) {
        console.warn('[IndexGraph] Category::Index subscribe failed:', err);
        walkComplete = true;
      }
    })();

    return () => {
      cancelled = true;
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
        flushTimer = null;
      }
      pendingUpdates.clear();
      pendingDeletes.clear();
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
    e.preventDefault();
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
  <!-- Search -->
  <div class="browser-header">
    <span class="browser-title">INDEX</span>
    <span class="browser-count">{totalCount}</span>
    <input
      class="browser-search"
      type="text"
      placeholder="filter vaults…"
      bind:value={searchQuery}
    />
  </div>

  <!-- Vault list -->
  <div class="browser-body">
    {#if activeNodes.length === 0 && !walkComplete}
      <div class="browser-loading">
        <span class="loading-glyph">◆</span>
        <span>scanning vaults…</span>
      </div>
    {:else if categories.length === 0 && searchQuery}
      <div class="browser-empty">no vaults match "{searchQuery}"</div>
    {:else}
      {#each categories as group (group.kind)}
        <!-- Category header -->
        <button
          type="button"
          class="category-header"
          onclick={() => toggleKind(group.kind)}
          aria-expanded={!group.collapsed}
        >
          <span class="category-chevron" class:collapsed={group.collapsed}>▾</span>
          <span class="category-glyph kind-{group.kind}">{group.glyph}</span>
          <span class="category-label">{group.label}</span>
          <span class="category-count">{group.vaults.length}</span>
        </button>

        <!-- Vault rows -->
        {#if !group.collapsed}
          <div class="category-body">
            {#each group.vaults as vault (vault.id)}
              {@const state = nodeState(vault)}
              {@const conns = connectionsFor(vault.id)}
              {@const isHighlighted = hoveredId === vault.id || hoveredConnections.has(vault.id)}
              {@const isSelected = selectedId === vault.id}
              <button
                type="button"
                class="vault-row"
                class:highlighted={isHighlighted}
                class:selected={isSelected}
                class:dragging={draggingId === vault.id}
                onmouseenter={() => { hoveredId = vault.id; }}
                onmouseleave={() => { if (hoveredId === vault.id) hoveredId = null; }}
                onclick={() => { selectedId = selectedId === vault.id ? null : vault.id; }}
                onmousedown={(e) => onRowMouseDown(e, vault)}
              >
                <span class="state-dot state-{state}"></span>
                <span class="vault-glyph kind-{vault.kind}">{KIND_GLYPH[vault.kind]}</span>
                <span class="vault-id">{vault.id}</span>
                <span class="vault-name">{vault.displayName || vault.shortLabel || ''}</span>
                {#if vault.updatedMs}
                  <span class="vault-age">{formatAge(vault.updatedMs)}</span>
                {/if}
                {#if conns.length > 0}
                  <span class="vault-refs" title={conns.join(', ')}>
                    ⟷{conns.length}
                  </span>
                {/if}
              </button>

              <!-- Expanded detail (Phase 2 click-to-expand) -->
              {#if isSelected}
                <div class="vault-detail">
                  {#if vault.path}
                    <div class="detail-row">
                      <span class="detail-label">PATH</span>
                      <span class="detail-value">{vault.path}</span>
                    </div>
                  {/if}
                  {#if conns.length > 0}
                    <div class="detail-row">
                      <span class="detail-label">LINKS</span>
                      <span class="detail-value detail-links">
                        {#each conns as ref}
                          <span
                            class="detail-link kind-{inferKind(ref)}"
                            role="button"
                            tabindex="0"
                            onclick={(e) => { e.stopPropagation(); selectedId = ref; }}
                            onkeydown={(e) => { if (e.key === 'Enter') { selectedId = ref; } }}
                          >{ref}</span>
                        {/each}
                      </span>
                    </div>
                  {/if}
                  {#if vault.updatedMs}
                    <div class="detail-row">
                      <span class="detail-label">MODIFIED</span>
                      <span class="detail-value">{new Date(vault.updatedMs).toLocaleString()}</span>
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
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    color: var(--amber-warm);
    user-select: none;
  }

  /* Header: title + count + search */
  .browser-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }
  .browser-title {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    color: var(--amber-dim);
  }
  .browser-count {
    font-size: 9px;
    font-weight: 700;
    color: var(--bg-base);
    background: var(--amber-dim);
    padding: 1px 5px;
    min-width: 18px;
    text-align: center;
  }
  .browser-search {
    flex: 1;
    min-width: 0;
    height: 22px;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 11px;
    padding: 0 8px;
    outline: none;
    transition: border-color 0.15s;
  }
  .browser-search::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .browser-search:focus {
    border-color: var(--amber-dim);
    box-shadow: 0 0 0 1px rgba(255, 168, 38, 0.15);
  }

  /* Scrollable body */
  .browser-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-color: var(--amber-faint) transparent;
    scrollbar-width: thin;
  }
  .browser-body::-webkit-scrollbar { width: 6px; }
  .browser-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* Loading / empty */
  .browser-loading, .browser-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px 12px;
    color: var(--amber-faint);
    font-size: 11px;
    font-style: italic;
  }
  .loading-glyph {
    font-size: 16px;
    font-style: normal;
    animation: pulse-glyph 1.6s ease-in-out infinite;
  }
  @keyframes pulse-glyph {
    0%, 100% { opacity: 0.3; text-shadow: none; }
    50%      { opacity: 1;   text-shadow: var(--glow-amber); }
  }

  /* Category header */
  .category-header {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 10px;
    background: var(--bg-surface);
    border: none;
    border-bottom: 1px solid var(--border-subtle);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    cursor: pointer;
    transition: color 0.12s, background 0.12s;
    text-align: left;
  }
  .category-header:hover {
    color: var(--amber-warm);
    background: var(--bg-hover);
  }
  .category-chevron {
    font-size: 10px;
    transition: transform 0.15s ease;
    display: inline-block;
    width: 12px;
    text-align: center;
  }
  .category-chevron.collapsed { transform: rotate(-90deg); }
  .category-glyph { font-size: 11px; }
  .category-label { flex: 1; }
  .category-count {
    font-size: 9px;
    color: var(--amber-faint);
    font-weight: 400;
  }

  /* Category body */
  .category-body {
    border-bottom: 1px solid var(--border-subtle);
  }

  /* Vault row */
  .vault-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 10px 4px 20px;
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 11px;
    cursor: pointer;
    text-align: left;
    transition: background 0.1s, border-color 0.1s, color 0.1s;
  }
  .vault-row:hover {
    background: var(--bg-hover);
    border-left-color: var(--amber-dim);
  }
  .vault-row.highlighted {
    background: rgba(255, 168, 38, 0.06);
    border-left-color: var(--amber-warm);
  }
  .vault-row.selected {
    background: var(--bg-hover);
    border-left-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .vault-row.dragging {
    opacity: 0.4;
  }

  /* State dot */
  .state-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .state-active    { background: var(--amber-bright); box-shadow: 0 0 6px var(--amber-bright); }
  .state-recent    { background: var(--amber-warm); }
  .state-ambient   { background: var(--amber-faint); }
  .state-background { background: var(--border-subtle); }

  /* Vault row elements */
  .vault-glyph {
    font-size: 10px;
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }
  .vault-id {
    font-weight: 600;
    color: var(--amber-primary);
    min-width: 48px;
    flex-shrink: 0;
  }
  .vault-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--amber-dim);
    font-size: 10px;
  }
  .vault-age {
    font-size: 9px;
    color: var(--amber-faint);
    font-style: italic;
    flex-shrink: 0;
  }
  .vault-refs {
    font-size: 9px;
    color: var(--amber-faint);
    flex-shrink: 0;
    padding: 0 4px;
    border: 1px solid var(--border-subtle);
    line-height: 14px;
  }

  /* Kind colors — applied via .kind-X on glyphs */
  .kind-p    { color: var(--amber-bright); }
  .kind-pr   { color: var(--amber-warm); }
  .kind-r    { color: var(--term-cyan); }
  .kind-s    { color: var(--term-blue); }
  .kind-lore { color: var(--term-purple); }
  .kind-agt  { color: var(--term-green); }
  .kind-h    { color: var(--amber-faint); }

  /* Selected vault detail */
  .vault-detail {
    padding: 6px 10px 8px 42px;
    background: var(--bg-surface);
    border-left: 2px solid var(--amber-dim);
    font-size: 10px;
  }
  .detail-row {
    display: flex;
    gap: 8px;
    padding: 2px 0;
    align-items: baseline;
  }
  .detail-label {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    min-width: 52px;
    flex-shrink: 0;
  }
  .detail-value {
    color: var(--amber-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .detail-links {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    white-space: normal;
  }
  .detail-link {
    padding: 1px 6px;
    border: 1px solid var(--border-subtle);
    cursor: pointer;
    font-size: 9px;
    transition: border-color 0.12s, background 0.12s;
  }
  .detail-link:hover {
    border-color: currentColor;
    background: rgba(255, 168, 38, 0.08);
  }

  /* Drag ghost */
  .drag-ghost {
    position: fixed;
    pointer-events: none;
    z-index: 5000;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    background: var(--bg-elevated);
    border: 1px solid var(--amber-dim);
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    color: var(--amber-bright);
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.5);
    opacity: 0.92;
  }
  .drag-ghost-glyph { font-size: 12px; }
</style>
