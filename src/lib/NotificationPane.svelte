<script lang="ts">
  // §10.4 + §10.8 — notification tab anatomy. Four modular sections:
  //   1. Status header        (top — single line summarising state)
  //   2. Live activity strip  (animated row of in-flight events)
  //   3. Recent events log    (scrollable history)
  //   4. Persistent state     (config / counters / pinned items)
  //
  // Phase 5: when `categoryFilter` is set, this pane subscribes to the
  // RiftBus through `bus_subscribe` and renders real envelopes through
  // the four sections. Without a filter, it falls back to the empty
  // chassis (Phase 3 behaviour) so tabs without registered translators
  // remain inert.
  //
  // Phase 3.5a: when `onDragBack` is provided (the side-pane instance),
  // a small drag-handle bar renders above the status header. Dragging
  // it back onto the tab strip triggers demote in the parent.

  import { onMount, onDestroy } from 'svelte';
  import { subscribe, publish, type Category, type Envelope } from './bus';

  interface Props {
    title: string;
    icon: string;
    accent?: 'amber' | 'cyan' | 'purple' | 'red';
    /** When set, drives `bus_subscribe`. */
    categoryFilter?: Category;
    /** Phase 3.5a — side-pane only. Renders a draggable handle bar
     *  whose drop on the tab strip demotes the pane back to a tab. */
    onDragBack?: () => void;
  }

  let { title, icon, accent = 'amber', categoryFilter, onDragBack }: Props = $props();

  const RECENT_LOG_LIMIT = 100;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  let events = $state<Envelope[]>([]);
  let kindHistogram = $state<Record<string, number>>({});
  let lastTickTs = $state<number>(Date.now());
  let unsubscribe: (() => Promise<void>) | undefined;

  // Derived sections
  const liveEvents = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_ACTIVITY_WINDOW_MS;
    return events.filter((e) => e.ts >= cutoff);
  });
  const recentEvents = $derived(events.slice(-RECENT_LOG_LIMIT).reverse());
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

  function bumpHistogram(kind: string) {
    kindHistogram = { ...kindHistogram, [kind]: (kindHistogram[kind] ?? 0) + 1 };
  }

  function handleEnvelope(env: Envelope) {
    events = [...events, env];
    if (events.length > RECENT_LOG_LIMIT * 2) {
      // Trim the underlying buffer so we never grow unbounded.
      events = events.slice(-RECENT_LOG_LIMIT);
    }
    bumpHistogram(env.kind);
    lastTickTs = Date.now();
  }

  // Tick lastTickTs every second so the live-activity window slides and
  // "X seconds ago" labels stay accurate without per-event work.
  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    if (categoryFilter) {
      try {
        unsubscribe = await subscribe({ category: categoryFilter }, handleEnvelope);
      } catch (err) {
        console.error(`[NotificationPane:${title}] bus_subscribe failed`, err);
      }
    }
    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
    }, 1000);
  });

  onDestroy(() => {
    if (tickTimer) clearInterval(tickTimer);
    unsubscribe?.().catch(() => {});
  });

  async function publishDemo() {
    if (!categoryFilter) return;
    try {
      await publish(categoryFilter, 'demo.click', {
        source: 'notification-pane-demo',
        clientTime: new Date().toISOString(),
      });
    } catch (err) {
      console.error('[NotificationPane] bus_publish failed', err);
    }
  }

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  function formatPayload(payload: unknown): string {
    if (payload === null || payload === undefined) return '';
    if (typeof payload === 'string') return payload;
    try {
      return JSON.stringify(payload);
    } catch {
      return String(payload);
    }
  }

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Sentinel payload — the tab strip's drop handler doesn't read it,
      // it just calls onDemote(), but we still need data set for the
      // drag to be considered valid by the platform.
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }
</script>

