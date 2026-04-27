<script lang="ts">
  // IndexGraph.svelte — Phase 8.3
  //
  // D3 force-directed graph of Abyssal Index vault nodes.
  // Renders a static fixture of 10 real vault nodes + representative edges.
  //
  // DOM ownership contract:
  //   Svelte owns:  <svg> container + <defs> + outer <g class="zoom-target">
  //   D3 owns:      everything appended *inside* .zoom-target
  //                 (<g class="links"> and <g class="nodes">)
  // There are NO Svelte {#each} blocks targeting children D3 also renders.
  //
  // Lifecycle (per r004 canonical mount pattern + pr003 d3-svelte-effect-lifecycle):
  //   $effect guard → container present → build sim → bind zoom → bind drag
  //   → cleanup: simulation.stop() + simulation.on('tick', null) [BOTH required]
  //
  // TODO(8.4): mount in cockpit Index slot per mockup #3 integrated view.
  // TODO(8.5): replace static fixture with vault-walker live data via Category::Index subscription.

  import { forceSimulation, forceLink, forceManyBody, forceCenter } from 'd3-force';
  import type { Simulation, SimulationNodeDatum, SimulationLinkDatum } from 'd3-force';
  import { select } from 'd3-selection';
  import { zoom, zoomIdentity } from 'd3-zoom';
  import type { ZoomBehavior, ZoomTransform } from 'd3-zoom';
  import { drag } from 'd3-drag';
  import type { DragBehavior, D3DragEvent, SubjectPosition } from 'd3-drag';

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
    /** D3-managed position + velocity (set on SimulationNodeDatum) */
    x?: number;
    y?: number;
    vx?: number;
    vy?: number;
    /** null = free; number = pinned. */
    fx?: number | null;
    fy?: number | null;
  }

  /**
   * A directed link between two vault nodes.
   * D3 resolves source/target from string IDs into VaultNode references after
   * forceLink processes the array; the union type reflects both states.
   */
  interface VaultLink extends SimulationLinkDatum<VaultNode> {
    source: string | VaultNode;
    target: string | VaultNode;
  }

  // ---------------------------------------------------------------------------
  // Static fixture — 10 real Abyssal Index vault nodes + representative edges
  //
  // Node IDs match actual vault filenames (without .md) in the Abyssal Index.
  // Edges reflect real cross-vault dependencies:
  //   p006 → r004   (Rift v2 uses Tauri+Svelte stack documented in r004)
  //   p003 → r004   (AIDE uses same Tauri+Svelte stack)
  //   pr001 ↔ pr003 (global rules ↔ gotchas are tightly coupled)
  //   pr003 → p006  (gotchas entry d3-svelte-effect-lifecycle filed for Rift)
  //   pr003 → p003  (gotchas entries reference AIDE project evidence)
  //   pr001 → p006  (global practices govern Rift build)
  //   pr001 → p003  (global practices govern AIDE build)
  //   p001 → pr001  (personal practices vault links global rules)
  //   p002 → pr001  (second personal vault also links global rules)
  //   p004 → r004   (fourth project also on Tauri stack)
  //   p005 → pr001  (fifth project governed by global practices)
  //   r004 → pr003  (r004 cross-references gotchas for lifecycle patterns)
  //   p006 → pr003  (Rift v2 phase plan references gotchas entries)
  //   p004 → pr001  (project governed by global rules)
  //   p001 → p006   (Abyssal Arts main project spawned Rift v2)
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
  // Canonical Svelte 5 $effect mount pattern (r004 GRAPH LIB section verbatim)
  //
  // Guard:     if (!container) return   — prevents D3 from running before DOM mount
  // Cleanup:   simulation.stop()        — halts the simulation loop
  //            simulation.on('tick', null) — detaches tick listener (leak guard)
  //            BOTH are required (r004 + pr003 d3-svelte-effect-lifecycle)
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (!container) return;

    // Deep-copy the fixture arrays so D3 can mutate x/y/vx/vy/fx/fy safely
    // without corrupting the module-level constants on re-mount.
    const nodes: VaultNode[] = STATIC_NODES.map((n) => ({ ...n }));
    const links: VaultLink[] = STATIC_LINKS.map((l) => ({ ...l }));

    // ------------------------------------------------------------------
    // Dimensions — read from the SVG element's bounding rect.
    // container.getBoundingClientRect() is safe here because $effect runs
    // after the DOM has painted (Svelte 5 guarantee).
    // ------------------------------------------------------------------
    const rect = container.getBoundingClientRect();
    const W = rect.width  || 640;
    const H = rect.height || 480;

    // ------------------------------------------------------------------
    // D3 selection of the inner group that D3 fully owns.
    // Svelte rendered the <g class="zoom-target"> scaffold; D3 appends
    // into it.  We never let Svelte {#each} render children here.
    // ------------------------------------------------------------------
    const svg = select<SVGSVGElement, unknown>(container);
    const g   = svg.select<SVGGElement>('g.zoom-target');

    // Remove any previously D3-appended children (e.g. on hot-module reload
    // where the component re-mounts without a full page refresh).
    g.selectAll('*').remove();

    // Link group — rendered first so nodes paint on top.
    const linkGroup = g.append('g').attr('class', 'links');
    // Node group.
    const nodeGroup = g.append('g').attr('class', 'nodes');

    // ------------------------------------------------------------------
    // Force simulation
    // alphaDecay: 0.0228 (D3 default — do NOT touch alphaTarget;
    //   pr003: d3-force-alpha-decay-vs-alpha-target-confusion)
    // alphaTarget left at 0 (D3 default) — simulation settles cleanly.
    // ------------------------------------------------------------------
    simulation = forceSimulation<VaultNode, VaultLink>(nodes)
      // alphaDecay is 0.0228 by default — explicitly noted; not changed.
      .force(
        'link',
        forceLink<VaultNode, VaultLink>(links)
          .id((d) => d.id)
          .distance(90),
      )
      .force('charge', forceManyBody<VaultNode>().strength(-300))
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
    // D3 data joins — nodes (each node is a <g> containing circle + text)
    // ------------------------------------------------------------------

    /** Map vault kind → CSS class name for color styling. */
    function kindClass(kind: VaultKind): string {
      return `node-${kind}`;
    }

    const nodeSel = nodeGroup
      .selectAll<SVGGElement, VaultNode>('g.graph-node')
      .data(nodes, (d) => d.id)
      .join('g')
      .attr('class', (d) => `graph-node ${kindClass(d.kind)}`)
      .attr('cursor', 'grab');

    // Circle per node — r=8, category-colored via CSS class.
    nodeSel
      .append('circle')
      .attr('r', 8)
      .attr('class', (d) => `node-circle ${kindClass(d.kind)}`);

    // Label below the node.
    nodeSel
      .append('text')
      .attr('class', 'node-label')
      .attr('dy', 20)
      .attr('text-anchor', 'middle')
      .text((d) => d.id);

    // Amber-glow filter reference on hover — applied via CSS class toggle
    // but we also wire mouseenter/mouseleave for the CSS class approach.
    nodeSel
      .on('mouseenter', function () {
        select(this).classed('hovered', true);
      })
      .on('mouseleave', function () {
        select(this).classed('hovered', false);
      });

    // ------------------------------------------------------------------
    // Tick handler — updates node + link positions each simulation step.
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
    // Drag behavior — pins fx/fy on drag-start; releases on drag-end so
    // the simulation re-settles after user releases the node.
    // Cursor changes indicate drag state.
    // ------------------------------------------------------------------
    const dragBehavior: DragBehavior<SVGGElement, VaultNode, VaultNode | SubjectPosition> = drag<SVGGElement, VaultNode>()
      .on('start', (event: D3DragEvent<SVGGElement, VaultNode, VaultNode>, d: VaultNode) => {
        // Re-heat simulation so the graph reacts visually to the drag.
        if (!event.active) simulation.alpha(0.3).restart();
        d.fx = d.x;
        d.fy = d.y;
        select(event.sourceEvent.currentTarget as SVGGElement).attr('cursor', 'grabbing');
      })
      .on('drag', (event: D3DragEvent<SVGGElement, VaultNode, VaultNode>, d: VaultNode) => {
        d.fx = event.x;
        d.fy = event.y;
      })
      .on('end', (event: D3DragEvent<SVGGElement, VaultNode, VaultNode>, d: VaultNode) => {
        // Release pin — let simulation settle the node naturally.
        if (!event.active) simulation.alphaTarget(0);
        d.fx = null;
        d.fy = null;
        select(event.sourceEvent.currentTarget as SVGGElement).attr('cursor', 'grab');
      });

    // Apply drag to node groups.
    // The cast is required because d3-drag types use a generic that doesn't
    // perfectly align with d3-selection's .call() overloads without assertion.
    nodeSel.call(dragBehavior as unknown as (sel: typeof nodeSel) => void);

    // ------------------------------------------------------------------
    // Pan + zoom — d3-zoom on the SVG container; transform applied to the
    // inner <g class="zoom-target"> group.  The {x, y, k} triple is the
    // serializable zoom state (Phase 8.7 will persist it; no-op here).
    // ------------------------------------------------------------------
    const zoomBehavior: ZoomBehavior<SVGSVGElement, unknown> = zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.2, 5])
      .on('zoom', (event: { transform: ZoomTransform }) => {
        // Serialize: event.transform.x, event.transform.y, event.transform.k
        // (retained as plain numbers — ready for Phase 8.7 persistence).
        g.attr('transform', event.transform.toString());
      });

    svg.call(zoomBehavior);

    // Set a sensible initial zoom identity so the graph starts centered.
    svg.call(zoomBehavior.transform, zoomIdentity);

    // ------------------------------------------------------------------
    // CLEANUP — BOTH steps required (r004 canonical; pr003 leak guard)
    //   simulation.stop()          — halts requestAnimationFrame loop
    //   simulation.on('tick', null) — detaches the tick listener closure
    //     (stop() alone leaves the listener registered; re-mount would
    //      double-fire the old tick handler until GC collects it)
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

  TODO(8.4): mount in cockpit Index slot per mockup #3 integrated view.
