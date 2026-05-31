<script lang="ts">
  // TerminalGrid.svelte — flat, geometry-solving split-pane renderer.
  //
  // ── Why this is flat, not recursive ──────────────────────────────────────
  // The previous version rendered the SplitNode tree recursively: a leaf
  // rendered <Terminal> inside an `{#if node.type === 'terminal'}` branch, and
  // a split rendered two recursive <TerminalGrid> children in the `{:else}`.
  // When a pane was split, its node flipped from leaf → branch, so that
  // TerminalGrid instance switched `{#if}` → `{:else}` and Svelte DESTROYED the
  // running <Terminal> (→ pty_kill, session lost) and mounted fresh shells.
  // PTY lifetime was bound to a DOM position in a tree that restructures on
  // every split — the root cause of "split kills my session".
  //
  // This version decouples component identity from tree structure:
  //   • A pure solver walks the SplitNode tree and produces, per leaf, an
  //     absolute rect (in %) and, per split, a draggable splitter bar.
  //   • All <Terminal>s render in ONE flat `{#each leaves (leaf.id)}` list,
  //     keyed by pane id. Splitting only ADDS a new keyed entry; existing
  //     terminals keep their identity and are merely repositioned via CSS.
  //     Nothing remounts → the running session (process AND scrollback) is
  //     untouched. Terminal's ResizeObserver refits when its slot resizes.
  //
  // Ratios live in a local reactive map keyed by a structure-stable signature
  // (min leaf id of each child subtree), seeded from the SplitNode default and
  // persisted to localStorage so pane sizes survive reloads.

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
    /** Fired when a leaf pane's PTY exits. Carries the leaf's node ID. */
    onPtyExited?: (paneId: number) => void;
  }

  let {
    node,
    projectPath,
    focusedId = $bindable(),
    onSplit,
    onClose,
    onFocus,
    onPtyExited,
  }: Props = $props();

  // -------------------------------------------------------------------------
  // Geometry solver — pure walk of the SplitNode tree.
  // -------------------------------------------------------------------------

  interface Rect { x: number; y: number; w: number; h: number } // percentages
  interface LeafRect extends Rect { id: number }
  interface Bar {
    sig: string;
    orientation: 'vertical' | 'horizontal';
    pos: number;      // boundary position in % (x for vertical, y for horizontal)
    ratio: number;    // resolved ratio (for aria + keyboard)
    region: Rect;     // the region this bar divides, in %
  }

  /** Structure-stable signature for a split's ratio: the min leaf id of each
   *  child subtree. Stable across ratio changes and across splitting WITHIN a
   *  child (min id only ever stays or shrinks as the original leaf is kept). */
  function minLeaf(n: SplitNode): number {
    return n.type === 'terminal'
      ? n.id
      : Math.min(minLeaf(n.children[0]), minLeaf(n.children[1]));
  }

  function solve(
    n: SplitNode,
    rect: Rect,
    rmap: Record<string, number>,
    leaves: LeafRect[],
    bars: Bar[],
  ): void {
    if (n.type === 'terminal') {
      leaves.push({ id: n.id, ...rect });
      return;
    }
    const sig = `${minLeaf(n.children[0])}_${minLeaf(n.children[1])}`;
    const r = rmap[sig] ?? n.ratio ?? 0.5;
    if (n.type === 'vsplit') {
      const leftW = rect.w * r;
      bars.push({ sig, orientation: 'vertical', pos: rect.x + leftW, ratio: r, region: rect });
      solve(n.children[0], { x: rect.x, y: rect.y, w: leftW, h: rect.h }, rmap, leaves, bars);
      solve(n.children[1], { x: rect.x + leftW, y: rect.y, w: rect.w - leftW, h: rect.h }, rmap, leaves, bars);
    } else {
      const topH = rect.h * r;
      bars.push({ sig, orientation: 'horizontal', pos: rect.y + topH, ratio: r, region: rect });
      solve(n.children[0], { x: rect.x, y: rect.y, w: rect.w, h: topH }, rmap, leaves, bars);
      solve(n.children[1], { x: rect.x, y: rect.y + topH, w: rect.w, h: rect.h - topH }, rmap, leaves, bars);
    }
  }

  // Live ratio overrides (drag + persisted). Keyed by split signature.
  let ratios = $state<Record<string, number>>({});

  const solved = $derived.by(() => {
    const leaves: LeafRect[] = [];
    const bars: Bar[] = [];
    solve(node, { x: 0, y: 0, w: 100, h: 100 }, ratios, leaves, bars);
    return { leaves, bars };
  });

  // Seed each splitter ratio from localStorage exactly once. `seededSigs` is
  // plain (non-reactive) bookkeeping: the effect re-runs whenever `solved.bars`
  // changes (every drag re-solves), but each sig is probed only once, so there
  // is no per-mousemove localStorage churn and no reactive loop.
  const seededSigs = new Set<string>();
  $effect(() => {
    for (const b of solved.bars) {
      if (seededSigs.has(b.sig)) continue;
      seededSigs.add(b.sig);
      try {
        const saved = localStorage.getItem(`rift.split.ratio.${b.sig}`);
        if (saved !== null) {
          const p = parseFloat(saved);
          if (!Number.isNaN(p) && p > 0.05 && p < 0.95) {
            ratios = { ...ratios, [b.sig]: p };
          }
        }
      } catch { /* localStorage unavailable */ }
    }
  });

  function persistRatio(sig: string): void {
    try {
      const v = ratios[sig];
      if (v !== undefined) localStorage.setItem(`rift.split.ratio.${sig}`, String(v));
    } catch { /* quota / private mode */ }
  }

  // -------------------------------------------------------------------------
  // Splitter drag — maps pointer movement to the ratio of the divided region.
  // -------------------------------------------------------------------------

  let containerEl: HTMLDivElement | undefined = $state(undefined);
  let draggingSig = $state<string | null>(null);

  function onSplitterPointerDown(e: PointerEvent, bar: Bar): void {
    if (e.button !== 0 || !containerEl) return;
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    draggingSig = bar.sig;

    function onMove(ev: PointerEvent): void {
      if (!containerEl) return;
      const cr = containerEl.getBoundingClientRect();
      let newRatio: number;
      if (bar.orientation === 'vertical') {
        const start = cr.left + (bar.region.x / 100) * cr.width;
        const extent = (bar.region.w / 100) * cr.width;
        newRatio = extent > 0 ? (ev.clientX - start) / extent : 0.5;
      } else {
        const start = cr.top + (bar.region.y / 100) * cr.height;
        const extent = (bar.region.h / 100) * cr.height;
        newRatio = extent > 0 ? (ev.clientY - start) / extent : 0.5;
      }
      ratios = { ...ratios, [bar.sig]: Math.max(0.1, Math.min(0.9, newRatio)) };
    }

    function onUp(ev: PointerEvent): void {
      draggingSig = null;
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerup', onUp);
      if (target.hasPointerCapture(ev.pointerId)) target.releasePointerCapture(ev.pointerId);
      persistRatio(bar.sig);
    }

    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
  }

  function resetRatio(sig: string): void {
    ratios = { ...ratios, [sig]: 0.5 };
    persistRatio(sig);
  }

  function onSplitterKeydown(ev: KeyboardEvent, bar: Bar): void {
    const step = ev.shiftKey ? 0.1 : 0.02;
    const cur = ratios[bar.sig] ?? bar.ratio;
    let next = cur;
    if (bar.orientation === 'vertical') {
      if (ev.key === 'ArrowLeft') { ev.preventDefault(); next = cur - step; }
      else if (ev.key === 'ArrowRight') { ev.preventDefault(); next = cur + step; }
    } else {
      if (ev.key === 'ArrowUp') { ev.preventDefault(); next = cur - step; }
      else if (ev.key === 'ArrowDown') { ev.preventDefault(); next = cur + step; }
    }
    if (ev.key === 'Home') { ev.preventDefault(); next = 0.1; }
    if (ev.key === 'End') { ev.preventDefault(); next = 0.9; }
    if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); resetRatio(bar.sig); return; }
    next = Math.max(0.1, Math.min(0.9, next));
    if (next !== cur) {
      ratios = { ...ratios, [bar.sig]: next };
      persistRatio(bar.sig);
    }
  }

  // -------------------------------------------------------------------------
  // Focus — clicking anywhere in a pane focuses it.
  // -------------------------------------------------------------------------

  function handleFocusClick(id: number): void {
    focusedId = id;
    onFocus?.(id);
  }