<section class="pane" data-accent={accent}>
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
      <span class="handle-title">{title}</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  <header class="status">
    <span class="title"><span class="icon">{icon}</span>{title.toUpperCase()}</span>
    <span class="state">
      {#if categoryFilter}
        {totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
      {:else}
        idle · 0 events
      {/if}
    </span>
    <span class="spacer"></span>
    {#if categoryFilter}
      <button type="button" class="demo-btn" onclick={publishDemo}>
        publish demo {categoryFilter}
      </button>
      <span class="meta">subscribed: <span class="cat">{categoryFilter}</span></span>
    {:else}
      <span class="meta">no integration registered — populates Phase 5+</span>
    {/if}
  </header>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if !categoryFilter || liveEvents.length === 0}
      <span class="strip-empty">(no in-flight events)</span>
    {:else}
      <div class="strip-events">
        {#each liveEvents as e (e.ts + e.kind + Math.random())}
          <span class="strip-event">{e.kind}</span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="log">
    <div class="log-header">RECENT EVENTS</div>
    <div class="log-body">
      {#if !categoryFilter}
        <div class="empty">no events yet — this surface populates from registered integrations</div>
      {:else if recentEvents.length === 0}
        <div class="empty">subscribed to <span class="cat">{categoryFilter}</span> — no events received yet</div>
      {:else}
        {#each recentEvents as e (e.ts + e.kind)}
          <div class="row">
            <span class="ts">{formatTs(e.ts)}</span>
            <span class="kind">{e.kind}</span>
            <span class="payload">{formatPayload(e.payload)}</span>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">PERSISTENT STATE</div>
    <div class="state-body">
      <div class="row k-row"><span class="k">subscribed</span><span class="v">{categoryFilter ?? 'none'}</span></div>
      <div class="row k-row"><span class="k">total events</span><span class="v">{totalCount}</span></div>
      <div class="row k-row"><span class="k">distinct kinds</span><span class="v">{Object.keys(kindHistogram).length}</span></div>
      <div class="row k-row"><span class="k">last seen</span><span class="v">{lastSeenLabel}</span></div>
      {#if Object.keys(kindHistogram).length > 0}
        <div class="histogram">
          {#each Object.entries(kindHistogram).sort(([, a], [, b]) => b - a).slice(0, 6) as [k, n] (k)}
            <div class="histo-row">
              <span class="histo-kind">{k}</span>
              <span class="histo-count">{n}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </footer>
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    color: var(--amber-primary);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
  }

  /* Per-accent border tints — tag accent semantics from §10.1 */
  .pane[data-accent="amber"]  { --accent: var(--amber-primary); }
  .pane[data-accent="cyan"]   { --accent: var(--term-cyan); }
  .pane[data-accent="purple"] { --accent: var(--term-purple); }
  .pane[data-accent="red"]    { --accent: var(--term-red); }

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
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--accent, var(--amber-primary));
    font-size: 12px;
    text-shadow: var(--glow-amber-faint);
  }
  .drag-handle .handle-title {
    color: var(--accent, var(--amber-primary));
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
    display: flex; align-items: center; gap: 14px;
    color: var(--amber-warm);
    font-size: 11px; letter-spacing: 0.1em; font-weight: 700;
  }
  .status .title {
    color: var(--accent, var(--amber-primary));
    text-shadow: var(--glow-amber-faint);
  }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .state { color: var(--amber-dim); font-weight: 400; letter-spacing: 0.04em; }
  .status .spacer { flex: 1; }
  .status .meta {
    color: var(--amber-faint);
    font-weight: 400;
    letter-spacing: 0.04em;
    font-style: italic;
  }
  .status .meta .cat {
    color: var(--accent, var(--amber-primary));
    font-style: normal;
    font-weight: 600;
  }
  .demo-btn {
    background: transparent;
    border: 1px solid var(--accent, var(--amber-primary));
    color: var(--accent, var(--amber-primary));
    font-family: inherit;
    font-size: 9px;
    letter-spacing: 0.1em;
    font-weight: 700;
    padding: 2px 8px;
    cursor: pointer;
  }
  .demo-btn:hover {
    background: rgba(212, 137, 10, 0.08);
    text-shadow: var(--glow-amber-faint);
  }

  .strip {
    height: 26px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-subtle);
    display: flex; align-items: center; gap: 14px;
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.04), transparent);
    color: var(--amber-dim);
    font-size: 10px;
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--accent, var(--amber-primary)); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events {
    display: flex; gap: 6px; flex: 1; overflow: hidden;
  }
  .strip-event {
    padding: 1px 6px;
    border: 1px solid var(--accent, var(--amber-primary));
    color: var(--accent, var(--amber-primary));
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(212, 137, 10, 0.04);
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    border-bottom: 1px solid var(--border-subtle);
  }
  .log-header {
    padding: 6px 14px;
    color: var(--amber-warm);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 14px;
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.5;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }
  .empty {
    color: var(--amber-faint);
    font-style: italic;
  }

  .log-body .row {
    display: grid;
    grid-template-columns: 70px 140px 1fr;
    gap: 12px;
    align-items: baseline;
    padding: 1px 0;
    white-space: nowrap;
  }
  .log-body .row:hover { background: rgba(212, 137, 10, 0.04); }
  .log-body .ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: 10px;
  }
  .log-body .kind {
    color: var(--accent, var(--amber-primary));
    font-weight: 600;
  }
  .log-body .payload {
    color: var(--amber-dim);
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 180px;
    overflow-y: auto;
  }
  .state-header {
    padding: 6px 14px;
    color: var(--amber-warm);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    border-bottom: 1px solid var(--border-subtle);
  }
  .state-body {
    padding: 8px 14px 12px;
    display: flex; flex-direction: column; gap: 4px;
  }
  .state-body .k-row {
    display: flex; align-items: center; justify-content: space-between;
    font-size: 10px; letter-spacing: 0.04em;
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }

  .histogram {
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border-subtle);
    display: flex; flex-direction: column; gap: 2px;
  }
  .histo-row {
    display: flex; justify-content: space-between;
    font-size: 10px;
  }
  .histo-kind { color: var(--accent, var(--amber-primary)); }
  .histo-count { color: var(--amber-warm); font-weight: 700; font-variant-numeric: tabular-nums; }
</style>
