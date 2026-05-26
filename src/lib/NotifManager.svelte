<script lang="ts">
  // Phase 8.7h — notif tab manager popout (§10.7 capability-driven UI
  // made user-discoverable).
  //
  // Right-click on a notif tab in the strip already toggles its `enabled`
  // field, but the gesture isn't discoverable. This popout surfaces the
  // same control as a list with checkboxes + a reset button.
  //
  // Architecture: stateless. App.svelte owns `notifs` $state; we receive
  // a `getTabs()` getter and toggle/reset callbacks, then render a
  // $derived view of the tabs. Each toggle bounces back to App.svelte
  // via the callback, which mutates notifs, which re-runs getTabs().

  import type { NotifTabSummary } from './popouts.svelte';
  import { popouts } from './popouts.svelte';

  interface Props {
    /** Id of the enclosing PopoutEntry — used to dismiss this popout. */
    popoutId: number;
    /** Reactive getter — returns current notifs state every read. */
    getTabs: () => NotifTabSummary[];
    /** Toggle a tab's `enabled` field. */
    onToggle: (id: string) => void;
    /** Reset all tabs to default (enabled). */
    onReset: () => void;
  }

  let { popoutId, getTabs, onToggle, onReset }: Props = $props();

  // $derived re-evaluates whenever the underlying notifs $state changes
  // (App.svelte reassigns immutably, which Svelte 5 deeply tracks).
  const tabs = $derived(getTabs());

  function dismiss(): void {
    popouts.dismiss(popoutId);
  }

  function onResetClick(): void {
    onReset();
  }

  // Counts for the footer summary
  const enabledCount = $derived(tabs.filter((t) => t.enabled).length);
  const detectedCount = $derived(tabs.filter((t) => t.detected).length);
</script>

<div class="manager"
     onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); dismiss(); } }}
     role="dialog"
     tabindex="-1"
>
  <div class="manager-section">
    <div class="manager-section-label">
      Visible notification tabs
      <span class="manager-section-hint">
        right-click any tab in the strip to toggle the same way
      </span>
    </div>
    <ul class="manager-list" role="list">
      {#each tabs as tab (tab.id)}
        <li class="manager-row" class:disabled={!tab.enabled} class:undetected={!tab.detected}>
          <label class="manager-toggle">
            <input
              type="checkbox"
              checked={tab.enabled}
              onchange={() => onToggle(tab.id)}
              aria-label="toggle {tab.title}"
            />
            <span class="manager-icon">{tab.icon}</span>
            <span class="manager-title">{tab.title}</span>
            {#if !tab.detected}
              <span class="manager-status">integration not loaded</span>
            {/if}
          </label>
        </li>
      {/each}
    </ul>
  </div>

  <div class="manager-footer">
    <span class="manager-summary">
      {enabledCount}/{tabs.length} enabled · {detectedCount} integrations detected
    </span>
    <div class="manager-actions">
      <button type="button" class="manager-btn manager-btn-secondary" onclick={onResetClick}>
        Reset
      </button>
      <button type="button" class="manager-btn manager-btn-primary" onclick={dismiss}>
        Done
      </button>
    </div>
  </div>
</div>

<style>
  .manager {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-family: var(--font-family);
    color: var(--amber-warm);
  }

  .manager-section {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-14) 18px var(--space-12);
  }
  .manager-section-label {
    font-size: var(--text-xs);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    margin-bottom: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .manager-section-hint {
    font-size: var(--text-2xs);
    letter-spacing: 0.06em;
    color: var(--amber-faint);
    text-transform: none;
    font-style: italic;
  }

  .manager-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .manager-row {
    background: transparent;
    border: 1px solid var(--border-subtle);
    transition: background 0.12s, border-color 0.12s;
  }
  .manager-row:hover {
    border-color: var(--amber-faint);
    background: rgba(255, 168, 38, 0.06);
  }
  .manager-row.disabled .manager-title {
    color: var(--amber-faint);
    text-decoration: line-through;
  }
  .manager-row.undetected {
    opacity: 0.7;
  }

  .manager-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-12);
    padding: var(--space-8) var(--space-12);
    cursor: pointer;
    user-select: none;
  }
  .manager-toggle input[type="checkbox"] {
    width: 14px;
    height: var(--space-14);
    accent-color: var(--amber-bright);
    cursor: pointer;
  }
  .manager-icon {
    font-size: var(--text-lg);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    min-width: 18px;
    text-align: center;
  }
  .manager-title {
    flex: 1;
    color: var(--amber-warm);
    font-size: var(--text-base);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .manager-status {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
    font-style: italic;
  }

  .manager-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-md) 18px;
    border-top: 1px solid var(--border-subtle);
    gap: var(--space-12);
  }
  .manager-summary {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
  }
  .manager-actions {
    display: flex;
    gap: var(--space-8);
  }
  .manager-btn {
    padding: 5px var(--space-14);
    font-family: inherit;
    font-size: var(--text-sm);
    letter-spacing: 0.1em;
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s, background 0.12s;
  }
  .manager-btn-secondary {
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-dim);
  }
  .manager-btn-secondary:hover {
    border-color: var(--amber-warm);
    color: var(--amber-warm);
  }
  .manager-btn-primary {
    background: transparent;
    border: 1px solid var(--amber-bright);
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .manager-btn-primary:hover {
    background: rgba(255, 200, 64, 0.08);
    box-shadow: var(--glow-amber-faint);
  }
</style>
