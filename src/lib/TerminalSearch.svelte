<script lang="ts">
  import type { SearchAddon } from '@xterm/addon-search';

  interface Props {
    searchAddon: SearchAddon;
    onclose: () => void;
  }

  let { searchAddon, onclose }: Props = $props();

  let query = $state('');
  let caseSensitive = $state(false);
  let regex = $state(false);
  let inputEl: HTMLInputElement = $state(undefined!);

  function doSearch(direction: 'next' | 'prev') {
    if (!query) {
      return;
    }
    const opts = { caseSensitive, regex };
    if (direction === 'next') {
      searchAddon.findNext(query, opts);
    } else {
      searchAddon.findPrevious(query, opts);
    }
  }

  function onInput() {
    if (!query) {
      searchAddon.clearDecorations();
      return;
    }
    doSearch('next');
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      doSearch(e.shiftKey ? 'prev' : 'next');
      return;
    }
  }

  function close() {
    searchAddon.clearDecorations();
    query = '';
    onclose();
  }

  $effect(() => {
    if (inputEl) {
      inputEl.focus();
    }
  });
</script>

<div class="search-bar" role="search">
  <input
    bind:this={inputEl}
    bind:value={query}
    oninput={onInput}
    onkeydown={onKeydown}
    placeholder="Search terminal…"
    spellcheck="false"
    autocomplete="off"
  />
  <button
    class="toggle"
    class:active={caseSensitive}
    title="Case sensitive"
    onclick={() => { caseSensitive = !caseSensitive; onInput(); }}
  >Aa</button>
  <button
    class="toggle"
    class:active={regex}
    title="Regex"
    onclick={() => { regex = !regex; onInput(); }}
  >.*</button>
  <button class="nav" title="Previous (Shift+Enter)" onclick={() => doSearch('prev')}>&#x25B2;</button>
  <button class="nav" title="Next (Enter)" onclick={() => doSearch('next')}>&#x25BC;</button>
  <button class="close" title="Close (Esc)" onclick={close}>&#x2715;</button>
</div>

<style>
  .search-bar {
    position: absolute;
    top: 4px;
    right: 12px;
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 2px;
    background: var(--bg-surface, #1a1814);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: var(--radius-md, 6px);
    padding: 3px 4px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
  }

  input {
    background: transparent;
    border: none;
    color: var(--term-white, #E8E4D8);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    width: 200px;
    padding: 3px 6px;
    outline: none;
  }
  input::placeholder {
    color: var(--amber-faint, #A87830);
    opacity: 0.7;
  }

  button {
    background: transparent;
    border: 1px solid transparent;
    color: var(--amber-faint, #A87830);
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    padding: 2px 5px;
    cursor: pointer;
    border-radius: 3px;
    line-height: 1;
  }
  button:hover {
    color: var(--amber-bright, #FFC840);
    background: rgba(255, 168, 38, 0.08);
  }

  .toggle.active {
    color: var(--amber-bright, #FFC840);
    border-color: var(--amber-bright, #FFC840);
    background: rgba(255, 200, 64, 0.12);
  }

  .close:hover {
    color: var(--term-red, #FF4848);
  }

  .nav {
    font-size: 9px;
    padding: 2px 4px;
  }
</style>
