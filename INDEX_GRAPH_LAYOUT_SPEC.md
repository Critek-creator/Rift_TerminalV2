# IndexGraph Layout Rewrite — Implementation Spec

**Target file:** `src/lib/IndexGraph.svelte`  
**Goal:** Fix label overlap, handle asymmetric category sizes, scale to 200+ nodes.  
**Constraint:** Zero changes to data flow, bus subscription, drag-to-terminal, or Rust backend.

---

## Problem Statement

The current `d3.tree()` horizontal dendrogram fails because:

1. **No collapse/expand.** HISTORY alone has 30+ archive nodes. Every node renders simultaneously, making the graph unreadable at any zoom level.
2. **No label collision detection.** Labels extend rightward with a fixed `dx` offset. When sibling nodes are closer than label height, text overlaps.
3. **Fixed `nodeSize` can't adapt.** Categories with 3 nodes get the same spacing as categories with 30. The tree layout treats them uniformly, wasting space on small categories and crushing large ones.

---

## Architecture Overview

Replace the single `d3.tree()` layout with a **two-tier system**:

- **Tier 1 — Category ring:** The 7 category group nodes (`PROJECTS`, `PRACTICES`, `RESEARCH`, `SKILLS`, `LORE`, `AGENTS`, `HISTORY`) are positioned in a radial layout around the `INDEX` root. Fixed positions. Always visible. Each shows a **count badge** (e.g., `HISTORY (32)`).
- **Tier 2 — Expanded children:** When a category is expanded, its children render in a **local vertical list** extending outward from the category node. Only one category expands at a time (single-expand mode).

This means the default view shows ~8 nodes (INDEX + 7 categories). Clean. Readable. Navigable.

---

## Detailed Implementation

### 1. Collapse/Expand State

Add reactive state to track which categories are expanded:

```typescript
/** Set of expanded category kind keys. Default: all collapsed. */
let expandedKinds = $state<Set<VaultKind>>(new Set());

/** Toggle a category's expand/collapse state (single-expand mode). */
function toggleKind(kind: VaultKind): void {
  if (expandedKinds.has(kind)) {
    expandedKinds = new Set(); // collapse all
  } else {
    expandedKinds = new Set([kind]); // expand only this one
  }
}
```

**Wire the click handler** on group nodes. In the `<g>` element for group nodes (the `{#each renderedNodes}` block), add:

```svelte
onclick={() => {
  if (node.isGroup) toggleKind(node.kind);
}}
```

And change the group node cursor from `'default'` to `'pointer'`.

### 2. Category Ring Layout (Tier 1)

Replace the `d3.tree()` layout with a hand-computed radial layout for the category nodes. This gives full control over spacing and eliminates the tree layout's uniform-spacing problem.

```typescript
// Inside the $effect that currently builds the tree layout:

const INDEX_ID = '__INDEX__';
const CENTER_X = 0;
const CENTER_Y = 0;
const RING_RADIUS = 180 * indexDensityScale;

// Position category group nodes in a circle around INDEX
const presentKinds = KIND_ORDER.filter((k) => kindGroups.has(k));
const angleStep = (2 * Math.PI) / presentKinds.length;
// Start from top (12 o'clock = -π/2)
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
```

### 3. Expanded Children Layout (Tier 2)

When a category is expanded, position its children in a vertical column extending **outward** from the category node (away from center). This ensures children don't overlap the central ring.

```typescript
const CHILD_OFFSET_OUTWARD = 100; // Distance from group node to first child
const CHILD_ROW_HEIGHT = 20 * indexDensityScale; // Vertical spacing between children

for (const kind of presentKinds) {
  if (!expandedKinds.has(kind)) continue; // skip collapsed

  const groupId = `__GROUP_${kind}__`;
  const groupPos = posMap.get(groupId)!;
  const nodes = kindGroups.get(kind)!;

  // Direction vector: from center to group node (normalized)
  const dx = groupPos.x - CENTER_X;
  const dy = groupPos.y - CENTER_Y;
  const dist = Math.sqrt(dx * dx + dy * dy) || 1;
  const dirX = dx / dist;
  const dirY = dy / dist;

  // Perpendicular vector for fanning children vertically
  // relative to the outward direction
  const perpX = -dirY;
  const perpY = dirX;

  // Start point: offset outward from group node
  const startX = groupPos.x + dirX * CHILD_OFFSET_OUTWARD;
  const startY = groupPos.y + dirY * CHILD_OFFSET_OUTWARD;

  // Build list: top-level vaults (with sub-doors nested under parents)
  // Reuse existing parent/child nesting logic from the current code.
  // topLevel = array of nodes that have no parentId (or whose parent
  // is not in this kind group). Sub-doors are nested under their parent.
  const topLevel = buildTopLevelList(nodes);

  // Center the fan: offset by half the total height
  const totalHeight = (topLevel.length - 1) * CHILD_ROW_HEIGHT;
  const offsetStart = -totalHeight / 2;

  for (let j = 0; j < topLevel.length; j++) {
    const child = topLevel[j];
    const fanOffset = offsetStart + j * CHILD_ROW_HEIGHT;
    posMap.set(child.id, {
      x: startX + perpX * fanOffset,
      y: startY + perpY * fanOffset,
    });

    // Sub-doors: indent further outward
    if (child.children) {
      for (let k = 0; k < child.children.length; k++) {
        const sub = child.children[k];
        const subFanOffset = offsetStart
          + (j + 0.5 + k * 0.6) * CHILD_ROW_HEIGHT;
        posMap.set(sub.id, {
          x: startX + dirX * 80 + perpX * subFanOffset,
          y: startY + dirY * 80 + perpY * subFanOffset,
        });
      }
    }
  }
}
```

