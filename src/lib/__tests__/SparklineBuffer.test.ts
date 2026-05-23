import { describe, it, expect, beforeEach } from 'vitest';
import { SparklineBuffer } from '../SparklineBuffer';

// Unit tests for SparklineBuffer — ring buffer for 60-second sparkline data.
// Tests tick advancement, record counting, snapshot ordering, and capacity.

let buf: SparklineBuffer;

beforeEach(() => {
  buf = new SparklineBuffer();
});

describe('SparklineBuffer initial state', () => {
  it('snapshot returns 60 zeroes on fresh buffer', () => {
    const snap = buf.snapshot();
    expect(snap).toHaveLength(60);
    expect(snap.every((v) => v === 0)).toBe(true);
  });
});

describe('SparklineBuffer.record', () => {
  it('increments the current bucket', () => {
    buf.record();
    buf.record();
    buf.record();
    const snap = buf.snapshot();
    // Current bucket is the last element in snapshot
    expect(snap[59]).toBe(3);
  });

  it('accumulates in the same bucket without tick', () => {
    for (let i = 0; i < 100; i++) buf.record();
    expect(buf.snapshot()[59]).toBe(100);
  });
});

describe('SparklineBuffer.tick', () => {
  it('advances to a fresh bucket (zeroed)', () => {
    buf.record();
    buf.record();
    buf.tick();
    const snap = buf.snapshot();
    expect(snap[59]).toBe(0); // new bucket
    expect(snap[58]).toBe(2); // previous bucket
  });

  it('wraps around after 60 ticks (ring buffer)', () => {
    buf.record(); // write 1 in initial bucket
    for (let i = 0; i < 60; i++) buf.tick(); // full revolution
    // After 60 ticks, we're back at the original slot, which was zeroed
    // by the tick that entered it.
    const snap = buf.snapshot();
    expect(snap[59]).toBe(0);
  });
});

describe('SparklineBuffer.snapshot ordering', () => {
  it('returns oldest-to-newest (index 0 = oldest, 59 = current)', () => {
    // Record different counts in successive buckets
    buf.record(); // bucket 0: 1
    buf.tick();
    buf.record();
    buf.record(); // bucket 1: 2
    buf.tick();
    buf.record();
    buf.record();
    buf.record(); // bucket 2: 3

    const snap = buf.snapshot();
    // The three most recent should be at the end
    expect(snap[57]).toBe(1);
    expect(snap[58]).toBe(2);
    expect(snap[59]).toBe(3);
  });

  it('always returns exactly 60 entries', () => {
    for (let i = 0; i < 100; i++) {
      buf.record();
      buf.tick();
    }
    expect(buf.snapshot()).toHaveLength(60);
  });
});
