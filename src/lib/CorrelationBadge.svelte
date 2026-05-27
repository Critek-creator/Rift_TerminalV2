<script lang="ts">
  import type { Envelope } from './bus';
  import type { CorrelationIndex } from './correlationIndex';

  interface Props {
    env: Envelope;
    index: CorrelationIndex;
  }

  let { env, index }: Props = $props();

  const count = $derived(index.chainSize(env.correlation_id));
  let expanded = $state(false);
  const related = $derived(expanded ? index.getRelated(env) : []);

  const CAT_COLOR: Record<string, string> = {
    pty:      'var(--term-white)',
    hook:     'var(--term-cyan)',
    agent:    'var(--term-purple)',
    fs:       'var(--amber-faint)',
    index:    'var(--term-blue)',
    aegis:    'var(--amber-primary)',
    status:   'var(--amber-bright)',
    system:   'var(--term-red)',
    mcp:      'var(--term-purple)',
    sentinel: 'var(--term-red)',
  };

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }
</script>

{#if count > 1}
  <button type="button"
    class="corr-badge"
    onclick={() => (expanded = !expanded)}
    aria-expanded={expanded}
    aria-label="{count - 1} correlated event{count - 1 === 1 ? '' : 's'}"
    title="show {count - 1} related event{count - 1 === 1 ? '' : 's'}"
  >
    +{count - 1}
  </button>
  {#if expanded}
    <div class="corr-flyout">
      <div class="corr-flyout-title">correlated events ({count})</div>
      {#each related as r, i (r.ts + ':' + r.kind + ':' + i)}
        <div class="corr-event">
          <span class="corr-dot" style="background: {CAT_COLOR[r.category] ?? 'var(--amber-dim)'}"></span>
          <span class="corr-ts">{formatTs(r.ts)}</span>
          <span class="corr-cat" style="color: {CAT_COLOR[r.category] ?? 'var(--amber-dim)'}">{r.category}</span>
          <span class="corr-kind">{r.kind}</span>
        </div>
      {/each}
    </div>
  {/if}
{/if}

<style>
  .corr-badge {
    display: inline-flex;
    align-items: center;
    background: var(--bg-blue-tint);
    border: 1px solid var(--border-blue-tint);
    color: var(--term-blue);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 600;
    padding: 0 var(--space-xs);
    border-radius: 3px;
    cursor: pointer;
    line-height: 14px;
    margin-left: var(--space-xs);
    flex-shrink: 0;
  }
  .corr-badge:hover {
    background: var(--bg-blue-tint-hover);
    border-color: var(--term-blue);
  }
  .corr-flyout {
    position: absolute;
    right: 8px;
    z-index: 20;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    padding: var(--space-8) var(--space-md);
    min-width: 240px;
    max-width: 360px;
    box-shadow: var(--shadow-flyout);
  }
  .corr-flyout-title {
    color: var(--amber-warm);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    margin-bottom: var(--space-sm);
    padding-bottom: var(--space-xs);
    box-shadow: var(--sep-depth);
  }
  .corr-event {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: 2px 0;
    font-size: var(--text-xs);
  }
  .corr-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .corr-ts {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    flex-shrink: 0;
  }
  .corr-cat {
    font-weight: 600;
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }
  .corr-kind {
    color: var(--amber-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
