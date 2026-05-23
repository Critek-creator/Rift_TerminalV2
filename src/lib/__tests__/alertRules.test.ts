import { describe, it, expect } from 'vitest';
import { evaluateRule, triggerAction, newAlertRuleId } from '../alertRules';
import { SparklineBuffer } from '../SparklineBuffer';
import type { AlertRule } from '../riftConfig';

// Unit tests for alertRules.ts — severity comparison, threshold evaluation,
// and triggerAction dispatch. playAlertTone is side-effectful (Web Audio)
// and is not directly tested here.

function makeRule(overrides: Partial<AlertRule> = {}): AlertRule {
  return {
    id: 'test-rule',
    tab_id: 'errors',
    severity: 'warn',
    threshold: 3,
    window_secs: 5,
    action: 'flash',
    enabled: true,
    ...overrides,
  };
}

function makeBuffer(values: number[]): SparklineBuffer {
  const buf = new SparklineBuffer();
  // Fill the buffer: tick through all 60 slots, recording into the last
  // `values.length` slots.
  for (let i = 0; i < 60; i++) buf.tick();
  // Now ptr is back at slot 0 after 60 ticks. Tick through and record
  // values into the trailing slots.
  for (const v of values) {
    buf.tick();
    for (let j = 0; j < v; j++) buf.record();
  }
  return buf;
}

describe('evaluateRule', () => {
  it('returns false when rule is disabled', () => {
    const rule = makeRule({ enabled: false });
    const buf = makeBuffer([10, 10, 10]);
    expect(evaluateRule(rule, buf, 'error.crash')).toBe(false);
  });

  it('returns false when event severity is below rule severity', () => {
    const rule = makeRule({ severity: 'error' });
    const buf = makeBuffer([10, 10, 10]);
    // "info" < "error" threshold
    expect(evaluateRule(rule, buf, 'info.heartbeat')).toBe(false);
  });

  it('returns true when event severity meets threshold and count exceeds', () => {
    const rule = makeRule({ severity: 'warn', threshold: 3, window_secs: 2 });
    const buf = makeBuffer([2, 2]); // sum of last 2 = 4 >= 3
    expect(evaluateRule(rule, buf, 'warn.disk')).toBe(true);
  });

  it('returns false when event count is below threshold', () => {
    const rule = makeRule({ severity: 'info', threshold: 10, window_secs: 3 });
    const buf = makeBuffer([1, 1, 1]); // sum = 3 < 10
    expect(evaluateRule(rule, buf, 'info.tick')).toBe(false);
  });

  it('clamps window_secs to at least 1', () => {
    const rule = makeRule({ threshold: 1, window_secs: 0 });
    const buf = makeBuffer([5]);
    // window_secs=0 clamps to 1, so it reads the last bucket
    expect(evaluateRule(rule, buf, 'warn.test')).toBe(true);
  });

  it('clamps window_secs to snapshot length when larger', () => {
    const rule = makeRule({ threshold: 1, window_secs: 999 });
    const buf = makeBuffer([1]);
    // window exceeds 60 — clamped to 60
    expect(evaluateRule(rule, buf, 'error.overflow')).toBe(true);
  });
});

describe('triggerAction', () => {
  it('flash action sets flash=true, promote=false', () => {
    expect(triggerAction('flash')).toEqual({ flash: true, promote: false });
  });

  it('promote action sets promote=true, flash=false', () => {
    expect(triggerAction('promote')).toEqual({ flash: false, promote: true });
  });

  it('tone action sets flash=true (and plays tone)', () => {
    const result = triggerAction('tone');
    expect(result.flash).toBe(true);
    // promote remains false for tone
    expect(result.promote).toBe(false);
  });
});

describe('newAlertRuleId', () => {
  it('returns unique ids on successive calls', () => {
    const a = newAlertRuleId();
    const b = newAlertRuleId();
    expect(a).not.toBe(b);
    expect(a).toMatch(/^alert-/);
    expect(b).toMatch(/^alert-/);
  });
});
