<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { subscribe, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, kindToSeverity, type SeverityLevel } from './notifFilter';
  import { HeatstripBuffer } from './HeatstripBuffer';
  import HeatstripTimeline from './HeatstripTimeline.svelte';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
  }

  let { severityThreshold = 'info', onDragBack }: Props = $props();

  const LOG_LIMIT = 200;
  const LIVE_WINDOW_MS = 4000;

  let connected = $state(false);
  let error = $state('');
  let events = $state<Envelope[]>([]);
  let opHistogram = $state<Record<string, number>>({});
  let dirHistogram = $state<Record<string, number>>({});
  let lastTickTs = $state<number>(Date.now());
  let paused = $state(false);
  let unsubscribe: (() => Promise<void>) | undefined;
  let mounted = true;
  const heatstrip = new HeatstripBuffer();
  let heatstripData = $state(heatstrip.snapshot());
  let heatstripTickCounter = 0;
  let logBodyEl: HTMLDivElement | undefined = $state(undefined);

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

  function extractPath(payload: unknown): string {
    if (!payload || typeof payload !== 'object') return '';
    const p = payload as Record<string, unknown>;
    return String(p.path ?? p.file ?? p.src ?? '');
  }

  function extractOp(kind: string): string {
    const k = kind.toLowerCase();
    if (k.includes('create') || k.includes('new')) return 'CREATE';
    if (k.includes('write') || k.includes('modify') || k.includes('change')) return 'WRITE';
    if (k.includes('delete') || k.includes('remove')) return 'DELETE';
    if (k.includes('rename') || k.includes('move')) return 'RENAME';
    if (k.includes('read') || k.includes('open') || k.includes('access')) return 'READ';
    return 'EVENT';
  }

  function opColor(op: string): string {
    switch (op) {
      case 'CREATE': return 'var(--term-green)';
      case 'WRITE': return 'var(--term-blue)';
      case 'DELETE': return 'var(--term-red)';
      case 'RENAME': return 'var(--term-purple)';
      case 'READ': return 'var(--amber-dim)';
      default: return 'var(--amber-faint)';
    }
  }

  function parentDir(filePath: string): string {
    const parts = filePath.replace(/\\/g, '/').split('/');
    if (parts.length <= 1) return filePath || '(root)';
    return parts.slice(0, -1).join('/');
  }

  function basename(filePath: string): string {
    const parts = filePath.replace(/\\/g, '/').split('/');
    return parts[parts.length - 1] || filePath;
  }

  function handleEnvelope(env: Envelope) {
    if (paused) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    heatstrip.push(kindToSeverity(env.kind));
    events = [...events, env];
    if (events.length > LOG_LIMIT * 2) events = events.slice(-LOG_LIMIT);

    const op = extractOp(env.kind);
    opHistogram = { ...opHistogram, [op]: (opHistogram[op] ?? 0) + 1 };

    const fp = extractPath(env.payload);
    if (fp) {
      const dir = parentDir(fp);
      dirHistogram = { ...dirHistogram, [dir]: (dirHistogram[dir] ?? 0) + 1 };
    }
    lastTickTs = Date.now();
  }

  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    try {
      const u = await subscribe({ category: 'fs' }, handleEnvelope);
      if (!mounted) {
        void u().catch(() => {});
      } else {
        unsubscribe = u;
        connected = true;
      }
    } catch (err) {
      console.error('[FsTab] bus_subscribe failed', err);
      error = (err as Error).message || 'Connection failed';
    }
    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
      heatstripTickCounter += 1;
      if (heatstripTickCounter >= 60) {
        heatstripTickCounter = 0;
        heatstrip.tick();
      }
      heatstripData = heatstrip.snapshot();
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

  function clearEvents(): void {
    events = [];
    opHistogram = {};
    dirHistogram = {};
    heatstrip.clear();
    heatstripData = heatstrip.snapshot();
  }

  function handleHeatstripSeek(minuteOffset: number): void {
    if (!logBodyEl) return;
    const now = Date.now();
    const minutesAgo = 59 - minuteOffset;
    const bucketStart = now - (minutesAgo + 1) * 60_000;
    const bucketEnd = now - minutesAgo * 60_000;
    const rows = logBodyEl.querySelectorAll<HTMLElement>('[data-ts]');
    for (const row of rows) {
      const ts = Number(row.dataset.ts);
      if (ts >= bucketStart && ts < bucketEnd) {
        row.scrollIntoView({ behavior: 'smooth', block: 'center' });
        row.style.background = 'rgba(255, 200, 64, 0.15)';
        setTimeout(() => { row.style.background = ''; }, 1200);
        return;
      }
    }
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
      <span class="handle-title">filesystem</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if error}
    <div class="error-state">⚠ Bus connection failed: {error}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else}
  <header class="status">
    <span class="title"><span class="icon">📂</span>FILESYSTEM</span>
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

  <div class="heatstrip-row">
    <HeatstripTimeline buckets={heatstripData} onseek={handleHeatstripSeek} />
  </div>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if liveEvents.length === 0}
      <span class="strip-empty">(no in-flight events)</span>
    {:else}
      <div class="strip-events">
        {#each liveEvents.slice(0, 12) as e, i (e.ts + ':' + e.kind + ':' + i)}
          {@const op = extractOp(e.kind)}
          <span class="strip-event" style="color: {opColor(op)}; border-color: {opColor(op)}">
            {op} {basename(extractPath(e.payload))}
          </span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="log">
    <div class="log-header">RECENT EVENTS</div>
    <div class="log-body" bind:this={logBodyEl}>
      {#if recentEvents.length === 0}
        <div class="empty-card">
          <div class="empty-title">Waiting for filesystem events</div>
          <div class="empty-desc">Activity appears when files are read, written, created, or deleted in the project directory.</div>
        </div>
      {:else}
        {#each recentEvents as e, i (e.ts + ':' + e.kind + ':' + i)}
          {@const op = extractOp(e.kind)}
          {@const fp = extractPath(e.payload)}
          <div class="row" data-ts={e.ts}>
            <span class="ts">{formatTs(e.ts)}</span>
            <span class="op" style="color: {opColor(op)}">{op}</span>
            <span class="filepath" title={fp}>
              {#if fp}
                <span class="dir">{parentDir(fp)}/</span><span class="file">{basename(fp)}</span>
              {:else}
                <span class="kind-raw">{e.kind}</span>
              {/if}
            </span>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">ACTIVITY SUMMARY</div>
    <div class="state-body">
      <div class="k-row"><span class="k">total events</span><span class="v">{totalCount}</span></div>
      {#if Object.keys(opHistogram).length > 0}
        <div class="histogram">
          {#each Object.entries(opHistogram).sort(([, a], [, b]) => b - a) as [op, n] (op)}
            <div class="histo-row">
              <span class="histo-kind" style="color: {opColor(op)}">{op}</span>
              <span class="histo-count">{n}</span>
            </div>
          {/each}
        </div>
      {/if}
      {#if Object.keys(dirHistogram).length > 0}
        <div class="dir-section">
          <span class="dir-label">TOP DIRECTORIES</span>
          {#each Object.entries(dirHistogram).sort(([, a], [, b]) => b - a).slice(0, 5) as [dir, n] (dir)}
            <div class="histo-row">
              <span class="histo-kind dir-path">{dir}</span>
              <span class="histo-count">{n}</span>
            </div>
          {/each}
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
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background: var(--bg-base);
    color: var(--term-cyan);
    font-family: var(--font-family);
    font-size: var(--text-base);
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
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    font-weight: 700;
  }
  .drag-handle { transition: background var(--duration-base) ease-out; }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--term-cyan);
    font-size: var(--text-base);
  }
  .drag-handle .handle-title {
    color: var(--term-cyan);
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
    font-size: var(--text-sm); letter-spacing: 0.1em; font-weight: 700;
  }
  .status .title { color: var(--term-cyan); text-shadow: 0 0 4px rgba(111, 224, 224, 0.35); }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .state { color: var(--amber-dim); font-weight: 400; letter-spacing: 0.04em; }
  .status .spacer { flex: 1; }

  .heatstrip-row {
    padding: 4px 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }

  .strip {
    height: 26px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
    display: flex; align-items: center; gap: 14px;
    background: linear-gradient(to bottom, rgba(111, 224, 224, 0.06), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--term-cyan); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events { display: flex; gap: 6px; flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px 6px;
    border: 1px solid;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(111, 224, 224, 0.06);
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
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }
  .error-state {
    color: var(--term-red);
    padding: 12px 14px;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
    border-bottom: 1px solid rgba(255, 72, 72, 0.2);
    background: rgba(255, 72, 72, 0.06);
  }
  .empty-card {
    border: 1px dashed var(--border-subtle);
    padding: 12px 14px;
    background: rgba(111, 224, 224, 0.05);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.55;
  }
  .empty-title {
    color: var(--term-cyan);
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    margin-bottom: 6px;
  }
  .empty-desc {
    color: var(--amber-dim);
    font-size: var(--text-xs);
  }

  .log-body .row {
    display: grid;
    grid-template-columns: 70px 60px minmax(0, 1fr);
    gap: 8px;
    align-items: baseline;
    padding: 1px 0;
    white-space: nowrap;
    transition: background var(--duration-base) ease-out;
  }
  .log-body .row:hover { background: rgba(111, 224, 224, 0.06); }
  .ts { color: var(--amber-faint); font-variant-numeric: tabular-nums; font-size: var(--text-xs); }
  .op { font-weight: 700; font-size: var(--text-xs); letter-spacing: 0.06em; }
  .filepath { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--amber-warm); font-size: var(--text-xs); }
  .dir { color: var(--amber-faint); }
  .file { color: var(--amber-warm); font-weight: 600; }
  .kind-raw { color: var(--amber-dim); }

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
    font-size: var(--text-xs); letter-spacing: 0.04em;
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }
  .histogram {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--border-subtle);
    display: flex; flex-direction: column; gap: 2px;
  }
  .histo-row {
    display: flex; justify-content: space-between;
    font-size: var(--text-xs);
  }
  .histo-kind { font-weight: 600; }
  .histo-count { color: var(--amber-warm); font-weight: 700; font-variant-numeric: tabular-nums; }
  .dir-section {
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border-subtle);
    display: flex; flex-direction: column; gap: 2px;
  }
  .dir-label {
    color: var(--amber-warm);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    margin-bottom: 2px;
  }
  .dir-path {
    color: var(--amber-dim);
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 240px;
  }

  .ctrl-btn {
    background: none;
    border: 1px solid var(--border-subtle);
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    padding: 1px 6px;
    cursor: pointer;
    border-radius: 3px;
    line-height: 1;
    transition: color var(--duration-base) ease-out, border-color var(--duration-base) ease-out, background var(--duration-base) ease-out;
  }
  .ctrl-btn:hover { color: var(--term-cyan); border-color: var(--term-cyan); }
  .ctrl-btn:focus-visible { outline: 1px solid var(--term-cyan); outline-offset: 1px; }
  .ctrl-btn.active { color: var(--term-green); border-color: var(--term-green); }
</style>
