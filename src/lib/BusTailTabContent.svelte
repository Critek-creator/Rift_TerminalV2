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
  import { SparklineBuffer } from './SparklineBuffer';
  import SparklineChart from './SparklineChart.svelte';
  import { HeatstripBuffer } from './HeatstripBuffer';
  import HeatstripTimeline from './HeatstripTimeline.svelte';
  import { kindToSeverity } from './notifFilter';
  import CorrelationBadge from './CorrelationBadge.svelte';
  import type { CorrelationIndex } from './correlationIndex';
  import { annotationStore, envelopeKey as annKey, TAG_META } from './busAnnotations';
  import { bookmarkStore, savedQueryStore, type SavedQuery } from './busBookmarks';
  import AnnotationPopover from './AnnotationPopover.svelte';
  import BookmarksPanel from './BookmarksPanel.svelte';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
    correlationIndex?: CorrelationIndex | null;
  }

  let { severityThreshold = 'debug', onDragBack, correlationIndex = null }: Props = $props();

  const RECENT_LOG_LIMIT = 200;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  const ALL_CATEGORIES: Category[] = [
    'pty', 'hook', 'agent', 'fs', 'index', 'aegis', 'status', 'system', 'mcp', 'sentinel', 'llm',
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
    mcp:      'var(--term-purple, #C58FFF)',
    sentinel: 'var(--term-red)',
    llm:      'var(--amber-primary)',
  };

  // Local sequence number — monotonically incremented when each envelope is
  // admitted into the events array. Used as the stable {#each} key instead
  // of the array index, so buffer trims don't cause full DOM teardown.
  let _nextSeq = 0;
  type EnvelopeWithSeq = Envelope & { _seq: number };

  let connected = $state(false);
  let error = $state('');
  let events = $state<EnvelopeWithSeq[]>([]);
  let paused = $state(false);
  let mutedCats = $state<Set<Category>>(new Set());
  let lastTickTs = $state<number>(Date.now());
  const sparkline = new SparklineBuffer();
  let sparklineData = $state<number[]>(sparkline.snapshot());
  const heatstrip = new HeatstripBuffer();
  let heatstripData = $state(heatstrip.snapshot());
  let heatstripTickCounter = 0;
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

  let pendingBatch: Envelope[] = []; // raw, seq stamped in flushBatch
  let flushTimer: ReturnType<typeof setTimeout> | undefined;
  const FLUSH_INTERVAL_MS = 80;
  const PENDING_BATCH_CAP = 200;

  function flushBatch(): void {
    flushTimer = undefined;
    if (pendingBatch.length === 0) return;
    // Cap the pending batch under PTY-burst: keep the newest PENDING_BATCH_CAP
    // events and discard older overflow accumulated between frames.
    const raw = pendingBatch.length > PENDING_BATCH_CAP
      ? pendingBatch.slice(-PENDING_BATCH_CAP)
      : pendingBatch;
    pendingBatch = [];
    // Stamp each incoming envelope with a monotonic sequence number so the
    // {#each} key is stable even when the buffer is trimmed (index-keying
    // causes full DOM teardown on every trim).
    const batch: EnvelopeWithSeq[] = raw.map((e) => ({ ...e, _seq: _nextSeq++ }));
    let next = [...events, ...batch];
    if (next.length > RECENT_LOG_LIMIT * 2) {
      next = next.slice(-RECENT_LOG_LIMIT);
    }
    events = next;
    lastTickTs = Date.now();
  }

  function handleEnvelope(env: Envelope) {
    if (paused) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    sparkline.record();
    heatstrip.push(kindToSeverity(env.kind));
    pendingBatch.push(env);
    if (!flushTimer) {
      flushTimer = setTimeout(flushBatch, FLUSH_INTERVAL_MS);
    }
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
      sparkline.tick();
      sparklineData = sparkline.snapshot();
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
    if (flushTimer) clearTimeout(flushTimer);
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

  let logBodyEl: HTMLDivElement | undefined = $state(undefined);

  function handleHeatstripSeek(minuteOffset: number): void {
    // Scroll the recent-events log to rows whose timestamps fall within the
    // clicked minute bucket. minuteOffset 0 = 59 minutes ago, 59 = now.
    if (!logBodyEl) return;
    const now = Date.now();
    const minutesAgo = 59 - minuteOffset;
    const bucketStart = now - (minutesAgo + 1) * 60_000;
    const bucketEnd = now - minutesAgo * 60_000;

    // Find the first visible row element whose data-ts falls in range.
    const rows = logBodyEl.querySelectorAll<HTMLElement>('[data-ts]');
    for (const row of rows) {
      const ts = Number(row.dataset.ts);
      if (ts >= bucketStart && ts < bucketEnd) {
        row.scrollIntoView({ behavior: 'smooth', block: 'center' });
        // Brief highlight flash.
        row.style.background = 'rgba(255, 200, 64, 0.15)';
        setTimeout(() => { row.style.background = ''; }, 1200);
        return;
      }
    }
  }

  // ----- Bookmarks & Annotations -----
  let bookmarkVersion = $state(0);
  let annotationVersion = $state(0);

  onMount(() => {
    const ubm = bookmarkStore.onChange(() => { bookmarkVersion += 1; });
    const uann = annotationStore.onChange(() => { annotationVersion += 1; });
    return () => { ubm(); uann(); };
  });

  // Annotation popover state.
  let annotatingEnvelope = $state<Envelope | null>(null);
  let annotatingAnchorY = $state(0);

  function openAnnotation(env: Envelope, ev: MouseEvent): void {
    ev.stopPropagation();
    const rect = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    const paneRect = (ev.currentTarget as HTMLElement).closest('.pane')?.getBoundingClientRect();
    annotatingAnchorY = rect.top - (paneRect?.top ?? 0) + 20;
    annotatingEnvelope = env;
  }

  function closeAnnotation(): void {
    annotatingEnvelope = null;
  }

  function toggleBookmark(env: Envelope, ev: MouseEvent): void {
    ev.stopPropagation();
    bookmarkStore.toggle(env);
  }

  function isBookmarked(env: Envelope): boolean {
    // Reference bookmarkVersion to trigger reactivity.
    void bookmarkVersion;
    return bookmarkStore.isEnvelopeBookmarked(env);
  }

  function hasAnnotation(env: Envelope): boolean {
    void annotationVersion;
    return annotationStore.has(annKey(env));
  }

  // Saved query application — filter events to match query.
  let activeQuery = $state<SavedQuery | null>(null);

  function applyQuery(query: SavedQuery): void {
    activeQuery = query;
  }

  function clearActiveQuery(): void {
    activeQuery = null;
  }

  /** Events filtered by active saved query (on top of category muting). */
  const queryFilteredEvents = $derived.by(() => {
    if (!activeQuery) return recentEvents;
    return recentEvents.filter((e) => savedQueryStore.matches(e, activeQuery!));
  });

  function handleJumpToEvent(env: Envelope): void {
    if (!logBodyEl) return;
    const rows = logBodyEl.querySelectorAll<HTMLElement>('[data-ts]');
    for (const row of rows) {
      const ts = Number(row.dataset.ts);
      if (ts === env.ts) {
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
      onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onDragBack?.(); } }}
      title="drag back to tab strip to dock"
      aria-label="Bus tail — drag to dock back to tab strip"
    >
      <span class="handle-glyph">▣</span>
      <span class="handle-title">bus tail</span>
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
  {/if}

  <header class="status">
    <span class="title"><span class="icon">⌁</span>BUS TAIL</span>
    <span class="state">
      {visibleCount}/{totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
    </span>
    <SparklineChart data={sparklineData} />
    {#if activeQuery}
      <span class="active-query-badge" title="Active filter: {activeQuery.name}">
        {activeQuery.name}
        <button type="button" class="query-clear" onclick={clearActiveQuery} aria-label="Clear active query">x</button>
      </span>
    {/if}
    <span class="spacer"></span>
    <button type="button" class="ctl-btn" class:active={paused} onclick={togglePause}>
      {paused ? 'paused' : 'live'}
    </button>
    <button type="button" class="ctl-btn" onclick={clearLog} disabled={events.length === 0}>
      clear
    </button>
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
        {#each liveEvents as e (e._seq)}
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
    <div class="log-body" aria-live="polite" bind:this={logBodyEl}>
      {#if recentEvents.length === 0}
        <div class="empty-state">
          {#if paused}
            <span class="empty-state-icon">⏸</span>
            <span class="empty-state-text">Paused</span>
            <span class="empty-state-hint">Click LIVE to resume the event stream.</span>
          {:else}
            <span class="empty-state-icon">▣</span>
            <span class="empty-state-text">Bus is quiet</span>
            <span class="empty-state-hint">event stream from all bus categories will populate here</span>
          {/if}
        </div>
      {:else}
        {#each queryFilteredEvents as e (e._seq)}
          {@const rowKey = String(e._seq)}
          {@const isExpanded = expandedRows.has(rowKey)}
          {@const starred = isBookmarked(e)}
          {@const annotated = hasAnnotation(e)}
          <div
            class="row"
            class:expanded={isExpanded}
            class:bookmarked={starred}
            data-ts={e.ts}
            role="button"
            tabindex="0"
            aria-expanded={isExpanded}
            onclick={(ev) => {
              const target = ev.target as HTMLElement;
              if (target.closest('.payload-expanded') || target.closest('.row-action')) return;
              toggleRow(rowKey);
            }}
            onkeydown={(ev) => {
              if (ev.key === 'Enter' || ev.key === ' ') {
                ev.preventDefault();
                toggleRow(rowKey);
              }
            }}
            title="click to {isExpanded ? 'collapse' : 'expand'}"
          >
            <span class="row-actions">
              <button
                type="button"
                class="row-action star"
                class:active={starred}
                title={starred ? 'Remove bookmark' : 'Bookmark event'}
                onclick={(ev) => toggleBookmark(e, ev)}
              >*</button>
              <button
                type="button"
                class="row-action pencil"
                class:active={annotated}
                title="Annotate event"
                onclick={(ev) => openAnnotation(e, ev)}
              >/</button>
            </span>
            <span class="caret">{isExpanded ? '▼' : '▶'}</span>
            <span class="ts">{formatTs(e.ts)}</span>
            <span class="cat" style="color: {CAT_COLOR[e.category]};">{e.category}</span>
            <span class="kind">{e.kind}</span>
            {#if correlationIndex}
              <CorrelationBadge env={e} index={correlationIndex} />
            {/if}
            {#if annotated}
              {@const ann = annotationStore.getForEnvelope(e)}
              {#if ann && ann.tags.length > 0}
                <span class="row-tags">
                  {#each ann.tags as tag (tag)}
                    <span class="row-tag" style="color: {TAG_META[tag].cssVar}; border-color: {TAG_META[tag].cssVar}">{TAG_META[tag].label}</span>
                  {/each}
                </span>
              {/if}
            {/if}
            {#if isExpanded}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions — stopPropagation prevents parent row toggle during text selection -->
              <pre
                class="payload-expanded"
                onmousedown={(ev) => ev.stopPropagation()}
              >{formatPayloadExpanded(e.payload)}</pre>
              {#if annotated}
                {@const ann = annotationStore.getForEnvelope(e)}
                {#if ann && ann.note}
                  <div class="annotation-note">
                    <span class="annotation-label">NOTE</span>
                    <span class="annotation-text">{ann.note}</span>
                  </div>
                {/if}
              {/if}
            {:else}
              <span class="payload">{formatPayload(e.payload)}</span>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
    {#if annotatingEnvelope}
      <AnnotationPopover
        envelope={annotatingEnvelope}
        anchorY={annotatingAnchorY}
        onClose={closeAnnotation}
      />
    {/if}
  </div>

  <footer class="state-panel bookmarks-state-panel">
    <div class="state-header">BOOKMARKS & SAVED FILTERS</div>
    <div class="state-body">
      <BookmarksPanel
        onApplyQuery={applyQuery}
        onJumpToEvent={handleJumpToEvent}
      />
    </div>
  </footer>

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
    font-family: var(--font-family);
    /* Reset <button> defaults */
    border: none;
    border-radius: 0;
    width: 100%;
    text-align: left;
    transition: background var(--duration-base) ease-out;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .drag-handle .handle-glyph {
    color: var(--amber-bright);
    font-size: var(--text-base);
    text-shadow: var(--glow-amber-faint);
  }
  .drag-handle .handle-title {
    color: var(--amber-bright);
    text-transform: uppercase;
  }

  .status {
    height: 36px;
    padding: 0 var(--space-lg);
    background: var(--bg-elevated);
    box-shadow: var(--sep-glow);
    display: flex; align-items: center; gap: var(--space-md);
    color: var(--amber-warm);
    overflow: hidden;
    flex-shrink: 0;
  }
  .status .title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; }
  .status .state {
    color: var(--amber-dim);
    font-size: var(--type-caption-size);
    font-weight: var(--type-caption-weight);
    letter-spacing: var(--type-caption-spacing);
  }
  .status .spacer { flex: 1; }
  .ctl-btn {
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    font-weight: 700;
    padding: 2px var(--space-8);
    cursor: pointer;
    text-transform: uppercase;
    transition: color var(--duration-base) ease-out, background var(--duration-base) ease-out, border-color var(--duration-base) ease-out, opacity var(--duration-base) ease-out;
  }
  .ctl-btn:hover:not(:disabled) {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .ctl-btn:focus-visible {
    outline: 1px solid var(--amber-bright);
    outline-offset: 1px;
  }
  .ctl-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .ctl-btn.active {
    background: var(--term-red);
    border-color: var(--term-red);
    color: var(--bg-base);
  }

  .heatstrip-row {
    padding: var(--space-xs) var(--space-14);
    background: var(--bg-elevated);
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }

  .strip {
    height: var(--control-sm);
    padding: 0 var(--space-14);
    box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-14);
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--amber-bright); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }
  .strip-events {
    display: flex; gap: var(--space-sm); flex: 1; overflow: hidden;
  }
  .strip-event {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 1px var(--space-sm);
    border: 1px solid var(--cat-color);
    color: var(--cat-color);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(212, 137, 10, 0.06);
  }
  .strip-cat { opacity: 0.7; }
  .strip-kind { font-weight: 700; }

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
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }
  .connecting-state {
    color: var(--amber-faint);
    padding: 1rem var(--space-14);
    font-style: italic;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
  }
  .error-state {
    color: var(--term-red);
    padding: var(--space-12) var(--space-lg);
    font-size: var(--type-body-size);
    letter-spacing: var(--type-body-spacing);
    background: var(--bg-red-tint);
    box-shadow: var(--sep-depth);
  }

  .log-body .row {
    display: grid;
    grid-template-columns: 30px 14px 70px 60px 140px minmax(0, 1fr);
    gap: var(--space-8);
    align-items: baseline;
    padding: 1px 0;
    white-space: nowrap;
    cursor: pointer;
    user-select: text;
    transition: background var(--duration-base) ease-out;
  }
  .log-body .row:hover { background: rgba(212, 137, 10, 0.06); }
  .log-body .row.expanded {
    grid-template-columns: 30px 14px 70px 60px minmax(0, 1fr);
    grid-template-areas:
      "actions caret ts    cat   kind"
      "pl      pl    pl    pl    pl";
    background: rgba(212, 137, 10, 0.05);
    padding: var(--space-xs) 0 var(--space-sm);
    white-space: normal;
  }
  .log-body .row.expanded .row-actions { grid-area: actions; opacity: 1; }
  .log-body .row.expanded .caret { grid-area: caret; color: var(--amber-bright); }
  .log-body .row.expanded .ts    { grid-area: ts; }
  .log-body .row.expanded .cat   { grid-area: cat; }
  .log-body .row.expanded .kind  { grid-area: kind; }
  .log-body .caret {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    line-height: 1.5;
    user-select: none;
  }
  .log-body .ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
  }
  .log-body .cat {
    font-weight: 700;
    font-size: var(--text-2xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .log-body .kind {
    color: var(--amber-warm);
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
    margin: var(--space-xs) 0 0 22px;
    padding: var(--space-sm) var(--space-8);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    color: var(--amber-warm);
    font-family: var(--font-family);
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

  /* Active query badge in status header */
  .active-query-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 1px var(--space-8);
    background: rgba(108, 182, 255, 0.12);
    border: 1px solid rgba(108, 182, 255, 0.3);
    border-radius: var(--radius-sm, 2px);
    color: var(--term-blue);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
  }
  .query-clear {
    background: transparent;
    border: none;
    color: var(--term-blue);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
    border-radius: var(--radius-sm);
  }
  .query-clear:hover {
    color: var(--term-red);
  }

  /* Row action buttons (bookmark star + annotation pencil) */
  .row-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity var(--duration-base) ease-out;
  }
  .log-body .row:hover .row-actions,
  .log-body .row.bookmarked .row-actions {
    opacity: 1;
  }
  .row-action {
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
    transition: color var(--duration-base) ease-out;
  }
  .row-action:hover {
    color: var(--amber-bright);
  }
  .row-action.star.active {
    color: var(--amber-bright);
    opacity: 1;
  }
  .row-action.pencil.active {
    color: var(--term-cyan);
  }
  .log-body .row.bookmarked {
    border-left: 2px solid var(--amber-bright);
    padding-left: var(--space-xs);
  }

  /* Inline annotation tags on rows */
  .row-tags {
    display: inline-flex;
    gap: 3px;
    flex-shrink: 0;
  }
  .row-tag {
    display: inline-flex;
    padding: 0 3px;
    border: 1px solid;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    line-height: 13px;
  }

  /* Annotation note shown in expanded rows */
  .annotation-note {
    grid-area: pl;
    margin: 2px 0 0 22px;
    padding: var(--space-xs) var(--space-8);
    background: rgba(111, 224, 224, 0.06);
    border: 1px solid rgba(111, 224, 224, 0.15);
    border-radius: var(--radius-sm, 2px);
    display: flex;
    align-items: baseline;
    gap: var(--space-8);
    font-size: var(--text-xs);
  }
  .annotation-label {
    color: var(--term-cyan);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    flex-shrink: 0;
  }
  .annotation-text {
    color: var(--amber-dim);
    font-style: italic;
    line-height: 1.4;
  }

  /* Bookmarks state panel override for taller height */
  .bookmarks-state-panel {
    max-height: 220px;
  }
  .bookmarks-state-panel .state-body {
    padding: 0;
  }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    box-shadow: var(--depth-lift), var(--depth-edge-light);
    max-height: 180px;
    overflow-y: auto;
  }
  .state-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    box-shadow: var(--sep-depth);
  }
  .state-body {
    padding: var(--space-md) var(--space-lg) var(--space-14);
  }
  .cat-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-xs) var(--space-lg);
  }
  .cat-row {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    cursor: pointer;
    user-select: none;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    transition: opacity var(--duration-base) ease-out;
  }
  .cat-row.muted .cat-name,
  .cat-row.muted .cat-count {
    color: var(--amber-faint);
    text-decoration: line-through;
  }
  .cat-row input[type="checkbox"] {
    width: var(--space-12);
    height: var(--space-12);
    accent-color: var(--amber-bright);
    cursor: pointer;
  }
  .cat-dot {
    width: var(--space-8);
    height: var(--space-8);
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
