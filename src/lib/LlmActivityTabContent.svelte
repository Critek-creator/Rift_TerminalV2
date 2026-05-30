<script lang="ts">
  import { subscribe, type Envelope } from './bus';
  import { shouldShow, type SeverityLevel } from './notifFilter';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { llmModels } from './llmModels.svelte';
  import { llmRouting, handleRouteEvent, handleResponseEvent } from './llmRouting.svelte';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
  }

  let { severityThreshold = 'info', onDragBack }: Props = $props();

  const RECENT_LOG_LIMIT = 200;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  let entries = $state<Envelope[]>([]);
  let lastTickTs = $state<number>(Date.now());

  const liveEntries = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_ACTIVITY_WINDOW_MS;
    return entries.filter((e) => e.ts >= cutoff);
  });

  const recentEntries = $derived(entries.slice(-RECENT_LOG_LIMIT).reverse());

  const routeCount = $derived(entries.filter((e) => e.kind === 'llm.route').length);
  const requestCount = $derived(entries.filter((e) => e.kind === 'llm.request').length);

  function handleEnvelope(env: Envelope) {
    if (!shouldShow(env.kind, severityThreshold)) return;
    entries = [...entries, env];
    if (entries.length > RECENT_LOG_LIMIT * 2) {
      entries = entries.slice(-RECENT_LOG_LIMIT);
    }
    // Feed routing store
    const p = env.payload as Record<string, unknown>;
    if (env.kind === 'llm.route') handleRouteEvent(p);
    if (env.kind === 'llm.response') handleResponseEvent(p);
  }

  let unsubscribeFn: (() => Promise<void>) | undefined;

  $effect(() => {
    (async () => {
      try {
        unsubscribeFn = await subscribe({ category: 'llm' }, handleEnvelope);
      } catch (err) {
        console.error('[LlmActivityTab] bus subscribe failed', err);
      }
    })();

    const ticker = setInterval(() => { lastTickTs = Date.now(); }, 1000);

    return () => {
      clearInterval(ticker);
      (async () => { await unsubscribeFn?.(); })();
    };
  });

  function kindLabel(kind: string): string {
    const map: Record<string, string> = {
      'llm.route': 'ROUTE',
      'llm.request': 'REQ',
      'llm.response': 'RESP',
      'llm.stream.start': 'STREAM',
      'llm.stream.end': 'DONE',
      'llm.health': 'HEALTH',
      'llm.process.start': 'START',
      'llm.process.stop': 'STOP',
      'llm.process.crash': 'CRASH',
      'llm.error': 'ERR',
    };
    return map[kind] ?? kind.replace('llm.', '').toUpperCase();
  }

  function kindColor(kind: string): string {
    if (kind.includes('crash') || kind.includes('error')) return 'var(--term-red)';
    if (kind.includes('health') || kind.includes('start')) return 'var(--term-green)';
    if (kind.includes('route')) return 'var(--amber-bright)';
    if (kind.includes('response') || kind.includes('stream.end')) return 'var(--term-blue)';
    return 'var(--amber-faint)';
  }

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  function modelName(env: Envelope): string {
    const p = env.payload as any;
    const id = p?.model_id ?? p?.model ?? '';
    const m = llmModels.getModel(id);
    return m?.short_id ?? id.slice(0, 8) ?? '';
  }

  function eventSummary(env: Envelope): string {
    const p = env.payload as any;
    switch (env.kind) {
      case 'llm.route':
        return `→ ${p?.model_id ?? '?'} (${p?.reason ?? 'manual'})${p?.was_overridden ? ' [override]' : ''}`;
      case 'llm.request':
        return `${p?.estimated_tokens ?? '?'} tokens est.`;
      case 'llm.response': {
        const cost = p?.cost_usd as number | undefined;
        const costStr = cost ? ` — ${llmRouting.formatCost(cost)}` : '';
        const escStr = p?.escalated ? ' [escalated]' : '';
        return `${p?.tokens_in ?? 0} in / ${p?.tokens_out ?? 0} out — ${p?.latency_ms ?? 0}ms${costStr}${escStr}`;
      }
      case 'llm.error':
        return `${p?.error ?? 'unknown error'}${p?.retryable ? ' [retryable]' : ''}`;
      case 'llm.health':
        return `${p?.status ?? 'unknown'}${p?.latency_ms ? ` (${p.latency_ms}ms)` : ''}`;
      case 'llm.process.start':
        return `pid ${p?.pid ?? '?'} on :${p?.port ?? '?'}`;
      case 'llm.process.stop':
        return `pid ${p?.pid ?? '?'} stopped`;
      case 'llm.process.crash':
        return `pid ${p?.pid ?? '?'} — ${p?.exit_info ?? 'unexpected exit'}`;
      default:
        return JSON.stringify(p).slice(0, 60);
    }
  }

  function onDragStart(e: DragEvent) {
    e.dataTransfer?.setData(NOTIF_TAB_MIME, 'llm-activity');
  }
</script>

