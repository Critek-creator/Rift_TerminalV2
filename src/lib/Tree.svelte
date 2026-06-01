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
  import { enrichmentStore, type EnrichmentEntry } from './enrichmentStore.svelte';
  import { buildEnrichmentTitle, dotX } from './enrichmentUtils';
  import TreeContextMenu from './TreeContextMenu.svelte';
  import { RIFT_VAULT_DROP_EVENT, type RiftVaultDropDetail } from './dragMime';
  import { fileColor } from './fileColors';
  import { crossRefHighlight } from './crossRefHighlight.svelte';
  import type { RiftConfig } from './riftConfig';

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

  // D-020 heatmap config — loaded on mount, refreshed on config.changed.
  let heatmapEnabled = $state(false);
  let heatmapWindowMs = $state(15 * 60_000); // default 15 min in ms

  /** O(1) path→node lookup for tree mutation hot paths. Rebuilt on full tree load,
   *  updated incrementally on insert/remove. */
  let pathMap = new Map<string, TreeNode>();

  function rebuildPathMap(root: TreeNode): void {
    pathMap.clear();
    function walk(node: TreeNode): void {
      pathMap.set(node.path, node);
      for (const child of node.children) walk(child);
    }
    walk(root);
  }

  /**
   * Set of directory paths currently collapsed by the user.
   * Empty by default (all dirs expanded). Svelte 5 Set reactivity requires
   * assign-replace on mutation — see toggleCollapse.
   */
  let collapsedDirs = $state(new Set<string>());

  // ---------------------------------------------------------------------------
  // Enrichment indicator state (Phase 8.6.2)
  // ---------------------------------------------------------------------------

  /**
   * Tree-level hover pointer for enrichment dot tooltips.
   * Only one dot can show its tooltip at a time; each dot's onmouseenter
   * sets this to item.node.path and onmouseleave clears it to null.
   * Tree-level $state (not per-row) matches the existing pattern where all
   * reactive state is declared at the component root (no per-{#each}-row $state).
   */
  let hoveredEnrichmentPath = $state<string | null>(null);

  /** File paths highlighted by vault-browser hover (cross-component, cyan glow). */
  const vaultHighlightedPaths = $derived.by<Set<string>>(() => {
    const vaultId = crossRefHighlight.hoveredVaultId;
    if (!vaultId) return new Set();
    const paths = new Set<string>();
    for (const [fsPath, entries] of enrichmentStore.map) {
      if (entries.some((e) => e.vault_id === vaultId)) {
        paths.add(fsPath);
      }
    }
    return paths;
  });

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
  // Aggregate memoization (M1 perf fix)
  //
  // Problem: treeActivity.snapshot ticks every rAF while glow decays,
  // causing the layout $derived to re-run computeAggregate recursively
  // on every collapsed directory per frame (up to 6 levels deep each).
  //
  // Solution: cache aggregate results keyed on dirPath. Invalidate the
  // entire cache only when the snapshot Map REFERENCE changes (structural
  // mutation — mark/cycle/dismiss/clear all assign a new Map). Pure glow
  // decay within a single rAF tick does NOT change which children are
  // active/recent/ambient — it only adjusts glowIntensity — so the
  // aggregate state (which only cares about max-glow > 0 and has-pinned)
  // is stable across decay frames within the same generation.
  //
  // The generation counter bumps when the entries Map reference changes
  // (checked via object identity). Decay ticks that produce a new Map
  // (decayTick assigns `entries = next` when mutated) DO bump the
  // generation, but that happens at most once per rAF — not per-node.
  // ---------------------------------------------------------------------------

  interface AggregateCache {
    generation: number;
    snapshotRef: Map<string, import('./treeActivity.svelte').ActivityEntry> | null;
    results: Map<string, { aggregateGlow: number; hasPinnedDesc: boolean }>;
  }

  const aggCache: AggregateCache = {
    generation: 0,
    snapshotRef: null,
    results: new Map(),
  };

  /**
   * Memoized wrapper around computeAggregate. Returns cached result when
   * the snapshot Map reference hasn't changed since the last call for
   * this directory path.
   */
  function computeAggregateMemo(
    node: TreeNode,
    depthRemaining: number,
  ): { aggregateGlow: number; hasPinnedDesc: boolean } {
    const currentSnapshot = treeActivity.snapshot;
    if (currentSnapshot !== aggCache.snapshotRef) {
      // Snapshot reference changed — new generation, clear all cached results.
      aggCache.generation++;
      aggCache.snapshotRef = currentSnapshot;
      aggCache.results.clear();
    }

    const cached = aggCache.results.get(node.path);
    if (cached) return cached;

    const result = computeAggregate(node, depthRemaining);
    aggCache.results.set(node.path, result);
    return result;
  }

  // ---------------------------------------------------------------------------
  // Flat layout — two-phase split (P-02):
  //   Phase 1 (structuralLayout): tree walk producing stable node positions.
  //     Re-runs only when treeRoot or collapsedDirs change.
  //   Phase 2 (layout): decorative pass applying glow/heat to stable positions.
  //     Re-runs on treeActivity.snapshot/heatLog changes (rAF during decay)
  //     but is O(visible nodes) flat iteration, not recursive tree walk.
  // ---------------------------------------------------------------------------

  interface StructuralNode {
    node: TreeNode;
    x: number;
    y: number;
    parentX: number | null;
    parentY: number | null;
  }

  interface LayoutNode extends StructuralNode {
    /** Non-null only for collapsed dirs: max descendant glow [0,1]. */
    aggregateGlow: number | null;
    /** Non-null only for collapsed dirs: synthetic state driven by descendants. */
    aggregateState: ActivityState | null;
    /** D-020: normalized heat value 0–1 (0 = cold, 1 = hottest in view). */
    heatValue: number;
  }

  const structuralLayout = $derived.by((): StructuralNode[] => {
    void collapsedDirs;
    if (!treeRoot) return [];

    const rows: StructuralNode[] = [];
    let row = 0;

    function walk(
      node: TreeNode,
      depth: number,
      parentX: number | null,
      parentY: number | null,
    ) {
      const x = ROOT_X + depth * X_STEP;
      const y = ROW_H / 2 + row * ROW_H;
      rows.push({ node, x, y, parentX, parentY });
      row++;

      const isCollapsed = node.isDir && collapsedDirs.has(node.path);
      if (!isCollapsed) {
        for (const child of node.children) {
          walk(child, depth + 1, x, y);
        }
      }
    }

    walk(treeRoot, 0, null, null);
    return rows;
  });

  const layout = $derived.by((): LayoutNode[] => {
    void treeActivity.snapshot;
    void treeActivity.heatLog;

    const structural = structuralLayout;
    if (structural.length === 0) return [];

    const heatCounts = heatmapEnabled
      ? treeActivity.heatSnapshot(heatmapWindowMs)
      : null;

    const rows: LayoutNode[] = structural.map(({ node, x, y, parentX, parentY }) => {
      const isCollapsed = node.isDir && collapsedDirs.has(node.path);
      let aggregateGlow: number | null = null;
      let aggState: ActivityState | null = null;

      if (isCollapsed) {
        const agg = computeAggregateMemo(node, MAX_BUBBLE_DEPTH);
        aggregateGlow = agg.aggregateGlow;
        aggState = aggregateStateFromGlow(agg.aggregateGlow, agg.hasPinnedDesc);
      }

      const rawHeat = heatCounts?.get(node.path) ?? 0;
      return {
        node, x, y, parentX, parentY, aggregateGlow, aggregateState: aggState,
        heatValue: rawHeat,
      };
    });

    // D-020: bubble up heat for collapsed dirs + normalize to 0–1.
    if (heatCounts && rows.length > 0) {
      const hc = heatCounts;
      for (const item of rows) {
        if (item.node.isDir && collapsedDirs.has(item.node.path)) {
          let sum = item.heatValue;
          function sumChildren(n: TreeNode): void {
            for (const child of n.children) {
              sum += hc.get(child.path) ?? 0;
              if (child.isDir) sumChildren(child);
            }
          }
          sumChildren(item.node);
          item.heatValue = sum;
        }
      }

      let maxHeat = 0;
      for (const item of rows) {
        if (item.heatValue > maxHeat) maxHeat = item.heatValue;
      }
      if (maxHeat > 0) {
        for (const item of rows) {
          item.heatValue = item.heatValue / maxHeat;
        }
      }
    }

    return rows;
  });

  const svgHeight = $derived(layout.length * ROW_H + PADDING_BOTTOM);

  // --- Keyboard navigation (WAI-ARIA tree pattern) -------------------------
  // SVG <g> treeitems can't reliably hold DOM focus in WebView2 (same gotcha
  // as the SVG drag bug), so the <svg> owns focus (tabindex=0) and we track the
  // active row index here, surfacing it via aria-activedescendant + a visual
  // focus band. Purely additive over the existing mouse interaction — arrow
  // keys move the active row, Right/Left expand/collapse, Enter/Space activate.
  let kbdIndex = $state(0);
  const kbdActive = $derived(layout.length ? Math.min(kbdIndex, layout.length - 1) : -1);
  const kbdActiveId = $derived(kbdActive >= 0 ? `tree-node-${kbdActive}` : undefined);

  /** Nearest preceding row at a shallower indent (smaller x) = the parent. */
  function kbdParentIndex(i: number): number {
    const x = layout[i].x;
    for (let j = i - 1; j >= 0; j--) {
      if (layout[j].x < x) return j;
    }
    return -1;
  }

  function onTreeKeydown(e: KeyboardEvent): void {
    const n = layout.length;
    if (n === 0) return;
    const i = kbdActive;
    const item = layout[i];
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); kbdIndex = Math.min(n - 1, i + 1); break;
      case 'ArrowUp':   e.preventDefault(); kbdIndex = Math.max(0, i - 1); break;
      case 'Home':      e.preventDefault(); kbdIndex = 0; break;
      case 'End':       e.preventDefault(); kbdIndex = n - 1; break;
      case 'ArrowRight':
        e.preventDefault();
        if (item.node.isDir) {
          if (collapsedDirs.has(item.node.path)) toggleCollapse(item.node.path); // expand
          else if (i + 1 < n) kbdIndex = i + 1;                                   // into first child
        }
        break;
      case 'ArrowLeft': {
        e.preventDefault();
        if (item.node.isDir && !collapsedDirs.has(item.node.path)) {
          toggleCollapse(item.node.path);          // collapse open dir
        } else {
          const p = kbdParentIndex(i);             // jump to parent row
          if (p !== -1) kbdIndex = p;
        }
        break;
      }
      case 'Enter':
      case ' ':
        e.preventDefault();
        handleNodeClick(item.node);
        break;
      default:
        return;
    }
  }

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

    const cached = pathMap.get(parentPath);
    if (cached) return cached;

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
    pathMap.delete(path);
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
      updated.sort((a, b) => {
        if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
        return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
      });
    }

    pathMap.set(newNode.path, newNode);

    function rebuild(node: TreeNode): TreeNode {
      if (node.path === parent!.path) {
        const rebuilt = { ...node, children: updated };
        pathMap.set(rebuilt.path, rebuilt);
        return rebuilt;
      }
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
          const old = pathMap.get(from) ?? null;
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
        if (mounted) {
          treeRoot = root;
          rebuildPathMap(root);
        }
      })
      .catch((err: unknown) => {
        if (mounted) {
          fetchError = String(err);
        }
      });

    // D-020: load heatmap config.
    invoke<RiftConfig>('config_get')
      .then((cfg) => {
        if (mounted) {
          heatmapEnabled = cfg.tree.heatmap_enabled;
          heatmapWindowMs = cfg.tree.heatmap_window_minutes * 60_000;
        }
      })
      .catch(() => {});

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
                rebuildPathMap(root);
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
      // Clean up drag listeners if component unmounts mid-drag.
      if (treeDragActive || treeDragNode) {
        document.removeEventListener('mousemove', onTreeDocMouseMove);
        document.removeEventListener('mouseup', onTreeDocMouseUp);
        document.body.style.cursor = '';
        treeDragNode = null;
        treeDragActive = false;
      }
    };
  });

  // ---------------------------------------------------------------------------
  // Helpers for per-node rendering (design calls D, E, G)
  // ---------------------------------------------------------------------------

  /**
   * D-020: interpolate heat value (0–1) to an RGB color on the ramp
   * amber-faint (#A87830) → amber-primary (#FFA826) → term-red (#FF4848).
   */
  function heatColor(t: number): string {
    if (t <= 0.5) {
      const s = t * 2; // 0–1 within first half
      const r = Math.round(168 + (255 - 168) * s);
      const g = Math.round(120 + (168 - 120) * s);
      const b = Math.round(48 + (38 - 48) * s);
      return `rgb(${r},${g},${b})`;
    }
    const s = (t - 0.5) * 2; // 0–1 within second half
    const r = Math.round(255 + (255 - 255) * s);
    const g = Math.round(168 + (72 - 168) * s);
    const b = Math.round(38 + (72 - 38) * s);
    return `rgb(${r},${g},${b})`;
  }

  /**
   * State class for files and expanded dirs — reads the node's OWN activity entry.
   * Collapsed dirs use aggregateState from the layout (pre-computed in the walk).
   * Do NOT call this for collapsed dirs; it would return the dir's own (irrelevant) entry.
   */
  function stateClass(path: string): string {
    return treeActivity.getEntry(path).state;
  }

  /**
   * Click routing (corrected per user spec — drift from earlier Phase 6.x
   * implementation):
   *   dir  → toggleCollapse (design call E — unchanged)
   *   file → open Viewer popout + dismiss any active glow
   *
   * Activity glow is reserved for AI/agent file-access events from the bus
   * (Category::Fs envelopes from translators). The user is the OBSERVER of
   * AI activity, not a participant — clicking a file acknowledges "I've
   * seen what AI is doing" and the glow goes away (treeActivity.dismiss).
   * Unclicked files decay naturally per the existing decay loop.
   *
   * The ORIGINAL Phase 6.2 implementation called treeActivity.cycle here,
   * which pinned files on click — opposite of intended UX. Per session
   * spec correction, single-click now opens the Viewer popout (the previous
   * dblclick-to-open behavior) and dismisses the glow side-effect.
   *
   * Phase 6.x: shift-click could become a "pin to keep visible" gesture
   * via treeActivity.cycle (still exported), but that's deferred until
   * operational signal demands it.
   */
  function handleNodeClick(node: TreeNode): void {
    // Keep keyboard navigation in sync with mouse selection.
    const idx = layout.findIndex((it) => it.node.path === node.path);
    if (idx !== -1) kbdIndex = idx;
    if (node.isDir) {
      toggleCollapse(node.path);
      return;
    }
    treeActivity.dismiss(node.path);
    popouts.summon({
      content: { kind: 'viewer', path: node.path },
      width: 'min(1024px, 90vw)',
    });
  }

  /**
   * Double-click routing — currently a no-op for both dirs and files
   * (single-click handles file opening per the spec correction above;
   * single-click already toggles collapse for dirs).
   *
   * Reserved for future polish (e.g. "open in external editor", "open in
   * a new viewer popout instead of replacing"). Dispatcher kept so the
   * markup reference at line 559 doesn't have to change when polish lands.
   */
  function handleNodeDblClick(_node: TreeNode): void {
    /* no-op — see comment above */
  }

  // Right-click context menu for tree nodes (inject / open / cd / copy / reveal).
  let contextMenu = $state<{
    node: TreeNode;
    x: number;
    y: number;
    enrichments: EnrichmentEntry[] | undefined;
  } | null>(null);

  function handleNodeContextMenu(e: MouseEvent, node: TreeNode): void {
    e.preventDefault(); // suppress the native WebView2 menu (scoped to the node)
    e.stopPropagation(); // don't let it reach the menu's window-level closer
    contextMenu = {
      node,
      x: e.clientX,
      y: e.clientY,
      enrichments: enrichmentStore.get(node.path),
    };
  }

  /** L-shaped edge: vertical drop then horizontal run to child node. */
  function edgePath(px: number, py: number, cx: number, cy: number): string {
    // Elbow at (px, cy) — vertical segment down then horizontal to child.
    return `M ${px} ${py + DIR_H / 2 + 2} L ${px} ${cy} L ${cx - FILE_R - 2} ${cy}`;
  }

  // ---------------------------------------------------------------------------
  // Drag-node-into-terminal (Phase 6.6 — design calls A, B)
  // ---------------------------------------------------------------------------

  // ---------------------------------------------------------------------------
  // Manual-gesture drag-into-terminal (Phase 8.7g.4 — replaces HTML5 drag)
  //
  // WHY MANUAL: WebView2 does NOT initiate HTML5 drag on SVG <g> elements
  // even when `draggable="true"` is set as an HTML attribute. The Phase 6.6
  // pattern (ondragstart on SVG <g>) silently never fired — the feature
  // was assumed-working but actually broken since ship. Same gotcha that
  // bit IndexGraph in Phase 8.7; the manual mousedown/move/up gesture
  // there is now mirrored here for Tree.
  //
  // Gesture: mousedown on a node → register document mousemove + mouseup.
  // Once movement crosses the threshold, set drag-active state. On mouseup,
  // hit-test elementFromPoint for `.terminal-host` and dispatch
  // RIFT_VAULT_DROP_EVENT (reused — Terminal.svelte's existing listener
  // simply pastes the path into the active session).
  // ---------------------------------------------------------------------------

  const TREE_DRAG_THRESHOLD_PX = 5;

  let treeDragNode: TreeNode | null = null;
  let treeDragStartX = 0;
  let treeDragStartY = 0;
  let treeDragActive = $state(false);
  let treeGhostX = $state(0);
  let treeGhostY = $state(0);
  let treeGhostLabel = $state('');

  function onTreeNodeMouseDown(e: MouseEvent, node: TreeNode): void {
    if (e.button !== 0) return;       // left-click only
    treeDragNode = node;
    treeDragStartX = e.clientX;
    treeDragStartY = e.clientY;
    treeDragActive = false;
    document.addEventListener('mousemove', onTreeDocMouseMove);
    document.addEventListener('mouseup', onTreeDocMouseUp);
    // Don't preventDefault here — we want click + dblclick to still fire
    // if the user mouseup's without crossing the threshold.
  }

  function onTreeDocMouseMove(e: MouseEvent): void {
    if (!treeDragNode) return;
    if (!treeDragActive) {
      const dx = Math.abs(e.clientX - treeDragStartX);
      const dy = Math.abs(e.clientY - treeDragStartY);
      if (dx > TREE_DRAG_THRESHOLD_PX || dy > TREE_DRAG_THRESHOLD_PX) {
        treeDragActive = true;
        treeGhostLabel = treeDragNode.path;
        document.body.style.cursor = 'grabbing';
      }
    }
    if (treeDragActive) {
      treeGhostX = e.clientX;
      treeGhostY = e.clientY;
    }
  }

  function onTreeDocMouseUp(e: MouseEvent): void {
    document.removeEventListener('mousemove', onTreeDocMouseMove);
    document.removeEventListener('mouseup', onTreeDocMouseUp);
    document.body.style.cursor = '';

    const node = treeDragNode;
    const wasActive = treeDragActive;
    treeDragNode = null;
    treeDragActive = false;
    treeGhostLabel = '';

    if (!wasActive || !node) return;  // mouseup without threshold = a click

    const target = document.elementFromPoint(e.clientX, e.clientY);
    const terminalHost = target?.closest('.terminal-host');
    if (!terminalHost) return;

    const detail: RiftVaultDropDetail = { path: node.path };
    terminalHost.dispatchEvent(
      new CustomEvent(RIFT_VAULT_DROP_EVENT, { detail, bubbles: true }),
    );
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
      role="tree"
      aria-label="filesystem tree (arrow keys to navigate)"
      tabindex="0"
      aria-activedescendant={kbdActiveId}
      onkeydown={onTreeKeydown}
    >
      <!-- Edges — rendered below nodes so nodes paint on top -->
      {#each layout as item (item.node.path + '_edge')}
        {#if item.parentX !== null && item.parentY !== null}
          {@const sc = item.aggregateState ?? stateClass(item.node.path)}
          {@const isCrossRef = vaultHighlightedPaths.has(item.node.path)}
          <path
            class="edge {sc === 'active' ? 'active' : sc === 'background' ? 'background' : ''}{isCrossRef ? ' crossref' : ''}"
            d={edgePath(item.parentX, item.parentY, item.x, item.y)}
          />
        {/if}
      {/each}

      <!-- Nodes (design calls D, F, G) -->
      {#each layout as item, i (item.node.path)}
        {@const isCollapsedDir = item.node.isDir && collapsedDirs.has(item.node.path)}
        {@const sc = item.aggregateState ?? stateClass(item.node.path)}
        {@const glow = isCollapsedDir
          ? (item.aggregateGlow ?? 0)
          : treeActivity.getEntry(item.node.path).glowIntensity}
        {@const enrichments = enrichmentStore.get(item.node.path)}
        {@const nodeColor = item.node.isDir ? null : fileColor(item.node.name)}
        {@const isCrossRef = vaultHighlightedPaths.has(item.node.path)}
        {@const heat = item.heatValue}
        <!-- svelte-ignore a11y_no_static_element_interactions — SVG <g> does not support role/tabindex natively in WebView2; keyboard nav handled by parent <svg> -->
        <!--
          Phase 8.7g.4 — replaced HTML5 `draggable="true"` (silently broken
          on SVG <g> in WebView2 — the same gotcha that bit IndexGraph in
          Phase 8.7) with manual mousedown→mousemove→mouseup gesture.
          Click and dblclick still fire when the user releases without
          crossing the drag threshold.
        -->
        <g
          class="tree-node"
          class:kbd-active={i === kbdActive}
          id="tree-node-{i}"
          role="treeitem"
          aria-selected={i === kbdActive}
          tabindex="-1"
          aria-expanded={item.node.isDir ? !collapsedDirs.has(item.node.path) : undefined}
          aria-label={item.node.name}
          onmousedown={(e) => onTreeNodeMouseDown(e, item.node)}
          onclick={() => handleNodeClick(item.node)}
          onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); handleNodeClick(item.node); } }}
          ondblclick={() => handleNodeDblClick(item.node)}
          oncontextmenu={(e) => handleNodeContextMenu(e, item.node)}
          style="cursor: pointer;"
        >
          <!-- Full-width row hover band — gives the SVG tree the same
               "row lights up on hover" feel as the INDEX list rows so the two
               sidebar halves share one interaction language, without flattening
               the node-link graph. Translucent so connector edges still read
               through; pointer-events:none so empty row space never intercepts
               node clicks (hover is driven by the node/label). -->
          <rect
            class="row-band"
            x="0"
            y={item.y - ROW_H / 2}
            width={SVG_WIDTH}
            height={ROW_H}
          />
          {#if item.node.isDir}
            <!-- Directory: rounded rectangle.
                 Collapsed dirs use aggregateGlow for drop-shadow;
                 expanded dirs use their own entry's glow (design call F). -->
            <rect
              class="node-bg node-state-{sc}{isCrossRef ? ' node-crossref' : ''}"
              x={item.x - DIR_W / 2}
              y={item.y - DIR_H / 2}
              width={DIR_W}
              height={DIR_H}
              rx={DIR_RX}
              ry={DIR_RX}
              style={(isCrossRef
                ? `filter: drop-shadow(0 0 8px rgba(74,212,212,0.65));`
                : sc === 'recent' && glow > 0
                  ? `filter: drop-shadow(0 0 ${4 + glow * 8}px rgba(212,137,10,${0.3 + glow * 0.45}));`
                  : '')
                + (heat > 0.1 && !isCrossRef ? ` stroke: ${heatColor(heat)}; stroke-width: ${1 + heat};` : '')}
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
            <!-- File: circle — stroke colored by file type, amber glow on activity -->
            <circle
              class="node-bg node-state-{sc}{isCrossRef ? ' node-crossref' : ''}"
              cx={item.x}
              cy={item.y}
              r={FILE_R}
              style="{isCrossRef
                ? `stroke: var(--term-cyan); filter: drop-shadow(0 0 8px rgba(111,224,224,0.65));`
                : (heat > 0.1 ? `stroke: ${heatColor(heat)}; stroke-width: ${1 + heat};` : sc !== 'background' && nodeColor ? `stroke: ${nodeColor};` : '') + (sc === 'recent' && glow > 0
                  ? `filter: drop-shadow(0 0 ${3 + glow * 6}px rgba(255,168,38,${0.25 + glow * 0.45}));`
                  : '')}"
            />
          {/if}

          <!-- Label to the right of the node — file-type color via inline style -->
          <text
            class="tree-node-label {sc}{isCrossRef ? ' crossref' : ''}"
            x={item.x + (item.node.isDir ? DIR_W / 2 : FILE_R) + 6}
            y={item.y}
            style={isCrossRef ? 'fill: var(--term-cyan);' : sc !== 'background' && nodeColor ? `fill: ${nodeColor};` : ''}
          >{item.node.name}</text>

          <!-- Enrichment dot (Phase 8.6.2) — muted-amber §10.1 "meta/timestamps" lane.
               Rendered only when EnrichmentStore has entries for this node's path.
               `enrichments` declared at {#each} level (above) — {#if} validates it here.
               Dot pointer-events:none so drag continues to bubble to the parent <g>.
               Hover state: tree-level hoveredEnrichmentPath pointer (not per-row $state)
               — matches Tree.svelte's existing pattern of no per-{#each}-row $state. -->
          {#if enrichments && enrichments.length > 0}
            {@const ex = dotX(item.x, item.node.isDir, item.node.name)}
            {@const isHovered = hoveredEnrichmentPath === item.node.path}
            <g
              class="enrichment-dot-group"
              aria-label="Enriched"
              onmouseenter={() => { hoveredEnrichmentPath = item.node.path; crossRefHighlight.hoveredTreePath = item.node.path; }}
              onmouseleave={() => { hoveredEnrichmentPath = null; crossRefHighlight.hoveredTreePath = null; }}
            >
              <!-- Screen-reader + native tooltip fallback -->
              <title>{buildEnrichmentTitle(enrichments)}</title>

              <!-- The dot itself — pointer-events:none on the circle, events on the <g> -->
              <circle
                cx={ex}
                cy={item.y}
                r={FILE_R / 2}
                fill="var(--amber-faint)"
                stroke="none"
                style="pointer-events: none;"
              />

              <!-- Visual tooltip — foreignObject so it inherits SVG transforms and
                   scrolls with the tree (avoids getBoundingClientRect scroll-detach
                   in the overflow-y:auto .tree-container). Renders only while hovered. -->
              {#if isHovered}
                <foreignObject
                  x={ex + 8}
                  y={item.y - 12}
                  width="200"
                  height="1"
                  style="overflow: visible;"
                >
                  <div class="enrichment-tooltip" xmlns="http://www.w3.org/1999/xhtml">
                    {#each enrichments as entry (entry.provider_id + ':' + entry.entry_id)}
                      <div class="enrichment-tooltip-row">
                        <span class="et-vault-id">{entry.label ?? entry.vault_id ?? entry.entry_id}</span>
                        {#if entry.vault_kind}
                          <span class="et-vault-kind"> ({entry.vault_kind})</span>
                        {:else if entry.provider_id !== 'index'}
                          <span class="et-vault-kind"> [{entry.provider_id}]</span>
                        {/if}
                        {#if entry.tags.length > 0}
                          <div class="et-tags">{entry.tags.join(', ')}</div>
                        {/if}
                      </div>
                    {/each}
                  </div>
                </foreignObject>
              {/if}
            </g>
          {/if}
        </g>
      {/each}
    </svg>
  {/if}

  <!--
    Phase 8.7g.4 — drag ghost. Fixed-position so it escapes the
    .tree-container overflow:auto + the cockpit-right pane's overflow:
    hidden boundary, letting the user visually carry a node over the
    terminal pane. pointer-events:none preserves elementFromPoint so
    the .terminal-host hit-test on mouseup works.
  -->
  {#if treeDragActive}
    <div
      class="tree-drag-ghost"
      style="left: {treeGhostX}px; top: {treeGhostY}px;"
      aria-hidden="true"
    >
      <span class="tree-drag-ghost-glyph">↗</span>
      <span class="tree-drag-ghost-label">{treeGhostLabel}</span>
    </div>
  {/if}

  {#if contextMenu}
    <TreeContextMenu
      node={contextMenu.node}
      x={contextMenu.x}
      y={contextMenu.y}
      enrichments={contextMenu.enrichments}
      onClose={() => { contextMenu = null; }}
    />
  {/if}
</div>

<style>
  .tree-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-xs) 0;
  }
  .tree-container::-webkit-scrollbar { width: 5px; }
  .tree-container::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* SVG tree — node geometry from mockup §tree */
  .tree-svg {
    width: 100%;
    display: block;
    flex-shrink: 0;
  }

  :global(.tree-node:focus-visible) {
    outline: 1px solid var(--amber-warm);
    outline-offset: -1px;
  }

  /* Row hover band — matches the INDEX list-row hover language. Translucent
     warm tint (not opaque bg-hover) so connector edges stay visible through
     it; pointer-events:none keeps clicks landing on the node, not the band. */
  :global(.tree-node .row-band) {
    fill: transparent;
    pointer-events: none;
    transition: fill var(--duration-base) var(--ease-out);
  }
  :global(.tree-node:hover .row-band) {
    fill: rgba(255, 168, 38, 0.07);
  }
  /* Keyboard-active row (aria-activedescendant) — stronger than hover + an
     amber edge so the focused row reads clearly when navigating by arrows. */
  :global(.tree-node.kbd-active .row-band) {
    fill: var(--bg-amber-selected);
    stroke: var(--amber-dim);
    stroke-width: 1;
  }

  /* Node shapes */
  :global(.node-bg) {
    transition: filter var(--duration-med) var(--ease-out);
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
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 700;
    text-anchor: middle;
    dominant-baseline: middle;
    pointer-events: none;
  }
  :global(.node-glyph.active)     { fill: var(--amber-bright); }
  :global(.node-glyph.recent)     { fill: var(--amber-primary); }
  :global(.node-glyph.background) { fill: var(--amber-faint); }

  /* Cross-ref highlight — vault browser hover (cyan accent from Index tab) */
  :global(.node-crossref) {
    stroke: var(--term-cyan, #6FE0E0);
    stroke-width: 2;
    animation: crossref-pulse 0.35s ease-out;
  }
  @keyframes -global-crossref-pulse {
    from { filter: drop-shadow(0 0 14px rgba(74, 212, 212, 0.9)); }
    to   { filter: drop-shadow(0 0 8px rgba(74, 212, 212, 0.65)); }
  }

  /* Labels */
  :global(.tree-node-label) {
    fill: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 500;
    dominant-baseline: middle;
    pointer-events: none;
    user-select: none;
  }
  :global(.tree-node-label.active)     { fill: var(--amber-bright); font-weight: 700; }
  :global(.tree-node-label.recent)     { fill: var(--amber-warm);   font-weight: 600; }
  :global(.tree-node-label.background) { fill: var(--amber-faint); }
  :global(.tree-node-label.crossref) { fill: var(--term-cyan, #6FE0E0); font-weight: 700; }

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
  :global(.edge.crossref) {
    stroke: var(--term-cyan, #6FE0E0);
    stroke-width: 1.5;
    opacity: 0.8;
    filter: drop-shadow(0 0 3px rgba(74, 212, 212, 0.5));
  }

  /* Enrichment dot tooltip (Phase 8.6.2) */
  :global(.enrichment-dot-group) {
    cursor: default;
  }
  :global(.enrichment-tooltip) {
    background: var(--bg-base);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    padding: var(--space-xs) 7px;
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    color: var(--amber-primary);
    white-space: nowrap;
    pointer-events: none;
    width: max-content;
    max-width: 200px;
  }
  :global(.enrichment-tooltip-row) {
    line-height: 1.5;
  }
  :global(.et-vault-id) {
    font-weight: 700;
    color: var(--amber-primary);
  }
  :global(.et-vault-kind) {
    font-weight: 400;
    color: var(--amber-faint);
  }
  :global(.et-tags) {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    padding-left: var(--space-xs);
  }

  /* Unavailable / loading state */
  .tree-unavailable {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-8);
    padding: var(--space-2xl) var(--space-lg);
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-sm);
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
    font-size: var(--text-2xs);
    text-align: center;
    max-width: 240px;
    word-break: break-all;
  }

  /* Phase 8.7g.4 — manual-gesture drag ghost. Fixed-positioned so it
     escapes the tree-container overflow + cockpit-right clipping; lets
     the user carry a tree path visually over the terminal pane. */
  .tree-drag-ghost {
    position: fixed;
    transform: translate(-50%, -50%);
    pointer-events: none;
    z-index: 5000;
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-xs) var(--space-md);
    background: rgba(15, 12, 6, 0.94);
    border: 1px solid var(--amber-bright);
    border-radius: 12px;
    font-family: var(--font-family);
    font-size: var(--text-sm);
    color: var(--amber-warm);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5),
                0 0 12px rgba(255, 168, 38, 0.45);
    white-space: nowrap;
    max-width: 60vw;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tree-drag-ghost-glyph {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-strong);
  }
  .tree-drag-ghost-label {
    color: var(--amber-warm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
