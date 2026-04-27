<script lang="ts">
  // Phase 6.5 — in-cockpit file viewer (§11).
  //
  // Props:
  //   path      — project-relative path, forwarded to fs_read_text / fs_write_text.
  //   popoutId  — id of the enclosing PopoutEntry; used for Esc-dismiss in view mode.
  //
  // Keyboard shortcuts (owned by this component, not the Popout chrome):
  //   Ctrl+E — enter edit mode (focuses textarea via $effect).
  //   Ctrl+S — save (no-op when content === originalContent).
  //   Esc    — if edit mode → discard + back to view; if view mode → dismiss popout.
  //
  // Scope (§11 HARD): syntax highlighting + quick edit → save only.
  // OUT OF SCOPE: multi-file ops, LSP, debug tooling, command palette, extensions.

  import { invoke } from '@tauri-apps/api/core';
  import { codeToHtml } from 'shiki';
  import { popouts } from './popouts.svelte';

  // ---------------------------------------------------------------------------
  // Props
  // ---------------------------------------------------------------------------

  interface Props {
    /** Project-relative file path (e.g. "src/lib.rs"). */
    path: string;
    /** Id of the enclosing PopoutEntry — used to dismiss in view-mode Esc. */
    popoutId: number;
  }

  let { path, popoutId }: Props = $props();

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let content = $state('');
  let originalContent = $state('');
  let mode = $state<'view' | 'edit'>('view');
  let saving = $state(false);
  let error = $state<string | null>(null);
  let savedFlash = $state(false);
  let highlightedHtml = $state<string>('');

  /** Reference to the textarea for focus-on-enter-edit. */
  let textareaEl = $state<HTMLTextAreaElement | null>(null);

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  const dirty = $derived(content !== originalContent);

  // ---------------------------------------------------------------------------
  // Language resolution
  // ---------------------------------------------------------------------------

  /**
   * Map a file extension to a Shiki language id.
   * Returns undefined for unknown extensions so the caller can fall back to
   * 'plaintext'. Keep this list minimal — §11 "no extensions" means we do NOT
   * add a plugin system here.
   */
  function langForExt(filePath: string): string | undefined {
    const ext = filePath.split('.').at(-1)?.toLowerCase();
    const map: Record<string, string> = {
      rs: 'rust',
      ts: 'typescript',
      tsx: 'tsx',
      js: 'javascript',
      jsx: 'jsx',
      svelte: 'svelte',
      json: 'json',
      toml: 'toml',
      yaml: 'yaml',
      yml: 'yaml',
      md: 'markdown',
      html: 'html',
      css: 'css',
      sh: 'bash',
      bash: 'bash',
      zsh: 'bash',
      py: 'python',
      go: 'go',
      c: 'c',
      cpp: 'cpp',
      h: 'cpp',
      hpp: 'cpp',
      lock: 'toml',
    };
    return ext !== undefined ? map[ext] : undefined;
  }

  // ---------------------------------------------------------------------------
  // Load file on mount
  // ---------------------------------------------------------------------------

  $effect(() => {
    // Run once on mount (path is a prop; re-running on path change would need
    // explicit tracking — for v1 the viewer is created fresh per popout).
    let cancelled = false;

    (async () => {
      try {
        const text = await invoke<string>('fs_read_text', { path });
        if (cancelled) return;
        content = text;
        originalContent = text;
      } catch (e: unknown) {
        if (cancelled) return;
        error = String(e);
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  // ---------------------------------------------------------------------------
  // Shiki highlight — re-runs when content changes in view mode
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (mode !== 'view') return;
    const snapshot = content;
    let cancelled = false;

    const lang = langForExt(path) ?? 'plaintext';

    (async () => {
      try {
        const html = await codeToHtml(snapshot, {
          lang,
          // 'vesper' ships in Shiki 1.x+ and 3.x. If the bundle doesn't
          // include it at runtime, Shiki throws — catch falls back gracefully.
          theme: 'vesper',
        });
        if (cancelled) return;
        highlightedHtml = html;
      } catch {
        // vesper not available in this Shiki build — retry with github-dark.
        try {
          const html = await codeToHtml(snapshot, { lang, theme: 'github-dark' });
          if (!cancelled) highlightedHtml = html;
        } catch {
          // Highlighting fully unavailable — display raw text safely.
          if (!cancelled) highlightedHtml = '';
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  // ---------------------------------------------------------------------------
  // Focus textarea when entering edit mode
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (mode === 'edit' && textareaEl) {
      textareaEl.focus();
    }
  });

  // ---------------------------------------------------------------------------
  // Keyboard shortcuts
  // ---------------------------------------------------------------------------

  function onKeyDown(e: KeyboardEvent): void {
    // stopPropagation on every claimed branch — Popout.svelte attaches a
    // window-level keydown listener (for Esc-dismiss across overlay stack).
    // Without stopPropagation, Esc in edit mode discards local edits AND
    // bubbles to the window listener which then dismisses the viewer popout.
    // Net: double action. Lesson `popout-keydown-bubble-collision` (6.5 BV).
    if (e.ctrlKey && e.key === 'e') {
      e.preventDefault();
      e.stopPropagation();
      mode = 'edit';
      return;
    }

    if (e.ctrlKey && e.key === 's') {
      e.preventDefault();
      e.stopPropagation();
      if (mode === 'edit' && dirty) {
        void save();
      }
      return;
    }

    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (mode === 'edit') {
        // Discard local edits — keep the popout open.
        content = originalContent;
        mode = 'view';
      } else {
        // View-mode Esc explicitly dismisses the popout.
        popouts.dismiss(popoutId);
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Save flow
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (saving) return;
    saving = true;
    error = null;
    try {
      await invoke('fs_write_text', { path, content });
      originalContent = content;
      mode = 'view';
      savedFlash = true;
      setTimeout(() => {
        savedFlash = false;
      }, 1200);
    } catch (e: unknown) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Retry (re-fetch after error)
  // ---------------------------------------------------------------------------

  function retry(): void {
    error = null;
    content = '';
    originalContent = '';
    highlightedHtml = '';
    // Trigger load $effect by reassigning content to '' — the effect will
    // re-run because it is registered as a side-effect (no dependency tracking
    // on `content` in that effect; it fires once on mount). To force a reload
    // we invoke directly here.
    let cancelled = false;
    (async () => {
      try {
        const text = await invoke<string>('fs_read_text', { path });
        if (cancelled) return;
        content = text;
        originalContent = text;
      } catch (e: unknown) {
        if (cancelled) return;
        error = String(e);
      }
    })();
    // Note: cancelled never set to true here because retry is fire-and-forget
    // from user gesture; component lifetime handles cleanup via unmount.
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="viewer"
  role="region"
  aria-label="file viewer"
  onkeydown={onKeyDown}
>
  <!-- Status bar: path + dirty indicator + saved flash + mode badge -->
  <div class="viewer-status">
    <span class="viewer-path">{path}</span>
    {#if dirty}
      <span class="viewer-dirty" title="Unsaved changes">●</span>
    {/if}
    {#if savedFlash}
      <span class="viewer-saved">saved</span>
    {/if}
    <span class="viewer-mode">{mode === 'edit' ? 'EDIT' : 'VIEW'}</span>
    {#if mode === 'view'}
      <button
        type="button"
        class="viewer-btn-edit"
        onclick={() => (mode = 'edit')}
        title="Edit (Ctrl+E)"
      >Edit</button>
    {:else}
      <button
        type="button"
        class="viewer-btn-save"
        onclick={() => { if (dirty) void save(); }}
        disabled={saving || !dirty}
        title="Save (Ctrl+S)"
      >{saving ? 'Saving…' : 'Save'}</button>
      <button
        type="button"
        class="viewer-btn-cancel"
        onclick={() => { content = originalContent; mode = 'view'; }}
        title="Discard (Esc)"
      >Discard</button>
    {/if}
  </div>

  <!-- Content area -->
  <div class="viewer-body">
    {#if error !== null}
      <!-- Inline error card with retry -->
      <div class="viewer-error">
        <span class="viewer-error-glyph">◇</span>
        <span class="viewer-error-msg">{error}</span>
        <button type="button" class="viewer-btn-retry" onclick={retry}>Retry</button>
      </div>
    {:else if mode === 'edit'}
      <textarea
        class="viewer-textarea"
        bind:value={content}
        bind:this={textareaEl}
        spellcheck={false}
        aria-label="file editor"
      ></textarea>
    {:else if content === '' && highlightedHtml === ''}
      <!-- Loading state -->
      <div class="viewer-loading">
        <span class="viewer-loading-glyph">◈</span>
        <span>loading…</span>
      </div>
    {:else if highlightedHtml !== ''}
      <!-- Syntax-highlighted view via Shiki -->
      <!-- CSP note: Tauri webview allows inline HTML from trusted sources;
           csp: null in tauri.conf.json covers this. -->
      <pre class="viewer-pre">{@html highlightedHtml}</pre>
    {:else}
      <!-- Fallback: Shiki not ready yet, show raw text -->
      <pre class="viewer-pre viewer-pre-raw">{content}</pre>
    {/if}
  </div>
</div>

<style>
  .viewer {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg-base);
    color: var(--amber-warm);
    font-family: 'JetBrains Mono', monospace;
  }

  /* Status bar */
  .viewer-status {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    flex-shrink: 0;
    font-size: 11px;
    flex-wrap: wrap;
  }

  .viewer-path {
    color: var(--amber-dim);
    font-size: 10px;
    letter-spacing: 0.04em;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .viewer-dirty {
    color: var(--amber-bright);
    font-size: 14px;
    line-height: 1;
    text-shadow: var(--glow-amber-faint);
  }

  .viewer-saved {
    color: var(--term-green);
    font-size: 10px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    animation: viewer-flash 1.2s ease-out forwards;
  }

  @keyframes viewer-flash {
    0%   { opacity: 1; }
    80%  { opacity: 1; }
    100% { opacity: 0; }
  }

  .viewer-mode {
    color: var(--amber-faint);
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    border: 1px solid var(--border-subtle);
    padding: 1px 5px;
  }

  .viewer-btn-edit,
  .viewer-btn-save,
  .viewer-btn-cancel,
  .viewer-btn-retry {
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 0.06em;
    cursor: pointer;
    padding: 2px 8px;
    border-radius: 0;
  }

  .viewer-btn-edit {
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-dim);
  }
  .viewer-btn-edit:hover {
    border-color: var(--amber-warm);
    color: var(--amber-warm);
  }

  .viewer-btn-save {
    background: var(--amber-bright);
    border: 1px solid var(--amber-bright);
    color: var(--bg-base);
    font-weight: 700;
  }
  .viewer-btn-save:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .viewer-btn-save:not(:disabled):hover {
    box-shadow: var(--glow-amber-strong);
  }

  .viewer-btn-cancel {
    background: transparent;
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
  }
  .viewer-btn-cancel:hover {
    border-color: var(--term-red);
    color: var(--term-red);
  }

  /* Content body */
  .viewer-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    position: relative;
  }
  .viewer-body::-webkit-scrollbar { width: 5px; }
  .viewer-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* Shiki output — override Shiki's injected <pre> background so it blends */
  .viewer-pre {
    margin: 0;
    padding: 12px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    line-height: 1.55;
    /* Let Shiki's theme colors show through; only override font/spacing. */
    background: transparent !important;
    min-height: 100%;
  }

  /* Shiki wraps output in a <pre><code> — make code inherit our font. */
  .viewer-pre :global(code) {
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    line-height: 1.55;
  }

  /* Shiki injects an outer <pre> with its own bg — make it transparent */
  .viewer-pre :global(pre) {
    background: transparent !important;
    margin: 0;
    padding: 0;
  }

  .viewer-pre-raw {
    color: var(--amber-warm);
    white-space: pre-wrap;
    word-break: break-all;
  }

  /* Edit textarea */
  .viewer-textarea {
    width: 100%;
    height: 100%;
    min-height: 300px;
    background: var(--bg-base);
    color: var(--amber-warm);
    border: none;
    outline: none;
    resize: none;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    line-height: 1.55;
    padding: 12px;
    box-sizing: border-box;
    caret-color: var(--amber-bright);
  }
  .viewer-textarea::selection {
    background: rgba(245, 158, 11, 0.25);
  }

  /* Loading state */
  .viewer-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px 16px;
    color: var(--amber-faint);
    font-size: 11px;
    font-style: italic;
  }
  .viewer-loading-glyph {
    font-size: 20px;
    opacity: 0.5;
  }

  /* Error state */
  .viewer-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 48px 24px;
    color: var(--term-red);
    font-size: 11px;
    text-align: center;
  }
  .viewer-error-glyph {
    font-size: 22px;
    opacity: 0.7;
  }
  .viewer-error-msg {
    color: var(--amber-dim);
    max-width: 360px;
    word-break: break-all;
    line-height: 1.4;
  }
  .viewer-btn-retry {
    background: transparent;
    border: 1px solid var(--term-red);
    color: var(--term-red);
    margin-top: 4px;
  }
  .viewer-btn-retry:hover {
    background: rgba(204, 51, 51, 0.12);
  }
</style>
