<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { NOTIF_TAB_MIME } from './dragMime';
  import SessionCompare from './SessionCompare.svelte';

  interface SessionMeta {
    id: string;
    date: string;
    event_count: number;
    size_bytes: number;
  }

  interface SessionEvent {
    id: string;
    version: number;
    timestamp: number;
    category: string;
    kind: string;
    payload: unknown;
  }

  type ViewMode = 'list' | 'replay' | 'select-baseline' | 'select-compare' | 'compare';

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
      <span class="handle-title">sessions</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if error}
    <div class="error-state">&#x26A0; {error}</div>
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
      <span class="state">
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

    <div class="log">
      <div class="log-header">
        {#if isSelectingMode}
          {viewMode === 'select-baseline' ? 'SELECT BASELINE' : 'SELECT COMPARISON'}
        {:else}
          SAVED SESSIONS
        {/if}
      </div>
      <div class="log-body">
        {#if loading && sessions.length === 0}
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
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="session-row"
              class:session-row--baseline={isBaseline}
              class:session-row--disabled={isBaseline && viewMode === 'select-compare'}
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
            </div>
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
    </header>

    <div class="log">
      <div class="log-header">SESSION EVENTS</div>
      <div class="log-body">
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
          {#each events as e, i (i)}
            {@const isExpanded = expandedRows.has(i)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="row"
              class:expanded={isExpanded}
              onclick={(ev) => {
                const target = ev.target as HTMLElement;
                if (target.closest('.payload-expanded')) return;
                toggleRow(i);
              }}
              title="click to {isExpanded ? 'collapse' : 'expand'}"
            >
              <span class="caret">{isExpanded ? '&#x25BC;' : '&#x25B6;'}</span>
              <span class="ts">{formatTs(e.timestamp)}</span>
              <span class="cat" style="color: {catColor(e.category)};">{e.category}</span>
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
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    gap: var(--space-md);
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    font-weight: 700;
  }
  .drag-handle { transition: background var(--duration-base) ease-out; }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--amber-bright);
    font-size: var(--text-base);
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
    height: var(--control-md);
    padding: 0 var(--space-14);
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
    font-size: var(--text-sm); letter-spacing: 0.1em; font-weight: 700;
  }
  .status .title {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; }
  .status .state { color: var(--amber-dim); font-weight: 400; letter-spacing: 0.04em; }
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
    padding: var(--space-12) var(--space-14);
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
    border-bottom: 1px solid rgba(255, 72, 72, 0.2);
    background: rgba(255, 72, 72, 0.06);
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    min-width: 0;
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
    border: 1px dashed var(--border-subtle);
    padding: var(--space-12) var(--space-14);
    background: rgba(212, 137, 10, 0.05);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.55;
  }
  .empty-title {
    color: var(--amber-bright);
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    margin-bottom: var(--space-sm);
  }
  .empty-desc {
    color: var(--amber-dim);
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
    color: var(--amber-dim);
    background: rgba(108, 182, 255, 0.06);
    border-bottom: 1px solid rgba(108, 182, 255, 0.15);
    letter-spacing: 0.04em;
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
    padding: var(--space-8) var(--space-xs);
    border-bottom: 1px solid rgba(255, 168, 38, 0.06);
    cursor: pointer;
    transition: background var(--duration-base) ease-out;
  }
  .session-row:hover { background: rgba(212, 137, 10, 0.08); }
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
    font-size: 8px;
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
    font-size: var(--text-base);
    min-width: 140px;
  }
  .session-count {
    color: var(--amber-warm);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    min-width: 100px;
  }
  .session-size {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
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
    transition: background var(--duration-base) ease-out;
  }
  .log-body .row:hover { background: rgba(212, 137, 10, 0.06); }
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
</style>
