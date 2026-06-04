import { describe, it, expect } from 'vitest';
import {
  readScrollbackTail,
  assembleFailureContext,
  summarizeFailureContext,
  errorActionId,
  isErrorExplainAction,
  failureClusterKey,
  buildExplainPrompt,
  ERROR_EXPLAIN_ACTION,
  type BufferLike,
  type CommandCapture,
  type FailureContext,
} from '../errorHandoff';

/** Build a fake xterm buffer from an array of lines. */
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

describe('readScrollbackTail', () => {
  it('reads the inclusive region [startRow, endRow]', () => {
    const buf = fakeBuffer(['a', 'b', 'c', 'd', 'e']);
    expect(readScrollbackTail(buf, 1, 3)).toEqual(['b', 'c', 'd']);
  });

  it('drops trailing blank lines but keeps interior blanks', () => {
    const buf = fakeBuffer(['x', '', 'y', '', '']);
    expect(readScrollbackTail(buf, 0, 4)).toEqual(['x', '', 'y']);
  });

  it('keeps the most recent lines when over the line cap', () => {
    const buf = fakeBuffer(['1', '2', '3', '4', '5']);
    expect(readScrollbackTail(buf, 0, 4, { maxLines: 2 })).toEqual(['4', '5']);
  });

  it('enforces the byte budget by dropping oldest lines first', () => {
    const buf = fakeBuffer(['aaaa', 'bbbb', 'cccc']);
    // budget 6 bytes: 'cccc'(4)+1=5 fits; adding 'bbbb' would be 10 > 6.
    expect(readScrollbackTail(buf, 0, 2, { maxBytes: 6 })).toEqual(['cccc']);
  });

  it('never returns fewer than one line even under a tiny byte budget', () => {
    const buf = fakeBuffer(['enormous-single-line-of-output']);
    expect(readScrollbackTail(buf, 0, 0, { maxBytes: 1 })).toEqual([
      'enormous-single-line-of-output',
    ]);
  });

  it('falls back to the last maxLines before endRow when startRow is bogus', () => {
    const buf = fakeBuffer(['1', '2', '3', '4', '5', '6']);
    // startRow past endRow → fallback window above endRow, then the line cap
    // trims it to the most-recent maxLines.
    expect(readScrollbackTail(buf, 99, 5, { maxLines: 3 })).toEqual(['4', '5', '6']);
  });

  it('clamps an out-of-range endRow to the last buffer row', () => {
    const buf = fakeBuffer(['only', 'two']);
    expect(readScrollbackTail(buf, 0, 999)).toEqual(['only', 'two']);
  });

  it('handles NaN rows without throwing', () => {
    const buf = fakeBuffer(['a', 'b']);
    expect(() => readScrollbackTail(buf, NaN, NaN)).not.toThrow();
  });
});

describe('assembleFailureContext', () => {
  const capture: CommandCapture = {
    command: 'npm run build',
    cwd: '/home/g/proj',
    startRow: 2,
    ts: 1_000,
  };

  it('pairs the capture with the outcome and reads the tail', () => {
    const buffer = fakeBuffer(['p0', 'p1', '$ npm run build', 'err line 1', 'err line 2', '✗ 1']);
    const ctx = assembleFailureContext(capture, {
      exitCode: 1,
      durationMs: 420,
      endRow: 4,
      buffer,
    });
    expect(ctx).toMatchObject({
      command: 'npm run build',
      cwd: '/home/g/proj',
      exitCode: 1,
      durationMs: 420,
      startRow: 2,
      endRow: 4,
    });
    expect(ctx.scrollbackTail).toEqual(['$ npm run build', 'err line 1', 'err line 2']);
  });

  it('tolerates a null cwd and null duration', () => {
    const buffer = fakeBuffer(['$ false', '✗ 1']);
    const ctx = assembleFailureContext(
      { command: 'false', cwd: null, startRow: 0, ts: 0 },
      { exitCode: 1, durationMs: null, endRow: 1, buffer },
    );
    expect(ctx.cwd).toBeNull();
    expect(ctx.durationMs).toBeNull();
  });
});

describe('action-id helpers (B1 collision fix)', () => {
  it('builds per-failure unique ids namespaced by pane + seq', () => {
    expect(errorActionId(ERROR_EXPLAIN_ACTION, 3, 7)).toBe('rift.error.explain::3::7');
    // different pane or seq → different id (no registry-state collision)
    expect(errorActionId(ERROR_EXPLAIN_ACTION, 3, 7)).not.toBe(
      errorActionId(ERROR_EXPLAIN_ACTION, 3, 8),
    );
  });

  it('recognizes explain action ids and rejects others', () => {
    expect(isErrorExplainAction('rift.error.explain::1::0')).toBe(true);
    expect(isErrorExplainAction('rift.error.fix::1::0')).toBe(false);
    expect(isErrorExplainAction('rift.llm.reset-ledger')).toBe(false);
    expect(isErrorExplainAction('rift.error.explain')).toBe(false); // bare base, no ::
  });

  it('clusters identical consecutive failures by command + exit', () => {
    expect(failureClusterKey('npm test', 1)).toBe(failureClusterKey('  npm test ', 1));
    expect(failureClusterKey('npm test', 1)).not.toBe(failureClusterKey('npm test', 2));
    expect(failureClusterKey('npm test', 1)).not.toBe(failureClusterKey('npm run', 1));
  });
});

describe('buildExplainPrompt', () => {
  const base: FailureContext = {
    command: 'cargo build',
    cwd: '/w/proj',
    exitCode: 101,
    durationMs: 1200,
    startRow: 0,
    endRow: 3,
    scrollbackTail: ['error[E0382]: borrow of moved value', '  --> src/main.rs:4:5'],
  };

  it('includes command, exit code, cwd, and the output tail', () => {
    const prompt = buildExplainPrompt(base);
    expect(prompt).toContain('cargo build');
    expect(prompt).toContain('101');
    expect(prompt).toContain('/w/proj');
    expect(prompt).toContain('E0382');
  });

  it('omits the cwd line when cwd is null and notes missing output', () => {
    const prompt = buildExplainPrompt({ ...base, cwd: null, scrollbackTail: [] });
    expect(prompt).not.toContain('Working dir:');
    expect(prompt).toContain('(no captured output)');
  });
});

describe('summarizeFailureContext', () => {
  it('reduces the tail to a count + last-5 preview for compact logging', () => {
    const buffer = fakeBuffer(Array.from({ length: 30 }, (_, i) => `line ${i}`));
    const ctx = assembleFailureContext(
      { command: 'x', cwd: null, startRow: 0, ts: 0 },
      { exitCode: 2, durationMs: 5, endRow: 29, buffer },
    );
    const sum = summarizeFailureContext(ctx);
    expect(sum.tailLineCount).toBe(30);
    expect(sum.tailPreview).toHaveLength(5);
    expect(sum.tailPreview[4]).toBe('line 29');
    expect(sum.exitCode).toBe(2);
  });
});
