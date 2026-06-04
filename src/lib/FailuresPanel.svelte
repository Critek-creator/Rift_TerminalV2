<script lang="ts">
  // FailuresPanel.svelte — Phase 5 / R1.5: the persistent issues list.
  //
  // Opened from the status-line "✗ N" chip. Lists recent shell-command
  // failures (commandFailureStore) that have scrolled out of the terminal, and
  // lets the user click any row to explain it INLINE — reusing the same local-
  // only provider + action registry as the in-line badge, so the privacy
  // guarantee (nothing leaves the machine) and result rendering are identical.

  import { commandFailureStore } from './commandFailureStore.svelte';
  import { actionRegistry } from './actionRegistry.svelte';
  import { errorActionId, ERROR_EXPLAIN_ACTION } from './errorHandoff';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  // Mark everything acknowledged the moment the panel opens (clears the chip).
  commandFailureStore.acknowledgeAll();

  let expandedId = $state<string | null>(null);
  // rowId → the per-failure unique action id used for its explain invocation.
  const actionIds = new Map<string, string>();
  let explainSeq = 0;

  function ensureExplain(rowId: string): void {
    if (actionIds.has(rowId)) return;
    const ctx = commandFailureStore.contextFor(rowId);
    if (!ctx) return;
    // pane -2 = "issues list" namespace, keeps ids distinct from pane badges.
    const actionId = errorActionId(ERROR_EXPLAIN_ACTION, -2, ++explainSeq);
    actionIds.set(rowId, actionId);
    void actionRegistry
      .invoke({ id: actionId, target: 'failures', label: 'explain error' }, ctx)
      .catch((err) => console.warn('[FailuresPanel] explain invoke failed', err));
  }

  function toggle(rowId: string): void {
    if (expandedId === rowId) {
      expandedId = null;
      return;
    }
    expandedId = rowId;
    ensureExplain(rowId);
  }

  function clockTime(ts: number): string {
    return new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }

  function shortCmd(cmd: string): string {
    return cmd.length > 80 ? cmd.slice(0, 79) + '…' : cmd;
  }
</script>

