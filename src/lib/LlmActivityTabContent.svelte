<script lang="ts">
  import { subscribe, type Envelope } from './bus';
  import { shouldShow, type SeverityLevel } from './notifFilter';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { llmModels } from './llmModels.svelte';

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
  const totalTokensOut = $derived(
    entries
      .filter((e) => e.kind === 'llm.response')
      .reduce((sum, e) => sum + ((e.payload as any)?.tokens_out ?? 0), 0),
  );

  function handleEnvelope(env: Envelope) {
    if (!shouldShow(env.kind, severityThreshold)) return;
    entries = [...entries, env];
    if (entries.length > RECENT_LOG_LIMIT * 2) {
      entries = entries.slice(-RECENT_LOG_LIMIT);
    }
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
        return `→ ${p?.selected_model ?? '?'} (${p?.reason ?? 'manual'})`;
      case 'llm.request':
        return `${p?.estimated_tokens ?? '?'} tokens est.`;
      case 'llm.response':
        return `${p?.tokens_in ?? 0} in / ${p?.tokens_out ?? 0} out — ${p?.latency_ms ?? 0}ms`;
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
      {routeCount} routes / {requestCount} reqs / {totalTokensOut} tokens out
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
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
  }

  .status-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 10px;
    border-bottom: 1px solid rgba(168, 120, 48, 0.15);
    user-select: none;
  }

  .header-title {
    font-weight: 700;
    color: var(--amber-bright, #FFC840);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .icon {
    font-size: 10px;
  }

  .drag-back-btn {
    background: none;
    border: 1px solid rgba(168, 120, 48, 0.3);
    border-radius: 3px;
    color: var(--amber-faint);
    cursor: pointer;
    font-size: 10px;
    padding: 0 4px;
    line-height: 1.4;
  }

  .header-stats {
    font-size: 9px;
    color: var(--amber-faint, #A87830);
  }

  .live-strip {
    display: flex;
    gap: 2px;
    padding: 3px 10px;
    border-bottom: 1px solid rgba(168, 120, 48, 0.1);
    min-height: 10px;
    flex-wrap: wrap;
  }

  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    opacity: 0.8;
  }

  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .log-entry {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 2px 10px;
    line-height: 1.5;
  }
  .log-entry:hover {
    background: rgba(255, 200, 64, 0.03);
  }

  .ts {
    font-size: 9px;
    color: var(--amber-faint, #A87830);
    flex-shrink: 0;
    font-style: italic;
  }

  .kind-badge {
    font-size: 8px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border: 1px solid;
    border-radius: 3px;
    padding: 0 4px;
    flex-shrink: 0;
  }

  .model-id {
    font-weight: 700;
    color: var(--amber-bright, #FFC840);
    flex-shrink: 0;
    min-width: 28px;
  }

  .summary {
    color: var(--term-white, #E8E4D8);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    padding: 12px 10px;
    color: var(--amber-faint, #A87830);
    font-style: italic;
    font-size: 10px;
  }

  .persistent-panel {
    border-top: 1px solid rgba(168, 120, 48, 0.15);
    padding: 6px 10px;
  }

  .model-status-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 0;
    font-size: 10px;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .model-sid {
    font-weight: 700;
    color: var(--amber-bright, #FFC840);
    min-width: 28px;
  }

  .model-dname {
    color: var(--term-white, #E8E4D8);
    flex: 1;
  }

  .model-hosting {
    font-size: 9px;
    color: var(--amber-faint, #A87830);
    text-transform: uppercase;
  }
</style>
