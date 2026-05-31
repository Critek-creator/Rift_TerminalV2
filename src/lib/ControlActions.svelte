<script lang="ts">
  // Generic render surface for §9 control endpoints (candidate 568). Drop
  // `<ControlActions target="<tab-id>" />` into any tab; it renders whatever
  // actions integrations have declared for that target, handles the optional
  // confirm step, invokes over the bus, and shows per-action result feedback.
  // Renders nothing when no actions are declared for the target, so it is safe
  // to place in every tab — it only appears when an integration lights it up.
  import { actionRegistry, type DeclaredAction } from './actionRegistry.svelte';

  let { target, label = 'ACTIONS' }: { target: string; label?: string } = $props();

  const actions = $derived(actionRegistry.actionsFor(target));

  let confirming = $state<string | null>(null);
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;

  function trigger(a: DeclaredAction): void {
    if (a.confirm && confirming !== a.id) {
      confirming = a.id;
      if (confirmTimer) clearTimeout(confirmTimer);
      confirmTimer = setTimeout(() => { confirming = null; }, 3000);
      return;
    }
    if (confirmTimer) clearTimeout(confirmTimer);
    confirming = null;
    void actionRegistry.invoke(a);
  }

  $effect(() => () => { if (confirmTimer) clearTimeout(confirmTimer); });
</script>

{#if actions.length > 0}
  <div class="control-actions">
    <span class="ca-label">{label}</span>
    {#each actions as a (a.id)}
      {@const pending = actionRegistry.isPending(a.id)}
      {@const result = actionRegistry.resultFor(a.id)}
      <div class="ca-item">
        <button
          type="button"
          class="ca-btn"
          class:danger={a.danger}
          class:confirming={confirming === a.id}
          disabled={pending}
          onclick={() => trigger(a)}
          title={a.description ?? a.label}
          aria-label={a.description ?? a.label}
        >
          {#if pending}…{:else if confirming === a.id}confirm?{:else}{a.label}{/if}
        </button>
        {#if result && !pending}
          <span class="ca-result {result.status}" title={result.message ?? result.status}>
            {result.status === 'ok' ? '✓' : '✗'}
          </span>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .control-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xs) var(--space-sm);
  }
  .ca-label {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    margin-right: var(--space-xs);
  }
  .ca-item {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .ca-btn {
    background: rgba(212, 137, 10, 0.08);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: lowercase;
    padding: 2px var(--space-8);
    cursor: pointer;
    transition: color var(--duration-base) ease-out, background var(--duration-base) ease-out,
      border-color var(--duration-base) ease-out, opacity var(--duration-base) ease-out;
  }
  .ca-btn:hover:not(:disabled) {
    color: var(--amber-bright);
    border-color: var(--amber-bright);
    background: rgba(212, 137, 10, 0.16);
  }
  .ca-btn:disabled { opacity: 0.6; cursor: default; }
  .ca-btn:focus-visible { outline: 1px solid var(--amber-bright); outline-offset: 1px; }
  .ca-btn.danger { color: var(--term-red); border-color: rgba(255, 72, 72, 0.5); }
  .ca-btn.danger:hover:not(:disabled) {
    background: rgba(255, 72, 72, 0.15);
    border-color: var(--term-red);
  }
  .ca-btn.confirming {
    color: var(--bg-base, #0a0a0a);
    background: var(--amber-primary);
    border-color: var(--amber-primary);
  }
  .ca-result { font-size: var(--text-xs); font-weight: 700; }
  .ca-result.ok { color: var(--term-green); }
  .ca-result.error { color: var(--term-red); }
</style>
