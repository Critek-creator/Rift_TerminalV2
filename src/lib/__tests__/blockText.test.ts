import { describe, it, expect } from 'vitest';
import { readBufferRange } from '../blockText';
import type { BufferLike } from '../errorHandoff';

/** Fake xterm buffer from an array of lines (mirrors errorHandoff.test.ts). */
function fakeBuffer(lines: string[]): BufferLike {
  return {
    length: lines.length,
    getLine(row: number) {
      const text = lines[row];
      if (text === undefined) return undefined;
      return { translateToString: () => text };
    },
  };
}

describe('readBufferRange', () => {
  it('reads the inclusive region [startRow, endRow]', () => {
    const buf = fakeBuffer(['a', 'b', 'c', 'd', 'e']);
    expect(readBufferRange(buf, 1, 3)).toBe('b\nc\nd');
  });

  it('drops trailing blank lines but keeps interior blanks', () => {
    const buf = fakeBuffer(['x', '', 'y', '', '']);
    expect(readBufferRange(buf, 0, 4)).toBe('x\n\ny');
  });

  it('normalizes a swapped row pair', () => {
    const buf = fakeBuffer(['a', 'b', 'c']);
    expect(readBufferRange(buf, 2, 0)).toBe('a\nb\nc');
  });

  it('clamps an out-of-range endRow to the last buffer row', () => {
    const buf = fakeBuffer(['only', 'two']);
    expect(readBufferRange(buf, 0, 999)).toBe('only\ntwo');
  });

  it('clamps a negative startRow to 0', () => {
    const buf = fakeBuffer(['a', 'b']);
    expect(readBufferRange(buf, -5, 1)).toBe('a\nb');
  });

  it('tolerates NaN rows without throwing', () => {
    const buf = fakeBuffer(['a', 'b']);
    expect(() => readBufferRange(buf, NaN, NaN)).not.toThrow();
  });

  it('returns empty string for an empty buffer', () => {
    expect(readBufferRange(fakeBuffer([]), 0, 3)).toBe('');
  });

  it('returns empty string for an all-blank region', () => {
    const buf = fakeBuffer(['', '  ', '']);
    expect(readBufferRange(buf, 0, 2)).toBe('');
  });

  it('reads a single-row region', () => {
    const buf = fakeBuffer(['$ npm run build', 'output', '✓ 0']);
    expect(readBufferRange(buf, 0, 0)).toBe('$ npm run build');
  });
});
