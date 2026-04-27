<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { invoke, Channel } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { Terminal as XTerm } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';

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
    fit.fit();

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

<div class="terminal-host" bind:this={host}></div>

<style>
  .terminal-host {
    flex: 1;
    background: var(--bg-base);
    padding: 8px;
    overflow: hidden;
    min-height: 0;
  }
</style>
