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
  import { subscribe, type Category, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, kindToSeverity, type SeverityLevel } from './notifFilter';
  import { clusterEvents, type EventCluster, type SourceLocation } from './errorClustering';
  import { popouts } from './popouts.svelte';

  interface Props {
    title: string;
    icon: string;
    accent?: 'amber' | 'cyan' | 'purple' | 'red';
    /** When set, drives `bus_subscribe`. */
    categoryFilter?: Category;
    /** Minimum severity for events to render. Default: info. */
    severityThreshold?: SeverityLevel;
    /** Phase 3.5a — side-pane only. Renders a draggable handle bar
     *  whose drop on the tab strip demotes the pane back to a tab. */
    onDragBack?: () => void;
  }

  let { title, icon, accent = 'amber', categoryFilter, severityThreshold = 'info', onDragBack }: Props = $props();

  const RECENT_LOG_LIMIT = 100;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  let events = $state<Envelope[]>([]);
  let kindHistogram = $state<Record<string, number>>({});
  let lastTickTs = $state<number>(Date.now());
  let paused = $state(false);
  let unsubscribe: (() => Promise<void>) | undefined;

  // Candidate 0ec — cluster the recent log by structural signature so a
  // repeating error reads as one row + occurrence count instead of a
  // scrolling wall of near-identical lines. Opt-in per pane (off by
  // default so other tabs are unchanged); the Errors tab is the primary
  // user of this toggle.
  let grouped = $state(false);
  let expandedClusters = $state<Set<string>>(new Set());

  // Derived sections
  const liveEvents = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_ACTIVITY_WINDOW_MS;
    return events.filter((e) => e.ts >= cutoff);
  });
  const recentEvents = $derived(events.slice(-RECENT_LOG_LIMIT).reverse());
  // Cluster over the full buffered window (not just the last 100) so the
  // count reflects every folded duplicate; recomputed only while grouped.
  const clusters = $derived.by<EventCluster[]>(() => (grouped ? clusterEvents(events) : []));
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
    if (paused) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    events = [...events, env];
    if (events.length > RECENT_LOG_LIMIT + 20) {
      events = events.slice(-RECENT_LOG_LIMIT);
    }
    bumpHistogram(env.kind);
    lastTickTs = Date.now();
  }

  // Tick lastTickTs every second so the live-activity window slides and
  // "X seconds ago" labels stay accurate without per-event work.
  let tickTimer: ReturnType<typeof setInterval> | undefined;

  // Mount-race guard — Tree.svelte / App.svelte $effect pattern. If the
  // component unmounts before `subscribe()` resolves, the in-flight handle
  // would otherwise leak (`unsubscribe` is still undefined when onDestroy
  // runs). On resolution we check `mounted` and clean up immediately if
  // we lost the race.
  let mounted = true;

  onMount(async () => {
    if (categoryFilter) {
      try {
        const u = await subscribe({ category: categoryFilter }, handleEnvelope);
        if (!mounted) {
          void u().catch(() => {});
        } else {
          unsubscribe = u;
        }
      } catch (err) {
        console.error(`[NotificationPane:${title}] bus_subscribe failed`, err);
      }
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

  /** Pretty-printed payload for the expanded view — wrapped JSON over
   *  multiple lines for legibility + copy-paste. Falls back to formatPayload
   *  when serialization fails. */
  function formatPayloadExpanded(payload: unknown): string {
    if (payload === null || payload === undefined) return '';
    if (typeof payload === 'string') return payload;
    try {
      return JSON.stringify(payload, null, 2);
    } catch {
      return formatPayload(payload);
    }
  }

  /** Phase 8.7q.2 — set of row keys currently expanded. Keyed identically
   *  to the {#each} block so identity matches even after the events array
   *  shifts. Rows toggle on click; expanded rows render full pre-wrapped
   *  payload + are user-selectable for copy-paste. */
  let expandedRows = $state<Set<string>>(new Set());

  function toggleRow(key: string): void {
    const next = new Set(expandedRows);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedRows = next;
  }

  function toggleCluster(key: string): void {
    const next = new Set(expandedClusters);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedClusters = next;
  }

  /** Time-context label for a cluster header — a single timestamp for a
   *  lone event, or the span over which the duplicates arrived. */
  function clusterRangeLabel(c: EventCluster): string {
    if (c.count <= 1) return formatTs(c.lastTs);
    const spanMs = Math.max(0, c.lastTs - c.firstTs);
    if (spanMs < 1000) return `${formatTs(c.lastTs)} · burst`;
    if (spanMs < 60_000) return `over ${Math.round(spanMs / 1000)}s`;
    if (spanMs < 3_600_000) return `over ${Math.round(spanMs / 60_000)}m`;
    return `over ${Math.round(spanMs / 3_600_000)}h`;
  }

  /** Jump-to-source — open the parsed file in the Viewer popout. The Viewer
   *  does not accept a line param yet (same limitation TodoTabContent notes),
   *  so we open the file by path; the line is shown in the label for
   *  reference and follows when Viewer gains line-jump support. */
  function openLocation(loc: SourceLocation): void {
    popouts.summon({ content: { kind: 'viewer', path: loc.file } });
  }

  function clearEvents(): void {
    events = [];
    kindHistogram = {};
    expandedRows = new Set();
    expandedClusters = new Set();
  }

  function kindColor(kind: string): string {
    const k = kind.toLowerCase();
    if (k.includes('error') || k.includes('fail') || k.includes('panic')) return 'var(--term-red)';
    if (k.includes('warn')) return 'var(--amber-primary)';
    if (k.includes('ok') || k.includes('success')) return 'var(--term-green)';
    if (k.startsWith('fs.')) return 'var(--term-cyan)';
    if (k.startsWith('claude.') || k.includes('llm')) return 'var(--term-blue)';
    if (k.startsWith('agent.')) return 'var(--term-purple)';
    if (k.startsWith('aegis.')) return 'var(--amber-primary)';
    if (k.startsWith('hook.')) return 'var(--term-cyan)';
    return 'var(--accent, var(--amber-primary))';
  }

  function severityColor(kind: string): string {
    switch (kindToSeverity(kind)) {
      case 'error': return 'var(--term-red)';
      case 'warn': return 'var(--amber-primary)';
      case 'debug': return 'var(--amber-faint)';
      default: return 'var(--amber-dim)';
    }
  }

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Marker MIME — TabBar.onStripDrop filters by NOTIF_TAB_MIME presence
      // and rejects drags missing it. text/plain alone is silently dropped.
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
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
      aria-label="drag {title} back to tab strip to dock"
    >
      <span class="handle-glyph" style="color: var({accent === 'cyan' ? '--term-cyan' : accent === 'red' ? '--term-red' : accent === 'purple' ? '--term-purple' : '--amber-warm'}); font-size: 14px">{icon}</span>
      <span class="handle-title">{title}</span>
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
    <span class="title"><span class="icon">{icon}</span>{title.toUpperCase()}</span>
    <span class="state">
      {#if categoryFilter}
        {#if grouped}
          {clusters.length} group{clusters.length === 1 ? '' : 's'} · {totalCount} event{totalCount === 1 ? '' : 's'}
        {:else}
          {totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
        {/if}
      {:else}
        idle · 0 events
      {/if}
    </span>
    <span class="spacer"></span>
    {#if categoryFilter}
      <button type="button"
        class="ctrl-btn"
        class:active={grouped}
        onclick={() => (grouped = !grouped)}
        title={grouped ? 'ungroup — show every event' : 'group similar events'}
        aria-label={grouped ? 'Ungroup events' : 'Group similar events'}
      >⧉</button>
      <button type="button"
        class="ctrl-btn"
        class:active={!paused}
        onclick={() => (paused = !paused)}
        title={paused ? 'resume' : 'pause'}
        aria-label={paused ? 'Resume event stream' : 'Pause event stream'}
      >{paused ? '▶' : '⏸'}</button>
      <button type="button"
        class="ctrl-btn"
        onclick={clearEvents}
        title="clear"
        aria-label="Clear events"
      >✕</button>
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
        {#each liveEvents as e, i (e.ts + ':' + e.kind + ':' + i)}
          <span class="strip-event" style="color: {kindColor(e.kind)}; border-color: {kindColor(e.kind)}">{e.kind}</span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="log">
    <div class="log-header">RECENT EVENTS</div>
    <div class="log-body" aria-live="polite">
      {#if !categoryFilter}
        <div class="empty-state">
          <span class="empty-state-icon">◆</span>
          <span class="empty-state-text">Awaiting signals</span>
          <span class="empty-state-hint">events from registered integrations populate this surface</span>
        </div>
      {:else if recentEvents.length === 0}
        <div class="empty-state">
          <span class="empty-state-icon">◇</span>
          <span class="empty-state-text">Subscribed to {categoryFilter}</span>
          <span class="empty-state-hint">no events received yet — they will stream in as activity occurs</span>
        </div>
      {:else if grouped}
        {#each clusters as c (c.key)}
          {@const isOpen = expandedClusters.has(c.key)}
          <div
            class="cluster"
            class:expanded={isOpen}
            role="button"
            tabindex="0"
            onclick={() => toggleCluster(c.key)}
            onkeydown={(ev) => {
              if (ev.key === 'Enter' || ev.key === ' ') {
                ev.preventDefault();
                toggleCluster(c.key);
              }
            }}
            aria-expanded={isOpen}
            title="{c.count} occurrence{c.count === 1 ? '' : 's'} — click to {isOpen ? 'collapse' : 'expand'}"
          >
            <span class="caret" style="color: {severityColor(c.kind)}">{isOpen ? '▼' : '▶'}</span>
            <span class="cluster-count" class:multi={c.count > 1}>×{c.count}</span>
            <span class="kind" style="color: {kindColor(c.kind)}">{c.kind}</span>
            <span class="cluster-sample">{c.sample}</span>
            <span class="cluster-range">{clusterRangeLabel(c)}</span>
            {#if c.location}
              <button
                type="button"
                class="cluster-jump"
                onclick={(ev) => { ev.stopPropagation(); openLocation(c.location!); }}
                title="open {c.location.file} (line {c.location.line})"
                aria-label="Open {c.location.file} at line {c.location.line}"
              >↗ {c.location.file.split(/[\\/]/).pop()}:{c.location.line}</button>
            {/if}
            {#if isOpen}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <div class="cluster-instances" onclick={(ev) => ev.stopPropagation()} onkeydown={(ev) => ev.stopPropagation()} role="list">
                {#each c.instances.slice().reverse() as inst, j (inst.ts + ':' + j)}
                  <div class="cluster-inst" role="listitem">
                    <span class="ts">{formatTs(inst.ts)}</span>
                    <span class="inst-payload">{formatPayload(inst.payload)}</span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      {:else}
        {#each recentEvents as e, i (e.ts + ':' + e.kind + ':' + i)}
          {@const rowKey = e.ts + ':' + e.kind + ':' + i}
          {@const isExpanded = expandedRows.has(rowKey)}
          <div
            class="row"
            class:expanded={isExpanded}
            role="button"
            tabindex="0"
            onclick={(ev) => {
              const target = ev.target as HTMLElement;
              if (target.closest('.payload-expanded')) return;
              toggleRow(rowKey);
            }}
            onkeydown={(ev) => {
              if (ev.key === 'Enter' || ev.key === ' ') {
                ev.preventDefault();
                toggleRow(rowKey);
              }
            }}
            aria-expanded={isExpanded}
            title="click to {isExpanded ? 'collapse' : 'expand'}"
          >
            <span class="caret" style="color: {severityColor(e.kind)}">{isExpanded ? '▼' : '▶'}</span>
            <span class="ts">{formatTs(e.ts)}</span>
            <span class="kind" style="color: {kindColor(e.kind)}">{e.kind}</span>
            {#if isExpanded}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions — stopPropagation prevents parent row toggle during text selection -->
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
              <span class="histo-kind" style="color: {kindColor(k)}">{k}</span>
              <span class="histo-count">{n}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <!--
      Sentinel placeholder (Phase 7.5, capability-driven empty state per §10.7).
      DEFERRED.md D-010 names the unblocking event:
      Sentinel architecture spec lands AND a Sentinel-side implementation
      produces sentinel.* envelopes on a documented schema. Then this card
      becomes a live subscriber.
    -->
    {#if categoryFilter === 'sentinel'}
    <div class="sentinel-card">
      <div class="sentinel-heading">Sentinel</div>
      <div class="sentinel-status">integration not loaded</div>
      <div class="sentinel-subtitle">
        Source-of-truth for agent misbehavior detection (§10.11). Will populate
        when a Sentinel translator self-registers and emits sentinel.* envelopes.
      </div>
    </div>
    {/if}
  </footer>
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    /* Matte panel fill + grain, consistent with the cockpit sidebar. */
    background-color: var(--bg-panel);
    background-image: var(--grain);
    color: var(--amber-primary);
    font-family: var(--font-family);
    font-size: var(--text-base);
    gap: 1px;
    transition: box-shadow var(--duration-med) var(--ease-out);
  }
  /* Active-panel amber edge glow — the focused pane lights up so the eye
     lands on the surface you're actually working in. */
  .pane:focus-within {
    box-shadow: var(--glow-active-inset);
  }

  /* Per-accent border tints — tag accent semantics from §10.1 */
  .pane[data-accent="amber"]  { --accent: var(--amber-primary); --accent-glow: rgba(212, 137, 10, 0.25); --accent-bg: rgba(212, 137, 10, 0.06); }
  .pane[data-accent="cyan"]   { --accent: var(--term-cyan); --accent-glow: rgba(74, 212, 212, 0.25); --accent-bg: rgba(74, 212, 212, 0.06); }
  .pane[data-accent="purple"] { --accent: var(--term-purple); --accent-glow: rgba(176, 120, 232, 0.25); --accent-bg: rgba(176, 120, 232, 0.06); }
  .pane[data-accent="red"]    { --accent: var(--term-red); --accent-glow: rgba(204, 51, 51, 0.25); --accent-bg: rgba(204, 51, 51, 0.06); }

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
  .drag-handle:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .drag-handle:active { cursor: grabbing; background: var(--bg-hover); }
  .drag-handle:hover {
    background: linear-gradient(to bottom, var(--bg-hover), var(--bg-elevated));
    border-bottom-color: var(--accent, var(--amber-primary));
  }
  .drag-handle .handle-glyph {
    color: var(--accent, var(--amber-primary));
    font-size: var(--text-md);
    text-shadow: 0 0 6px var(--accent-glow, rgba(255, 168, 38, 0.35));
  }
  .drag-handle .handle-title {
    color: var(--accent, var(--amber-primary));
    text-transform: uppercase;
    text-shadow: 0 0 4px var(--accent-glow, rgba(255, 168, 38, 0.2));
  }

  .status {
    height: 38px;
    padding: 0 var(--space-lg);
    background: linear-gradient(to bottom, var(--bg-elevated), var(--bg-surface));
    border-left: 3px solid var(--accent, var(--amber-primary));
    border-radius: 0 var(--radius-md, 4px) 0 0;
    box-shadow: var(--sep-glow);
    display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
    flex-shrink: 0;
  }
  .status .title {
    color: var(--accent, var(--amber-primary));
    text-shadow: 0 0 8px var(--accent-glow, rgba(255, 168, 38, 0.35));
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
  .status .spacer { flex: 1; }
  .status .meta {
    color: var(--amber-faint);
    font-weight: 400;
    letter-spacing: 0.04em;
    font-style: italic;
  }
  .strip {
    height: var(--control-lg);
    padding: 0 var(--space-lg);
    box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-14);
    background: linear-gradient(to bottom, var(--accent-bg, rgba(212, 137, 10, 0.06)), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
    position: relative;
    flex-shrink: 0;
  }
  .strip::before {
    content: '';
    position: absolute;
    left: 0; top: 0; bottom: 0;
    width: 3px;
    background: var(--accent, var(--amber-primary));
    opacity: 0.35;
  }
  .strip-label {
    color: var(--accent, var(--amber-primary));
    font-weight: 700;
    text-shadow: 0 0 4px var(--accent-glow, rgba(255, 168, 38, 0.2));
    padding-left: var(--space-sm);
  }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events {
    display: flex; gap: var(--space-sm); flex: 1; overflow: hidden;
  }
  .strip-event {
    padding: 2px var(--space-8);
    border: 1px solid var(--accent, var(--amber-primary));
    color: var(--accent, var(--amber-primary));
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: var(--accent-bg, rgba(212, 137, 10, 0.06));
    box-shadow: 0 0 4px var(--accent-glow, rgba(212, 137, 10, 0.15));
    animation: strip-event-fade 4s ease-out forwards;
  }
  @keyframes strip-event-fade {
    0% { opacity: 1; box-shadow: 0 0 8px var(--accent-glow, rgba(212, 137, 10, 0.3)); }
    70% { opacity: 1; }
    100% { opacity: 0.5; box-shadow: none; }
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .log-header {
    padding: var(--space-sm) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    border-left: 3px solid var(--accent, var(--amber-primary));
    background: linear-gradient(to right, var(--accent-bg, rgba(212, 137, 10, 0.06)), var(--bg-surface));
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: var(--space-12) var(--space-lg);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.55;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); border-radius: var(--radius-sm); }
  .log-body::-webkit-scrollbar-thumb:hover { background: var(--amber-dim); }
  .log-body .row {
    display: grid;
    grid-template-columns: 14px 70px 140px minmax(0, 1fr);
    gap: var(--space-md);
    align-items: baseline;
    padding: var(--space-xs) var(--space-sm);
    white-space: nowrap;
    cursor: pointer;
    user-select: text;
    border-left: 2px solid transparent;
    border-radius: var(--radius-md, 4px);
    transition: background var(--duration-base), border-color var(--duration-base);
  }
  .log-body .row:hover {
    background: rgba(212, 137, 10, 0.06);
    border-left-color: var(--accent, var(--amber-primary));
  }
  .log-body .row.expanded {
    /* Phase 8.7q.2 — expanded view: full payload over multiple lines for
       legibility + copy-paste. The pre-element handles its own wrapping;
       the row drops nowrap so the inline cells (caret/ts/kind) wrap if
       the pane is narrow.
       Phase 8.7q.3 — minmax(0, 1fr) replaces 1fr to allow the kind +
       payload columns to shrink below their intrinsic content width. */
    grid-template-columns: 14px 70px minmax(0, 1fr);
    grid-template-areas:
      "caret ts    kind"
      "pl    pl    pl";
    background: var(--accent-bg, rgba(212, 137, 10, 0.05));
    border-left-color: var(--accent, var(--amber-primary));
    padding: var(--space-xs) var(--space-xs) var(--space-sm);
    white-space: normal;
  }
  .log-body .row.expanded .caret  { grid-area: caret; }
  .log-body .row.expanded .ts     { grid-area: ts; }
  .log-body .row.expanded .kind   { grid-area: kind; }
  .log-body .caret {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    line-height: 1.5;
    user-select: none;
    transition: color var(--duration-base);
  }
  .log-body .row:hover .caret { color: var(--amber-dim); }
  .log-body .row.expanded .caret { color: var(--accent, var(--amber-bright)); }
  .log-body .ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
  }
  .log-body .kind {
    color: var(--accent, var(--amber-primary));
    font-weight: 600;
  }
  .log-body .payload {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .log-body .payload-expanded {
    grid-area: pl;
    margin: var(--space-sm) 0 0 22px;
    padding: var(--space-8) var(--space-md);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-left: 2px solid var(--accent, var(--amber-primary));
    box-shadow: inset 0 2px 6px rgba(0, 0, 0, 0.35), 0 0 4px var(--accent-glow, rgba(212, 137, 10, 0.1));
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: 10.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    border-radius: var(--radius-sm);
    /* Phase 8.7q.3 — explicit min-width: 0 + overflow-x: auto. word-break
       does NOT defeat all unbreakable tokens (long URLs, paths, base64),
       so the <pre> can still demand wider than its grid track. min-width
       lets the track shrink to its container; overflow-x lets long lines
       scroll horizontally inside the box rather than blowing it out. */
    min-width: 0;
    max-height: 320px;
    overflow-x: auto;
    overflow-y: auto;
    user-select: text;
    cursor: text;
  }
  .log-body .payload-expanded::-webkit-scrollbar { width: 5px; }
  .log-body .payload-expanded::-webkit-scrollbar-thumb { background: var(--amber-faint); border-radius: var(--radius-sm); }

  /* Candidate 0ec — clustered event rows (group toggle on). One row per
     structural signature; ×N count badge; expandable to raw instances. */
  .log-body .cluster {
    display: grid;
    grid-template-columns: 14px auto 140px minmax(0, 1fr) auto;
    gap: var(--space-md);
    align-items: baseline;
    padding: var(--space-xs) var(--space-sm);
    cursor: pointer;
    user-select: none;
    border-left: 2px solid transparent;
    border-radius: var(--radius-md, 4px);
    transition: background var(--duration-base), border-color var(--duration-base);
  }
  .log-body .cluster:hover {
    background: rgba(212, 137, 10, 0.06);
    border-left-color: var(--accent, var(--amber-primary));
  }
  .log-body .cluster.expanded {
    background: var(--accent-bg, rgba(212, 137, 10, 0.05));
    border-left-color: var(--accent, var(--amber-primary));
  }
  .log-body .cluster:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .cluster-count {
    font-variant-numeric: tabular-nums;
    font-weight: 700;
    font-size: var(--text-xs);
    color: var(--amber-faint);
    padding: 0 var(--space-sm);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    line-height: 1.5;
    white-space: nowrap;
    text-align: center;
  }
  .cluster-count.multi {
    color: var(--bg-base);
    background: var(--amber-primary);
    border-color: var(--amber-primary);
  }
  .cluster-sample {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .cluster-range {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-style: italic;
    white-space: nowrap;
    justify-self: end;
  }
  .cluster-jump {
    grid-column: 1 / -1;
    justify-self: start;
    margin: 2px 0 0 22px;
    background: transparent;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--term-cyan);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    padding: 1px var(--space-sm);
    cursor: pointer;
    transition: color var(--duration-base), border-color var(--duration-base), background var(--duration-base);
  }
  .cluster-jump:hover {
    color: var(--bg-base);
    background: var(--term-cyan);
    border-color: var(--term-cyan);
  }
  .cluster-jump:focus-visible { outline: 1px solid var(--term-cyan); outline-offset: 1px; }
  .cluster-instances {
    grid-column: 1 / -1;
    margin: var(--space-sm) 0 var(--space-xs) 22px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-left: 1px solid var(--border-subtle);
    padding-left: var(--space-md);
    cursor: default;
  }
  .cluster-inst {
    display: grid;
    grid-template-columns: 70px minmax(0, 1fr);
    gap: var(--space-md);
    font-size: var(--text-2xs);
    align-items: baseline;
  }
  .cluster-inst .ts { color: var(--amber-faint); font-variant-numeric: tabular-nums; }
  .cluster-inst .inst-payload {
    color: var(--amber-dim);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
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
    border-left: 3px solid var(--accent, var(--amber-primary));
    background: linear-gradient(to right, var(--accent-bg, rgba(212, 137, 10, 0.06)), var(--bg-surface));
    box-shadow: var(--sep-depth);
  }
  .state-body {
    padding: var(--space-12) var(--space-lg) var(--space-14);
    display: flex; flex-direction: column; gap: 5px;
  }
  .state-body .k-row {
    display: flex; align-items: center; justify-content: space-between;
    font-size: var(--text-xs); letter-spacing: 0.04em;
    padding: 2px var(--space-xs);
    border-radius: var(--radius-sm);
    transition: background var(--duration-base);
  }
  .state-body .k-row:hover { background: rgba(212, 137, 10, 0.06); }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }

  .histogram {
    margin-top: var(--space-sm);
    padding-top: var(--space-sm);
    display: flex; flex-direction: column; gap: 2px;
  }
  .histo-row {
    display: flex; justify-content: space-between;
    font-size: var(--text-xs);
    padding: 1px var(--space-xs);
    border-radius: var(--radius-sm);
    transition: background var(--duration-base);
  }
  .histo-row:hover { background: rgba(212, 137, 10, 0.06); }
  .histo-kind { color: var(--accent, var(--amber-primary)); }
  .histo-count { color: var(--amber-warm); font-weight: 700; font-variant-numeric: tabular-nums; }

  /* Sentinel placeholder card — capability-driven empty state (§10.7, Phase 7.5) */
  .sentinel-card {
    margin: var(--space-md) var(--space-8) var(--space-8);
    padding: var(--space-12) var(--space-14);
    border: 1px dashed var(--border-subtle);
    border-left: 2px solid var(--amber-faint);
    border-radius: var(--radius-md, 4px);
    box-shadow: var(--depth-inset);
    display: flex;
    flex-direction: column;
    gap: 5px;
    opacity: 0.70;
    transition: opacity var(--duration-med);
  }
  .sentinel-card:hover { opacity: 0.75; }
  .sentinel-heading {
    color: var(--amber-warm);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  .sentinel-status {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    font-style: italic;
    letter-spacing: 0.04em;
  }
  .sentinel-subtitle {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-weight: 400;
    letter-spacing: 0.03em;
    line-height: 1.5;
    opacity: 0.8;
  }

  .ctrl-btn {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    padding: 3px var(--space-md);
    cursor: pointer;
    border-radius: var(--radius-md, 4px);
    line-height: 1;
    font-weight: 500;
    transition: color var(--duration-base), border-color var(--duration-base), box-shadow var(--duration-base),
                background var(--duration-base);
  }
  .ctrl-btn:hover {
    color: var(--amber-bright);
    border-color: var(--amber-primary);
    background: var(--bg-hover);
    box-shadow: 0 0 6px rgba(212, 137, 10, 0.2);
  }
  .ctrl-btn:active {
    background: rgba(255, 168, 38, 0.1);
  }
  .ctrl-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .ctrl-btn.active {
    color: var(--term-green);
    border-color: var(--term-green);
    background: rgba(79, 232, 85, 0.06);
    box-shadow: 0 0 4px rgba(51, 204, 51, 0.2);
  }
</style>
