// errorHandoff.ts — Phase 5 / R0: capture foundation for the error→agent handoff.
//
// Pure assembly of a FailureContext from a captured command + the xterm
// scrollback, with NO agent call, NO UI, and NO bus dependency — so it is
// unit-testable in isolation (the genuinely-isolatable R0 per the revised
// spec §10, unlike the old stub-invoke that would hang `pending` forever).
//
// The original spec assumed a backend command_history lookup; the 4-agent
// red-team falsified that (no session_id on CommandRecord, write-only API), so
// capture lives in the frontend: Terminal.svelte caches the submitted command +
// its start row, then pairs it with the exit code on completion and reads the
// output region straight out of the xterm buffer.

/** Minimal slice of xterm's IBufferLine we depend on (keeps this testable
 *  without pulling in the whole xterm type surface or a real terminal). */
export interface BufferLineLike {
  translateToString(trimRight?: boolean): string;
}

/** Minimal slice of xterm's IBuffer we depend on. */
export interface BufferLike {
  /** Total lines in the buffer (scrollback + viewport). */
  readonly length: number;
  getLine(row: number): BufferLineLike | undefined;
}

/** What Terminal.svelte caches when a command is submitted, to be paired with
 *  its exit code on completion. */
export interface CommandCapture {
  command: string;
  cwd: string | null;
  /** Absolute buffer row of the prompt line at submit time
   *  (`buffer.active.baseY + cursorY`). */
  startRow: number;
  /** Epoch ms at submit time. */
  ts: number;
}

/** The packaged failure context handed to the affordance/provider in R1. */
export interface FailureContext {
  command: string;
  cwd: string | null;
  exitCode: number;
  durationMs: number | null;
  startRow: number;
  endRow: number;
  /** Bounded tail of the command's output region (startRow..endRow). */
  scrollbackTail: string[];
}

/** Bounds for the scrollback tail read. */
export interface TailBounds {
  /** Hard cap on number of lines (most recent kept). Default 200. */
  maxLines?: number;
  /** Hard cap on total bytes (UTF-16 length proxy). Default 16384. */
  maxBytes?: number;
}

const DEFAULT_MAX_LINES = 200;
const DEFAULT_MAX_BYTES = 16 * 1024;

/**
 * Read the output region [startRow, endRow] out of an xterm buffer, bounded by
 * line count and byte budget. Returns the MOST RECENT lines when truncated
 * (the tail nearest the failure is the useful part). Trailing blank lines are
 * dropped; interior blanks are preserved.
 *
 * Robust to a bogus startRow (>= endRow, negative, or NaN): falls back to the
 * last `maxLines` before endRow, matching the spec's documented fallback.
 */
export function readScrollbackTail(
  buffer: BufferLike,
  startRow: number,
  endRow: number,
  bounds: TailBounds = {},
): string[] {
  const maxLines = bounds.maxLines ?? DEFAULT_MAX_LINES;
  const maxBytes = bounds.maxBytes ?? DEFAULT_MAX_BYTES;

  const end = clampRow(endRow, buffer.length);
  let start = clampRow(startRow, buffer.length);
  // Fallback for a missing/bogus start: take the window just above endRow.
  if (!Number.isFinite(startRow) || start >= end) {
    start = Math.max(0, end - maxLines);
  }

  // Collect the region top→bottom, then keep only the most-recent tail.
  const region: string[] = [];
  for (let row = start; row <= end; row++) {
    const line = buffer.getLine(row);
    region.push(line ? line.translateToString(true) : '');
  }

  // Drop trailing blank lines (the badge row / prompt re-draw is noise).
  while (region.length > 0 && region[region.length - 1].trim() === '') {
    region.pop();
  }

  // Keep the most recent `maxLines`.
  let tail = region.length > maxLines ? region.slice(region.length - maxLines) : region;

  // Enforce the byte budget from the bottom up (drop oldest lines first).
  let total = tail.reduce((n, l) => n + l.length + 1, 0);
  while (tail.length > 1 && total > maxBytes) {
    total -= tail[0].length + 1;
    tail = tail.slice(1);
  }

  return tail;
}

/**
 * Pair a cached command with its completion outcome and the buffer to produce a
 * FailureContext. Pure — the caller supplies the buffer + resolved end row.
 */
export function assembleFailureContext(
  capture: CommandCapture,
  outcome: {
    exitCode: number;
    durationMs: number | null;
    endRow: number;
    buffer: BufferLike;
  },
  bounds: TailBounds = {},
): FailureContext {
  return {
    command: capture.command,
    cwd: capture.cwd,
    exitCode: outcome.exitCode,
    durationMs: outcome.durationMs,
    startRow: capture.startRow,
    endRow: outcome.endRow,
    scrollbackTail: readScrollbackTail(outcome.buffer, capture.startRow, outcome.endRow, bounds),
  };
}

/** A compact, log/bus-friendly view of a FailureContext — the full scrollback
 *  tail is reduced to a line count + a short preview so the verification
 *  envelope (and session .jsonl) stays small. */
export function summarizeFailureContext(ctx: FailureContext): {
  command: string;
  cwd: string | null;
  exitCode: number;
  durationMs: number | null;
  startRow: number;
  endRow: number;
  tailLineCount: number;
  tailPreview: string[];
} {
  return {
    command: ctx.command,
    cwd: ctx.cwd,
    exitCode: ctx.exitCode,
    durationMs: ctx.durationMs,
    startRow: ctx.startRow,
    endRow: ctx.endRow,
    tailLineCount: ctx.scrollbackTail.length,
    tailPreview: ctx.scrollbackTail.slice(-5),
  };
}

function clampRow(row: number, length: number): number {
  if (!Number.isFinite(row)) return Math.max(0, length - 1);
  if (row < 0) return 0;
  if (row > length - 1) return Math.max(0, length - 1);
  return Math.floor(row);
}
