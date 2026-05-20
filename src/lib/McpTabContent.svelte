<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { subscribe, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, type SeverityLevel } from './notifFilter';
  import { McpStatsStore, percentile } from './mcpStats';
  import SparklineChart from './SparklineChart.svelte';
  import CorrelationBadge from './CorrelationBadge.svelte';
  import type { CorrelationIndex } from './correlationIndex';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
    correlationIndex?: CorrelationIndex | null;
  }

  let { severityThreshold = 'info', onDragBack, correlationIndex = null }: Props = $props();

  const LOG_LIMIT = 200;
  const LIVE_WINDOW_MS = 4000;

  let connected = $state(false);
  let error = $state('');
  let events = $state<Envelope[]>([]);
  let toolHistogram = $state<Record<string, number>>({});
  let methodHistogram = $state<Record<string, number>>({});
  let lastTickTs = $state<number>(Date.now());
  let paused = $state(false);
  let unsubscribe: (() => Promise<void>) | undefined;
  let mounted = true;
  const mcpStore = new McpStatsStore();
  let dashboardTools = $state<Array<{ tool: string; stats: import('./mcpStats').McpToolStats }>>([]);
  type DashSort = 'calls' | 'p95' | 'errors';
  let dashSort = $state<DashSort>('calls');

  const liveEvents = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_WINDOW_MS;
    return events.filter((e) => e.ts >= cutoff);
  });
  const recentEvents = $derived(events.slice(-LOG_LIMIT).reverse());
  const totalCount = $derived(events.length);
  const lastSeenLabel = $derived.by(() => {
    if (events.length === 0) return '—';
    const last = events[events.length - 1];
    const ageMs = Math.max(0, lastTickTs - last.ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  });

  function extractTool(payload: unknown): string {
    if (!payload || typeof payload !== 'object') return '';
    const p = payload as Record<string, unknown>;
    return String(p.tool ?? p.tool_name ?? p.name ?? '');
  }

  function extractMethod(kind: string): string {
    const k = kind.toLowerCase();
    if (k.includes('invoke') || k.includes('call') || k.includes('request')) return 'INVOKE';
    if (k.includes('response') || k.includes('result')) return 'RESPONSE';
    if (k.includes('handshake') || k.includes('init') || k.includes('connect')) return 'HANDSHAKE';
    if (k.includes('notify') || k.includes('notification')) return 'NOTIFY';
    if (k.includes('error') || k.includes('fail')) return 'ERROR';
    if (k.includes('audit') || k.includes('log')) return 'AUDIT';
    return 'EVENT';
  }

  function methodColor(method: string): string {
    switch (method) {
      case 'INVOKE': return 'var(--term-blue)';
      case 'RESPONSE': return 'var(--term-green)';
      case 'HANDSHAKE': return 'var(--term-purple)';
      case 'NOTIFY': return 'var(--term-cyan)';
      case 'ERROR': return 'var(--term-red)';
      case 'AUDIT': return 'var(--amber-primary)';
      default: return 'var(--amber-dim)';
    }
  }

  function handleEnvelope(env: Envelope) {
    if (paused) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    events = [...events, env];
    if (events.length > LOG_LIMIT * 2) events = events.slice(-LOG_LIMIT);

    const tool = extractTool(env.payload);
    if (tool) {
      toolHistogram = { ...toolHistogram, [tool]: (toolHistogram[tool] ?? 0) + 1 };
    }

    const method = extractMethod(env.kind);
    methodHistogram = { ...methodHistogram, [method]: (methodHistogram[method] ?? 0) + 1 };

    const p = env.payload as Record<string, unknown> | null;
    const reqId = String(p?.request_id ?? p?.id ?? '');
    if (method === 'INVOKE' || method === 'AUDIT') {
      mcpStore.recordInvoke(tool, reqId, env.ts);
    } else if (method === 'RESPONSE') {
      const isErr = env.kind.toLowerCase().includes('error');
      mcpStore.recordResponse(reqId, env.ts, isErr);
    }

    lastTickTs = Date.now();
  }

  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    try {
      const u = await subscribe({ category: 'mcp' }, handleEnvelope);
      if (!mounted) {
        void u().catch(() => {});
      } else {
        unsubscribe = u;
        connected = true;
      }
    } catch (err) {
      console.error('[McpTab] bus_subscribe failed', err);
      error = (err as Error).message || 'Connection failed';
    }
    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
      mcpStore.tick();
      refreshDashboard();
    }, 1000);
  });

  onDestroy(() => {
    mounted = false;
    if (tickTimer) clearInterval(tickTimer);
    unsubscribe?.().catch(() => {});
  });

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  function formatPayload(payload: unknown): string {
    if (payload === null || payload === undefined) return '';
    if (typeof payload === 'string') return payload;
    try { return JSON.stringify(payload); } catch { return String(payload); }
  }

  function refreshDashboard(): void {
    let tools = mcpStore.allTools();
    if (dashSort === 'p95') {
      tools.sort((a, b) => percentile(b.stats.latencies, 95) - percentile(a.stats.latencies, 95));
    } else if (dashSort === 'errors') {
      tools.sort((a, b) => b.stats.errors - a.stats.errors);
    }
    dashboardTools = tools;
  }

  function clearEvents(): void {
    events = [];
    toolHistogram = {};
    methodHistogram = {};
    mcpStore.reset();
    dashboardTools = [];
  }

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }
</script>

