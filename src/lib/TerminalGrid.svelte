<script lang="ts">
  // TerminalGrid.svelte — recursive split-pane layout renderer.
  //
  // Walks a SplitNode tree and renders:
  //   leaf  → <Terminal> wrapped in a focus-click div
  //   split → two recursive <TerminalGrid> children separated by an inline
  //           splitter bar. Ratio is owned locally (float 0-1) and stored to
  //           localStorage so pane sizes survive reloads.
  //
  // No backend changes are required — each leaf spawns its own PTY via
  // Terminal.svelte's onMount → pty_start. The grid is purely a layout concern.

  import Terminal from './Terminal.svelte';
  import type { SplitNode } from './splitTypes';

  interface Props {
    node: SplitNode;
    projectPath: string | null;
    focusedId: number;
    /** Notify App that the user wants to split this terminal. */
    onSplit: (id: number, direction: 'hsplit' | 'vsplit') => void;
    /** Notify App that the user wants to close this pane. */
    onClose: (id: number) => void;
    /** Propagate focus changes up to App so shortcuts know the target. */
    onFocus?: (id: number) => void;
  }

  let {
    node,
    projectPath,
    focusedId = $bindable(),
    onSplit,
    onClose,
    onFocus,
  }: Props = $props();

  // -------------------------------------------------------------------------
  // Inline splitter state for branch nodes.
  // Ratio lives here (not in the SplitNode) so mutations don't trigger full
  // tree re-evaluation in App.svelte. Each branch instance persists its own
  // ratio to localStorage using a stable key derived from the children's ids.
  // On first render the ratio from the SplitNode is used as the default.
  // -------------------------------------------------------------------------

  /** Resolved ratio for THIS node (only relevant when node is a split). */
  let ratio = $state(node.type !== 'terminal' ? node.ratio : 0.5);

  // Drag state for the inline splitter bar.
  let dragging = $state(false);
  let splitterEl: HTMLDivElement | undefined = $state(undefined);
  let containerEl: HTMLDivElement | undefined = $state(undefined);
  let startCoord = 0;
  let startRatio = 0;

  // Storage key — stable as long as the leaf ids under this split don't
  // change (they don't; each leaf gets a monotonically increasing id).
  const storageKey = $derived(
    node.type !== 'terminal'
      ? `rift.split.ratio.${node.children[0].type === 'terminal' ? node.children[0].id : 'x'}_${node.children[1].type === 'terminal' ? node.children[1].id : 'y'}`
      : ''
  );

  // Restore persisted ratio on mount when this is a split node.
  $effect(() => {
    if (node.type === 'terminal' || !storageKey) return;
    try {
      const saved = localStorage.getItem(storageKey);
      if (saved !== null) {
        const parsed = parseFloat(saved);
        if (!Number.isNaN(parsed) && parsed > 0.05 && parsed < 0.95) {
          ratio = parsed;
        }
      }
    } catch {
      // localStorage unavailable — use node.ratio default.
    }
  });

  // -------------------------------------------------------------------------
  // Inline splitter drag logic.
  // Uses pointer capture so the drag works even when the mouse leaves the bar.
  // -------------------------------------------------------------------------

  function onSplitterPointerDown(e: PointerEvent): void {
    if (e.button !== 0 || node.type === 'terminal' || !containerEl || !splitterEl) return;
    e.preventDefault();
    dragging = true;
    splitterEl.setPointerCapture(e.pointerId);

    const rect = containerEl.getBoundingClientRect();
    const extent = node.type === 'vsplit' ? rect.width : rect.height;
    startCoord = node.type === 'vsplit' ? e.clientX : e.clientY;
    startRatio = ratio;

    function onMove(ev: PointerEvent): void {
      if (!dragging) return;
      const coord = node.type === 'vsplit' ? ev.clientX : ev.clientY;
      const delta = coord - startCoord;
      const rectNow = containerEl!.getBoundingClientRect();
      const extentNow = node.type === 'vsplit' ? rectNow.width : rectNow.height;
      const newRatio = startRatio + delta / (extentNow > 0 ? extentNow : extent);
      ratio = Math.max(0.1, Math.min(0.9, newRatio));
    }

    function onUp(ev: PointerEvent): void {
      dragging = false;
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerup', onUp);
      if (splitterEl?.hasPointerCapture(ev.pointerId)) {
        splitterEl.releasePointerCapture(ev.pointerId);
      }
      try {
        if (storageKey) localStorage.setItem(storageKey, String(ratio));
      } catch { /* quota / private mode */ }
    }

    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
  }

  function onSplitterDblClick(): void {
    ratio = 0.5;
    try {
      if (storageKey) localStorage.setItem(storageKey, '0.5');
    } catch { /* quota / private mode */ }
  }

  // -------------------------------------------------------------------------
  // Focus handling — clicking anywhere in a terminal pane focuses it.
  // -------------------------------------------------------------------------

  function handleFocusClick(id: number): void {
    focusedId = id;
    onFocus?.(id);
  }
</script>

