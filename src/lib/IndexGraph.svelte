<script lang="ts">
  // IndexGraph.svelte — Vault Browser (list-based replacement for radial graph)
  //
  // Structured grouped list of Abyssal Index vaults. Categories collapse/expand,
  // vaults show state dots + connection badges, search filters across all.
  //
  // Data flow (unchanged from graph era):
  //   vault_walker (Rust) → Category::Index/vault.update → liveNodeMap
  //   vault_walker (Rust) → Category::Index/walk.complete → walkComplete flag
  //
  // Drag-to-terminal preserved: mousedown on vault row → ghost → drop on terminal.

  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { subscribe } from './bus';
  import { RIFT_VAULT_DROP_EVENT, type RiftVaultDropDetail } from './dragMime';
  import { crossRefHighlight } from './crossRefHighlight.svelte';
  import { enrichmentStore } from './enrichmentStore.svelte';
  import IndexContentBrowser from './IndexContentBrowser.svelte';

  type ViewMode = 'graph' | 'vaults' | 'content';
  let viewMode = $state<ViewMode>('vaults');

  type VaultKind = 'p' | 'pr' | 'r' | 's' | 'lore' | 'agt' | 'h';
  type NodeState = 'active' | 'recent' | 'ambient' | 'background';

  const RECENT_WINDOW_MS = 60 * 60 * 1000;

  const KIND_GLYPH: Record<VaultKind, string> = {
    p: '◐', pr: '§', r: '✦', s: '⚙', lore: '✧', agt: '⚝', h: '⏱',
  };

  const KIND_LABEL: Record<VaultKind, string> = {
    p: 'PROJECTS', pr: 'PRACTICES', r: 'RESEARCH',
    s: 'SKILLS', lore: 'LORE', agt: 'AGENTS', h: 'HISTORY',
  };

  const CATEGORY_ORDER: VaultKind[] = ['p', 'r', 'pr', 's', 'lore', 'agt', 'h'];

  interface VaultNode {
    id: string;
    kind: VaultKind;
    label: string;
    shortLabel?: string;
    displayName?: string;
    updatedMs?: number;
    path?: string;
    crossRefs?: string[];
    parentId?: string | null;
  }

  interface VaultLink {
    source: string;
    target: string;
    parent?: boolean;
  }

  interface VaultUpdatePayload {
    vault_id: string;
    parent_vault_id?: string | null;
    path: string;
    change_kind: 'created' | 'modified' | 'deleted';
    name?: string;
    short_label?: string | null;
    updated_ms?: number | null;
    cross_refs?: string[];
  }

  // ---------------------------------------------------------------------------
  // Static fixture fallback
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

  // ---------------------------------------------------------------------------
  // Live data state
  // ---------------------------------------------------------------------------

  let liveNodeMap = $state<Map<string, VaultNode>>(new Map());
  let walkComplete = $state(false);

  const activeNodes = $derived.by<VaultNode[]>(() => {
    if (liveNodeMap.size > 0) {
      return Array.from(liveNodeMap.values()).filter(
        (n) => !n.id.includes('archive') && !(n.path && n.path.includes('/archive/'))
      );
    }
    if (walkComplete) return STATIC_NODES.map((n) => ({ ...n }));
    return [];
  });

  const KNOWN_EDGES: [string, string][] = [
    ['p001', 'r001'], ['p002', 'r001'], ['p003', 'r004'],
    ['p006', 'r006'], ['p004', 'r003'], ['p009', 'r001'],
    ['pr001', 'pr003'], ['pr001', 'pr004'], ['pr002', 'pr001'],
    ['p001', 'pr001'], ['p002', 'pr001'], ['p003', 'pr001'],
    ['p004', 'pr001'], ['p005', 'pr001'], ['p006', 'pr001'],
    ['p007', 'pr001'], ['p008', 'pr001'], ['p009', 'pr001'],
    ['p010', 'pr001'],
    ['s001', 'pr001'], ['s001', 'pr003'],
    ['s011', 'p006'],
    ['r004', 'p003'], ['r004', 'p006'],
  ];

  const activeEdges = $derived.by<VaultLink[]>(() => {
    const nodeIds = new Set(activeNodes.map(n => n.id));
    const edges: VaultLink[] = [];
    const seen = new Set<string>();

    if (liveNodeMap.size > 0) {
      for (const [id, node] of liveNodeMap) {
        for (const ref of node.crossRefs ?? []) {
          if (liveNodeMap.has(ref)) {
            const key = [id, ref].sort().join('-');
            if (!seen.has(key)) { seen.add(key); edges.push({ source: id, target: ref }); }
          }
        }
        if (node.parentId && liveNodeMap.has(node.parentId)) {
          edges.push({ source: id, target: node.parentId, parent: true });
        }
      }
    }

    for (const [src, tgt] of KNOWN_EDGES) {
      if (nodeIds.has(src) && nodeIds.has(tgt)) {
        const key = [src, tgt].sort().join('-');
        if (!seen.has(key)) { seen.add(key); edges.push({ source: src, target: tgt }); }
      }
    }

    return edges;
  });

  // ---------------------------------------------------------------------------
  // UI state
  // ---------------------------------------------------------------------------

  let searchQuery = $state('');
  let collapsedKinds = $state<Set<VaultKind>>(new Set());
  let selectedId = $state<string | null>(null);
  let hoveredId = $state<string | null>(null);
  let searchInput = $state<HTMLInputElement | undefined>(undefined);
  let activeKindFilter = $state<VaultKind | null>(null);

  function toggleKind(kind: VaultKind): void {
    const next = new Set(collapsedKinds);
    if (next.has(kind)) next.delete(kind); else next.add(kind);
    collapsedKinds = next;
  }

  function toggleKindFilter(kind: VaultKind): void {
    activeKindFilter = activeKindFilter === kind ? null : kind;
  }

  // ---------------------------------------------------------------------------
  // Derived: categorized + filtered
  // ---------------------------------------------------------------------------

  function inferKind(vaultId: string): VaultKind {
    if (vaultId.startsWith('pr'))   return 'pr';
    if (vaultId.startsWith('lore')) return 'lore';
    if (vaultId.startsWith('agt'))  return 'agt';
    if (vaultId.startsWith('p'))    return 'p';
    if (vaultId.startsWith('r'))    return 'r';
    if (vaultId.startsWith('s'))    return 's';
    if (vaultId.startsWith('h'))    return 'h';
    return 'p';
  }

  function nodeState(node: VaultNode): NodeState {
    if (!node.updatedMs) return 'background';
    const age = Date.now() - node.updatedMs;
    if (age < 5 * 60 * 1000) return 'active';
    if (age < RECENT_WINDOW_MS) return 'recent';
    if (age < 24 * 60 * 60 * 1000) return 'ambient';
    return 'background';
  }

  function connectionsFor(id: string): string[] {
    const refs = new Set<string>();
    for (const e of activeEdges) {
      if (e.source === id) refs.add(e.target);
      if (e.target === id) refs.add(e.source);
    }
    return Array.from(refs);
  }

  interface CategoryGroup {
    kind: VaultKind;
    label: string;
    glyph: string;
    vaults: VaultNode[];
    collapsed: boolean;
  }

  const RECENTS_LIMIT = 5;
  const RECENTS_WINDOW_MS = 24 * 60 * 60 * 1000;

  const recentVaults = $derived.by<VaultNode[]>(() => {
    return activeNodes
      .filter((n) => n.updatedMs && (Date.now() - n.updatedMs) < RECENTS_WINDOW_MS)
      .sort((a, b) => (b.updatedMs ?? 0) - (a.updatedMs ?? 0))
      .slice(0, RECENTS_LIMIT);
  });

  function matchesSearch(n: VaultNode, q: string): boolean {
    return n.id.toLowerCase().includes(q) ||
      n.label.toLowerCase().includes(q) ||
      (n.displayName ?? '').toLowerCase().includes(q) ||
      (n.shortLabel ?? '').toLowerCase().includes(q);
  }

  const categories = $derived.by<CategoryGroup[]>(() => {
    const q = searchQuery.toLowerCase().trim();
    const kindsToShow = activeKindFilter ? [activeKindFilter] : CATEGORY_ORDER;
    return kindsToShow
      .map((kind) => {
        let vaults = activeNodes.filter((n) => n.kind === kind);
        if (q) {
          vaults = vaults.filter((n) => matchesSearch(n, q));
        }
        return {
          kind,
          label: KIND_LABEL[kind],
          glyph: KIND_GLYPH[kind],
          vaults,
          collapsed: collapsedKinds.has(kind),
        };
      })
      .filter((g) => g.vaults.length > 0);
  });

  const kindCounts = $derived.by<Record<VaultKind, number>>(() => {
    const counts: Record<string, number> = {};
    for (const n of activeNodes) {
      counts[n.kind] = (counts[n.kind] ?? 0) + 1;
    }
    return counts as Record<VaultKind, number>;
  });

  const totalCount = $derived(activeNodes.length);

  const hoveredConnections = $derived.by<Set<string>>(() => {
    if (!hoveredId) return new Set();
    return new Set(connectionsFor(hoveredId));
  });

  /** Vault IDs highlighted by Tree enrichment-dot hover (cross-component). */
  const treeHighlightedVaultIds = $derived.by<Set<string>>(() => {
    const treePath = crossRefHighlight.hoveredTreePath;
    if (!treePath) return new Set();
    const entries = enrichmentStore.get(treePath);
    if (!entries) return new Set();
    return new Set(entries.map((e) => e.vault_id));
  });

  // ---------------------------------------------------------------------------
  // Graph simulation
  // ---------------------------------------------------------------------------

  interface SimNode extends VaultNode {
    x: number;
    y: number;
    vx: number;
    vy: number;
    fx?: number;
    fy?: number;
  }

  interface SimEdge {
    source: SimNode;
    target: SimNode;
    parent?: boolean;
  }

  const CATEGORY_CENTERS: Record<VaultKind, { x: number; y: number }> = {
    p:    { x: 0.3, y: 0.3 },
    pr:   { x: 0.5, y: 0.2 },
    r:    { x: 0.7, y: 0.3 },
    s:    { x: 0.3, y: 0.7 },
    lore: { x: 0.7, y: 0.7 },
    agt:  { x: 0.5, y: 0.8 },
    h:    { x: 0.5, y: 0.5 },
  };

  const SIM_REPULSION = 1500;
  const SIM_SPRING_K = 0.03;
  const SIM_SPRING_REST = 60;
  const SIM_CATEGORY_GRAVITY = 0.015;
  const SIM_CENTER_GRAVITY = 0.005;
  const SIM_DAMPING = 0.85;
  const SIM_MAX_V = 10;
  const SIM_ALPHA_DECAY = 0.005;
  const SIM_ALPHA_MIN = 0.01;

  // --- Physics layer (plain JS, NOT reactive) ---
  let _physNodes: SimNode[] = [];
  let _physEdges: SimEdge[] = [];
  let _alpha = 1.0;
  let _rafId: number | null = null;
  let _settled = false;

  // Canvas refs
  let _canvasEl: HTMLCanvasElement | undefined = $state(undefined);
  let _ctx: CanvasRenderingContext2D | null = null;
  let _resizeObs: ResizeObserver | null = null;

  // Pan/zoom state (written on user interaction only, not per-frame)
  let viewBoxX = $state(0);
  let viewBoxY = $state(0);
  let viewBoxW = $state(600);
  let viewBoxH = $state(400);
  let isPanning = false;
  let panStartX = 0;
  let panStartY = 0;
  let panStartVBX = 0;
  let panStartVBY = 0;

  // Graph node drag state
  let graphDragNode: SimNode | null = null;
  let graphDragStartClientX = 0;
  let graphDragStartClientY = 0;
  let graphDragActive = false;

  // Tooltip state (reactive — drives HTML overlay)
  let tooltipVisible = $state(false);
  let tooltipX = $state(0);
  let tooltipY = $state(0);
  let tooltipNode = $state<SimNode | null>(null);

  // Hovered-node connections cache (non-reactive, used by drawFrame)
  let _hoveredConns: Set<string> = new Set();

  // --- Canvas color helpers ---
  const KIND_COLORS: Record<VaultKind, string> = {
    p: '#FFC840',     // amber-bright (projects)
    pr: '#FFA826',    // amber-warm (practices)
    r: '#6FE0E0',     // term-cyan (research)
    s: '#6CB6FF',     // term-blue (skills)
    lore: '#C58FFF',  // term-purple (lore)
    agt: '#4FE855',   // term-green (agents)
    h: '#C49A50',     // amber-faint (history)
  };

  function kindColor(kind: VaultKind): string {
    return KIND_COLORS[kind] ?? '#FFA826';
  }

  function colorWithAlpha(hex: string, alpha: number): string {
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }

  function rebuildPhysNodes(nodes: VaultNode[], edges: VaultLink[]): void {
    const existing = new Map<string, SimNode>();
    for (const sn of _physNodes) existing.set(sn.id, sn);
    _physNodes = nodes.map((n) => {
      const prev = existing.get(n.id);
      if (prev) {
        return { ...prev, ...n, x: prev.x, y: prev.y, vx: prev.vx, vy: prev.vy, fx: prev.fx, fy: prev.fy };
      }
      const center = CATEGORY_CENTERS[n.kind] ?? { x: 0.5, y: 0.5 };
      return {
        ...n,
        x: center.x * viewBoxW + viewBoxX + (Math.random() - 0.5) * 80,
        y: center.y * viewBoxH + viewBoxY + (Math.random() - 0.5) * 80,
        vx: 0,
        vy: 0,
      };
    });
    // Build edges referencing the new physics nodes
    const nodeMap = new Map<string, SimNode>();
    for (const sn of _physNodes) nodeMap.set(sn.id, sn);
    _physEdges = [];
    for (const e of edges) {
      const src = nodeMap.get(e.source);
      const tgt = nodeMap.get(e.target);
      if (src && tgt) _physEdges.push({ source: src, target: tgt, parent: e.parent });
    }
  }

  // --- Physics tick (plain JS — no reactivity) ---
  function tickPhysics(): void {
    const nodes = _physNodes;
    const edges = _physEdges;
    const cx = viewBoxX + viewBoxW / 2;
    const cy = viewBoxY + viewBoxH / 2;
    const alpha = _alpha;

    // Repulsion (n^2)
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const a = nodes[i];
        const b = nodes[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) { dist = 1; dx = (Math.random() - 0.5) * 2; dy = (Math.random() - 0.5) * 2; }
        const force = alpha * SIM_REPULSION / (dist * dist);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        a.vx -= fx;
        a.vy -= fy;
        b.vx += fx;
        b.vy += fy;
      }
    }

    // Spring attraction along edges
    for (const e of edges) {
      const dx = e.target.x - e.source.x;
      const dy = e.target.y - e.source.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const force = alpha * SIM_SPRING_K * (dist - SIM_SPRING_REST);
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      e.source.vx += fx;
      e.source.vy += fy;
      e.target.vx -= fx;
      e.target.vy -= fy;
    }

    // Category gravity + center gravity
    for (const node of nodes) {
      const catCenter = CATEGORY_CENTERS[node.kind] ?? { x: 0.5, y: 0.5 };
      const catX = viewBoxX + catCenter.x * viewBoxW;
      const catY = viewBoxY + catCenter.y * viewBoxH;
      node.vx += (catX - node.x) * SIM_CATEGORY_GRAVITY * alpha;
      node.vy += (catY - node.y) * SIM_CATEGORY_GRAVITY * alpha;
      node.vx += (cx - node.x) * SIM_CENTER_GRAVITY * alpha;
      node.vy += (cy - node.y) * SIM_CENTER_GRAVITY * alpha;
    }

    // Damping + velocity cap + position integration
    for (const node of nodes) {
      node.vx *= SIM_DAMPING;
      node.vy *= SIM_DAMPING;
      const speed = Math.sqrt(node.vx * node.vx + node.vy * node.vy);
      if (speed > SIM_MAX_V) {
        node.vx = (node.vx / speed) * SIM_MAX_V;
        node.vy = (node.vy / speed) * SIM_MAX_V;
      }
      if (node.fx !== undefined) { node.x = node.fx; node.vx = 0; }
      else { node.x += node.vx; }
      if (node.fy !== undefined) { node.y = node.fy; node.vy = 0; }
      else { node.y += node.vy; }
    }
  }

  // --- Canvas draw ---
  function drawFrame(): void {
    if (!_ctx || !_canvasEl) return;
    const dpr = devicePixelRatio || 1;
    const w = _canvasEl.width / dpr;
    const h = _canvasEl.height / dpr;
    _ctx.clearRect(0, 0, w, h);

    _ctx.save();
    // Map viewBox coordinates to canvas pixel space
    const sx = w / viewBoxW;
    const sy = h / viewBoxH;
    _ctx.translate(-viewBoxX * sx, -viewBoxY * sy);
    _ctx.scale(sx, sy);

    // Draw edges
    for (const edge of _physEdges) {
      const dimmed = isNodeDimmedPhys(edge.source) || isNodeDimmedPhys(edge.target);
      const isHovEdge = hoveredId !== null && (edge.source.id === hoveredId || edge.target.id === hoveredId);
      _ctx.strokeStyle = dimmed ? 'rgba(168, 120, 48, 0.08)'
        : isHovEdge ? 'rgba(255, 200, 64, 0.8)'
        : 'rgba(168, 120, 48, 0.4)';
      _ctx.lineWidth = edge.parent ? 2.5 : 1.5;
      if (edge.parent && !dimmed && !isHovEdge) {
        _ctx.setLineDash([4, 3]);
      } else {
        _ctx.setLineDash([]);
      }
      _ctx.beginPath();
      _ctx.moveTo(edge.source.x, edge.source.y);
      _ctx.lineTo(edge.target.x, edge.target.y);
      _ctx.stroke();
    }
    _ctx.setLineDash([]);

    // Draw nodes
    for (const node of _physNodes) {
      const dimmed = isNodeDimmedPhys(node);
      const isHovered = hoveredId === node.id;
      const isConnected = hoveredId !== null && _hoveredConns.has(node.id);
      const r = isHovered ? 18 : 14;
      const color = kindColor(node.kind);

      _ctx.globalAlpha = dimmed ? 0.15 : 1;

      // Circle fill
      _ctx.fillStyle = colorWithAlpha(color, 0.2);
      _ctx.beginPath();
      _ctx.arc(node.x, node.y, r, 0, Math.PI * 2);
      _ctx.fill();

      // Circle stroke
      _ctx.strokeStyle = color;
      _ctx.lineWidth = isHovered ? 2.5 : (isConnected ? 2 : 1.5);
      _ctx.stroke();

      // Glow for hovered/connected
      if (isHovered || isConnected) {
        _ctx.save();
        _ctx.shadowColor = color;
        _ctx.shadowBlur = isHovered ? 12 : 6;
        _ctx.beginPath();
        _ctx.arc(node.x, node.y, r, 0, Math.PI * 2);
        _ctx.stroke();
        _ctx.restore();
      }

      // Label (vault ID only)
      _ctx.fillStyle = dimmed ? 'rgba(168, 120, 48, 0.3)' : color;
      _ctx.font = '8px "JetBrains Mono", monospace';
      _ctx.textAlign = 'center';
      _ctx.textBaseline = 'middle';
      _ctx.fillText(node.id, node.x, node.y);

      _ctx.globalAlpha = 1;
    }

    _ctx.restore();
  }

  // --- Simulation loop ---
  function startPhysicsLoop(): void {
    stopPhysicsLoop();
    _alpha = 1.0;
    _settled = false;
    function loop(): void {
      tickPhysics();
      _alpha -= SIM_ALPHA_DECAY;
      drawFrame();
      if (_alpha >= SIM_ALPHA_MIN) {
        _rafId = requestAnimationFrame(loop);
      } else {
        _rafId = null;
        _settled = true;
        drawFrame(); // final settled frame
      }
    }
    _rafId = requestAnimationFrame(loop);
  }

  function stopPhysicsLoop(): void {
    if (_rafId !== null) { cancelAnimationFrame(_rafId); _rafId = null; }
  }

  function requestRedraw(): void {
    if (_settled && _rafId === null) {
      drawFrame();
    }
  }

  function reheat(): void {
    _alpha = 0.5;
    _settled = false;
    if (_rafId === null) startPhysicsLoop();
  }

  // --- Debounced $effect to rebuild physics on data change ---
  let _buildTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (viewMode !== 'graph') { stopPhysicsLoop(); return; }
    const nodes = activeNodes;  // read reactive dep
    const edges = activeEdges;  // read reactive dep
    if (nodes.length === 0) { stopPhysicsLoop(); return; }

    if (_buildTimer !== null) clearTimeout(_buildTimer);
    _buildTimer = setTimeout(() => {
      _buildTimer = null;
      rebuildPhysNodes(nodes, edges);
      startPhysicsLoop();
    }, 200);
  });

  // Canvas setup + resize observer
  $effect(() => {
    if (viewMode === 'graph' && _canvasEl) {
      _ctx = _canvasEl.getContext('2d');
      const container = _canvasEl.parentElement;
      if (container) {
        const dpr = devicePixelRatio || 1;
        const fitCanvas = () => {
          if (!_canvasEl || !_canvasEl.parentElement) return;
          const rect = _canvasEl.parentElement.getBoundingClientRect();
          _canvasEl.width = rect.width * dpr;
          _canvasEl.height = rect.height * dpr;
          _canvasEl.style.width = rect.width + 'px';
          _canvasEl.style.height = rect.height + 'px';
          if (_ctx) {
            _ctx.setTransform(1, 0, 0, 1, 0, 0);
            _ctx.scale(dpr, dpr);
          }
          requestRedraw();
        };
        fitCanvas();

        _resizeObs?.disconnect();
        _resizeObs = new ResizeObserver(() => fitCanvas());
        _resizeObs.observe(container);
      }
    }
    return () => {
      _resizeObs?.disconnect();
      _resizeObs = null;
    };
  });

  // Redraw when search/filter changes (settled sim, no RAF running)
  $effect(() => {
    void searchQuery;
    void activeKindFilter;
    requestRedraw();
  });

  // --- Canvas coordinate helper ---
  function clientToSvgCanvas(clientX: number, clientY: number): { x: number; y: number } {
    if (!_canvasEl) return { x: clientX, y: clientY };
    const rect = _canvasEl.getBoundingClientRect();
    return {
      x: viewBoxX + ((clientX - rect.left) / rect.width) * viewBoxW,
      y: viewBoxY + ((clientY - rect.top) / rect.height) * viewBoxH,
    };
  }

  // --- Hit-testing ---
  function findNodeAt(clientX: number, clientY: number): SimNode | null {
    if (!_canvasEl) return null;
    const rect = _canvasEl.getBoundingClientRect();
    const px = viewBoxX + ((clientX - rect.left) / rect.width) * viewBoxW;
    const py = viewBoxY + ((clientY - rect.top) / rect.height) * viewBoxH;
    // Reverse order so topmost-drawn node wins
    for (let i = _physNodes.length - 1; i >= 0; i--) {
      const n = _physNodes[i];
      const dx = n.x - px;
      const dy = n.y - py;
      if (dx * dx + dy * dy <= 18 * 18) return n;
    }
    return null;
  }

  // --- Canvas mouse handlers ---
  function onCanvasMouseDown(e: MouseEvent): void {
    if (e.button !== 0) return;
    const node = findNodeAt(e.clientX, e.clientY);
    if (node) {
      graphDragNode = node;
      graphDragStartClientX = e.clientX;
      graphDragStartClientY = e.clientY;
      graphDragActive = false;
      const pos = clientToSvgCanvas(e.clientX, e.clientY);
      node.fx = pos.x;
      node.fy = pos.y;
      reheat();
    } else {
      isPanning = true;
      panStartX = e.clientX;
      panStartY = e.clientY;
      panStartVBX = viewBoxX;
      panStartVBY = viewBoxY;
    }
    e.preventDefault();
  }

  function onCanvasMouseMove(e: MouseEvent): void {
    if (graphDragNode) {
      const pos = clientToSvgCanvas(e.clientX, e.clientY);
      graphDragNode.fx = pos.x;
      graphDragNode.fy = pos.y;
      if (!graphDragActive) {
        const dx = e.clientX - graphDragStartClientX;
        const dy = e.clientY - graphDragStartClientY;
        if (Math.abs(dx) + Math.abs(dy) >= DRAG_THRESHOLD_PX) graphDragActive = true;
      }
      // Check if dragged outside canvas for vault drop
      if (graphDragActive && _canvasEl) {
        const rect = _canvasEl.getBoundingClientRect();
        if (e.clientX < rect.left || e.clientX > rect.right ||
            e.clientY < rect.top || e.clientY > rect.bottom) {
          draggingId = graphDragNode.id;
          ghostLabel = graphDragNode.shortLabel || graphDragNode.displayName || graphDragNode.id;
          ghostKind = graphDragNode.kind;
          ghostX = e.clientX + 12;
          ghostY = e.clientY - 8;
          ghostVisible = true;
        } else {
          ghostVisible = false;
          draggingId = null;
        }
      }
      requestRedraw();
      return;
    }
    if (isPanning) {
      if (!_canvasEl) return;
      const rect = _canvasEl.getBoundingClientRect();
      const dx = ((e.clientX - panStartX) / rect.width) * viewBoxW;
      const dy = ((e.clientY - panStartY) / rect.height) * viewBoxH;
      viewBoxX = panStartVBX - dx;
      viewBoxY = panStartVBY - dy;
      requestRedraw();
      return;
    }
    // Hover detection
    const node = findNodeAt(e.clientX, e.clientY);
    const newId = node?.id ?? null;
    if (newId !== hoveredId) {
      hoveredId = newId;
      _hoveredConns = newId ? new Set(connectionsFor(newId)) : new Set();
      requestRedraw();
    }
    if (node) {
      tooltipNode = node;
      tooltipX = e.clientX + 14;
      tooltipY = e.clientY - 10;
      tooltipVisible = true;
    } else {
      tooltipVisible = false;
      tooltipNode = null;
    }
  }

  function onCanvasMouseUp(e: MouseEvent): void {
    if (graphDragNode) {
      if (graphDragActive && graphDragNode.path) {
        const target = document.elementFromPoint(e.clientX, e.clientY);
        const terminal = target?.closest('.terminal-host');
        if (terminal) {
          terminal.dispatchEvent(new CustomEvent<RiftVaultDropDetail>(
            RIFT_VAULT_DROP_EVENT,
            { detail: { path: graphDragNode.path }, bubbles: true },
          ));
        }
      }
      graphDragNode.fx = undefined;
      graphDragNode.fy = undefined;
      graphDragNode = null;
      graphDragActive = false;
      ghostVisible = false;
      draggingId = null;
      requestRedraw();
      return;
    }
    isPanning = false;
  }

  function onCanvasWheel(e: WheelEvent): void {
    e.preventDefault();
    if (!_canvasEl) return;
    const scale = e.deltaY > 0 ? 1.1 : 0.9;
    const rect = _canvasEl.getBoundingClientRect();
    const mx = (e.clientX - rect.left) / rect.width;
    const my = (e.clientY - rect.top) / rect.height;
    const newW = viewBoxW * scale;
    const newH = viewBoxH * scale;
    viewBoxX += (viewBoxW - newW) * mx;
    viewBoxY += (viewBoxH - newH) * my;
    viewBoxW = newW;
    viewBoxH = newH;
    requestRedraw();
  }

  function onCanvasMouseLeave(): void {
    hoveredId = null;
    tooltipVisible = false;
    tooltipNode = null;
    _hoveredConns = new Set();
    requestRedraw();
  }

  // --- Dimming helpers (read reactive search/filter values) ---
  function isNodeDimmedPhys(node: SimNode): boolean {
    const q = searchQuery.toLowerCase().trim();
    if (q && !matchesSearch(node, q)) return true;
    if (activeKindFilter && node.kind !== activeKindFilter) return true;
    return false;
  }

  // ---------------------------------------------------------------------------
  // Bus subscription (same debounce pattern as old graph)
  // ---------------------------------------------------------------------------

  onMount(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | undefined;

    function onKeyDown(e: KeyboardEvent): void {
      if (e.key === '/' && document.activeElement?.tagName !== 'INPUT') {
        e.preventDefault();
        searchInput?.focus();
      }
    }
    document.addEventListener('keydown', onKeyDown);

    const pendingUpdates = new Map<string, VaultNode>();
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
      for (const id of pendingDeletes) next.delete(id);
      for (const [id, node] of pendingUpdates) next.set(id, node);
      pendingDeletes.clear();
      pendingUpdates.clear();
      liveNodeMap = next;
    }

    function scheduleFlush(): void {
      if (flushTimer !== null) window.clearTimeout(flushTimer);
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
            if (p.change_kind === 'deleted') {
              pendingUpdates.delete(p.vault_id);
              pendingDeletes.add(p.vault_id);
            } else {
              const node: VaultNode = {
                id: p.vault_id,
                kind: inferKind(p.vault_id),
                label: p.name ? `${p.vault_id} — ${p.name}` : p.vault_id,
                displayName: p.name ?? undefined,
                shortLabel: p.short_label ?? undefined,
                updatedMs: p.updated_ms ?? undefined,
                crossRefs: p.cross_refs ?? [],
                parentId: p.parent_vault_id ?? undefined,
                path: p.path,
              };
              pendingDeletes.delete(p.vault_id);
              pendingUpdates.set(p.vault_id, node);
            }
            scheduleFlush();
          } else if (env.kind === 'walk.complete') {
            flushPendingVaults();
            walkComplete = true;
          }
        });
        if (cancelled) { void u().catch(() => {}); }
        else { unsub = u; }

        // Trigger a vault rescan after subscribing. Boot-walk events may have
        // been evicted from the 512-entry replay ring buffer by other bus
        // traffic (fs, status, hooks). The rescan re-publishes all vault.update
        // + walk.complete events so this subscription picks them up as live
        // events. Duplicates are harmless — liveNodeMap keys by vault_id.
        if (!cancelled) {
          invoke('vault_rescan').catch((err: unknown) => {
            console.warn('[IndexGraph] vault_rescan failed:', err);
          });
        }
      } catch (err) {
        console.warn('[IndexGraph] Category::Index subscribe failed:', err);
        walkComplete = true;
      }
    })();

    return () => {
      cancelled = true;
      document.removeEventListener('keydown', onKeyDown);
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer);
        flushTimer = null;
      }
      pendingUpdates.clear();
      pendingDeletes.clear();
      stopPhysicsLoop();
      _resizeObs?.disconnect();
      _resizeObs = null;
      if (_buildTimer !== null) { clearTimeout(_buildTimer); _buildTimer = null; }
      if (dragActive || dragNode) {
        document.removeEventListener('mousemove', onDocMouseMove);
        document.removeEventListener('mouseup', onDocMouseUp);
        dragActive = false;
        dragNode = null;
      }
      void (async () => { await unsub?.(); })();
    };
  });

  // ---------------------------------------------------------------------------
  // Drag-to-terminal (manual gesture — WebView2 SVG drag limitation)
  // ---------------------------------------------------------------------------

  let ghostVisible = $state(false);
  let ghostX = $state(0);
  let ghostY = $state(0);
  let ghostLabel = $state('');
  let ghostKind = $state<VaultKind>('p');
  let draggingId = $state<string | null>(null);

  const DRAG_THRESHOLD_PX = 5;
  let dragNode: VaultNode | null = null;
  let dragStartX = 0;
  let dragStartY = 0;
  let dragActive = false;

  function onRowMouseDown(e: MouseEvent, node: VaultNode): void {
    if (e.button !== 0) return;
    dragNode = node;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    dragActive = false;
    document.addEventListener('mousemove', onDocMouseMove);
    document.addEventListener('mouseup', onDocMouseUp);
  }

  function onDocMouseMove(e: MouseEvent): void {
    if (!dragNode) return;
    const dx = e.clientX - dragStartX;
    const dy = e.clientY - dragStartY;
    if (!dragActive && Math.abs(dx) + Math.abs(dy) < DRAG_THRESHOLD_PX) return;
    if (!dragActive) {
      dragActive = true;
      draggingId = dragNode.id;
      ghostLabel = dragNode.shortLabel || dragNode.displayName || dragNode.id;
      ghostKind = dragNode.kind;
    }
    ghostX = e.clientX + 12;
    ghostY = e.clientY - 8;
    ghostVisible = true;
  }

  function onDocMouseUp(e: MouseEvent): void {
    document.removeEventListener('mousemove', onDocMouseMove);
    document.removeEventListener('mouseup', onDocMouseUp);
    if (dragActive && dragNode?.path) {
      const target = document.elementFromPoint(e.clientX, e.clientY);
      const terminal = target?.closest('.terminal-host');
      if (terminal) {
        terminal.dispatchEvent(new CustomEvent<RiftVaultDropDetail>(
          RIFT_VAULT_DROP_EVENT,
          { detail: { path: dragNode.path }, bubbles: true },
        ));
      }
    }
    dragNode = null;
    dragActive = false;
    draggingId = null;
    ghostVisible = false;
  }

  function formatAge(ms?: number): string {
    if (!ms) return '';
    const age = Date.now() - ms;
    const sec = Math.floor(age / 1000);
    if (sec < 60) return 'now';
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}m`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h`;
    const d = Math.floor(hr / 24);
    return `${d}d`;
  }
