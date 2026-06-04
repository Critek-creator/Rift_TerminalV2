// commandBlockStore.svelte.ts — N3.1: addressable command blocks (the N3 spine).
//
// Principle N5/N3 of RIFT_IA_NORTHSTAR.md: "blocks, not scroll". Every command
// that runs becomes a CommandBlock — a named, addressable unit
// {command, cwd, exit, duration, ts} keyed by a stable id. This is the data
// layer behind the N3 block affordances built on top of it:
//   • N3.2 — the sticky command header (which block spans the viewport top)
//   • N3.3 — block copy + bookmark
//   • N3.4 — palette jump-to-block
//
// It deliberately holds NO xterm objects. Live row anchoring (xterm IMarkers,
// which auto-track reflow + scrollback eviction) is owned per-pane by
// Terminal.svelte, since a marker is bound to one terminal instance and cannot
// live in a cross-pane store. `startRow`/`endRow` here are SNAPSHOT-TIME
// integers captured at completion — useful for reference/display, but NOT the
// authoritative live anchor (a raw row drifts as scrollback evicts; the pane
// marker added in N3.2 is the truth). Keeping them is cheap and lets the store
// stand alone (and be unit-tested) before markers exist.
//
// Mirrors commandFailureStore's bounded-reactive-ring shape, but spans ALL
// commands (success + failure), not failures only. Exported as a class +
// singleton (the errorClusterStore pattern) so tests get a fresh instance with
// no cross-test bleed.

export interface CommandBlock {
  /** Stable id for keying, bookmarking, and the jump action. */
  id: string;
  /** Correlation key — the PTY session this command ran in (Phase 4 key). */
  sessionId: number | null;
  command: string;
  cwd: string | null;
  exitCode: number;
  durationMs: number | null;
  /** Snapshot-time buffer rows (non-authoritative — see file header). */
  startRow: number;
  endRow: number;
  /** Epoch ms at completion. */
  ts: number;
  /** User-pinned: kept addressable and never evicted by the ring (N3.3). */
  bookmarked: boolean;
}

/** What a caller passes to record() — everything except the store-owned id
 *  and the (always-false-at-birth) bookmarked flag. */
export type CommandBlockInput = Omit<CommandBlock, 'id' | 'bookmarked'>;

const MAX_BLOCKS = 500;

export class CommandBlockStore {
  #blocks = $state<CommandBlock[]>([]);
  #nextId = 0;

  /** Newest-first list of recorded command blocks. */
  get blocks(): CommandBlock[] {
    return this.#blocks;
  }
  get count(): number {
    return this.#blocks.length;
  }
  /** Bookmarked blocks, newest-first (drives the N3.4 bookmarks list). */
  get bookmarks(): CommandBlock[] {
    return this.#blocks.filter((b) => b.bookmarked);
  }

  byId(id: string): CommandBlock | undefined {
    return this.#blocks.find((b) => b.id === id);
  }
  /** All blocks for one PTY session (the per-pane history). */
  forSession(sessionId: number): CommandBlock[] {
    return this.#blocks.filter((b) => b.sessionId === sessionId);
  }

  /**
   * Record a finished command. Returns the created block (with its assigned id)
   * so the caller can associate per-pane live markers with it (N3.2). Prepends
   * newest-first; the bounded ring then drops the oldest NON-bookmarked blocks
   * back down to the cap — a bookmark the user pinned is never silently evicted.
   */
  record(input: CommandBlockInput): CommandBlock {
    const block: CommandBlock = { ...input, id: `blk-${this.#nextId++}`, bookmarked: false };
    this.#blocks = this.#trim([block, ...this.#blocks]);
    return block;
  }

  /** Toggle a block's bookmark. Returns the new bookmarked state (false if the
   *  id is unknown — e.g. already evicted). */
  toggleBookmark(id: string): boolean {
    let next = false;
    this.#blocks = this.#blocks.map((b) => {
      if (b.id !== id) return b;
      next = !b.bookmarked;
      return { ...b, bookmarked: next };
    });
    return next;
  }

  remove(id: string): void {
    this.#blocks = this.#blocks.filter((b) => b.id !== id);
  }
  clear(): void {
    this.#blocks = [];
  }

  /** Cap the ring at MAX_BLOCKS, dropping the oldest NON-bookmarked blocks
   *  first. Bookmarked blocks are retained even past the cap (the list may
   *  exceed MAX_BLOCKS only by the number of bookmarks — a small, intentional
   *  set). Input is already newest-first. */
  #trim(list: CommandBlock[]): CommandBlock[] {
    if (list.length <= MAX_BLOCKS) return list;
    const kept: CommandBlock[] = [];
    for (const b of list) {
      if (kept.length < MAX_BLOCKS || b.bookmarked) kept.push(b);
      // else: oldest non-bookmarked overflow — dropped.
    }
    return kept;
  }
}

/** Process-wide singleton — the shared block index every surface reads. */
export const commandBlockStore = new CommandBlockStore();
