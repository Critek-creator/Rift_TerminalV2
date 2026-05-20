import { SparklineBuffer } from './SparklineBuffer';

export interface McpToolStats {
  calls: number;
  errors: number;
  latencies: number[];
  sparkline: SparklineBuffer;
}

export class McpStatsStore {
  private stats = new Map<string, McpToolStats>();
  private pending = new Map<string, { tool: string; startTs: number }>();

  private getOrCreate(tool: string): McpToolStats {
    let s = this.stats.get(tool);
    if (!s) {
      s = { calls: 0, errors: 0, latencies: [], sparkline: new SparklineBuffer() };
      this.stats.set(tool, s);
    }
    return s;
  }

  recordInvoke(tool: string, requestId: string, ts: number): void {
    if (!tool || !requestId) return;
    const s = this.getOrCreate(tool);
    s.calls += 1;
    s.sparkline.record();
    this.pending.set(requestId, { tool, startTs: ts });
  }

  recordResponse(requestId: string, ts: number, isError: boolean): void {
    const req = this.pending.get(requestId);
    if (!req) return;
    this.pending.delete(requestId);
    const s = this.stats.get(req.tool);
    if (!s) return;
    if (isError) s.errors += 1;
    const latency = ts - req.startTs;
    if (latency >= 0 && latency < 300_000) {
      s.latencies.push(latency);
    }
  }

  tick(): void {
    for (const s of this.stats.values()) s.sparkline.tick();
    // Evict stale pending entries (>30s).
    const cutoff = Date.now() - 30_000;
    for (const [id, req] of this.pending.entries()) {
      if (req.startTs < cutoff) this.pending.delete(id);
    }
  }

  allTools(): Array<{ tool: string; stats: McpToolStats }> {
    const result: Array<{ tool: string; stats: McpToolStats }> = [];
    for (const [tool, stats] of this.stats) {
      result.push({ tool, stats });
    }
    result.sort((a, b) => b.stats.calls - a.stats.calls);
    return result;
  }

  reset(): void {
    this.stats.clear();
    this.pending.clear();
  }

  get size(): number {
    return this.stats.size;
  }
}

export function percentile(latencies: number[], p: number): number {
  if (latencies.length === 0) return 0;
  const sorted = [...latencies].sort((a, b) => a - b);
  const idx = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, idx)];
}
