<script lang="ts">
  // Phase 8.7k — Agents tracker tab (§10.11 display layer + §10.17 grouping).
  //
  // Subscribes to Category::Agent and tracks live agents from envelope
  // streams. Cancel button publishes `agent.cancel` to the bus; integrations
  // (Aegis today, Sentinel post-D-010) listen and fulfill (§9 control
  // endpoint pattern). When no integration has published yet, renders a
  // capability-driven empty state instead of an error.
  //
  // Schema this tab assumes (documented here so future translators have
  // a contract to target — they're free to add more kinds, these are the
  // ones that drive the live-running set):
  //
  //   agent.start    — payload: { id, name?, kind?, source? }
  //   agent.activity — payload: { id, message?, progress? (0..1) }
  //   agent.end      — payload: { id, status: 'completed'|'cancelled'|'error', message? }
  //   agent.cancel   — published BY this tab, payload: { id, reason?, requested_by: 'rift-ui' }
  //
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: $effect cleanup must be
  // sync; bus unsubscribe wraps in IIFE.

  import { onMount, onDestroy } from 'svelte';
  import { subscribe, publish, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, kindToSeverity, type SeverityLevel } from './notifFilter';
  import { HeatstripBuffer } from './HeatstripBuffer';
  import HeatstripTimeline from './HeatstripTimeline.svelte';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
  }

  let { severityThreshold = 'info', onDragBack }: Props = $props();

  type AgentStatus = 'running' | 'cancelling' | 'completed' | 'cancelled' | 'error';

  interface AgentState {
    id: string;
    name: string;
    kind?: string;
    source?: string;
    status: AgentStatus;
    startedTs: number;
    lastActivityTs: number;
    lastMessage?: string;
    progress?: number;
    endStatus?: string;
    endMessage?: string;
  }

  const ARCHIVE_LIMIT = 50;
  const RUNNING_INACTIVITY_HINT_MS = 30_000; // > 30s with no activity = "stuck?" hint

  // Live registry — keyed by agent id. Reactive map via reassignment.
  let connected = $state(false);
  let connectError = $state<string | null>(null);
  let agents = $state<Record<string, AgentState>>({});
  // Archive of finished agents, newest first.
  let archive = $state<AgentState[]>([]);
  let totalEvents = $state(0);
  let lastTickTs = $state<number>(Date.now());
  let unsubscribe: (() => Promise<void>) | undefined;
  let mounted = true;
  const heatstrip = new HeatstripBuffer();
  let heatstripData = $state(heatstrip.snapshot());
  let heatstripTickCounter = 0;

  // Sorted live agents — running on top, cancelling next, then by start ts.
  const runningAgents = $derived.by(() => {
    return Object.values(agents)
      .filter((a) => a.status === 'running' || a.status === 'cancelling')
      .sort((a, b) => {
        if (a.status !== b.status) {
          return a.status === 'running' ? -1 : 1;
        }
        return a.startedTs - b.startedTs;
      });
  });

  const runningCount = $derived(runningAgents.length);
  const archiveCount = $derived(archive.length);
  const integrationDetected = $derived(totalEvents > 0);

  function handleEnvelope(env: Envelope) {
    if (!shouldShow(env.kind, severityThreshold)) return;
    heatstrip.push(kindToSeverity(env.kind));
    totalEvents += 1;
    lastTickTs = Date.now();

    const payload = (env.payload ?? {}) as Record<string, unknown>;
    const id = typeof payload.id === 'string' ? payload.id : null;

    switch (env.kind) {
      case 'agent.start': {
        if (!id) return;
        const next: AgentState = {
          id,
          name: stringOr(payload.name, id),
          kind: stringOr(payload.kind, undefined),
          source: stringOr(payload.source, undefined),
          status: 'running',
          startedTs: env.ts,
          lastActivityTs: env.ts,
        };
        agents = { ...agents, [id]: next };
        break;
      }

      case 'agent.activity': {
        if (!id || !agents[id]) return;
        const cur = agents[id];
        const message = stringOr(payload.message, undefined);
        const progress = numberOr(payload.progress, undefined);
        agents = {
          ...agents,
          [id]: {
            ...cur,
            lastActivityTs: env.ts,
            lastMessage: message ?? cur.lastMessage,
            progress: progress ?? cur.progress,
          },
        };
        break;
      }

      case 'agent.end': {
        if (!id || !agents[id]) return;
        const cur = agents[id];
        const endStatus = stringOr(payload.status, 'completed');
        const endMessage = stringOr(payload.message, undefined);
        const status: AgentStatus =
          endStatus === 'cancelled' ? 'cancelled'
          : endStatus === 'error' ? 'error'
          : 'completed';
        const ended: AgentState = {
          ...cur,
          status,
          endStatus,
          endMessage,
          lastActivityTs: env.ts,
        };
        // Move out of live registry into archive (newest first, capped).
        const { [id]: _, ...rest } = agents;
        agents = rest;
        archive = [ended, ...archive].slice(0, ARCHIVE_LIMIT);
        break;
      }

      default:
        break;
    }
  }

  function stringOr<T>(v: unknown, fallback: T): string | T {
    return typeof v === 'string' ? v : fallback;
  }
  function numberOr<T>(v: unknown, fallback: T): number | T {
    return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
  }

  async function cancelAgent(a: AgentState) {
    // Optimistically mark cancelling so the UI reflects intent immediately.
    if (agents[a.id]) {
      agents = {
        ...agents,
        [a.id]: { ...agents[a.id], status: 'cancelling' },
      };
    }
    try {
      await publish('agent', 'agent.cancel', {
        id: a.id,
        reason: 'user requested via Rift agents tab',
        requested_by: 'rift-ui',
      });
    } catch (err) {
      console.error('[AgentsTab] failed to publish agent.cancel', err);
      // Roll back the optimistic transition so the user can retry.
      if (agents[a.id]) {
        agents = {
          ...agents,
          [a.id]: { ...agents[a.id], status: 'running' },
        };
      }
    }
  }

  function clearArchive() {
    archive = [];
  }

  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    try {
      const u = await subscribe({ category: 'agent' }, handleEnvelope);
      if (!mounted) {
        void u().catch(() => {});
      } else {
        unsubscribe = u;
        connected = true;
      }
    } catch (err) {
      console.error('[AgentsTab] bus_subscribe failed', err);
      connectError = err instanceof Error ? err.message : String(err);
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

  function formatAge(ts: number): string {
    const ageMs = Math.max(0, lastTickTs - ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  }

  function formatDuration(startedTs: number, endTs: number): string {
    const ms = Math.max(0, endTs - startedTs);
    if (ms < 1000) return `${ms}ms`;
    if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
    if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
    return `${Math.floor(ms / 3_600_000)}h ${Math.floor((ms % 3_600_000) / 60_000)}m`;
  }

  function isInactive(a: AgentState): boolean {
    return a.status === 'running'
      && (lastTickTs - a.lastActivityTs) > RUNNING_INACTIVITY_HINT_MS;
  }

  function archiveStatusColor(status: AgentStatus): string {
    if (status === 'completed') return 'var(--term-green, #4FE855)';
    if (status === 'cancelled') return 'var(--amber-faint)';
    if (status === 'error') return 'var(--term-red)';
    return 'var(--amber-warm)';
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
      <span class="handle-title">agents</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if connectError}
    <div class="connect-error">{connectError}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else}
  <header class="status">
    <span class="title"><span class="icon">◊</span>AGENTS</span>
    <span class="state">
      {#if integrationDetected}
        {runningCount} running · {archiveCount} archived · {totalEvents} event{totalEvents === 1 ? '' : 's'}
      {:else}
        no agent envelopes received yet
      {/if}
    </span>
  </header>

  <div class="heatstrip-row">
    <HeatstripTimeline buckets={heatstripData} />
  </div>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if runningCount === 0}
      <span class="strip-empty">(no running agents)</span>
    {:else}
      {#each runningAgents.slice(0, 8) as a (a.id)}
        <span class="strip-event" class:cancelling={a.status === 'cancelling'}>
          {a.name}
          {#if a.progress !== undefined}
            <span class="strip-progress">{Math.round(a.progress * 100)}%</span>
          {/if}
        </span>
      {/each}
      {#if runningCount > 8}
        <span class="strip-overflow">+{runningCount - 8}</span>
      {/if}
    {/if}
  </div>

  <div class="log">
    <div class="log-header">RUNNING AGENTS</div>
    <div class="log-body">
      {#if !integrationDetected}
        <div class="capability-card">
          <div class="cap-heading">No agent integration loaded</div>
          <div class="cap-subtitle">
            This tab subscribes to <code>Category::Agent</code> envelopes and
            renders any agent the protocol surfaces. It populates automatically
            when a translator (Aegis, Sentinel, or another integration) starts
            publishing on the bus.
          </div>
          <div class="cap-protocol">
            Expected envelope kinds:
            <ul>
              <li><code>agent.start</code> — <span class="cap-payload">{`{ id, name?, kind?, source? }`}</span></li>
              <li><code>agent.activity</code> — <span class="cap-payload">{`{ id, message?, progress? }`}</span></li>
              <li><code>agent.end</code> — <span class="cap-payload">{`{ id, status, message? }`}</span></li>
            </ul>
            The cancel button publishes <code>agent.cancel</code> on the bus —
            translators listen and fulfill (§9 control-endpoint pattern).
          </div>
        </div>
      {:else if runningCount === 0}
        <div class="empty">
          no agents currently running — completed agents move to the archive below
        </div>
      {:else}
        {#each runningAgents as a (a.id)}
          <div class="card" class:inactive={isInactive(a)} class:cancelling={a.status === 'cancelling'}>
            <div class="card-row card-top">
              <span class="card-name">{a.name}</span>
              {#if a.kind}<span class="card-kind">{a.kind}</span>{/if}
              {#if a.source}<span class="card-source">{a.source}</span>{/if}
              <span class="spacer"></span>
              {#if a.status === 'cancelling'}
                <span class="card-status card-cancelling">cancelling…</span>
              {:else if isInactive(a)}
                <span class="card-status card-inactive">no activity {formatAge(a.lastActivityTs)}</span>
              {:else}
                <span class="card-status card-running">running</span>
              {/if}
              <button
                type="button"
                class="card-cancel"
                onclick={() => cancelAgent(a)}
                disabled={a.status === 'cancelling'}
                title={a.status === 'cancelling' ? 'cancel already requested' : 'cancel agent'}
                aria-label="cancel agent"
              >×</button>
            </div>
            <div class="card-row card-meta">
              <span class="card-id" title={a.id}>{a.id}</span>
              <span class="card-sep">·</span>
              <span class="card-age">started {formatAge(a.startedTs)}</span>
              {#if a.progress !== undefined}
                <span class="card-sep">·</span>
                <span class="card-progress-num">{Math.round(a.progress * 100)}%</span>
              {/if}
            </div>
            {#if a.lastMessage}
              <div class="card-message">{a.lastMessage}</div>
            {/if}
            {#if a.progress !== undefined}
              <div class="card-progress-bar">
                <div class="card-progress-fill" style="width: {Math.min(100, Math.max(0, a.progress * 100))}%;"></div>
              </div>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">
      <span>RECENT ARCHIVE</span>
      {#if archiveCount > 0}
        <button type="button" class="archive-clear" onclick={clearArchive}>clear</button>
      {/if}
    </div>
    <div class="state-body">
      {#if archive.length === 0}
        <div class="empty small">no completed agents yet</div>
      {:else}
        {#each archive.slice(0, 12) as a (a.id + ':' + a.lastActivityTs)}
          <div class="archive-row">
            <span class="archive-status" style="color: {archiveStatusColor(a.status)};">
              {a.endStatus ?? a.status}
            </span>
            <span class="archive-name">{a.name}</span>
            <span class="archive-dur">{formatDuration(a.startedTs, a.lastActivityTs)}</span>
            <span class="archive-when">{formatAge(a.lastActivityTs)}</span>
          </div>
        {/each}
        {#if archiveCount > 12}
          <div class="archive-overflow">+{archiveCount - 12} older</div>
        {/if}
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
  .connect-error {
    color: var(--term-red);
    padding: 8px 14px;
    font-style: italic;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
    opacity: 0.9;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
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
    transition: background var(--duration-base)ease-out;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--term-purple);
    font-size: var(--text-base);
  }
  .drag-handle .handle-title {
    color: var(--term-purple);
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
  .status .title {
    color: var(--term-purple);
    text-shadow: 0 0 6px rgba(176, 120, 232, 0.4);
  }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .state {
    color: var(--amber-dim);
    font-weight: 400;
    letter-spacing: 0.04em;
  }

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
    display: flex; align-items: center; gap: 8px;
    background: linear-gradient(to bottom, rgba(176, 120, 232, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--term-purple); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-event {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border: 1px solid var(--term-purple);
    color: var(--term-purple);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(176, 120, 232, 0.08);
  }
  .strip-event.cancelling {
    border-color: var(--amber-faint);
    color: var(--amber-faint);
    background: transparent;
  }
  .strip-progress {
    color: var(--amber-bright);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .strip-overflow {
    color: var(--amber-faint);
    font-style: italic;
    margin-left: 4px;
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
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
    padding: 10px 16px;
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .empty {
    color: var(--amber-faint);
    font-style: italic;
  }
  .empty.small { font-size: var(--text-xs); }

  .capability-card {
    border: 1px dashed var(--border-subtle);
    padding: 12px 14px;
    background: rgba(176, 120, 232, 0.05);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.55;
  }
  .cap-heading {
    color: var(--term-purple);
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    margin-bottom: 8px;
  }
  .cap-subtitle {
    color: var(--amber-warm);
    margin-bottom: 10px;
  }
  .cap-protocol {
    color: var(--amber-dim);
    font-size: var(--text-xs);
  }
  .cap-protocol code {
    color: var(--term-purple);
    font-size: var(--text-xs);
  }
  .cap-protocol ul {
    list-style: none;
    padding: 6px 0 8px;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cap-protocol li {
    padding-left: 12px;
    position: relative;
  }
  .cap-protocol li::before {
    content: '·';
    position: absolute;
    left: 4px;
    color: var(--amber-faint);
  }
  .cap-payload { color: var(--amber-faint); font-style: italic; }

  .card {
    border: 1px solid var(--term-purple);
    background: rgba(176, 120, 232, 0.06);
    padding: 8px 10px;
    margin-bottom: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .card.inactive {
    border-color: var(--amber-faint);
    background: rgba(168, 120, 48, 0.06);
  }
  .card.cancelling {
    border-color: var(--amber-faint);
    opacity: 0.75;
  }
  .card-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .card-name {
    color: var(--term-purple);
    font-weight: 700;
    font-size: var(--text-base);
    letter-spacing: 0.04em;
  }
  .card-kind {
    color: var(--amber-warm);
    border: 1px solid var(--amber-faint);
    padding: 0 4px;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .card-source {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    font-style: italic;
  }
  .spacer { flex: 1; }
  .card-status {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    padding: 1px 6px;
    border: 1px solid currentColor;
  }
  .card-running {
    color: var(--term-green, #4FE855);
  }
  .card-inactive {
    color: var(--amber-faint);
  }
  .card-cancelling {
    color: var(--amber-faint);
  }
  .card-cancel {
    background: transparent;
    border: 1px solid var(--term-red);
    color: var(--term-red);
    font-family: inherit;
    font-size: var(--text-lg);
    line-height: 1;
    padding: 0 6px;
    cursor: pointer;
    transition: background var(--duration-base)ease-out, color var(--duration-base)ease-out;
  }
  .card-cancel:hover:not(:disabled) {
    background: var(--term-red);
    color: var(--bg-base);
  }
  .card-cancel:disabled { opacity: 0.4; cursor: not-allowed; }

  .card-meta {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .card-id {
    color: var(--amber-dim);
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 200px;
  }
  .card-sep { color: var(--amber-faint); }
  .card-progress-num {
    color: var(--amber-bright);
    font-weight: 700;
  }
  .card-message {
    color: var(--amber-warm);
    font-size: var(--text-xs);
    background: var(--bg-surface);
    padding: 4px 8px;
    border-left: 2px solid var(--term-purple);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-progress-bar {
    height: 3px;
    background: var(--bg-surface);
    overflow: hidden;
  }
  .card-progress-fill {
    height: 100%;
    background: linear-gradient(to right, var(--term-purple), var(--amber-bright));
    transition: width var(--duration-slow) ease;
  }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 200px;
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
    box-shadow: var(--depth-edge-light);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .archive-clear {
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-family: inherit;
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    cursor: pointer;
    padding: 0;
    transition: color var(--duration-base)ease-out;
  }
  .archive-clear:hover { color: var(--term-red); }
  .state-body {
    padding: 10px 16px 14px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .archive-row {
    display: grid;
    grid-template-columns: 80px 1fr 60px 80px;
    gap: 10px;
    align-items: baseline;
    font-size: var(--text-xs);
    padding: 1px 0;
    transition: background var(--duration-base)ease-out;
  }
  .archive-row:hover { background: rgba(212, 137, 10, 0.06); }
  .archive-status {
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: var(--text-2xs);
  }
  .archive-name {
    color: var(--amber-warm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .archive-dur {
    color: var(--amber-bright);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .archive-when {
    color: var(--amber-faint);
    text-align: right;
    font-style: italic;
  }
  .archive-overflow {
    color: var(--amber-faint);
    font-style: italic;
    font-size: var(--text-2xs);
    padding-top: 4px;
  }

</style>
