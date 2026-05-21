<script lang="ts">
  // Sentinel watchdog tab — §10.11 display layer for Abyssal Sentinel.
  //
  // Subscribes to Category::Sentinel. Shows rule violations, detection
  // status, and health. When no Sentinel integration is connected, renders
  // a capability-driven empty state explaining what Sentinel does.
  //
  // Event kinds this tab consumes:
  //   sentinel.violation — payload: { ruleId, level, message, file?, line?, source? }
  //   sentinel.status    — payload: { active: bool, rules_loaded: number }
  //   sentinel.health    — payload: { uptime_ms, checks_run, last_check_ts }

  import { onMount, onDestroy } from 'svelte';
  import { subscribe, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, type SeverityLevel } from './notifFilter';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
  }

  let { severityThreshold = 'info', onDragBack }: Props = $props();

  interface Violation {
    ts: number;
    ruleId: string;
    level: string;
    message: string;
    file?: string;
    line?: number | null;
    source?: string;
  }

  const LOG_LIMIT = 200;
  const LIVE_WINDOW_MS = 4000;

  let connected = $state(false);
  let error = $state('');
  let integrationActive = $state(false);
  let rulesLoaded = $state(0);
  let violations = $state<Violation[]>([]);
  let events = $state<Envelope[]>([]);
  let lastTickTs = $state<number>(Date.now());
  let paused = $state(false);
  let unsubscribe: (() => Promise<void>) | undefined;
  let mounted = true;

  const liveEvents = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_WINDOW_MS;
    return events.filter((e) => e.ts >= cutoff);
  });

  const criticalCount = $derived(violations.filter((v) => v.level === 'critical').length);
  const errorCount = $derived(violations.filter((v) => v.level === 'error').length);
  const warningCount = $derived(violations.filter((v) => v.level === 'warning').length);

  const lastSeenLabel = $derived.by(() => {
    if (events.length === 0) return '—';
    const last = events[events.length - 1];
    const ageMs = Math.max(0, lastTickTs - last.ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  });

  function levelColor(level: string): string {
    switch (level) {
      case 'critical': return 'var(--term-red)';
      case 'error': return 'var(--term-red)';
      case 'warning': return 'var(--amber-primary)';
      case 'info': return 'var(--term-cyan)';
      default: return 'var(--amber-dim)';
    }
  }

  function levelIcon(level: string): string {
    switch (level) {
      case 'critical': return '⊘';
      case 'error': return '⚠';
      case 'warning': return '◆';
      case 'info': return '◇';
      default: return '·';
    }
  }

  function handleEnvelope(env: Envelope) {
    if (paused) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    events = [...events, env];
    if (events.length > LOG_LIMIT * 2) events = events.slice(-LOG_LIMIT);

    if (env.kind.startsWith('sentinel.violation')) {
      const p = (env.payload ?? {}) as Record<string, unknown>;
      const v: Violation = {
        ts: env.ts,
        ruleId: String(p.ruleId ?? p.rule_id ?? 'unknown'),
        level: String(p.level ?? 'warning'),
        message: String(p.message ?? ''),
        file: p.file ? String(p.file) : undefined,
        line: typeof p.line === 'number' ? p.line : null,
        source: p.source ? String(p.source) : undefined,
      };
      violations = [v, ...violations].slice(0, LOG_LIMIT);
    }

    if (env.kind === 'sentinel.status') {
      const p = (env.payload ?? {}) as Record<string, unknown>;
      integrationActive = Boolean(p.active);
      if (typeof p.rules_loaded === 'number') rulesLoaded = p.rules_loaded;
    }

    lastTickTs = Date.now();
  }

  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    try {
      const u = await subscribe({ category: 'sentinel' }, handleEnvelope);
      if (!mounted) {
        void u().catch(() => {});
      } else {
        unsubscribe = u;
        connected = true;
      }
    } catch (err) {
      console.error('[SentinelTab] bus_subscribe failed', err);
      error = (err as Error).message || 'Connection failed';
    }
    tickTimer = setInterval(() => { lastTickTs = Date.now(); }, 1000);
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
    violations = [];
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
      <span class="handle-title">sentinel</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if error}
    <div class="error-state">⚠ Bus connection failed: {error}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else}
  <header class="status">
    <span class="title"><span class="icon">⊘</span>SENTINEL</span>
    <span class="state">
      {#if integrationActive}
        active · {rulesLoaded} rules · last {lastSeenLabel}
      {:else}
        standby · last {lastSeenLabel}
      {/if}
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
      <span class="strip-empty">(no recent alerts)</span>
    {:else}
      <div class="strip-events">
        {#each liveEvents.slice(0, 10) as e, i (e.ts + ':' + e.kind + ':' + i)}
          {@const p = (e.payload ?? {}) as Record<string, unknown>}
          {@const level = String(p.level ?? 'info')}
          <span class="strip-event" style="color: {levelColor(level)}; border-color: {levelColor(level)}">
            {levelIcon(level)} {String(p.ruleId ?? p.rule_id ?? e.kind.split('.').pop())}
          </span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="log">
    <div class="log-header">VIOLATIONS</div>
    <div class="log-body">
      {#if violations.length === 0 && !integrationActive}
        <div class="empty-card">
          <div class="empty-glyph">⊘</div>
          <div class="empty-title">Sentinel watchdog</div>
          <div class="empty-desc">
            Sentinel monitors agent behavior — detecting stuck agents, runaway edits,
            and unauthorized changes. It surfaces violations here when connected.
          </div>
          <div class="empty-hint">
            Sentinel events will appear here when an integration publishes to the sentinel bus category.
          </div>
        </div>
      {:else if violations.length === 0}
        <div class="all-clear">
          <span class="clear-icon">✓</span>
          <span>All clear — no violations detected</span>
        </div>
      {:else}
        {#each violations as v, i (v.ts + ':' + v.ruleId + ':' + i)}
          <div class="row level-{v.level}">
            <span class="ts">{formatTs(v.ts)}</span>
            <span class="level-icon" style="color: {levelColor(v.level)}">{levelIcon(v.level)}</span>
            <span class="rule-id">[{v.ruleId}]</span>
            <span class="msg">{v.message}</span>
            {#if v.file}
              <span class="file">{v.file.split(/[\\/]/).pop()}{v.line ? `:${v.line}` : ''}</span>
            {/if}
            {#if v.source}
              <span class="source">{v.source}</span>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">SENTINEL STATUS</div>
    <div class="state-body">
      <div class="k-row">
        <span class="k">integration</span>
        <span class="v" class:v-active={integrationActive} class:v-standby={!integrationActive}>
          {integrationActive ? 'connected' : 'standby'}
        </span>
      </div>
      {#if integrationActive}
        <div class="k-row"><span class="k">rules loaded</span><span class="v">{rulesLoaded}</span></div>
      {/if}
      <div class="k-row"><span class="k">violations</span><span class="v">{violations.length}</span></div>
      {#if criticalCount > 0}
        <div class="k-row"><span class="k">critical</span><span class="v v-critical">{criticalCount}</span></div>
      {/if}
      {#if errorCount > 0}
        <div class="k-row"><span class="k">errors</span><span class="v v-error">{errorCount}</span></div>
      {/if}
      {#if warningCount > 0}
        <div class="k-row"><span class="k">warnings</span><span class="v v-warning">{warningCount}</span></div>
      {/if}
      <div class="k-row"><span class="k">total events</span><span class="v">{events.length}</span></div>
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
    color: var(--amber-warm);
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
    color: var(--term-red);
    font-size: var(--text-base);
  }
  .drag-handle .handle-title {
    color: var(--term-red);
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
  .status .title { color: var(--term-red); text-shadow: 0 0 4px rgba(255, 72, 72, 0.35); }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .state { color: var(--amber-dim); font-weight: 400; letter-spacing: 0.04em; }
  .status .spacer { flex: 1; }

  .ctrl-btn {
    background: none; border: 1px solid var(--border-subtle);
    color: var(--amber-warm); padding: 1px 8px;
    font-family: inherit; font-size: var(--text-xs); cursor: pointer;
    transition: background var(--duration-base), border-color var(--duration-base);
  }
  .ctrl-btn:hover { background: var(--bg-hover); border-color: var(--amber-faint); }
  .ctrl-btn.active { border-color: var(--term-red); color: var(--term-red); }

  .strip {
    height: 26px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
    display: flex; align-items: center; gap: 14px;
    background: linear-gradient(to bottom, rgba(255, 72, 72, 0.04), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--term-red); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events { display: flex; gap: 6px; flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px 6px;
    border: 1px solid;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(255, 72, 72, 0.06);
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
    padding: 24px 20px;
    text-align: center;
    color: var(--amber-faint);
    margin: 12px 0;
  }
  .empty-glyph {
    font-size: 32px;
    color: var(--term-red);
    opacity: 0.5;
    margin-bottom: 12px;
  }
  .empty-title {
    color: var(--amber-warm);
    font-weight: 700;
    font-size: var(--text-base);
    letter-spacing: 0.08em;
    margin-bottom: 8px;
  }
  .empty-desc {
    font-size: var(--text-sm);
    line-height: 1.6;
    max-width: 320px;
    margin: 0 auto 8px;
    color: var(--amber-dim);
  }
  .empty-hint {
    font-size: var(--text-xs);
    font-style: italic;
    color: var(--amber-faint);
  }

  .all-clear {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px;
    color: var(--term-green);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: 0.06em;
  }
  .clear-icon { font-size: 16px; }

  .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 3px 0;
    border-bottom: 1px solid rgba(168, 120, 48, 0.06);
    min-width: 0;
  }
  .row:hover { background: rgba(212, 137, 10, 0.06); }
  .row.level-critical { color: var(--term-red); }
  .row.level-error { color: var(--term-red); opacity: 0.9; }
  .row.level-warning { color: var(--amber-primary); }

  .ts {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    flex-shrink: 0;
    min-width: 60px;
    font-variant-numeric: tabular-nums;
  }
  .level-icon { font-size: var(--text-2xs); flex-shrink: 0; }
  .rule-id {
    color: var(--amber-warm);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    flex-shrink: 0;
  }
  .msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }
  .file {
    color: var(--term-cyan);
    font-size: var(--text-2xs);
    flex-shrink: 0;
    opacity: 0.8;
  }
  .source {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-style: italic;
    flex-shrink: 0;
  }

  .state-panel { border-top: 1px solid var(--border-subtle); }
  .state-header {
    padding: var(--section-header-padding, 8px 16px);
    color: var(--amber-warm);
    font-size: var(--section-header-size, 11px);
    font-weight: 700;
    letter-spacing: var(--section-header-spacing, 0.1em);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
  }
  .state-body { padding: 10px 16px; }
  .k-row {
    display: flex;
    justify-content: space-between;
    padding: 2px 0;
    font-size: var(--text-sm);
  }
  .k { color: var(--amber-dim); }
  .v { color: var(--amber-warm); font-weight: 600; font-variant-numeric: tabular-nums; }
  .v-active { color: var(--term-green); }
  .v-standby { color: var(--amber-faint); }
  .v-critical { color: var(--term-red); font-weight: 700; }
  .v-error { color: var(--term-red); }
  .v-warning { color: var(--amber-primary); }
</style>
