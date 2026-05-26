<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { subscribe, type Envelope } from './bus';

  interface Props {
    project?: string | null;
    cwd?: string | null;
  }

  let { project = null, cwd = null }: Props = $props();

  interface CommandStat {
    command: string;
    count: number;
    failures: number;
    last_used_ms: number;
  }

  interface CommandStats {
    top_commands: CommandStat[];
    total_commands: number;
    total_sessions: number;
  }

  let stats = $state<CommandStats | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let unsubscribeBus: (() => Promise<void>) | undefined;

  const failureHotspots = $derived.by(() => {
    if (!stats) return [];
    return stats.top_commands
      .filter((c) => c.count >= 3 && c.failures / c.count > 0.2)
      .sort((a, b) => b.failures / b.count - a.failures / a.count)
      .slice(0, 5);
  });

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

  function failureRate(c: CommandStat): number {
    return c.count > 0 ? (c.failures / c.count) * 100 : 0;
  }

  function failureColor(rate: number): string {
    if (rate > 50) return 'var(--term-red)';
    if (rate > 20) return 'var(--amber-primary)';
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
        if (env.kind === 'cmd.end') {
          const p = env.payload as { command?: string; exit_code?: number; cwd?: string } | null;
          if (p?.command) {
            invoke('command_history_record', {
              command: p.command,
              exitCode: p.exit_code ?? 0,
              cwd: p.cwd ?? null,
              project: project ?? null,
            }).catch(() => {});
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
</script>

<div class="cmd-intel-panel" aria-busy={loading}>
  <div class="section-header">
    <span class="section-title">COMMAND INTELLIGENCE</span>
    {#if project}
      <span class="project-badge">{project}</span>
    {/if}
  </div>

  {#if loading && !stats}
    <div class="empty-state">Loading...</div>
  {:else if error}
    <div class="empty-state error-text">{error}</div>
  {:else if !stats || stats.top_commands.length === 0}
    <div class="empty-state">
      <span class="empty-state-icon">⌘</span>
      <span class="empty-state-text">No command history yet</span>
      <span class="empty-state-hint">shell commands and exit codes will be tracked here</span>
    </div>
  {:else}
    <div class="stats-summary">
      <span>{stats.total_commands} commands</span>
      <span class="sep">/</span>
      <span>{stats.total_sessions} sessions</span>
    </div>

    <div class="chart-section">
      <div class="subsection-label">TOP COMMANDS</div>
      {#each stats.top_commands.slice(0, 8) as cmd}
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

    {#if failureHotspots.length > 0}
      <div class="chart-section">
        <div class="subsection-label">FAILURE HOTSPOTS</div>
        {#each failureHotspots as cmd}
          <div class="failure-row">
            <span class="failure-cmd" title={cmd.command}>{truncateCmd(cmd.command, 30)}</span>
            <span class="failure-stats">
              <span style:color={failureColor(failureRate(cmd))}>
                {failureRate(cmd).toFixed(0)}%
              </span>
              <span class="failure-detail">({cmd.failures}/{cmd.count})</span>
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
    gap: var(--space-sm);
    padding: var(--space-md);
    height: 100%;
    overflow-y: auto;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--section-header-padding);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .section-title {
    font-size: var(--section-header-size);
    font-weight: var(--section-header-weight);
    letter-spacing: var(--section-header-spacing);
    color: var(--amber-bright);
    text-transform: uppercase;
  }

  .project-badge {
    font-size: var(--text-2xs);
    padding: 1px 6px;
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    color: var(--amber-dim);
    text-transform: uppercase;
  }

  .stats-summary {
    font-size: var(--text-xs);
    color: var(--amber-faint);
    padding: 0 var(--space-sm);
  }

  .stats-summary .sep {
    margin: 0 var(--space-xs);
    opacity: 0.4;
  }

  .chart-section {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: var(--space-sm);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .subsection-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    color: var(--amber-faint);
    text-transform: uppercase;
    margin-bottom: var(--space-xs);
  }

  .bar-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 20px;
  }

  .bar-label {
    flex: 0 0 140px;
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
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
    transition: width var(--duration-base) var(--ease-out);
    min-width: 2px;
  }

  .bar-count {
    flex: 0 0 32px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--amber-dim);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
  }

  .failure-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 0;
  }

  .failure-cmd {
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    color: var(--amber-warm);
  }

  .failure-stats {
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    font-weight: 600;
  }

  .failure-detail {
    color: var(--amber-faint);
    margin-left: var(--space-xs);
    font-weight: 400;
  }

  .empty-state {
    font-size: var(--text-sm);
    color: var(--amber-faint);
    text-align: center;
    padding: var(--space-xl) var(--space-md);
  }

  .error-text {
    color: var(--term-red);
  }
</style>
