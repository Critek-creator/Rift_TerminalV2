<script lang="ts">
  // Parallel agent run timeline — a gantt of concurrent agent lifespans
  // (candidate 7d8). Each agent run is one horizontal bar from its start to
  // its end (or to a live "now" while still running); stacking the bars on
  // separate rows over a shared time axis makes concurrency and overlap
  // visible at a glance. Pure presentation — the parent (AgentsTabContent)
  // owns the agent registry and passes a flattened run list.

  interface AgentRun {
    id: string;
    name: string;
    /** 'running' | 'cancelling' | 'completed' | 'cancelled' | 'error' */
    status: string;
    startedTs: number;
    /** End timestamp, or null while the agent is still running. */
    endTs: number | null;
  }

  let { runs }: { runs: AgentRun[] } = $props();

  // Live edge: while any run is still open, advance `now` once a second so the
  // running bars grow in real time. The interval is torn down when nothing is
  // running so an idle timeline costs nothing.
  let now = $state(Date.now());
  $effect(() => {
    if (!runs.some((r) => r.endTs === null)) return;
    const t = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(t);
  });

  const bounds = $derived.by(() => {
    if (runs.length === 0) return { t0: 0, t1: 1 };
    let t0 = Infinity;
    let t1 = -Infinity;
    for (const r of runs) {
      t0 = Math.min(t0, r.startedTs);
      t1 = Math.max(t1, r.endTs ?? now);
    }
    if (!isFinite(t0) || !isFinite(t1)) return { t0: 0, t1: 1 };
    if (t1 <= t0) t1 = t0 + 1;
    return { t0, t1 };
  });

  function pct(ts: number): number {
    const { t0, t1 } = bounds;
    return ((ts - t0) / (t1 - t0)) * 100;
  }

  function barStyle(r: AgentRun): string {
    const left = Math.max(0, pct(r.startedTs));
    const right = Math.min(100, pct(r.endTs ?? now));
    const width = Math.max(0.6, right - left);
    return `left:${left}%; width:${width}%;`;
  }

  function statusColor(status: string): string {
    switch (status) {
      case 'completed': return 'var(--term-green)';
      case 'error': return 'var(--term-red)';
      case 'cancelled': return 'var(--amber-faint)';
      case 'cancelling': return 'var(--amber-primary)';
      default: return 'var(--term-purple)'; // running
    }
  }

  function fmtDuration(ms: number): string {
    if (ms < 1000) return `${Math.max(0, ms)}ms`;
    if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
    const m = Math.floor(ms / 60_000);
    const s = Math.round((ms % 60_000) / 1000);
    return `${m}m${s.toString().padStart(2, '0')}s`;
  }

  function fmtClock(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  function runDuration(r: AgentRun): number {
    return (r.endTs ?? now) - r.startedTs;
  }
</script>

{#if runs.length === 0}
  <div class="gantt-empty">No agent runs to chart yet.</div>
{:else}
  <div class="gantt" role="img" aria-label="Timeline of agent runs">
    <div class="gantt-axis">
      <span class="axis-start">{fmtClock(bounds.t0)}</span>
      <span class="axis-span">{fmtDuration(bounds.t1 - bounds.t0)} span · {runs.length} run{runs.length === 1 ? '' : 's'}</span>
      <span class="axis-end">{fmtClock(bounds.t1)}{#if runs.some((r) => r.endTs === null)} (live){/if}</span>
    </div>
    <div class="gantt-rows">
      {#each runs as r (r.id + ':' + r.startedTs)}
        {@const live = r.endTs === null}
        <div class="gantt-row" title="{r.name} — {r.status} — {fmtDuration(runDuration(r))}">
          <span class="row-label" style="color: {statusColor(r.status)}">{r.name}</span>
          <div class="row-track">
            <div
              class="row-bar"
              class:live
              style="{barStyle(r)} background: {statusColor(r.status)};"
            ></div>
            <span class="row-dur">{fmtDuration(runDuration(r))}</span>
          </div>
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .gantt-empty {
    padding: var(--space-md);
    color: var(--amber-faint);
    font-size: var(--text-xs);
    font-style: italic;
  }
  .gantt {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm);
    min-width: 0;
  }
  .gantt-axis {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-sm);
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    border-bottom: 1px solid var(--border-subtle);
    padding-bottom: var(--space-xs);
  }
  .axis-span { color: var(--amber-dim); }
  .gantt-rows {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .gantt-row {
    display: grid;
    grid-template-columns: minmax(80px, 160px) minmax(0, 1fr);
    gap: var(--space-sm);
    align-items: center;
  }
  .row-label {
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-track {
    position: relative;
    height: 16px;
    background: rgba(212, 137, 10, 0.05);
    border-radius: var(--radius-sm);
    min-width: 0;
  }
  .row-bar {
    position: absolute;
    top: 2px;
    bottom: 2px;
    min-width: 3px;
    border-radius: var(--radius-sm);
    opacity: 0.85;
  }
  /* Subtle pulse on still-running bars to read as "in flight". */
  .row-bar.live {
    animation: gantt-pulse 1.6s ease-in-out infinite;
  }
  @keyframes gantt-pulse {
    0%, 100% { opacity: 0.85; }
    50% { opacity: 0.45; }
  }
  .row-dur {
    position: absolute;
    right: 4px;
    top: 50%;
    transform: translateY(-50%);
    font-size: var(--text-2xs);
    color: var(--amber-warm);
    font-variant-numeric: tabular-nums;
    pointer-events: none;
    text-shadow: 0 0 3px var(--term-bg, #000);
  }
</style>
