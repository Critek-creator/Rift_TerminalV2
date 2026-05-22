<script lang="ts">
  import { keybindings, categoryLabels, categoryOrder, type Keybinding } from './keybindings';

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  const grouped = $derived.by(() => {
    const map = new Map<Keybinding['category'], Keybinding[]>();
    for (const cat of categoryOrder) {
      map.set(cat, keybindings.filter((k) => k.category === cat));
    }
    return map;
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }
</script>

<div class="overlay-backdrop" role="presentation" onclick={onclose} onkeydown={onKeydown}>
  <div class="overlay-panel" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()}>
    <header class="overlay-header">
      <span class="header-icon">⌨</span>
      <span class="header-title">KEYBOARD SHORTCUTS</span>
      <button type="button" class="close-btn" onclick={onclose}>ESC</button>
    </header>
    <div class="overlay-body">
      {#each categoryOrder as cat}
        {@const bindings = grouped.get(cat) ?? []}
        {#if bindings.length > 0}
          <div class="category-section">
            <div class="category-label">{categoryLabels[cat]}</div>
            {#each bindings as kb (kb.key)}
              <div class="binding-row">
                <kbd class="key">{kb.key}</kbd>
                <span class="desc">{kb.description}</span>
              </div>
            {/each}
          </div>
        {/if}
      {/each}
    </div>
  </div>
</div>

<style>
  .overlay-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .overlay-panel {
    width: min(480px, 85vw);
    max-height: 70vh;
    background: var(--bg-surface, #1a1814);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: var(--radius-md, 6px);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.7);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .overlay-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    background: var(--bg-elevated, #1e1a14);
  }

  .header-icon {
    font-size: 14px;
    color: var(--amber-bright, #FFC840);
    text-shadow: var(--glow-amber-faint);
  }

  .header-title {
    flex: 1;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.14em;
    color: var(--amber-bright, #FFC840);
  }

  .close-btn {
    background: transparent;
    border: 1px solid var(--amber-faint, #A87830);
    color: var(--amber-faint, #A87830);
    font-family: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    padding: 2px 8px;
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s;
  }
  .close-btn:hover {
    color: var(--amber-bright, #FFC840);
    border-color: var(--amber-bright, #FFC840);
  }

  .overlay-body {
    overflow-y: auto;
    padding: 8px 0 12px;
  }
  .overlay-body::-webkit-scrollbar { width: 5px; }
  .overlay-body::-webkit-scrollbar-thumb { background: var(--amber-faint, #A87830); }

  .category-section {
    padding: 0 16px;
  }
  .category-section + .category-section {
    margin-top: 12px;
  }

  .category-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--amber-faint, #A87830);
    text-transform: uppercase;
    padding: 6px 0 4px;
    border-bottom: 1px solid rgba(168, 120, 48, 0.15);
    margin-bottom: 4px;
  }

  .binding-row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 5px 0;
  }

  .key {
    display: inline-block;
    min-width: 110px;
    text-align: right;
    background: var(--bg-elevated, #1e1a14);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: 3px;
    color: var(--amber-primary, #FFA826);
    font-family: inherit;
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    white-space: nowrap;
  }

  .desc {
    color: var(--term-white, #E8E4D8);
    font-size: 12px;
  }
</style>