{#if node.type === 'terminal'}
  <!-- Leaf: single terminal pane -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="pane-leaf"
    class:focused={focusedId === node.id}
    onclick={() => handleFocusClick(node.id)}
  >
    <div class="pane-toolbar">
      <button
        class="pane-tool-btn"
        title="Split horizontal (top/bottom) — Ctrl+Shift+E"
        onclick={(e) => { e.stopPropagation(); onSplit(node.id, 'hsplit'); }}
      >⬓</button>
      <button
        class="pane-tool-btn"
        title="Split vertical (left/right) — Ctrl+Shift+D"
        onclick={(e) => { e.stopPropagation(); onSplit(node.id, 'vsplit'); }}
      >⬒</button>
      <button
        class="pane-tool-btn pane-tool-close"
        title="Close pane — Ctrl+Shift+W"
        onclick={(e) => { e.stopPropagation(); onClose(node.id); }}
      >✕</button>
    </div>
    <Terminal visible={true} {projectPath} />
  </div>
{:else}
  <!-- Branch: two children separated by a draggable bar -->
  <div
    bind:this={containerEl}
    class="pane-split pane-{node.type}"
  >
    <div
      class="pane-child"
      style="flex: {ratio} 1 0%; min-{node.type === 'vsplit' ? 'width' : 'height'}: 0;"
    >
      <svelte:self
        node={node.children[0]}
        {projectPath}
        bind:focusedId
        {onSplit}
        {onClose}
        {onFocus}
      />
    </div>

    <!-- Inline splitter bar -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      bind:this={splitterEl}
      class="pane-splitter pane-splitter-{node.type === 'vsplit' ? 'vertical' : 'horizontal'}"
      class:dragging
      role="separator"
      aria-orientation={node.type === 'vsplit' ? 'vertical' : 'horizontal'}
      tabindex="-1"
      onpointerdown={onSplitterPointerDown}
      ondblclick={onSplitterDblClick}
    ></div>

    <div
      class="pane-child"
      style="flex: {1 - ratio} 1 0%; min-{node.type === 'vsplit' ? 'width' : 'height'}: 0;"
    >
      <svelte:self
        node={node.children[1]}
        {projectPath}
        bind:focusedId
        {onSplit}
        {onClose}
        {onFocus}
      />
    </div>
  </div>
{/if}

<style>
  /* Leaf pane — wraps a single Terminal. Takes all available space in its
     flex slot. The amber border flickers on when this pane is focused. */
  .pane-leaf {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    /* Transparent border always present so layout doesn't shift on focus */
    border: 1px solid transparent;
    transition: border-color 0.1s;
    box-sizing: border-box;
  }

  .pane-leaf.focused {
    border-color: rgba(255, 168, 38, 0.55);
  }

  /* Branch pane — flex container for two children + splitter bar. */
  .pane-split {
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  /* vsplit: children side by side (left | right) */
  .pane-vsplit {
    flex-direction: row;
  }

  /* hsplit: children stacked (top / bottom) */
  .pane-hsplit {
    flex-direction: column;
  }

  /* Each child fills its ratio slice. flex set inline. */
  .pane-child {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* Inline splitter bar — 4px, amber-tinted on hover/drag. */
  .pane-splitter {
    flex-shrink: 0;
    background: var(--border-subtle, #2a2520);
    transition: background 0.12s;
    user-select: none;
    -webkit-user-select: none;
    position: relative;
    z-index: 2;
  }

  .pane-splitter:hover,
  .pane-splitter.dragging {
    background: rgba(255, 191, 0, 0.45);
  }

  .pane-splitter-vertical {
    width: 4px;
    height: 100%;
    cursor: col-resize;
  }

  .pane-splitter-horizontal {
    height: 4px;
    width: 100%;
    cursor: row-resize;
  }

  /* Extended hit area without changing visual size. */
  .pane-splitter::before {
    content: '';
    position: absolute;
    inset: -3px 0;
  }
  .pane-splitter-vertical::before {
    inset: 0 -3px;
  }

  /* Pane toolbar — hover-reveal split/close buttons in top-right of leaf. */
  .pane-toolbar {
    position: absolute;
    top: 4px;
    right: 4px;
    z-index: 5;
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 0.15s;
    pointer-events: none;
  }
  .pane-leaf:hover > .pane-toolbar,
  .pane-leaf.focused > .pane-toolbar {
    opacity: 1;
    pointer-events: auto;
  }
  .pane-leaf {
    position: relative;
  }
  .pane-tool-btn {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    line-height: 1;
    background: rgba(30, 26, 20, 0.85);
    color: var(--amber-dim, #A87830);
    border: 1px solid var(--border-subtle, #2a2520);
    border-radius: 3px;
    cursor: pointer;
    transition: background 0.1s, color 0.1s, border-color 0.1s;
    padding: 0;
    font-family: inherit;
  }
  .pane-tool-btn:hover {
    background: rgba(50, 42, 30, 0.95);
    color: var(--amber-bright, #FFC840);
    border-color: var(--amber-faint, #A87830);
  }
  .pane-tool-close:hover {
    color: var(--term-red, #FF4848);
    border-color: var(--term-red, #FF4848);
  }
</style>
