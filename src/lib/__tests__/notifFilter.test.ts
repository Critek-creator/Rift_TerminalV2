import { describe, it, expect } from 'vitest';
import { kindToSeverity, shouldShow, parseSeverity, floorAtWarn, resolveThreshold } from '../notifFilter';

// Unit tests for notifFilter.ts — severity derivation from kind strings,
// threshold gating, and parseSeverity edge cases.

describe('kindToSeverity', () => {
  it('maps error-containing kinds to error', () => {
    expect(kindToSeverity('fs.error')).toBe('error');
    expect(kindToSeverity('CompilationError')).toBe('error');
    expect(kindToSeverity('ERROR_LOUD')).toBe('error');
  });

  it('maps "failed" and "panic" to error', () => {
    expect(kindToSeverity('build.failed')).toBe('error');
    expect(kindToSeverity('thread.panic')).toBe('error');
  });

  it('maps warn-containing kinds to warn', () => {
    expect(kindToSeverity('disk.warn')).toBe('warn');
    expect(kindToSeverity('DeprecationWarning')).toBe('warn');
  });

  it('maps debug/trace-containing kinds to debug', () => {
    expect(kindToSeverity('debug.verbose')).toBe('debug');
    expect(kindToSeverity('hook.trace')).toBe('debug');
  });

  it('defaults to info for unrecognized kinds', () => {
    expect(kindToSeverity('heartbeat')).toBe('info');
    expect(kindToSeverity('fs.write')).toBe('info');
    expect(kindToSeverity('')).toBe('info');
  });

  it('is case-insensitive', () => {
    expect(kindToSeverity('FATAL_ERROR')).toBe('error');
    expect(kindToSeverity('Warning')).toBe('warn');
    expect(kindToSeverity('DEBUG')).toBe('debug');
  });

  // notif-filter audit 2026-05-31: domain-noun kinds emitted by real
  // translators that previously fell through to `info` and got hidden at
  // raised thresholds.
  it('classifies real crash/fatal kinds as error', () => {
    expect(kindToSeverity('llm.process.crash')).toBe('error');
    expect(kindToSeverity('worker.fatal')).toBe('error');
  });

  it('classifies violation/deny kinds as warn', () => {
    expect(kindToSeverity('sentinel.violation')).toBe('warn');
    expect(kindToSeverity('mcp.handshake.deny')).toBe('warn');
    expect(kindToSeverity('access.denied')).toBe('warn');
  });

  it('leaves genuinely-info kinds unaffected by the new matches', () => {
    expect(kindToSeverity('command.submitted')).toBe('info');
    expect(kindToSeverity('fs.create')).toBe('info');
    expect(kindToSeverity('health.portfolio')).toBe('info');
    expect(kindToSeverity('llm.process.start')).toBe('info');
  });
});

describe('shouldShow', () => {
  it('shows events at or above threshold', () => {
    expect(shouldShow('error.crash', 'warn')).toBe(true);
    expect(shouldShow('warn.disk', 'warn')).toBe(true);
    expect(shouldShow('error.crash', 'error')).toBe(true);
  });

  it('hides events below threshold', () => {
    expect(shouldShow('info.heartbeat', 'warn')).toBe(false);
    expect(shouldShow('debug.trace', 'info')).toBe(false);
    expect(shouldShow('info.tick', 'error')).toBe(false);
  });

  it('shows everything at debug threshold', () => {
    expect(shouldShow('debug.trace', 'debug')).toBe(true);
    expect(shouldShow('info.heartbeat', 'debug')).toBe(true);
    expect(shouldShow('error.crash', 'debug')).toBe(true);
  });

  it('shows only errors at error threshold', () => {
    expect(shouldShow('warn.disk', 'error')).toBe(false);
    expect(shouldShow('info.heartbeat', 'error')).toBe(false);
    expect(shouldShow('error.crash', 'error')).toBe(true);
  });

  // Regression guard for the audit fix: these kinds must survive a raised
  // threshold instead of being silently filtered out as `info`.
  it('keeps crash/violation/deny visible at raised thresholds', () => {
    expect(shouldShow('llm.process.crash', 'error')).toBe(true);
    expect(shouldShow('sentinel.violation', 'warn')).toBe(true);
    expect(shouldShow('mcp.handshake.deny', 'warn')).toBe(true);
    // violation is warn-tier, so an error-only threshold still suppresses it.
    expect(shouldShow('sentinel.violation', 'error')).toBe(false);
  });
});

describe('floorAtWarn', () => {
  it('raises debug/info up to warn', () => {
    expect(floorAtWarn('debug')).toBe('warn');
    expect(floorAtWarn('info')).toBe('warn');
  });

  it('leaves warn/error unchanged', () => {
    expect(floorAtWarn('warn')).toBe('warn');
    expect(floorAtWarn('error')).toBe('error');
  });

  it('keeps system noise out of an errors tab floored at warn', () => {
    // health.portfolio / project.changed are info → suppressed at the warn floor
    expect(shouldShow('health.portfolio', floorAtWarn('info'))).toBe(false);
    expect(shouldShow('project.changed', floorAtWarn('info'))).toBe(false);
    // genuine system errors still pass
    expect(shouldShow('error', floorAtWarn('info'))).toBe(true);
  });
});

describe('resolveThreshold', () => {
  it('respects an explicit per-tab override for normal tabs', () => {
    expect(resolveThreshold('agents', 'info', { agents: 'error' })).toBe('error');
    expect(resolveThreshold('mcp', 'warn', { mcp: 'debug' })).toBe('debug');
  });

  it('floors the errors tab at warn (default and per-tab)', () => {
    expect(resolveThreshold('errors', 'info', {})).toBe('warn');
    expect(resolveThreshold('errors', 'debug', { errors: 'debug' })).toBe('warn');
    expect(resolveThreshold('errors', 'info', { errors: 'error' })).toBe('error');
  });

  it('floors bustail at debug (firehose)', () => {
    expect(resolveThreshold('bustail', 'error', {})).toBe('debug');
  });

  it('falls back to the default threshold when no per-tab entry', () => {
    expect(resolveThreshold('agents', 'info', {})).toBe('info');
    expect(resolveThreshold('filesystem', 'warn', {})).toBe('warn');
  });
});

describe('parseSeverity', () => {
  it('returns valid severity levels as-is', () => {
    expect(parseSeverity('debug')).toBe('debug');
    expect(parseSeverity('info')).toBe('info');
    expect(parseSeverity('warn')).toBe('warn');
    expect(parseSeverity('error')).toBe('error');
  });

  it('defaults to info for undefined/null', () => {
    expect(parseSeverity(undefined)).toBe('info');
    expect(parseSeverity(null)).toBe('info');
  });

  it('defaults to info for invalid strings', () => {
    expect(parseSeverity('fatal')).toBe('info');
    expect(parseSeverity('WARNING')).toBe('info');
    expect(parseSeverity('')).toBe('info');
  });
});
