<script lang="ts">
  // ErrorResultPopout.svelte — Phase 5 / R1 result surface.
  //
  // The badge-anchored surface that shows the outcome of an error→agent
  // "explain" invocation. None existed before (ControlActions is tab-bound), so
  // this is a new, pane-scoped overlay: it reads the invocation's state straight
  // from the action registry (isPending → resultFor), so it works no matter
  // which provider answered. Mounted inside Terminal.svelte's relative
  // terminal-host, anchored bottom-right.
  //
  // States: pending ("explaining locally…") → ok (the explanation) | error (the
  // degrade notice, e.g. no local model loaded). Always dismissable.

  import { actionRegistry } from './actionRegistry.svelte';
  import type { FailureContext } from './errorHandoff';

  interface Props {
    actionId: string;
    failure: FailureContext;
    onDismiss: () => void;
  }

  let { actionId, failure, onDismiss }: Props = $props();

  const pending = $derived(actionRegistry.isPending(actionId));
  const result = $derived(actionRegistry.resultFor(actionId));
  const isError = $derived(result?.status === 'error');

  // Truncate the command for the header so a long pipeline doesn't blow out the
  // card width; the full command stays available via title.
  const cmdShort = $derived(
    failure.command.length > 64 ? failure.command.slice(0, 63) + '…' : failure.command,
  );
</script>

<div class="err-popout" role="dialog" aria-label="Error explanation" aria-live="polite">
  <header class="err-head">
    <span class="err-mark" aria-hidden="true">✗</span>
    <span class="err-code">{failure.exitCode}</span>
    <code class="err-cmd" title={failure.command}>{cmdShort}</code>
    <button type="button" class="err-dismiss" onclick={onDismiss} aria-label="Dismiss explanation" title="Dismiss">✕</button>
  </header>

  <div class="err-body">
    {#if pending && !result}
      <div class="err-pending">
        <span class="spinner" aria-hidden="true"></span>
        <span>Explaining locally…</span>
      </div>
    {:else if result}
      {#if isError}
        <p class="err-degrade">{result.message}</p>
      {:else}
        <p class="err-explanation">{result.message}</p>
      {/if}
    {:else}
      <p class="err-degrade">No explanation available.</p>
    {/if}
  </div>

  <footer class="err-foot">
    <span class="err-foot-note">
      {#if pending && !result}local model · nothing leaves this machine
      {:else if isError}offline · nothing was sent
      {:else}explained by a local model{/if}
    </span>
    <button type="button" class="err-ack" onclick={onDismiss}>Dismiss</button>
  </footer>
</div>

<style>
  .err-popout {
    position: absolute;
    right: var(--space-md);
    bottom: var(--space-md);
    z-index: 40;
    width: min(440px, calc(100% - 2 * var(--space-md)));
    max-height: 60%;
    display: flex;
    flex-direction: column;
    background-color: var(--bg-panel);
    background-image: var(--grain);
    border: 1px solid var(--border-active);
    border-left: 2px solid var(--term-red);
    border-radius: var(--radius-md, 6px);
    box-shadow: 0 10px 32px rgba(0, 0, 0, 0.6);
    font-family: var(--font-family);
    overflow: hidden;
  }

  .err-head {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    background: linear-gradient(180deg, rgba(255, 72, 72, 0.12), rgba(255, 72, 72, 0.02));
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }
  .err-mark {
    color: var(--term-red);
    font-weight: 700;
  }
  .err-code {
    color: var(--term-red);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
  }
  .err-cmd {
    flex: 1;
    min-width: 0;
    color: var(--amber-warm);
    font-size: var(--text-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .err-dismiss {
    flex-shrink: 0;
    background: transparent;
    border: none;
    color: var(--amber-dim);
    cursor: pointer;
    font-size: var(--text-sm);
    line-height: 1;
    padding: 2px 4px;
    border-radius: var(--radius-sm);
    transition: color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }
  .err-dismiss:hover {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.08);
  }

  .err-body {
    padding: var(--space-md);
    overflow-y: auto;
    min-height: 0;
  }
  .err-body::-webkit-scrollbar { width: 5px; }
  .err-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .err-pending {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    color: var(--amber-dim);
    font-size: var(--text-sm);
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--amber-faint);
    border-top-color: var(--amber-bright);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .err-explanation {
    margin: 0;
    color: var(--text-primary);
    font-size: var(--text-sm);
    line-height: 1.5;
    white-space: pre-wrap;
  }
  .err-degrade {
    margin: 0;
    color: var(--amber-dim);
    font-size: var(--text-sm);
    line-height: 1.5;
  }

  .err-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-md);
    border-top: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }
  .err-foot-note {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    letter-spacing: 0.03em;
  }
  .err-ack {
    background: transparent;
    border: 1px solid var(--border-subtle);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 2px 10px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: color var(--duration-fast) var(--ease-out), border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }
  .err-ack:hover {
    color: var(--amber-bright);
    border-color: var(--amber-dim);
    background: rgba(255, 200, 64, 0.06);
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner { animation-duration: 2s; }
  }
</style>
