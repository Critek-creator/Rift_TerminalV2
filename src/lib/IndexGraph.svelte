<script lang="ts">
  // IndexGraph.svelte — Radial ring layout with expand/collapse
  //
  // Abyssal Index vault graph. Two-tier layout:
  //   Tier 1 — 7 category group nodes in a radial ring around INDEX root
  //   Tier 2 — children fan outward from expanded category (single-expand mode)
  //
  // Data flow (unchanged):
  //   vault_walker (Rust) →  Category::Index/vault.update  → nodes/edges state
  //   vault_walker (Rust) →  Category::Index/walk.complete → walkComplete flag
  //   $effect reads reactive arrays, computes positions deterministically.
  //
  // Fallback:
  //   If walkComplete is true and nodes is still empty (abyssal-index absent),
  //   the static fixture is displayed — §10.7 capability-driven empty state.
  //
  // DOM ownership: Svelte owns all rendering via {#each}. D3 provides
  //   zoom/pan only (d3-zoom). No d3-hierarchy, no force simulation.
  //
  // Drag: manual mousedown/move/up gesture for drag-into-terminal
  //   (WebView2 does not initiate HTML5 drag on SVG <g> elements).

  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  // d3-hierarchy removed — radial ring layout replaces d3.tree()
  import { select } from 'd3-selection';
  import { zoom, zoomIdentity } from 'd3-zoom';
  import type { ZoomBehavior, ZoomTransform } from 'd3-zoom';
  import { subscribe } from './bus';
  import { RIFT_VAULT_DROP_EVENT, type RiftVaultDropDetail } from './dragMime';

  // ---------------------------------------------------------------------------
  // Types
  // ---------------------------------------------------------------------------

  /** Vault kind prefix — drives node color class. */
  type VaultKind = 'p' | 'pr' | 'r' | 's' | 'lore' | 'agt' | 'h';

  /**
   * A vault node in the force simulation.
   * D3 mutates x, y, vx, vy in-place during simulation; fx/fy pin a node.
   */
  /** §10.1 + mockup §10.3 visual-state hierarchy. Drives node radius, glow,
   *  label brightness. Computed at render time from updatedMs + hover. */
  type NodeState = 'active' | 'recent' | 'ambient' | 'background';

  /** "recent" cutoff — vaults modified within this many ms are highlighted. */
  const RECENT_WINDOW_MS = 60 * 60 * 1000; // 1 hour

  /** Per-kind Unicode glyph rendered inside the node circle (mockup §10.3).
   *  All chosen for monospace amber-friendly rendering on JetBrains Mono. */
  const KIND_GLYPH: Record<VaultKind, string> = {
    p: '◐',     // project — circle-half (active orbit)
    pr: '§',    // practices/rules
    r: '✦',     // research (four-pointed star)
    s: '⚙',     // skill (gear)
    lore: '✧',  // lore (open star)
    agt: '⚝',   // agent (outlined star)
    h: '⏱',     // history (stopwatch)
  };

  interface VaultNode {
    id: string;
    kind: VaultKind;
    label: string;
    shortLabel?: string;
    displayName?: string;
    updatedMs?: number;
    x?: number;
    y?: number;
    path?: string;
  }

  /**
   * A directed link between two vault nodes.
   * D3 resolves source/target from string IDs into VaultNode references after
   * forceLink processes the array.
   */
  interface VaultLink {
    source: string;
    target: string;
    parent?: boolean;
  }

  // ---------------------------------------------------------------------------
  // vault.update envelope payload shape (mirrors VaultUpdatePayload in Rust)
  // ---------------------------------------------------------------------------

  interface VaultUpdatePayload {
    vault_id: string;
    /** D-015 / Phase 8.7n — present on sub-doors. Top-level vaults emit
     *  null (which JSON.parse delivers as null, not undefined). */
    parent_vault_id?: string | null;
    path: string;
    change_kind: 'created' | 'modified' | 'deleted';
    /** Phase 8.7p — rich payload only. Telegraphic VAULT line "name" field. */
    name?: string;
    /** Phase 8.7p — short tagline (`label:` field or derived from name). */
    short_label?: string | null;
    /** Phase 8.7p — vault file mtime as Unix epoch ms. */
    updated_ms?: number | null;
    /** Frontmatter cross-reference vault ids (rich payload only). */
    cross_refs?: string[];
  }

  // ---------------------------------------------------------------------------
  // Static fixture — fallback when walkComplete=true but no live data
  //
  // Preserved from 8.3. Used only when ~/.claude/abyssal-index/ is absent.
  // When live data is present the fixture is superseded.
  // ---------------------------------------------------------------------------

  const STATIC_NODES: VaultNode[] = [
    { id: 'p001',  kind: 'p',  label: 'p001 — Abyssal Arts Main' },
    { id: 'p002',  kind: 'p',  label: 'p002 — AIDE (Abyssal IDE)' },
    { id: 'p003',  kind: 'p',  label: 'p003 — AIDE v2' },
    { id: 'p004',  kind: 'p',  label: 'p004 — Abyssal Indexer' },
    { id: 'p005',  kind: 'p',  label: 'p005 — Abyssal Insight' },
    { id: 'p006',  kind: 'p',  label: 'p006 — Rift Terminal V2' },
    { id: 'pr001', kind: 'pr', label: 'pr001 — Global Practices' },
    { id: 'pr003', kind: 'pr', label: 'pr003 — Lessons/Gotchas' },
    { id: 'pr004', kind: 'pr', label: 'pr004 — Agent Protocols' },
    { id: 'r004',  kind: 'r',  label: 'r004 — Tauri+Svelte Research' },
  ];

  const STATIC_LINKS: VaultLink[] = [
    { source: 'p006',  target: 'r004'  },
    { source: 'p003',  target: 'r004'  },
    { source: 'pr001', target: 'pr003' },
    { source: 'pr003', target: 'pr001' },
    { source: 'pr003', target: 'p006'  },
    { source: 'pr003', target: 'p003'  },
    { source: 'pr001', target: 'p006'  },
    { source: 'pr001', target: 'p003'  },
    { source: 'p001',  target: 'pr001' },
    { source: 'p002',  target: 'pr001' },
    { source: 'p004',  target: 'r004'  },
    { source: 'p005',  target: 'pr001' },
    { source: 'r004',  target: 'pr003' },
    { source: 'p006',  target: 'pr003' },
    { source: 'p001',  target: 'p006'  },
  ];

  // ---------------------------------------------------------------------------
  // Live data state (Phase 8.5)
  // ---------------------------------------------------------------------------

  /** Map of vault_id → node. Updated by vault.update envelopes. */
  let liveNodeMap = $state<Map<string, VaultNode>>(new Map());

  /**
   * True once a walk.complete envelope arrives. Lets the graph distinguish
   * "still loading from walker" from "walker done / abyssal-index absent".
   */
  let walkComplete = $state(false);

  /**
   * Derive the active node/edge arrays from liveNodeMap.
   * When walkComplete=true and liveNodeMap is empty → use static fixture.
   */
  const activeNodes = $derived.by<VaultNode[]>(() => {
    if (liveNodeMap.size > 0) {
      return Array.from(liveNodeMap.values());
    }
    if (walkComplete) {
      // Fallback: abyssal-index absent or walker found no vaults.
      return STATIC_NODES.map((n) => ({ ...n }));
    }
    // Still loading — render nothing yet (graph will re-trigger when data arrives).
    return [];
  });

  /**
   * Rebuild edges from cross_refs stored on nodes.
   * When using the static fixture, use STATIC_LINKS.
   */
  const activeEdges = $derived.by<VaultLink[]>(() => {
    if (liveNodeMap.size === 0 && walkComplete) {
      return STATIC_LINKS.map((l) => ({ ...l }));
    }
    // Build edges from:
    //   1. cross_refs stored on each node's `crossRefs` field (frontmatter)
    //   2. parent_vault_id stored on each node (D-015 sub-doors → parent)
    // Both kinds of edge use the same VaultLink shape; the parent kind is
    // marked with `parent: true` so the renderer can style them differently
    // if it wants to. v1 uses the same style for both.
    const edges: VaultLink[] = [];
    for (const [id, node] of liveNodeMap) {
      const extra = node as VaultNode & {
        crossRefs?: string[];
        parentId?: string | null;
      };
      for (const ref of extra.crossRefs ?? []) {
        if (liveNodeMap.has(ref)) {
          edges.push({ source: id, target: ref });
        }
      }
      if (extra.parentId && liveNodeMap.has(extra.parentId)) {
        edges.push({ source: id, target: extra.parentId, parent: true });
      }
    }
    return edges;
  });

  // ---------------------------------------------------------------------------
  // Infer VaultKind from vault_id prefix
  // ---------------------------------------------------------------------------

  function inferKind(vaultId: string): VaultKind {
    if (vaultId.startsWith('pr'))  return 'pr';
    if (vaultId.startsWith('lore')) return 'lore';
    if (vaultId.startsWith('agt')) return 'agt';
    if (vaultId.startsWith('p'))   return 'p';
    if (vaultId.startsWith('r'))   return 'r';
    if (vaultId.startsWith('s'))   return 's';
    if (vaultId.startsWith('h'))   return 'h';
    return 'p'; // default
  }

  // ---------------------------------------------------------------------------
  // Category::Index subscription — mount-race guarded (pr003 subscribe-mount-race)
  // ---------------------------------------------------------------------------

  onMount(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    // Phase 8.7p — load IndexConfig at mount AND on every Settings save.
    // Density change re-runs the radial-layout $effect (indexDensityScale
    // is reactive); label-visibility re-runs the template binding directly.
    async function loadIndexConfig(): Promise<void> {
      try {
        const cfg = await invoke<{
          index: {
            label_visibility?: 'always' | 'hover_only' | 'on_zoom2x' | 'unknown';
            density?: 'compact' | 'standard' | 'spacious' | 'unknown';
          };
        }>('config_get');
        const ix = cfg?.index;
        // Phase 8.7q.4 — verbose diagnostic logging to debug why density +
        // label_visibility setting reserved for future use (currently
        // labels always show with outward positioning).
        // Phase 8.7q.4 — wider density spread (0.55 / 1.0 / 1.7) so users can
        // actually SEE the difference. Earlier 0.85/1.0/1.2 was a 15% delta,
        // dwarfed by force-collide radius (which is label-width-driven and
        // doesn't scale with density). New values create visible spread.
        switch (ix?.density) {
          case 'compact':  indexDensityScale = 0.55; break;
          case 'spacious': indexDensityScale = 1.70; break;
          default:         indexDensityScale = 1.00; break;
        }
      } catch (err) {
        // eslint-disable-next-line no-console
        console.warn('[IndexGraph] config_get failed:', err);
      }
    }
    void loadIndexConfig();

    // Phase 8.7q.1 — Settings panel dispatches `rift:config-changed` on every
    // successful config_save so config-driven UI re-reads without remount.
    // Listener bound to window so it fires regardless of focus tree.
    const onConfigChanged = () => { void loadIndexConfig(); };
    window.addEventListener('rift:config-changed', onConfigChanged);

    // ------------------------------------------------------------------
    // Phase 8.7q.3 — vault.update debounce buffer.
    //
    // Without this, every `vault.update` envelope mutates liveNodeMap,
    // which triggers `activeNodes` $derived → triggers the $effect that
    // owns the d3 simulation → tears down + rebuilds the entire force
    // simulation + zoomBehavior + zoom-fit timer. During the boot walk
    // this fires N times (one per vault) at ~50+ vaults — error flood
    // source per the reactive-cycle audit (2026-04-29).
    //
    // Strategy: buffer envelopes in `pendingUpdates` + `pendingDeletes`,
    // commit to liveNodeMap on either (a) walk.complete OR (b) a 150ms
    // quiet window. Single Map mutation per flush → single $effect run.
    // ------------------------------------------------------------------
    const pendingUpdates = new Map<string, VaultNode & {
      crossRefs?: string[];
      parentId?: string | null;
    }>();
    const pendingDeletes = new Set<string>();
    let flushTimer: number | null = null;
    const FLUSH_QUIET_MS = 150;

    function flushPendingVaults(): void {
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
        flushTimer = null;
      }
      if (pendingUpdates.size === 0 && pendingDeletes.size === 0) return;
      const next = new Map(liveNodeMap);
      for (const id of pendingDeletes) {
        next.delete(id);
      }
      for (const [id, node] of pendingUpdates) {
        next.set(id, node);
      }
      pendingDeletes.clear();
      pendingUpdates.clear();
      liveNodeMap = next;
    }

    function scheduleFlush(): void {
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
      }
      flushTimer = window.setTimeout(() => {
        flushTimer = null;
        flushPendingVaults();
      }, FLUSH_QUIET_MS);
    }

    void (async () => {
      try {
        const u = await subscribe({ category: 'index' }, (env) => {
          if (env.kind === 'vault.update') {
            const p = env.payload as VaultUpdatePayload;
            // Phase 8.7q — pulse edges when a vault's file is touched after
            // first-load. `created` events on boot fill the graph and would
            // strobe everything; only `modified` reflects real activity.
            if (p.change_kind === 'modified') {
              pulseVault(p.vault_id);
            }
            if (p.change_kind === 'deleted') {
              pendingUpdates.delete(p.vault_id);
              pendingDeletes.add(p.vault_id);
            } else {
              const raw = p;
              const newNode: VaultNode & {
                crossRefs?: string[];
                parentId?: string | null;
              } = {
                id: p.vault_id,
                kind: inferKind(p.vault_id),
                label: raw.name ? `${p.vault_id} — ${raw.name}` : p.vault_id,
                displayName: raw.name ?? undefined,
                shortLabel: raw.short_label ?? undefined,
                updatedMs: raw.updated_ms ?? undefined,
                crossRefs: raw.cross_refs ?? [],
                parentId: p.parent_vault_id ?? undefined,
                path: p.path,
              };
              pendingDeletes.delete(p.vault_id);
              pendingUpdates.set(p.vault_id, newNode);
            }
            scheduleFlush();
          } else if (env.kind === 'walk.complete') {
            // Flush whatever's pending immediately — boot walk done.
            flushPendingVaults();
            walkComplete = true;
          }
        });

        if (cancelled) {
          void u().catch(() => {});
        } else {
          unsub = u;
        }
      } catch (err) {
        console.warn('[IndexGraph] Category::Index subscribe failed:', err);
        // Ensure static fallback renders even if subscribe fails.
        walkComplete = true;
      }
    })();

    return () => {
      cancelled = true;
      window.removeEventListener('rift:config-changed', onConfigChanged);
      // Phase 8.7q.3 — clear any in-flight vault-update flush timer.
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
        flushTimer = null;
      }
      pendingUpdates.clear();
      pendingDeletes.clear();
      // Phase 8.7q.3 — null the persistent zoom behavior so a remount
      // doesn't reuse a closure over the now-detached `g.zoom-target`.
      zoomBehavior = null;
      void (async () => {
        await unsub?.();
      })();
    };
  });

  // ---------------------------------------------------------------------------
  // Component state
  // ---------------------------------------------------------------------------

  /** SVG root — bound via bind:this; D3 attaches zoom behavior here. */
  let container: SVGSVGElement;

  /** Tree-computed positions by node id. Used by zoom-to-fit and drag. */
  let treePositions = new Map<string, { x: number; y: number }>();

  /** Human-readable kind names for synthetic group nodes. */
  const KIND_NAME: Record<VaultKind, string> = {
    p: 'PROJECTS',
    pr: 'PRACTICES',
    r: 'RESEARCH',
    s: 'SKILLS',
    lore: 'LORE',
    agt: 'AGENTS',
    h: 'HISTORY',
  };

  /** Stable kind ordering for the tree layout (clockwise from 12 o'clock). */
  const KIND_ORDER: VaultKind[] = ['p', 'pr', 'r', 's', 'lore', 'agt', 'h'];

  // ---------------------------------------------------------------------------
  // Per-tick render state (Phase 8.7)
  //
  // D3 mutates simNodes/simLinks in place; the tick handler reads positions
  // and writes them to these arrays. Svelte renders <path> + <g> from these
  // arrays via {#each}. $state.raw avoids deep-proxy overhead at 60 Hz.
  // ---------------------------------------------------------------------------

  interface RenderedNode {
    id: string;
    kind: VaultKind;
    /** Phase 8.7p — the visible top label, e.g. "p006" or "INDEX". */
    label: string;
    /** Phase 8.7p — visible bottom subtitle, e.g. "term-rust" or "rules". */
    subtitle?: string;
    /** Full human name surfaced via SVG <title> on hover (browser-native tooltip). */
    tooltip?: string;
    path?: string;
    isIndex: boolean;
    isGroup: boolean;
    draggable: boolean;
    cursor: 'default' | 'grab' | 'pointer';
    x: number;
    y: number;
    /** Mockup §10.3 visual-state class — drives radius + glow + label brightness. */
    state: NodeState;
    /** Per-state radius — pre-computed at tick time. */
    r: number;
    /** Per-kind glyph rendered inside the circle. */
    glyph: string;
    /** 'start' for leaf labels (extend right), 'middle' for group/index. */
    labelAnchor: 'start' | 'end' | 'middle';
  }

  interface RenderedLink {
    key: string;
    cls: string;
    d: string;
  }

  let renderedNodes = $state.raw<RenderedNode[]>([]);
  let renderedLinks = $state.raw<RenderedLink[]>([]);

  /** Set of expanded category kind keys. Default: all collapsed. */
  let expandedKinds = $state<Set<VaultKind>>(new Set());

  /** Toggle a category's expand/collapse state (single-expand mode). */
  function toggleKind(kind: VaultKind): void {
    if (expandedKinds.has(kind)) {
      expandedKinds = new Set();
    } else {
      expandedKinds = new Set([kind]);
    }
  }

  /** Phase 8.7p — zoom-to-fit fires once per mount; guard prevents replays
   *  on data-update $effect re-runs that would clobber user pan/zoom. */
  let zoomFitDone = false;
  let prevExpandedSnapshot = '';
  // (density change detection is no longer needed — tree layout recomputes deterministically)
  /** Phase 8.7q.3 — persistent zoom behavior. Built once on first $effect
   *  run; re-used (not rebuilt) on subsequent runs. The prior pattern was
   *  rebuilding + re-binding via `svg.call(zoomBehavior.transform, zoomIdentity)`
   *  on every $effect run (data update OR density change), which interrupted
   *  any in-flight zoom transition AND reset the user's pan/zoom every time.
   *  See reactive-cycle audit 2026-04-29. */
  let zoomBehavior: ZoomBehavior<SVGSVGElement, unknown> | null = null;
  /** Outstanding zoom-fit timer; cleared on unmount (r004 leak guard). */
  let pendingZoomFitTimer: number | null = null;
  /** Reactive density scale — when this changes (via Settings save), the
   *  $effect that builds the tree re-runs with the new ROW_HEIGHT. */
  let indexDensityScale = $state(1.0);

  // ---------------------------------------------------------------------------
  // Phase 8.7q — visual flair: edge dash-flow + cursor-blink + boot type-in
  // ---------------------------------------------------------------------------

  /** Vault ids whose edges are currently pulsing (cross-ref activity).
   *  Populated for 1.5s on every `change_kind=modified` envelope; cleared
   *  via setTimeout. Cheap Set comparison in the link render path. */
  let pulsingIds = $state<Set<string>>(new Set());
  /** Outstanding clear-timers per vault id — coalesce repeat-modify into
   *  a single sliding window instead of stacking timers. */
  const pulsingTimers = new Map<string, number>();
  const PULSE_DURATION_MS = 1500;

  /** Boot type-in progress in [0, 1]. 0 = no labels visible; 1 = full text.
   *  Animated once after walk-complete + zoom-fit settle, then frozen at 1. */
  let bootProgress = $state(1); // default 1 so re-mounts (already settled) skip the reveal
  /** Outstanding boot-reveal interval id; cleared on unmount. */
  let pendingBootRevealTimer: number | null = null;
  /** Boot reveal animates exactly once per IndexGraph mount. */
  let bootRevealStarted = false;

  /** Mark a vault as pulsing (cross-ref activity). Restarts the 1.5s
   *  clear timer if the vault was already pulsing. */
  function pulseVault(id: string): void {
    const next = new Set(pulsingIds);
    next.add(id);
    pulsingIds = next;
    const prior = pulsingTimers.get(id);
    if (prior !== undefined) window.clearTimeout(prior);
    const timer = window.setTimeout(() => {
      const after = new Set(pulsingIds);
      after.delete(id);
      pulsingIds = after;
      pulsingTimers.delete(id);
    }, PULSE_DURATION_MS);
    pulsingTimers.set(id, timer);
  }

  const MAX_LABEL_CHARS = 30;
  function truncateLabel(text: string): string {
    if (text.length <= MAX_LABEL_CHARS) return text;
    return text.slice(0, MAX_LABEL_CHARS - 1) + '…';
  }

  /** Truncate a label to the boot-progress window, preserving full text once
   *  bootProgress >= 1. The slice is by characters (not bytes) since labels
   *  are ASCII; multi-codepoint glyphs would need a [...str] split. */
  function bootRevealLabel(text: string): string {
    if (bootProgress >= 1) return text;
    const visibleCount = Math.ceil(text.length * bootProgress);
    return text.slice(0, visibleCount);
  }

  /** Currently hovered node id — drives `.hovered` class on the <g>. */
  let hoveredId = $state<string | null>(null);

  /**
   * Drag ghost — a fixed-position element that follows the cursor while a
   * vault node is being dragged. Lets the gesture visually escape the
   * IndexGraph pane (which is clipped by overflow:hidden in App.svelte's
   * .graph-pane / .graph-body) so the user can drop onto the terminal on
   * the left half of the cockpit. `pointer-events: none` keeps elementFromPoint
   * hit-testing correct on mouseup (so .terminal-host is reachable).
   */
  let ghostVisible = $state(false);
  let ghostX = $state(0);
  let ghostY = $state(0);
  let ghostLabel = $state('');
  let ghostKind = $state<VaultKind>('p');

  /** Node id currently being dragged — drives `.dragging` class on the source <g>. */
  let draggingId = $state<string | null>(null);

  // ---------------------------------------------------------------------------
  // Manual-gesture drag-into-terminal (Phase 8.7 — replaces HTML5 drag)
  //
  // WHY MANUAL: WebView2 does NOT initiate HTML5 drag on SVG `<g>` elements
  // even when `draggable="true"` is set as an HTML attribute. The runtime
  // diagnostic 2026-04-29 confirmed: `g.getAttribute('draggable') === "true"`
  // but `g.draggable === undefined` (the IDL property is HTMLElement-only).
  // No `dragstart` event ever fires — captured at window level via
  // `addEventListener('dragstart', ..., true)`. So we sidestep HTML5 drag
  // and run our own gesture: mousedown → mousemove (threshold) → mouseup
  // hit-test against `.terminal-host` → dispatch RIFT_VAULT_DROP_EVENT.
  //
  // The d3-zoom filter still yields on vault-node mousedowns (so the canvas
  // does not pan when a gesture starts), but no longer relies on the
  // `[draggable="true"]` attribute — it uses the `index-root` class check.
  // ---------------------------------------------------------------------------

  /** Pixel threshold the cursor must move before mousedown promotes to drag. */
  const DRAG_THRESHOLD_PX = 5;

  let dragNode: RenderedNode | null = null;
  let dragStartX = 0;
  let dragStartY = 0;
  let dragActive = false;

  function onNodeMouseDown(e: MouseEvent, node: RenderedNode): void {
    if (e.button !== 0) return;       // left-click only
    if (node.isIndex) return;         // INDEX is not a drag source
    dragNode = node;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    dragActive = false;
    document.addEventListener('mousemove', onDocMouseMove);
    document.addEventListener('mouseup', onDocMouseUp);
    // Prevent text selection during the gesture.
    e.preventDefault();
  }

  function onDocMouseMove(e: MouseEvent): void {
    if (!dragNode) return;
    if (!dragActive) {
      const dx = Math.abs(e.clientX - dragStartX);
      const dy = Math.abs(e.clientY - dragStartY);
      if (dx > DRAG_THRESHOLD_PX || dy > DRAG_THRESHOLD_PX) {
        dragActive = true;
        document.body.style.cursor = 'grabbing';
        // Activate the drag ghost — the visual representation of the
        // gesture in viewport coordinates. The source node in the graph
        // stays put; the ghost is what the user sees crossing the
        // .graph-pane overflow:hidden boundary onto the terminal.
        ghostLabel = dragNode.id;
        ghostKind = dragNode.kind;
        ghostVisible = true;
        draggingId = dragNode.id;
      }
    }
    if (!dragActive) return;
    // Ghost tracks cursor in viewport coords (escapes clipping ancestors).
    // No fx/fy mutation on the simulation node — keeps zero shake on the
    // graph. Standard "drag a copy onto a drop target" UX, not "rearrange
    // the graph layout." The radial layout is auto-positioned and would
    // fight any user-imposed positioning anyway.
    ghostX = e.clientX;
    ghostY = e.clientY;
  }

  function onDocMouseUp(e: MouseEvent): void {
    document.removeEventListener('mousemove', onDocMouseMove);
    document.removeEventListener('mouseup', onDocMouseUp);
    document.body.style.cursor = '';

    const node = dragNode;
    const wasActive = dragActive;
    dragNode = null;
    dragActive = false;
    ghostVisible = false;
    draggingId = null;

    if (!wasActive || !node) return;  // mouseup without crossing threshold = a click, not a drag

    // No fx/fy to clear (we don't pin during drag — see onDocMouseMove for
    // rationale). Hit-test the cursor for a terminal drop target.
    const target = document.elementFromPoint(e.clientX, e.clientY);
    const terminalHost = target?.closest('.terminal-host');
    if (!terminalHost) return;

    const payload = node.path ?? node.id;
    const detail: RiftVaultDropDetail = { path: payload };
    terminalHost.dispatchEvent(
      new CustomEvent(RIFT_VAULT_DROP_EVENT, { detail, bubbles: true }),
    );
  }

  // ---------------------------------------------------------------------------
  // Radial ring layout $effect — re-runs when activeNodes, activeEdges,
  // expandedKinds, or indexDensityScale change. Hand-computed radial
  // positions (no d3.tree / force simulation). Category ring + expand/collapse.
  // ---------------------------------------------------------------------------

  interface TreeChild {
    id: string;
    kind: VaultKind;
    label: string;
    shortLabel?: string;
    displayName?: string;
    updatedMs?: number;
    path?: string;
    children?: TreeChild[];
  }

  function buildTopLevelList(
    nodes: (VaultNode & { parentId?: string | null })[]
  ): TreeChild[] {
    const datumMap = new Map<string, TreeChild>();
    for (const n of nodes) {
      datumMap.set(n.id, {
        id: n.id,
        kind: n.kind,
        label: n.id,
        shortLabel: n.shortLabel,
        displayName: n.displayName,
        updatedMs: n.updatedMs,
        path: n.path,
      });
    }
    const topLevel: TreeChild[] = [];
    for (const n of nodes) {
      const datum = datumMap.get(n.id)!;
      if (n.parentId && datumMap.has(n.parentId)) {
        const parent = datumMap.get(n.parentId)!;
        if (!parent.children) parent.children = [];
        parent.children.push(datum);
      } else {
        topLevel.push(datum);
      }
    }
    return topLevel;
  }

  $effect(() => {
    if (!container) return;

    const vaultNodes: VaultNode[] = activeNodes.map((n) => ({ ...n }));
    const vaultLinks: VaultLink[] = activeEdges.map((l) => ({ ...l }));

    if (vaultNodes.length === 0) return;

    const rect = container.getBoundingClientRect();
    const W = rect.width  || 640;
    const H = rect.height || 480;
    const INDEX_ID = '__INDEX__';
    const CENTER_X = 0;
    const CENTER_Y = 0;
    const RING_RADIUS = 180 * indexDensityScale;
    const CHILD_OFFSET_OUTWARD = 100;
    const CHILD_ROW_HEIGHT = 58 * indexDensityScale;

    // ------------------------------------------------------------------
    // Build kind groups from vault nodes
    // ------------------------------------------------------------------
    const kindGroups = new Map<VaultKind, (VaultNode & { parentId?: string | null })[]>();
    for (const node of vaultNodes) {
      const extra = node as VaultNode & { parentId?: string | null };
      const group = kindGroups.get(node.kind) ?? [];
      group.push(extra);
      kindGroups.set(node.kind, group);
    }

    // ------------------------------------------------------------------
    // Tier 1 — Category ring: radial layout around INDEX
    // ------------------------------------------------------------------
    const presentKinds = KIND_ORDER.filter((k) => kindGroups.has(k));
    const angleStep = (2 * Math.PI) / presentKinds.length;
    const startAngle = -Math.PI / 2;

    const posMap = new Map<string, { x: number; y: number }>();
    posMap.set(INDEX_ID, { x: CENTER_X, y: CENTER_Y });

    for (let i = 0; i < presentKinds.length; i++) {
      const kind = presentKinds[i];
      const angle = startAngle + i * angleStep;
      const groupId = `__GROUP_${kind}__`;
      posMap.set(groupId, {
        x: CENTER_X + Math.cos(angle) * RING_RADIUS,
        y: CENTER_Y + Math.sin(angle) * RING_RADIUS,
      });
    }

    // ------------------------------------------------------------------
    // Tier 2 — Expanded children: outward fan from category node
    // ------------------------------------------------------------------
    for (const kind of presentKinds) {
      if (!expandedKinds.has(kind)) continue;

      const groupId = `__GROUP_${kind}__`;
      const groupPos = posMap.get(groupId)!;
      const nodes = kindGroups.get(kind)!;

      const dx = groupPos.x - CENTER_X;
      const dy = groupPos.y - CENTER_Y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const dirX = dx / dist;
      const dirY = dy / dist;

      let perpX = -dirY;
      let perpY = dirX;

      // Guarantee minimum vertical component so horizontal labels don't
      // overlap when a group sits at 12/6 o'clock (pure horizontal perp).
      const MIN_VERT = 0.55;
      if (Math.abs(perpY) < MIN_VERT) {
        const sign = perpY >= 0 ? 1 : -1;
        perpY = sign * MIN_VERT;
        perpX = Math.sign(perpX || 1) * Math.sqrt(1 - perpY * perpY);
      }

      const startX = groupPos.x + dirX * CHILD_OFFSET_OUTWARD;
      const startY = groupPos.y + dirY * CHILD_OFFSET_OUTWARD;

      const topLevel = buildTopLevelList(nodes);

      let totalSlots = 0;
      for (const child of topLevel) {
        totalSlots++;
        if (child.children) totalSlots += child.children.length;
      }
      const totalHeight = (totalSlots - 1) * CHILD_ROW_HEIGHT;
      const offsetStart = -totalHeight / 2;

      let slot = 0;
      for (let j = 0; j < topLevel.length; j++) {
        const child = topLevel[j];
        const fanOffset = offsetStart + slot * CHILD_ROW_HEIGHT;
        posMap.set(child.id, {
          x: startX + perpX * fanOffset,
          y: startY + perpY * fanOffset,
        });
        slot++;

        if (child.children) {
          for (let k = 0; k < child.children.length; k++) {
            const sub = child.children[k];
            const subFanOffset = offsetStart + slot * CHILD_ROW_HEIGHT;
            posMap.set(sub.id, {
              x: startX + dirX * 110 + perpX * subFanOffset,
              y: startY + dirY * 110 + perpY * subFanOffset,
            });
            slot++;
          }
        }
      }
    }

    treePositions = posMap;

    // ------------------------------------------------------------------
    // Build rendered links
    // ------------------------------------------------------------------
    const allLinks: RenderedLink[] = [];

    // Index spokes — INDEX → each category group (always visible)
    for (const kind of presentKinds) {
      const groupId = `__GROUP_${kind}__`;
      const srcPos = posMap.get(INDEX_ID);
      const tgtPos = posMap.get(groupId);
      if (!srcPos || !tgtPos) continue;
      const midX = (srcPos.x + tgtPos.x) / 2;
      const midY = (srcPos.y + tgtPos.y) / 2;
      const d = `M${srcPos.x},${srcPos.y} Q${midX},${midY} ${tgtPos.x},${tgtPos.y}`;
      allLinks.push({
        key: `${INDEX_ID}->${groupId}`,
        cls: 'graph-edge index-spoke',
        d,
      });
    }

    // Tree edges — group → children (only for expanded groups)
    for (const kind of presentKinds) {
      if (!expandedKinds.has(kind)) continue;
      const groupId = `__GROUP_${kind}__`;
      const groupPos = posMap.get(groupId);
      if (!groupPos) continue;
      const nodes = kindGroups.get(kind)!;
      const topLevel = buildTopLevelList(nodes);
      for (const child of topLevel) {
        const childPos = posMap.get(child.id);
        if (!childPos) continue;
        const midX = (groupPos.x + childPos.x) / 2;
        const midY = (groupPos.y + childPos.y) / 2;
        const d = `M${groupPos.x},${groupPos.y} Q${midX},${midY} ${childPos.x},${childPos.y}`;
        let cls = 'graph-edge tree-edge';
        if (pulsingIds.has(child.id)) cls += ' pulsing';
        allLinks.push({ key: `${groupId}->${child.id}`, cls, d });

        if (child.children) {
          for (const sub of child.children) {
            const subPos = posMap.get(sub.id);
            if (!subPos) continue;
            const smX = (childPos.x + subPos.x) / 2;
            const smY = (childPos.y + subPos.y) / 2;
            const sd = `M${childPos.x},${childPos.y} Q${smX},${smY} ${subPos.x},${subPos.y}`;
            let scls = 'graph-edge tree-edge';
            if (pulsingIds.has(sub.id)) scls += ' pulsing';
            allLinks.push({ key: `${child.id}->${sub.id}`, cls: scls, d: sd });
          }
        }
      }
    }

    // Cross-ref edges — only render if BOTH endpoints are visible
    for (const link of vaultLinks) {
      const srcPos = posMap.get(link.source);
      const tgtPos = posMap.get(link.target);
      if (!srcPos || !tgtPos) continue;
      const d = `M${srcPos.x},${srcPos.y} L${tgtPos.x},${tgtPos.y}`;
      let cls = 'graph-edge cross-ref';
      if (pulsingIds.has(link.source) || pulsingIds.has(link.target)) {
        cls += ' pulsing';
      }
      allLinks.push({ key: `xref:${link.source}->${link.target}`, cls, d });
    }
    renderedLinks = allLinks;

    // ------------------------------------------------------------------
    // Build rendered nodes from tree
    // ------------------------------------------------------------------
    const now = Date.now();
    const densityNodeScale = indexDensityScale <= 0.55 ? 0.8 : indexDensityScale >= 1.7 ? 1.2 : 1.0;
    const LEAF_RADIUS: Record<NodeState, number> = {
      active:     Math.round(7 * densityNodeScale),
      recent:     Math.round(6 * densityNodeScale),
      ambient:    Math.max(3, Math.round(5 * densityNodeScale)),
      background: Math.max(3, Math.round(4 * densityNodeScale)),
    };
    const GROUP_RADIUS = Math.round(10 * densityNodeScale);

    const allNodes: RenderedNode[] = [];

    // INDEX root node
    const indexPos = posMap.get(INDEX_ID)!;
    allNodes.push({
      id: INDEX_ID,
      kind: 'p',
      label: 'INDEX',
      subtitle: undefined,
      tooltip: undefined,
      path: undefined,
      isIndex: true,
      isGroup: false,
      draggable: false,
      cursor: 'default',
      x: indexPos.x,
      y: indexPos.y,
      state: 'ambient',
      r: 14,
      glyph: '',
      labelAnchor: 'middle',
    });

    // Group nodes + their visible children
    for (const kind of presentKinds) {
      const groupId = `__GROUP_${kind}__`;
      const groupPos = posMap.get(groupId);
      if (!groupPos) continue;

      const childCount = kindGroups.get(kind)?.length ?? 0;
      const isExpanded = expandedKinds.has(kind);

      allNodes.push({
        id: groupId,
        kind,
        label: isExpanded ? KIND_NAME[kind] : `${KIND_NAME[kind]} (${childCount})`,
        subtitle: undefined,
        tooltip: KIND_NAME[kind],
        path: undefined,
        isIndex: false,
        isGroup: true,
        draggable: false,
        cursor: 'pointer',
        x: groupPos.x,
        y: groupPos.y,
        state: 'ambient',
        r: GROUP_RADIUS,
        glyph: KIND_GLYPH[kind] ?? '',
        labelAnchor: 'middle',
      });

      // Leaf nodes — only if this kind is expanded
      if (!expandedKinds.has(kind)) continue;
      const nodes = kindGroups.get(kind)!;
      const topLevel = buildTopLevelList(nodes);
      for (const child of topLevel) {
        const childPos = posMap.get(child.id);
        if (!childPos) continue;

        let state: NodeState = 'ambient';
        if (child.id === hoveredId) state = 'active';
        else if (child.updatedMs && now - child.updatedMs <= RECENT_WINDOW_MS) state = 'recent';

        allNodes.push({
          id: child.id,
          kind: child.kind,
          label: child.shortLabel ? `${child.id} — ${child.shortLabel}`
            : child.displayName ? `${child.id} — ${child.displayName}`
            : child.id,
          subtitle: undefined,
          tooltip: child.displayName ?? child.id,
          path: child.path,
          isIndex: false,
          isGroup: false,
          draggable: true,
          cursor: 'grab',
          x: childPos.x,
          y: childPos.y,
          state,
          r: LEAF_RADIUS[state],
          glyph: '',
          labelAnchor: childPos.x >= CENTER_X ? 'start' : 'end',
        });

        if (child.children) {
          for (const sub of child.children) {
            const subPos = posMap.get(sub.id);
            if (!subPos) continue;

            let subState: NodeState = 'ambient';
            if (sub.id === hoveredId) subState = 'active';
            else if (sub.updatedMs && now - sub.updatedMs <= RECENT_WINDOW_MS) subState = 'recent';

            allNodes.push({
              id: sub.id,
              kind: sub.kind,
              label: sub.shortLabel ? `${sub.id} — ${sub.shortLabel}`
                : sub.displayName ? `${sub.id} — ${sub.displayName}`
                : sub.id,
              subtitle: undefined,
              tooltip: sub.displayName ?? sub.id,
              path: sub.path,
              isIndex: false,
              isGroup: false,
              draggable: true,
              cursor: 'grab',
              x: subPos.x,
              y: subPos.y,
              state: subState,
              r: LEAF_RADIUS[subState],
              glyph: '',
              labelAnchor: subPos.x >= CENTER_X ? 'start' : 'end',
            });
          }
        }
      }
    }
    renderedNodes = allNodes;


    // ------------------------------------------------------------------
    // Pan + zoom
    // ------------------------------------------------------------------
    const svg = select<SVGSVGElement, unknown>(container);
    const g   = svg.select<SVGGElement>('g.zoom-target');

    if (zoomBehavior === null) {
      zoomBehavior = zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.3, 3])
        .filter((event: Event) => {
          if (event.type === 'wheel') return true;
          const target = event.target as Element | null;
          if (target?.closest('g.graph-node')) return false;
          const e = event as MouseEvent & { ctrlKey: boolean; button: number };
          return !e.ctrlKey && !e.button;
        })
        .on('zoom', (event: { transform: ZoomTransform }) => {
          g.attr('transform', event.transform.toString());
        });

      svg.call(zoomBehavior);
      svg.call(zoomBehavior.transform, zoomIdentity);
    }

    // ------------------------------------------------------------------
    // Zoom-to-fit (tree positions are immediate — no settle delay needed)
    // ------------------------------------------------------------------
    if (!zoomFitDone && vaultNodes.length >= 3) {
      zoomFitDone = true;

      if (!bootRevealStarted) {
        bootRevealStarted = true;
        bootProgress = 0;
        const REVEAL_DURATION_MS = 700;
        const startedAt = performance.now();
        const tickReveal = () => {
          const elapsed = performance.now() - startedAt;
          const t = Math.min(1, elapsed / REVEAL_DURATION_MS);
          bootProgress = t;
          if (t < 1) {
            pendingBootRevealTimer = window.setTimeout(tickReveal, 30);
          } else {
            pendingBootRevealTimer = null;
          }
        };
        pendingBootRevealTimer = window.setTimeout(tickReveal, 150);
      }

      // Tree layout is instant — fit immediately (small delay for DOM settle)
      const fitTimeout = window.setTimeout(() => {
        if (!container) return;
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const pos of treePositions.values()) {
          if (pos.x < minX) minX = pos.x;
          if (pos.y < minY) minY = pos.y;
          if (pos.x > maxX) maxX = pos.x;
          if (pos.y > maxY) maxY = pos.y;
        }
        if (!isFinite(minX)) return;
        // Labels extend rightward from leaf nodes — add margin for text.
        const LABEL_MARGIN_RIGHT = 140;
        const LABEL_MARGIN_LEFT = 20;
        const LABEL_MARGIN_Y = 20;
        minX -= LABEL_MARGIN_LEFT;
        maxX += LABEL_MARGIN_RIGHT;
        minY -= LABEL_MARGIN_Y;
        maxY += LABEL_MARGIN_Y;
        const bw = maxX - minX;
        const bh = maxY - minY;
        if (bw <= 0 || bh <= 0) return;
        const margin = indexDensityScale <= 0.55 ? 20
                     : indexDensityScale >= 1.7  ? 40
                     : 30;
        // Cap minimum scale so text stays readable (≥5px at 9px base).
        const kRaw = Math.min(
          (W - margin * 2) / bw,
          (H - margin * 2) / bh,
          2.0,
        );
        const k = Math.max(0.55, kRaw);
        const tx = W / 2 - ((minX + maxX) / 2) * k;
        const ty = H / 2 - ((minY + maxY) / 2) * k;
        const fitTransform = zoomIdentity.translate(tx, ty).scale(k);
        if (zoomBehavior) {
          svg.call(zoomBehavior.transform, fitTransform);
        }
      }, 100);
      pendingZoomFitTimer = fitTimeout;
    }

    // ------------------------------------------------------------------
    // Re-fit on expand/collapse toggle (animated 300ms transition)
    // ------------------------------------------------------------------
    const expandedSnapshot = [...expandedKinds].sort().join(',');
    if (expandedSnapshot !== prevExpandedSnapshot) {
      prevExpandedSnapshot = expandedSnapshot;
      const fitTimeout2 = window.setTimeout(() => {
        if (!container || !zoomBehavior) return;
        let minX = Infinity, minY = Infinity;
        let maxX = -Infinity, maxY = -Infinity;
        for (const pos of posMap.values()) {
          if (pos.x < minX) minX = pos.x;
          if (pos.y < minY) minY = pos.y;
          if (pos.x > maxX) maxX = pos.x;
          if (pos.y > maxY) maxY = pos.y;
        }
        if (!isFinite(minX)) return;
        const LABEL_MARGIN = 100;
        minX -= LABEL_MARGIN;
        maxX += LABEL_MARGIN;
        minY -= LABEL_MARGIN;
        maxY += LABEL_MARGIN;
        const bw = maxX - minX;
        const bh = maxY - minY;
        if (bw <= 0 || bh <= 0) return;
        const margin = 30;
        const rect2 = container.getBoundingClientRect();
        const W2 = rect2.width || 640;
        const H2 = rect2.height || 480;
        const k = Math.max(0.4, Math.min(
          (W2 - margin * 2) / bw,
          (H2 - margin * 2) / bh,
          2.0,
        ));
        const tx = W2 / 2 - ((minX + maxX) / 2) * k;
        const ty = H2 / 2 - ((minY + maxY) / 2) * k;
        const fitTransform = zoomIdentity.translate(tx, ty).scale(k);
        const svgEl = select<SVGSVGElement, unknown>(container);
        svgEl.call(zoomBehavior!.transform, fitTransform);
      }, 50);
      pendingZoomFitTimer = fitTimeout2;
    }

    // ------------------------------------------------------------------
    // CLEANUP
    // ------------------------------------------------------------------
    return () => {
      if (pendingZoomFitTimer !== null) {
        window.clearTimeout(pendingZoomFitTimer);
        pendingZoomFitTimer = null;
      }
      if (pendingBootRevealTimer !== null) {
        window.clearTimeout(pendingBootRevealTimer);
        pendingBootRevealTimer = null;
      }
      for (const t of pulsingTimers.values()) {
        window.clearTimeout(t);
      }
      pulsingTimers.clear();
    };
  });
