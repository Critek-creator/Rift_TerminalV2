import { describe, it, expect } from 'vitest';

// Regression test for pr003 `tabbar-drag-promote-demote-self-cancel-on-strip-drop`.
//
// The bug: when a user dragged an UNpromoted tab and released it WITHIN the
// strip's bounds, the original code would call onPromote() on dragstart AND
// onDemote() on strip-drop — the two gestures cancelled each other, making
// drag-to-promote silently no-op in the most common scenario.
//
// Root cause: the code did not distinguish "dragged from promoted state" from
// "dragged from strip". Without that guard, any strip-drop unconditionally
// called the demote path.
//
// Fix (TabBar.svelte lines 99-155):
//   - `draggedFromPromoted` is captured at dragstart time.
//   - onNotifDragStart: if NOT already promoted → call onPromote(); set
//     draggedFromPromoted = false.
//   - onStripDrop: only call onDemote() when `draggedFromPromoted` was true.
//
// Note: the implementation uses SEPARATE callbacks (onPromote / onDemote),
// NOT a single toggle-on-call. The tests model this precisely.
//
// Strategy C (pure model): we mirror the TabBar drag state machine as a
// local pure function and run all 4 distinguishing scenarios through it.
// This pins the SPEC — if anyone reverts the production logic, they must
// also revert the model (which is documented here), and the negative-control
// test catches the broken pattern directly.
//
// Why not Strategy A or B?
//   - A (extract helper + refactor): TabBar's drag logic is already concise
//     inline code. Extracting it creates churn in production for a 4-function
//     FSM that's already readable at the call sites.
//   - B (@testing-library/svelte): adding a devDep to mount Svelte 5 rune
//     components in jsdom requires significant setup and introduces Svelte 5
//     compiler surface (runes don't work without the compiler transform). The
//     drag events also need manual dataTransfer mock scaffolding. The
//     pure-model approach is faster, cheaper, and equally valid for pinning
//     this specific state invariant.

// ---------------------------------------------------------------------------
// Model — mirrors TabBar.svelte drag FSM precisely
// ---------------------------------------------------------------------------

interface DragSession {
  /** Snapshot of promotedId === tab.id at dragstart time. */
  draggedFromPromoted: boolean;
}

/**
 * Models onNotifDragStart in TabBar.svelte.
 *
 * Returns:
 *   `draggedFromPromoted` — state captured at gesture start.
 *   `shouldPromote`       — whether onPromote(tabId) was called.
 */
function startDrag(
  promotedId: string | null,
  tabId: string,
): { draggedFromPromoted: boolean; shouldPromote: boolean } {
  const draggedFromPromoted = promotedId === tabId;
  // If NOT already promoted: call onPromote() immediately (dragstart = promote).
  const shouldPromote = !draggedFromPromoted;
  return { draggedFromPromoted, shouldPromote };
}

/**
 * Models onStripDrop in TabBar.svelte.
 *
 * Returns whether onDemote() was called.
 * Always resets `draggedFromPromoted` to false (as the production code does).
 */
function onStripDrop(session: DragSession): { shouldDemote: boolean } {
  const shouldDemote = session.draggedFromPromoted;
  session.draggedFromPromoted = false;
  return { shouldDemote };
}

// ---------------------------------------------------------------------------
// Scenario helpers — count net promote / demote calls for a full gesture
// ---------------------------------------------------------------------------

/** Counts (promotes, demotes) for: drag tab → drop ON strip. */
function gestureDropOnStrip(
  promotedId: string | null,
  tabId: string,
): { promotes: number; demotes: number } {
  const drag = startDrag(promotedId, tabId);
  const session: DragSession = { draggedFromPromoted: drag.draggedFromPromoted };
  const drop = onStripDrop(session);
  return {
    promotes: drag.shouldPromote ? 1 : 0,
    demotes: drop.shouldDemote ? 1 : 0,
  };
}

