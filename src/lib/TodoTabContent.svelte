<script lang="ts">
  // Phase 8.7i — TODO tab. Surfaces TODO/FIXME/XXX/HACK markers across the
  // active project so they're discoverable instead of buried in source.
  //
  // Backend: `todo_scan_command` synchronously walks the project root with
  // the same ignore-globs as fs_tree, capping at 1000 results and 1 MiB
  // per file. We invoke on mount + on the Refresh button.
  //
  // Click any row → opens the file in the Viewer popout. Line jump is a
  // future enhancement (Viewer doesn't accept a line param yet).

  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { popouts } from './popouts.svelte';
  import { NOTIF_TAB_MIME } from './dragMime';

  interface Props {
    onDragBack?: () => void;
  }

  let { onDragBack }: Props = $props();

  type Marker = 'TODO' | 'FIXME' | 'XXX' | 'HACK';
  interface TodoEntry {
    path: string;
    line: number;
    marker: Marker;
    message: string;
  }

  const ALL_MARKERS: Marker[] = ['TODO', 'FIXME', 'XXX', 'HACK'];

  // §10.1-adjacent palette — give each marker its own colour
  const MARKER_COLOR: Record<Marker, string> = {
    TODO:  'var(--amber-bright)',
    FIXME: 'var(--term-red)',
    XXX:   'var(--term-purple)',
    HACK:  'var(--term-cyan)',
  };

  let entries = $state<TodoEntry[]>([]);
  let scanning = $state(false);
  let lastScanTs = $state<number | null>(null);
  let scanError = $state<string | null>(null);
  let activeMarker = $state<Marker | 'ALL'>('ALL');

  // Phase 8.7j — dismissed-entry persistence. Key shape: `path:line:marker`.
  // Survives reloads via localStorage so checked-off TODOs stay out of the
  // way until the source line changes (different line number → new key).
  const DISMISSED_KEY = 'rift.todo.dismissed';
  let dismissed = $state<Set<string>>(loadDismissed());
  let showDismissed = $state(false);

  function entryKey(e: TodoEntry): string {
    return `${e.path}:${e.line}:${e.marker}`;
  }
  function loadDismissed(): Set<string> {
    try {
      const raw = localStorage.getItem(DISMISSED_KEY);
      if (!raw) return new Set();
      const arr = JSON.parse(raw) as unknown;
      if (!Array.isArray(arr)) return new Set();
      return new Set(arr.filter((s): s is string => typeof s === 'string'));
    } catch {
      return new Set();
    }
  }
  function persistDismissed(s: Set<string>) {
    try {
      localStorage.setItem(DISMISSED_KEY, JSON.stringify([...s]));
    } catch {
      // best-effort
    }
  }
  function dismissEntry(e: TodoEntry) {
    const next = new Set(dismissed);
    next.add(entryKey(e));
    dismissed = next;
    persistDismissed(next);
  }
  function restoreEntry(e: TodoEntry) {
    const next = new Set(dismissed);
    next.delete(entryKey(e));
    dismissed = next;
    persistDismissed(next);
  }
  function clearDismissed() {
    dismissed = new Set();
    persistDismissed(dismissed);
  }

  const visibleEntries = $derived(
    showDismissed ? entries : entries.filter((e) => !dismissed.has(entryKey(e)))
  );
  const filtered = $derived(
    activeMarker === 'ALL'
      ? visibleEntries
      : visibleEntries.filter((e) => e.marker === activeMarker)
  );
  const totalCount = $derived(visibleEntries.length);
  const rawTotalCount = $derived(entries.length);
  const dismissedCount = $derived(
    entries.reduce((n, e) => (dismissed.has(entryKey(e)) ? n + 1 : n), 0)
  );
  const filteredCount = $derived(filtered.length);

  const markerCounts = $derived.by(() => {
    const h: Record<Marker, number> = { TODO: 0, FIXME: 0, XXX: 0, HACK: 0 };
    for (const e of visibleEntries) h[e.marker] += 1;
    return h;
  });
  const fileCount = $derived.by(() => {
    const seen = new Set<string>();
    for (const e of visibleEntries) seen.add(e.path);
    return seen.size;
  });

  async function runScan() {
    scanning = true;
    scanError = null;
    try {
      const result = await invoke<TodoEntry[]>('todo_scan_command');
      entries = result;
      lastScanTs = Date.now();
    } catch (err) {
      scanError = String(err);
      console.error('[TodoTab] todo_scan_command failed', err);
    } finally {
      scanning = false;
    }
  }

  onMount(() => {
    void runScan();
  });

  function openEntry(entry: TodoEntry) {
    popouts.summon({
      content: { kind: 'viewer', path: entry.path },
    });
  }

  function formatScanLabel(): string {
    if (scanning) return 'scanning…';
    if (scanError) return `error: ${scanError}`;
    if (lastScanTs === null) return 'awaiting scan';
    const ageMs = Date.now() - lastScanTs;
    if (ageMs < 60_000) return `scanned ${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `scanned ${Math.floor(ageMs / 60_000)}m ago`;
    return `scanned ${Math.floor(ageMs / 3_600_000)}h ago`;
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
      <span class="handle-glyph" style="color: var(--amber-warm); font-size: 14px">☐</span>
      <span class="handle-title">todo</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  <header class="status">
    <span class="title"><span class="icon">⊜</span>TODO</span>
    <span class="state">
      {filteredCount}/{totalCount} marker{totalCount === 1 ? '' : 's'} · {fileCount} file{fileCount === 1 ? '' : 's'} · {formatScanLabel()}
    </span>
    <span class="spacer"></span>
    <button
      type="button"
      class="ctl-btn"
      class:active={showDismissed}
      onclick={() => (showDismissed = !showDismissed)}
      title="toggle visibility of dismissed entries"
      disabled={dismissedCount === 0 && !showDismissed}
    >
      {showDismissed ? `hide done (${dismissedCount})` : `show done (${dismissedCount})`}
    </button>
    <button type="button" class="ctl-btn" onclick={runScan} disabled={scanning}>
      {scanning ? '…' : 'rescan'}
    </button>
  </header>

  <div class="strip">
    <span class="strip-label">FILTER</span>
    <div class="filter-row">
      <button
        type="button"
        class="chip"
        class:active={activeMarker === 'ALL'}
        onclick={() => (activeMarker = 'ALL')}
      >
        all <span class="chip-n">{totalCount}</span>
      </button>
      {#each ALL_MARKERS as m (m)}
        <button
          type="button"
          class="chip"
          class:active={activeMarker === m}
          style="--chip-color: {MARKER_COLOR[m]};"
          onclick={() => (activeMarker = m)}
        >
          {m.toLowerCase()} <span class="chip-n">{markerCounts[m]}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="log">
    <div class="log-header">MARKERS · click any row to open file</div>
    <div class="log-body" aria-live="polite">
      {#if scanError}
        <div class="empty error">scan failed — {scanError}</div>
      {:else if scanning && entries.length === 0}
        <div class="empty">scanning project…</div>
      {:else if filtered.length === 0}
        <div class="empty">
          {activeMarker === 'ALL'
            ? 'no markers found in project source'
            : `no ${activeMarker} markers — switch filter to see other kinds`}
        </div>
      {:else}
        {#each filtered as e, i (entryKey(e) + ':' + i)}
          {@const isDismissed = dismissed.has(entryKey(e))}
          <div class="row" class:row-dismissed={isDismissed}>
            <button
              type="button"
              class="row-main"
              onclick={() => openEntry(e)}
              title="open {e.path}"
            >
              <span class="marker" style="color: {MARKER_COLOR[e.marker]};">{e.marker}</span>
              <span class="path">{e.path}<span class="line-sep">:</span><span class="lineno">{e.line}</span></span>
              <span class="message">{e.message || '(no message)'}</span>
            </button>
            {#if isDismissed}
              <button
                type="button"
                class="row-action restore"
                onclick={() => restoreEntry(e)}
                title="restore (un-dismiss)"
                aria-label="restore"
              >↺</button>
            {:else}
              <button
                type="button"
                class="row-action dismiss"
                onclick={() => dismissEntry(e)}
                title="dismiss (mark done)"
                aria-label="dismiss"
              >×</button>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">SUMMARY</div>
    <div class="state-body">
      {#each ALL_MARKERS as m (m)}
        <div class="k-row">
          <span class="k" style="color: {MARKER_COLOR[m]};">{m}</span>
          <span class="v">{markerCounts[m]}</span>
        </div>
      {/each}
      <div class="k-row total">
        <span class="k">total</span>
        <span class="v">{totalCount}</span>
      </div>
      <div class="k-row total">
        <span class="k">files affected</span>
        <span class="v">{fileCount}</span>
      </div>
      {#if dismissedCount > 0}
        <div class="k-row total">
          <span class="k">dismissed</span>
          <span class="v">
            {dismissedCount}
            <button
              type="button"
              class="inline-clear"
              onclick={clearDismissed}
              title="clear all dismissed entries (they'll reappear)"
            >clear</button>
          </span>
        </div>
      {/if}
      {#if rawTotalCount >= 1000}
        <div class="cap-note">
          (capped at 1000 results — refine ignore globs in settings to surface deeper hits)
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
    transition: background var(--duration-base) ease-out;
  }
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
  .status .state { color: var(--amber-dim); font-size: var(--type-caption-size); font-weight: var(--type-caption-weight); letter-spacing: var(--type-caption-spacing); }
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
    background: var(--amber-bright);
    border-color: var(--amber-bright);
    color: var(--bg-base);
  }

  .strip {
    min-height: 32px;
    padding: var(--space-xs) var(--space-14);
    box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-md);
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--type-caption-size);
    letter-spacing: var(--type-caption-spacing);
    flex-wrap: wrap;
  }
  .strip-label { color: var(--amber-bright); font-weight: 700; }
  .filter-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    flex: 1;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: 2px var(--space-8);
    border: 1px solid var(--chip-color, var(--amber-faint));
    color: var(--chip-color, var(--amber-warm));
    background: transparent;
    font-family: inherit;
    font-size: var(--text-2xs);
    letter-spacing: 0.08em;
    font-weight: 700;
    cursor: pointer;
    text-transform: uppercase;
    transition: background var(--duration-base) ease-out, color var(--duration-base) ease-out;
  }
  .chip:hover { background: rgba(212, 137, 10, 0.06); }
  .chip.active {
    background: var(--chip-color, var(--amber-bright));
    color: var(--bg-base);
  }
  .chip-n {
    font-variant-numeric: tabular-nums;
    opacity: 0.7;
  }
  .chip.active .chip-n { opacity: 1; }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
  }
  .log-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    box-shadow: var(--sep-depth);
    background: var(--bg-surface);
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-sm) var(--space-lg);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    box-shadow: var(--depth-inset);
    line-height: 1.5;
    display: flex;
    flex-direction: column;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .empty {
    color: var(--amber-dim);
    font-size: var(--type-caption-size);
    font-style: italic;
    padding: var(--space-sm) 0;
  }
  .empty.error { color: var(--term-red); font-style: normal; font-size: var(--type-body-size); }

  .row {
    display: flex;
    align-items: stretch;
    border-left: 2px solid transparent;
    width: 100%;
    transition: background var(--duration-base) ease-out, border-left-color var(--duration-base) ease-out;
  }
  .row:hover {
    background: rgba(212, 137, 10, 0.06);
    border-left-color: var(--amber-bright);
  }
  .row.row-dismissed {
    opacity: 0.45;
  }
  .row.row-dismissed .marker,
  .row.row-dismissed .path,
  .row.row-dismissed .message {
    text-decoration: line-through;
  }
  .row-main {
    display: grid;
    grid-template-columns: 60px 1fr 1.4fr;
    gap: var(--space-12);
    align-items: baseline;
    padding: 3px var(--space-xs);
    background: transparent;
    border: none;
    color: inherit;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    flex: 1;
    min-width: 0;
  }
  .row-action {
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-family: inherit;
    transition: color var(--duration-base) ease-out;
    font-size: var(--text-lg);
    line-height: 1;
    padding: 0 var(--space-8);
    cursor: pointer;
    flex-shrink: 0;
    align-self: center;
  }
  .row-action.dismiss:hover { color: var(--term-red); }
  .row-action.restore:hover { color: var(--amber-bright); }
  .inline-clear {
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-family: inherit;
    font-size: var(--text-2xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    margin-left: var(--space-8);
    padding: 0;
    transition: color var(--duration-base) ease-out;
  }
  .inline-clear:hover { color: var(--term-red); }
  .marker {
    font-weight: 700;
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
  }
  .path {
    color: var(--amber-warm);
    font-weight: 600;
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line-sep { color: var(--amber-faint); margin: 0 2px; }
  .lineno {
    color: var(--amber-bright);
    font-variant-numeric: tabular-nums;
  }
  .message {
    color: var(--amber-dim);
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 180px;
    overflow-y: auto;
    box-shadow: var(--depth-lift), var(--depth-edge-light);
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
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .k-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .k-row .k {
    font-weight: 700;
    text-transform: uppercase;
  }
  .k-row .v {
    color: var(--amber-bright);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .k-row.total {
    border-top: 1px solid var(--border-subtle);
    padding-top: var(--space-xs);
    margin-top: var(--space-xs);
  }
  .k-row.total:nth-of-type(1) { border-top: 1px solid var(--border-subtle); margin-top: var(--space-sm); }
  .k-row.total .k { color: var(--amber-warm); }
  .cap-note {
    color: var(--amber-faint);
    font-style: italic;
    font-size: var(--text-2xs);
    margin-top: var(--space-sm);
  }
</style>
