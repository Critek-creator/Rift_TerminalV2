<script lang="ts">
  // Phase 8.7i — Bus tail tab. Firehose view of every category — the
  // dev-self-help surface that would have made Issue 2's debugging trivial.
  //
  // Subscribes with no category filter so every envelope passes through.
  // Each row is color-tagged by category (matches §10.1 lane palette) so
  // a glance tells you which subsystem fired.
  //
  // Controls:
  //   - pause toggle    — freeze the tail without unsubscribing
  //   - clear           — drop the buffer
  //   - category filter — checkboxes to mute noisy categories
  //
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: $effect cleanup must be
  // sync; bus unsubscribe wraps in IIFE.

  import { onMount, onDestroy } from 'svelte';
  import { subscribe, type Category, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, type SeverityLevel } from './notifFilter';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
  }

  let { severityThreshold = 'debug', onDragBack }: Props = $props();

  const RECENT_LOG_LIMIT = 200;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  const ALL_CATEGORIES: Category[] = [
    'pty', 'hook', 'agent', 'fs', 'index', 'aegis', 'status', 'system', 'mcp',
  ];

  // §10.1 lane palette → category accent colour
  const CAT_COLOR: Record<Category, string> = {
    pty:    'var(--term-white)',
    hook:   'var(--term-cyan)',
    agent:  'var(--term-purple)',
    fs:     'var(--amber-faint)',
    index:  'var(--status-blue-bright, #6CB6FF)',
    aegis:  'var(--amber-primary)',
    status: 'var(--amber-bright)',
    system: 'var(--term-red)',
    // D-014: MCP traffic — neutral lane to keep audit trail visible without
    // colliding with hook (cyan) or aegis (amber).
    mcp:    'var(--term-purple, #C58FFF)',
  };

  let connected = $state(false);
  let error = $state('');
  let events = $state<Envelope[]>([]);
  let paused = $state(false);
  let mutedCats = $state<Set<Category>>(new Set());
  let lastTickTs = $state<number>(Date.now());
  let unsubscribe: (() => Promise<void>) | undefined;
  let mounted = true;

  const visibleEvents = $derived(
    events.filter((e) => !mutedCats.has(e.category))
  );
  const recentEvents = $derived(visibleEvents.slice(-RECENT_LOG_LIMIT).reverse());
  const liveEvents = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_ACTIVITY_WINDOW_MS;
    return visibleEvents.filter((e) => e.ts >= cutoff);
  });
  const totalCount = $derived(events.length);
  const visibleCount = $derived(visibleEvents.length);
  const lastSeenLabel = $derived.by(() => {
    if (events.length === 0) return '—';
    const last = events[events.length - 1];
    const ageMs = Math.max(0, lastTickTs - last.ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  });

  // Per-category counts for the persistent state panel
  const catHistogram = $derived.by(() => {
    const h: Partial<Record<Category, number>> = {};
    for (const e of events) h[e.category] = (h[e.category] ?? 0) + 1;
    return h;
  });

  function handleEnvelope(env: Envelope) {
    if (paused) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    events = [...events, env];
    if (events.length > RECENT_LOG_LIMIT * 2) {
      events = events.slice(-RECENT_LOG_LIMIT);
    }
    lastTickTs = Date.now();
  }

  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    try {
      const u = await subscribe({}, handleEnvelope);
      if (!mounted) {
        void u().catch(() => {});
      } else {
        unsubscribe = u;
        connected = true;
      }
    } catch (err) {
      console.error('[BusTail] bus_subscribe failed', err);
      error = (err as Error).message || 'Connection failed';
    }
    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
    }, 1000);
  });

  onDestroy(() => {
    mounted = false;
    if (tickTimer) clearInterval(tickTimer);
    unsubscribe?.().catch(() => {});
  });

  function togglePause() {
    paused = !paused;
  }
  function clearLog() {
    events = [];
  }
  function toggleCat(cat: Category) {
    const next = new Set(mutedCats);
    if (next.has(cat)) next.delete(cat);
    else next.add(cat);
    mutedCats = next;
  }

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }
  function formatPayload(payload: unknown): string {
    if (payload === null || payload === undefined) return '';
    if (typeof payload === 'string') return payload;
    try { return JSON.stringify(payload); } catch { return String(payload); }
  }
  function formatPayloadExpanded(payload: unknown): string {
    if (payload === null || payload === undefined) return '';
    if (typeof payload === 'string') return payload;
    try { return JSON.stringify(payload, null, 2); } catch { return String(payload); }
  }

  // Phase 8.7q.4 — click-to-expand row pattern (mirrors NotificationPane).
  let expandedRows = $state<Set<string>>(new Set());
  function toggleRow(key: string): void {
    const next = new Set(expandedRows);
    if (next.has(key)) next.delete(key); else next.add(key);
    expandedRows = next;
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
      <span class="handle-title">bus tail</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if error}
    <div class="error-state">⚠ Bus connection failed: {error}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {/if}

  <header class="status">
    <span class="title"><span class="icon">⌁</span>BUS TAIL</span>
    <span class="state">
      {visibleCount}/{totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
    </span>
    <span class="spacer"></span>
    <button type="button" class="ctl-btn" class:active={paused} onclick={togglePause}>
      {paused ? 'paused' : 'live'}
    </button>
    <button type="button" class="ctl-btn" onclick={clearLog} disabled={events.length === 0}>
      clear
    </button>
  </header>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if liveEvents.length === 0}
      <span class="strip-empty">(no in-flight events)</span>
    {:else}
      <div class="strip-events">
        {#each liveEvents as e, i (e.ts + ':' + e.category + ':' + e.kind + ':' + i)}
          <span class="strip-event" style="--cat-color: {CAT_COLOR[e.category]};">
            <span class="strip-cat">{e.category}</span>
            <span class="strip-kind">{e.kind}</span>
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
          {#if paused}
            <div class="empty-title">Paused</div>
            <div class="empty-desc">Click LIVE to resume the event stream.</div>
          {:else}
            <div class="empty-title">No events received yet</div>
            <div class="empty-desc">This firehose subscribes to all bus categories. Events appear as integrations publish to the Rift event bus.</div>
          {/if}
        </div>
      {:else}
        {#each recentEvents as e, i (e.ts + ':' + e.category + ':' + e.kind + ':' + i)}
          {@const rowKey = e.ts + ':' + e.category + ':' + e.kind + ':' + i}
          {@const isExpanded = expandedRows.has(rowKey)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="row"
            class:expanded={isExpanded}
            onclick={(ev) => {
              const target = ev.target as HTMLElement;
              if (target.closest('.payload-expanded')) return;
              toggleRow(rowKey);
            }}
            title="click to {isExpanded ? 'collapse' : 'expand'}"
          >
            <span class="caret">{isExpanded ? '▼' : '▶'}</span>
            <span class="ts">{formatTs(e.ts)}</span>
            <span class="cat" style="color: {CAT_COLOR[e.category]};">{e.category}</span>
            <span class="kind">{e.kind}</span>
            {#if isExpanded}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <pre
                class="payload-expanded"
                onmousedown={(ev) => ev.stopPropagation()}
              >{formatPayloadExpanded(e.payload)}</pre>
            {:else}
              <span class="payload">{formatPayload(e.payload)}</span>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">CATEGORY FILTER</div>
    <div class="state-body">
      <div class="cat-grid">
        {#each ALL_CATEGORIES as cat (cat)}
          {@const count = catHistogram[cat] ?? 0}
          {@const muted = mutedCats.has(cat)}
          <label class="cat-row" class:muted>
            <input
              type="checkbox"
              checked={!muted}
              onchange={() => toggleCat(cat)}
            />
            <span class="cat-dot" style="background: {CAT_COLOR[cat]};"></span>
            <span class="cat-name">{cat}</span>
            <span class="cat-count">{count}</span>
          </label>
        {/each}
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
    /* Phase 8.7q.3 — see NotificationPane same-named comment. */
    min-width: 0;
    background: var(--bg-base);
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
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
    color: var(--amber-bright);
    font-size: 12px;
    text-shadow: var(--glow-amber-faint);
  }
  .drag-handle .handle-title {
    color: var(--amber-bright);
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
  .status .title {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .state { color: var(--amber-dim); font-weight: 400; letter-spacing: 0.04em; }
  .status .spacer { flex: 1; }
  .ctl-btn {
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
    transition: color 0.12s ease-out, background 0.12s ease-out, border-color 0.12s ease-out, opacity 0.12s ease-out;
  }
  .ctl-btn:hover:not(:disabled) {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .ctl-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .ctl-btn.active {
    background: var(--term-red);
    border-color: var(--term-red);
    color: var(--bg-base);
  }

  .strip {
    height: 26px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
    display: flex; align-items: center; gap: 14px;
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.05), transparent);
    color: var(--amber-dim);
    font-size: 10px;
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--amber-bright); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events {
    display: flex; gap: 6px; flex: 1; overflow: hidden;
  }
  .strip-event {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border: 1px solid var(--cat-color);
    color: var(--cat-color);
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(212, 137, 10, 0.04);
  }
  .strip-cat { opacity: 0.7; }
  .strip-kind { font-weight: 700; }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    min-width: 0;
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
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: 8px 14px;
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }
  .connecting-state {
    color: var(--amber-faint);
    padding: 1rem 14px;
    font-style: italic;
    font-size: 11px;
    letter-spacing: 0.04em;
  }
  .error-state {
    color: var(--term-red);
    padding: 12px 14px;
    font-size: 11px;
    letter-spacing: 0.04em;
    border-bottom: 1px solid rgba(255, 72, 72, 0.2);
    background: rgba(255, 72, 72, 0.04);
  }
  .empty-card {
    border: 1px dashed var(--border-subtle);
    padding: 12px 14px;
    background: rgba(212, 137, 10, 0.03);
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.55;
  }
  .empty-title {
    color: var(--amber-bright);
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
    grid-template-columns: 14px 70px 60px 140px minmax(0, 1fr);
    gap: 8px;
    align-items: baseline;
    padding: 1px 0;
    white-space: nowrap;
    cursor: pointer;
    user-select: text;
    transition: background 0.12s ease-out;
  }
  .log-body .row:hover { background: rgba(212, 137, 10, 0.04); }
  .log-body .row.expanded {
    grid-template-columns: 14px 70px 60px minmax(0, 1fr);
    grid-template-areas:
      "caret ts    cat   kind"
      "pl    pl    pl    pl";
    background: rgba(212, 137, 10, 0.05);
    padding: 4px 0 6px;
    white-space: normal;
  }
  .log-body .row.expanded .caret { grid-area: caret; color: var(--amber-bright); }
  .log-body .row.expanded .ts    { grid-area: ts; }
  .log-body .row.expanded .cat   { grid-area: cat; }
  .log-body .row.expanded .kind  { grid-area: kind; }
  .log-body .caret {
    color: var(--amber-faint);
    font-size: 9px;
    line-height: 1.5;
    user-select: none;
  }
  .log-body .ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: 10px;
  }
  .log-body .cat {
    font-weight: 700;
    font-size: 9px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .log-body .kind {
    color: var(--amber-warm);
    font-weight: 600;
  }
  .log-body .payload {
    color: var(--amber-dim);
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .log-body .payload-expanded {
    grid-area: pl;
    margin: 4px 0 0 22px;
    padding: 6px 8px;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 10.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    min-width: 0;
    max-height: 320px;
    overflow-x: auto;
    overflow-y: auto;
    user-select: text;
    cursor: text;
  }
  .log-body .payload-expanded::-webkit-scrollbar { width: 5px; }
  .log-body .payload-expanded::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    border-top: 1px solid var(--border-subtle);
    box-shadow: var(--depth-lift), var(--depth-edge-light);
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
    box-shadow: var(--depth-edge-light);
  }
  .state-body {
    padding: 8px 14px 12px;
  }
  .cat-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 4px 16px;
  }
  .cat-row {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    user-select: none;
    font-size: 10px;
    letter-spacing: 0.04em;
    transition: opacity 0.12s ease-out;
  }
  .cat-row.muted .cat-name,
  .cat-row.muted .cat-count {
    color: var(--amber-faint);
    text-decoration: line-through;
  }
  .cat-row input[type="checkbox"] {
    width: 12px;
    height: 12px;
    accent-color: var(--amber-bright);
    cursor: pointer;
  }
  .cat-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .cat-name {
    flex: 1;
    color: var(--amber-warm);
    text-transform: uppercase;
    font-weight: 600;
  }
  .cat-count {
    color: var(--amber-dim);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
  }
</style>
