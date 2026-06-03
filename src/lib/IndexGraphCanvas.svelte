<script lang="ts">
  // IndexGraphCanvas.svelte — D3 force-directed render of the Abyssal Index
  // vault graph (Phase 8 §10.18 redemption: restores the node/edge graph that
  // D-019 had downgraded to a list). Consumes the SAME activeNodes/activeEdges
  // data IndexGraph already computes — this is purely a render layer.
  //
  // Stack: d3-force (sim) + d3-selection (DOM join) + d3-zoom (pan/zoom) +
  // d3-drag (pin). SVG render so the amber-glow + matte aesthetic (§10.3) comes
  // free via filters. Mount pattern + cleanup per decisions/§10.18.

  import { untrack } from 'svelte';
  import {
    forceSimulation,
    forceLink,
    forceManyBody,
    forceCenter,
    forceCollide,
    type Simulation,
    type SimulationNodeDatum,
    type SimulationLinkDatum,
  } from 'd3-force';
  import { select } from 'd3-selection';
  import { zoom as d3zoom, type ZoomBehavior } from 'd3-zoom';
  import { drag as d3drag } from 'd3-drag';

  // Only the fields the graph needs from IndexGraph's VaultNode / VaultLink.
  interface GNodeIn { id: string; kind: string; label: string }
  interface GLinkIn { source: string; target: string; parent?: boolean }

  interface SimNode extends SimulationNodeDatum {
    id: string;
    kind: string;
    label: string;
    deg: number;
  }
  type SimLink = SimulationLinkDatum<SimNode> & { parent?: boolean };

  interface Props {
    nodes: GNodeIn[];
    edges: GLinkIn[];
    selectedId: string | null;
    /** Reports the clicked node id; the parent owns toggle/deselect semantics. */
    onSelect: (id: string) => void;
  }

  let { nodes, edges, selectedId, onSelect }: Props = $props();

  let svgEl = $state<SVGSVGElement | undefined>(undefined);
  let gEl = $state<SVGGElement | undefined>(undefined);
  let wrapEl = $state<HTMLDivElement | undefined>(undefined);

  // Kind → fill, mirroring the list's kind colors (§10.1 lane palette).
  const KIND_FILL: Record<string, string> = {
    p: '#f59e0b', pr: '#D4890A', r: '#6FE0E0', s: '#6CB6FF',
    lore: '#C58FFF', agt: '#4FE855', h: '#9a7b2e',
  };
  const fill = (kind: string): string => KIND_FILL[kind] ?? '#d8d4c8';

  const BASE_R = 5;
  // Node size encodes connectedness — the graph's reason to exist over a list
  // (a hub vault reads big; an isolated one reads small). Capped so one
  // super-connected node can't dominate.
  const radius = (deg: number): number => BASE_R + Math.min(deg, 8) * 1.7;

  // d3 types link endpoints as string | number | SimNode (numeric indices are
  // allowed before forceLink resolves them); normalize all three to an id.
  const idOf = (x: string | number | SimNode): string =>
    typeof x === 'object' ? x.id : String(x);

  // Structural signature — rebuild the simulation only when the node id-set or
  // edge count changes, NOT on every vault.update (which would re-throw the
  // layout from scratch and look chaotic during the initial walk).
  const graphSig = $derived(
    nodes.map((n) => n.id).sort().join(',') + '||' + edges.length,
  );

  let sim: Simulation<SimNode, SimLink> | undefined;
  let zoomBehavior: ZoomBehavior<SVGSVGElement, unknown> | undefined;

  // Build / rebuild the graph when its structure changes.
  $effect(() => {
    void graphSig; // dependency: re-run on structural change only
    if (!svgEl || !gEl) return;

    // d3-force mutates node/link objects (x/y/vx/vy, string→ref). Snapshot the
    // inputs (untracked so this effect doesn't re-run on unrelated field churn)
    // and work on copies so the reactive VaultNode objects are never touched.
    const inNodes = untrack(() => nodes);
    const inEdges = untrack(() => edges);
    // Untracked: the parent passes an inline arrow (new identity each render);
    // tracking it would rebuild the whole graph on every unrelated parent change.
    // The captured arrow still reads live state through its reactive closure.
    const onSel = untrack(() => onSelect);

    const degree = new Map<string, number>();
    for (const e of inEdges) {
      degree.set(e.source, (degree.get(e.source) ?? 0) + 1);
      degree.set(e.target, (degree.get(e.target) ?? 0) + 1);
    }
    const simNodes: SimNode[] = inNodes.map((n) => ({
      id: n.id, kind: n.kind, label: n.label, deg: degree.get(n.id) ?? 0,
    }));
    const byId = new Map(simNodes.map((n) => [n.id, n]));
    const simLinks: SimLink[] = inEdges
      .filter((e) => byId.has(e.source) && byId.has(e.target))
      .map((e) => ({ source: e.source, target: e.target, parent: e.parent }));

    const width = wrapEl?.clientWidth || 600;
    const height = wrapEl?.clientHeight || 400;

    const g = select(gEl);
    const linkSel = g
      .select<SVGGElement>('g.links')
      .selectAll<SVGLineElement, SimLink>('line')
      .data(simLinks, (d) => `${idOf(d.source)}->${idOf(d.target)}`)
      .join('line')
      .attr('stroke', (d) => (d.parent ? 'rgba(245,158,11,0.30)' : 'rgba(216,212,200,0.16)'))
      .attr('stroke-width', (d) => (d.parent ? 1.4 : 1));

    const nodeSel = g
      .select<SVGGElement>('g.nodes')
      .selectAll<SVGGElement, SimNode>('g.node')
      .data(simNodes, (d) => d.id)
      .join((enter) => {
        const gn = enter.append('g').attr('class', 'node');
        gn.append('circle');
        gn.append('text');
        return gn;
      });

    nodeSel
      .select('circle')
      .attr('r', (d) => radius(d.deg))
      .attr('fill', (d) => fill(d.kind))
      .attr('filter', 'url(#node-glow)');
    nodeSel
      .select('text')
      .text((d) => d.id)
      .attr('x', (d) => radius(d.deg) + 3)
      .attr('y', 3)
      .attr('fill', 'var(--amber-faint)')
      .attr('font-size', '9px')
      .attr('font-family', 'var(--font-family)')
      .attr('pointer-events', 'none');

    nodeSel.attr('cursor', 'pointer').on('click', (_ev, d) => onSel(d.id));

    nodeSel.call(
      d3drag<SVGGElement, SimNode>()
        .on('start', (ev, d) => {
          if (!ev.active) sim?.alphaTarget(0.2).restart();
          d.fx = d.x;
          d.fy = d.y;
        })
        .on('drag', (ev, d) => {
          d.fx = ev.x;
          d.fy = ev.y;
        })
        .on('end', (ev) => {
          if (!ev.active) sim?.alphaTarget(0);
          // fx/fy left set → node stays pinned where dropped (per §10.18).
        }),
    );

    sim?.stop();
    sim = forceSimulation<SimNode>(simNodes)
      .force('link', forceLink<SimNode, SimLink>(simLinks).id((d) => d.id).distance(55).strength(0.5))
      .force('charge', forceManyBody().strength(-190))
      .force('center', forceCenter(width / 2, height / 2))
      .force('collide', forceCollide<SimNode>().radius((d) => radius(d.deg) + 4))
      .on('tick', () => {
        linkSel
          .attr('x1', (d) => (d.source as SimNode).x ?? 0)
          .attr('y1', (d) => (d.source as SimNode).y ?? 0)
          .attr('x2', (d) => (d.target as SimNode).x ?? 0)
          .attr('y2', (d) => (d.target as SimNode).y ?? 0);
        nodeSel.attr('transform', (d) => `translate(${d.x ?? 0},${d.y ?? 0})`);
      });

    return () => {
      sim?.stop();
      sim?.on('tick', null);
      sim = undefined;
    };
  });

  // Pan/zoom — attach once when the svg + transform group are mounted.
  $effect(() => {
    if (!svgEl || !gEl) return;
    const g = select(gEl);
    zoomBehavior = d3zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.2, 4])
      .on('zoom', (ev) => g.attr('transform', ev.transform.toString()));
    const svg = select(svgEl);
    svg.call(zoomBehavior);
    return () => {
      svg.on('.zoom', null);
    };
  });

  // Selection highlight — reactive to selectedId, independent of the rebuild.
  $effect(() => {
    const sel = selectedId;
    if (!gEl) return;
    select(gEl)
      .selectAll<SVGGElement, SimNode>('g.node')
      .select('circle')
      .attr('stroke', (d) => (d.id === sel ? 'var(--amber-bright)' : 'none'))
      .attr('stroke-width', (d) => (d.id === sel ? 2 : 0));
  });