<div class="llm-activity" role="region" aria-label="LLM activity">
  <!-- §10.8 Section 1 — Status header -->
  <div
    class="status-header"
    role={onDragBack ? 'button' : undefined}
    draggable={!!onDragBack}
    ondragstart={onDragStart}
  >
    <span class="header-title">
      <span class="icon">◆</span> models
      {#if onDragBack}
        <button type="button" class="drag-back-btn" onclick={onDragBack} title="Return to tab bar">↩</button>
      {/if}
    </span>
    <span class="header-stats">
      {routeCount} routes / {requestCount} reqs / {llmRouting.formatTokens(llmRouting.totalTokensOut)} out / {llmRouting.formatCost(llmRouting.sessionCostUsd)}
      {#if llmRouting.escalationCount > 0}
        <span class="escalation-badge">{llmRouting.escalationCount} esc</span>
      {/if}
    </span>
  </div>

  <!-- §10.8 Section 2 — Live activity strip -->
  {#if liveEntries.length > 0}
  <div class="live-strip">
    {#each liveEntries as env}
      <span class="live-dot" style="background: {kindColor(env.kind)}" title={env.kind}></span>
    {/each}
  </div>
  {/if}

  <!-- §10.8 Section 3 — Recent events log -->
  <div class="log-body" aria-live="polite">
    {#if recentEntries.length === 0}
      <div class="empty">No LLM events yet. Configure a model in Settings → Models.</div>
    {/if}
    {#each recentEntries as env}
      <div class="log-entry">
        <span class="ts">{formatTs(env.ts)}</span>
        <span class="kind-badge" style="border-color: {kindColor(env.kind)}; color: {kindColor(env.kind)}">{kindLabel(env.kind)}</span>
        <span class="model-id">{modelName(env)}</span>
        <span class="summary">{eventSummary(env)}</span>
      </div>
    {/each}
  </div>

  <!-- §10.8 Section 4 — Persistent panel (model status overview) -->
  <div class="persistent-panel">
    {#if llmModels.models.length === 0}
      <div class="empty">No models configured.</div>
    {:else}
      {#each llmModels.models as m}
        <div class="model-status-row">
          <span class="status-dot" style="background: {llmModels.modelStatusColor(m.id)}"></span>
          <span class="model-sid">{m.short_id || '???'}</span>
          <span class="model-dname">{m.display_name}</span>
          <span class="model-hosting">{m.hosting.mode}</span>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .llm-activity {
    display: flex;
    flex-direction: column;
    height: 100%;
    font-family: var(--font-family);
    font-size: var(--text-sm);
  }

  .status-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--space-sm) var(--space-md);
    border-bottom: 1px solid rgba(168, 120, 48, 0.15);
    user-select: none;
  }

  .status-header[role="button"]:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .header-title {
    font-weight: 700;
    color: var(--amber-bright);
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .icon {
    font-size: var(--text-xs);
  }

  .drag-back-btn {
    background: none;
    border: 1px solid rgba(168, 120, 48, 0.3);
    border-radius: var(--radius-sm);
    color: var(--amber-faint);
    cursor: pointer;
    font-size: var(--text-xs);
    padding: 0 var(--space-xs);
    line-height: 1.4;
    font-family: var(--font-family);
  }

  .drag-back-btn:hover {
    color: var(--amber-warm);
    border-color: var(--amber-faint);
  }

  .drag-back-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .header-stats {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
  }

  .live-strip {
    display: flex;
    gap: var(--space-xs);
    padding: var(--space-xs) var(--space-md);
    border-bottom: 1px solid rgba(168, 120, 48, 0.1);
    min-height: 10px;
    flex-wrap: wrap;
  }

  .live-dot {
    width: var(--space-sm);
    height: var(--space-sm);
    border-radius: var(--radius-full);
    opacity: 0.8;
  }

  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-xs) 0;
  }

  .log-entry {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    padding: 2px var(--space-md);
    line-height: 1.5;
  }
  .log-entry:hover {
    background: rgba(255, 200, 64, 0.03);
  }

  .ts {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    flex-shrink: 0;
    font-style: italic;
  }

  .kind-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 0 var(--space-xs);
    flex-shrink: 0;
  }

  .model-id {
    font-weight: 700;
    color: var(--amber-bright);
    flex-shrink: 0;
    min-width: 28px;
  }

  .summary {
    color: var(--term-white);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    padding: var(--space-12) var(--space-md);
    color: var(--amber-faint);
    font-style: italic;
    font-size: var(--text-xs);
  }

  .persistent-panel {
    border-top: 1px solid rgba(168, 120, 48, 0.15);
    padding: var(--space-sm) var(--space-md);
  }

  .model-status-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: 2px 0;
    font-size: var(--text-xs);
  }

  .status-dot {
    width: var(--space-sm);
    height: var(--space-sm);
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .model-sid {
    font-weight: 700;
    color: var(--amber-bright);
    min-width: 28px;
  }

  .model-dname {
    color: var(--term-white);
    flex: 1;
  }

  .model-hosting {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    text-transform: uppercase;
  }

  .escalation-badge {
    color: var(--term-red);
    font-weight: 700;
    margin-left: var(--space-xs);
  }
</style>
