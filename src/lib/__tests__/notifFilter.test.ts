import { describe, it, expect } from 'vitest';
import { kindToSeverity, shouldShow, parseSeverity } from '../notifFilter';

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
