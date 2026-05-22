// splitTypes.ts — split-pane layout tree for Rift Terminal V2.
//
// A SplitNode describes the recursive layout of terminal panes within a single
// session tab. A leaf node ('terminal') holds the numeric id of the PTY session.
// Branch nodes ('hsplit' / 'vsplit') hold two children and a ratio in [0, 1]
// that controls how space is divided between them.
//
// Direction naming convention (matches Splitter.svelte's bar-geometry names):
//   'hsplit' — a horizontal bar divides TOP (children[0]) and BOTTOM (children[1])
//   'vsplit' — a vertical bar divides LEFT (children[0]) and RIGHT (children[1])

export type SplitNode =
  | { type: 'terminal'; id: number }
  | { type: 'hsplit' | 'vsplit'; children: [SplitNode, SplitNode]; ratio: number };

/**
 * Walk the layout tree and collect every terminal leaf id.
 */
export function collectLeafIds(node: SplitNode): number[] {
  if (node.type === 'terminal') return [node.id];
  return [...collectLeafIds(node.children[0]), ...collectLeafIds(node.children[1])];
}

/**
 * Replace the leaf with `targetId` in the tree with `replacement`.
 * Returns the original node unchanged if `targetId` is not found.
 */
export function replaceLeaf(node: SplitNode, targetId: number, replacement: SplitNode): SplitNode {
  if (node.type === 'terminal') {
    return node.id === targetId ? replacement : node;
  }
  return {
    ...node,
    children: [
      replaceLeaf(node.children[0], targetId, replacement),
      replaceLeaf(node.children[1], targetId, replacement),
    ],
  };
}

/**
 * Remove the leaf with `targetId` from the tree, collapsing its parent split
 * into the sibling. Returns null if the tree itself is the leaf being removed
 * (i.e. last pane in the tab — callers should close the tab instead).
 */
export function removeLeaf(node: SplitNode, targetId: number): SplitNode | null {
  if (node.type === 'terminal') {
    return node.id === targetId ? null : node;
  }
  const [a, b] = node.children;
  if (a.type === 'terminal' && a.id === targetId) return b;
  if (b.type === 'terminal' && b.id === targetId) return a;
  const newA = removeLeaf(a, targetId);
  const newB = removeLeaf(b, targetId);
  if (newA === null) return newB;
  if (newB === null) return newA;
  return { ...node, children: [newA, newB] };
}