<section class="pane">
  {#if onDragBack}
    <div
      class="drag-handle"
      role="button"
      tabindex="0"
      draggable={true}
      ondragstart={onHandleDragStart}
      title="drag back to tab strip to dock"
    >
      <span class="handle-glyph">↙</span>
      <span class="handle-title">mcp</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if error}
    <div class="error-state">⚠ Bus connection failed: {error}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else}
  <header class="status">
    <span class="title"><span class="icon">⬡</span>MCP</span>
    <span class="state">
      {totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
    </span>
    <span class="spacer"></span>
    <button
      class="ctrl-btn"
      class:active={!paused}
      onclick={() => (paused = !paused)}
      title={paused ? 'resume' : 'pause'}
    >{paused ? '▶' : '⏸'}</button>
    <button class="ctrl-btn" onclick={clearEvents} title="clear">✕</button>
  </header>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if liveEvents.length === 0}
      <span class="strip-empty">(no in-flight events)</span>
    {:else}
      <div class="strip-events">
        {#each liveEvents.slice(0, 10) as e, i (e.ts + ':' + e.kind + ':' + i)}
          {@const method = extractMethod(e.kind)}
          {@const tool = extractTool(e.payload)}
          <span class="strip-event" style="color: {methodColor(method)}; border-color: {methodColor(method)}">
            {method}{#if tool} {tool}{/if}
          </span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="log">
    <div class="log-header">RECENT EVENTS</div>
    <div class="log-body">
      {#if recentEvents.length === 0}
        <div class="empty-card">
          <div class="empty-title">Waiting for MCP traffic</div>
          <div class="empty-desc">This tab renders when the rift-mcp translator publishes JSON-RPC events on the bus.</div>
        </div>
      {:else}
        {#each recentEvents as e, i (e.ts + ':' + e.kind + ':' + i)}
          {@const method = extractMethod(e.kind)}
          {@const tool = extractTool(e.payload)}
          <div class="row">
            <span class="ts">{formatTs(e.ts)}</span>
            <span class="method" style="color: {methodColor(method)}">{method}</span>
            <span class="tool">{tool || '—'}</span>
            <span class="payload">{formatPayload(e.payload)}</span>
            {#if correlationIndex}
              <CorrelationBadge env={e} index={correlationIndex} />
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">MCP SUMMARY</div>
    <div class="state-body">
      <div class="k-row"><span class="k">total events</span><span class="v">{totalCount}</span></div>
      {#if Object.keys(methodHistogram).length > 0}
        <div class="histogram">
          <span class="histo-label">BY METHOD</span>
          {#each Object.entries(methodHistogram).sort(([, a], [, b]) => b - a) as [m, n] (m)}
            <div class="histo-row">
              <span class="histo-kind" style="color: {methodColor(m)}">{m}</span>
              <span class="histo-count">{n}</span>
            </div>
          {/each}
        </div>
      {/if}
      {#if Object.keys(toolHistogram).length > 0}
        <div class="tool-section">
          <span class="histo-label">TOP TOOLS</span>
          {#each Object.entries(toolHistogram).sort(([, a], [, b]) => b - a).slice(0, 8) as [t, n] (t)}
            <div class="histo-row">
              <span class="histo-kind tool-name">{t}</span>
              <span class="histo-count">{n}</span>
            </div>
          {/each}
        </div>
      {/if}
      {#if dashboardTools.length > 0}
        <div class="perf-section">
          <div class="perf-header">
            <span class="histo-label">TOOL PERFORMANCE</span>
            <span class="perf-sort">
              {#each (['calls', 'p95', 'errors'] as const) as s (s)}
                <button
                  class="sort-btn"
                  class:active={dashSort === s}
                  onclick={() => { dashSort = s; refreshDashboard(); }}
                >{s}</button>
              {/each}
            </span>
          </div>
          <div class="perf-table">
            <div class="perf-row perf-thead">
              <span class="perf-cell tool-col">tool</span>
              <span class="perf-cell num-col">calls</span>
              <span class="perf-cell num-col">err</span>
              <span class="perf-cell num-col">p50</span>
              <span class="perf-cell num-col">p95</span>
              <span class="perf-cell spark-col">rate</span>
            </div>
            {#each dashboardTools.slice(0, 12) as { tool, stats } (tool)}
              <div class="perf-row">
                <span class="perf-cell tool-col tool-name">{tool}</span>
                <span class="perf-cell num-col">{stats.calls}</span>
                <span class="perf-cell num-col" class:has-errors={stats.errors > 0}>{stats.errors}</span>
                <span class="perf-cell num-col">{stats.latencies.length > 0 ? `${percentile(stats.latencies, 50)}ms` : '—'}</span>
                <span class="perf-cell num-col">{stats.latencies.length > 0 ? `${percentile(stats.latencies, 95)}ms` : '—'}</span>
                <span class="perf-cell spark-col">
                  <SparklineChart data={stats.sparkline.snapshot()} />
                </span>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </footer>
  {/if}
</section>

<style>
  .connecting-state {
    color: var(--amber-faint);
    padding: 1rem 14px;
    font-style: italic;
    font-size: 11px;
    letter-spacing: 0.04em;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background: var(--bg-base);
    color: var(--term-blue);
    font-family: var(--font-family);
    font-size: 12px;
  }

  .drag-handle {
    height: 26px;
    padding: 0 12px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: 10px;
    letter-spacing: 0.1em;
    font-weight: 700;
  }
  .drag-handle { transition: background 0.12s ease-out; }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--term-blue);
    font-size: 12px;
  }
  .drag-handle .handle-title {
    color: var(--term-blue);
    text-transform: uppercase;
  }
  .drag-handle .handle-hint {
    margin-left: auto;
    color: var(--amber-faint);
    font-style: italic;
    font-weight: 400;
    letter-spacing: 0.04em;
  }

  .status {
    height: 30px;
    padding: 0 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    display: flex; align-items: center; gap: 14px;
    color: var(--amber-warm);
    font-size: 11px; letter-spacing: 0.1em; font-weight: 700;
  }
  .status .title { color: var(--term-blue); text-shadow: 0 0 4px rgba(108, 182, 255, 0.35); }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .state { color: var(--amber-dim); font-weight: 400; letter-spacing: 0.04em; }
  .status .spacer { flex: 1; }

  .strip {
    height: 26px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
    display: flex; align-items: center; gap: 14px;
    background: linear-gradient(to bottom, rgba(108, 182, 255, 0.06), transparent);
    color: var(--amber-dim);
    font-size: 10px;
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--term-blue); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events { display: flex; gap: 6px; flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px 6px;
    border: 1px solid;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(108, 182, 255, 0.06);
    flex-shrink: 0;
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    min-width: 0;
    border-bottom: 1px solid var(--border-subtle);
  }
  .log-header {
    padding: var(--section-header-padding, 8px 16px);
    color: var(--amber-warm);
    font-size: var(--section-header-size, 11px);
    font-weight: 700;
    letter-spacing: var(--section-header-spacing, 0.1em);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: 10px 16px;
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }
  .error-state {
    color: var(--term-red);
    padding: 12px 14px;
    font-size: 11px;
    letter-spacing: 0.04em;
    border-bottom: 1px solid rgba(255, 72, 72, 0.2);
    background: rgba(255, 72, 72, 0.06);
  }
  .empty-card {
    border: 1px dashed var(--border-subtle);
    padding: 12px 14px;
    background: rgba(108, 182, 255, 0.05);
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.55;
  }
  .empty-title {
    color: var(--term-blue);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    margin-bottom: 6px;
  }
  .empty-desc {
    color: var(--amber-dim);
    font-size: 10px;
  }

  .log-body .row {
    display: grid;
    grid-template-columns: 70px 72px 120px minmax(0, 1fr);
    gap: 8px;
    align-items: baseline;
    padding: 1px 0;
    white-space: nowrap;
    transition: background 0.12s ease-out;
  }
  .log-body .row:hover { background: rgba(108, 182, 255, 0.06); }
  .ts { color: var(--amber-faint); font-variant-numeric: tabular-nums; font-size: 10px; }
  .method { font-weight: 700; font-size: 10px; letter-spacing: 0.06em; }
  .tool { color: var(--term-blue); font-weight: 600; font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .payload { color: var(--amber-dim); font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 180px;
    overflow-y: auto;
    border-top: 1px solid var(--border-subtle);
    box-shadow: var(--depth-lift), var(--depth-edge-light);
  }
  .state-header {
    padding: var(--section-header-padding, 8px 16px);
    color: var(--amber-warm);
    font-size: var(--section-header-size, 11px);
    font-weight: 700;
    letter-spacing: var(--section-header-spacing, 0.1em);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
  }
  .state-body {
    padding: 10px 16px 14px;
    display: flex; flex-direction: column; gap: 4px;
  }
  .k-row {
    display: flex; align-items: center; justify-content: space-between;
    font-size: 10px; letter-spacing: 0.04em;
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }
  .histogram, .tool-section {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--border-subtle);
    display: flex; flex-direction: column; gap: 2px;
  }
  .histo-label {
    color: var(--amber-warm);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    margin-bottom: 2px;
  }
  .histo-row {
    display: flex; justify-content: space-between;
    font-size: 10px;
  }
  .histo-kind { font-weight: 600; }
  .histo-count { color: var(--amber-warm); font-weight: 700; font-variant-numeric: tabular-nums; }
  .tool-name { color: var(--term-blue); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }

  .ctrl-btn {
    background: none;
    border: 1px solid var(--border-subtle);
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: 11px;
    padding: 1px 6px;
    cursor: pointer;
    border-radius: 3px;
    line-height: 1;
    transition: color 0.12s ease-out, border-color 0.12s ease-out, background 0.12s ease-out;
  }
  .ctrl-btn:hover { color: var(--term-blue); border-color: var(--term-blue); }
  .ctrl-btn:focus-visible { outline: 1px solid var(--term-blue); outline-offset: 1px; }
  .ctrl-btn.active { color: var(--term-green); border-color: var(--term-green); }

  .perf-section {
    margin-top: 8px;
    border-top: 1px solid var(--border-subtle);
    padding-top: 8px;
  }
  .perf-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .perf-sort { display: flex; gap: 4px; }
  .sort-btn {
    background: transparent;
    border: 1px solid var(--border-subtle);
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: 9px;
    padding: 1px 5px;
    cursor: pointer;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .sort-btn.active {
    color: var(--amber-bright);
    border-color: var(--amber-bright);
  }
  .perf-table { display: flex; flex-direction: column; gap: 1px; }
  .perf-row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 0;
  }
  .perf-thead {
    color: var(--amber-faint);
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    border-bottom: 1px solid var(--border-subtle);
    padding-bottom: 3px;
    margin-bottom: 2px;
  }
  .perf-cell { font-size: 10px; }
  .tool-col { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .num-col { width: 42px; text-align: right; flex-shrink: 0; color: var(--amber-dim); }
  .spark-col { width: 86px; flex-shrink: 0; display: flex; justify-content: flex-end; }
  .has-errors { color: var(--term-red); }
</style>
