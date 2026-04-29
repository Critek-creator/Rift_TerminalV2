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

  const filtered = $derived(
    activeMarker === 'ALL'
      ? entries
      : entries.filter((e) => e.marker === activeMarker)
  );
  const totalCount = $derived(entries.length);
  const filteredCount = $derived(filtered.length);

  const markerCounts = $derived.by(() => {
    const h: Record<Marker, number> = { TODO: 0, FIXME: 0, XXX: 0, HACK: 0 };
    for (const e of entries) h[e.marker] += 1;
    return h;
  });
  const fileCount = $derived.by(() => {
    const seen = new Set<string>();
    for (const e of entries) seen.add(e.path);
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
      <span class="handle-glyph">↙</span>
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
    <div class="log-body">
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
        {#each filtered as e, i (e.path + ':' + e.line + ':' + i)}
          <button
            type="button"
            class="row"
            onclick={() => openEntry(e)}
            title="open {e.path}"
          >
            <span class="marker" style="color: {MARKER_COLOR[e.marker]};">{e.marker}</span>
            <span class="path">{e.path}<span class="line-sep">:</span><span class="lineno">{e.line}</span></span>
            <span class="message">{e.message || '(no message)'}</span>
          </button>
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
      {#if totalCount >= 1000}
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
  }
  .ctl-btn:hover:not(:disabled) {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .ctl-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .strip {
    min-height: 32px;
    padding: 4px 14px;
    border-bottom: 1px solid var(--border-subtle);
    display: flex; align-items: center; gap: 10px;
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.04), transparent);
    color: var(--amber-dim);
    font-size: 10px;
    letter-spacing: 0.1em;
    flex-wrap: wrap;
  }
  .strip-label { color: var(--amber-bright); font-weight: 700; }
  .filter-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    flex: 1;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--chip-color, var(--amber-faint));
    color: var(--chip-color, var(--amber-warm));
    background: transparent;
    font-family: inherit;
    font-size: 9px;
    letter-spacing: 0.08em;
    font-weight: 700;
    cursor: pointer;
    text-transform: uppercase;
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
    padding: 4px 14px;
    color: var(--amber-warm);
    font-size: 11px;
    line-height: 1.5;
    display: flex;
    flex-direction: column;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .empty {
    color: var(--amber-faint);
    font-style: italic;
    padding: 6px 0;
  }
  .empty.error { color: var(--term-red); font-style: normal; }

  .row {
    display: grid;
    grid-template-columns: 60px 1fr 1.4fr;
    gap: 12px;
    align-items: baseline;
    padding: 3px 4px;
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: inherit;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    width: 100%;
  }
  .row:hover {
    background: rgba(212, 137, 10, 0.06);
    border-left-color: var(--amber-bright);
  }
  .marker {
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.06em;
  }
  .path {
    color: var(--amber-warm);
    font-weight: 600;
    font-size: 11px;
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
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .k-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 10px;
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
    padding-top: 4px;
    margin-top: 4px;
  }
  .k-row.total:nth-of-type(1) { border-top: 1px solid var(--border-subtle); margin-top: 6px; }
  .k-row.total .k { color: var(--amber-warm); }
  .cap-note {
    color: var(--amber-faint);
    font-style: italic;
    font-size: 9px;
    margin-top: 6px;
  }
</style>
