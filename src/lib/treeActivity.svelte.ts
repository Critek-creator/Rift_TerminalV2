// treeActivity.svelte.ts — Phase 6.2
//
// Runes-backed activity store that tracks per-path visual state for the
// filesystem tree (and, later, the Abyssal Index graph in Phase 8).
//
// Every path that the live watcher touches gets an `ActivityEntry` that
// carries a `state` (ambient | recent | active | background) and a
// `glowIntensity` (0–1) that decays to 0 over `DECAY_MS` milliseconds.
//
// A single `$effect.root` drives one `requestAnimationFrame` loop for the
// decay; no timers or setInterval calls are used.
//
// Phase 6.4 unblock: when detached cockpit window arrives, this store is
// shared via Tauri events instead of in-process module state.

/** Decay duration in milliseconds for `recent` entries. */
const DECAY_MS = 8_000;

/** Visual state of a tree node. */
export type ActivityState = 'ambient' | 'recent' | 'active' | 'background';

/** Per-path entry held inside the store. */
export interface ActivityEntry {
  state: ActivityState;
  /** 0.0–1.0. Full glow on `mark()`; decays toward 0 for `recent` entries.
   *  Pinned (`active`) entries stay at 1.0. Background entries are at 0. */
  glowIntensity: number;
  /** `Date.now()` at the last `mark()` call. */
  lastTouchMs: number;
}

/** Default entry returned for paths not yet seen. */
const AMBIENT_DEFAULT: ActivityEntry = {
  state: 'ambient',
  glowIntensity: 0,
  lastTouchMs: 0,
};

// ---------------------------------------------------------------------------
// Internal rune state — `.svelte.ts` extension is required for $state.
// ---------------------------------------------------------------------------

let entries = $state(new Map<string, ActivityEntry>());

/**
 * Per-path array of touch timestamps (ms epoch). Used by the D-020 temporal
 * activity heatmap to track HOW OFTEN a file gets touched over a sliding
 * window, as opposed to the glow which tracks WHETHER it was just touched.
 */
let heatLog = $state(new Map<string, number[]>());

// ---------------------------------------------------------------------------
// Decay loop — rAF loop that only runs while entries are decaying.
// ---------------------------------------------------------------------------

let lastFrameTs = 0;
let animating = false;
let rafId: number | null = null;

function startDecayLoop(): void {
  if (animating) return;
  animating = true;
  lastFrameTs = 0;
  rafId = requestAnimationFrame(loop);
}

function stopDecayLoop(): void {
  if (!animating) return;
  animating = false;
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
}

function decayTick(nowMs: number): void {
  if (lastFrameTs === 0) {
    lastFrameTs = nowMs;
    return;
  }
  const deltaMs = nowMs - lastFrameTs;
  lastFrameTs = nowMs;

  let mutated = false;
  const next = new Map(entries);
  let stillDecaying = false;

  for (const [path, entry] of next) {
    if (entry.state !== 'recent') continue;

    const newIntensity = Math.max(0, entry.glowIntensity - deltaMs / DECAY_MS);
    if (newIntensity === entry.glowIntensity) continue;

    mutated = true;
    if (newIntensity <= 0) {
      next.set(path, { ...entry, state: 'ambient', glowIntensity: 0 });
    } else {
      next.set(path, { ...entry, glowIntensity: newIntensity });
      stillDecaying = true;
    }
  }

  if (mutated) {
    entries = next;
  }

  if (!stillDecaying) {
    stopDecayLoop();
  }
}

function loop(ts: number): void {
  decayTick(ts);
  if (animating) {
    rafId = requestAnimationFrame(loop);
  }
}

