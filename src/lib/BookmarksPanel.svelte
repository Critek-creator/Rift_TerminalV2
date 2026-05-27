<script lang="ts">
  /**
   * BookmarksPanel — persistent state panel for the BusTail tab.
   * Shows bookmarked events and annotated events in a single filterable
   * list. Includes a toggle between "Bookmarks" and "Saved Filters" views.
   */

  import { onMount, onDestroy } from 'svelte';
  import type { Category, Envelope } from './bus';
  import { bookmarkStore, savedQueryStore, type SavedQuery } from './busBookmarks';
  import { annotationStore, TAG_META, type Annotation } from './busAnnotations';

  interface Props {
    /** Callback to apply a saved query as the active filter. */
    onApplyQuery?: (query: SavedQuery) => void;
    /** Callback to scroll/jump to an event in the log. */
    onJumpToEvent?: (env: Envelope) => void;
  }

  let { onApplyQuery, onJumpToEvent }: Props = $props();

  type ViewMode = 'bookmarks' | 'filters';
  let viewMode = $state<ViewMode>('bookmarks');

  // Reactive store snapshots — re-read on store change.
  let bookmarks = $state(bookmarkStore.getAll());
  let annotations = $state(annotationStore.getAll());
  let savedQueries = $state(savedQueryStore.getAll());

  let searchText = $state('');

  // Subscribe to store changes for reactivity.
  let unsubBookmarks: (() => void) | undefined;
  let unsubAnnotations: (() => void) | undefined;
  let unsubQueries: (() => void) | undefined;

  onMount(() => {
    unsubBookmarks = bookmarkStore.onChange(() => {
      bookmarks = bookmarkStore.getAll();
    });
    unsubAnnotations = annotationStore.onChange(() => {
      annotations = annotationStore.getAll();
    });
    unsubQueries = savedQueryStore.onChange(() => {
      savedQueries = savedQueryStore.getAll();
    });
  });

  onDestroy(() => {
    unsubBookmarks?.();
    unsubAnnotations?.();
    unsubQueries?.();
  });

  /** Merged list of bookmarked + annotated events (deduped by envelope key). */
  const mergedItems = $derived.by(() => {
    const seen = new Set<string>();
    const items: Array<{
      id: string;
      envelope: Envelope;
      annotation: Annotation | null;
      isBookmarked: boolean;
      sortTs: number;
    }> = [];

    // Bookmarks first.
    for (const bm of bookmarks) {
      seen.add(bm.envelopeId);
      items.push({
        id: bm.envelopeId,
        envelope: bm.envelope,
        annotation: annotationStore.get(bm.envelopeId),
        isBookmarked: true,
        sortTs: bm.createdAt,
      });
    }

    // Annotated events not already bookmarked.
    for (const ann of annotations) {
      if (seen.has(ann.envelopeId)) continue;
      items.push({
        id: ann.envelopeId,
        envelope: ann.envelope,
        annotation: ann,
        isBookmarked: false,
        sortTs: ann.createdAt,
      });
    }

    return items.sort((a, b) => b.sortTs - a.sortTs);
  });

  /** Filtered items based on search text. */
  const filteredItems = $derived.by(() => {
    if (!searchText.trim()) return mergedItems;
    const q = searchText.toLowerCase();
    return mergedItems.filter((item) => {
      const ann = item.annotation;
      return (
        item.envelope.kind.toLowerCase().includes(q) ||
        item.envelope.category.toLowerCase().includes(q) ||
        (ann && ann.note.toLowerCase().includes(q)) ||
        (ann && ann.tags.some((t) => t.includes(q)))
      );
    });
  });

  const CAT_COLOR: Record<Category, string> = {
    pty:      'var(--term-white)',
    hook:     'var(--term-cyan)',
    agent:    'var(--term-purple)',
    fs:       'var(--amber-faint)',
    index:    'var(--status-blue-bright, #6CB6FF)',
    aegis:    'var(--amber-primary)',
    status:   'var(--amber-bright)',
    system:   'var(--term-red)',
    mcp:      'var(--term-purple, #C58FFF)',
    sentinel: 'var(--term-red)',
  };

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  function handleUnbookmark(id: string): void {
    bookmarkStore.unbookmark(id);
  }

  function handleRemoveQuery(id: string): void {
    savedQueryStore.remove(id);
  }

  // ----- New query form -----
  let showNewQueryForm = $state(false);
  let newQueryName = $state('');
  let newQueryKind = $state('');
  let newQueryPayload = $state('');

  function handleSaveNewQuery(): void {
    const name = newQueryName.trim();
    if (!name) return;
    savedQueryStore.save({
      name,
      kindPattern: newQueryKind.trim() || undefined,
      payloadPattern: newQueryPayload.trim() || undefined,
    });
    newQueryName = '';
    newQueryKind = '';
    newQueryPayload = '';
    showNewQueryForm = false;
  }
