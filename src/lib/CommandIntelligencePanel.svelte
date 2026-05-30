<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { subscribe, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';

  interface Props {
    project?: string | null;
    cwd?: string | null;
    onDragBack?: () => void;
  }

  let { project = null, cwd = null, onDragBack }: Props = $props();

  interface CommandFrequency {
    command: string;
    count: number;
    failure_rate: number;
  }

  interface CommandStats {
    total_count: number;
    top_commands: CommandFrequency[];
    failure_hotspots: CommandFrequency[];
  }

  let stats = $state<CommandStats | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let unsubscribeBus: (() => Promise<void>) | undefined;

  const maxCount = $derived(
    stats ? Math.max(1, ...stats.top_commands.map((c) => c.count)) : 1,
  );

  async function fetchStats() {
    loading = true;
    error = null;
    try {
      stats = await invoke<CommandStats>('command_stats', {
        project: project ?? null,
        cwd: cwd ?? null,
      });
    } catch (e) {
      error = String(e);
      stats = null;
    } finally {
      loading = false;
    }
  }

  function failureColor(rate: number): string {
    if (rate > 0.5) return 'var(--term-red)';
    if (rate > 0.2) return 'var(--amber-primary)';
    return 'var(--amber-faint)';
  }

  function truncateCmd(cmd: string, max = 40): string {
    return cmd.length > max ? cmd.slice(0, max - 1) + '…' : cmd;
  }

  $effect(() => {
    void project;
    void cwd;
    let cancelled = false;
    (async () => {
      loading = true;
      try {
        const result = await invoke<CommandStats>('command_stats', {
          project: project ?? null,
          cwd: cwd ?? null,
        });
        if (!cancelled) { stats = result; loading = false; }
      } catch {
        if (!cancelled) loading = false;
      }
    })();
    return () => { cancelled = true; };
  });

  onMount(async () => {
    try {
      unsubscribeBus = await subscribe({ category: 'pty' }, (env: Envelope) => {
        if (env.kind === 'command.submitted') {
          const p = env.payload as { command?: string; session_id?: number } | null;
          if (p?.command) {
            invoke('command_history_record', {
              record: {
                command: p.command,
                cwd: cwd ?? '',
                project: project ?? null,
                started_at: new Date().toISOString(),
                duration_ms: null,
                exit_code: null,
                lane: null,
              },
            }).catch((err) => console.error('[CmdIntel] record failed:', err));
          }
          clearTimeout(refreshTimer);
          refreshTimer = setTimeout(fetchStats, 500);
        }
      });
    } catch { /* bus not ready yet */ }
  });

  let refreshTimer: ReturnType<typeof setTimeout> | undefined;

  onDestroy(() => {
    clearTimeout(refreshTimer);
    void unsubscribeBus?.().catch(() => {});
  });

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }
</script>

