<script lang="ts">
  import { sectionCatalog } from './sectionCatalog.svelte';
  import { popouts } from './popouts.svelte';

  interface PaletteEntry {
    id: string;
    label: string;
    icon: string;
    category: 'tab' | 'action' | 'shortcut';
    action?: () => void;
  }

  interface Props {
    onclose: () => void;
    onActivateNotif: (id: string) => void;
  }

  let { onclose, onActivateNotif }: Props = $props();

  let query = $state('');
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
    );

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
    if (cat === 'shortcut') return 'SHORTCUTS';
    return cat.toUpperCase();
  }

</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="palette-backdrop" onclick={onclose} onkeydown={onKeydown}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="palette-panel" onclick={(e) => e.stopPropagation()}>
    <input
      bind:this={inputEl}
      bind:value={query}
      onkeydown={onKeydown}
      placeholder="Search tabs, actions, shortcuts…"
      spellcheck="false"
      autocomplete="off"
    />
    <div class="results">
      {#each filtered as entry, i}
        {@const showHeader = i === 0 || filtered[i - 1]?.category !== entry.category}
        {#if showHeader}
          <div class="category-header">{categoryLabel(entry.category)}</div>
        {/if}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="entry"
          class:selected={i === selectedIdx}
          class:actionable={entry.category !== 'shortcut'}
          role="option"
          tabindex="-1"
          aria-selected={i === selectedIdx}
          onclick={() => { if (entry.action) entry.action(); }}
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
        <div class="empty">No matches</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .palette-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(0, 0, 0, 0.55);
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
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.7);
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
    font-size: 14px;
    padding: 12px 16px;
    outline: none;
  }
  input::placeholder {
    color: var(--amber-faint, #A87830);
    opacity: 0.6;
  }

  .results {
    overflow-y: auto;
    padding: 4px 0;
  }

  .category-header {
    padding: 8px 16px 4px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--amber-faint, #A87830);
    text-transform: uppercase;
  }

  .entry {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 16px;
    cursor: default;
    font-size: 13px;
    color: var(--term-white, #E8E4D8);
  }
  .entry.actionable {
    cursor: pointer;
  }
  .entry.selected {
    background: rgba(255, 168, 38, 0.10);
  }
  .entry:hover {
    background: rgba(255, 168, 38, 0.08);
  }

  .entry-icon {
    width: 20px;
    text-align: center;
    font-size: 14px;
    flex-shrink: 0;
  }

  .entry-label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry-badge {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--amber-faint, #A87830);
    border: 1px solid var(--amber-faint, #A87830);
    border-radius: 3px;
    padding: 1px 5px;
  }

  .empty {
    padding: 16px;
    text-align: center;
    color: var(--amber-faint, #A87830);
    font-size: 12px;
  }
</style>
