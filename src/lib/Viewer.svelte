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
  // Phase 8.7m / D-017 — CodeMirror 6 in edit mode (replaces plain textarea).
  // Imports stay top-level (vite tree-shakes lang packs we don't use per file).
  import { EditorView, keymap, lineNumbers, highlightActiveLine } from '@codemirror/view';
  import { EditorState, Compartment, type Extension } from '@codemirror/state';
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { rust } from '@codemirror/lang-rust';
  import { javascript } from '@codemirror/lang-javascript';
  import { json } from '@codemirror/lang-json';
  import { yaml } from '@codemirror/lang-yaml';
  import { markdown } from '@codemirror/lang-markdown';
  import { html } from '@codemirror/lang-html';
  import { css } from '@codemirror/lang-css';
  import { python } from '@codemirror/lang-python';
  import { cpp } from '@codemirror/lang-cpp';

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
  let savedFlashTimer: ReturnType<typeof setTimeout> | undefined;
  let highlightedHtml = $state<string>('');

  /** Container for the CodeMirror EditorView (mounted in edit mode). */
  let editorEl = $state<HTMLDivElement | null>(null);
  /** Active CM EditorView — created on first edit, swapped on path change,
   *  destroyed on unmount. */
  let editorView: EditorView | null = null;
  /** Compartment for the language extension so we can hot-swap when the
   *  file extension changes without rebuilding the whole state. */
  const langCompartment = new Compartment();

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

    const rawLang = langForExt(path);
    const lang = rawLang && /^[a-z]+$/.test(rawLang) ? rawLang : 'plaintext';

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
  // CodeMirror 6 — language resolution (re-uses Shiki ext map indirectly via
  // a sister function so the lang sets stay in lockstep)
  // ---------------------------------------------------------------------------

  function cmLangForExt(filePath: string): Extension[] {
    const ext = filePath.split('.').at(-1)?.toLowerCase();
    switch (ext) {
      case 'rs':   return [rust()];
      case 'ts':
      case 'tsx':  return [javascript({ typescript: true, jsx: ext === 'tsx' })];
      case 'js':
      case 'jsx':
      case 'mjs':
      case 'cjs':  return [javascript({ jsx: ext === 'jsx' })];
      case 'json': return [json()];
      case 'yaml':
      case 'yml':  return [yaml()];
      case 'md':
      case 'mdx':  return [markdown()];
      case 'html':
      case 'svelte':
      case 'vue':  return [html()];
      case 'css':
      case 'scss':
      case 'sass':
      case 'less': return [css()];
      case 'py':   return [python()];
      case 'c':
      case 'cpp':
      case 'cc':
      case 'cxx':
      case 'h':
      case 'hpp':
      case 'hxx':  return [cpp()];
      default:     return []; // plain text — no language extension
    }
  }

  // ---------------------------------------------------------------------------
  // CodeMirror 6 — amber theme matching the rift aesthetic.
  // EditorView.theme returns an Extension; selectors prefix all rules so we
  // don't bleed onto the rest of the app.
  // ---------------------------------------------------------------------------

  const riftEditorTheme = EditorView.theme(
    {
      '&': {
        color: 'var(--amber-warm)',
        backgroundColor: 'var(--bg-base)',
        height: '100%',
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: '12px',
      },
      '.cm-content': {
        caretColor: 'var(--amber-bright)',
        padding: '12px 0',
      },
      '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--amber-bright)' },
      '&.cm-focused .cm-cursor': { borderLeftColor: 'var(--amber-bright)' },
      '&.cm-focused': { outline: 'none' },
      '.cm-line': { padding: '0 12px' },
      '&.cm-focused .cm-selectionBackground, ::selection, .cm-selectionBackground': {
        backgroundColor: 'rgba(245, 158, 11, 0.22)',
      },
      '.cm-activeLine': { backgroundColor: 'rgba(212, 137, 10, 0.05)' },
      '.cm-activeLineGutter': {
        backgroundColor: 'rgba(212, 137, 10, 0.08)',
        color: 'var(--amber-bright)',
      },
      '.cm-gutters': {
        backgroundColor: 'var(--bg-elevated)',
        color: 'var(--amber-faint)',
        border: 'none',
        borderRight: '1px solid var(--border-subtle)',
      },
      '.cm-lineNumbers .cm-gutterElement': {
        padding: '0 8px 0 12px',
        minWidth: '28px',
        textAlign: 'right',
      },
      // Syntax highlight tokens — basic mapping to amber/red/green/cyan/purple
      // per §10.1 lane semantics. CM6 uses the @lezer/highlight tag system;
      // these classes are how the default highlightStyle injects them.
      '.tok-keyword':       { color: 'var(--term-purple)', fontWeight: '600' },
      '.tok-string':        { color: 'var(--term-green, #4FE855)' },
      '.tok-number':        { color: 'var(--amber-bright)' },
      '.tok-comment':       { color: 'var(--amber-faint)', fontStyle: 'italic' },
      '.tok-typeName':      { color: 'var(--term-cyan)' },
      '.tok-className':     { color: 'var(--term-cyan)' },
      '.tok-function':      { color: 'var(--amber-bright)' },
      '.tok-propertyName':  { color: 'var(--amber-warm)' },
      '.tok-operator':      { color: 'var(--amber-dim)' },
      '.tok-punctuation':   { color: 'var(--amber-dim)' },
      '.cm-scroller': {
        fontFamily: "'JetBrains Mono', monospace",
        lineHeight: '1.55',
        overflow: 'auto',
      },
    },
    { dark: true },
  );

  // ---------------------------------------------------------------------------
  // CodeMirror 6 lifecycle — mount when entering edit mode, destroy on exit.
  // The editor's doc state is the source of truth while in edit mode; we
  // mirror it back into `content` via an updateListener so the dirty
  // indicator + Save flow keep working unchanged.
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (mode !== 'edit' || !editorEl) {
      // Tear down when leaving edit mode so memory + listeners free.
      if (editorView) {
        editorView.destroy();
        editorView = null;
      }
      return;
    }

    const updateListener = EditorView.updateListener.of((u) => {
      if (u.docChanged) {
        content = u.state.doc.toString();
      }
    });

    const state = EditorState.create({
      doc: content,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
        langCompartment.of(cmLangForExt(path)),
        riftEditorTheme,
        EditorView.lineWrapping,
        updateListener,
      ],
    });

    editorView = new EditorView({
      state,
      parent: editorEl,
    });

    editorView.focus();

    return () => {
      editorView?.destroy();
      editorView = null;
    };
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
      if (savedFlashTimer) clearTimeout(savedFlashTimer);
      savedFlashTimer = setTimeout(() => {
        savedFlash = false;
        savedFlashTimer = undefined;
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
    (async () => {
      try {
        const text = await invoke<string>('fs_read_text', { path });
        content = text;
        originalContent = text;
      } catch (e: unknown) {
        error = String(e);
      }
    })();
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
      <!-- D-017 / Phase 8.7m: CodeMirror 6 EditorView mounts here. The
           lifecycle effect above creates/destroys the EditorView based on
           `mode`. The `viewer-cm` class only handles container layout —
           CM owns its own internal styling via `riftEditorTheme`. -->
      <div
        class="viewer-cm"
        bind:this={editorEl}
        aria-label="file editor"
      ></div>
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
    font-family: var(--font-family);
  }

  /* Status bar */
  .viewer-status {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-8) var(--space-14);
    background: linear-gradient(to bottom, var(--bg-elevated), var(--bg-surface));
    box-shadow: var(--sep-glow);
    flex-shrink: 0;
    font-size: var(--text-sm);
    flex-wrap: wrap;
  }

  .viewer-path {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .viewer-dirty {
    color: var(--amber-bright);
    font-size: var(--text-lg);
    line-height: 1;
    text-shadow: var(--glow-amber-faint);
  }

  .viewer-saved {
    color: var(--term-green);
    font-size: var(--text-xs);
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
    font-size: var(--text-2xs);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    padding: 2px 7px;
    background: var(--bg-base);
  }

  .viewer-btn-edit,
  .viewer-btn-save,
  .viewer-btn-cancel,
  .viewer-btn-retry {
    font-family: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    letter-spacing: 0.06em;
    cursor: pointer;
    padding: 3px var(--space-md);
    border-radius: var(--radius-md, 4px);
    transition: color 0.12s ease-out, background 0.12s ease-out, border-color 0.12s ease-out, box-shadow 0.12s ease-out, opacity 0.12s ease-out;
  }

  .viewer-btn-edit {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
  }
  .viewer-btn-edit:hover {
    border-color: var(--amber-dim);
    color: var(--amber-warm);
    background: var(--bg-hover);
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
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
  }
  .viewer-btn-cancel:hover {
    border-color: var(--term-red);
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.06);
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

  /* Shiki output — override Shiki's injected <pre> background so it blends.
     Phase 8.7g.6 — soft-wrap long lines so syntax-highlighted code doesn't
     run off the right edge of the popout. Indentation is preserved
     (pre-wrap, not normal). overflow-wrap: anywhere lets very long
     unbroken tokens (URLs, base64) break too. */
  .viewer-pre {
    margin: 0;
    padding: var(--space-12);
    font-family: var(--font-family);
    font-size: var(--text-base);
    line-height: 1.55;
    /* Let Shiki's theme colors show through; only override font/spacing. */
    background: transparent !important;
    min-height: 100%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* Shiki wraps output in a <pre><code> — make code inherit our font. */
  .viewer-pre :global(code) {
    font-family: var(--font-family);
    font-size: var(--text-base);
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* Shiki injects an outer <pre> with its own bg — make it transparent.
     Force soft-wrap here too because Shiki's nested <pre> has its own
     inline styles. */
  .viewer-pre :global(pre) {
    background: transparent !important;
    margin: 0;
    padding: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .viewer-pre-raw {
    color: var(--amber-warm);
    white-space: pre-wrap;
    word-break: break-all;
  }

  /* CodeMirror 6 mount container — fills the body, lets CM own internals. */
  .viewer-cm {
    width: 100%;
    height: 100%;
    min-height: 300px;
    box-sizing: border-box;
    overflow: hidden;
  }
  .viewer-cm :global(.cm-editor) {
    height: 100%;
  }
  .viewer-cm :global(.cm-scroller::-webkit-scrollbar) { width: 5px; }
  .viewer-cm :global(.cm-scroller::-webkit-scrollbar-thumb) {
    background: var(--amber-faint);
  }

  /* Loading state */
  .viewer-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-8);
    padding: var(--space-4xl)var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--text-sm);
    font-style: italic;
  }
  .viewer-loading-glyph {
    font-size: var(--space-xl);
    opacity: 0.5;
  }

  /* Error state */
  .viewer-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-12);
    padding: var(--space-4xl)var(--space-24);
    color: var(--term-red);
    font-size: var(--text-sm);
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
    background: var(--bg-elevated);
    border: 1px solid var(--term-red);
    color: var(--term-red);
    margin-top: var(--space-xs);
  }
  .viewer-btn-retry:hover {
    background: rgba(204, 51, 51, 0.12);
    box-shadow: 0 0 4px rgba(204, 51, 51, 0.15);
  }
</style>
