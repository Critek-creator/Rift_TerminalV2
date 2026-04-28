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
  // Lifecycle:
  //   $effect guard → container present → build sim on current nodes/edges
  //   → bind zoom → bind drag → cleanup: simulation.stop() + .on('tick', null)
  //   The $effect re-runs whenever `nodes` or `edges` reactive state changes.

  import { onMount } from 'svelte';
  import { forceSimulation, forceLink, forceManyBody, forceCenter } from 'd3-force';
  import type { Simulation, SimulationNodeDatum, SimulationLinkDatum } from 'd3-force';
  import { select } from 'd3-selection';
  import { zoom, zoomIdentity } from 'd3-zoom';
  import type { ZoomBehavior, ZoomTransform } from 'd3-zoom';
  import { drag } from 'd3-drag';
  import type { DragBehavior, D3DragEvent, SubjectPosition } from 'd3-drag';
  import { subscribe } from './bus';

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
    const nodes: VaultNode[] = activeNodes.map((n) => ({ ...n }));
    const links: VaultLink[] = activeEdges.map((l) => ({ ...l }));

    // Nothing to render yet — still waiting for walk.complete or live data.
    if (nodes.length === 0) return;

    // ------------------------------------------------------------------
    // Dimensions
    // ------------------------------------------------------------------
    const rect = container.getBoundingClientRect();
    const W = rect.width  || 640;
    const H = rect.height || 480;

    // ------------------------------------------------------------------
    // D3 selection
    // ------------------------------------------------------------------
    const svg = select<SVGSVGElement, unknown>(container);
    const g   = svg.select<SVGGElement>('g.zoom-target');

    g.selectAll('*').remove();

    const linkGroup = g.append('g').attr('class', 'links');
    const nodeGroup = g.append('g').attr('class', 'nodes');

    // ------------------------------------------------------------------
    // Force simulation
    //
    // Parameters tuned for the live-data scale (~40 vault nodes from the
    // Abyssal Index) — the original 8.3 values (charge=-300, link=90) were
    // calibrated for the 10-node static fixture and produced an extreme
    // spread when applied to 4× the node count (Phase 8.5 visual regression
    // — graph nodes scattered well outside the viewport even at max zoom-out).
    //
    // Charge strength scales inversely with node count: stronger repulsion
    // on small graphs (preserves 8.3 layout when fixture fallback fires),
    // weaker on large graphs (keeps live-data graph viewport-fit).
    //   N ≤ 12  → -300  (matches static fixture aesthetic)
    //   N ≤ 25  → -150
    //   N ≥ 26  → -80   (live data scale)
    // distanceMax caps repulsion radius so isolated nodes don't fly to infinity.
    // ------------------------------------------------------------------
    const chargeStrength = nodes.length <= 12 ? -300 : nodes.length <= 25 ? -150 : -80;
    const linkDistance   = nodes.length <= 12 ? 90  : nodes.length <= 25 ? 60  : 45;

    simulation = forceSimulation<VaultNode, VaultLink>(nodes)
      .force(
        'link',
        forceLink<VaultNode, VaultLink>(links)
          .id((d) => d.id)
          .distance(linkDistance),
      )
      .force(
        'charge',
        forceManyBody<VaultNode>().strength(chargeStrength).distanceMax(220),
      )
      .force('center', forceCenter<VaultNode>(W / 2, H / 2))
      .on('tick', tick);

    // ------------------------------------------------------------------
    // D3 data joins — links
    // ------------------------------------------------------------------
    let linkSel = linkGroup
      .selectAll<SVGLineElement, VaultLink>('line')
      .data(links)
      .join('line')
      .attr('class', 'graph-edge');

    // ------------------------------------------------------------------
    // D3 data joins — nodes
    // ------------------------------------------------------------------

    function kindClass(kind: VaultKind): string {
      return `node-${kind}`;
    }

    const nodeSel = nodeGroup
      .selectAll<SVGGElement, VaultNode>('g.graph-node')
      .data(nodes, (d) => d.id)
      .join('g')
      .attr('class', (d) => `graph-node ${kindClass(d.kind)}`)
      .attr('cursor', 'grab');

    nodeSel
      .append('circle')
      .attr('r', 8)
      .attr('class', (d) => `node-circle ${kindClass(d.kind)}`);

    nodeSel
      .append('text')
      .attr('class', 'node-label')
      .attr('dy', 20)
      .attr('text-anchor', 'middle')
      .text((d) => d.id);

    nodeSel
      .on('mouseenter', function () {
        select(this).classed('hovered', true);
      })
      .on('mouseleave', function () {
        select(this).classed('hovered', false);
      });

    // ------------------------------------------------------------------
    // Tick handler
    // ------------------------------------------------------------------
    function tick(): void {
      linkSel
        .attr('x1', (d) => (d.source as VaultNode).x ?? 0)
        .attr('y1', (d) => (d.source as VaultNode).y ?? 0)
        .attr('x2', (d) => (d.target as VaultNode).x ?? 0)
        .attr('y2', (d) => (d.target as VaultNode).y ?? 0);

      nodeSel
        .attr('transform', (d) => `translate(${d.x ?? 0},${d.y ?? 0})`);
    }

    // ------------------------------------------------------------------
    // Drag behavior
    // ------------------------------------------------------------------
    const dragBehavior: DragBehavior<SVGGElement, VaultNode, VaultNode | SubjectPosition> = drag<SVGGElement, VaultNode>()
      .on('start', function (this: SVGGElement, event: D3DragEvent<SVGGElement, VaultNode, VaultNode>, d: VaultNode) {
        if (!event.active) simulation.alpha(0.3).restart();
        d.fx = d.x;
        d.fy = d.y;
        select(this).attr('cursor', 'grabbing');
      })
      .on('drag', (event: D3DragEvent<SVGGElement, VaultNode, VaultNode>, d: VaultNode) => {
        d.fx = event.x;
        d.fy = event.y;
      })
      .on('end', function (this: SVGGElement, event: D3DragEvent<SVGGElement, VaultNode, VaultNode>, d: VaultNode) {
        if (!event.active) simulation.alphaTarget(0);
        d.fx = null;
        d.fy = null;
        select(this).attr('cursor', 'grab');
      });

    nodeSel.call(dragBehavior as unknown as (sel: typeof nodeSel) => void);

    // ------------------------------------------------------------------
    // Pan + zoom
    // ------------------------------------------------------------------
    const zoomBehavior: ZoomBehavior<SVGSVGElement, unknown> = zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.2, 5])
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
      zoom-target: the group D3 transforms for pan/zoom.
      D3 appends <g class="links"> and <g class="nodes"> here at mount time.
      Do NOT add Svelte children inside this element.
    -->
    <g class="zoom-target"></g>
  </svg>
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

  /* -------------------------------------------------------------------------
     Zoom-target group
     ------------------------------------------------------------------------- */
  :global(.zoom-target) {
    will-change: transform;
  }
</style>
