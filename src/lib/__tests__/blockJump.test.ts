/**
 * Unit tests for blockJump.ts (N3.4) — the jump-to-block scroller registry.
 * Plain module singleton, so each test cleans up the sessionIds it registers.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  registerScroller,
  unregisterScroller,
  paneForSession,
  requestScrollToBlock,
} from '../blockJump';

describe('blockJump registry', () => {
  it('routes a request to the registered pane scroller', () => {
    const scroll = vi.fn();
    registerScroller(101, 7, scroll);
    expect(requestScrollToBlock(101, 'blk-3')).toBe(true);
    expect(scroll).toHaveBeenCalledWith('blk-3');
    unregisterScroller(101);
  });

  it('returns false when no pane owns the session', () => {
    const scroll = vi.fn();
    expect(requestScrollToBlock(999, 'blk-1')).toBe(false);
    expect(scroll).not.toHaveBeenCalled();
  });

  it('resolves the owning paneId for tab activation', () => {
    registerScroller(202, 12, vi.fn());
    expect(paneForSession(202)).toBe(12);
    expect(paneForSession(404)).toBeUndefined();
    unregisterScroller(202);
  });

  it('stops routing once unregistered', () => {
    const scroll = vi.fn();
    registerScroller(303, 5, scroll);
    unregisterScroller(303);
    expect(requestScrollToBlock(303, 'blk-9')).toBe(false);
    expect(paneForSession(303)).toBeUndefined();
    expect(scroll).not.toHaveBeenCalled();
  });

  it('re-registering a session replaces the prior scroller + paneId', () => {
    const first = vi.fn();
    const second = vi.fn();
    registerScroller(404, 1, first);
    registerScroller(404, 2, second);
    expect(paneForSession(404)).toBe(2);
    requestScrollToBlock(404, 'blk-0');
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith('blk-0');
    unregisterScroller(404);
  });
});
