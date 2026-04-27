<script module lang="ts">
  // Tree.svelte — Phase 6.3
  //
  // Module script: exports the TreeNode type so consumers (App.svelte, Phase 8
  // graph pane) can import it without importing the component itself.

  /** TypeScript mirror of the Rust `TreeNode` struct (camelCase via serde). */
  export interface TreeNode {
    path: string;
    name: string;
    isDir: boolean;
    children: TreeNode[];
  }
</script>

<script lang="ts">
  // Instance script: runes-based component logic.
  //
  // Node-based SVG filesystem tree. Fetches a static [`TreeNode`] snapshot
  // from the Rust backend via `fs_tree` on mount, then subscribes to live
  // `Category::Fs` envelopes to update activity state + mutate the structure
  // on create/delete/rename events.
  //
  // Layout: simple hierarchical depth-first layout.
  //   • L0 (root) at x = ROOT_X.  Each level adds X_STEP.
  //   • Rows spaced ROW_H px apart; row counter increments depth-first.
  //   • Per-node: L-shaped parent→child edge, node shape (circle=file,
  //     rounded-rect=dir), glyph (▾/▶ for dirs), label.
  //
  // Visual state classes (ambient | recent | active | background) are read
  // from `treeActivity` and applied reactively via `$derived`.
  //
  // Phase 6.3 additions:
  //   • Per-directory collapse state (collapsedDirs Set, toggleCollapse).
  //   • Layout walk skips subtrees of collapsed dirs.
  //   • Aggregate glow + hasPinnedDescendant for collapsed dirs (sub-walk,
  //     capped at MAX_BUBBLE_DEPTH levels below the collapsed dir).
  //   • Reactive ▾/▶ glyph via collapsedDirs read in render.
  //   • Click routing: dir click → toggleCollapse, file click → treeActivity.cycle.

  import { invoke } from '@tauri-apps/api/core';
  import { subscribe, type Envelope } from './bus';
  import { treeActivity, type ActivityState } from './treeActivity.svelte';
  import { popouts } from './popouts.svelte';

  // ---------------------------------------------------------------------------
  // Layout constants
  // ---------------------------------------------------------------------------

  const ROOT_X = 16;
  const X_STEP = 22;
  const ROW_H = 22;
  const SVG_WIDTH = 320;
  const PADDING_BOTTOM = 12;

  // Node geometry
  const DIR_RX = 3; // rounded-rect corner radius
  const DIR_W = 10;
  const DIR_H = 9;
  const FILE_R = 4.5; // circle radius

  /**
   * Maximum recursion depth for the aggregate sub-walk on collapsed dirs.
   * Prevents pathological bubble-up on deep/noisy project trees.
   * 6 levels covers the overwhelming majority of real project layouts.
   */
  const MAX_BUBBLE_DEPTH = 6;

  // ---------------------------------------------------------------------------
  // Props — bindable outputs for App.svelte's pane header
  // ---------------------------------------------------------------------------

  interface Props {
    /** Reactive total node count — bound by App.svelte for pane header. */
    nodeCount?: number;
    /** Root name label — bound by App.svelte for pane header. */
    watchedPathLabel?: string;
  }

  let {
    nodeCount = $bindable(0),
    watchedPathLabel = $bindable('…'),
  }: Props = $props();

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let treeRoot = $state<TreeNode | null>(null);
  let fetchError = $state<string | null>(null);

  /**
   * Set of directory paths currently collapsed by the user.
   * Empty by default (all dirs expanded). Svelte 5 Set reactivity requires
   * assign-replace on mutation — see toggleCollapse.
   */
  let collapsedDirs = $state(new Set<string>());

  // ---------------------------------------------------------------------------
  // Collapse helpers (design call A)
  // ---------------------------------------------------------------------------

  /** Toggle collapse state for a directory path. Assign-replace for Svelte 5 reactivity. */
  function toggleCollapse(path: string): void {
    const next = new Set(collapsedDirs);
    if (next.has(path)) next.delete(path); else next.add(path);
    collapsedDirs = next;
  }

  // ---------------------------------------------------------------------------
  // Aggregate helpers for collapsed dirs (design call C)
  // ---------------------------------------------------------------------------

  /**
   * Compute the synthetic ActivityState for a collapsed directory from its
   * aggregate glow and pinned-descendant flag.
   * Pinned > any glow > ambient, mirroring the file visual hierarchy.
   */
  function aggregateStateFromGlow(glow: number, hasPinned: boolean): ActivityState {
    if (hasPinned) return 'active';   // any pinned descendant → dir reads as active
    if (glow > 0)  return 'recent';   // any decaying glow → dir reads as recent
    return 'ambient';
  }

  /**
   * Walk descendants of a collapsed dir (not emitted to layout) to compute:
   *   aggregateGlow    — max glowIntensity across all reachable descendants.
   *   hasPinnedDesc    — true if any descendant has state === 'active'.
   *
   * Stops descending after MAX_BUBBLE_DEPTH levels to bound cost on deep trees.
   * Out-of-tree activity (paths not in snapshot) is simply absent — no crash.
   */
  function computeAggregate(
    node: TreeNode,
    depthRemaining: number,
  ): { aggregateGlow: number; hasPinnedDesc: boolean } {
    let aggregateGlow = 0;
    let hasPinnedDesc = false;

    if (depthRemaining <= 0) return { aggregateGlow, hasPinnedDesc };

    for (const child of node.children) {
      const entry = treeActivity.getEntry(child.path);
      if (entry.glowIntensity > aggregateGlow) aggregateGlow = entry.glowIntensity;
      if (entry.state === 'active') hasPinnedDesc = true;

      if (child.isDir && child.children.length > 0) {
        const sub = computeAggregate(child, depthRemaining - 1);
        if (sub.aggregateGlow > aggregateGlow) aggregateGlow = sub.aggregateGlow;
        if (sub.hasPinnedDesc) hasPinnedDesc = true;
      }

      // Short-circuit: can't improve beyond maximum values.
      if (hasPinnedDesc && aggregateGlow >= 1.0) break;
    }

    return { aggregateGlow, hasPinnedDesc };
  }

  // ---------------------------------------------------------------------------
  // Flat layout — computed reactively from treeRoot + treeActivity.snapshot
  //               + collapsedDirs (design calls B, C)
  // ---------------------------------------------------------------------------

  interface LayoutNode {
    node: TreeNode;
    x: number;
    y: number;
    parentX: number | null;
    parentY: number | null;
    /** Non-null only for collapsed dirs: max descendant glow [0,1]. */
    aggregateGlow: number | null;
    /** Non-null only for collapsed dirs: synthetic state driven by descendants. */
    aggregateState: ActivityState | null;
  }

  const layout = $derived.by((): LayoutNode[] => {
    // Read snapshot + collapsedDirs so this derived re-runs when either changes.
    void treeActivity.snapshot;
    void collapsedDirs;

    if (!treeRoot) return [];
    const rows: LayoutNode[] = [];
    let row = 0;

    function walk(
      node: TreeNode,
      depth: number,
      parentX: number | null,
      parentY: number | null,
    ) {
      const x = ROOT_X + depth * X_STEP;
      const y = ROW_H / 2 + row * ROW_H;
      const isCollapsed = node.isDir && collapsedDirs.has(node.path);

      let aggregateGlow: number | null = null;
      let aggState: ActivityState | null = null;

      if (isCollapsed) {
        // Pre-compute aggregate so render doesn't re-walk on every paint.
        const agg = computeAggregate(node, MAX_BUBBLE_DEPTH);
        aggregateGlow = agg.aggregateGlow;
        aggState = aggregateStateFromGlow(agg.aggregateGlow, agg.hasPinnedDesc);
      }

      rows.push({ node, x, y, parentX, parentY, aggregateGlow, aggregateState: aggState });
      row++;

      // Skip children of collapsed dirs (design call B).
      if (!isCollapsed) {
        for (const child of node.children) {
          walk(child, depth + 1, x, y);
        }
      }
    }

    walk(treeRoot, 0, null, null);
    return rows;
  });

  const svgHeight = $derived(layout.length * ROW_H + PADDING_BOTTOM);

  const derivedNodeCount = $derived.by(() => {
    if (!treeRoot) return 0;
    function count(n: TreeNode): number {
      return 1 + n.children.reduce((s, c) => s + count(c), 0);
    }
    return count(treeRoot);
  });

  // watchedPathLabel: root node name displayed in the pane header.
  // The Tauri command uses cwd — Phase 6.7 will expose the full path via config.
  const derivedWatchedPathLabel = $derived(treeRoot?.name ?? '…');

  // Push derived values up to the bindable props so App.svelte's pane header
  // stays in sync without polling or additional stores.
  $effect(() => {
    nodeCount = derivedNodeCount;
    watchedPathLabel = derivedWatchedPathLabel;
  });

  // ---------------------------------------------------------------------------
  // Tree mutation helpers (called by the fs envelope handler)
  // ---------------------------------------------------------------------------

  function findParent(root: TreeNode, relPath: string): TreeNode | null {
    const segments = relPath.split('/');
    if (segments.length <= 1) return root;
    const parentPath = segments.slice(0, -1).join('/');

    function search(node: TreeNode): TreeNode | null {
      if (node.path === parentPath) return node;
      for (const child of node.children) {
        const found = search(child);
        if (found) return found;
      }
      return null;
    }
    return search(root);
  }

  function removeNode(root: TreeNode, path: string): TreeNode {
    return {
      ...root,
      children: root.children
        .filter((c) => c.path !== path)
        .map((c) => removeNode(c, path)),
    };
  }

  function insertNode(root: TreeNode, newNode: TreeNode): TreeNode {
    const parent = findParent(root, newNode.path);
    if (!parent) return root;

    const existingIdx = parent.children.findIndex((c) => c.path === newNode.path);
    const updated = [...parent.children];
    if (existingIdx >= 0) {
      updated[existingIdx] = newNode;
    } else {
      updated.push(newNode);
      // Re-sort: dirs first, then files, alphabetical.
      updated.sort((a, b) => {
        if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
        return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
      });
    }

    function rebuild(node: TreeNode): TreeNode {
      if (node.path === parent!.path) return { ...node, children: updated };
      return { ...node, children: node.children.map(rebuild) };
    }
    return rebuild(root);
  }

  // ---------------------------------------------------------------------------
  // Fs envelope handler
  // ---------------------------------------------------------------------------

  function onEnvelope(env: Envelope): void {
    if (env.category !== 'fs') return;
    const payload = env.payload as Record<string, string> | null;
    if (!payload) return;

    switch (env.kind) {
      case 'create': {
        const path = payload['path'];
        if (!path) break;
        treeActivity.mark(path, 'create');
        if (treeRoot) {
          const name = path.split('/').at(-1) ?? path;
          // We can't know from the event alone if it's a dir or file;
          // default to file. Phase 6.x: enrich with `is_dir` in payload.
          const newNode: TreeNode = { path, name, isDir: false, children: [] };
          treeRoot = insertNode(treeRoot, newNode);
        }
        break;
      }
      case 'write': {
        const path = payload['path'];
        if (path) treeActivity.mark(path, 'write');
        break;
      }
      case 'delete': {
        const path = payload['path'];
        if (!path) break;
        treeActivity.mark(path, 'delete');
        if (treeRoot) treeRoot = removeNode(treeRoot, path);
        break;
      }
      case 'rename': {
        const from = payload['from'];
        const to = payload['to'];
        if (!from || !to) break;
        treeActivity.mark(to, 'rename');
        if (treeRoot) {
          // Find the old node to preserve its isDir flag, then remove and re-insert.
          function findNode(root: TreeNode, path: string): TreeNode | null {
            if (root.path === path) return root;
            for (const c of root.children) {
              const f = findNode(c, path);
              if (f) return f;
            }
            return null;
          }
          const old = findNode(treeRoot, from);
          const toName = to.split('/').at(-1) ?? to;
          const newNode: TreeNode = {
            path: to,
            name: toName,
            isDir: old?.isDir ?? false,
            children: old?.children ?? [],
          };
          treeRoot = insertNode(removeNode(treeRoot, from), newNode);
        }
        break;
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Mount / teardown
  // ---------------------------------------------------------------------------

  $effect(() => {
    let unsubscribeFn: (() => Promise<void>) | undefined;
    let unsubscribeSysFn: (() => Promise<void>) | undefined;
    let mounted = true;

    // Fetch initial tree snapshot.
    invoke<TreeNode>('fs_tree', {})
      .then((root) => {
        if (mounted) treeRoot = root;
      })
      .catch((err: unknown) => {
        if (mounted) {
          fetchError = String(err);
        }
      });

    // Subscribe to live fs events.
    subscribe({ category: 'fs' }, onEnvelope)
      .then((unsub) => {
        if (mounted) {
          unsubscribeFn = unsub;
        } else {
          // Component torn down before subscribe resolved — unsubscribe immediately.
          unsub().catch(() => {});
        }
      })
      .catch((err: unknown) => {
        console.error('[Tree] bus subscribe failed', err);
      });

    // Phase 6.7: Subscribe to system envelopes for project.changed.
    // Uses sync-shell + IIFE pattern (pr003 svelte5-async-cleanup-via-sync-shell-iife).
    void (async () => {
      try {
        const unsub = await subscribe({ category: 'system' }, (env) => {
          if (env.kind !== 'project.changed') return;
          // Clear stale activity from the previous project.
          treeActivity.clear();
          // Re-fetch the tree for the new project root.
          invoke<TreeNode>('fs_tree', {})
            .then((root) => {
              if (mounted) {
                treeRoot = root;
                fetchError = null;
              }
            })
            .catch((err: unknown) => {
              if (mounted) fetchError = String(err);
            });
        });
        if (mounted) {
          unsubscribeSysFn = unsub;
        } else {
          unsub().catch(() => {});
        }
      } catch (err: unknown) {
        console.error('[Tree] system bus subscribe failed', err);
      }
    })();

    return () => {
      mounted = false;
      unsubscribeFn?.().catch(() => {});
      unsubscribeSysFn?.().catch(() => {});
    };
  });

  // ---------------------------------------------------------------------------
  // Helpers for per-node rendering (design calls D, E, G)
  // ---------------------------------------------------------------------------

  /**
   * State class for files and expanded dirs — reads the node's OWN activity entry.
   * Collapsed dirs use aggregateState from the layout (pre-computed in the walk).
   * Do NOT call this for collapsed dirs; it would return the dir's own (irrelevant) entry.
   */
  function stateClass(path: string): string {
    return treeActivity.getEntry(path).state;
  }

  /**
   * Click routing:
   *   dir  → toggleCollapse (design call E)
   *   file → treeActivity.cycle (existing Phase 6.2 behaviour)
   *
   * Phase 6.x: shift-click on dir to pin (currently click = toggle only).
   */
  function handleNodeClick(node: TreeNode): void {
    if (node.isDir) {
      toggleCollapse(node.path);
    } else {
      treeActivity.cycle(node.path);
    }
  }

  /**
   * Double-click routing (Phase 6.5):
   *   dir  → no-op (single-click already toggles collapse; dblclick is harmless)
   *   file → open Viewer popout at `node.path`
   *
   * Acknowledged minor side-effect: the browser fires onclick before ondblclick,
   * so activity gets cycled once before the viewer opens (visual flicker only).
   * // Phase 6.x: double-click cancels pending single-click via 250ms timer if UX feedback warrants.
   */
  function handleNodeDblClick(node: TreeNode): void {
    if (node.isDir) return;
    popouts.summon({
      content: { kind: 'viewer', path: node.path },
      width: 'min(1024px, 90vw)',
    });
  }

  /** L-shaped edge: vertical drop then horizontal run to child node. */
  function edgePath(px: number, py: number, cx: number, cy: number): string {
    // Elbow at (px, cy) — vertical segment down then horizontal to child.
    return `M ${px} ${py + DIR_H / 2 + 2} L ${px} ${cy} L ${cx - FILE_R - 2} ${cy}`;
  }

  // ---------------------------------------------------------------------------
  // Drag-node-into-terminal (Phase 6.6 — design calls A, B)
  // ---------------------------------------------------------------------------

  /** Custom MIME type isolates tree-path drags from all other drag sources. */
  const TREE_PATH_MIME = 'application/x-rift-tree-path';

  /**
   * Dragstart handler for tree nodes (files AND dirs).
   * Sets effectAllowed to 'copy' and writes the project-relative path as the
   * primary payload.  A secondary text/plain entry (prefixed 'rift-tree:') aids
   * browser-level drag-image tooltips on platforms that surface it.
   */
  function onNodeDragStart(e: DragEvent, node: TreeNode): void {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = 'copy';
    e.dataTransfer.setData(TREE_PATH_MIME, node.path);
    // Secondary — some platforms show text/plain in the drag ghost; prefix
    // discriminates our payload from any generic text drop that might land
    // on a foreign target.
    e.dataTransfer.setData('text/plain', `rift-tree:${node.path}`);
  }
</script>

<!--
  Outer wrapper matches the `.gui-tree` / `.tree-body` structure from
  the mockup so App.svelte can mount it directly inside `.cockpit-right`.
  The pane-header chrome (title + nodeCount + watchedPathLabel) is rendered
  by App.svelte per Design call A; Tree owns only the SVG body.
-->
<div class="tree-container">
  {#if fetchError}
    <div class="tree-unavailable">
      <span class="unavail-glyph">◇</span>
      <span class="unavail-text">tree unavailable</span>
      <span class="unavail-detail">{fetchError}</span>
    </div>
  {:else if !treeRoot}
    <div class="tree-unavailable">
      <span class="unavail-glyph">◈</span>
      <span class="unavail-text">loading…</span>
    </div>
  {:else}
    <svg
      class="tree-svg"
      width={SVG_WIDTH}
      viewBox="0 0 {SVG_WIDTH} {svgHeight}"
      style="height: {svgHeight}px;"
      aria-label="filesystem tree"
    >
      <!-- Edges — rendered below nodes so nodes paint on top -->
      {#each layout as item (item.node.path + '_edge')}
        {#if item.parentX !== null && item.parentY !== null}
          {@const sc = item.aggregateState ?? stateClass(item.node.path)}
          <path
            class="edge {sc === 'active' ? 'active' : sc === 'background' ? 'background' : ''}"
            d={edgePath(item.parentX, item.parentY, item.x, item.y)}
          />
        {/if}
      {/each}

      <!-- Nodes (design calls D, F, G) -->
      {#each layout as item (item.node.path)}
        {@const isCollapsedDir = item.node.isDir && collapsedDirs.has(item.node.path)}
        {@const sc = item.aggregateState ?? stateClass(item.node.path)}
        {@const glow = isCollapsedDir
          ? (item.aggregateGlow ?? 0)
          : treeActivity.getEntry(item.node.path).glowIntensity}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- `draggable` lives on HTMLAttributes in Svelte's TS surface so direct
             usage on an SVG <g> fails type-check; spread bypasses the check
             while preserving runtime behavior (Chromium webview2 supports
             draggable on SVG since ~2018). -->
        <g
          class="tree-node"
          {...({ draggable: 'true' } as Record<string, string>)}
          onclick={() => handleNodeClick(item.node)}
          ondblclick={() => handleNodeDblClick(item.node)}
          ondragstart={(e) => onNodeDragStart(e, item.node)}
          style="cursor: pointer;"
        >
          {#if item.node.isDir}
            <!-- Directory: rounded rectangle.
                 Collapsed dirs use aggregateGlow for drop-shadow;
                 expanded dirs use their own entry's glow (design call F). -->
            <rect
              class="node-bg node-state-{sc}"
              x={item.x - DIR_W / 2}
              y={item.y - DIR_H / 2}
              width={DIR_W}
              height={DIR_H}
              rx={DIR_RX}
              ry={DIR_RX}
              style={sc === 'recent' && glow > 0
                ? `filter: drop-shadow(0 0 ${4 + glow * 8}px rgba(212,137,10,${0.3 + glow * 0.45}));`
                : ''}
            />
            <!-- Folder glyph: ▶ when collapsed, ▾ when expanded (design call D).
                 Glyph state class tracks the same sc as the dir bg. -->
            <text
              class="node-glyph {sc}"
              x={item.x}
              y={item.y}
              text-anchor="middle"
              dominant-baseline="middle"
              font-size="7"
            >{isCollapsedDir ? '▶' : '▾'}</text>
          {:else}
            <!-- File: circle -->
            <circle
              class="node-bg node-state-{sc}"
              cx={item.x}
              cy={item.y}
              r={FILE_R}
              style={sc === 'recent' && glow > 0
                ? `filter: drop-shadow(0 0 ${3 + glow * 6}px rgba(212,137,10,${0.25 + glow * 0.45}));`
                : ''}
            />
          {/if}

          <!-- Label to the right of the node -->
          <text
            class="tree-node-label {sc}"
            x={item.x + (item.node.isDir ? DIR_W / 2 : FILE_R) + 6}
            y={item.y}
          >{item.node.name}</text>
        </g>
      {/each}
    </svg>
  {/if}
</div>

<style>
  .tree-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0;
  }
  .tree-container::-webkit-scrollbar { width: 5px; }
  .tree-container::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* SVG tree — node geometry from mockup §tree */
  .tree-svg {
    width: 100%;
    display: block;
    flex-shrink: 0;
  }

  /* Node shapes */
  :global(.node-bg) {
    transition: filter 0.15s ease;
  }
  :global(.node-state-ambient) {
    fill: var(--bg-elevated);
    stroke: var(--amber-warm);
    stroke-width: 1;
    filter: drop-shadow(0 0 3px rgba(176, 122, 18, 0.3));
  }
  :global(.node-state-recent) {
    fill: var(--bg-elevated);
    stroke: var(--amber-primary);
    stroke-width: 1.5;
    filter: drop-shadow(0 0 6px rgba(212, 137, 10, 0.55));
  }
  :global(.node-state-active) {
    fill: var(--bg-elevated);
    stroke: var(--amber-bright);
    stroke-width: 2;
    filter: drop-shadow(0 0 12px rgba(245, 158, 11, 0.85));
    animation: pulse-glow 1.6s ease-in-out infinite;
  }
  :global(.node-state-background) {
    fill: var(--bg-surface);
    stroke: var(--border-subtle);
    stroke-width: 1;
    opacity: 0.55;
  }
  @keyframes -global-pulse-glow {
    0%, 100% { filter: drop-shadow(0 0 12px rgba(245, 158, 11, 0.85)); }
    50%       { filter: drop-shadow(0 0 18px rgba(245, 158, 11, 1.0)); }
  }

  /* Node glyphs */
  :global(.node-glyph) {
    fill: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    font-weight: 700;
    text-anchor: middle;
    dominant-baseline: middle;
    pointer-events: none;
  }
  :global(.node-glyph.active)     { fill: var(--amber-bright); }
  :global(.node-glyph.recent)     { fill: var(--amber-primary); }
  :global(.node-glyph.background) { fill: var(--amber-faint); }

  /* Labels */
  :global(.tree-node-label) {
    fill: var(--amber-dim);
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    font-weight: 500;
    dominant-baseline: middle;
    pointer-events: none;
    user-select: none;
  }
  :global(.tree-node-label.active)     { fill: var(--amber-bright); font-weight: 700; }
  :global(.tree-node-label.recent)     { fill: var(--amber-warm);   font-weight: 600; }
  :global(.tree-node-label.background) { fill: var(--amber-faint); }

  /* Edges */
  :global(.edge) {
    stroke: var(--amber-faint);
    stroke-width: 1;
    fill: none;
    opacity: 0.5;
  }
  :global(.edge.active) {
    stroke: var(--amber-primary);
    stroke-width: 1.5;
    opacity: 0.9;
    filter: drop-shadow(0 0 3px rgba(212, 137, 10, 0.5));
  }
  :global(.edge.background) {
    stroke: var(--border-subtle);
    opacity: 0.3;
  }

  /* Unavailable / loading state */
  .tree-unavailable {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 32px 16px;
    color: var(--amber-faint);
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    font-style: italic;
  }
  .unavail-glyph {
    font-size: 22px;
    opacity: 0.5;
  }
  .unavail-text {
    color: var(--amber-dim);
    font-style: normal;
    letter-spacing: 0.08em;
  }
  .unavail-detail {
    font-size: 9px;
    text-align: center;
    max-width: 240px;
    word-break: break-all;
  }
</style>
