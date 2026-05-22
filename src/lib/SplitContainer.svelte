<script module lang="ts">
  export type SplitDirection = 'horizontal' | 'vertical';

  export interface SplitNode {
    id: string;
    type: 'leaf' | 'branch';
    direction?: SplitDirection;
    children?: [SplitNode, SplitNode];
    size: number;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    root: SplitNode;
    activePaneId: string;
    onFocus: (paneId: string) => void;
    onResize: (nodeId: string, newSize: number) => void;
    paneContent: Snippet<[string]>;
  }

  let { root, activePaneId, onFocus, onResize, paneContent }: Props = $props();

  const MIN_PANE_SIZE = 120;
  let containerEl: HTMLDivElement | undefined = $state(undefined);
  let dragging = $state<{ nodeId: string; startPos: number; direction: SplitDirection } | null>(null);

  function handleMouseDown(e: MouseEvent, nodeId: string, direction: SplitDirection) {
    e.preventDefault();
    dragging = { nodeId, startPos: direction === 'horizontal' ? e.clientX : e.clientY, direction };
    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);
  }

  function handleMouseMove(e: MouseEvent) {
    if (!dragging || !containerEl) return;
    const containerRect = containerEl.getBoundingClientRect();
    const totalSize = dragging.direction === 'horizontal' ? containerRect.width : containerRect.height;
    const pos = dragging.direction === 'horizontal' ? e.clientX : e.clientY;
    const containerStart = dragging.direction === 'horizontal' ? containerRect.left : containerRect.top;
    const relativePos = pos - containerStart;
    const minPct = (MIN_PANE_SIZE / totalSize) * 100;
    const newSize = Math.max(minPct, Math.min(100 - minPct, (relativePos / totalSize) * 100));
    onResize(dragging.nodeId, newSize);
  }

  function handleMouseUp() {
    dragging = null;
    window.removeEventListener('mousemove', handleMouseMove);
    window.removeEventListener('mouseup', handleMouseUp);
  }
</script>

<div class="split-container" bind:this={containerEl}>
  {#snippet renderNode(node: SplitNode)}
    {#if node.type === 'leaf'}
      <div
        class="split-pane"
        class:active={node.id === activePaneId}
        style="flex-basis: {node.size}%;"
        role="group"
        aria-label="Terminal pane {node.id}"
        onclick={() => onFocus(node.id)}
        onkeydown={(e) => { if (e.key === 'Enter') onFocus(node.id); }}
      >
        <div class="pane-content">
          {@render paneContent(node.id)}
        </div>
      </div>
    {:else if node.children}
      <div
        class="split-branch"
        class:horizontal={node.direction === 'horizontal'}
        class:vertical={node.direction === 'vertical'}
        style="flex-basis: {node.size}%;"
      >
        {@render renderNode(node.children[0])}
        <div
          class="resize-handle"
          class:handle-horizontal={node.direction === 'horizontal'}
          class:handle-vertical={node.direction === 'vertical'}
          role="separator"
          aria-orientation={node.direction}
          tabindex="0"
          onmousedown={(e) => handleMouseDown(e, node.id, node.direction!)}
        ></div>
        {@render renderNode(node.children[1])}
      </div>
    {/if}
  {/snippet}

  {@render renderNode(root)}
</div>

<style>
  .split-container {
    width: 100%;
    height: 100%;
    display: flex;
    overflow: hidden;
  }

  .split-branch {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .split-branch.horizontal {
    flex-direction: row;
  }

  .split-branch.vertical {
    flex-direction: column;
  }

  .split-pane {
    position: relative;
    min-width: 120px;
    min-height: 120px;
    overflow: hidden;
    border: 1px solid var(--border-subtle);
    transition: border-color var(--duration-fast) var(--ease-out);
  }

  .split-pane.active {
    border-color: var(--amber-primary);
    box-shadow: inset 0 0 0 1px var(--amber-primary);
  }

  .pane-content {
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  .resize-handle {
    flex-shrink: 0;
    background: var(--border-subtle);
    transition: background var(--duration-fast) var(--ease-out);
    z-index: 10;
  }

  .resize-handle:hover,
  .resize-handle:focus-visible {
    background: var(--amber-dim);
  }

  .handle-horizontal {
    width: 4px;
    cursor: col-resize;
  }

  .handle-vertical {
    height: 4px;
    cursor: row-resize;
  }
</style>
