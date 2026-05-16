<script lang="ts">
  // Splitter.svelte — Phase 8.7e — draggable pane divider with localStorage size persistence.
  //
  // Usage:
  //   <Splitter direction="vertical" storageKey="rift.cockpit.right_width_px"
  //             unit="px" bind:size={rightWidth} min={280} max={800} />
  //   <Splitter direction="horizontal" storageKey="rift.cockpit.graph_height_pct"
  //             unit="percent" bind:size={graphPct} min={25} max={75} />
  //
  // Direction names follow the bar's geometry, NOT the resize axis:
  //   "horizontal" → bar runs horizontally → divides top/bottom siblings → drag adjusts HEIGHT
  //   "vertical"   → bar runs vertically   → divides left/right siblings → drag adjusts WIDTH
  //
  // The parent owns the size state via $bindable; this component only handles the drag
  // gesture, clamps min/max, persists on drag-end. Parent is responsible for applying
  // the size to a sibling's flex-basis (or equivalent) inline style.

  import { onMount } from 'svelte';

  interface Props {
    direction: 'horizontal' | 'vertical';
    storageKey: string;
    /** Bound by the parent — single source of truth for the pane size. */
    size?: number;
    /** Unit of `size`. 'percent' = 0-100, 'px' = absolute pixels. */
    unit?: 'percent' | 'px';
    /** Minimum size in the same unit. */
    min?: number;
    /** Maximum size in the same unit. */
    max?: number;
    /** Optional double-click handler — typical use is "reset to default". */
    onDblClick?: () => void;
  }

  let {
    direction,
    storageKey,
    size = $bindable(50),
    unit = 'percent',
    min = 10,
    max = 90,
    onDblClick,
  }: Props = $props();

  let dragging = $state(false);
  let splitterEl: HTMLDivElement;

  // Drag-state captured on mousedown so onPointerMove can compute deltas
  // against a stable reference rather than chasing the live `size` value.
  let startCoord = 0;
  let startSize = 0;
  let containerExtent = 0; // parent width or height in px (only relevant for percent unit)

  function clamp(s: number): number {
    return Math.max(min, Math.min(max, s));
  }

  onMount(() => {
    try {
      const saved = localStorage.getItem(storageKey);
      if (saved !== null) {
        const parsed = parseFloat(saved);
        if (!Number.isNaN(parsed)) {
          size = clamp(parsed);
        }
      }
    } catch {
      // localStorage may be inaccessible (private browsing / quota); silent fallback
      // to the parent-provided default.
    }
  });

  function onPointerDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging = true;
    splitterEl.setPointerCapture(e.pointerId);

    const parent = splitterEl.parentElement;
    if (parent) {
      const rect = parent.getBoundingClientRect();
      containerExtent = direction === 'vertical' ? rect.width : rect.height;
    }
    startCoord = direction === 'vertical' ? e.clientX : e.clientY;
    startSize = size;

    document.addEventListener('pointermove', onPointerMove);
    document.addEventListener('pointerup', onPointerUp);
  }

  function onPointerMove(e: PointerEvent): void {
    if (!dragging) return;
    const coord = direction === 'vertical' ? e.clientX : e.clientY;
    const deltaPx = coord - startCoord;
    let next: number;
    if (unit === 'percent') {
      const deltaPct = containerExtent > 0 ? (deltaPx / containerExtent) * 100 : 0;
      next = startSize + deltaPct;
    } else {
      next = startSize + deltaPx;
    }
    size = clamp(next);
  }

  function onPointerUp(e: PointerEvent): void {
    dragging = false;
    document.removeEventListener('pointermove', onPointerMove);
    document.removeEventListener('pointerup', onPointerUp);
    if (splitterEl?.hasPointerCapture(e.pointerId)) {
      splitterEl.releasePointerCapture(e.pointerId);
    }
    try {
      localStorage.setItem(storageKey, String(size));
    } catch {
      // Quota / private browsing — non-fatal.
    }
  }
</script>

<div
  bind:this={splitterEl}
  class="splitter splitter-{direction}"
  class:dragging
  onpointerdown={onPointerDown}
  ondblclick={onDblClick}
  role="separator"
  aria-orientation={direction === 'vertical' ? 'vertical' : 'horizontal'}
  tabindex="-1"
></div>

<style>
  .splitter {
    background: var(--border-subtle);
    flex-shrink: 0;
    transition: background 0.12s;
    user-select: none;
    -webkit-user-select: none;
    position: relative;
    z-index: 2;
  }
  .splitter:hover,
  .splitter.dragging {
    background: var(--amber-primary, #FFA826);
  }
  .splitter-horizontal {
    height: 4px;
    width: 100%;
    cursor: row-resize;
  }
  .splitter-vertical {
    width: 4px;
    height: 100%;
    cursor: col-resize;
  }
  /* Larger invisible hit area for easier grabbing without changing visual width. */
  .splitter::before {
    content: '';
    position: absolute;
    inset: -3px 0;
  }
  .splitter-vertical::before {
    inset: 0 -3px;
  }
</style>
