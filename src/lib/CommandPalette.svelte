<script lang="ts">
  import { fade } from 'svelte/transition';
  import { sectionCatalog } from './sectionCatalog.svelte';
  import { popouts } from './popouts.svelte';
  import { llmModels } from './llmModels.svelte';

  interface PaletteEntry {
    id: string;
    label: string;
    icon: string;
    category: 'tab' | 'action' | 'shortcut' | 'model';
    action?: () => void;
  }

  interface Props {
    onclose: () => void;
    onActivateNotif: (id: string) => void;
    initialQuery?: string;
  }

  let { onclose, onActivateNotif, initialQuery = '' }: Props = $props();

  // svelte-ignore state_referenced_locally
  let query = $state(initialQuery);
  let selectedIdx = $state(0);
  let inputEl: HTMLInputElement = $state(undefined!);

  const entries = $derived.by((): PaletteEntry[] => {
    const items: PaletteEntry[] = [];

    for (const tab of sectionCatalog.allTabs) {
      items.push({
        id: `tab:${tab.id}`,
        label: tab.title,
        icon: tab.icon,
        category: 'tab',
        action: () => { onActivateNotif(tab.id); onclose(); },
      });
    }

    items.push(
      { id: 'act:settings', label: 'Open Settings', icon: '⚙', category: 'action',
        action: () => { popouts.summon({ content: { kind: 'settings' } }); onclose(); } },
      { id: 'act:notif-manager', label: 'Notification Manager', icon: '◫', category: 'action',
        action: () => { onclose(); } },
      { id: 'act:llm-chat', label: 'Router Prompt', icon: '◆', category: 'action',
        action: () => { popouts.summon({ content: { kind: 'llm-chat' }, width: 'min(720px, 85vw)' }); onclose(); } },
      { id: 'act:llm-ensemble', label: 'Ensemble Compare', icon: '⊞', category: 'action',
        action: () => { popouts.summon({ content: { kind: 'llm-ensemble' }, width: 'min(1100px, 95vw)' }); onclose(); } },
    );

    if (llmModels.enabled) {
      const statusIcon = (id: string) => {
        const s = llmModels.processStatus[id];
        return s === 'running' ? '●' : s === 'starting' ? '◐' : s === 'error' ? '✕' : '○';
      };
      for (const m of llmModels.models) {
        const isActive = llmModels.activeModelId === m.id;
        items.push({
          id: `model:${m.id}`,
          label: `${m.short_id || '???'}  ${m.display_name || m.model_identifier}${isActive ? '  (active)' : ''}`,
          icon: statusIcon(m.id),
          category: 'model',
          action: () => { llmModels.setActiveModel(m.id); onclose(); },
        });
      }
    }

    items.push(
      { id: 'key:search', label: 'Ctrl+Shift+F — Search terminal', icon: '⌕', category: 'shortcut' },
      { id: 'key:zoom-in', label: 'Ctrl+= — Zoom in', icon: '⊕', category: 'shortcut' },
      { id: 'key:zoom-out', label: 'Ctrl+- — Zoom out', icon: '⊖', category: 'shortcut' },
      { id: 'key:zoom-reset', label: 'Ctrl+0 — Reset zoom', icon: '⊙', category: 'shortcut' },
      { id: 'key:cockpit', label: 'Ctrl+B — Toggle cockpit', icon: '⊞', category: 'shortcut' },
      { id: 'key:new-tab', label: 'Ctrl+Shift+T — New session', icon: '⊕', category: 'shortcut' },
      { id: 'key:close-tab', label: 'Ctrl+Shift+W — Close session', icon: '⊗', category: 'shortcut' },
      { id: 'key:copy', label: 'Ctrl+C — Copy (with selection)', icon: '⊡', category: 'shortcut' },
      { id: 'key:paste', label: 'Ctrl+V — Paste', icon: '⊟', category: 'shortcut' },
    );

    return items;
  });

  const filtered = $derived.by(() => {
    if (!query.trim()) return entries;
    const q = query.toLowerCase();
    return entries.filter((e) =>
      e.label.toLowerCase().includes(q) ||
      e.category.includes(q)
    );
  });

  $effect(() => {
    void filtered.length;
    selectedIdx = 0;
  });

  $effect(() => {
    if (inputEl) inputEl.focus();
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIdx = Math.min(selectedIdx + 1, filtered.length - 1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIdx = Math.max(selectedIdx - 1, 0);
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      const entry = filtered[selectedIdx];
      if (entry?.action) entry.action();
      return;
    }
  }

  function categoryLabel(cat: string): string {
    if (cat === 'tab') return 'TABS';
    if (cat === 'action') return 'ACTIONS';
    if (cat === 'model') return 'MODELS';
    if (cat === 'shortcut') return 'SHORTCUTS';
    return cat.toUpperCase();
  }

</script>

<div class="palette-backdrop" role="presentation" onclick={onclose} onkeydown={onKeydown} transition:fade={{ duration: 150 }}>
  <div class="palette-panel" role="dialog" aria-label="Command palette" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onKeydown}>
    <input
      bind:this={inputEl}
      bind:value={query}
      onkeydown={onKeydown}
      placeholder="Search tabs, actions, shortcuts…"
      role="combobox"
      aria-label="Search commands and shortcuts"
      aria-expanded={filtered.length > 0}
      aria-controls="palette-listbox"
      aria-activedescendant={filtered.length > 0 ? `palette-option-${selectedIdx}` : undefined}
      aria-autocomplete="list"
      spellcheck="false"
      autocomplete="off"
    />
    <div class="results" id="palette-listbox" role="listbox" aria-label="Commands">
      {#each filtered as entry, i}
        {@const showHeader = i === 0 || filtered[i - 1]?.category !== entry.category}
        {#if showHeader}
          <div class="category-header" role="presentation">{categoryLabel(entry.category)}</div>
        {/if}
        <div
          id="palette-option-{i}"
          class="entry"
          class:selected={i === selectedIdx}
          class:actionable={entry.category !== 'shortcut'}
          role="option"
          tabindex="-1"
          aria-selected={i === selectedIdx}
          onclick={() => { if (entry.action) entry.action(); }}
          onkeydown={(e) => { if ((e.key === 'Enter' || e.key === ' ') && entry.action) { e.preventDefault(); entry.action(); } }}
          onmouseenter={() => { selectedIdx = i; }}
        >
          <span class="entry-icon">{entry.icon}</span>
          <span class="entry-label">{entry.label}</span>
          {#if entry.category === 'tab'}
            <span class="entry-badge">tab</span>
          {/if}
        </div>
      {/each}
      {#if filtered.length === 0}
        <div class="empty" role="status">No matches</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .palette-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: var(--backdrop-overlay);
    display: flex;
    justify-content: center;
    padding-top: 80px;
  }

  .palette-panel {
    width: min(520px, 85vw);
    max-height: 420px;
    background: var(--bg-surface, #1a1814);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: var(--radius-md, 6px);
    box-shadow: var(--shadow-overlay);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  input {
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    color: var(--term-white, #E8E4D8);
    font-family: 'JetBrains Mono', monospace;
    font-size: var(--text-lg);
    padding: var(--space-12) var(--space-lg);
    outline: 2px solid transparent;
  }
  input::placeholder {
    color: var(--amber-faint, #A87830);
    opacity: 0.6;
  }

  .results {
    overflow-y: auto;
    padding: var(--space-xs)0;
  }

  .category-header {
    padding: var(--space-8) var(--space-lg) var(--space-xs);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--amber-faint, #A87830);
    text-transform: uppercase;
  }

  .entry {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    cursor: default;
    font-size: var(--text-md);
    color: var(--term-white, #E8E4D8);
  }
  .entry.actionable {
    cursor: pointer;
  }
  .entry.selected {
    background: var(--bg-amber-selected);
  }
  .entry:hover {
    background: var(--bg-amber-hover);
  }

  .entry-icon {
    width: 20px;
    text-align: center;
    font-size: var(--text-lg);
    flex-shrink: 0;
  }

  .entry-label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--amber-faint, #A87830);
    border: 1px solid var(--amber-faint, #A87830);
    border-radius: var(--radius-sm);
    padding: 1px 5px;
  }

  .empty {
    padding: var(--space-lg);
    text-align: center;
    color: var(--amber-faint, #A87830);
    font-size: var(--text-base);
  }
</style>
