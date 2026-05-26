<script lang="ts">
  // Phase 6.7 — project-picker popout (§11 "Easy project swap via menu").
  //
  // Props:
  //   popoutId — id of the enclosing PopoutEntry; used for programmatic dismiss.
  //
  // UX:
  //   - Lists recent projects (sorted by last_used DESC) as clickable rows.
  //   - "Type a path" input for a project not yet in the recent list.
  //   - Confirm swaps via the input; Cancel closes without swapping.
  //   - Clicking a recent-project row swaps immediately (no confirm needed).
  //   - Esc → e.stopPropagation() + dismiss (pr003 popout-keydown-bubble-collision).
  //
  // Keyboard: Esc is consumed here with stopPropagation so it does NOT also
  // fire Popout.svelte's window-level listener (which would double-dismiss).

  import { invoke } from '@tauri-apps/api/core';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { popouts } from './popouts.svelte';
  import type { RiftConfig, ProjectEntry } from './riftConfig';

  // ---------------------------------------------------------------------------
  // Props
  // ---------------------------------------------------------------------------

  interface Props {
    /** Id of the enclosing PopoutEntry — used to dismiss this popout. */
    popoutId: number;
    /** When set, the picker calls this instead of invoke('project_swap').
     *  Used by project-per-tab to open a project in a new tab. */
    onSelect?: (path: string) => void;
  }

  let { popoutId, onSelect }: Props = $props();

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg = $state<RiftConfig | null>(null);
  let newPath = $state('');
  let error = $state<string | null>(null);
  let busy = $state(false);

  // ---------------------------------------------------------------------------
  // Load config on mount (sync-shell + IIFE pattern — pr003)
  // ---------------------------------------------------------------------------

  $effect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const loaded = await invoke<RiftConfig>('config_get');
        if (!cancelled) cfg = loaded;
      } catch (e: unknown) {
        if (!cancelled) error = String(e);
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  /** Recent projects already sorted DESC by last_used_ms from the backend;
   *  re-sort defensively in case the config arrives unsorted. */
  const recentProjects = $derived(
    cfg
      ? [...cfg.projects].sort((a, b) => b.last_used_ms - a.last_used_ms)
      : [],
  );

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  async function swapToPath(path: string): Promise<void> {
    if (busy || !path.trim()) return;
    busy = true;
    error = null;
    try {
      if (onSelect) {
        onSelect(path.trim());
      } else {
        await invoke('project_swap', { path: path.trim() });
      }
      popouts.dismiss(popoutId);
    } catch (e: unknown) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function onConfirm(): void {
    void swapToPath(newPath);
  }

  function onCancel(): void {
    popouts.dismiss(popoutId);
  }

  function onRecentClick(entry: ProjectEntry): void {
    void swapToPath(entry.path);
  }

  /**
   * Phase 8.7d — open the OS native folder picker via @tauri-apps/plugin-dialog.
   * Selecting a directory writes its path to `newPath`; the user can then click
   * Open (or hit Enter) to swap, or edit the path before confirming. Cancel in
   * the picker is a silent no-op (open() returns null).
   */
  async function onBrowse(): Promise<void> {
    if (busy) return;
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: 'Select project folder',
      });
      if (typeof selected === 'string' && selected.length > 0) {
        newPath = selected;
        error = null;
      }
    } catch (e: unknown) {
      error = `Folder picker failed: ${String(e)}`;
    }
  }

  // ---------------------------------------------------------------------------
  // Keyboard — Esc must stopPropagation (pr003 popout-keydown-bubble-collision)
  // ---------------------------------------------------------------------------

  function onKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation(); // prevent Popout.svelte's window listener from also firing
      popouts.dismiss(popoutId);
    }
  }

  // ---------------------------------------------------------------------------
  // Format helpers
  // ---------------------------------------------------------------------------

  /** Format a unix-epoch-ms timestamp as a human-readable relative string. */
  function formatRelative(ms: number): string {
    const diff = Date.now() - ms;
    const sec = Math.floor(diff / 1000);
    if (sec < 60) return 'just now';
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    const day = Math.floor(hr / 24);
    return `${day}d ago`;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="picker"
  role="dialog"
  aria-label="switch project"
  tabindex={-1}
  onkeydown={onKeyDown}
>
  <!-- Recent projects list -->
  <div class="picker-section">
    <div class="picker-section-label">Recent projects</div>
    {#if cfg === null && error === null}
      <div class="picker-loading">
        <span class="picker-loading-glyph">◈</span>
        <span>loading…</span>
      </div>
    {:else if recentProjects.length === 0}
      <div class="picker-empty">
        <span class="picker-empty-icon">◇</span>
        <span class="picker-empty-text">No recent projects</span>
        <span class="picker-empty-hint">use the path input below to open a project</span>
      </div>
    {:else}
      <ul class="picker-list" role="listbox" aria-label="recent projects">
        {#each recentProjects as entry (entry.path)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            class="picker-item"
            role="option"
            aria-selected="false"
            onclick={() => onRecentClick(entry)}
          >
            <span class="picker-item-icon">▦</span>
            <span class="picker-item-name">{entry.name}</span>
            <span class="picker-item-path">{entry.path}</span>
            <span class="picker-item-time">{formatRelative(entry.last_used_ms)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- Manual path input + native folder picker -->
  <div class="picker-section picker-section-input">
    <div class="picker-section-label">Open a different path</div>
    <div class="picker-input-row">
      <input
        class="picker-input"
        type="text"
        placeholder="e.g. C:\Users\me\my-project or /home/me/my-project"
        bind:value={newPath}
        disabled={busy}
        aria-label="project path"
        onkeydown={(e) => {
          if (e.key === 'Enter') onConfirm();
          // Let Esc bubble up to the outer onKeyDown handler (same element).
        }}
      />
      <button
        type="button"
        class="picker-browse"
        onclick={onBrowse}
        disabled={busy}
        aria-label="browse for project folder"
        title="Browse… (native folder picker)"
      >▦ Browse…</button>
    </div>
  </div>

  <!-- Error display -->
  {#if error !== null}
    <div class="picker-error" role="alert">
      <span class="picker-error-glyph">◇</span>
      <span class="picker-error-msg">{error}</span>
    </div>
  {/if}

  <!-- Actions -->
  <div class="picker-actions">
    <button
      type="button"
      class="picker-btn picker-btn-cancel"
      onclick={onCancel}
      disabled={busy}
    >Cancel</button>
    <button
      type="button"
      class="picker-btn picker-btn-confirm"
      onclick={onConfirm}
      disabled={busy || !newPath.trim()}
    >{busy ? 'Switching…' : 'Open'}</button>
  </div>
</div>

<style>
  /* ─── Shell ──────────────────────────────────────────────────────────── */
  .picker {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-family: var(--font-family);
    color: var(--amber-warm);
    background: var(--bg-elevated);
  }

  /* ─── Sections ───────────────────────────────────────────────────────── */
  .picker-section {
    padding: var(--space-lg);
    box-shadow: var(--sep-depth);
  }
  .picker-section-input {
    padding-bottom: var(--space-lg);
  }

  .picker-section-label {
    font-size: var(--type-section-size);
    letter-spacing: var(--type-section-spacing);
    text-transform: uppercase;
    font-weight: var(--type-section-weight);
    color: var(--amber-faint);
    margin-bottom: var(--space-md);
    padding-bottom: var(--space-sm);
    box-shadow: var(--sep-depth);
  }

  /* ─── Recent list ────────────────────────────────────────────────────── */
  .picker-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 260px;
    overflow-y: auto;
  }

  .picker-item {
    display: grid;
    grid-template-columns: 20px auto 1fr auto;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-md);
    cursor: pointer;
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    margin-bottom: 2px;
    transition: border-color var(--duration-fast) var(--ease-out),
                background var(--duration-fast) var(--ease-out);
  }
  .picker-item:hover {
    background: var(--bg-hover);
    border-color: var(--border-subtle);
  }
  .picker-item:active {
    background: var(--bg-surface);
    border-color: var(--amber-dim);
  }

  .picker-item-icon {
    color: var(--amber-faint);
    font-size: var(--text-base);
    text-align: center;
    transition: color var(--duration-fast) var(--ease-out);
  }
  .picker-item:hover .picker-item-icon {
    color: var(--amber-bright);
  }
  .picker-item-name {
    font-size: var(--type-body-size);
    font-weight: 600;
    color: var(--amber-warm);
    white-space: nowrap;
    transition: color var(--duration-fast) var(--ease-out);
  }
  .picker-item:hover .picker-item-name {
    color: var(--amber-bright);
  }
  .picker-item-path {
    font-size: var(--type-caption-size);
    color: var(--amber-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    letter-spacing: var(--type-caption-spacing);
  }
  .picker-item-time {
    font-size: var(--type-caption-size);
    color: var(--amber-faint);
    white-space: nowrap;
    font-style: italic;
    opacity: 0.7;
    transition: opacity var(--duration-fast) var(--ease-out);
  }
  .picker-item:hover .picker-item-time {
    opacity: 1;
  }

  /* ─── Loading / empty states ─────────────────────────────────────────── */
  .picker-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-md);
    padding: var(--space-xl) 0;
    color: var(--amber-faint);
    font-size: var(--text-sm);
    font-style: italic;
  }
  .picker-loading-glyph {
    font-size: 18px;
    animation: picker-pulse 1.4s ease-in-out infinite;
  }
  @keyframes picker-pulse {
    0%, 100% { opacity: 0.3; text-shadow: none; }
    50%       { opacity: 1;   text-shadow: var(--glow-amber); }
  }

  .picker-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
    padding: var(--space-2xl) 0;
    color: var(--amber-faint);
  }
  .picker-empty-icon {
    font-size: 24px;
    opacity: 0.4;
  }
  .picker-empty-text {
    font-size: var(--text-sm);
    font-style: italic;
    letter-spacing: 0.04em;
  }
  .picker-empty-hint {
    font-size: var(--text-2xs);
    opacity: 0.7;
  }

  /* ─── Path input row ─────────────────────────────────────────────────── */
  .picker-input-row {
    display: flex;
    align-items: stretch;
    height: 36px;
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .picker-input {
    flex: 1;
    min-width: 0;
    background: var(--bg-base);
    border: 1px solid var(--border-active);
    border-right: none;
    border-radius: var(--radius-md) 0 0 var(--radius-md);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    padding: 0 var(--space-md);
    outline: none;
    box-sizing: border-box;
    caret-color: var(--amber-bright);
    transition: border-color var(--duration-base) var(--ease-out),
                box-shadow var(--duration-med) var(--ease-out);
  }
  .picker-input:focus {
    border-color: var(--amber-primary);
    box-shadow: 0 0 0 1px var(--amber-dim), var(--glow-amber);
    border-right: 1px solid var(--amber-primary);
  }
  .picker-input:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .picker-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }

  .picker-browse {
    flex: 0 0 auto;
    background: var(--bg-panel);
    border: 1px solid var(--border-active);
    border-radius: 0 var(--radius-md) var(--radius-md) 0;
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 0 var(--space-lg);
    cursor: pointer;
    white-space: nowrap;
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out);
    height: 36px;
    line-height: 36px;
  }
  .picker-browse:not(:disabled):hover {
    color: var(--amber-warm);
    border-color: var(--amber-primary);
    background: var(--bg-hover);
  }
  .picker-browse:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ─── Error display ──────────────────────────────────────────────────── */
  .picker-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-lg);
    background: rgba(255, 72, 72, 0.05);
    box-shadow: inset 0 -1px 0 0 var(--term-red), 0 1px 3px rgba(255, 72, 72, 0.15);
    font-size: var(--text-xs);
  }
  .picker-error-glyph {
    flex-shrink: 0;
    color: var(--term-red);
  }
  .picker-error-msg {
    color: var(--amber-dim);
    word-break: break-all;
    line-height: 1.5;
  }

  /* ─── Action buttons ─────────────────────────────────────────────────── */
  .picker-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-lg);
    background: var(--bg-panel);
    box-shadow: var(--sep-glow);
  }

  .picker-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-family);
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    font-weight: 700;
    height: 34px;
    padding: 0 var(--space-lg);
    cursor: pointer;
    border-radius: var(--radius-md);
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out),
                box-shadow var(--duration-base) var(--ease-out);
    text-transform: uppercase;
    user-select: none;
  }
  .picker-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .picker-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .picker-btn-cancel {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
  }
  .picker-btn-cancel:not(:disabled):hover {
    border-color: var(--amber-dim);
    color: var(--amber-bright);
    background: var(--bg-hover);
    box-shadow: 0 0 4px rgba(255, 168, 38, 0.1);
  }

  .picker-btn-confirm {
    background: var(--amber-bright);
    border: 1px solid var(--amber-bright);
    color: var(--bg-base);
    font-weight: 700;
  }
  .picker-btn-confirm:not(:disabled):hover {
    box-shadow: var(--glow-amber);
  }
</style>