<div class="failures-panel" role="dialog" aria-label="Command failures">
  <header class="fp-head">
    <span class="fp-title">Command failures</span>
    <span class="fp-count">{commandFailureStore.count}</span>
    <span class="fp-spacer"></span>
    {#if commandFailureStore.count > 0}
      <button type="button" class="fp-clear" onclick={() => commandFailureStore.clear()}>clear all</button>
    {/if}
    <button type="button" class="fp-close" onclick={onclose} aria-label="Close failures list" title="Close">✕</button>
  </header>

  {#if commandFailureStore.entries.length === 0}
    <div class="fp-empty">
      No command failures yet. When a command exits non-zero, it'll appear here —
      click to explain it with a local model.
    </div>
  {:else}
    <ul class="fp-list">
      {#each commandFailureStore.entries as e (e.id)}
        {@const actionId = actionIds.get(e.id)}
        {@const result = actionId ? actionRegistry.resultFor(actionId) : undefined}
        {@const pending = actionId ? actionRegistry.isPending(actionId) : false}
        <li class="fp-row" class:expanded={expandedId === e.id}>
          <div class="fp-row-head">
            <button type="button" class="fp-row-btn" onclick={() => toggle(e.id)} aria-expanded={expandedId === e.id}>
              <span class="fp-mark" aria-hidden="true">✗</span>
              <span class="fp-code">{e.exitCode}</span>
              <code class="fp-cmd" title={e.command}>{shortCmd(e.command)}</code>
              {#if e.repeatCount > 1}<span class="fp-repeat" title="consecutive identical failures">×{e.repeatCount}</span>{/if}
              <span class="fp-time">{clockTime(e.ts)}</span>
              <span class="fp-chevron" aria-hidden="true">{expandedId === e.id ? '▾' : '▸'}</span>
            </button>
            <button type="button" class="fp-row-dismiss" onclick={() => commandFailureStore.remove(e.id)} aria-label="Remove this failure" title="Remove">✕</button>
          </div>

          {#if expandedId === e.id}
            <div class="fp-detail">
              {#if e.cwd}<div class="fp-cwd" title={e.cwd}>{e.cwd}</div>{/if}
              {#if pending && !result}
                <div class="fp-pending"><span class="fp-spinner" aria-hidden="true"></span> Explaining locally…</div>
              {:else if result}
                <p class="fp-explanation" class:degrade={result.status === 'error'}>{result.message}</p>
              {/if}
              <span class="fp-privacy">
                {#if result?.status === 'error'}offline · nothing was sent
                {:else}local model · nothing leaves this machine{/if}
              </span>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .failures-panel {
    position: fixed;
    left: var(--space-md);
    bottom: 52px; /* clears the two-row status line + mode-hint bar */
    z-index: 60;
    width: min(560px, calc(100vw - 2 * var(--space-md)));
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background-color: var(--bg-panel);
    background-image: var(--grain);
    border: 1px solid var(--border-active);
    border-left: 2px solid var(--term-red);
    border-radius: var(--radius-md, 6px);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.65);
    font-family: var(--font-family);
    overflow: hidden;
  }

  .fp-head {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    background: linear-gradient(180deg, rgba(255, 72, 72, 0.10), rgba(255, 72, 72, 0.015));
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }
  .fp-title {
    color: var(--amber-bright);
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.06em;
  }
  .fp-count {
    color: var(--term-red);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
    background: rgba(255, 72, 72, 0.12);
    border-radius: var(--radius-sm);
    padding: 0 6px;
  }
  .fp-spacer { flex: 1; }
  .fp-clear, .fp-close {
    background: transparent;
    border: 1px solid transparent;
    color: var(--amber-dim);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    transition: color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }
  .fp-clear:hover, .fp-close:hover { color: var(--amber-bright); background: rgba(255, 200, 64, 0.08); }

  .fp-empty {
    padding: var(--space-lg) var(--space-md);
    color: var(--amber-dim);
    font-size: var(--text-sm);
    line-height: 1.5;
  }

  .fp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
  }
  .fp-list::-webkit-scrollbar { width: 6px; }
  .fp-list::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .fp-row { border-bottom: 1px solid var(--border-subtle); }
  .fp-row.expanded { background: rgba(255, 200, 64, 0.03); }

  .fp-row-head { display: flex; align-items: stretch; }
  .fp-row-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    padding: var(--space-sm) var(--space-md);
    font-family: inherit;
    color: var(--text-primary);
    transition: background var(--duration-fast) var(--ease-out);
  }
  .fp-row-btn:hover { background: rgba(255, 200, 64, 0.05); }
  .fp-mark { color: var(--term-red); font-weight: 700; }
  .fp-code { color: var(--term-red); font-weight: 700; font-variant-numeric: tabular-nums; font-size: var(--text-xs); }
  .fp-cmd {
    flex: 1;
    min-width: 0;
    color: var(--amber-warm);
    font-size: var(--text-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fp-repeat {
    color: var(--term-red);
    font-size: var(--text-2xs);
    font-weight: 700;
    background: rgba(255, 72, 72, 0.12);
    border-radius: var(--radius-sm);
    padding: 0 4px;
    flex-shrink: 0;
  }
  .fp-time { color: var(--amber-faint); font-size: var(--text-2xs); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .fp-chevron { color: var(--amber-dim); font-size: var(--text-2xs); flex-shrink: 0; }
  .fp-row-dismiss {
    flex-shrink: 0;
    background: transparent;
    border: none;
    color: var(--amber-faint);
    cursor: pointer;
    padding: 0 var(--space-sm);
    font-size: var(--text-xs);
    transition: color var(--duration-fast) var(--ease-out);
  }
  .fp-row-dismiss:hover { color: var(--term-red); }

  .fp-detail {
    padding: 0 var(--space-md) var(--space-md) calc(var(--space-md) + 18px);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .fp-cwd {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fp-pending { display: flex; align-items: center; gap: var(--space-sm); color: var(--amber-dim); font-size: var(--text-sm); }
  .fp-spinner {
    width: 12px; height: 12px;
    border: 2px solid var(--amber-faint);
    border-top-color: var(--amber-bright);
    border-radius: 50%;
    animation: fp-spin 0.7s linear infinite;
    flex-shrink: 0;
  }
  @keyframes fp-spin { to { transform: rotate(360deg); } }
  .fp-explanation {
    margin: 0;
    color: var(--text-primary);
    font-size: var(--text-sm);
    line-height: 1.5;
    white-space: pre-wrap;
  }
  .fp-explanation.degrade { color: var(--amber-dim); }
  .fp-privacy { color: var(--amber-faint); font-size: var(--text-2xs); letter-spacing: 0.03em; }

  @media (prefers-reduced-motion: reduce) {
    .fp-spinner { animation-duration: 2s; }
  }
</style>