</script>

<div class="bookmarks-panel" aria-label="Bookmarks panel">
  <div class="panel-tabs" role="tablist" aria-label="Bookmark views">
    <button
      type="button"
      class="panel-tab"
      class:active={viewMode === 'bookmarks'}
      role="tab"
      aria-selected={viewMode === 'bookmarks'}
      onclick={() => (viewMode = 'bookmarks')}
    >
      BOOKMARKS
      {#if mergedItems.length > 0}
        <span class="badge">{mergedItems.length}</span>
      {/if}
    </button>
    <button
      type="button"
      class="panel-tab"
      class:active={viewMode === 'filters'}
      role="tab"
      aria-selected={viewMode === 'filters'}
      onclick={() => (viewMode = 'filters')}
    >
      SAVED FILTERS
      {#if savedQueries.length > 0}
        <span class="badge">{savedQueries.length}</span>
      {/if}
    </button>
  </div>

  {#if viewMode === 'bookmarks'}
    <div class="panel-search">
      <input
        type="text"
        class="search-input"
        placeholder="Filter bookmarks..."
        aria-label="filter bookmarks"
        bind:value={searchText}
      />
    </div>

    <div class="panel-body">
      {#if filteredItems.length === 0}
        <div class="empty-state">
          {#if mergedItems.length === 0}
            No bookmarks or annotations yet. Click the star or pencil icon on any event to get started.
          {:else}
            No matches for "{searchText}".
          {/if}
        </div>
      {:else}
        {#each filteredItems as item (item.id)}
          <div class="bm-row">
            <div class="bm-row-main">
              {#if item.isBookmarked}
                <button
                  type="button"
                  class="bm-icon star active"
                  title="Remove bookmark"
                  onclick={() => handleUnbookmark(item.id)}
                >*</button>
              {:else}
                <span class="bm-icon dot"></span>
              {/if}
              <span class="bm-ts">{formatTs(item.envelope.ts)}</span>
              <span class="bm-cat" style="color: {CAT_COLOR[item.envelope.category]}">
                {item.envelope.category}
              </span>
              <span class="bm-kind">{item.envelope.kind}</span>
              {#if onJumpToEvent}
                <button
                  type="button"
                  class="bm-jump"
                  title="Jump to event"
                  onclick={() => onJumpToEvent?.(item.envelope)}
                >-></button>
              {/if}
            </div>
            {#if item.annotation}
              <div class="bm-annotation">
                {#if item.annotation.tags.length > 0}
                  <span class="bm-tags">
                    {#each item.annotation.tags as tag (tag)}
                      <span
                        class="bm-tag"
                        style="color: {TAG_META[tag].cssVar}; border-color: {TAG_META[tag].cssVar}"
                      >{TAG_META[tag].label}</span>
                    {/each}
                  </span>
                {/if}
                {#if item.annotation.note}
                  <span class="bm-note">{item.annotation.note}</span>
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      {/if}
    </div>

  {:else}
    <!-- Saved Filters view -->
    <div class="panel-body">
      {#each savedQueries as query (query.id)}
        <div class="sq-row">
          <span class="sq-name">{query.name}</span>
          <span class="sq-meta">
            {#if query.categories}
              {query.categories.join(', ')}
            {/if}
            {#if query.kindPattern}
              kind:{query.kindPattern}
            {/if}
            {#if query.payloadPattern}
              payload:{query.payloadPattern}
            {/if}
            {#if query.minSeverity}
              >={query.minSeverity}
            {/if}
          </span>
          <span class="sq-actions">
            {#if onApplyQuery}
              <button
                type="button"
                class="rift-btn rift-btn--ghost rift-btn--sm"
                onclick={() => onApplyQuery?.(query)}
              >APPLY</button>
            {/if}
            {#if query.createdAt > 0}
              <button
                type="button"
                class="rift-btn rift-btn--danger rift-btn--sm"
                onclick={() => handleRemoveQuery(query.id)}
              >X</button>
            {/if}
          </span>
        </div>
      {/each}

      {#if showNewQueryForm}
        <div class="sq-form">
          <input
            type="text"
            class="search-input"
            placeholder="Query name..."
            bind:value={newQueryName}
          />
          <input
            type="text"
            class="search-input"
            placeholder="Kind pattern (optional)..."
            bind:value={newQueryKind}
          />
          <input
            type="text"
            class="search-input"
            placeholder="Payload pattern (optional)..."
            bind:value={newQueryPayload}
          />
          <div class="sq-form-actions">
            <button
              type="button"
              class="rift-btn rift-btn--ghost rift-btn--sm"
              onclick={() => (showNewQueryForm = false)}
            >CANCEL</button>
            <button
              type="button"
              class="rift-btn rift-btn--primary rift-btn--sm"
              onclick={handleSaveNewQuery}
              disabled={!newQueryName.trim()}
            >SAVE</button>
          </div>
        </div>
      {:else}
        <button
          type="button"
          class="rift-btn rift-btn--sm add-query-btn"
          onclick={() => (showNewQueryForm = true)}
        >+ NEW QUERY</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .bookmarks-panel {
    display: flex;
    flex-direction: column;
    max-height: 100%;
    overflow: hidden;
  }

  .panel-tabs {
    display: flex;
    box-shadow: var(--sep-depth);
    background: var(--bg-surface);
  }
  .panel-tab {
    flex: 1;
    padding: var(--space-sm) var(--space-12);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
    transition: color var(--duration-base) ease-out, border-color var(--duration-base) ease-out;
  }
  .panel-tab:hover {
    color: var(--amber-warm);
  }
  .panel-tab.active {
    color: var(--amber-bright);
    border-bottom-color: var(--amber-bright);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 16px;
    height: var(--space-14);
    padding: 0 var(--space-xs);
    background: rgba(255, 168, 38, 0.15);
    border-radius: 7px;
    font-size: var(--text-2xs);
    font-weight: 700;
    color: var(--amber-bright);
  }

  .panel-search {
    padding: var(--space-sm) var(--space-md);
    box-shadow: var(--sep-depth);
  }
  .search-input {
    width: 100%;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    padding: var(--space-xs) var(--space-8);
    line-height: 1.4;
  }
  .search-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .search-input:focus {
    outline: 2px solid transparent;
    border-color: var(--amber-dim);
  }

  .panel-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-sm) var(--space-md) var(--space-md);
  }

  .empty-state {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    font-style: italic;
    padding: var(--space-8) var(--space-xs);
    line-height: 1.5;
  }

  /* Bookmark rows */
  .bm-row {
    padding: var(--space-xs)0;
    border-bottom: 1px solid rgba(42, 36, 24, 0.5);
  }
  .bm-row:last-child {
    border-bottom: none;
  }
  .bm-row-main {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-xs);
  }
  .bm-icon {
    flex-shrink: 0;
    width: 14px;
    text-align: center;
  }
  .bm-icon.star {
    background: transparent;
    border: none;
    color: var(--amber-bright);
    font-family: var(--font-family);
    font-size: var(--text-base);
    cursor: pointer;
    padding: 0;
    line-height: 1;
  }
  .bm-icon.star:hover {
    color: var(--term-red);
  }
  .bm-icon.dot::after {
    content: '';
    display: inline-block;
    width: 6px;
    height: 6px;
    background: var(--amber-dim);
    border-radius: 50%;
  }
  .bm-ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-2xs);
    flex-shrink: 0;
  }
  .bm-cat {
    font-weight: 700;
    font-size: var(--text-2xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    flex-shrink: 0;
  }
  .bm-kind {
    color: var(--amber-warm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .bm-jump {
    flex-shrink: 0;
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    cursor: pointer;
    padding: 0 2px;
  }
  .bm-jump:hover {
    color: var(--amber-bright);
  }

  .bm-annotation {
    margin-left: var(--space-xl);
    margin-top: 2px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .bm-tags {
    display: flex;
    gap: var(--space-xs);
    flex-wrap: wrap;
  }
  .bm-tag {
    display: inline-flex;
    padding: 0 var(--space-xs);
    border: 1px solid;
    border-radius: 2px;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    line-height: 14px;
  }
  .bm-note {
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    font-style: italic;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }

  /* Saved query rows */
  .sq-row {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-xs) 0;
    border-bottom: 1px solid rgba(42, 36, 24, 0.5);
  }
  .sq-row:last-child {
    border-bottom: none;
  }
  .sq-name {
    color: var(--amber-warm);
    font-size: var(--text-xs);
    font-weight: 600;
    flex-shrink: 0;
  }
  .sq-meta {
    flex: 1;
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sq-actions {
    display: flex;
    gap: var(--space-xs);
    flex-shrink: 0;
  }

  .sq-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    margin-top: var(--space-sm);
    padding: var(--space-8);
    background: rgba(212, 137, 10, 0.05);
    border: 1px dashed var(--border-subtle);
    border-radius: var(--radius-sm);
  }
  .sq-form-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-xs);
    margin-top: var(--space-xs);
  }
  .add-query-btn {
    margin-top: var(--space-sm);
    width: 100%;
  }
</style>
