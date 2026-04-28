<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { invoke, Channel } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { Terminal as XTerm } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { deferredFit } from './terminal-fit-timing';

  type PtyExited = { id: number; code: number };

  interface Props {
    /** When false the host is `display:none`; xterm needs a refresh + fit
     *  pass on the transition false → true to redraw what arrived while
     *  hidden. */
    visible?: boolean;
  }

  let { visible = true }: Props = $props();

  let host: HTMLDivElement;
  let term: XTerm | undefined;
  let fit: FitAddon | undefined;
  let resizeObs: ResizeObserver | undefined;
  let unlistenExited: UnlistenFn | undefined;
  let sessionId: number | null = null;
  let alive = false;

  // ---------------------------------------------------------------------------
  // Drag-node-into-terminal (Phase 6.6 — design calls A, C, D)
  // ---------------------------------------------------------------------------

  /** Custom MIME type that Tree.svelte sets on tree-path drags. */
  const TREE_PATH_MIME = 'application/x-rift-tree-path';

  /** True while a valid tree-path drag hovers the terminal host. */
  let dragHover = $state(false);

  /**
   * Quote a path for shell insertion.
   * Wraps in double-quotes when the path contains spaces — safe for cmd,
   * PowerShell, and bash (the three shells a Windows Rift user most likely runs).
   *
   * Paths containing literal `"` characters are NOT escaped in v1 (extremely
   * rare; Phase 6.x should add backslash/caret escaping if it surfaces).
   */
  function quotePath(path: string): string {
    return path.includes(' ') ? `"${path}"` : path;
  }

  /**
   * Insert path text at the terminal cursor exactly as if the user typed it.
   * Appends a trailing space for ergonomics (user can keep typing or hit Enter).
   * Guards against term being unmounted mid-drag.
   */
  function pasteIntoTerminal(path: string): void {
    if (!term) return;
    term.paste(quotePath(path) + ' ');
  }

  function onTermDragOver(e: DragEvent): void {
    // Only claim the drop when the payload is ours — lets other drag sources pass through.
    if (!e.dataTransfer?.types.includes(TREE_PATH_MIME)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    dragHover = true;
  }

  function onTermDragLeave(): void {
    dragHover = false;
  }

  function onTermDrop(e: DragEvent): void {
    dragHover = false;
    const path = e.dataTransfer?.getData(TREE_PATH_MIME);
    if (!path) return;
    e.preventDefault();
    pasteIntoTerminal(path);
  }

  const encoder = new TextEncoder();

  $effect(() => {
    // visible: false → true. Wait one tick so layout is real, then refit
    // and force a render so the buffer that accumulated while hidden draws.
    if (visible && term && fit) {
      tick().then(() => {
        fit?.fit();
        if (term) term.refresh(0, term.rows - 1);
      });
    }
  });

  onMount(async () => {
    term = new XTerm({
      fontFamily: '"JetBrains Mono", monospace',
      fontSize: 13,
      lineHeight: 1.55,
      cursorBlink: true,
      theme: {
        background: '#080806',
        foreground: '#D4890A',
        cursor: '#f59e0b',
        cursorAccent: '#080806',
        selectionBackground: 'rgba(212, 137, 10, 0.25)',
        black: '#080806',
        red: '#CC3333',
        green: '#33CC33',
        yellow: '#f59e0b',
        blue: '#4a9eff',
        magenta: '#b078e8',
        cyan: '#4ad4d4',
        white: '#d8d4c8',
        brightBlack: '#5a4410',
        brightRed: '#CC3333',
        brightGreen: '#33CC33',
        brightYellow: '#f59e0b',
        brightBlue: '#4a9eff',
        brightMagenta: '#b078e8',
        brightCyan: '#4ad4d4',
        brightWhite: '#d8d4c8',
      },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);

    // Defer fit until layout has actually settled.
    //
    // On INITIAL app render, this Terminal component is mounted by App.svelte's
    // {#each sessions} block at the same time as siblings in cockpit-right
    // (IndexGraph, Tree). Svelte's onMount fires after the component reaches
    // the DOM but BEFORE the parent flex containers have laid out their final
    // dimensions. Calling fit.fit() at this moment can measure terminal-host
    // as 0×0 (or close to it) — xterm then sizes its canvas to 0×0 and PTY
    // gets started with bogus rows/cols. The shell launches but its prompt
    // bytes land in a 0-line ring buffer; later layout settles via the
    // ResizeObserver but cmd.exe doesn't re-emit the prompt → terminal stays
    // black for the initial session.
    //
    // Sessions opened later via the "+" button don't hit this because by then
    // the cockpit layout is fully settled.
    //
    // Fix: tick() yields to Svelte's microtask queue (parent $effects + flex
    // recalculation), then a single requestAnimationFrame guarantees the
    // browser has actually computed final layout dimensions before we measure.
    // This narrowly addresses pr003 lesson `terminal-fit-races-initial-flex-layout`.
    // Timing sequence extracted to `terminal-fit-timing.ts` for unit-testability.
    await deferredFit(fit.fit.bind(fit), tick);

    const onChunk = new Channel<number[]>();
    onChunk.onmessage = (chunk) => {
      term?.write(new Uint8Array(chunk));
    };

    try {
      sessionId = await invoke<number>('pty_start', {
        rows: term.rows,
        cols: term.cols,
        onChunk,
      });
      alive = true;
    } catch (err) {
      term.writeln(`\r\n\x1b[31m[pty_start failed: ${err}]\x1b[0m`);
      return;
    }

    term.onData((data) => {
      if (sessionId === null || !alive) return;
      const bytes = Array.from(encoder.encode(data));
      invoke('pty_write', { id: sessionId, bytes }).catch((err) => {
        term?.writeln(`\r\n\x1b[31m[pty_write failed: ${err}]\x1b[0m`);
      });
    });

    resizeObs = new ResizeObserver(() => {
      fit?.fit();
      if (sessionId !== null && alive && term) {
        invoke('pty_resize', {
          id: sessionId,
          rows: term.rows,
          cols: term.cols,
        }).catch(() => { /* best-effort */ });
      }
    });
    resizeObs.observe(host);

    unlistenExited = await listen<PtyExited>('pty_exited', (event) => {
      if (event.payload.id !== sessionId) return;
      alive = false;
      term?.writeln(
        `\r\n\x1b[2;33m[session ${event.payload.id} exited code=${event.payload.code}]\x1b[0m`
      );
    });
  });

  onDestroy(() => {
    if (sessionId !== null && alive) {
      invoke('pty_kill', { id: sessionId }).catch(() => {});
    }
    unlistenExited?.();
    resizeObs?.disconnect();
    term?.dispose();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="terminal-host"
  class:drag-hover={dragHover}
  bind:this={host}
  role="application"
  aria-label="terminal"
  ondragover={onTermDragOver}
  ondragleave={onTermDragLeave}
  ondrop={onTermDrop}
></div>

<style>
  .terminal-host {
    flex: 1;
    background: var(--bg-base);
    padding: 8px;
    overflow: hidden;
    min-height: 0;
  }
  /* Phase 6.6 — subtle amber inset glow while a tree-path drag hovers */
  .terminal-host.drag-hover {
    box-shadow: inset 0 0 0 2px var(--amber-bright);
  }
</style>