</script>

<!-- IndexGraph — Svelte renders nodes/links via {#each}; D3 provides zoom only. -->
<div class="index-graph-wrapper">
  {#if !walkComplete && liveNodeMap.size === 0}
    <!--
      Loading state — walker hasn't completed yet.
      Shows until walk.complete arrives or subscribe fails.
    -->
    <div class="index-graph-overlay">
      <span class="overlay-text">scanning vault...</span>
    </div>
  {/if}

  <svg
    bind:this={container}
    class="index-graph"
    aria-label="Abyssal Index vault graph"
  >
    <defs>
      <!--
        Amber-glow SVG filter — applied to nodes on hover via CSS class.
        Matches §10.3 non-negotiable aesthetic (amber-bright = #f59e0b).
      -->
      <filter id="amber-glow" x="-50%" y="-50%" width="200%" height="200%">
        <feGaussianBlur in="SourceGraphic" stdDeviation="3" result="blur" />
        <feDropShadow
          dx="0"
          dy="0"
          stdDeviation="3"
          flood-color="#f59e0b"
          flood-opacity="0.85"
        />
      </filter>
      <!--
        Faint-glow variant — always-on ambient glow for unselected nodes.
      -->
      <filter id="ambient-glow" x="-30%" y="-30%" width="160%" height="160%">
        <feGaussianBlur in="SourceGraphic" stdDeviation="1.5" result="blur" />
        <feDropShadow
          dx="0"
          dy="0"
          stdDeviation="2"
          flood-color="#D4890A"
          flood-opacity="0.35"
        />
      </filter>
    </defs>

    <!--
      zoom-target: d3-zoom transforms this <g> for pan/zoom (Phase 8.3).
      Phase 8.7: Svelte owns the rendering inside via {#each}; D3 runs the
      simulation only. The mirror of Tree.svelte's working SVG-drag pattern.
    -->
    <g class="zoom-target">
      <g class="links">
        {#each renderedLinks as link (link.key)}
          <path
            class={link.cls}
            d={link.d}
          />
        {/each}
      </g>
      <g class="nodes">
        {#each renderedNodes as node (node.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
          <g
            class={(node.isIndex ? 'graph-node index-root' : node.isGroup ? `graph-node group-node node-${node.kind}` : `graph-node node-${node.kind} state-${node.state}`) +
                   (hoveredId === node.id ? ' hovered' : '') +
                   (draggingId === node.id ? ' dragging-source' : '')}
            cursor={node.cursor}
            transform="translate({node.x},{node.y})"
            onmouseenter={() => (hoveredId = node.id)}
            onmouseleave={() => {
              if (hoveredId === node.id) hoveredId = null;
            }}
            onmousedown={(e) => onNodeMouseDown(e, node)}
            onclick={() => {
              if (node.isGroup) toggleKind(node.kind);
            }}
          >
            {#if node.tooltip}
              <title>{node.tooltip}</title>
            {/if}
            <circle
              r={node.r}
              class={node.isIndex ? 'node-circle index-root' : node.isGroup ? `node-circle group-node node-${node.kind}` : `node-circle node-${node.kind} state-${node.state}`}
            />
            {#if node.isGroup && node.glyph}
              <text
                class="node-glyph group-glyph"
                text-anchor="middle"
                dominant-baseline="middle"
              >
                {node.glyph}
              </text>
            {/if}
            {#if node.isIndex}
              <text
                class="node-label index-root"
                dy={-(node.r + 6)}
                text-anchor="middle"
              >
                {bootRevealLabel(node.label)}
              </text>
            {:else if node.isGroup}
              <text
                class="node-label group-label"
                dy={-(node.r + 4)}
                text-anchor="middle"
              >
                {bootRevealLabel(node.label)}
              </text>
              <text
                class="node-expand-indicator"
                dx={node.r + 2}
                dy={-node.r + 2}
                text-anchor="start"
                font-size="8"
                style="pointer-events: none;"
              >
                {expandedKinds.has(node.kind) ? '−' : '+'}
              </text>
            {:else}
              <text
                class="node-label leaf-label state-{node.state}"
                dx={node.labelAnchor === 'start' ? node.r + 6 : -(node.r + 6)}
                dy="3"
                text-anchor={node.labelAnchor}
              >
                {truncateLabel(node.label)}
              </text>
            {/if}
          </g>
        {/each}
      </g>
    </g>
  </svg>

  <!--
    Drag ghost — fixed-positioned visual that follows the cursor while a
    vault node is being dragged so the gesture isn't clipped by the
    .graph-pane overflow:hidden constraint. Lets the user visually carry
    the node over the terminal pane and drop it there.
    pointer-events:none → elementFromPoint hit-tests the underlying
    .terminal-host on mouseup, not the ghost.
  -->
  {#if ghostVisible}
    <div
      class="drag-ghost drag-ghost-kind-{ghostKind}"
      style="left: {ghostX}px; top: {ghostY}px;"
      aria-hidden="true"
    >
      <span class="drag-ghost-dot"></span>
      <span class="drag-ghost-label">{ghostLabel}</span>
    </div>
  {/if}
</div>

<style>
  /* -------------------------------------------------------------------------
     Wrapper — positions the overlay above the SVG
     ------------------------------------------------------------------------- */
  .index-graph-wrapper {
    position: relative;
    display: block;
    width: 100%;
    height: 100%;
  }

  /* -------------------------------------------------------------------------
     IndexGraph layout — full-bleed inside whatever slot App.svelte / cockpit provides
     ------------------------------------------------------------------------- */
  .index-graph {
    display: block;
    width: 100%;
    height: 100%;
    background: var(--bg-base);
    user-select: none;
    -webkit-user-select: none;
  }

  /* -------------------------------------------------------------------------
     Loading overlay — shown while walk hasn't completed and no live data
     ------------------------------------------------------------------------- */
  .index-graph-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    z-index: 1;
  }

  .overlay-text {
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    color: var(--amber-dim);
    opacity: 0.6;
    letter-spacing: 0.05em;
    animation: overlay-pulse 2s ease-in-out infinite;
  }

  @keyframes overlay-pulse {
    0%, 100% { opacity: 0.4; }
    50%       { opacity: 0.8; }
  }

  /* -------------------------------------------------------------------------
     Edge lines — faint amber stroke
     ------------------------------------------------------------------------- */
  :global(.graph-edge) {
    stroke: var(--amber-faint);
    stroke-width: 1;
    opacity: 0.45;
    fill: none;
  }

  /* Phase 8.7q — dash-flow on cross-ref activity. Fires for 1.5s when a
     vault's file is modified; both endpoints' edges animate. The dash
     pattern moves along the line at 1 cycle / 0.6s, like a packet
     traveling through the wire. */
  :global(.graph-edge.pulsing) {
    stroke: var(--amber-primary);
    stroke-width: 1.5;
    opacity: 0.95;
    stroke-dasharray: 6 4;
    animation: edge-dash-flow 0.6s linear infinite;
    filter: drop-shadow(0 0 3px rgba(212, 137, 10, 0.6));
  }
  @keyframes edge-dash-flow {
    from { stroke-dashoffset: 0; }
    to   { stroke-dashoffset: -10; }
  }

  /* Phase 8.7q — terminal cursor blink on the active node's label. 1Hz
     square-wave (50% duty) matches real CRT cursor cadence rather than the
     softer sine-fade you'd see on a modern web cursor. */
  :global(.cursor-blink) {
    fill: var(--amber-bright);
    animation: cursor-blink-1hz 1s step-end infinite;
  }
  @keyframes cursor-blink-1hz {
    0%, 49.9%   { opacity: 1; }
    50%, 100%   { opacity: 0; }
  }

  /* -------------------------------------------------------------------------
     Node circles — base style; category color via kind-prefixed class
     ------------------------------------------------------------------------- */
  :global(.node-circle) {
    stroke-width: 1.5;
    fill: var(--bg-elevated);
    filter: url(#ambient-glow);
    transition: filter 0.15s ease, stroke-width 0.15s ease;
  }

  /* Per-kind colors — diverged from all-amber mockup for readability.
     Uses §10.1 lane palette so colors are semantically consistent. */
  :global(.node-circle.node-p)    { stroke: #f59e0b; fill: rgba(245, 158, 11, 0.12); }
  :global(.node-circle.node-pr)   { stroke: #4a9eff; fill: rgba(74, 158, 255, 0.12); }
  :global(.node-circle.node-r)    { stroke: #33CC33; fill: rgba(51, 204, 51, 0.12); }
  :global(.node-circle.node-s)    { stroke: #4ad4d4; fill: rgba(74, 212, 212, 0.12); }
  :global(.node-circle.node-lore) { stroke: #b078e8; fill: rgba(176, 120, 232, 0.12); }
  :global(.node-circle.node-agt)  { stroke: #CC3333; fill: rgba(204, 51, 51, 0.12); }
  :global(.node-circle.node-h)    { stroke: #7a6420; fill: rgba(90, 68, 16, 0.12); }

  /* Group nodes — semi-transparent filled, thicker stroke */
  :global(.node-circle.group-node) {
    stroke-width: 2;
    filter: none;
  }
  :global(.node-circle.group-node.node-p)    { fill: rgba(245, 158, 11, 0.2); }
  :global(.node-circle.group-node.node-pr)   { fill: rgba(74, 158, 255, 0.2); }
  :global(.node-circle.group-node.node-r)    { fill: rgba(51, 204, 51, 0.2); }
  :global(.node-circle.group-node.node-s)    { fill: rgba(74, 212, 212, 0.2); }
  :global(.node-circle.group-node.node-lore) { fill: rgba(176, 120, 232, 0.2); }
  :global(.node-circle.group-node.node-agt)  { fill: rgba(204, 51, 51, 0.2); }
  :global(.node-circle.group-node.node-h)    { fill: rgba(90, 68, 16, 0.2); }

  /* INDEX root — solid amber-bright fill, larger radius set inline (r=16). */
  :global(.node-circle.index-root) {
    fill: var(--amber-bright);
    stroke: var(--amber-bright);
    stroke-width: 2;
    filter: url(#amber-glow);
  }

  /* INDEX-spoke edges — fainter than cross-vault links so the radial
     scaffolding doesn't dominate over genuine cross-references. */
  :global(.graph-edge.index-spoke) {
    stroke: var(--amber-faint);
    opacity: 0.22;
  }

  /* Tree hierarchy edges — solid, slightly brighter than spokes. */
  :global(.graph-edge.tree-edge) {
    stroke: var(--amber-dim, #8a6d2b);
    stroke-width: 1;
    opacity: 0.55;
  }

  /* Cross-ref edges — dashed overlay so they're visually distinct from
     the tree structure. Thinner and more transparent than tree edges. */
  :global(.graph-edge.cross-ref) {
    stroke: var(--term-cyan);
    stroke-width: 0.8;
    stroke-dasharray: 4 3;
    opacity: 0.3;
  }

  /* Hover state */
  :global(.graph-node.hovered .node-circle) {
    filter: url(#amber-glow);
    stroke: var(--amber-bright);
    stroke-width: 2;
  }

  :global(.graph-node.group-node) {
    cursor: pointer;
  }
  :global(.graph-node.group-node:hover .node-circle) {
    filter: url(#amber-glow);
    stroke-width: 2;
  }

  :global(.node-expand-indicator) {
    font-family: 'JetBrains Mono', monospace;
    font-weight: 700;
    fill: var(--amber-dim);
    opacity: 0.5;
    transition: opacity 0.2s;
  }
  :global(.graph-node.group-node:hover .node-expand-indicator) {
    opacity: 1;
    fill: var(--amber-bright);
  }

  /* -------------------------------------------------------------------------
     Node labels (Phase 8.7p — id on top, short tagline subtitle below)
     ------------------------------------------------------------------------- */
  :global(.node-label) {
    fill: var(--amber-dim);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.04em;
    pointer-events: none;
    user-select: none;
  }
  :global(.node-label.leaf-label) { font-size: 9px; font-weight: 500; }
  :global(.node-label.leaf-label.state-active)     { fill: var(--amber-bright); font-weight: 700; }
  :global(.node-label.leaf-label.state-recent)     { fill: var(--amber-primary); font-weight: 600; }
  :global(.node-label.leaf-label.state-ambient)    { fill: var(--amber-warm); }
  :global(.node-label.leaf-label.state-background) { fill: var(--amber-faint); opacity: 0.7; }

  :global(.node-label.group-label) {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    fill: var(--amber-warm);
  }

  :global(.node-subtitle) {
    fill: var(--amber-faint);
    font-family: 'JetBrains Mono', monospace;
    font-size: 7px;
    font-weight: 500;
    font-style: italic;
    letter-spacing: 0.02em;
    pointer-events: none;
    user-select: none;
    opacity: 0.85;
  }
  :global(.node-subtitle.state-active)     { fill: var(--amber-bright); opacity: 1; }
  :global(.node-subtitle.state-recent)     { fill: var(--amber-primary); opacity: 1; }

  :global(.node-glyph) {
    fill: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 700;
    pointer-events: none;
    user-select: none;
  }
  :global(.node-glyph.group-glyph) { font-size: 10px; }

  :global(.node-label.index-root) {
    fill: var(--amber-bright);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.15em;
  }

  /* -------------------------------------------------------------------------
     Per-state node circle (mockup §10.3 visual hierarchy)
     ------------------------------------------------------------------------- */
  :global(.node-circle.state-active) {
    fill: var(--bg-elevated);
    stroke-width: 2;
    filter: drop-shadow(0 0 12px rgba(245, 158, 11, 0.85));
    animation: index-pulse-glow 1.6s ease-in-out infinite;
  }
  :global(.node-circle.state-recent) {
    fill: var(--bg-elevated);
    stroke-width: 1.8;
    filter: drop-shadow(0 0 6px rgba(212, 137, 10, 0.55));
  }
  :global(.node-circle.state-ambient) {
    fill: var(--bg-elevated);
    stroke-width: 1.5;
    filter: drop-shadow(0 0 3px rgba(176, 122, 18, 0.3));
  }
  :global(.node-circle.state-background) {
    fill: var(--bg-surface);
    stroke-width: 1;
    opacity: 0.55;
  }

  @keyframes index-pulse-glow {
    0%, 100% { filter: drop-shadow(0 0 10px rgba(245, 158, 11, 0.75)); }
    50%      { filter: drop-shadow(0 0 16px rgba(245, 158, 11, 1.0)); }
  }

  /* -------------------------------------------------------------------------
     Cluster bubbles — translucent amber rects per kind (mockup §10.3
     .node-bg.cluster.bubble). Sits behind links + nodes; ignores pointer
     events so it never intercepts node clicks.
     ------------------------------------------------------------------------- */
  :global(.node-bg.cluster.bubble) {
    fill: rgba(212, 137, 10, 0.05);
    stroke: var(--amber-faint);
    stroke-width: 1;
    stroke-dasharray: 3 4;
    pointer-events: none;
    opacity: 0.7;
    filter: drop-shadow(0 0 6px rgba(212, 137, 10, 0.12));
    transition: opacity 0.3s ease;
  }

  /* -------------------------------------------------------------------------
     Zoom-target group
     ------------------------------------------------------------------------- */
  :global(.zoom-target) {
    will-change: transform;
  }

  /* Source-node visual while a drag is in progress — dimmed to signal the
     ghost is the carried representation. */
  :global(.graph-node.dragging-source .node-circle) {
    opacity: 0.35;
  }
  :global(.graph-node.dragging-source .node-label) {
    opacity: 0.5;
  }

  /* -------------------------------------------------------------------------
     Drag ghost — fixed-positioned during vault-node drag (Phase 8.7b).
     Escapes .graph-pane overflow:hidden so the user can drop on terminal.
     pointer-events:none preserves underlying elementFromPoint hit-test.
     ------------------------------------------------------------------------- */
  .drag-ghost {
    position: fixed;
    transform: translate(-50%, -50%);
    pointer-events: none;
    z-index: 5000;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px 4px 6px;
    background: rgba(15, 12, 6, 0.92);
    border: 1px solid var(--amber-bright);
    border-radius: 12px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    color: var(--amber-bright);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5),
                0 0 12px rgba(245, 158, 11, 0.4);
    white-space: nowrap;
  }
  .drag-ghost-dot {
    display: block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--amber-bright);
    box-shadow: 0 0 6px rgba(245, 158, 11, 0.8);
  }
  .drag-ghost-label {
    color: var(--amber-warm, #d8d4c8);
    letter-spacing: 0.08em;
  }
</style>
