<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import SparklineChart from './SparklineChart.svelte';

  interface CategoryCount {
    category: string;
    count: number;
  }

  interface SessionSummary {
    session_id: string;
    event_count: number;
    error_count: number;
    duration_ms: number;
    category_counts: CategoryCount[];
    event_types: string[];
    timeline_buckets: number[];
  }

  interface FrequencyDelta {
    category: string;
    baseline_count: number;
    compare_count: number;
    delta: number;
  }

  interface SessionDiff {
    baseline: SessionSummary;
    compare: SessionSummary;
    new_types: string[];
    missing_types: string[];
    frequency_deltas: FrequencyDelta[];
    error_delta: number;
    duration_delta: number;
  }

  interface Props {
    baselineId: string;
    compareId: string;
    baselineDate: string;
    compareDate: string;
    onBack: () => void;
  }

  let { baselineId, compareId, baselineDate, compareDate, onBack }: Props = $props();

  let diff = $state<SessionDiff | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let overlayMode = $state(false);

  const CAT_COLOR: Record<string, string> = {
    pty:      'var(--term-white)',
    hook:     'var(--term-cyan)',
    agent:    'var(--term-purple)',
    fs:       'var(--amber-faint)',
    index:    'var(--status-blue-bright, #6CB6FF)',
    aegis:    'var(--amber-primary)',
    status:   'var(--amber-bright)',
    system:   'var(--term-red)',
    mcp:      'var(--term-purple, #C58FFF)',
    sentinel: 'var(--term-red)',
  };

  function catColor(cat: string): string {
    return CAT_COLOR[cat] ?? 'var(--amber-dim)';
  }

  function formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`;
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60);
    const rem = s % 60;
    if (m < 60) return `${m}m ${rem}s`;
    const h = Math.floor(m / 60);
    const remM = m % 60;
    return `${h}h ${remM}m`;
  }

  function formatDelta(n: number): string {
    if (n > 0) return `+${n.toLocaleString()}`;
    if (n < 0) return n.toLocaleString();
    return '0';
  }

  function deltaClass(n: number): string {
    if (n > 0) return 'delta-up';
    if (n < 0) return 'delta-down';
    return 'delta-zero';
  }

  function errorDeltaClass(n: number): string {
    // For errors, increase = bad (red), decrease = good (green)
    if (n > 0) return 'delta-error-up';
    if (n < 0) return 'delta-error-down';
    return 'delta-zero';
  }

  /** Normalize timeline buckets to the same length for overlay comparison. */
  function normalizeTimeline(buckets: number[], targetLen: number): number[] {
    if (buckets.length === 0) return new Array(targetLen).fill(0);
    if (buckets.length === targetLen) return buckets;
    const result = new Array(targetLen).fill(0);
    const scale = buckets.length / targetLen;
    for (let i = 0; i < targetLen; i++) {
      const srcIdx = Math.min(Math.floor(i * scale), buckets.length - 1);
      result[i] = buckets[srcIdx] ?? 0;
    }
    return result;
  }

  const overlayLen = $derived(
    diff
      ? Math.max(
          diff.baseline.timeline_buckets.length,
          diff.compare.timeline_buckets.length,
          1,
        )
      : 1,
  );

  const baselineTimeline = $derived(
    diff ? normalizeTimeline(diff.baseline.timeline_buckets, overlayLen) : [],
  );
  const compareTimeline = $derived(
    diff ? normalizeTimeline(diff.compare.timeline_buckets, overlayLen) : [],
  );

  const overlayMax = $derived(
    Math.max(1, ...baselineTimeline, ...compareTimeline),
  );

  function overlayPoints(data: number[], max: number): string {
    if (data.length === 0) return '';
    if (data.length === 1) return '100,16';
    return data
      .map(
        (v, i) =>
          `${(i / (data.length - 1)) * 200},${32 - 4 - (v / max) * 24}`,
      )
      .join(' ');
  }

  async function loadComparison() {
    loading = true;
    error = null;
    try {
      diff = await invoke<SessionDiff>('compare_sessions', {
        baselineId,
        compareId,
      });
    } catch (err) {
      error = String((err as Error).message ?? err);
      diff = null;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Re-run when ids change.
    void baselineId;
    void compareId;
    loadComparison();
  });
</script>

<section class="compare-root">
  <header class="compare-header">
    <button type="button" class="rift-btn rift-btn--sm" onclick={onBack}>
      &#x25C0; back
    </button>
    <span class="compare-title">SESSION COMPARISON</span>
    <span class="compare-subtitle">
      {baselineDate} vs {compareDate}
    </span>
    <span class="spacer"></span>
    <button
      type="button"
      class="rift-btn rift-btn--sm"
      class:rift-btn--active={overlayMode}
      onclick={() => (overlayMode = !overlayMode)}
    >
      {overlayMode ? 'split view' : 'overlay'}
    </button>
  </header>

  {#if error}
    <div class="error-state">&#x26A0; {error}</div>
  {/if}

  {#if loading}
    <div class="loading-card">
      <div class="loading-title">Analyzing sessions...</div>
      <div class="loading-desc">Computing event pattern diff between sessions.</div>
    </div>
  {:else if diff}
    <!-- Summary delta strip -->
    <div class="delta-strip">
      <div class="delta-chip">
        <span class="delta-label">Events</span>
        <span class="delta-val {deltaClass(diff.compare.event_count - diff.baseline.event_count)}">
          {formatDelta(diff.compare.event_count - diff.baseline.event_count)}
        </span>
      </div>
      <div class="delta-chip">
        <span class="delta-label">Errors</span>
        <span class="delta-val {errorDeltaClass(diff.error_delta)}">
          {formatDelta(diff.error_delta)}
        </span>
      </div>
      <div class="delta-chip">
        <span class="delta-label">Duration</span>
        <span class="delta-val {deltaClass(diff.duration_delta)}">
          {diff.duration_delta > 0 ? '+' : ''}{formatDuration(Math.abs(diff.duration_delta))}
        </span>
      </div>
      <div class="delta-chip">
        <span class="delta-label">New types</span>
        <span class="delta-val delta-new">{diff.new_types.length}</span>
      </div>
      <div class="delta-chip">
        <span class="delta-label">Gone</span>
        <span class="delta-val delta-gone">{diff.missing_types.length}</span>
      </div>
    </div>

    <!-- Timeline sparklines -->
    <div class="timeline-section">
      <div class="section-label">TIMELINE</div>
      {#if overlayMode}
        <div class="overlay-sparkline-wrap">
          <div class="overlay-chart">
            <svg
              class="overlay-svg"
              viewBox="0 0 200 32"
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              <!-- Baseline line (faint) -->
              <polyline
                points={overlayPoints(baselineTimeline, overlayMax)}
                fill="none"
                stroke="var(--amber-faint, #A87830)"
                stroke-width="1.5"
                stroke-linejoin="round"
                stroke-linecap="round"
                opacity="0.8"
              />
              <!-- Compare line (bright) -->
              <polyline
                points={overlayPoints(compareTimeline, overlayMax)}
                fill="none"
                stroke="var(--amber-bright, #FFC840)"
                stroke-width="1.5"
                stroke-linejoin="round"
                stroke-linecap="round"
              />
            </svg>
          </div>
          <div class="overlay-legend">
            <span class="legend-item legend-baseline">baseline</span>
            <span class="legend-item legend-compare">compare</span>
          </div>
        </div>
      {:else}
        <div class="split-sparklines">
          <div class="sparkline-col">
            <div class="sparkline-label">BASELINE</div>
            <SparklineChart data={diff.baseline.timeline_buckets} />
            <span class="sparkline-stat">{diff.baseline.event_count.toLocaleString()} events</span>
          </div>
          <div class="sparkline-col">
            <div class="sparkline-label">COMPARE</div>
            <SparklineChart data={diff.compare.timeline_buckets} />
            <span class="sparkline-stat">{diff.compare.event_count.toLocaleString()} events</span>
          </div>
        </div>
      {/if}
    </div>

    <!-- Frequency delta table -->
    <div class="freq-section">
      <div class="section-label">FREQUENCY BY CATEGORY</div>
      <div class="freq-table">
        <div class="freq-row freq-row--header">
          <span class="freq-cat">Category</span>
          <span class="freq-base">Baseline</span>
          <span class="freq-comp">Compare</span>
          <span class="freq-delta">Delta</span>
        </div>
        {#each diff.frequency_deltas as fd (fd.category)}
          <div class="freq-row">
            <span class="freq-cat" style="color: {catColor(fd.category)};">{fd.category}</span>
            <span class="freq-base">{fd.baseline_count.toLocaleString()}</span>
            <span class="freq-comp">{fd.compare_count.toLocaleString()}</span>
            <span class="freq-delta {deltaClass(fd.delta)}">{formatDelta(fd.delta)}</span>
          </div>
        {/each}
      </div>
    </div>

    <!-- New / Missing event types -->
    {#if diff.new_types.length > 0 || diff.missing_types.length > 0}
      <div class="types-section">
        <div class="section-label">EVENT TYPE CHANGES</div>
        <div class="types-grid">
          {#if diff.new_types.length > 0}
            <div class="types-col">
              <div class="types-col-label types-new-label">NEW</div>
              {#each diff.new_types as t (t)}
                <div class="type-row type-new">{t}</div>
              {/each}
            </div>
          {/if}
          {#if diff.missing_types.length > 0}
            <div class="types-col">
              <div class="types-col-label types-gone-label">GONE</div>
              {#each diff.missing_types as t (t)}
                <div class="type-row type-gone">{t}</div>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    {/if}
  {/if}
</section>

<style>
  .compare-root {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow-y: auto;
    background: var(--bg-base);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: 12px;
  }
  .compare-root::-webkit-scrollbar { width: 5px; }
  .compare-root::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .compare-header {
    height: 30px;
    padding: 0 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    display: flex;
    align-items: center;
    gap: 14px;
    color: var(--amber-warm);
    font-size: 11px;
    letter-spacing: 0.1em;
    font-weight: 700;
    flex-shrink: 0;
  }
  .compare-title {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .compare-subtitle {
    color: var(--amber-dim);
    font-weight: 400;
    letter-spacing: 0.04em;
    font-size: 10px;
  }
  .spacer { flex: 1; }

  /* .rift-btn tokens */
  .rift-btn {
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 9px;
    letter-spacing: 0.1em;
    font-weight: 700;
    padding: 2px 8px;
    cursor: pointer;
    text-transform: uppercase;
    border-radius: var(--radius-md, 4px);
    transition: color var(--duration-base, 0.12s) var(--ease-out, ease-out),
                background var(--duration-base, 0.12s) var(--ease-out, ease-out),
                border-color var(--duration-base, 0.12s) var(--ease-out, ease-out);
  }
  .rift-btn:hover {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .rift-btn:focus-visible {
    outline: 1px solid var(--amber-bright);
    outline-offset: 1px;
  }
  .rift-btn--sm {
    padding: 2px 8px;
    font-size: 9px;
  }
  .rift-btn--active {
    background: rgba(255, 200, 64, 0.12);
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }

  .error-state {
    color: var(--term-red);
    padding: 12px 14px;
    font-size: 11px;
    letter-spacing: 0.04em;
    border-bottom: 1px solid rgba(255, 72, 72, 0.2);
    background: rgba(255, 72, 72, 0.06);
    flex-shrink: 0;
  }

  .loading-card {
    border: 1px dashed var(--border-subtle);
    padding: 12px 14px;
    margin: 10px 16px;
    background: rgba(212, 137, 10, 0.05);
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.55;
  }
  .loading-title {
    color: var(--amber-bright);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    margin-bottom: 6px;
  }
  .loading-desc {
    color: var(--amber-dim);
    font-size: 10px;
  }

  /* --- Delta strip --- */
  .delta-strip {
    display: flex;
    gap: 8px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border-subtle);
    flex-wrap: wrap;
    flex-shrink: 0;
  }
  .delta-chip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    min-width: 64px;
    padding: 6px 10px;
    border: 1px solid rgba(255, 168, 38, 0.15);
    border-radius: var(--radius-md, 4px);
    background: rgba(255, 168, 38, 0.05);
  }
  .delta-label {
    font-size: 9px;
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 600;
  }
  .delta-val {
    font-size: 13px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .delta-up { color: var(--amber-bright); }
  .delta-down { color: var(--term-blue); }
  .delta-zero { color: var(--amber-dim); }
  .delta-error-up { color: var(--term-red); }
  .delta-error-down { color: var(--term-green); }
  .delta-new { color: var(--term-green); }
  .delta-gone { color: var(--term-red); }

  /* --- Section labels --- */
  .section-label {
    padding: var(--section-header-padding, 8px 16px);
    color: var(--amber-warm);
    font-size: var(--section-header-size, 11px);
    font-weight: var(--section-header-weight, 700);
    letter-spacing: var(--section-header-spacing, 0.1em);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
  }

  /* --- Timeline sparklines --- */
  .timeline-section {
    flex-shrink: 0;
    border-bottom: 1px solid var(--border-subtle);
  }

  .split-sparklines {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    padding: 12px 16px;
  }
  .sparkline-col {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }
  .sparkline-label {
    font-size: 9px;
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 600;
  }
  .sparkline-stat {
    font-size: 10px;
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
  }

  .overlay-sparkline-wrap {
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .overlay-chart {
    border: 1px solid rgba(255, 168, 38, 0.15);
    border-radius: var(--radius-md, 4px);
    background: rgba(255, 168, 38, 0.05);
    padding: 4px;
  }
  .overlay-svg {
    display: block;
    width: 100%;
    height: 32px;
  }
  .overlay-legend {
    display: flex;
    gap: 16px;
    font-size: 9px;
    letter-spacing: 0.06em;
  }
  .legend-item {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .legend-item::before {
    content: '';
    display: inline-block;
    width: 12px;
    height: 2px;
    border-radius: 1px;
  }
  .legend-baseline::before {
    background: var(--amber-faint, #A87830);
  }
  .legend-compare::before {
    background: var(--amber-bright, #FFC840);
  }
  .legend-baseline { color: var(--amber-faint); }
  .legend-compare { color: var(--amber-bright); }

  /* --- Frequency table --- */
  .freq-section {
    flex-shrink: 0;
    border-bottom: 1px solid var(--border-subtle);
  }
  .freq-table {
    padding: 4px 16px 10px;
  }
  .freq-row {
    display: grid;
    grid-template-columns: 90px 80px 80px 80px;
    gap: 8px;
    align-items: baseline;
    padding: 3px 0;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .freq-row--header {
    font-size: 9px;
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 600;
    border-bottom: 1px solid rgba(255, 168, 38, 0.1);
    padding-bottom: 4px;
    margin-bottom: 2px;
  }
  .freq-cat {
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 10px;
  }
  .freq-base, .freq-comp {
    color: var(--amber-warm);
    text-align: right;
  }
  .freq-delta {
    text-align: right;
    font-weight: 700;
  }

  /* --- Event types --- */
  .types-section {
    flex-shrink: 0;
  }
  .types-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    padding: 10px 16px;
  }
  .types-col {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .types-col-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    padding-bottom: 4px;
    border-bottom: 1px solid rgba(255, 168, 38, 0.08);
    margin-bottom: 2px;
  }
  .types-new-label { color: var(--term-green); }
  .types-gone-label { color: var(--term-red); }
  .type-row {
    font-size: 10px;
    font-family: var(--font-family);
    padding: 2px 6px;
    border-radius: var(--radius-sm, 2px);
  }
  .type-new {
    color: var(--term-green);
    background: rgba(79, 232, 85, 0.06);
  }
  .type-gone {
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.06);
  }
</style>
