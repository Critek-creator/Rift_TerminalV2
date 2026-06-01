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

  function onHandleDragKeydown(e: KeyboardEvent) {
    if ((e.key === 'Enter' || e.key === ' ') && onDragBack) {
      e.preventDefault();
      onDragBack();
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
      onkeydown={onHandleDragKeydown}
      title="drag back to tab strip to dock"
      aria-label="Sentinel pane — drag to dock"
    >
      <span class="handle-glyph" style="color: var(--term-red); font-size: 14px">⌾</span>
      <span class="handle-title">sentinel</span>
      <button
        type="button"
        class="dock-btn"
        draggable={false}
        onclick={(e) => { e.stopPropagation(); onDragBack?.(); }}
        title="Return to tab strip"
        aria-label="Dock pane back to tab strip"
      >↩ dock</button>
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
    <button type="button"
      class="ctrl-btn"
      class:active={!paused}
      onclick={() => (paused = !paused)}
      title={paused ? 'resume' : 'pause'}
      aria-label={paused ? 'Resume event stream' : 'Pause event stream'}
    >{paused ? '▶' : '⏸'}</button>
    <button type="button" class="ctrl-btn" onclick={clearEvents} title="clear" aria-label="Clear events">✕</button>
  </header>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if liveEvents.length === 0}
      <span class="strip-empty">◇ no recent alerts</span>
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
    <div class="log-body" aria-live="polite">
      {#if violations.length === 0 && !integrationActive}
        <div class="empty-state">
          <span class="empty-state-icon">⊘</span>
          <span class="empty-state-text">Sentinel watchdog</span>
          <span class="empty-state-hint">
            Sentinel monitors agent behavior — detecting stuck agents, runaway edits,
            and unauthorized changes. Events appear when an integration publishes to the sentinel bus category.
          </span>
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
    padding: 1rem var(--space-14);
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
    height: var(--control-sm);
    padding: 0 var(--space-12);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    gap: var(--space-md);
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: var(--type-label-size);
    letter-spacing: var(--type-label-spacing);
    font-weight: var(--type-label-weight);
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

  .status {
    height: 36px;
    padding: 0 var(--space-lg);
    background: var(--bg-elevated);
    box-shadow: var(--sep-glow);
    display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
  }
  .status .title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--term-red);
    text-shadow: 0 0 4px rgba(255, 72, 72, 0.35);
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; font-size: var(--text-lg); }
  .status .state {
    font-size: var(--type-caption-size);
    font-weight: var(--type-caption-weight);
    letter-spacing: var(--type-caption-spacing);
    color: var(--amber-dim);
  }
  .status .spacer { flex: 1; }

  .ctrl-btn {
    background: none; border: 1px solid var(--border-subtle);
    color: var(--amber-warm); padding: 1px var(--space-8);
    font-family: inherit; font-size: var(--text-xs); cursor: pointer;
    transition: background var(--duration-base), border-color var(--duration-base);
  }
  .ctrl-btn:hover { background: var(--bg-hover); border-color: var(--amber-faint); }
  .ctrl-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .ctrl-btn.active { border-color: var(--term-red); color: var(--term-red); }

  .strip {
    height: 26px;
    padding: 0 var(--space-14);
    box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-14);
    background: linear-gradient(to bottom, rgba(255, 72, 72, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--term-red); font-weight: 700; }
  .strip-empty { color: var(--amber-dim); font-size: var(--type-caption-size); font-style: italic; letter-spacing: var(--type-caption-spacing); }
  .strip-events { display: flex; gap: var(--space-sm); flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px var(--space-sm);
    border: 1px solid;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: var(--bg-red-tint);
    flex-shrink: 0;
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .log-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: var(--space-md) var(--space-lg);
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .error-state {
    color: var(--term-red);
    padding: var(--space-12) var(--space-lg);
    font-size: var(--type-body-size);
    letter-spacing: var(--type-body-spacing);
    background: var(--bg-red-tint);
    box-shadow: var(--sep-depth);
  }

  .all-clear {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-lg);
    color: var(--term-green);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: 0.06em;
  }
  .clear-icon { font-size: var(--text-xl); }

  .row {
    display: flex;
    align-items: baseline;
    gap: var(--space-8);
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

  .state-panel { box-shadow: var(--depth-lift), var(--depth-edge-light); }
  .state-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
  }
  .state-body { padding: var(--space-md) var(--space-lg); }
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