**`buildTopLevelList` helper** — extract from the existing two-pass nesting logic (currently inside the `rootData` builder around lines 720-755). Factor it into a standalone function:

```typescript
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
```

### 4. Count Badge on Collapsed Categories

When a category is collapsed, show the count next to the label. Modify the `RenderedNode` building:

```typescript
// When building a group node:
const childCount = kindGroups.get(d.kind)?.length ?? 0;
const isExpanded = expandedKinds.has(d.kind);

allNodes.push({
  // ... existing fields ...
  label: isExpanded ? d.label : `${d.label} (${childCount})`,
  cursor: 'pointer', // was 'default' — now clickable
});
```

### 5. Only Render Visible Nodes

**Do NOT filter after the fact.** Instead, only add entries to `posMap` for nodes that should be visible:

- INDEX root: always in posMap
- Group nodes: always in posMap
- Leaf nodes: only in posMap if their parent kind is in `expandedKinds`

Nodes without a position in `posMap` simply don't get added to `renderedNodes`. Links with a missing source or target also don't render (existing null-guard on `posMap.get()` handles this).

### 6. Label Collision Avoidance

**Strategy A — Adaptive spacing (primary):**

Each category's children are in an isolated fan (not sharing space with other categories), so collision between categories is impossible. Within a single category, `CHILD_ROW_HEIGHT` of 20px at standard density gives enough room for 9px labels.

**Strategy B — Truncation with hover reveal (secondary):**

For leaf labels, enforce a max character width:

```typescript
const MAX_LABEL_CHARS = 30;
function truncateLabel(text: string): string {
  if (text.length <= MAX_LABEL_CHARS) return text;
  return text.slice(0, MAX_LABEL_CHARS - 1) + '…';
}
```

Apply in the leaf label `<text>` element. The full label is already in `node.tooltip` via the `<title>` element.

**Strategy C — Outward-aligned labels (keeps labels from overlapping the graph center):**

Leaf labels should use `text-anchor` based on which side of center they're on:

```typescript
// When building RenderedNode for a leaf:
const isRightSide = pos.x >= CENTER_X;
labelAnchor: isRightSide ? 'start' : 'end',
// And in the template, use dx of +(r+6) for right-side, -(r+6) for left-side:
dx={isRightSide ? node.r + 6 : -(node.r + 6)}
```

### 7. Visual Polish

**Expand/collapse indicator** — small `+`/`−` near the group circle:

```svelte
{#if node.isGroup}
  <text
    class="node-expand-indicator"
    dx={node.r + 2}
    dy={-node.r + 2}
    text-anchor="start"
    font-size="8"
    fill="var(--amber-dim)"
    style="pointer-events: none;"
  >
    {expandedKinds.has(node.kind) ? '−' : '+'}
  </text>
{/if}
```

CSS for the indicator:

```css
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
```

**Group node hover glow:**

```css
:global(.graph-node.group-node) {
  cursor: pointer;
}
:global(.graph-node.group-node:hover .node-circle) {
  filter: url(#amber-glow);
  stroke-width: 2;
}
```

---

## What NOT to Change

Preserve all of these existing systems exactly as-is:

1. **Data flow:** `onMount` → `subscribe({ category: 'index' })` → `liveNodeMap` → `activeNodes`/`activeEdges` derivations. Zero changes.
2. **Debounce buffer:** `pendingUpdates`/`pendingDeletes`/`flushPendingVaults()`. Zero changes.
3. **Drag-to-terminal gesture:** `onNodeMouseDown` → `onDocMouseMove` → `onDocMouseUp` → `RIFT_VAULT_DROP_EVENT`. Zero changes. Leaf nodes remain draggable; group nodes and INDEX do not.
4. **d3-zoom:** `zoomBehavior` creation, filter, `.on('zoom')`, `zoomFitDone` guard. Zero changes except: re-run zoom-to-fit when `expandedKinds` changes (see below).
5. **Boot reveal animation:** `bootProgress`, `bootRevealLabel()`, `bootRevealStarted`. Zero changes.
6. **Pulse animation:** `pulsingIds`, `pulseVault()`, `PULSE_DURATION_MS`. Zero changes.
7. **Config subscription:** `loadIndexConfig()`, `rift:config-changed` listener, `indexDensityScale`. Zero changes.
8. **Static fixture fallback:** `STATIC_NODES`/`STATIC_LINKS` used when `walkComplete && liveNodeMap.size === 0`. Zero changes.
9. **Rendered state shape:** `RenderedNode`, `RenderedLink`, `RenderedCluster` interfaces. Minimal additions only.
10. **All CSS classes and color variables.** Existing styles must continue to work.

