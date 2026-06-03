<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { RiftConfig } from './riftConfig';
  import { defaultTimelineConfig } from './riftConfig';
  import { type TimelineEntry, sourceMeta, isSeekable, activeSourceKeys } from './sessionTimeline';

  interface Props {
    /** Session id (date-stem) to build the timeline for. */
    sessionId: string;
    /** Deep-link callback into the replay log for a seekable row. */
    onSeek: (eventIdx: number) => void;
  }

  let { sessionId, onSeek }: Props = $props();

  const LIMIT = 500;

  let entries = $state<TimelineEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let activeSources = $state<string[]>([]);

  async function load(id: string): Promise<void> {
    if (!id) return;
    loading = true;
    error = null;
    try {
      // CORE-by-default: start from the default source set, then overlay the
      // user's saved config (which may have opted extra sources in). If config
      // is unavailable, the CORE defaults (commands + errors) still apply.
      let sources: Record<string, boolean> = { ...defaultTimelineConfig() };
      try {
        const cfg = await invoke<RiftConfig>('config_get');
        if (cfg?.timeline) sources = { ...sources, ...cfg.timeline };
      } catch {
        /* config unavailable — fall back to CORE defaults */
      }
      activeSources = activeSourceKeys(sources);
      entries = await invoke<TimelineEntry[]>('session_timeline', {
        sessionId: id,
        sources,
        limit: LIMIT,
      });
    } catch (e) {
      error = String(e);
      entries = [];
    } finally {
      loading = false;
    }
  }

  // Reload whenever the selected session changes.
  $effect(() => {
    void load(sessionId);
  });

  function fmtTs(ms: number): string {
    const d = new Date(ms);
    const p = (n: number) => String(n).padStart(2, '0');
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }

  function fmtDuration(ms: number | null): string {
    if (ms === null || ms === undefined) return '';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`;
  }
</script>

<div class="timeline">
  <div class="tl-header">
    <span class="tl-title">SESSION TIMELINE</span>
    {#if !loading && !error}
      <span class="tl-state">
        {entries.length} event{entries.length === 1 ? '' : 's'}
        {#if activeSources.length > 0}· {activeSources.join(' · ')}{/if}
      </span>
    {/if}
  </div>

  <div class="tl-body" aria-busy={loading}>
    {#if loading}
      <div class="empty-card">
        <div class="empty-title">Loading…</div>
        <div class="empty-desc">Merging command history and the session log.</div>
      </div>
    {:else if error}
      <div class="empty-card">
        <div class="empty-title">Couldn't build the timeline</div>
        <div class="empty-desc">{error}</div>
      </div>
    {:else if entries.length === 0}
      <div class="empty-card">
        <div class="empty-title">Nothing to show</div>
        <div class="empty-desc">
          No events matched the enabled sources. Turn on more sources in
          Settings → Session Timeline, or this session may be empty.
        </div>
      </div>
    {:else}
      {#each entries as e, i (e.ts + ':' + e.event_idx + ':' + i)}
        {@const meta = sourceMeta(e.source)}
        {@const seekable = isSeekable(e.event_idx)}
        {@const failed = e.exit_code !== null && e.exit_code !== 0}
        <button
          type="button"
          class="tl-row"
          class:seekable
          class:failed
          disabled={!seekable}
          title={seekable ? 'jump to this event in the replay log' : 'no replay position (history-only row)'}
          onclick={() => { if (seekable) onSeek(e.event_idx); }}
        >
          <span class="tl-ts">{fmtTs(e.ts)}</span>
          <span class="tl-source" style="color: {meta.color}; border-color: {meta.color};">{meta.label}</span>
          <span class="tl-summary">{e.summary || e.kind}</span>
          {#if e.exit_code !== null}
            <span class="tl-exit" class:bad={failed}>exit {e.exit_code}</span>
          {/if}
          {#if e.duration_ms !== null}
            <span class="tl-dur">{fmtDuration(e.duration_ms)}</span>
          {/if}
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  .timeline {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }

  .tl-header {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }
  .tl-title {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--amber-faint);
    text-transform: uppercase;
  }
  .tl-state {
    font-size: var(--text-2xs);
    color: var(--amber-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tl-body {
    overflow-y: auto;
    padding: var(--space-xs) 0;
    min-height: 0;
  }

  .tl-row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-md);
    padding: var(--space-xs) var(--space-lg);
    cursor: default;
  }
  .tl-row.seekable {
    cursor: pointer;
  }
  .tl-row.seekable:hover {
    background: var(--bg-amber-hover, rgba(255, 200, 64, 0.06));
    border-left-color: var(--amber-warm);
  }
  .tl-row.seekable:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .tl-row.failed {
    border-left-color: var(--term-red, #cc3333);
  }

  .tl-ts {
    flex-shrink: 0;
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
  }
  .tl-source {
    flex-shrink: 0;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 0 var(--space-xs);
    min-width: 3.2em;
    text-align: center;
  }
  .tl-summary {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tl-exit {
    flex-shrink: 0;
    font-size: var(--text-2xs);
    color: var(--term-green, #33cc33);
    font-variant-numeric: tabular-nums;
  }
  .tl-exit.bad {
    color: var(--term-red, #cc3333);
  }
  .tl-dur {
    flex-shrink: 0;
    font-size: var(--text-2xs);
    color: var(--amber-dim);
    font-variant-numeric: tabular-nums;
  }

  .empty-card {
    padding: var(--space-3xl) var(--space-lg);
    text-align: center;
  }
  .empty-title {
    color: var(--amber-warm);
    font-size: var(--text-md);
    margin-bottom: var(--space-xs);
  }
  .empty-desc {
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    max-width: 42ch;
    margin: 0 auto;
  }
</style>
