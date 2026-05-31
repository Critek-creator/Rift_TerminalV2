<script lang="ts">
  // Candidate 2aa — Integration Capability Inspector.
  //
  // Rift's §9 design deliberately suppresses "integration not found" errors,
  // which is right for users but leaves operators unable to tell a
  // never-loaded integration from a silently-dropped one. This tab is the
  // affirmative answer to "what is actually connected right now?": it
  // subscribes to every bus category and, per source, shows a live/idle/stale
  // state, last-envelope heartbeat, a 60-second event rate, and the §9
  // capability class the source exercises.
  //
  // Pure read-model over the bus — no new protocol, no translator changes. It
  // is the boundary-facing complement to the health tab (which reports core
  // internals): "is the outside world still talking to me" vs "is my core ok".

  import { onMount, onDestroy } from 'svelte';
  import { subscribe, type Category, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';

  interface Props {
    onDragBack?: () => void;
  }
  let { onDragBack }: Props = $props();

  type Tier = 'core' | 'integration';
  interface SourceDef {
    category: Category;
    label: string;
    tier: Tier;
    /** §9 capability class this source exercises. */
    capability: string;
  }

  // Ordered catalog of bus sources. Core lanes are always-on (present in every
  // install); integration-provided lanes light up only when their translator
  // self-registers and emits. Capability classes per RIFT_V2_VISION §9.
  const SOURCES: SourceDef[] = [
    { category: 'pty',      label: 'PTY',           tier: 'core',        capability: 'core stream' },
    { category: 'fs',       label: 'Filesystem',    tier: 'core',        capability: 'core stream' },
    { category: 'status',   label: 'Status line',   tier: 'core',        capability: 'core stream' },
    { category: 'system',   label: 'System',        tier: 'core',        capability: 'core stream' },
    { category: 'hook',     label: 'Hooks',         tier: 'integration', capability: 'event subscription' },
    { category: 'agent',    label: 'Agents',        tier: 'integration', capability: 'event subscription' },
    { category: 'aegis',    label: 'Aegis',         tier: 'integration', capability: 'event subscription' },
    { category: 'sentinel', label: 'Sentinel',      tier: 'integration', capability: 'event subscription' },
    { category: 'index',    label: 'Abyssal Index', tier: 'integration', capability: 'data enrichment' },
    { category: 'mcp',      label: 'MCP server',    tier: 'integration', capability: 'control endpoints' },
    { category: 'llm',      label: 'LLM router',    tier: 'integration', capability: 'event subscription' },
  ];

  type LiveState = 'live' | 'idle' | 'stale' | 'never';
  const LIVE_MS = 10_000;   // seen within 10s → live
  const IDLE_MS = 300_000;  // seen within 5m → idle; older → stale
  const RATE_WINDOW_MS = 60_000;

  interface Snap {
    count: number;
    lastTs: number | null;
    /** Events seen in the last 60s. */
    rate: number;
  }

  // Non-reactive accumulator mutated on every envelope; snapshotted into
  // reactive state once per second so render cost is decoupled from the
  // (bursty) event rate — the same decoupling NotificationPane uses for its
  // "X seconds ago" labels.
  const acc = new Map<string, { count: number; lastTs: number; recent: number[] }>();
  let snap = $state<Record<string, Snap>>({});
  let now = $state<number>(Date.now());

  let unsubscribe: (() => Promise<void>) | undefined;
  let tickTimer: ReturnType<typeof setInterval> | undefined;
  let mounted = true;

  function handleEnvelope(env: Envelope): void {
    let e = acc.get(env.category);
    if (!e) {
      e = { count: 0, lastTs: 0, recent: [] };
      acc.set(env.category, e);
    }
    e.count += 1;
    e.lastTs = env.ts;
    e.recent.push(env.ts);
  }

  function tick(): void {
    const t = Date.now();
    now = t;
    const cutoff = t - RATE_WINDOW_MS;
    const next: Record<string, Snap> = {};
    for (const s of SOURCES) {
      const e = acc.get(s.category);
      if (e) {
        e.recent = e.recent.filter((ts) => ts >= cutoff);
        next[s.category] = { count: e.count, lastTs: e.lastTs || null, rate: e.recent.length };
      } else {
        next[s.category] = { count: 0, lastTs: null, rate: 0 };
      }
    }
    snap = next;
  }

  function stateOf(s: SourceDef): LiveState {
    const info = snap[s.category];
    if (!info || info.lastTs === null) return 'never';
    const age = now - info.lastTs;
    if (age < LIVE_MS) return 'live';
    if (age < IDLE_MS) return 'idle';
    return 'stale';
  }

  function ageLabel(s: SourceDef): string {
    const info = snap[s.category];
    if (!info || info.lastTs === null) return '—';
    const age = Math.max(0, now - info.lastTs);
    if (age < 1000) return 'just now';
    if (age < 60_000) return `${Math.floor(age / 1000)}s ago`;
    if (age < 3_600_000) return `${Math.floor(age / 60_000)}m ago`;
    return `${Math.floor(age / 3_600_000)}h ago`;
  }

  const STATE_LABEL: Record<LiveState, string> = {
    live: 'live',
    idle: 'idle',
    stale: 'stale',
    never: 'not connected',
  };

  // Summary counts for the persistent state panel.
  const connectedIntegrations = $derived(
    SOURCES.filter((s) => s.tier === 'integration' && stateOf(s) !== 'never').length,
  );
  const totalIntegrations = $derived(SOURCES.filter((s) => s.tier === 'integration').length);
  const liveCount = $derived(SOURCES.filter((s) => stateOf(s) === 'live').length);

  onMount(async () => {
    tick();
    try {
      const u = await subscribe({}, handleEnvelope);
      if (!mounted) void u().catch(() => {});
      else unsubscribe = u;
    } catch (err) {
      console.error('[IntegrationInspector] bus_subscribe failed', err);
    }
    tickTimer = setInterval(tick, 1000);
  });

  onDestroy(() => {
    mounted = false;
    if (tickTimer) clearInterval(tickTimer);
    unsubscribe?.().catch(() => {});
  });

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
      aria-label="drag integrations back to tab strip to dock"
    >
      <span class="handle-glyph">⇄</span>
      <span class="handle-title">integrations</span>
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

  <header class="status">
    <span class="title"><span class="icon">⇄</span>INTEGRATIONS</span>
    <span class="state">{connectedIntegrations}/{totalIntegrations} connected · {liveCount} live</span>
  </header>

  <div class="log">
    <div class="log-header">CONNECTION MAP</div>
    <div class="log-body">
      <div class="ins-row ins-head" aria-hidden="true">
        <span class="ins-dot"></span>
        <span class="ins-label">source</span>
        <span class="ins-cap">capability (§9)</span>
        <span class="ins-rate">rate</span>
        <span class="ins-seen">last seen</span>
      </div>
      {#each SOURCES as s (s.category)}
        {@const st = stateOf(s)}
        {@const info = snap[s.category]}
        <div class="ins-row" data-state={st}>
          <span class="ins-dot" data-state={st} title={STATE_LABEL[st]}></span>
          <span class="ins-label">
            {s.label}
            <span class="ins-tier">{s.tier === 'core' ? 'core' : 'integration'}</span>
          </span>
          <span class="ins-cap">{s.capability}</span>
          <span class="ins-rate">{info && info.rate > 0 ? `${info.rate}/min` : '—'}</span>
          <span class="ins-seen">{ageLabel(s)}</span>
        </div>
      {/each}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">SUMMARY</div>
    <div class="state-body">
      <div class="k-row"><span class="k">integrations connected</span><span class="v">{connectedIntegrations}/{totalIntegrations}</span></div>
      <div class="k-row"><span class="k">live now</span><span class="v">{liveCount}</span></div>
      <div class="k-row legend">
        <span class="lg"><span class="ins-dot" data-state="live"></span>live</span>
        <span class="lg"><span class="ins-dot" data-state="idle"></span>idle</span>
        <span class="lg"><span class="ins-dot" data-state="stale"></span>stale</span>
        <span class="lg"><span class="ins-dot" data-state="never"></span>not connected</span>
      </div>
    </div>
  </footer>
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background-color: var(--bg-panel);
    background-image: var(--grain);
    color: var(--amber-primary);
    font-family: var(--font-family);
    font-size: var(--text-base);
    gap: 1px;
  }

  .drag-handle {
    height: 28px;
    padding: 0 var(--space-14);
    background: linear-gradient(to bottom, var(--bg-elevated), var(--bg-surface));
    box-shadow: var(--sep-glow);
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
  .drag-handle:active { cursor: grabbing; background: var(--bg-hover); }
  .drag-handle:hover { background: linear-gradient(to bottom, var(--bg-hover), var(--bg-elevated)); }
  .drag-handle:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -2px; }
  .drag-handle .handle-glyph { color: var(--amber-bright); font-size: var(--text-md); text-shadow: var(--glow-amber-faint); }
  .drag-handle .handle-title { color: var(--amber-bright); text-transform: uppercase; }

  .status {
    height: 38px;
    padding: 0 var(--space-lg);
    background: linear-gradient(to bottom, var(--bg-elevated), var(--bg-surface));
    border-left: 3px solid var(--amber-primary);
    box-shadow: var(--sep-glow);
    display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
    flex-shrink: 0;
  }
  .status .title {
    color: var(--amber-primary);
    text-shadow: var(--glow-amber-faint);
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.9; }
  .status .state {
    color: var(--amber-dim);
    font-size: var(--type-caption-size);
    font-weight: var(--type-caption-weight);
    letter-spacing: var(--type-caption-spacing);
  }

  .log { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .log-header {
    padding: var(--space-sm) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    border-left: 3px solid var(--amber-primary);
    background: linear-gradient(to right, rgba(212, 137, 10, 0.06), var(--bg-surface));
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: var(--space-8) var(--space-lg);
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); border-radius: var(--radius-sm); }

  .ins-row {
    display: grid;
    grid-template-columns: 14px minmax(0, 1.4fr) minmax(0, 1.3fr) 70px 80px;
    gap: var(--space-md);
    align-items: center;
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-md, 4px);
    transition: background var(--duration-base);
  }
  .ins-row:not(.ins-head):hover { background: rgba(212, 137, 10, 0.06); }
  .ins-head {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .ins-row[data-state="never"] { opacity: 0.55; }

  .ins-dot {
    width: 8px; height: 8px; border-radius: 50%;
    background: var(--amber-faint);
    justify-self: center;
  }
  .ins-dot[data-state="live"]  { background: var(--term-green); box-shadow: 0 0 6px var(--term-green); }
  .ins-dot[data-state="idle"]  { background: var(--amber-bright); box-shadow: 0 0 4px var(--amber-bright); }
  .ins-dot[data-state="stale"] { background: var(--amber-faint); }
  .ins-dot[data-state="never"] { background: transparent; border: 1px solid var(--amber-faint); }

  .ins-label {
    color: var(--amber-warm);
    font-size: var(--text-sm);
    font-weight: 600;
    display: flex; align-items: baseline; gap: var(--space-sm);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
  }
  .ins-tier {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-weight: 400;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .ins-cap {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
  }
  .ins-rate {
    color: var(--term-cyan);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .ins-seen {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 200px;
    overflow-y: auto;
    box-shadow: var(--depth-lift), var(--depth-edge-light);
  }
  .state-header {
    padding: var(--space-sm) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    border-left: 3px solid var(--amber-primary);
    background: linear-gradient(to right, rgba(212, 137, 10, 0.06), var(--bg-surface));
    box-shadow: var(--sep-depth);
  }
  .state-body { padding: var(--space-12) var(--space-lg) var(--space-14); display: flex; flex-direction: column; gap: 5px; }
  .k-row {
    display: flex; align-items: center; justify-content: space-between;
    font-size: var(--text-xs); letter-spacing: 0.04em;
    padding: 2px var(--space-xs);
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }
  .k-row.legend { justify-content: flex-start; gap: var(--space-14); flex-wrap: wrap; margin-top: var(--space-sm); }
  .legend .lg { display: flex; align-items: center; gap: var(--space-xs); color: var(--amber-faint); font-size: var(--text-2xs); }
</style>
