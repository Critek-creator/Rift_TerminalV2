<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { NOTIF_TAB_MIME } from './dragMime';
  import SessionCompare from './SessionCompare.svelte';
  import MarkerTimeline from './MarkerTimeline.svelte';
  import SessionTimeline from './SessionTimeline.svelte';

  interface SessionMeta {
    id: string;
    date: string;
    event_count: number;
    size_bytes: number;
  }

  interface SessionEvent {
    version: number;
    // Wire field is `ts` (see envelope.rs / bus.ts). The previous interface
    // read `timestamp`, which does not exist on persisted envelopes — every
    // replay row rendered NaN. Aligning to `ts` fixes that and lets the
    // marker timeline position pips by real time.
    ts: number;
    category: string;
    kind: string;
    payload: unknown;
  }

  type ViewMode = 'list' | 'replay' | 'timeline' | 'select-baseline' | 'select-compare' | 'compare';

  interface Props {
    onDragBack?: () => void;
  }

  let { onDragBack }: Props = $props();

  let sessions = $state<SessionMeta[]>([]);
  let selectedId = $state<string | null>(null);
  let events = $state<SessionEvent[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let expandedRows = $state<Set<number>>(new Set());

  /** Candidate 49d — session markers. Markers are `status`/`session.marker`
   *  envelopes (dropped via Ctrl+Shift+K during the live session); they ride
   *  the normal event stream and persist in the session .jsonl. In replay we
   *  surface them on a clickable MarkerTimeline that seeks the event log. */
  interface SessionMarker {
    idx: number;
    ts: number;
    label: string;
    note: string | null;
  }
  let replayLogBody = $state<HTMLDivElement | undefined>(undefined);
  let seekedIdx = $state<number | null>(null);
  let seekTimer: ReturnType<typeof setTimeout> | undefined;

  const markers = $derived.by<SessionMarker[]>(() =>
    events
      .map((e, i) => ({ e, i }))
      .filter(({ e }) => e.kind === 'session.marker')
      .map(({ e, i }) => {
        const p = (e.payload ?? {}) as { label?: unknown; note?: unknown };
        return {
          idx: i,
          ts: e.ts,
          label: typeof p.label === 'string' ? p.label : 'marker',
          note: typeof p.note === 'string' ? p.note : null,
        };
      }),
  );
  const sessionStartTs = $derived(events.length > 0 ? events[0].ts : 0);
  const sessionEndTs = $derived(events.length > 0 ? events[events.length - 1].ts : 0);

  function seekToEvent(idx: number): void {
    expandedRows = new Set(expandedRows).add(idx);
    seekedIdx = idx;
    if (seekTimer) clearTimeout(seekTimer);
    seekTimer = setTimeout(() => { seekedIdx = null; }, 1600);
    // Defer to next frame so the (now-expanded) row is laid out before scroll.
    requestAnimationFrame(() => {
      const row = replayLogBody?.querySelector<HTMLElement>(`[data-event-idx="${idx}"]`);
      row?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    });
  }

  // Timeline → replay deep-link: switch to the replay view, wait for it to
  // render, then seek to the chosen event (seekToEvent queries the replay DOM).
  async function seekFromTimeline(idx: number): Promise<void> {
    viewMode = 'replay';
    await tick();
    seekToEvent(idx);
  }

  // -- Global session search (candidate a1f) ---------------------------------
  // Full-text query across every persisted .jsonl log via the search_sessions
  // command. A hit carries the event index so a click loads that session and
  // seeks the replay log straight to the matched event.
  interface SessionSearchHit {
    session_id: string;
    date: string;
    event_idx: number;
    ts: number | null;
    category: string | null;
    kind: string | null;
    snippet: string;
  }
  const SEARCH_LIMIT = 200;
  let searchQuery = $state('');
  let searchHits = $state<SessionSearchHit[]>([]);
  let searching = $state(false);
  let searchError = $state<string | null>(null);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  // Monotonic search id — only the most-recently-issued query may write results,
  // so a slow earlier query resolving late can't clobber newer results.
  let searchSeq = 0;

  function onSearchInput(): void {
    if (searchTimer) clearTimeout(searchTimer);
    const q = searchQuery;
    if (!q.trim()) {
      searchSeq++; // invalidate any in-flight query
      searchHits = [];
      searchError = null;
      searching = false;
      return;
    }
    searching = true;
    searchTimer = setTimeout(() => runSearch(q), 220);
  }

  async function runSearch(q: string): Promise<void> {
    const seq = ++searchSeq;
    try {
      const hits = await invoke<SessionSearchHit[]>('search_sessions', {
        query: q,
        limit: SEARCH_LIMIT,
      });
      if (seq !== searchSeq) return; // a newer query superseded this one
      searchHits = hits;
      searchError = null;
    } catch (err) {
      if (seq !== searchSeq) return;
      searchError = String((err as Error).message ?? err);
      searchHits = [];
    } finally {
      if (seq === searchSeq) searching = false;
    }
  }

  function clearSearch(): void {
    if (searchTimer) clearTimeout(searchTimer);
    searchQuery = '';
    searchHits = [];
    searchError = null;
    searching = false;
  }

  async function openHit(hit: SessionSearchHit): Promise<void> {
    if (viewMode !== 'list') return;
    await selectSession(hit.session_id);
    await tick(); // let the replay log mount before seeking
    seekToEvent(hit.event_idx);
  }

  /** Comparison state */
  let viewMode = $state<ViewMode>('list');
  let baselineId = $state<string | null>(null);
  let compareId = $state<string | null>(null);

  const CAT_COLOR: Record<string, string> = {
    pty:      'var(--term-white)',
    hook:     'var(--term-cyan)',
    agent:    'var(--term-purple)',
    fs:       'var(--amber-faint)',
    index:    'var(--status-blue-bright, #6CB6FF)',
    aegis:    'var(--amber-primary)',
    status:   'var(--amber-bright)',
    system:   'var(--term-red)',
    mcp:      'var(--term-purple, #C58FFF)',
    sentinel: 'var(--term-red)',
  };

  function catColor(cat: string): string {
    return CAT_COLOR[cat] ?? 'var(--amber-dim)';
  }

  async function loadSessions() {
    loading = true;
    error = null;
    try {
      sessions = await invoke<SessionMeta[]>('list_sessions');
    } catch (err) {
      error = String((err as Error).message ?? err);
      sessions = [];
    } finally {
      loading = false;
    }
  }

  async function selectSession(id: string) {
    if (viewMode === 'select-baseline') {
      baselineId = id;
      viewMode = 'select-compare';
      return;
    }
    if (viewMode === 'select-compare') {
      if (id === baselineId) return; // Cannot compare a session to itself.
      compareId = id;
      viewMode = 'compare';
      return;
    }
    selectedId = id;
    viewMode = 'replay';
    loading = true;
    error = null;
    expandedRows = new Set();
    try {
      events = await invoke<SessionEvent[]>('load_session', { sessionId: id });
    } catch (err) {
      error = String((err as Error).message ?? err);
      events = [];
    } finally {
      loading = false;
    }
  }

  function goBack() {
    selectedId = null;
    events = [];
    expandedRows = new Set();
    error = null;
    viewMode = 'list';
  }

  function enterCompareMode() {
    baselineId = null;
    compareId = null;
    viewMode = 'select-baseline';
  }

  function exitCompareMode() {
    viewMode = 'list';
    baselineId = null;
    compareId = null;
  }

  function onCompareBack() {
    exitCompareMode();
  }

  const isSelectingMode = $derived(
    viewMode === 'select-baseline' || viewMode === 'select-compare',
  );

  const canCompare = $derived(sessions.length >= 2);

  const baselineSession = $derived(
    baselineId ? sessions.find((s) => s.id === baselineId) : undefined,
  );
  const compareSession = $derived(
    compareId ? sessions.find((s) => s.id === compareId) : undefined,
  );

  function toggleRow(idx: number) {
    const next = new Set(expandedRows);
    if (next.has(idx)) next.delete(idx); else next.add(idx);
    expandedRows = next;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function formatCount(n: number): string {
    return n.toLocaleString();
  }

  function formatTs(ts: number): string {
    const d = new Date(ts);
    const h = String(d.getHours()).padStart(2, '0');
    const m = String(d.getMinutes()).padStart(2, '0');
    const s = String(d.getSeconds()).padStart(2, '0');
    const ms = String(d.getMilliseconds()).padStart(3, '0');
    return `${h}:${m}:${s}.${ms}`;
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

  const selectedSession = $derived(
    selectedId ? sessions.find((s) => s.id === selectedId) : undefined,
  );

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }

  onMount(() => {
    loadSessions();
  });

  onDestroy(() => {
    if (seekTimer) clearTimeout(seekTimer);
    if (searchTimer) clearTimeout(searchTimer);
  });
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
      <span class="handle-glyph" style="color: var(--term-blue); font-size: 14px">⏱</span>
      <span class="handle-title">sessions</span>
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
    <div class="error-state" role="alert">&#x26A0; {error}</div>
  {/if}

  {#if viewMode === 'compare' && baselineId && compareId}
    <!-- Comparison view -->
    <SessionCompare
      baselineId={baselineId}
      compareId={compareId}
      baselineDate={baselineSession?.date ?? baselineId}
      compareDate={compareSession?.date ?? compareId}
      onBack={onCompareBack}
    />
  {:else if viewMode === 'list' || isSelectingMode}
    <!-- Session list view -->
    <header class="status">
      <span class="title"><span class="icon">&#x23F1;</span>SESSIONS</span>
      <span class="state" role="status" aria-live="polite">
        {#if isSelectingMode}
          {viewMode === 'select-baseline' ? 'select baseline session' : 'select comparison session'}
        {:else}
          {sessions.length} saved session{sessions.length === 1 ? '' : 's'}
        {/if}
      </span>
      <span class="spacer"></span>
      {#if isSelectingMode}
        <button type="button" class="ctl-btn" onclick={exitCompareMode}>
          cancel
        </button>
      {:else}
        <button
          type="button"
          class="ctl-btn compare-btn"
          onclick={enterCompareMode}
          disabled={!canCompare || loading}
          title={canCompare ? 'Compare two sessions side by side' : 'Need at least 2 sessions to compare'}
        >
          compare
        </button>
        <button type="button" class="ctl-btn" onclick={loadSessions} disabled={loading}>
          {loading ? 'loading...' : 'refresh'}
        </button>
      {/if}
    </header>

    {#if isSelectingMode && baselineId}
      <div class="selection-hint">
        Baseline: <strong>{baselineSession?.date ?? baselineId}</strong>
        — now click the session to compare against
      </div>
    {/if}

    {#if !isSelectingMode}
      {@const trimmedQuery = searchQuery.trim()}
      <div class="session-search">
        <span class="search-icon" aria-hidden="true">&#x2315;</span>
        <input
          class="search-input"
          type="text"
          placeholder="Search all sessions — commands, errors, kinds, payloads…"
          bind:value={searchQuery}
          oninput={onSearchInput}
          aria-label="Search across all persisted sessions"
        />
        {#if searching}
          <span class="search-status">searching…</span>
        {:else if trimmedQuery}
          <span class="search-status">{searchHits.length}{searchHits.length === SEARCH_LIMIT ? '+' : ''} hit{searchHits.length === 1 ? '' : 's'}</span>
        {/if}
        {#if trimmedQuery}
          <button type="button" class="ctl-btn" onclick={clearSearch}>clear</button>
        {/if}
      </div>
    {/if}

    <div class="log">
      <div class="log-header">
        {#if isSelectingMode}
          {viewMode === 'select-baseline' ? 'SELECT BASELINE' : 'SELECT COMPARISON'}
        {:else if searchQuery.trim()}
          SEARCH RESULTS
        {:else}
          SAVED SESSIONS
        {/if}
      </div>
      <div class="log-body" aria-busy={loading || searching}>
        {#if !isSelectingMode && searchQuery.trim()}
          {#if searchError}
            <div class="empty-card">
              <div class="empty-title">Search failed</div>
              <div class="empty-desc">{searchError}</div>
            </div>
          {:else if searching && searchHits.length === 0}
            <div class="empty-card">
              <div class="empty-title">Searching…</div>
              <div class="empty-desc">Scanning persisted session logs.</div>
            </div>
          {:else if searchHits.length === 0}
            <div class="empty-card">
              <div class="empty-title">No matches</div>
              <div class="empty-desc">Nothing in the persisted session logs matches &ldquo;{searchQuery.trim()}&rdquo;.</div>
            </div>
          {:else}
            {#each searchHits as hit, hi (hit.session_id + ':' + hit.event_idx + ':' + hi)}
              <button type="button" class="hit-row" onclick={() => openHit(hit)} title="Open session {hit.session_id} at this event">
                <span class="hit-date">{hit.date}</span>
                {#if hit.kind}
                  <span class="hit-kind" style="color: {catColor(hit.category ?? '')}">{hit.kind}</span>
                {/if}
                <span class="hit-snippet">{hit.snippet}</span>
              </button>
            {/each}
          {/if}
        {:else if loading && sessions.length === 0}
          <div class="empty-card">
            <div class="empty-title">Loading...</div>
            <div class="empty-desc">Scanning session log directory.</div>
          </div>
        {:else if sessions.length === 0}
          <div class="empty-card">
            <div class="empty-title">No saved sessions</div>
            <div class="empty-desc">Sessions are recorded automatically when enabled in settings. Check that session logging is active in rift-config.toml.</div>
          </div>
        {:else}
          {#each sessions as s (s.id)}
            {@const isBaseline = s.id === baselineId}
            <button
              type="button"
              class="session-row"
              class:session-row--baseline={isBaseline}
              class:session-row--disabled={isBaseline && viewMode === 'select-compare'}
              disabled={isBaseline && viewMode === 'select-compare'}
              onclick={() => selectSession(s.id)}
            >
              {#if isBaseline}
                <span class="session-badge">BASE</span>
              {/if}
              <span class="session-date">{s.date}</span>
              <span class="session-count">{formatCount(s.event_count)} events</span>
              <span class="session-size">{formatSize(s.size_bytes)}</span>
              {#if !isSelectingMode}
                <span class="session-arrow">&#x25B6;</span>
              {/if}
            </button>
          {/each}
        {/if}
      </div>
    </div>
  {:else if viewMode === 'replay'}
    <!-- Event viewer -->
    <header class="status">
      <button type="button" class="ctl-btn" onclick={goBack}>&#x25C0; back</button>
      <span class="title"><span class="icon">&#x23F1;</span>{selectedSession?.date ?? selectedId}</span>
      <span class="state">
        {formatCount(events.length)} event{events.length === 1 ? '' : 's'}
        {#if selectedSession}· {formatSize(selectedSession.size_bytes)}{/if}
      </span>
      <span class="spacer"></span>
      <button type="button" class="ctl-btn" onclick={() => { viewMode = 'timeline'; }}>timeline &#x25B8;</button>
    </header>

    {#if markers.length > 0}
      <MarkerTimeline
        markers={markers}
        startTs={sessionStartTs}
        endTs={sessionEndTs}
        onSeek={seekToEvent}
      />
    {/if}

    <div class="log">
      <div class="log-header">SESSION EVENTS</div>
      <div class="log-body" aria-busy={loading} bind:this={replayLogBody}>
        {#if loading}
          <div class="empty-card">
            <div class="empty-title">Loading...</div>
            <div class="empty-desc">Reading session events from disk.</div>
          </div>
        {:else if events.length === 0}
          <div class="empty-card">
            <div class="empty-title">No events in this session</div>
            <div class="empty-desc">The session file may be empty or corrupted.</div>
          </div>
        {:else}
          {#each events as e, i (e.ts + ':' + i)}
            {@const isExpanded = expandedRows.has(i)}
            <button
              type="button"
              class="row"
              class:expanded={isExpanded}
              class:seeked={seekedIdx === i}
              data-event-idx={i}
              onclick={(ev) => {
                const target = ev.target as HTMLElement;
                if (target.closest('.payload-expanded')) return;
                toggleRow(i);
              }}
              aria-expanded={isExpanded}
              title="click to {isExpanded ? 'collapse' : 'expand'}"
            >
              <span class="caret">{isExpanded ? '&#x25BC;' : '&#x25B6;'}</span>
              <span class="ts">{formatTs(e.ts)}</span>
              <span class="cat" style="color: {catColor(e.category)};">{e.category}</span>
              <span class="kind">{e.kind}</span>
              {#if isExpanded}
                <!-- svelte-ignore a11y_no_noninteractive_element_interactions — stopPropagation prevents parent row toggle during text selection -->
                <pre
                  class="payload-expanded"
                  onmousedown={(ev) => ev.stopPropagation()}
                >{formatPayloadExpanded(e.payload)}</pre>
              {:else}
                <span class="payload">{formatPayload(e.payload)}</span>
              {/if}
            </button>
          {/each}
        {/if}
      </div>
    </div>
  {:else if viewMode === 'timeline'}
    <header class="status">
      <button type="button" class="ctl-btn" onclick={goBack}>&#x25C0; back</button>
      <span class="title"><span class="icon">&#x23F1;</span>{selectedSession?.date ?? selectedId}</span>
      <span class="spacer"></span>
      <button type="button" class="ctl-btn" onclick={() => { viewMode = 'replay'; }}>&#x25C2; replay</button>
    </header>
    {#if selectedId}
      <SessionTimeline sessionId={selectedId} onSeek={seekFromTimeline} />
    {/if}
  {/if}
</section>

<style>
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
  .drag-handle:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -2px; }
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
    display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
  }
  .status .title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; font-size: var(--text-lg); }
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

  .error-state {
    color: var(--term-red);
    padding: var(--space-12) var(--space-lg);
    font-size: var(--type-body-size);
    letter-spacing: var(--type-body-spacing);
    background: var(--bg-red-tint);
    box-shadow: var(--sep-depth);
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
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .empty-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-8);
    padding: var(--space-2xl) var(--space-lg);
    text-align: center;
    min-height: 120px;
  }
  .empty-title {
    color: var(--amber-dim);
    font-size: var(--type-body-size);
    font-weight: var(--type-body-weight);
    letter-spacing: var(--type-body-spacing);
  }
  .empty-desc {
    color: var(--amber-faint);
    font-size: var(--type-caption-size);
    letter-spacing: var(--type-caption-spacing);
    font-style: italic;
    max-width: 320px;
  }

  /* Compare button */
  .compare-btn {
    border-color: rgba(108, 182, 255, 0.4);
    color: var(--term-blue);
  }
  .compare-btn:hover:not(:disabled) {
    border-color: var(--term-blue);
    color: var(--term-blue);
    background: rgba(108, 182, 255, 0.08);
  }

  /* Selection hint bar */
  .selection-hint {
    padding: var(--space-sm) var(--space-lg);
    font-size: var(--type-caption-size);
    color: var(--amber-dim);
    background: rgba(108, 182, 255, 0.06);
    box-shadow: var(--sep-depth);
    letter-spacing: var(--type-caption-spacing);
  }
  .selection-hint strong {
    color: var(--amber-bright);
    font-weight: 700;
  }

  /* Session list rows */
  .session-row {
    display: flex;
    align-items: center;
    gap: var(--space-14);
    padding: var(--space-md) var(--space-xs);
    width: 100%;
    background: transparent;
    border: none;
    color: inherit;
    font-family: inherit;
    font-size: inherit;
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-base) ease-out;
  }
  .session-row:hover:not(:disabled) { background: rgba(212, 137, 10, 0.08); }
  .session-row:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -2px; }
  .session-row + .session-row { margin-top: 1px; }
  .session-row--baseline {
    background: rgba(108, 182, 255, 0.08);
    border-left: 2px solid var(--term-blue);
  }
  .session-row--disabled {
    opacity: 0.4;
    cursor: not-allowed;
    pointer-events: none;
  }
  .session-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--term-blue);
    border: 1px solid rgba(108, 182, 255, 0.4);
    border-radius: var(--radius-sm, 2px);
    padding: 1px var(--space-xs);
    flex-shrink: 0;
  }
  .session-date {
    color: var(--amber-bright);
    font-weight: 600;
    font-size: var(--text-lg);
    min-width: 140px;
    text-shadow: var(--glow-amber-faint);
  }
  .session-count {
    color: var(--amber-dim);
    font-size: var(--type-caption-size);
    font-variant-numeric: tabular-nums;
    letter-spacing: var(--type-caption-spacing);
    min-width: 100px;
  }
  .session-size {
    color: var(--amber-faint);
    font-size: var(--type-caption-size);
    font-variant-numeric: tabular-nums;
    letter-spacing: var(--type-caption-spacing);
    min-width: 60px;
  }
  .session-arrow {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    margin-left: auto;
  }

  /* Event rows — mirrors BusTailTabContent grid pattern */
  .log-body .row {
    display: grid;
    grid-template-columns: 14px 85px 60px 140px minmax(0, 1fr);
    gap: var(--space-8);
    align-items: baseline;
    padding: 1px 0;
    white-space: nowrap;
    cursor: pointer;
    user-select: text;
    width: 100%;
    background: transparent;
    border: none;
    color: inherit;
    font-family: inherit;
    font-size: inherit;
    text-align: left;
    transition: background var(--duration-base) ease-out;
  }
  .log-body .row:hover { background: rgba(212, 137, 10, 0.06); }
  .log-body .row:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -1px; }
  /* Candidate 49d — transient flash when seeked via a MarkerTimeline pip. */
  .log-body .row.seeked { animation: seek-flash 1.6s ease-out; }
  @keyframes seek-flash {
    0%, 30% { background: rgba(255, 200, 64, 0.22); box-shadow: inset 2px 0 0 var(--amber-bright); }
    100% { background: transparent; box-shadow: none; }
  }
  .log-body .row.expanded {
    grid-template-columns: 14px 85px 60px minmax(0, 1fr);
    grid-template-areas:
      "caret ts    cat   kind"
      "pl    pl    pl    pl";
    background: rgba(212, 137, 10, 0.05);
    padding: var(--space-xs) 0 var(--space-sm);
    white-space: normal;
  }
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

  /* Global session search (candidate a1f) */
  .session-search {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-xs) var(--space-sm);
    border-bottom: 1px solid var(--border-subtle);
  }
  .session-search .search-icon {
    color: var(--amber-faint);
    font-size: var(--text-sm);
    flex-shrink: 0;
  }
  .search-input {
    flex: 1;
    min-width: 0;
    background: rgba(212, 137, 10, 0.05);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    color: var(--term-white);
    font-family: inherit;
    font-size: var(--text-xs);
    padding: var(--space-xs) var(--space-sm);
    transition: border-color var(--duration-base) ease-out, background var(--duration-base) ease-out;
  }
  .search-input::placeholder { color: var(--amber-faint); }
  .search-input:focus {
    outline: none;
    border-color: var(--amber-bright);
    background: rgba(212, 137, 10, 0.09);
  }
  .search-status {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .hit-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
    width: 100%;
    padding: var(--space-xs) var(--space-sm);
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: inherit;
    font-family: inherit;
    font-size: var(--text-xs);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-base) ease-out, border-color var(--duration-base) ease-out;
  }
  .hit-row:hover {
    background: rgba(212, 137, 10, 0.07);
    border-left-color: var(--amber-primary);
  }
  .hit-row:focus-visible { outline: 1px solid var(--amber-bright); outline-offset: -1px; }
  .hit-date {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .hit-kind {
    font-weight: 600;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .hit-snippet {
    color: var(--amber-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-family: var(--font-family);
  }
</style>
