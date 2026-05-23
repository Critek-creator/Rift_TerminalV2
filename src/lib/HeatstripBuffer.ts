/**
 * HeatstripBuffer -- 60-bucket circular buffer for the heatstrip timeline.
 *
 * Each bucket covers 1 minute. The buffer holds 60 buckets (= 60 minutes).
 * Events are pushed with a severity string; the buffer tracks total count,
 * error count, and max severity per bucket.
 *
 * Usage:
 *   const buf = new HeatstripBuffer();
 *   // On each qualifying event:
 *   buf.push('info');
 *   buf.push('error');
 *   // Every 60 seconds (tick timer):
 *   buf.tick();
 *   // For rendering:
 *   const data = buf.snapshot();
 */

import type { SeverityLevel } from './riftConfig';
import { SEVERITY_RANK } from './notifFilter';

const BUCKET_COUNT = 60;

export interface HeatstripBucket {
  /** Total event count in this minute. */
  count: number;
  /** Number of error-severity events in this minute. */
  errorCount: number;
  /** Highest severity seen in this minute. */
  maxSeverity: SeverityLevel;
}

function emptyBucket(): HeatstripBucket {
  return { count: 0, errorCount: 0, maxSeverity: 'debug' };
}

export class HeatstripBuffer {
  /** Internal ring buffer storing per-minute buckets. */
  private readonly ring: HeatstripBucket[] = Array.from(
    { length: BUCKET_COUNT },
    () => emptyBucket(),
  );

  /** Write pointer -- index of the current (active) bucket. */
  private ptr = 0;

  /**
   * Advance to the next bucket. Called once per 60 seconds by the tick timer.
   * The newly-entered bucket is zeroed so it starts clean.
   */
  tick(): void {
    this.ptr = (this.ptr + 1) % BUCKET_COUNT;
    this.ring[this.ptr] = emptyBucket();
  }

  /**
   * Record an event into the current bucket.
   * @param severity - The severity level of the event.
   */
  push(severity: SeverityLevel): void {
    const bucket = this.ring[this.ptr];
    bucket.count += 1;
    if (severity === 'error') {
      bucket.errorCount += 1;
    }
    if (SEVERITY_RANK[severity] > SEVERITY_RANK[bucket.maxSeverity]) {
      bucket.maxSeverity = severity;
    }
  }

  /**
   * Return a 60-entry array ordered oldest-to-newest for rendering.
   * Index 0 is the oldest bucket; index 59 is the current (live) bucket.
   */
  snapshot(): HeatstripBucket[] {
    const out: HeatstripBucket[] = new Array(BUCKET_COUNT);
    for (let i = 0; i < BUCKET_COUNT; i++) {
      const src = this.ring[(this.ptr + 1 + i) % BUCKET_COUNT];
      out[i] = { ...src };
    }
    return out;
  }

  /** Reset all buckets to empty. */
  clear(): void {
    for (let i = 0; i < BUCKET_COUNT; i++) {
      this.ring[i] = emptyBucket();
    }
    this.ptr = 0;
  }
}