</script>

<div class="index-browser">
  <!-- Mode toggle + header -->
  <div class="browser-header">
    <div class="mode-toggle">
      <button type="button"
        class="mode-btn"
        class:active={viewMode === 'graph'}
        onclick={() => { viewMode = 'graph'; }}
      >GRAPH</button>
      <button type="button"
        class="mode-btn"
        class:active={viewMode === 'vaults'}
        onclick={() => { viewMode = 'vaults'; }}
      >VAULTS</button>
      <button type="button"
        class="mode-btn"
        class:active={viewMode === 'content'}
        onclick={() => { viewMode = 'content'; }}
      >CONTENT</button>
    </div>
    {#if viewMode === 'vaults' || viewMode === 'graph'}
      <span class="browser-count">{totalCount}</span>
      <input
        class="browser-search"
        type="text"
        placeholder="filter vaults… ( / )"
        aria-label="filter vault nodes"
        bind:value={searchQuery}
        bind:this={searchInput}
      />
    {/if}
  </div>

  {#if viewMode === 'content'}
    <IndexContentBrowser />
  {:else if viewMode === 'graph'}
    <!-- Kind filter chips (shared with vaults) -->
    {#if activeNodes.length > 0 || walkComplete}
      <div class="kind-chips">
        {#each CATEGORY_ORDER as kind (kind)}
          {@const count = kindCounts[kind] ?? 0}
          {#if count > 0}
            <button
              type="button"
              class="kind-chip kind-chip-{kind}"
              class:active={activeKindFilter === kind}
              onclick={() => toggleKindFilter(kind)}
              title="{KIND_LABEL[kind]} ({count})"
            >
              <span class="chip-glyph">{KIND_GLYPH[kind]}</span>
              <span class="chip-label">{KIND_LABEL[kind]}</span>
              <span class="chip-count">{count}</span>
            </button>
          {/if}
        {/each}
        {#if activeKindFilter}
          <button
            type="button"
            class="kind-chip kind-chip-clear"
            onclick={() => { activeKindFilter = null; }}
            title="Clear filter"
          >ALL</button>
        {/if}
      </div>
    {/if}

    <!-- Force-directed graph view -->
    <div class="graph-container">
      {#if activeNodes.length === 0 && !walkComplete}
        <div class="browser-loading">
          <span class="loading-glyph">◆</span>
          <span>scanning vaults…</span>
        </div>
      {:else}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <canvas
          class="graph-canvas"
          bind:this={_canvasEl}
          onmousedown={onCanvasMouseDown}
          onmousemove={onCanvasMouseMove}
          onmouseup={onCanvasMouseUp}
          onwheel={onCanvasWheel}
          onmouseleave={onCanvasMouseLeave}
        ></canvas>
      {/if}
    </div>
  {:else}
  <!-- Tag chip filters -->
  {#if activeNodes.length > 0 || walkComplete}
    <div class="kind-chips">
      {#each CATEGORY_ORDER as kind (kind)}
        {@const count = kindCounts[kind] ?? 0}
        {#if count > 0}
          <button
            type="button"
            class="kind-chip kind-chip-{kind}"
            class:active={activeKindFilter === kind}
            onclick={() => toggleKindFilter(kind)}
            title="{KIND_LABEL[kind]} ({count})"
          >
            <span class="chip-glyph">{KIND_GLYPH[kind]}</span>
            <span class="chip-label">{KIND_LABEL[kind]}</span>
            <span class="chip-count">{count}</span>
          </button>
        {/if}
      {/each}
      {#if activeKindFilter}
        <button
          type="button"
          class="kind-chip kind-chip-clear"
          onclick={() => { activeKindFilter = null; }}
          title="Clear filter"
        >ALL</button>
      {/if}
    </div>
  {/if}

  <!-- Vault list -->
  <div class="browser-body">
    {#if activeNodes.length === 0 && !walkComplete}
      <div class="browser-loading">
        <span class="loading-glyph">◆</span>
        <span>scanning vaults…</span>
      </div>
    {:else if categories.length === 0 && searchQuery}
      <div class="browser-empty">no vaults match "{searchQuery}"</div>
    {:else}
      <!-- Recents section -->
      {#if recentVaults.length > 0 && !searchQuery && !activeKindFilter}
        <div class="recents-section">
          <div class="recents-header">
            <span class="recents-glyph">◆</span>
            <span>RECENT</span>
          </div>
          {#each recentVaults as vault (vault.id)}
            {@const state = nodeState(vault)}
            {@const conns = connectionsFor(vault.id)}
            <button
              type="button"
              class="vault-row recent-row"
              class:selected={selectedId === vault.id}
              class:dragging={draggingId === vault.id}
              onmouseenter={() => { hoveredId = vault.id; crossRefHighlight.hoveredVaultId = vault.id; }}
              onmouseleave={() => { if (hoveredId === vault.id) hoveredId = null; crossRefHighlight.hoveredVaultId = null; }}
              onclick={() => { selectedId = selectedId === vault.id ? null : vault.id; }}
              onmousedown={(e) => onRowMouseDown(e, vault)}
            >
              <span class="state-dot state-{state}"></span>
              <span class="vault-glyph kind-{vault.kind}">{KIND_GLYPH[vault.kind]}</span>
              <span class="vault-id">{vault.id}</span>
              <span class="vault-name">{vault.displayName || vault.shortLabel || ''}</span>
              <span class="vault-age">{formatAge(vault.updatedMs)}</span>
              {#if conns.length > 0}
                <span class="vault-refs" title={conns.join(', ')}>⟷{conns.length}</span>
              {/if}
            </button>
          {/each}
        </div>
      {/if}

      {#each categories as group (group.kind)}
        <!-- Category header -->
        <button
          type="button"
          class="category-header"
          onclick={() => toggleKind(group.kind)}
          aria-expanded={!group.collapsed}
        >
          <span class="category-chevron" class:collapsed={group.collapsed}>▾</span>
          <span class="category-glyph kind-{group.kind}">{group.glyph}</span>
          <span class="category-label">{group.label}</span>
          <span class="category-count">{group.vaults.length}</span>
        </button>

        <!-- Vault rows -->
        {#if !group.collapsed}
          <div class="category-body">
            {#each group.vaults as vault (vault.id)}
              {@const state = nodeState(vault)}
              {@const conns = connectionsFor(vault.id)}
              {@const isHighlighted = hoveredId === vault.id || hoveredConnections.has(vault.id)}
              {@const isTreeHighlighted = treeHighlightedVaultIds.has(vault.id)}
              {@const isSelected = selectedId === vault.id}
              <button
                type="button"
                class="vault-row"
                class:highlighted={isHighlighted}
                class:tree-highlighted={isTreeHighlighted}
                class:selected={isSelected}
                class:dragging={draggingId === vault.id}
                onmouseenter={() => { hoveredId = vault.id; crossRefHighlight.hoveredVaultId = vault.id; }}
                onmouseleave={() => { if (hoveredId === vault.id) hoveredId = null; crossRefHighlight.hoveredVaultId = null; }}
                onclick={() => { selectedId = selectedId === vault.id ? null : vault.id; }}
                onmousedown={(e) => onRowMouseDown(e, vault)}
              >
                <span class="state-dot state-{state}"></span>
                <span class="vault-glyph kind-{vault.kind}">{KIND_GLYPH[vault.kind]}</span>
                <span class="vault-id">{vault.id}</span>
                <span class="vault-name">{vault.displayName || vault.shortLabel || ''}</span>
                {#if vault.updatedMs}
                  <span class="vault-age">{formatAge(vault.updatedMs)}</span>
                {/if}
                {#if conns.length > 0}
                  <span class="vault-refs" title={conns.join(', ')}>
                    ⟷{conns.length}
                  </span>
                {/if}
              </button>

              <!-- Expanded detail (Phase 2 click-to-expand) -->
              {#if isSelected}
                <div class="vault-detail">
                  {#if vault.path}
                    <div class="detail-row">
                      <span class="detail-label">PATH</span>
                      <span class="detail-value">{vault.path}</span>
                    </div>
                  {/if}
                  {#if conns.length > 0}
                    <div class="detail-row">
                      <span class="detail-label">LINKS</span>
                      <span class="detail-value detail-links">
                        {#each conns as ref}
                          <span
                            class="detail-link kind-{inferKind(ref)}"
                            role="button"
                            tabindex="0"
                            onclick={(e) => { e.stopPropagation(); selectedId = ref; }}
                            onkeydown={(e) => { if (e.key === 'Enter') { selectedId = ref; } }}
                          >{ref}</span>
                        {/each}
                      </span>
                    </div>
                  {/if}
                  {#if vault.updatedMs}
                    <div class="detail-row">
                      <span class="detail-label">MODIFIED</span>
                      <span class="detail-value">{new Date(vault.updatedMs).toLocaleString()}</span>
                    </div>
                  {/if}
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      {/each}
    {/if}
  </div>
  {/if}
</div>

<!-- Graph tooltip -->
{#if tooltipVisible && tooltipNode}
  {@const conns = connectionsFor(tooltipNode.id)}
  <div
    class="graph-tooltip"
    style="left: {tooltipX}px; top: {tooltipY}px;"
  >
    <div class="tooltip-title kind-{tooltipNode.kind}">
      {tooltipNode.label}
    </div>
    <div class="tooltip-kind">{KIND_LABEL[tooltipNode.kind]}</div>
    {#if tooltipNode.updatedMs}
      <div class="tooltip-age">{formatAge(tooltipNode.updatedMs)} ago</div>
    {/if}
    {#if conns.length > 0}
      <div class="tooltip-conns">
        <span class="tooltip-conns-count">{conns.length} link{conns.length === 1 ? '' : 's'}</span>
        <span class="tooltip-conns-ids">{conns.join(', ')}</span>
      </div>
    {/if}
  </div>
{/if}

<!-- Drag ghost -->
{#if ghostVisible}
  <div
    class="drag-ghost kind-{ghostKind}"
    style="left: {ghostX}px; top: {ghostY}px;"
  >
    <span class="drag-ghost-glyph">{KIND_GLYPH[ghostKind]}</span>
    <span>{ghostLabel}</span>
  </div>
{/if}

<style>
  .index-browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-base);
    font-family: var(--font-family);
    font-size: var(--text-base);
    color: var(--amber-warm);
    user-select: none;
  }

  /* Header: mode toggle + count + search */
  .browser-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    background: var(--bg-surface);
    box-shadow: var(--sep-glow);
    flex-shrink: 0;
  }
  .mode-toggle {
    display: flex;
    gap: 0;
  }
  .mode-btn {
    padding: 3px var(--space-md);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    background: var(--bg-elevated);
    color: var(--amber-faint);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    cursor: pointer;
    transition: all 0.12s;
  }
  .mode-btn:first-child { border-top-right-radius: 0; border-bottom-right-radius: 0; border-right: none; }
  .mode-btn:not(:first-child):not(:last-child) { border-radius: 0; border-right: none; }
  .mode-btn:last-child { border-top-left-radius: 0; border-bottom-left-radius: 0; }
  .mode-btn.active {
    color: var(--term-cyan, #6FE0E0);
    border-color: var(--term-cyan, #6FE0E0);
    background: rgba(74, 212, 212, 0.1);
    box-shadow: 0 0 6px rgba(74, 212, 212, 0.15);
  }
  .mode-btn:hover:not(.active) {
    color: var(--amber-dim);
    border-color: var(--amber-dim);
    background: var(--bg-hover);
  }
  .browser-title {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    color: var(--amber-dim);
  }
  .browser-count {
    font-size: var(--text-2xs);
    font-weight: 700;
    color: var(--bg-base);
    background: var(--amber-dim);
    padding: 2px 7px;
    min-width: 20px;
    text-align: center;
    border-radius: 10px;
  }
  .browser-search {
    flex: 1;
    min-width: 0;
    height: var(--control-sm);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    padding: 0 var(--space-md);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .browser-search::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .browser-search:focus {
    border-color: var(--amber-dim);
    box-shadow: 0 0 0 1px rgba(255, 168, 38, 0.15),
                inset 0 1px 3px rgba(0, 0, 0, 0.2);
  }

  /* Kind filter chips */
  .kind-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-12) var(--space-8);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .kind-chip {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 3px var(--space-8);
    border: 1px solid var(--border-subtle);
    border-radius: 12px;
    background: var(--bg-elevated);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    cursor: pointer;
    transition: all 0.12s;
  }
  .kind-chip:hover {
    border-color: var(--amber-dim);
    background: var(--bg-hover);
    color: var(--amber-warm);
  }
  .kind-chip.active {
    border-color: currentColor;
    box-shadow: 0 0 6px rgba(255, 168, 38, 0.15);
  }
  .kind-chip-p.active       { color: var(--amber-bright); background: rgba(255, 200, 64, 0.1); }
  .kind-chip-pr.active      { color: var(--amber-warm);   background: rgba(240, 160, 48, 0.1); }
  .kind-chip-r.active       { color: var(--term-cyan);    background: rgba(111, 224, 224, 0.1); }
  .kind-chip-s.active       { color: var(--term-blue);    background: rgba(108, 182, 255, 0.1); }
  .kind-chip-lore.active    { color: var(--term-purple);  background: rgba(197, 143, 255, 0.1); }
  .kind-chip-agt.active     { color: var(--term-green);   background: rgba(79, 232, 85, 0.1); }
  .kind-chip-h.active       { color: var(--amber-faint);  background: rgba(168, 120, 48, 0.1); }
  .kind-chip-clear {
    color: var(--amber-faint);
    border-style: dashed;
    font-size: 8px;
    letter-spacing: 0.1em;
  }
  .kind-chip-clear:hover { color: var(--amber-warm); border-style: solid; }
  .chip-glyph { font-size: 10px; }
  .chip-label { text-transform: uppercase; }
  .chip-count {
    font-size: 8px;
    color: var(--amber-faint);
    font-weight: 400;
  }
  .kind-chip.active .chip-count { color: inherit; opacity: 0.7; }

  /* Recents section */
  .recents-section {
    box-shadow: var(--sep-depth);
    background: linear-gradient(to bottom, rgba(255, 200, 64, 0.03), transparent);
  }
  .recents-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-12);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    box-shadow: var(--sep-depth);
    background: var(--bg-surface);
  }
  .recents-glyph {
    font-size: var(--text-xs);
    text-shadow: var(--glow-amber);
  }
  .recent-row {
    background: rgba(255, 200, 64, 0.02);
  }
  .recent-row:hover {
    background: rgba(255, 200, 64, 0.06);
  }

  /* Scrollable body */
  .browser-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-color: var(--amber-faint) transparent;
    scrollbar-width: thin;
  }
  .browser-body::-webkit-scrollbar { width: 6px; }
  .browser-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* Loading / empty */
  .browser-loading, .browser-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-8);
    padding: var(--space-24) var(--space-12);
    color: var(--amber-faint);
    font-size: var(--text-sm);
    font-style: italic;
  }
  .loading-glyph {
    font-size: 16px;
    font-style: normal;
    animation: pulse-glyph 1.6s ease-in-out infinite;
  }
  @keyframes pulse-glyph {
    0%, 100% { opacity: 0.3; text-shadow: none; }
    50%      { opacity: 1;   text-shadow: var(--glow-amber); }
  }

  /* Category header */
  .category-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    width: 100%;
    padding: 7px var(--space-12);
    background: var(--bg-surface);
    border: none;
    box-shadow: var(--sep-depth);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    cursor: pointer;
    transition: color 0.12s, background 0.12s;
    text-align: left;
  }
  .category-header:hover {
    color: var(--amber-warm);
    background: var(--bg-hover);
  }
  .category-chevron {
    font-size: var(--text-xs);
    transition: transform 0.15s ease;
    display: inline-block;
    width: 12px;
    text-align: center;
  }
  .category-chevron.collapsed { transform: rotate(-90deg); }
  .category-glyph { font-size: 11px; }
  .category-label { flex: 1; }
  .category-count {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    font-weight: 400;
  }

  /* Category body */
  .category-body {
    box-shadow: var(--sep-depth);
  }

  /* Vault row */
  .vault-row {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    width: 100%;
    padding: 5px var(--space-12) 5px 22px;
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    text-align: left;
    transition: background 0.1s, border-color 0.1s, color 0.1s;
  }
  .vault-row:hover {
    background: var(--bg-hover);
    border-left-color: var(--amber-dim);
  }
  .vault-row.highlighted {
    background: rgba(255, 168, 38, 0.12);
    border-left-color: var(--amber-bright);
    animation: xref-flash 0.3s ease-out;
  }
  .vault-row.tree-highlighted {
    background: rgba(74, 212, 212, 0.1);
    border-left-color: var(--term-cyan);
    animation: xref-cyan-flash 0.3s ease-out;
  }
  @keyframes xref-flash {
    from { background: rgba(255, 168, 38, 0.25); }
    to   { background: rgba(255, 168, 38, 0.12); }
  }
  @keyframes xref-cyan-flash {
    from { background: rgba(74, 212, 212, 0.22); }
    to   { background: rgba(74, 212, 212, 0.1); }
  }
  .vault-row.selected {
    background: var(--bg-hover);
    border-left-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .vault-row.dragging {
    opacity: 0.4;
  }

  /* State dot */
  .state-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .state-active    { background: var(--amber-bright); box-shadow: 0 0 6px var(--amber-bright); }
  .state-recent    { background: var(--amber-warm); }
  .state-ambient   { background: var(--amber-faint); }
  .state-background { background: var(--border-subtle); }

  /* Vault row elements */
  .vault-glyph {
    font-size: var(--text-xs);
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }
  .vault-id {
    font-weight: 600;
    color: var(--amber-primary);
    min-width: 48px;
    flex-shrink: 0;
  }
  .vault-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--amber-dim);
    font-size: var(--text-xs);
  }
  .vault-age {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    font-style: italic;
    flex-shrink: 0;
  }
  .vault-refs {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    flex-shrink: 0;
    padding: 0 var(--space-xs);
    border: 1px solid var(--border-subtle);
    line-height: 14px;
  }

  /* Kind colors — applied via .kind-X on glyphs */
  .kind-p    { color: var(--amber-bright); }
  .kind-pr   { color: var(--amber-warm); }
  .kind-r    { color: var(--term-cyan); }
  .kind-s    { color: var(--term-blue); }
  .kind-lore { color: var(--term-purple); }
  .kind-agt  { color: var(--term-green); }
  .kind-h    { color: var(--amber-faint); }

  /* Selected vault detail */
  .vault-detail {
    padding: var(--space-sm) var(--space-md) var(--space-8) 42px;
    background: var(--bg-surface);
    border-left: 2px solid var(--amber-dim);
    font-size: var(--text-xs);
  }
  .detail-row {
    display: flex;
    gap: var(--space-8);
    padding: 2px 0;
    align-items: baseline;
  }
  .detail-label {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    min-width: 52px;
    flex-shrink: 0;
  }
  .detail-value {
    color: var(--amber-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .detail-links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    white-space: normal;
  }
  .detail-link {
    padding: 1px var(--space-sm);
    border: 1px solid var(--border-subtle);
    cursor: pointer;
    font-size: var(--text-2xs);
    transition: border-color 0.12s, background 0.12s;
  }
  .detail-link:hover {
    border-color: currentColor;
    background: rgba(255, 168, 38, 0.08);
  }

  /* Drag ghost */
  .drag-ghost {
    position: fixed;
    pointer-events: none;
    z-index: 5000;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md);
    background: var(--bg-elevated);
    border: 1px solid var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    color: var(--amber-bright);
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.5);
    opacity: 0.92;
  }
  .drag-ghost-glyph { font-size: 12px; }

  /* =========================================================================
     Force-directed graph view (Canvas)
     ========================================================================= */

  .graph-container {
    flex: 1;
    position: relative;
    overflow: hidden;
    background: var(--bg-base);
    min-height: 0;
  }

  .graph-canvas {
    width: 100%;
    height: 100%;
    display: block;
    cursor: grab;
  }
  .graph-canvas:active {
    cursor: grabbing;
  }

  /* Graph tooltip */
  .graph-tooltip {
    position: fixed;
    pointer-events: none;
    z-index: 5000;
    padding: var(--space-sm) var(--space-md);
    background: var(--bg-elevated);
    border: 1px solid var(--amber-dim);
    border-radius: var(--radius-md, 4px);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    color: var(--amber-warm);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);
    max-width: 260px;
    white-space: nowrap;
  }
  .tooltip-title {
    font-weight: 700;
    font-size: var(--text-sm);
    margin-bottom: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tooltip-kind {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--amber-faint);
    margin-bottom: 3px;
  }
  .tooltip-age {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    font-style: italic;
  }
  .tooltip-conns {
    margin-top: 4px;
    font-size: var(--text-2xs);
    color: var(--amber-dim);
    border-top: 1px solid var(--border-subtle);
    padding-top: 3px;
  }
  .tooltip-conns-count {
    font-weight: 600;
    margin-right: var(--space-xs);
  }
  .tooltip-conns-ids {
    color: var(--amber-faint);
  }
</style>