<div class="cmd-intel-panel" aria-busy={loading}>
  {#if onDragBack}
    <div
      class="drag-handle"
      role="button"
      tabindex="0"
      draggable={true}
      ondragstart={onHandleDragStart}
      title="drag back to tab strip to dock"
    >
      <span class="handle-glyph">◇</span>
      <span class="handle-title">ANALYTICS</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  <div class="section-header">
    <span class="section-title">COMMAND INTELLIGENCE</span>
    {#if project}
      <span class="project-badge">{project}</span>
    {/if}
  </div>

  {#if loading && !stats}
    <div class="empty-state">
      <span class="empty-state-icon">◇</span>
      <span class="empty-state-text">Loading command history…</span>
    </div>
  {:else if error}
    <div class="empty-state">
      <span class="empty-state-icon">◇</span>
      <span class="empty-state-text error-text">{error}</span>
    </div>
  {:else if !stats || stats.top_commands.length === 0}
    <div class="empty-state">
      <span class="empty-state-icon">⌘</span>
      <span class="empty-state-text">No command history yet</span>
      <span class="empty-state-hint">shell commands will be tracked here as you work</span>
    </div>
  {:else}
    <div class="stats-summary">
      <span>{stats.total_count} command{stats.total_count === 1 ? '' : 's'} recorded</span>
    </div>

    <div class="chart-section">
      <div class="subsection-label">TOP COMMANDS</div>
      {#each stats.top_commands.slice(0, 8) as cmd (cmd.command)}
        <div class="bar-row">
          <span class="bar-label" title={cmd.command}>{truncateCmd(cmd.command)}</span>
          <div class="bar-track">
            <div
              class="bar-fill"
              style:width="{(cmd.count / maxCount) * 100}%"
            ></div>
          </div>
          <span class="bar-count">{cmd.count}</span>
        </div>
      {/each}
    </div>

    {#if stats.failure_hotspots.length > 0}
      <div class="chart-section">
        <div class="subsection-label">FAILURE HOTSPOTS</div>
        {#each stats.failure_hotspots as cmd (cmd.command)}
          <div class="failure-row">
            <span class="failure-cmd" title={cmd.command}>{truncateCmd(cmd.command, 30)}</span>
            <span class="failure-stats">
              <span style:color={failureColor(cmd.failure_rate)}>
                {(cmd.failure_rate * 100).toFixed(0)}%
              </span>
              <span class="failure-detail">fail</span>
            </span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .cmd-intel-panel {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    min-width: 0;
    height: 100%;
    background: var(--bg-base);
    color: var(--amber-primary);
    font-family: var(--font-family);
    font-size: var(--text-base);
  }

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
  .drag-handle:active { cursor: grabbing; background: var(--bg-hover); }
  .drag-handle:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .drag-handle .handle-glyph {
    color: var(--amber-primary);
    font-size: var(--text-md);
    text-shadow: 0 0 6px rgba(255, 168, 38, 0.35);
  }
  .drag-handle .handle-title {
    color: var(--amber-primary);
    text-transform: uppercase;
    text-shadow: 0 0 4px rgba(255, 168, 38, 0.2);
  }
  .drag-handle .handle-hint {
    margin-left: auto;
    color: var(--amber-faint);
    font-style: italic;
    font-weight: 400;
    letter-spacing: 0.04em;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--section-header-padding);
    background: linear-gradient(to bottom, var(--bg-elevated), var(--bg-surface));
    border-left: 3px solid var(--amber-primary);
    box-shadow: var(--sep-glow);
    flex-shrink: 0;
  }

  .section-title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--amber-primary);
    text-shadow: 0 0 8px rgba(255, 168, 38, 0.35);
    text-transform: uppercase;
  }

  .project-badge {
    font-size: var(--text-2xs);
    padding: 1px var(--space-sm);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    color: var(--amber-dim);
    text-transform: uppercase;
  }

  .stats-summary {
    font-size: var(--text-xs);
    color: var(--amber-dim);
    padding: var(--space-sm) var(--space-lg);
    background: var(--bg-surface);
    border-left: 3px solid transparent;
    flex-shrink: 0;
  }

  .chart-section {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: var(--space-12) var(--space-lg);
    background: var(--bg-surface);
  }

  .subsection-label {
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    color: var(--amber-faint);
    text-transform: uppercase;
    margin-bottom: var(--space-xs);
    border-left: 3px solid var(--amber-primary);
    padding-left: var(--space-sm);
    background: linear-gradient(to right, rgba(212, 137, 10, 0.06), transparent);
  }

  .bar-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 20px;
    padding: 0 var(--space-xs);
    border-radius: var(--radius-sm);
    transition: background var(--duration-base);
  }
  .bar-row:hover { background: rgba(212, 137, 10, 0.06); }

  .bar-label {
    flex: 0 0 140px;
    font-size: var(--text-xs);
    font-family: var(--font-family);
    color: var(--amber-warm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .bar-track {
    flex: 1;
    height: 10px;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    background: linear-gradient(90deg, var(--amber-faint), var(--amber-primary));
    border-radius: var(--radius-sm);
    transition: width var(--duration-base);
    min-width: 2px;
  }

  .bar-count {
    flex: 0 0 32px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-variant-numeric: tabular-nums;
  }

  .failure-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px var(--space-xs);
    border-radius: var(--radius-sm);
    transition: background var(--duration-base);
  }
  .failure-row:hover { background: rgba(212, 137, 10, 0.06); }

  .failure-cmd {
    font-size: var(--text-xs);
    font-family: var(--font-family);
    color: var(--amber-warm);
  }

  .failure-stats {
    font-size: var(--text-xs);
    font-family: var(--font-family);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .failure-detail {
    color: var(--amber-faint);
    margin-left: var(--space-xs);
    font-weight: 400;
  }

  .empty-state {
    flex: 1;
  }
  .empty-state-icon {
    font-size: var(--text-2xl);
  }
  .error-text {
    color: var(--term-red);
  }
</style>