-->
<svg
  bind:this={container}
  class="index-graph"
  aria-label="Abyssal Index vault graph"
>
  <defs>
    <!--
      Amber-glow SVG filter — applied to nodes on hover via CSS class.
      Two-layer approach: Gaussian blur spreads the glow halo, drop-shadow
      composites it back onto the node with the brand flood-color (#f59e0b).
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
      Softer than the hover glow to establish depth without distraction.
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

<style>
  /* -------------------------------------------------------------------------
     IndexGraph layout wrapper
     Full-bleed inside whatever slot App.svelte / cockpit provides (8.4).
     ------------------------------------------------------------------------- */
  .index-graph {
    display: block;
    width: 100%;
    height: 100%;
    background: var(--bg-base);
    /* Prevent text-selection during drag operations */
    user-select: none;
    -webkit-user-select: none;
  }

  /* -------------------------------------------------------------------------
     Edge lines — faint amber stroke matching the tree edge vocabulary
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
    /* Ambient glow always-on */
    filter: url(#ambient-glow);
    transition: filter 0.15s ease, stroke-width 0.15s ease;
  }

  /* Category-driven stroke colors (§10.1 lane table mapping):
       p    → project vaults      → amber-warm    (project lane = amber)
       pr   → practice vaults     → amber-primary  (rules/gotchas = amber-primary)
       r    → research vaults     → term-cyan      (research = hook lane cyan)
       s    → skill vaults        → term-purple    (skill = agent lane purple)
       lore → lore vaults         → term-blue      (lore = claude lane blue)
       agt  → agent vaults        → term-purple    (agent lane)
       h    → hook vaults         → term-cyan      (hook lane)                   */
  :global(.node-circle.node-p)    { stroke: var(--amber-warm); }
  :global(.node-circle.node-pr)   { stroke: var(--amber-primary); }
  :global(.node-circle.node-r)    { stroke: var(--term-cyan); }
  :global(.node-circle.node-s)    { stroke: var(--term-purple); }
  :global(.node-circle.node-lore) { stroke: var(--term-blue); }
  :global(.node-circle.node-agt)  { stroke: var(--term-purple); }
  :global(.node-circle.node-h)    { stroke: var(--term-cyan); }

  /* Hover state — full amber-glow filter (matches node-state-active vocabulary) */
  :global(.graph-node.hovered .node-circle) {
    filter: url(#amber-glow);
    stroke: var(--amber-bright);
    stroke-width: 2;
  }

  /* -------------------------------------------------------------------------
     Node labels — below the circle, JetBrains Mono, amber-dim default
     Matches .tree-node-label voice from Tree.svelte / mockup §tree section.
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
     Zoom-target group — D3 sets the transform attribute directly at runtime;
     will-change hint reduces compositor overhead during pan/zoom.
     ------------------------------------------------------------------------- */
  :global(.zoom-target) {
    will-change: transform;
  }
</style>
