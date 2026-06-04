// blockJump.ts — N3.4: route a "jump to command block" request to the terminal
// pane that owns it, mirroring terminalInject's registry idiom.
//
// The command palette (cross-pane, N1) lists bookmarked blocks and, on select,
// activates the owning tab and asks its terminal to scroll to the block. Blocks
// are keyed by PTY sessionId (what commandBlockStore records); activation needs
// the frontend paneId (what sessionManager.activateSession expects), so each
// Terminal registers both.
//
// Scrolling is visible-aware INSIDE Terminal (a hidden pane defers the scroll
// until its next visible→refit), so the palette can activate the tab and request
// the scroll in the same turn without a timing race — the same pending-consume
// pattern sessionManager uses for initial commands.
//
// Plain module state (not Svelte `$state`) — callers are imperative.

type Scroller = (blockId: string) => void;

interface PaneScroll {
  scroll: Scroller;
  paneId: number;
}

const registry = new Map<number, PaneScroll>(); // keyed by PTY sessionId

/** Register a pane's block-scroller. Call once the PTY sessionId is known. */
export function registerScroller(sessionId: number, paneId: number, scroll: Scroller): void {
  registry.set(sessionId, { scroll, paneId });
}

/** Drop a pane's scroller. Call on Terminal cleanup. */
export function unregisterScroller(sessionId: number): void {
  registry.delete(sessionId);
}

/** The frontend pane/tab id that owns a block's PTY session, for tab
 *  activation — or undefined if no pane is registered (tab closed). */
export function paneForSession(sessionId: number): number | undefined {
  return registry.get(sessionId)?.paneId;
}

/** Ask the owning pane to scroll to a block. Returns false when no pane is
 *  registered for that session, so the caller can no-op gracefully. */
export function requestScrollToBlock(sessionId: number, blockId: string): boolean {
  const entry = registry.get(sessionId);
  if (!entry) return false;
  entry.scroll(blockId);
  return true;
}
