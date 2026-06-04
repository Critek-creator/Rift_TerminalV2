// blockText.ts — N3.3: programmatic range-read of a command block's region.
//
// Closes gap G5 (the scout's map): xterm only exposes the user's visual
// selection via getSelection(); there was no way to copy a block's rows by
// boundary. This pure helper reads an inclusive [startRow, endRow] region out
// of an xterm buffer into text, so "Copy command output" / "Copy block" can
// lift exactly the block's lines regardless of the current selection.
//
// Pure + buffer-agnostic (takes the same BufferLike shape readScrollbackTail
// uses), so it's unit-testable without a real terminal. Live row resolution
// (turning a block's start/end markers into row numbers) stays in
// Terminal.svelte, which owns the markers.

import type { BufferLike } from './errorHandoff';

function clampRow(row: number, length: number): number {
  if (!Number.isFinite(row)) return 0;
  if (row < 0) return 0;
  if (row > length - 1) return Math.max(0, length - 1);
  return Math.floor(row);
}

/**
 * Read the inclusive row region [startRow, endRow] out of an xterm buffer as
 * text (one line per row, '\n'-joined). Rows are clamped into range and the
 * order is normalized, so a swapped or out-of-bounds pair never throws. Trailing
 * blank lines are dropped (the trailing prompt re-draw is noise); interior
 * blanks are preserved. Returns '' for an empty buffer or an all-blank region.
 */
export function readBufferRange(buffer: BufferLike, startRow: number, endRow: number): string {
  if (buffer.length <= 0) return '';
  let a = clampRow(startRow, buffer.length);
  let b = clampRow(endRow, buffer.length);
  if (a > b) [a, b] = [b, a];

  const rows: string[] = [];
  for (let row = a; row <= b; row++) {
    const line = buffer.getLine(row);
    rows.push(line ? line.translateToString(true) : '');
  }
  while (rows.length > 0 && rows[rows.length - 1].trim() === '') {
    rows.pop();
  }
  return rows.join('\n');
}