// Module-level singleton effect — cleanup on module unload.
$effect.root(() => {
  return () => {
    stopDecayLoop();
  };
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/** Mark a path as recently touched by a filesystem event. */
function mark(path: string, _kind: 'create' | 'write' | 'delete' | 'rename'): void {
  const existing = entries.get(path);
  // Pinned (`active`) nodes are not downgraded to `recent` by live events.
  if (existing?.state === 'active') return;

  entries = new Map(entries).set(path, {
    state: 'recent',
    glowIntensity: 1.0,
    lastTouchMs: Date.now(),
  });

  // D-020 heatmap — record the touch timestamp.
  const now = Date.now();
  const stamps = heatLog.get(path) ?? [];
  stamps.push(now);
  // Memory guard: cap at 500 timestamps per path.
  if (stamps.length > 500) stamps.shift();
  heatLog = new Map(heatLog).set(path, stamps);

  startDecayLoop();
}

/**
 * Cycle the state of a path on user click:
 * `ambient` / `recent` → `active` → `background` → `ambient`.
 */
function cycle(path: string): void {
  const existing = entries.get(path) ?? AMBIENT_DEFAULT;
  let next: ActivityEntry;

  switch (existing.state) {
    case 'ambient':
    case 'recent':
      next = { state: 'active', glowIntensity: 1.0, lastTouchMs: Date.now() };
      break;
    case 'active':
      next = { state: 'background', glowIntensity: 0, lastTouchMs: Date.now() };
      break;
    case 'background':
      next = { state: 'ambient', glowIntensity: 0, lastTouchMs: Date.now() };
      break;
    default: {
      // Exhaustive guard — unreachable with current ActivityState values.
      const _exhaustive: never = existing.state;
      next = { state: 'ambient', glowIntensity: 0, lastTouchMs: Date.now() };
      void _exhaustive;
    }
  }

  entries = new Map(entries).set(path, next);
}

/** Return the activity entry for `path`, or the ambient default if unseen. */
function getEntry(path: string): ActivityEntry {
  return entries.get(path) ?? AMBIENT_DEFAULT;
}

/**
 * Reset all activity entries. Called by Tree.svelte on project.changed so
 * the new project starts with a clean slate (no stale glow from the prior
 * project). Assign-replace for Svelte 5 reactivity.
 *
 * Future consumers (Phase 8 graph) can also call this to reset view state
 * without re-mounting the component.
 */
function clear(): void {
  entries = new Map();
  heatLog = new Map();
}

/**
 * Count how many touch events for `path` fall within the last `windowMs`
 * milliseconds. Returns the raw count — normalization (0–1 mapping) happens
 * at render time in Tree.svelte.
 */
function getHeat(path: string, windowMs: number): number {
  const stamps = heatLog.get(path);
  if (!stamps || stamps.length === 0) return 0;
  const cutoff = Date.now() - windowMs;
  let count = 0;
  for (let i = stamps.length - 1; i >= 0; i--) {
    if (stamps[i] >= cutoff) count++;
    else break; // timestamps are insertion-ordered (oldest first)
  }
  return count;
}

/**
 * Build a snapshot of heat counts across all tracked paths for a given
 * window. Pure read — does NOT mutate heatLog (would violate Svelte 5's
 * no-mutation-inside-$derived rule). Pruning is deferred via microtask.
 *
 * Returns a new Map of path → count for entries with count > 0.
 */
function heatSnapshot(windowMs: number): Map<string, number> {
  const cutoff = Date.now() - windowMs;
  const result = new Map<string, number>();
  let needsPrune = false;

  for (const [path, stamps] of heatLog) {
    let firstValid = -1;
    for (let i = 0; i < stamps.length; i++) {
      if (stamps[i] >= cutoff) {
        firstValid = i;
        break;
      }
    }
    if (firstValid === -1) {
      needsPrune = true;
      continue;
    }
    if (firstValid > 0) needsPrune = true;
    const count = stamps.length - (firstValid > 0 ? firstValid : 0);
    result.set(path, count);
  }

  if (needsPrune) {
    queueMicrotask(() => pruneHeatLog(windowMs));
  }
  return result;
}

function pruneHeatLog(windowMs: number): void {
  const cutoff = Date.now() - windowMs;
  const pruned = new Map<string, number[]>();
  for (const [path, stamps] of heatLog) {
    let firstValid = -1;
    for (let i = 0; i < stamps.length; i++) {
      if (stamps[i] >= cutoff) { firstValid = i; break; }
    }
    if (firstValid === -1) continue;
    pruned.set(path, firstValid === 0 ? stamps : stamps.slice(firstValid));
  }
  heatLog = pruned;
}

/**
 * Dismiss the glow for `path` — set state to 'background' regardless of
 * current state. Used by user-click acknowledgement: "I've seen this
 * AI/agent activity, stop drawing my attention to it." Activity glow is
 * RESERVED for bus-driven AI/agent file-access events (Category::Fs from
 * translators) — the user is the OBSERVER of that activity, not a
 * participant. Clicking marks the file as seen and the glow goes away;
 * unclicked entries decay naturally per the existing decay loop.
 *
 * Distinct from `cycle` (which advances through pin states — still
 * exported for any future shift-click "pin to keep visible" gesture)
 * and from `clear` (which wipes all entries on project swap).
 *
 * No-op if the path has no entry, or is already in a no-glow state
 * (`ambient` or `background`) — idempotent. Activity envelopes from
 * the bus can re-promote the file to 'recent' later if AI accesses
 * it again; dismissal isn't permanent.
 */
function dismiss(path: string): void {
  const existing = entries.get(path);
  if (!existing) return;
  if (existing.state === 'background' || existing.state === 'ambient') return;
  entries = new Map(entries).set(path, {
    state: 'background',
    glowIntensity: 0,
    lastTouchMs: Date.now(),
  });
}

export const treeActivity = {
  mark,
  cycle,
  dismiss,
  getEntry,
  getHeat,
  heatSnapshot,
  clear,
  /** Reactive snapshot of all tracked entries. Consumers bind `$derived`
   *  on this for reactivity. */
  get snapshot(): Map<string, ActivityEntry> {
    return entries;
  },
  /** Reactive reference to the raw heat log. Consumers bind `$derived`
   *  on this for reactivity. */
  get heatLog(): Map<string, number[]> {
    return heatLog;
  },
};
