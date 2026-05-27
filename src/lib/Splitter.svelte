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
    persist();
  }

  function persist(): void {
    try {
      localStorage.setItem(storageKey, String(size));
    } catch {
      // Quota / private browsing — non-fatal.
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    const step = unit === 'percent' ? 2 : 20;
    const posKeys = direction === 'vertical' ? ['ArrowRight', 'ArrowUp'] : ['ArrowDown', 'ArrowRight'];
    const negKeys = direction === 'vertical' ? ['ArrowLeft', 'ArrowDown'] : ['ArrowUp', 'ArrowLeft'];
    if (posKeys.includes(e.key)) {
      e.preventDefault();
      size = clamp(size + step);
      persist();
    } else if (negKeys.includes(e.key)) {
      e.preventDefault();
      size = clamp(size - step);
      persist();
    } else if (e.key === 'Home') {
      e.preventDefault();
      size = min;
      persist();
    } else if (e.key === 'End') {
      e.preventDefault();
      size = max;
      persist();
    }
  }
</script>

<div
  bind:this={splitterEl}
  class="splitter splitter-{direction}"
  class:dragging
  onpointerdown={onPointerDown}
  onkeydown={onKeydown}
  ondblclick={onDblClick}
  role="separator"
  aria-orientation={direction === 'vertical' ? 'vertical' : 'horizontal'}
  aria-valuenow={Math.round(size)}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-label="Resize"
  tabindex="0"
></div>

<style>
  .splitter {
    background: var(--border-subtle);
    flex-shrink: 0;
    transition: background var(--duration-base);
    user-select: none;
    -webkit-user-select: none;
    position: relative;
    z-index: 2;
  }
  .splitter:hover,
  .splitter.dragging {
    background: var(--amber-primary, #FFA826);
  }
  .splitter:focus-visible {
    background: var(--amber-primary, #FFA826);
    outline: 1px solid var(--amber-warm);
    outline-offset: -1px;
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
  /* Grip indicator on hover — centered amber line for grab affordance. */
  .splitter:hover::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    border-radius: var(--radius-xs);
  }
  .splitter-vertical:hover::after {
    width: 2px;
    height: var(--space-24);
    background: var(--amber-primary, #FFA826);
  }
  .splitter-horizontal:hover::after {
    width: 24px;
    height: 2px;
    background: var(--amber-primary, #FFA826);
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
