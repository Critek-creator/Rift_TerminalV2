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
  import { popouts } from './popouts.svelte';

  // ---------------------------------------------------------------------------
  // Types
  // ---------------------------------------------------------------------------

  interface ProjectEntry {
    name: string;
    path: string; // PathBuf serializes to string over Tauri IPC
    last_used_ms: number;
  }

  interface RiftConfig {
    projects: ProjectEntry[];
    fs: { ignore_globs: string[]; max_depth: number };
    cockpit: { detached_pos: null | { x: number; y: number; width: number; height: number } };
  }

  // ---------------------------------------------------------------------------
  // Props
  // ---------------------------------------------------------------------------

  interface Props {
    /** Id of the enclosing PopoutEntry — used to dismiss this popout. */
    popoutId: number;
  }

  let { popoutId }: Props = $props();

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
      await invoke('project_swap', { path: path.trim() });
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
      <div class="picker-empty">No recent projects</div>
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
            <span class="picker-item-name">{entry.name}</span>
            <span class="picker-item-path">{entry.path}</span>
            <span class="picker-item-time">{formatRelative(entry.last_used_ms)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- Manual path input -->
  <div class="picker-section picker-section-input">
    <div class="picker-section-label">Open a different path</div>
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
  </div>

  <!-- Error display -->
  {#if error !== null}
    <div class="picker-error">
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
  .picker {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-family: 'JetBrains Mono', monospace;
    color: var(--amber-warm);
    background: var(--bg-elevated);
  }

  /* Sections */
  .picker-section {
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }
  .picker-section-input {
    padding-bottom: 14px;
  }

  .picker-section-label {
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--amber-faint);
    margin-bottom: 8px;
  }

  /* Recent list */
  .picker-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow-y: auto;
  }
  .picker-list::-webkit-scrollbar { width: 4px; }
  .picker-list::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .picker-item {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    cursor: pointer;
    border-radius: 2px;
    border: 1px solid transparent;
    transition: border-color 0.1s, background 0.1s;
  }
  .picker-item:hover {
    background: rgba(245, 158, 11, 0.06);
    border-color: var(--amber-faint);
  }

  .picker-item-name {
    font-size: 11px;
    font-weight: 600;
    color: var(--amber-warm);
    white-space: nowrap;
  }
  .picker-item-path {
    font-size: 9px;
    color: var(--amber-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .picker-item-time {
    font-size: 9px;
    color: var(--amber-faint);
    white-space: nowrap;
    font-style: italic;
  }

  /* Loading / empty states */
  .picker-loading {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0;
    color: var(--amber-faint);
    font-size: 11px;
    font-style: italic;
  }
  .picker-loading-glyph { font-size: 16px; opacity: 0.5; }

  .picker-empty {
    color: var(--amber-faint);
    font-size: 11px;
    font-style: italic;
    padding: 4px 0;
  }

  /* Input */
  .picker-input {
    width: 100%;
    background: var(--bg-base);
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    padding: 6px 8px;
    outline: none;
    box-sizing: border-box;
    caret-color: var(--amber-bright);
  }
  .picker-input:focus {
    border-color: var(--amber-primary);
    box-shadow: 0 0 0 1px var(--amber-faint);
  }
  .picker-input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .picker-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }

  /* Error */
  .picker-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 16px;
    color: var(--term-red);
    font-size: 10px;
    border-bottom: 1px solid var(--border-subtle);
  }
  .picker-error-glyph { flex-shrink: 0; }
  .picker-error-msg {
    color: var(--amber-dim);
    word-break: break-all;
    line-height: 1.4;
  }

  /* Actions */
  .picker-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
  }

  .picker-btn {
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    letter-spacing: 0.08em;
    font-weight: 600;
    padding: 4px 12px;
    cursor: pointer;
    border-radius: 0;
  }
  .picker-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .picker-btn-cancel {
    background: transparent;
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
  }
  .picker-btn-cancel:not(:disabled):hover {
    border-color: var(--amber-faint);
    color: var(--amber-warm);
  }

  .picker-btn-confirm {
    background: var(--amber-bright);
    border: 1px solid var(--amber-bright);
    color: var(--bg-base);
  }
  .picker-btn-confirm:not(:disabled):hover {
    box-shadow: var(--glow-amber-strong);
  }
</style>
