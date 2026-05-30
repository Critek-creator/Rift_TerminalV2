<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';

  interface IndexNode {
    id: string;
    title: string;
    domain: string;
    floor: string;
    tags: string[];
    summary: string | null;
    status: string;
    modified: string;
  }

  interface IndexNodeFull extends IndexNode {
    body: string;
    created: string;
    links: string[];
  }

  interface IndexStats {
    total_nodes: number;
    total_links: number;
    unique_tags: number;
    by_domain: [string, number][];
    by_floor: [string, number][];
  }

  const DOMAIN_COLORS: Record<string, string> = {
    theories: 'var(--domain-theories)',
    projects: 'var(--domain-projects)',
    research: 'var(--domain-research)',
    personal: 'var(--domain-personal)',
    creative: 'var(--domain-creative)',
    systems: 'var(--domain-systems)',
    reference: 'var(--domain-reference)',
  };

  const FLOOR_LABELS: Record<string, string> = {
    B2: 'The Void',
    B1: 'Workbench',
    G: 'Lobby',
    '1': 'Library',
    '2': 'Gallery',
  };

  const FLOOR_ORDER = ['2', '1', 'G', 'B1', 'B2'];

  let nodes = $state<IndexNode[]>([]);
  let stats = $state<IndexStats | null>(null);
  let selectedNode = $state<IndexNodeFull | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let filterDomain = $state<string | null>(null);
  let filterFloor = $state<string | null>(null);
  let searchQuery = $state('');

  const filteredNodes = $derived.by(() => {
    let result = nodes;
    if (filterDomain) result = result.filter((n) => n.domain === filterDomain);
    if (filterFloor) result = result.filter((n) => n.floor === filterFloor);
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase().trim();
      result = result.filter(
        (n) =>
          n.title.toLowerCase().includes(q) ||
          n.tags.some((t) => t.toLowerCase().includes(q)) ||
          (n.summary?.toLowerCase().includes(q) ?? false)
      );
    }
    return result;
  });

  const groupedByFloor = $derived.by(() => {
    const groups: { floor: string; label: string; nodes: IndexNode[] }[] = [];
    for (const f of FLOOR_ORDER) {
      const floorNodes = filteredNodes.filter((n) => n.floor === f);
      if (floorNodes.length > 0) {
        groups.push({ floor: f, label: FLOOR_LABELS[f] || f, nodes: floorNodes });
      }
    }
    return groups;
  });

  const domains = $derived.by(() => {
    const set = new Set<string>();
    for (const n of nodes) set.add(n.domain);
    return Array.from(set).sort();
  });

  async function loadNodes() {
    loading = true;
    error = null;
    try {
      const [nodeList, statsResult] = await Promise.all([
        invoke<IndexNode[]>('index_list_nodes', { domain: null, floor: null, tags: null }),
        invoke<IndexStats>('index_get_stats'),
      ]);
      nodes = nodeList;
      stats = statsResult;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function searchNodes() {
    if (!searchQuery.trim()) {
      await loadNodes();
      return;
    }
    loading = true;
    try {
      nodes = await invoke<IndexNode[]>('index_search_nodes', {
        query: searchQuery.trim(),
        limit: 100,
      });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function selectNode(id: string) {
    try {
      selectedNode = await invoke<IndexNodeFull>('index_get_node', { id });
    } catch (e) {
      console.error('Failed to load node:', e);
    }
  }

  function closeDetail() {
    selectedNode = null;
  }

  function domainColor(domain: string): string {
    return DOMAIN_COLORS[domain] ?? 'var(--amber-faint)';
  }

  onMount(() => {
    loadNodes();
  });
</script>

<div class="content-browser">
  <!-- Header -->
  <div class="cb-header">
    <span class="cb-title">CONTENT</span>
    {#if stats}
      <span class="cb-count">{filteredNodes.length}/{stats.total_nodes}</span>
    {/if}
    <input
      class="cb-search"
      type="text"
      placeholder="search thoughts…"
      aria-label="search vault thoughts"
      bind:value={searchQuery}
      onkeydown={(e) => { if (e.key === 'Enter') searchNodes(); }}
    />
  </div>

  <!-- Filters -->
  <div class="cb-filters">
    <button type="button"
      class="filter-chip"
      class:active={!filterDomain}
      onclick={() => { filterDomain = null; }}
    >all</button>
    {#each domains as d (d)}
      <button type="button"
        class="filter-chip"
        class:active={filterDomain === d}
        style="--chip-color: {domainColor(d)}"
        onclick={() => { filterDomain = filterDomain === d ? null : d; }}
      >{d}</button>
    {/each}
  </div>

  <div class="cb-floor-filters">
    {#each FLOOR_ORDER as f (f)}
      <button type="button"
        class="floor-chip"
        class:active={filterFloor === f}
        onclick={() => { filterFloor = filterFloor === f ? null : f; }}
      >{f}</button>
    {/each}
  </div>

  <!-- Body -->
  <div class="cb-body" aria-busy={loading}>
    {#if loading}
      <div class="cb-loading">loading index…</div>
    {:else if error}
      <div class="cb-error">{error}</div>
    {:else if filteredNodes.length === 0}
      <div class="cb-empty">no nodes match filters</div>
    {:else}
      {#each groupedByFloor as group (group.floor)}
        <div class="floor-group">
          <div class="floor-header">
            <span class="floor-badge">{group.floor}</span>
            <span class="floor-label">{group.label}</span>
            <span class="floor-count">{group.nodes.length}</span>
          </div>
          {#each group.nodes as node (node.id)}
            <button type="button"
              class="node-row"
              class:selected={selectedNode?.id === node.id}
              onclick={() => selectNode(node.id)}
            >
              <span class="node-domain-dot" style="background: {domainColor(node.domain)}"></span>
              <span class="node-title">{node.title}</span>
              {#if node.tags.length > 0}
                <span class="node-tag-count" title={node.tags.join(', ')}>{node.tags.length}</span>
              {/if}
            </button>
          {/each}
        </div>
      {/each}
    {/if}
  </div>

  <!-- Detail Panel -->
  {#if selectedNode}
    <div class="detail-panel">
      <div class="detail-header">
        <span class="detail-title">{selectedNode.title}</span>
        <button type="button" class="detail-close" onclick={closeDetail} aria-label="Close detail">×</button>
      </div>
      <div class="detail-meta">
        <span class="detail-domain" style="color: {domainColor(selectedNode.domain)}">{selectedNode.domain}</span>
        <span class="detail-floor">Floor {selectedNode.floor}</span>
        <span class="detail-status">{selectedNode.status}</span>
      </div>
      {#if selectedNode.tags.length > 0}
        <div class="detail-tags">
          {#each selectedNode.tags as tag (tag)}
            <span class="detail-tag">{tag}</span>
          {/each}
        </div>
      {/if}
      {#if selectedNode.summary}
        <div class="detail-summary">{selectedNode.summary}</div>
      {/if}
      {#if selectedNode.links.length > 0}
        <div class="detail-links">
          <span class="detail-links-label">LINKS</span>
          {#each selectedNode.links as link (link)}
            <span class="detail-link">[[{link}]]</span>
          {/each}
        </div>
      {/if}
      <div class="detail-body">{selectedNode.body}</div>
    </div>
  {/if}
</div>

<style>
  .content-browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-base);
    font-family: var(--font-family);
    font-size: var(--text-base);
    color: var(--amber-warm);
    user-select: none;
  }

  .cb-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-sm) var(--space-md);
    background: var(--bg-surface);
    box-shadow: var(--sep-glow);
    flex-shrink: 0;
  }
  .cb-title {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    color: var(--term-cyan);
  }
  .cb-count {
    font-size: var(--text-2xs);
    font-weight: 700;
    color: var(--bg-base);
    background: var(--term-cyan);
    padding: 1px var(--space-sm);
    min-width: 18px;
    text-align: center;
  }
  .cb-search {
    flex: 1;
    min-width: 0;
    height: 22px;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    padding: 0 var(--space-8);
    outline: 2px solid transparent;
  }
  .cb-search::placeholder { color: var(--amber-faint); font-style: italic; }
  .cb-search:focus { border-color: var(--term-cyan); }

  .cb-filters, .cb-floor-filters {
    display: flex;
    gap: var(--space-xs);
    padding: var(--space-xs) var(--space-md);
    flex-wrap: wrap;
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .filter-chip, .floor-chip {
    padding: 2px var(--space-8);
    border: 1px solid var(--border-subtle);
    background: transparent;
    color: var(--amber-dim);
    font-family: inherit;
    font-size: var(--text-2xs);
    cursor: pointer;
    transition: all var(--duration-base);
  }
  .filter-chip:hover, .floor-chip:hover {
    border-color: var(--amber-dim);
    color: var(--amber-warm);
  }
  .filter-chip:focus-visible, .floor-chip:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .filter-chip.active {
    border-color: var(--chip-color, var(--term-cyan));
    color: var(--chip-color, var(--term-cyan));
    background: var(--bg-amber-tint);
  }
  .floor-chip.active {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
    background: var(--bg-amber-tint);
  }

  .cb-body {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    scrollbar-color: var(--amber-faint) transparent;
    scrollbar-width: thin;
  }
  .cb-body::-webkit-scrollbar { width: 6px; }
  .cb-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .cb-loading, .cb-error, .cb-empty {
    padding: var(--space-24);
    text-align: center;
    color: var(--amber-faint);
    font-style: italic;
    font-size: var(--text-sm);
  }
  .cb-error { color: var(--term-red); }

  .floor-group { box-shadow: var(--sep-depth); }
  .floor-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: 5px var(--space-md);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
  }
  .floor-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    padding: 1px var(--space-sm);
    border: 1px solid var(--amber-dim);
    color: var(--amber-bright);
    letter-spacing: 0.05em;
  }
  .floor-label {
    font-size: var(--text-2xs);
    color: var(--amber-dim);
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  .floor-count {
    margin-left: auto;
    font-size: var(--text-2xs);
    color: var(--amber-faint);
  }

  .node-row {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    width: 100%;
    padding: var(--space-xs) var(--space-md) var(--space-xs) var(--space-xl);
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    text-align: left;
    transition: all var(--duration-fast);
  }
  .node-row:hover {
    background: var(--bg-hover);
    border-left-color: var(--amber-dim);
  }
  .node-row.selected {
    background: var(--bg-hover);
    border-left-color: var(--term-cyan);
    color: var(--term-cyan);
  }
  .node-row:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .node-domain-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .node-title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .node-tag-count {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    padding: 0 var(--space-xs);
    border: 1px solid var(--border-subtle);
  }

  .detail-panel {
    flex-shrink: 0;
    max-height: 45%;
    overflow-y: auto;
    padding: var(--space-md) var(--space-12);
    background: var(--bg-panel, var(--bg-surface));
    border-top: 1px solid var(--term-cyan);
    scrollbar-color: var(--amber-faint) transparent;
    scrollbar-width: thin;
  }
  .detail-panel::-webkit-scrollbar { width: 6px; }
  .detail-panel::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .detail-header {
    display: flex;
    align-items: center;
    margin-bottom: var(--space-8);
  }
  .detail-title {
    flex: 1;
    font-size: var(--text-md);
    font-weight: 700;
    color: var(--term-cyan);
  }
  .detail-close {
    background: none;
    border: none;
    color: var(--amber-dim);
    font-size: var(--text-xl);
    cursor: pointer;
    padding: 2px var(--space-sm);
    font-family: inherit;
  }
  .detail-close:hover { color: var(--amber-bright); }
  .detail-close:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: 1px; }

  .detail-meta {
    display: flex;
    gap: var(--space-md);
    font-size: var(--text-xs);
    margin-bottom: var(--space-sm);
  }
  .detail-floor { color: var(--amber-dim); }
  .detail-status { color: var(--amber-faint); font-style: italic; }

  .detail-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    margin-bottom: var(--space-8);
  }
  .detail-tag {
    padding: 1px var(--space-sm);
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
    font-size: var(--text-2xs);
  }

  .detail-summary {
    color: var(--amber-warm);
    font-size: var(--text-sm);
    font-style: italic;
    margin-bottom: var(--space-8);
    padding: var(--space-sm) var(--space-8);
    background: rgba(111, 224, 224, 0.06);
    border-left: 2px solid var(--term-cyan);
  }

  .detail-links {
    margin-bottom: var(--space-8);
    font-size: var(--text-xs);
  }
  .detail-links-label {
    display: block;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    margin-bottom: var(--space-xs);
  }
  .detail-link {
    display: inline-block;
    padding: 1px var(--space-sm);
    margin: 0 3px 3px 0;
    border: 1px solid var(--term-cyan);
    color: var(--term-cyan);
    font-size: var(--text-2xs);
  }

  .detail-body {
    font-size: var(--text-sm);
    color: var(--amber-dim);
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 200px;
    overflow-y: auto;
  }
</style>