---

## Zoom-to-Fit on Expand/Collapse

When `expandedKinds` changes, re-fit the view to the new bounding box:

```typescript
// Track previous expandedKinds to detect changes
let prevExpandedSnapshot = '';

// Inside the $effect, after computing posMap and renderedNodes:
const expandedSnapshot = [...expandedKinds].sort().join(',');
if (expandedSnapshot !== prevExpandedSnapshot) {
  prevExpandedSnapshot = expandedSnapshot;
  // Re-fit with a smooth transition
  const fitTimeout = window.setTimeout(() => {
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
    const rect = container.getBoundingClientRect();
    const W = rect.width || 640;
    const H = rect.height || 480;
    const k = Math.max(0.4, Math.min(
      (W - margin * 2) / bw,
      (H - margin * 2) / bh,
      2.0
    ));
    const tx = W / 2 - ((minX + maxX) / 2) * k;
    const ty = H / 2 - ((minY + maxY) / 2) * k;
    const fitTransform = zoomIdentity.translate(tx, ty).scale(k);
    const svg = select<SVGSVGElement, unknown>(container);
    // Animated transition (300ms) for smooth expand/collapse
    svg.transition().duration(300)
      .call(zoomBehavior!.transform, fitTransform);
  }, 50);
}
```

---

## Edge Rendering Changes

**Index spokes (INDEX → group):** Always render. Connect center to each category.

**Tree edges (group → child):** Only render when the group is expanded. The `posMap.get()` null-guard handles this automatically — if a child has no position, the link path can't be computed, so it's skipped.

**Cross-ref edges:** Only render if BOTH endpoints are visible. Same null-guard handles it.

No explicit edge filtering code needed — it falls out naturally from only populating `posMap` for visible nodes.

---

## Migration Checklist

Each step is independently testable. Complete in order.

### Step 1: Add `expandedKinds` state + `toggleKind()`
- Add the reactive state and toggle function (section 1)
- Wire `onclick` on group nodes in the template
- Change group cursor to `'pointer'`
- **Test:** Click a group node → `console.log([...expandedKinds])` confirms toggle

### Step 2: Replace `d3.tree()` with radial category layout
- Remove the `hierarchy()` + `tree()` + `treeLayout()` call chain
- Implement the category ring layout (section 2)
- Keep all children hidden initially (skip section 3 for now)
- **Test:** Graph shows INDEX + 7 category nodes in a clean ring, no children

### Step 3: Add expanded children layout
- Extract `buildTopLevelList()` helper
- Implement the outward-fan positioning (section 3)
- Only populate `posMap` entries for children of expanded categories
- **Test:** Click SKILLS (small category) → children fan outward cleanly. Click HISTORY (large) → 30+ nodes fan out without overlapping the ring.

### Step 4: Count badges
- Show `HISTORY (32)` on collapsed categories, just `HISTORY` when expanded
- **Test:** Visual confirmation

### Step 5: Label truncation + directional anchoring
- Add `truncateLabel()` (section 6B)
- Add left/right label anchor logic (section 6C)
- **Test:** Long archive labels truncate. Labels on left side of ring point left.

### Step 6: Visual polish
- Add `+`/`−` indicator on group nodes (section 7)
- Add hover glow on group nodes
- **Test:** Hovering a group glows it; indicator shows `+` collapsed, `−` expanded

### Step 7: Zoom-to-fit on toggle
- Implement animated re-fit when `expandedKinds` changes
- **Test:** Expanding HISTORY (30+ nodes) smoothly zooms out to fit

### Step 8: Cleanup
- Remove unused `d3-hierarchy` imports (`hierarchy`, `tree`) if fully replaced
- Remove `TreeDatum` interface if replaced by `TreeChild`
- Remove `renderedClusters` (was already `[]`)
- Verify: drag-to-terminal works, boot reveal plays, pulse animates, zoom/pan works, static fixture renders when no live data

---

## d3 Dependency Changes

| Before | After |
|--------|-------|
| `import { hierarchy, tree } from 'd3-hierarchy'` | Remove — no longer needed |
| `d3.tree().nodeSize()` computes all positions | Hand-computed ring + fan positions |
| `d3-zoom` for pan/zoom | Keep as-is |
| `d3-selection` for zoom binding | Keep as-is |

---

## File Scope

**Only `src/lib/IndexGraph.svelte` changes.** No other files are affected. The component's external API (props, events, bus subscription) is unchanged.
