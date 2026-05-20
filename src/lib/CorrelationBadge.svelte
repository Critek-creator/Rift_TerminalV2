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
  <button
    class="corr-badge"
    onclick={() => (expanded = !expanded)}
    title="show {count - 1} related event{count - 1 === 1 ? '' : 's'}"
  >
    +{count - 1}
  </button>
  {#if expanded}
    <div class="corr-flyout">
      <div class="corr-flyout-title">correlated events ({count})</div>
      {#each related as r (r.ts + ':' + r.kind)}
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
    background: rgba(108, 182, 255, 0.12);
    border: 1px solid rgba(108, 182, 255, 0.3);
    color: var(--term-blue);
    font-family: var(--font-family);
    font-size: 9px;
    font-weight: 600;
    padding: 0 4px;
    border-radius: 3px;
    cursor: pointer;
    line-height: 14px;
    margin-left: 4px;
    flex-shrink: 0;
  }
  .corr-badge:hover {
    background: rgba(108, 182, 255, 0.2);
    border-color: var(--term-blue);
  }
  .corr-flyout {
    position: absolute;
    right: 8px;
    z-index: 20;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    padding: 8px 10px;
    min-width: 240px;
    max-width: 360px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
  }
  .corr-flyout-title {
    color: var(--amber-warm);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    margin-bottom: 6px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border-subtle);
  }
  .corr-event {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 0;
    font-size: 10px;
  }
  .corr-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .corr-ts {
    color: var(--amber-faint);
    font-size: 9px;
    flex-shrink: 0;
  }
  .corr-cat {
    font-weight: 600;
    font-size: 9px;
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