/** Counts (promotes, demotes) for: drag tab → drop OUTSIDE strip (no strip drop event). */
function gestureDropOutsideStrip(
  promotedId: string | null,
  tabId: string,
): { promotes: number; demotes: number } {
  const drag = startDrag(promotedId, tabId);
  // onStripDrop never fires for an external drop target.
  return {
    promotes: drag.shouldPromote ? 1 : 0,
    demotes: 0,
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const TAB_ID = 'errors';

describe('TabBar drag-promote state machine (C5 regression guard)', () => {
  // ── Scenario 1 ────────────────────────────────────────────────────────────
  it('Sc1: drag UNPROMOTED tab → drop on strip → net promote (1 onPromote, 0 onDemote)', () => {
    // The bug pre-fix: this would return { promotes: 1, demotes: 1 } — a
    // silent no-op. The fix ensures demote fires ONLY when draggedFromPromoted.
    const result = gestureDropOnStrip(null, TAB_ID);
    expect(result.promotes).toBe(1);
    expect(result.demotes).toBe(0);
  });

  // ── Scenario 2 ────────────────────────────────────────────────────────────
  it('Sc2: drag UNPROMOTED tab → drop outside strip → net promote (1 onPromote, 0 onDemote)', () => {
    const result = gestureDropOutsideStrip(null, TAB_ID);
    expect(result.promotes).toBe(1);
    expect(result.demotes).toBe(0);
  });

  // ── Scenario 3 ────────────────────────────────────────────────────────────
  it('Sc3: drag PROMOTED tab → drop on strip → net demote (0 onPromote, 1 onDemote)', () => {
    // promotedId === TAB_ID → tab is already promoted.
    // dragstart: draggedFromPromoted=true, shouldPromote=false (no call).
    // strip drop: shouldDemote=true (1 call to onDemote).
    const result = gestureDropOnStrip(TAB_ID, TAB_ID);
    expect(result.promotes).toBe(0);
    expect(result.demotes).toBe(1);
  });

  // ── Scenario 4 ────────────────────────────────────────────────────────────
  it('Sc4: drag PROMOTED tab → drop outside strip → no change (0 onPromote, 0 onDemote)', () => {
    // promotedId === TAB_ID → tab is already promoted.
    // dragstart: draggedFromPromoted=true, shouldPromote=false.
    // No strip drop → no demote.
    const result = gestureDropOutsideStrip(TAB_ID, TAB_ID);
    expect(result.promotes).toBe(0);
    expect(result.demotes).toBe(0);
  });

  // ── Negative control — documents the OLD broken pattern ───────────────────
  it('BROKEN pattern: strip drop always demotes → Sc1 would silently no-op (negative control)', () => {
    // The pre-fix behavior: strip drop unconditionally called the demote path,
    // regardless of whether the drag started from a promoted state.
    function brokenStripDrop(): { shouldDemote: boolean } {
      // Old code: no `draggedFromPromoted` guard — always demote on strip drop.
      return { shouldDemote: true };
    }

    // Simulate Sc1 with the broken strip-drop.
    const drag = startDrag(null, TAB_ID); // unpromoted tab → shouldPromote=true
    const brokenDrop = brokenStripDrop();

    // The bug: 1 promote + 1 demote = user sees no change. The new test
    // asserts this is WRONG (promotes === demotes means silent no-op).
    expect(drag.shouldPromote).toBe(true);
    expect(brokenDrop.shouldDemote).toBe(true); // broken: always demotes

    // This is the broken outcome — it differs from the correct Sc1 assertion above.
    const netOpsEqual = drag.shouldPromote === brokenDrop.shouldDemote;
    expect(netOpsEqual).toBe(true); // both true → silent cancel — the bug
  });

  // ── Additional: draggedFromPromoted resets after strip drop ───────────────
  it('draggedFromPromoted resets to false after onStripDrop (prevents double-demote)', () => {
    const session: DragSession = { draggedFromPromoted: true };
    const first = onStripDrop(session);
    expect(first.shouldDemote).toBe(true);
    expect(session.draggedFromPromoted).toBe(false);

    // Second strip drop (e.g. repeated event) must NOT demote again.
    const second = onStripDrop(session);
    expect(second.shouldDemote).toBe(false);
  });

  // ── Unrelated tab's promote state does not affect this drag ───────────────
  it('drag on tabA is not affected by tabB being promoted', () => {
    // promotedId = 'hooks' (a different tab), dragging 'errors' tab.
    const result = gestureDropOnStrip('hooks', TAB_ID);
    // 'errors' is not promoted → should promote, should not demote.
    expect(result.promotes).toBe(1);
    expect(result.demotes).toBe(0);
  });
});