</script>

<div class="graph-wrap" bind:this={wrapEl}>
  {#if nodes.length === 0}
    <div class="graph-empty">No vault nodes yet — waiting for the index walk.</div>
  {/if}
  <svg bind:this={svgEl} class="graph-svg" role="presentation">
    <defs>
      <filter id="node-glow" x="-60%" y="-60%" width="220%" height="220%">
        <feDropShadow dx="0" dy="0" stdDeviation="2.2" flood-color="#f59e0b" flood-opacity="0.55" />
      </filter>
    </defs>
    <g bind:this={gEl}>
      <g class="links"></g>
      <g class="nodes"></g>
    </g>
  </svg>
  <div class="graph-hint">scroll to zoom · drag a node to pin · click to select</div>
</div>

<style>
  .graph-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
    background:
      radial-gradient(circle at 50% 40%, rgba(245, 158, 11, 0.03), transparent 70%),
      var(--bg-base);
  }
  .graph-svg {
    width: 100%;
    height: 100%;
    display: block;
    cursor: grab;
  }
  .graph-svg:active {
    cursor: grabbing;
  }
  .graph-empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    pointer-events: none;
  }
  .graph-hint {
    position: absolute;
    bottom: var(--space-xs);
    left: 50%;
    transform: translateX(-50%);
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
    pointer-events: none;
    opacity: 0.7;
    white-space: nowrap;
  }
</style>
