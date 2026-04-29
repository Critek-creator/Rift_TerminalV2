<script lang="ts">
  // IndexGraph.svelte — Phase 8.5
  //
  // D3 force-directed graph of Abyssal Index vault nodes.
  // Phase 8.5: replaced static fixture with live data from Category::Index
  // subscription (vault.update + walk.complete envelopes).
  //
  // Data flow:
  //   vault_walker (Rust) →  Category::Index/vault.update  → nodes/edges state
  //   vault_walker (Rust) →  Category::Index/walk.complete → walkComplete flag
  //   D3 simulation reads the reactive arrays each time they change.
  //
  // Fallback:
  //   If walkComplete is true and nodes is still empty (abyssal-index absent),
  //   the static fixture is displayed — §10.7 capability-driven empty state,
  //   mirroring how Aegis tab handles "not detected".
  //
  // DOM ownership contract (unchanged from 8.3):
  //   Svelte owns:  <svg> + <defs> + outer <g class="zoom-target"> + empty-state overlay
  //   D3 owns:      everything appended *inside* .zoom-target
  //                 (<g class="links"> and <g class="nodes">)
  //   NO Svelte {#each} blocks inside zoom-target.
  //
  // Lifecycle (Phase 8.7b — d3-zoom removed 2026-04-29):
  //   $effect guard → container present → build sim on current nodes/edges
  //   → tick writes positions to reactive renderedNodes/renderedLinks state
  //   → Svelte template renders <line> + <g class="graph-node"> from that state
  //   → manual mousedown/move/up gesture (top of script) handles node drag
  //     (live fx/fy pin while dragging) + drag-into-terminal on terminal-host
  //     mouseup hit-test
  //   → cleanup: simulation.stop() + .on('tick', null)
  //   The $effect re-runs whenever `nodes` or `edges` reactive state changes.
  //
  // Why Svelte owns the DOM inside zoom-target (changed from 8.5 D3-imperative):
  //   WebView2 / Chromium does not initiate HTML5 drag on SVG <g> elements
  //   created imperatively via D3 `.append('g')` + `.setAttribute('draggable',
  //   'true')`, even when the attribute is verifiably present in the DOM.
  //   Tree.svelte's working pattern is template-rendered <g draggable="true">
  //   — Phase 8.7 mirrors that pattern. D3 owns the simulation only.

  import { onMount } from 'svelte';
  import { forceSimulation, forceLink, forceManyBody, forceX, forceY, forceCollide } from 'd3-force';
  import type { Simulation, SimulationNodeDatum, SimulationLinkDatum } from 'd3-force';
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
  interface VaultNode extends SimulationNodeDatum {
    id: string;
    kind: VaultKind;
    label: string;
    x?: number;
    y?: number;
    vx?: number;
    vy?: number;
    fx?: number | null;
    fy?: number | null;
    /** Absolute vault file path — populated from VaultUpdatePayload; used as HTML5 drag payload (Phase 8.7). */
    path?: string;
  }

  /**
   * A directed link between two vault nodes.
   * D3 resolves source/target from string IDs into VaultNode references after
   * forceLink processes the array.
   */
  interface VaultLink extends SimulationLinkDatum<VaultNode> {
    source: string | VaultNode;
    target: string | VaultNode;
  }

  // ---------------------------------------------------------------------------
  // vault.update envelope payload shape (mirrors VaultUpdatePayload in Rust)
  // ---------------------------------------------------------------------------

  interface VaultUpdatePayload {
    vault_id: string;
    path: string;
    change_kind: 'created' | 'modified' | 'deleted';
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
    // Build edges from the cross_refs stored in each node's `crossRefs` extra
    // field. We attach crossRefs to nodes when processing vault.update envelopes.
    const edges: VaultLink[] = [];
    for (const [id, node] of liveNodeMap) {
      const refs = (node as VaultNode & { crossRefs?: string[] }).crossRefs ?? [];
      for (const ref of refs) {
        if (liveNodeMap.has(ref)) {
          edges.push({ source: id, target: ref });
        }
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

    void (async () => {
      try {
        const u = await subscribe({ category: 'index' }, (env) => {
          if (env.kind === 'vault.update') {
            const p = env.payload as VaultUpdatePayload;
            if (p.change_kind === 'deleted') {
              // Remove the node.
              liveNodeMap = new Map(
                [...liveNodeMap].filter(([k]) => k !== p.vault_id),
              );
            } else {
              // Create or update the node. Preserve existing D3 position if
              // the node already exists (avoids layout jump on modify).
              const existing = liveNodeMap.get(p.vault_id);
              // Extract cross_refs from payload if present (8.5 walker includes them).
              const raw = env.payload as VaultUpdatePayload & {
                cross_refs?: string[];
                name?: string;
              };
              const newNode: VaultNode & { crossRefs?: string[] } = {
                id: p.vault_id,
                kind: inferKind(p.vault_id),
                label: raw.name ? `${p.vault_id} — ${raw.name}` : p.vault_id,
                crossRefs: raw.cross_refs ?? [],
                path: p.path,
                // Preserve existing D3 x/y so the node doesn't snap to origin.
                x: existing?.x,
                y: existing?.y,
                vx: existing?.vx,
                vy: existing?.vy,
                fx: existing?.fx ?? null,
                fy: existing?.fy ?? null,
              };
              const next = new Map(liveNodeMap);
              next.set(p.vault_id, newNode);
              liveNodeMap = next;
            }
          } else if (env.kind === 'walk.complete') {
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

  /**
   * Active simulation reference — stored so the cleanup fn in $effect can
   * call both .stop() and .on('tick', null) on the same instance.
   * (pr003: d3-svelte-effect-lifecycle — stop() alone leaks the tick closure)
   */
  let simulation: Simulation<VaultNode, VaultLink>;

  /**
   * Live simulation-node array. Hoisted to module scope so the manual-gesture
   * drag handlers (onDocMouseMove / onDocMouseUp) can mutate fx/fy on the live
   * VaultNode the simulation is iterating — that's the standard d3-force drag
   * pattern (pin while dragging, release on drop).
   *
   * Reassigned by the $effect on every nodes/edges change. Drag handlers must
   * tolerate the array being replaced mid-gesture (rare; only on data update).
   */
  let simNodes: VaultNode[] = [];

  // ---------------------------------------------------------------------------
  // Per-tick render state (Phase 8.7)
  //
  // D3 mutates simNodes/simLinks in place; the tick handler reads positions
  // and writes them to these arrays. Svelte renders <line> + <g> from these
  // arrays via {#each}. $state.raw avoids deep-proxy overhead at 60 Hz.
  // ---------------------------------------------------------------------------

  interface RenderedNode {
    id: string;
    kind: VaultKind;
    label: string;
    path?: string;
    isIndex: boolean;
    draggable: boolean;
    cursor: 'default' | 'grab';
    x: number;
    y: number;
  }

  interface RenderedLink {
    key: string;
    cls: string;
    x1: number;
    y1: number;
    x2: number;
    y2: number;
  }

  let renderedNodes = $state.raw<RenderedNode[]>([]);
  let renderedLinks = $state.raw<RenderedLink[]>([]);

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
  // D3 simulation $effect — re-runs when activeNodes or activeEdges change
  //
  // Guard:     if (!container) return   — prevents D3 from running before DOM mount
  // Guard:     if (activeNodes.length === 0) return — no data yet (loading)
  // Cleanup:   simulation.stop()        — halts the simulation loop
  //            simulation.on('tick', null) — detaches tick listener (leak guard)
  //            BOTH are required (r004 + pr003 d3-svelte-effect-lifecycle)
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (!container) return;

    // Snapshot the derived arrays (D3 mutates them in-place via x/y/vx/vy).
    // Vault nodes only — the synthetic INDEX root is added below.
    const vaultNodes: VaultNode[] = activeNodes.map((n) => ({ ...n }));
    const vaultLinks: VaultLink[] = activeEdges.map((l) => ({ ...l }));

    // Nothing to render yet — still waiting for walk.complete or live data.
    if (vaultNodes.length === 0) return;

    // ------------------------------------------------------------------
    // Dimensions
    // ------------------------------------------------------------------
    const rect = container.getBoundingClientRect();
    const W = rect.width  || 640;
    const H = rect.height || 480;

    // ------------------------------------------------------------------
    // Radial layout (2026-04-28; rendering pivoted to Svelte template 8.7)
    //
    // Synthetic INDEX root pinned at viewport center; every vault is linked
    // to INDEX with a long spoke (RADIUS) plus its real cross-ref edges.
    // forceX/forceY pull each vault toward its kind's angular sector, so
    // nodes group by category (projects at 12 o'clock, research at 3, etc.).
    //
    // 8.7 change: Svelte owns DOM inside zoom-target ({#each renderedNodes,
    // renderedLinks}). The tick handler writes positions to those reactive
    // arrays; D3 is the simulation engine only. Required because WebView2 does
    // not initiate HTML5 drag on D3-imperative SVG <g> even with draggable="true".
    // ------------------------------------------------------------------
    const RADIUS = Math.min(W, H) * 0.48;
    const INDEX_ID = '__INDEX__';

    /** Angular sector per vault kind (radians; 0 = 3 o'clock, π/2 = 6 o'clock). */
    const KIND_ANGLE: Record<VaultKind, number> = {
      p:    -Math.PI / 2,           // 12 o'clock  — projects
      pr:   -Math.PI / 4,           // 1:30        — practices
      r:     0,                      // 3 o'clock   — research
      s:     Math.PI / 4,            // 4:30        — skills
      lore:  Math.PI / 2,            // 6 o'clock   — lore
      agt:   3 * Math.PI / 4,        // 7:30        — agents
      h:     Math.PI,                // 9 o'clock   — history
    };

    // Synthetic INDEX node, pinned at viewport center.
    const indexNode: VaultNode = {
      id: INDEX_ID,
      kind: 'p',                      // sentinel — special-cased in render
      label: 'INDEX',
      fx: W / 2,
      fy: H / 2,
    };

    // Spoke links from INDEX → every vault.
    const indexLinks: VaultLink[] = vaultNodes.map((n) => ({ source: INDEX_ID, target: n.id }));

    simNodes = [indexNode, ...vaultNodes];
    const simLinks: VaultLink[] = [...indexLinks, ...vaultLinks];

    // Charge scales with node count — weaker on large graphs to keep the
    // radial constraint dominant.
    const chargeStrength = vaultNodes.length <= 12 ? -240 : vaultNodes.length <= 25 ? -140 : -70;

    simulation = forceSimulation<VaultNode, VaultLink>(simNodes)
      .force(
        'link',
        forceLink<VaultNode, VaultLink>(simLinks)
          .id((d) => d.id)
          .distance((d) => {
            // INDEX-spoke links are RADIUS long; cross-vault links are short.
            const srcId = typeof d.source === 'string' ? d.source : (d.source as VaultNode).id;
            return srcId === INDEX_ID ? RADIUS : 38;
          })
          .strength((d) => {
            // INDEX-spoke links are strong (anchors the radial layout);
            // cross-vault links are gentle (don't fight the radial sector).
            const srcId = typeof d.source === 'string' ? d.source : (d.source as VaultNode).id;
            return srcId === INDEX_ID ? 0.9 : 0.2;
          }),
      )
      .force(
        'charge',
        forceManyBody<VaultNode>().strength(chargeStrength).distanceMax(180),
      )
      // Collision detection prevents node circles from overlapping. Radius 22 =
      // node circle r=8 + 14px breathing room (raised 2026-04-29 from r=14 for
      // tighter no-overlap + larger label-clearance per user feedback).
      .force('collide', forceCollide<VaultNode>(22))
      .force(
        'x',
        forceX<VaultNode>().x((d) => {
          if (d.id === INDEX_ID) return W / 2;
          const angle = KIND_ANGLE[d.kind] ?? 0;
          return W / 2 + Math.cos(angle) * RADIUS;
        }).strength(0.18),
      )
      .force(
        'y',
        forceY<VaultNode>().y((d) => {
          if (d.id === INDEX_ID) return H / 2;
          const angle = KIND_ANGLE[d.kind] ?? 0;
          return H / 2 + Math.sin(angle) * RADIUS;
        }).strength(0.18),
      )
      .on('tick', tick);

    // ------------------------------------------------------------------
    // Tick handler — writes simulation positions to reactive render state
    // ------------------------------------------------------------------
    function tick(): void {
      renderedLinks = simLinks.map((l) => {
        const s = l.source as VaultNode;
        const t = l.target as VaultNode;
        const srcId = typeof l.source === 'string' ? l.source : s.id;
        const tgtId = typeof l.target === 'string' ? l.target : t.id;
        return {
          key: `${srcId}->${tgtId}`,
          cls: srcId === INDEX_ID ? 'graph-edge index-spoke' : 'graph-edge',
          x1: s.x ?? 0,
          y1: s.y ?? 0,
          x2: t.x ?? 0,
          y2: t.y ?? 0,
        };
      });

      renderedNodes = simNodes.map((n) => ({
        id: n.id,
        kind: n.kind,
        label: n.label,
        path: n.path,
        isIndex: n.id === INDEX_ID,
        draggable: n.id !== INDEX_ID,
        cursor: n.id === INDEX_ID ? 'default' : 'grab',
        x: n.x ?? 0,
        y: n.y ?? 0,
      }));
    }

    // ------------------------------------------------------------------
    // Pan + zoom (Phase 8.7c — restored 2026-04-29 with safer filter).
    //
    // The Phase 8.7 race: d3-zoom's filter allowed pan on INDEX clicks
    // (because INDEX has class index-root and the filter only excluded
    // non-INDEX vault nodes). Clicking INDEX → d3-zoom panned the entire
    // <g class="zoom-target"> group → "tree drags."
    //
    // Safer filter (8.7c): wheel events allowed anywhere (zoom); mousedown
    // events only allowed when the target has NO ancestor g.graph-node
    // (drag-pan only on empty SVG background). Vault-node drag and
    // INDEX-click are both excluded from the zoom behavior, so the manual
    // mousedown/move/up gesture (above) owns those gestures uncontested.
    // ------------------------------------------------------------------
    const svg = select<SVGSVGElement, unknown>(container);
    const g   = svg.select<SVGGElement>('g.zoom-target');

    const zoomBehavior: ZoomBehavior<SVGSVGElement, unknown> = zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.4, 3])
      .filter((event: Event) => {
        // Always allow wheel events — zoom anywhere on the canvas.
        if (event.type === 'wheel') return true;
        // For mousedown (pan), only allow when NOT on any node.
        // closest('g.graph-node') matches both INDEX and vault nodes,
        // so the manual gesture handler keeps full ownership of node clicks.
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

    // ------------------------------------------------------------------
    // CLEANUP — BOTH steps required (r004 canonical; pr003 leak guard)
    // ------------------------------------------------------------------
    return () => {
      simulation.stop();
      simulation.on('tick', null);
    };
  });
</script>

<!--
  IndexGraph — SVG scaffold only.
  D3 populates everything inside <g class="zoom-target"> imperatively.
  NO Svelte {#each} inside zoom-target — D3 owns that subtree.
-->
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
          <line
            class={link.cls}
            x1={link.x1}
            y1={link.y1}
            x2={link.x2}
            y2={link.y2}
          />
        {/each}
      </g>
      <g class="nodes">
        {#each renderedNodes as node (node.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <g
            class={(node.isIndex ? 'graph-node index-root' : `graph-node node-${node.kind}`) +
                   (hoveredId === node.id ? ' hovered' : '') +
                   (draggingId === node.id ? ' dragging-source' : '')}
            cursor={node.cursor}
            transform="translate({node.x},{node.y})"
            onmouseenter={() => (hoveredId = node.id)}
            onmouseleave={() => {
              if (hoveredId === node.id) hoveredId = null;
            }}
            onmousedown={(e) => onNodeMouseDown(e, node)}
          >
            <circle
              r={node.isIndex ? 16 : 8}
              class={node.isIndex ? 'node-circle index-root' : `node-circle node-${node.kind}`}
            />
            <text
              class={node.isIndex ? 'node-label index-root' : 'node-label'}
              dy={node.isIndex ? 32 : 20}
              text-anchor="middle"
            >
              {node.isIndex ? 'INDEX' : node.id}
            </text>
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

  /* -------------------------------------------------------------------------
     Node circles — base style; category color via kind-prefixed class
     ------------------------------------------------------------------------- */
  :global(.node-circle) {
    stroke-width: 1.5;
    fill: var(--bg-elevated);
    filter: url(#ambient-glow);
    transition: filter 0.15s ease, stroke-width 0.15s ease;
  }

  /* Category-driven stroke colors (§10.1 lane table) */
  :global(.node-circle.node-p)    { stroke: var(--amber-warm); }
  :global(.node-circle.node-pr)   { stroke: var(--amber-primary); }
  :global(.node-circle.node-r)    { stroke: var(--term-cyan); }
  :global(.node-circle.node-s)    { stroke: var(--term-purple); }
  :global(.node-circle.node-lore) { stroke: var(--term-blue); }
  :global(.node-circle.node-agt)  { stroke: var(--term-purple); }
  :global(.node-circle.node-h)    { stroke: var(--term-cyan); }

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

  /* Hover state */
  :global(.graph-node.hovered .node-circle) {
    filter: url(#amber-glow);
    stroke: var(--amber-bright);
    stroke-width: 2;
  }

  /* -------------------------------------------------------------------------
     Node labels
     ------------------------------------------------------------------------- */
  :global(.node-label) {
    fill: var(--amber-dim);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 500;
    pointer-events: none;
    user-select: none;
  }

  :global(.graph-node.hovered .node-label) {
    fill: var(--amber-bright);
    font-weight: 700;
  }

  /* INDEX root label — bigger, bolder, always full amber. */
  :global(.node-label.index-root) {
    fill: var(--amber-bright);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.15em;
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
    z-index: 9999;
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
