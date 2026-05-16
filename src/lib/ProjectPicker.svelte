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
  /* ─── Shell ──────────────────────────────────────────────────────────── */
  .picker {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-family: 'JetBrains Mono', monospace;
    color: var(--amber-warm);
    background: var(--bg-elevated, #14140F);
  }

  /* ─── Sections ───────────────────────────────────────────────────────── */
  .picker-section {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }
  .picker-section-input {
    padding-bottom: 16px;
  }

  .picker-section-label {
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--amber-faint);
    margin-bottom: 10px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--border-subtle);
  }

  /* ─── Recent list ────────────────────────────────────────────────────── */
  .picker-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 232px;
    overflow-y: auto;
  }
  .picker-list::-webkit-scrollbar { width: 4px; }
  .picker-list::-webkit-scrollbar-thumb {
    background: var(--amber-faint);
    border-radius: 2px;
  }
  .picker-list::-webkit-scrollbar-track { background: transparent; }

  .picker-item {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    cursor: pointer;
    border-left: 2px solid transparent;
    border-bottom: 1px solid transparent;
    transition: border-color 0.1s, background 0.1s, color 0.1s;
  }
  .picker-item:last-child { border-bottom: none; }
  .picker-item:hover {
    background: var(--bg-hover, #1a1a14);
    border-left-color: var(--amber-dim, #D8A028);
  }
  .picker-item:active {
    background: var(--bg-surface, #0F0F0D);
    border-left-color: var(--amber-bright, #FFC840);
  }

  .picker-item-name {
    font-size: 11px;
    font-weight: 700;
    color: var(--amber-warm);
    white-space: nowrap;
    transition: color 0.1s;
  }
  .picker-item:hover .picker-item-name {
    color: var(--amber-bright);
  }
  .picker-item-path {
    font-size: 9px;
    color: var(--amber-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    letter-spacing: 0.02em;
  }
  .picker-item-time {
    font-size: 9px;
    color: var(--amber-faint);
    white-space: nowrap;
    font-style: italic;
    opacity: 0.8;
  }
  .picker-item:hover .picker-item-time {
    opacity: 1;
  }

  /* ─── Loading / empty states ─────────────────────────────────────────── */
  .picker-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 20px 0;
    color: var(--amber-faint);
    font-size: 11px;
    font-style: italic;
  }
  .picker-loading-glyph {
    font-size: 18px;
    animation: picker-pulse 1.4s ease-in-out infinite;
  }
  @keyframes picker-pulse {
    0%, 100% { opacity: 0.3; text-shadow: none; }
    50%       { opacity: 1;   text-shadow: var(--glow-amber, 0 0 8px rgba(255, 168, 38, 0.55)); }
  }

  .picker-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px 0;
    color: var(--amber-faint);
    font-size: 11px;
    font-style: italic;
    letter-spacing: 0.04em;
  }

  /* ─── Path input row ─────────────────────────────────────────────────── */
  /* Browse button is flush against the input — no gap, shared border. */
  .picker-input-row {
    display: flex;
    align-items: stretch;
    height: 36px;
  }

  .picker-input {
    flex: 1;
    min-width: 0;
    background: var(--bg-surface, #0F0F0D);
    border: 1px solid var(--border-active, #4a3818);
    border-right: none;
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    padding: 0 10px;
    outline: none;
    box-sizing: border-box;
    caret-color: var(--amber-bright);
    transition: border-color 0.12s, box-shadow 0.15s;
  }
  .picker-input:focus {
    border-color: var(--amber-primary, #FFA826);
    box-shadow: 0 0 0 1px var(--amber-dim, #D8A028),
                var(--glow-amber, 0 0 8px rgba(255, 168, 38, 0.55));
    /* restore right side so glow shows through */
    border-right: 1px solid var(--amber-primary, #FFA826);
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
    background: var(--bg-panel, #0c0c0a);
    border: 1px solid var(--border-active, #4a3818);
    color: var(--amber-dim);
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    padding: 0 14px;
    cursor: pointer;
    white-space: nowrap;
    transition: color 0.12s, border-color 0.12s, background 0.12s, transform 0.1s;
    height: 36px;
    line-height: 36px;
  }
  .picker-browse:not(:disabled):hover {
    color: var(--amber-warm);
    border-color: var(--amber-primary, #FFA826);
    background: var(--bg-hover, #1a1a14);
    transform: translateY(-1px);
  }
  .picker-browse:not(:disabled):active {
    transform: translateY(0);
  }
  .picker-browse:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ─── Error display ──────────────────────────────────────────────────── */
  .picker-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 16px;
    background: rgba(204, 51, 51, 0.06);
    border-bottom: 1px solid var(--term-red, #FF4848);
    font-size: 10px;
  }
  .picker-error-glyph {
    flex-shrink: 0;
    color: var(--term-red, #FF4848);
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
    gap: 8px;
    padding: 12px 16px;
    background: var(--bg-panel, #0c0c0a);
    border-top: 1px solid var(--border-subtle);
  }

  .picker-btn {
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    letter-spacing: 0.08em;
    font-weight: 700;
    height: 34px;
    padding: 0 16px;
    line-height: 34px;
    cursor: pointer;
    border-radius: 0;
    transition: color 0.12s, border-color 0.12s, background 0.12s,
                box-shadow 0.12s, transform 0.1s;
    text-transform: uppercase;
  }
  .picker-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .picker-btn-cancel {
    background: transparent;
    border: 1px solid var(--border-active, #4a3818);
    color: var(--amber-dim);
  }
  .picker-btn-cancel:not(:disabled):hover {
    border-color: var(--amber-faint);
    color: var(--amber-warm);
    transform: translateY(-1px);
  }
  .picker-btn-cancel:not(:disabled):active {
    transform: translateY(0);
  }

  .picker-btn-confirm {
    background: var(--amber-bright, #FFC840);
    border: 1px solid var(--amber-bright, #FFC840);
    color: var(--bg-base, #080806);
    font-weight: 800;
  }
  .picker-btn-confirm:not(:disabled):hover {
    box-shadow: var(--glow-amber-strong, 0 0 14px rgba(255, 200, 64, 0.85));
    transform: translateY(-1px);
  }
  .picker-btn-confirm:not(:disabled):active {
    transform: translateY(0);
    box-shadow: var(--glow-amber, 0 0 8px rgba(255, 168, 38, 0.55));
  }
</style>
