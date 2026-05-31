<script lang="ts">
  // Wave 6c — Feature Pipeline tab. Surfaces the Abyssal Feature Agent idea
  // store as a studio-wide pipeline: per-project counts, tier funnel, and a
  // leverage ranking (impact > effort > occurrence) over the decision-support
  // fields the generator now emits. Read-only; invoke-based (feature_pipeline_scan)
  // on mount + Refresh. Renders an empty-state card when the store is absent.

  import { onMount, flushSync } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { NOTIF_TAB_MIME } from './dragMime';

  interface Props {
    onDragBack?: () => void;
  }
  let { onDragBack }: Props = $props();

  type Stage = 'critical' | 'high' | 'medium' | 'low' | 'untriaged' | 'buried';
  interface PipelineEntry {
    id: string;
    title: string;
    target: string;
    stage: Stage;
    value_class: string | null;
    effort: string | null;
    impact: string | null;
    occurrence: number;
    promoted: boolean;
    first_seen: string | null;
  }

  const STAGE_ORDER: Stage[] = ['critical', 'high', 'medium', 'low', 'untriaged', 'buried'];
  const STAGE_COLOR: Record<Stage, string> = {
    critical: 'var(--term-red)',
    high: 'var(--amber-bright)',
    medium: 'var(--amber-warm)',
    low: 'var(--amber-faint)',
    untriaged: 'var(--amber-dim)',
    buried: 'var(--term-purple)',
  };

  const IMPACT_RANK: Record<string, number> = { high: 3, med: 2, low: 1 };
  const EFFORT_RANK: Record<string, number> = { S: 3, M: 2, L: 1 };

  let entries = $state<PipelineEntry[]>([]);
  let scanning = $state(false);
  let lastScanTs = $state<number | null>(null);
  let scanError = $state<string | null>(null);
  let activeProject = $state<string>('ALL');
  let activeStage = $state<Stage | 'ALL'>('ALL');
  let expandedId = $state<string | null>(null);

  const projects = $derived.by(() => {
    const h = new Map<string, number>();
    for (const e of entries) h.set(e.target, (h.get(e.target) ?? 0) + 1);
    return [...h.entries()].sort((a, b) => b[1] - a[1]);
  });

  const stageCounts = $derived.by(() => {
    const h: Record<Stage, number> = { critical: 0, high: 0, medium: 0, low: 0, untriaged: 0, buried: 0 };
    const scope = activeProject === 'ALL' ? entries : entries.filter((e) => e.target === activeProject);
    for (const e of scope) h[e.stage] += 1;
    return h;
  });

  const filtered = $derived.by(() => {
    let list = entries;
    if (activeProject !== 'ALL') list = list.filter((e) => e.target === activeProject);
    if (activeStage !== 'ALL') list = list.filter((e) => e.stage === activeStage);
    // Leverage ranking: impact desc, then effort (S best) desc, then occurrence desc.
    return [...list].sort((a, b) => {
      const im = (IMPACT_RANK[b.impact ?? ''] ?? 0) - (IMPACT_RANK[a.impact ?? ''] ?? 0);
      if (im !== 0) return im;
      const ef = (EFFORT_RANK[b.effort ?? ''] ?? 0) - (EFFORT_RANK[a.effort ?? ''] ?? 0);
      if (ef !== 0) return ef;
      return b.occurrence - a.occurrence;
    });
  });

  const totalCount = $derived(entries.length);
  const filteredCount = $derived(filtered.length);
  const actionable = $derived(stageCounts.critical + stageCounts.high);

  async function runScan() {
    scanning = true;
    scanError = null;
    try {
      const result = await invoke<PipelineEntry[]>('feature_pipeline_scan');
      // flushSync defensively: this assignment + the resulting re-render run in
      // the async continuation AFTER the await, so any error thrown during the
      // render (e.g. a Svelte each_key_duplicate from a bad store, as ALL+ALL
      // once hit when a candidate existed in both candidates/ and graveyard/)
      // is thrown in the reactive flush microtask, OUTSIDE this try's reach — it
      // silently wedges the scheduler and tears the pane instead of surfacing.
      // Forcing the flush here pulls any such error back into this try/catch so
      // it shows as "scan failed — …" rather than a frozen pane. Batched with the
      // scanning reset so the whole post-scan state lands in one pass.
      flushSync(() => {
        entries = result;
        lastScanTs = Date.now();
        scanning = false;
      });
    } catch (err) {
      scanError = String(err);
      console.error('[FeaturePipeline] feature_pipeline_scan failed', err);
      flushSync(() => {
        scanning = false;
      });
    }
  }

  onMount(() => {
    void runScan();
  });

  function toggleExpand(id: string) {
    expandedId = expandedId === id ? null : id;
  }

  function leverageBadge(e: PipelineEntry): string {
    const vc = e.value_class ? e.value_class : '—';
    const ef = e.effort ?? '?';
    const im = e.impact ?? '?';
    return `${vc} · ${ef}/${im}`;
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
      <span class="handle-glyph">⬢</span>
      <span class="handle-title">pipeline</span>
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
    <span class="title"><span class="icon">⬢</span>FEATURE PIPELINE</span>
    <span class="state">
      {filteredCount}/{totalCount} candidate{totalCount === 1 ? '' : 's'} · {projects.length} project{projects.length === 1 ? '' : 's'} · {actionable} actionable · {formatScanLabel()}
    </span>
    <span class="spacer"></span>
    <button type="button" class="ctl-btn" onclick={runScan} disabled={scanning}>
      {scanning ? '…' : 'rescan'}
    </button>
  </header>

  <div class="strip">
    <span class="strip-label">PROJECT</span>
    <div class="filter-row">
      <button type="button" class="chip" class:active={activeProject === 'ALL'} onclick={() => (activeProject = 'ALL')}>
        all <span class="chip-n">{totalCount}</span>
      </button>
      {#each projects as [proj, n] (proj)}
        <button type="button" class="chip" class:active={activeProject === proj} onclick={() => (activeProject = proj)}>
          {proj} <span class="chip-n">{n}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="strip">
    <span class="strip-label">STAGE</span>
    <div class="filter-row">
      <button type="button" class="chip" class:active={activeStage === 'ALL'} onclick={() => (activeStage = 'ALL')}>
        all
      </button>
      {#each STAGE_ORDER as s (s)}
        <button
          type="button"
          class="chip"
          class:active={activeStage === s}
          style="--chip-color: {STAGE_COLOR[s]};"
          onclick={() => (activeStage = s)}
        >
          {s} <span class="chip-n">{stageCounts[s]}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="log">
    <div class="log-header">CANDIDATES · ranked by leverage (impact → effort → occurrence) · click to expand</div>
    <div class="log-body" aria-live="polite">
      {#if scanError}
        <div class="empty error">scan failed — {scanError}</div>
      {:else if scanning && entries.length === 0}
        <div class="empty">scanning feature-agent store…</div>
      {:else if totalCount === 0}
        <div class="empty">no feature-agent store found (run /abyssal-feature-agent to populate it)</div>
      {:else if filtered.length === 0}
        <div class="empty">no candidates match the current filters</div>
      {:else}
        {#each filtered as e (e.target + '::' + e.stage + '::' + e.id)}
          {@const open = expandedId === e.id}
          <div class="row" class:row-open={open}>
            <button type="button" class="row-main" onclick={() => toggleExpand(e.id)} title="expand details">
              <span class="stage" style="color: {STAGE_COLOR[e.stage]};">{e.stage}</span>
              <span class="ttitle">{e.title}</span>
              <span class="lev">{leverageBadge(e)}</span>
              <span class="meta">
                {#if e.occurrence > 1}<span class="occ" title="occurrence count">×{e.occurrence}</span>{/if}
                {#if e.promoted}<span class="badge promoted" title="promoted to plan">planned</span>{/if}
                <span class="proj">{e.target}</span>
              </span>
            </button>
            {#if open}
              <div class="detail">
                <div class="d-row"><span class="d-k">id</span><span class="d-v">{e.id}</span></div>
                <div class="d-row"><span class="d-k">value class</span><span class="d-v">{e.value_class ?? '(unset)'}</span></div>
                <div class="d-row"><span class="d-k">effort / impact</span><span class="d-v">{e.effort ?? '?'} / {e.impact ?? '?'}</span></div>
                <div class="d-row"><span class="d-k">occurrence</span><span class="d-v">{e.occurrence}</span></div>
                <div class="d-row"><span class="d-k">promoted</span><span class="d-v">{e.promoted ? 'yes' : 'no'}</span></div>
                {#if e.first_seen}<div class="d-row"><span class="d-k">first seen</span><span class="d-v">{e.first_seen}</span></div>{/if}
              </div>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">FUNNEL{activeProject === 'ALL' ? '' : ` · ${activeProject}`}</div>
    <div class="state-body">
      {#each STAGE_ORDER as s (s)}
        <div class="k-row">
          <span class="k" style="color: {STAGE_COLOR[s]};">{s}</span>
          <span class="v">{stageCounts[s]}</span>
        </div>
      {/each}
      <div class="k-row total">
        <span class="k">actionable (high + critical)</span>
        <span class="v">{actionable}</span>
      </div>
    </div>
  </footer>
</section>

<style>
  .pane {
    flex: 1; display: flex; flex-direction: column; min-height: 0;
    background: var(--bg-base); color: var(--amber-warm);
    font-family: var(--font-family); font-size: var(--text-base);
  }
  .drag-handle {
    height: var(--control-sm); padding: 0 var(--space-12);
    background: var(--bg-surface); box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-md);
    cursor: grab; user-select: none; color: var(--amber-warm);
    font-size: var(--type-label-size); letter-spacing: var(--type-label-spacing);
    font-weight: var(--type-label-weight); transition: background var(--duration-base) ease-out;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -2px; }
  .drag-handle .handle-glyph { color: var(--amber-bright); font-size: var(--text-base); text-shadow: var(--glow-amber-faint); }
  .drag-handle .handle-title { color: var(--amber-bright); text-transform: uppercase; }

  .status {
    height: 36px; padding: 0 var(--space-lg); background: var(--bg-elevated);
    box-shadow: var(--sep-glow); display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
  }
  .status .title { font-size: var(--type-section-size); font-weight: var(--type-section-weight); letter-spacing: var(--type-section-spacing); color: var(--amber-bright); text-shadow: var(--glow-amber-faint); }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; font-size: var(--text-lg); }
  .status .state { color: var(--amber-dim); font-size: var(--type-caption-size); font-weight: var(--type-caption-weight); letter-spacing: var(--type-caption-spacing); }
  .status .spacer { flex: 1; }
  .ctl-btn {
    background: transparent; border: 1px solid var(--amber-faint); color: var(--amber-warm);
    font-family: inherit; font-size: var(--text-2xs); letter-spacing: 0.1em; font-weight: 700;
    padding: 2px var(--space-8); cursor: pointer; text-transform: uppercase;
    transition: color var(--duration-base) ease-out, background var(--duration-base) ease-out, border-color var(--duration-base) ease-out, opacity var(--duration-base) ease-out;
  }
  .ctl-btn:hover:not(:disabled) { border-color: var(--amber-bright); color: var(--amber-bright); }
  .ctl-btn:focus-visible { outline: 1px solid var(--amber-bright); outline-offset: 1px; }
  .ctl-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .strip {
    min-height: 32px; padding: var(--space-xs) var(--space-14); box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-md);
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.05), transparent);
    color: var(--amber-dim); font-size: var(--type-caption-size); letter-spacing: var(--type-caption-spacing); flex-wrap: wrap;
  }
  .strip-label { color: var(--amber-bright); font-weight: 700; min-width: 52px; }
  .filter-row { display: flex; flex-wrap: wrap; gap: var(--space-sm); flex: 1; }
  .chip {
    display: inline-flex; align-items: center; gap: var(--space-sm); padding: 2px var(--space-8);
    border: 1px solid var(--chip-color, var(--amber-faint)); color: var(--chip-color, var(--amber-warm));
    background: transparent; font-family: inherit; font-size: var(--text-2xs); letter-spacing: 0.08em;
    font-weight: 700; cursor: pointer; text-transform: uppercase;
    transition: background var(--duration-base) ease-out, color var(--duration-base) ease-out;
  }
  .chip:hover { background: rgba(212, 137, 10, 0.06); }
  .chip:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: 1px; }
  .chip.active { background: var(--chip-color, var(--amber-bright)); color: var(--bg-base); }
  .chip-n { font-variant-numeric: tabular-nums; opacity: 0.7; }
  .chip.active .chip-n { opacity: 1; }

  .log { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .log-header {
    padding: var(--space-8) var(--space-lg); color: var(--amber-faint);
    font-size: var(--type-label-size); font-weight: var(--type-label-weight); letter-spacing: var(--type-label-spacing);
    text-transform: uppercase; box-shadow: var(--sep-depth); background: var(--bg-surface);
  }
  .log-body {
    flex: 1; overflow-y: auto; padding: var(--space-sm) var(--space-lg);
    color: var(--amber-warm); font-size: var(--text-sm); box-shadow: var(--depth-inset);
    line-height: 1.5; display: flex; flex-direction: column;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .empty { color: var(--amber-dim); font-size: var(--type-caption-size); font-style: italic; padding: var(--space-sm) 0; }
  .empty.error { color: var(--term-red); font-style: normal; font-size: var(--type-body-size); }

  .row {
    display: flex; flex-direction: column; border-left: 2px solid transparent; width: 100%;
    transition: background var(--duration-base) ease-out, border-left-color var(--duration-base) ease-out;
  }
  .row:hover { background: rgba(212, 137, 10, 0.06); border-left-color: var(--amber-bright); }
  .row.row-open { background: rgba(212, 137, 10, 0.04); border-left-color: var(--amber-warm); }
  .row-main {
    display: grid; grid-template-columns: 72px 1fr auto auto; gap: var(--space-12);
    align-items: baseline; padding: 3px var(--space-xs); background: transparent; border: none;
    color: inherit; font-family: inherit; text-align: left; cursor: pointer; min-width: 0;
  }
  .stage { font-weight: 700; font-size: var(--text-xs); letter-spacing: 0.06em; text-transform: uppercase; }
  .ttitle { color: var(--amber-warm); font-weight: 600; font-size: var(--text-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lev { color: var(--amber-dim); font-size: var(--text-2xs); letter-spacing: 0.04em; text-transform: lowercase; white-space: nowrap; }
  .meta { display: inline-flex; align-items: center; gap: var(--space-sm); white-space: nowrap; }
  .occ { color: var(--amber-bright); font-variant-numeric: tabular-nums; font-size: var(--text-2xs); font-weight: 700; }
  .proj { color: var(--amber-faint); font-size: var(--text-2xs); }
  .badge {
    display: inline-block; border: 1px solid var(--amber-faint); border-radius: 3px;
    padding: 0 var(--space-xs); font-size: var(--text-2xs); letter-spacing: 0.06em; text-transform: uppercase;
  }
  .badge.promoted { color: var(--term-green); border-color: var(--term-green); }
  .detail {
    padding: var(--space-sm) var(--space-lg) var(--space-sm) var(--space-xl);
    display: flex; flex-direction: column; gap: 2px; background: var(--bg-panel);
  }
  .d-row { display: flex; justify-content: space-between; font-size: var(--text-2xs); letter-spacing: 0.04em; }
  .d-k { color: var(--amber-faint); text-transform: uppercase; }
  .d-v { color: var(--amber-warm); font-variant-numeric: tabular-nums; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 60%; }

  .state-panel {
    flex-shrink: 0; background: var(--bg-panel); max-height: 200px; overflow-y: auto;
    box-shadow: var(--depth-lift), var(--depth-edge-light);
  }
  .state-header {
    padding: var(--space-8) var(--space-lg); color: var(--amber-faint);
    font-size: var(--type-label-size); font-weight: var(--type-label-weight); letter-spacing: var(--type-label-spacing);
    text-transform: uppercase; box-shadow: var(--sep-depth);
  }
  .state-body { padding: var(--space-md) var(--space-lg) var(--space-14); display: flex; flex-direction: column; gap: 3px; }
  .k-row { display: flex; align-items: center; justify-content: space-between; font-size: var(--text-xs); letter-spacing: 0.04em; }
  .k-row .k { font-weight: 700; text-transform: uppercase; }
  .k-row .v { color: var(--amber-bright); font-weight: 700; font-variant-numeric: tabular-nums; }
  .k-row.total { border-top: 1px solid var(--border-subtle); padding-top: var(--space-xs); margin-top: var(--space-xs); }
  .k-row.total .k { color: var(--amber-warm); }
</style>
