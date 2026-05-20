/**
 * SparklineBuffer — fixed-size circular buffer for 60-second sparkline data.
 *
 * Usage:
 *   const buf = new SparklineBuffer();
 *   // On each bus event:
 *   buf.record();
 *   // Every 1 second (tick timer):
 *   buf.tick();
 *   // For rendering:
 *   const data = buf.snapshot();
 */

const BUFFER_SIZE = 60;

export class SparklineBuffer {
  /** Internal ring buffer storing per-second event counts. */
  private readonly ring: number[] = new Array<number>(BUFFER_SIZE).fill(0);

  /** Write pointer — index of the current (active) bucket. */
  private ptr = 0;

  /**
   * Advance to the next bucket. Called once per second by the tick timer.
   * The newly-entered bucket is zeroed so it starts clean.
   */
  tick(): void {
    this.ptr = (this.ptr + 1) % BUFFER_SIZE;
    this.ring[this.ptr] = 0;
  }

  /**
   * Increment the current bucket's count. Called on each qualifying bus event.
   */
  record(): void {
    this.ring[this.ptr] += 1;
  }

  /**
   * Return a 60-entry array ordered oldest-to-newest for rendering.
   * Index 0 is the oldest bucket; index 59 is the current (live) bucket.
   */
  snapshot(): number[] {
    const out = new Array<number>(BUFFER_SIZE);
    for (let i = 0; i < BUFFER_SIZE; i++) {
      out[i] = this.ring[(this.ptr + 1 + i) % BUFFER_SIZE];
    }
    return out;
  }
}