</script>

<div class="grid-root" bind:this={containerEl}>
  <!-- All terminals render in one flat keyed list. Keying by pane id is the
       load-bearing fix: a given pane's <Terminal> instance persists across
       every split/close/resize, so its PTY is never killed and respawned. -->
  {#each solved.leaves as leaf (leaf.id)}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
    <div
      class="pane-leaf"
      class:focused={focusedId === leaf.id}
      style="left: {leaf.x}%; top: {leaf.y}%; width: {leaf.w}%; height: {leaf.h}%;"
      role="group"
      tabindex="0"
      aria-label="Terminal pane"
      onclick={() => handleFocusClick(leaf.id)}
      onkeydown={(ev) => {
        if ((ev.key === 'Enter' || ev.key === ' ') && ev.target === ev.currentTarget) {
          ev.preventDefault();
          handleFocusClick(leaf.id);
        }
      }}
    >
      <div class="pane-toolbar">
        <button type="button"
          class="pane-tool-btn"
          title="Split horizontal (top/bottom) — Ctrl+Shift+E"
          onclick={(e) => { e.stopPropagation(); onSplit(leaf.id, 'hsplit'); }}
        >⬓</button>
        <button type="button"
          class="pane-tool-btn"
          title="Split vertical (left/right) — Ctrl+Shift+D"
          onclick={(e) => { e.stopPropagation(); onSplit(leaf.id, 'vsplit'); }}
        >⬒</button>
        <button type="button"
          class="pane-tool-btn pane-tool-close"
          title="Close pane — Ctrl+Shift+W"
          aria-label="Close pane"
          onclick={(e) => { e.stopPropagation(); onClose(leaf.id); }}
        >✕</button>
      </div>
      <Terminal visible={true} {projectPath} onPtyExited={() => onPtyExited?.(leaf.id)} />
    </div>
  {/each}

  <!-- Splitter bars — positioned on each split boundary. Keyed by signature so
       they're stable across ratio changes; re-created only on structural change
       (never disturbing the terminals). -->
  {#each solved.bars as bar (bar.sig)}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="pane-splitter pane-splitter-{bar.orientation}"
      class:dragging={draggingSig === bar.sig}
      style={bar.orientation === 'vertical'
        ? `left: ${bar.pos}%; top: ${bar.region.y}%; height: ${bar.region.h}%;`
        : `top: ${bar.pos}%; left: ${bar.region.x}%; width: ${bar.region.w}%;`}
      role="separator"
      aria-orientation={bar.orientation}
      aria-valuenow={Math.round(bar.ratio * 100)}
      aria-valuemin={10}
      aria-valuemax={90}
      aria-label="{bar.orientation === 'vertical' ? 'Vertical' : 'Horizontal'} split — arrow keys to resize, double-click to reset"
      tabindex="0"
      onpointerdown={(e) => onSplitterPointerDown(e, bar)}
      ondblclick={() => resetRatio(bar.sig)}
      onkeydown={(ev) => onSplitterKeydown(ev, bar)}
    ></div>
  {/each}
</div>

<style>
  /* Positioning context for the absolutely-placed panes + splitters. */
  .grid-root {
    position: relative;
    flex: 1;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  /* Leaf pane — absolutely positioned to its solved rect. The amber border
     flickers on when this pane is focused. */
  .pane-leaf {
    position: absolute;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    /* Transparent border always present so layout doesn't shift on focus */
    border: 1px solid transparent;
    transition: border-color var(--duration-fast);
    box-sizing: border-box;
  }
  .pane-leaf.focused {
    border-color: rgba(255, 168, 38, 0.55);
  }

  /* Splitter bar — absolutely positioned, centered on the boundary line. */
  .pane-splitter {
    position: absolute;
    background: var(--border-subtle, #2a2520);
    transition: background var(--duration-base);
    user-select: none;
    -webkit-user-select: none;
    z-index: 2;
  }
  .pane-splitter:hover,
  .pane-splitter.dragging {
    background: rgba(255, 200, 64, 0.4); /* --amber-bright at 0.4 */
  }
  .pane-splitter:focus-visible {
    outline: 2px solid var(--amber-warm);
    outline-offset: -1px;
    background: rgba(255, 200, 64, 0.2);
  }
  .pane-splitter-vertical {
    width: 4px;
    cursor: col-resize;
    transform: translateX(-50%);
  }
  .pane-splitter-horizontal {
    height: 4px;
    cursor: row-resize;
    transform: translateY(-50%);
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
    transition: opacity var(--duration-med);
    pointer-events: none;
  }
  .pane-leaf:hover > .pane-toolbar,
  .pane-leaf.focused > .pane-toolbar {
    opacity: 1;
    pointer-events: auto;
  }
  .pane-tool-btn {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--text-base);
    line-height: 1;
    background: rgba(30, 26, 20, 0.85);
    color: var(--amber-dim, #D8A028);
    border: 1px solid var(--border-subtle, #2a2418);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast), color var(--duration-fast), border-color var(--duration-fast);
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
