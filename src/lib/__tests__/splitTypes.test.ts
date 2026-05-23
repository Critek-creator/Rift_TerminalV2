import { describe, it, expect } from 'vitest';
import { collectLeafIds, replaceLeaf, removeLeaf } from '../splitTypes';
import type { SplitNode } from '../splitTypes';

// Unit tests for splitTypes.ts — split-pane layout tree operations:
// collectLeafIds, replaceLeaf, removeLeaf.

const leaf = (id: number): SplitNode => ({ type: 'terminal', id });

const hsplit = (a: SplitNode, b: SplitNode, ratio = 0.5): SplitNode => ({
  type: 'hsplit',
  children: [a, b],
  ratio,
});

const vsplit = (a: SplitNode, b: SplitNode, ratio = 0.5): SplitNode => ({
  type: 'vsplit',
  children: [a, b],
  ratio,
});

describe('collectLeafIds', () => {
  it('returns single id from a terminal node', () => {
    expect(collectLeafIds(leaf(1))).toEqual([1]);
  });

  it('collects all leaf ids from a flat split', () => {
    const tree = hsplit(leaf(1), leaf(2));
    expect(collectLeafIds(tree)).toEqual([1, 2]);
  });

  it('collects from deeply nested splits in left-to-right order', () => {
    const tree = vsplit(
      hsplit(leaf(1), leaf(2)),
      hsplit(leaf(3), vsplit(leaf(4), leaf(5))),
    );
    expect(collectLeafIds(tree)).toEqual([1, 2, 3, 4, 5]);
  });
});

describe('replaceLeaf', () => {
  it('replaces a matching terminal node', () => {
    const tree = leaf(1);
    const result = replaceLeaf(tree, 1, leaf(99));
    expect(result).toEqual(leaf(99));
  });

  it('returns original node if target id not found', () => {
    const tree = leaf(1);
    const result = replaceLeaf(tree, 999, leaf(99));
    expect(result).toBe(tree);
  });

  it('replaces leaf within a nested split', () => {
    const tree = hsplit(leaf(1), vsplit(leaf(2), leaf(3)));
    const replacement = hsplit(leaf(10), leaf(11));
    const result = replaceLeaf(tree, 2, replacement);

    // Leaf 2 should now be an hsplit of 10 and 11
    expect(collectLeafIds(result)).toEqual([1, 10, 11, 3]);
  });

  it('preserves ratio and type of parent splits', () => {
    const tree = vsplit(leaf(1), leaf(2), 0.7);
    const result = replaceLeaf(tree, 2, leaf(99));
    expect(result.type).toBe('vsplit');
    expect((result as { ratio: number }).ratio).toBe(0.7);
  });
});

describe('removeLeaf', () => {
  it('returns null when removing the only terminal (last pane)', () => {
    expect(removeLeaf(leaf(1), 1)).toBeNull();
  });

  it('returns unchanged node when target id not found', () => {
    const tree = leaf(1);
    expect(removeLeaf(tree, 999)).toBe(tree);
  });

  it('collapses to sibling when removing one child of a split', () => {
    const tree = hsplit(leaf(1), leaf(2));
    const result = removeLeaf(tree, 1);
    expect(result).toEqual(leaf(2));
  });

  it('collapses to sibling when removing the other child', () => {
    const tree = hsplit(leaf(1), leaf(2));
    const result = removeLeaf(tree, 2);
    expect(result).toEqual(leaf(1));
  });

  it('handles removal from deeply nested tree', () => {
    const tree = vsplit(
      hsplit(leaf(1), leaf(2)),
      leaf(3),
    );
    // Remove leaf 1 — its parent hsplit collapses to leaf 2
    const result = removeLeaf(tree, 1);
    expect(result).not.toBeNull();
    expect(collectLeafIds(result!)).toEqual([2, 3]);
  });

  it('handles removal when target is nested on the right', () => {
    const tree = vsplit(
      leaf(1),
      hsplit(leaf(2), leaf(3)),
    );
    // Remove leaf 3 — hsplit collapses to leaf 2
    const result = removeLeaf(tree, 3);
    expect(result).not.toBeNull();
    expect(collectLeafIds(result!)).toEqual([1, 2]);
  });
});
