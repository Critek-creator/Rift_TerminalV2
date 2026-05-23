<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { invoke, Channel } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { Terminal as XTerm } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import '@xterm/xterm/css/xterm.css';
  import { deferredFit } from './terminal-fit-timing';
  import { TREE_PATH_MIME, RIFT_VAULT_DROP_EVENT, type RiftVaultDropDetail } from './dragMime';
  import { laneFormatGated } from './laneFormat';
  import LaneGutter from './LaneGutter.svelte';
  import TerminalSearch from './TerminalSearch.svelte';
  import PathTooltip from './PathTooltip.svelte';
  import { subscribe as busSubscribe, type Envelope } from './bus';

  type PtyExited = { id: number; code: number };

  interface Props {
    /** When false the host is `display:none`; xterm needs a refresh + fit
     *  pass on the transition false → true to redraw what arrived while
     *  hidden. */
    visible?: boolean;
    /** Project directory for this session. When set, the PTY spawns with
     *  this cwd instead of the global ProjectRoot. */
    projectPath?: string | null;
  }

  let { visible = true, projectPath = null }: Props = $props();

  let host: HTMLDivElement = $state(undefined!);
  let term: XTerm | undefined = $state(undefined);
  let fit: FitAddon | undefined;
  let search = $state<SearchAddon | undefined>(undefined);
  let searchOpen = $state(false);
  let resizeObs: ResizeObserver | undefined;
  let unlistenExited: UnlistenFn | undefined;

  // Path intelligence tooltip state
  type FilePreview = { exists: boolean; size_bytes: number; modified_iso: string; language_hint: string; preview_lines: string[]; is_binary: boolean };
  let tooltipVisible = $state(false);
  let tooltipX = $state(0);
  let tooltipY = $state(0);
  let tooltipPreview = $state<FilePreview | null>(null);
  let tooltipFilename = $state('');
  let recoveryTimer: ReturnType<typeof setInterval> | undefined;
  let sessionId: number | null = null;
  let alive = false;

  // Terminal config defaults (mirror crates/rift-bus/src/config.rs constants).
  const TERM_DEFAULT_FONT_SIZE = 13;
  const TERM_MIN_FONT_SIZE = 8;
  const TERM_MAX_FONT_SIZE = 48;
  const TERM_DEFAULT_LINE_HEIGHT = 1.55;
  const TERM_DEFAULT_SCROLLBACK = 1000;

  /** Per-tab runtime font size — adjusted via Ctrl+= / Ctrl+- / Ctrl+0.
   *  Not persisted across tabs; Settings panel is the persistence point. */
  let runtimeFontSize = $state(TERM_DEFAULT_FONT_SIZE);
  /** Saved-config font size — Ctrl+0 resets to this value. */
  let configFontSize = TERM_DEFAULT_FONT_SIZE;
  /** §10.1 lane tag prefixes for Rift-emitted lines. Snapshot at mount;
   *  the user's Settings change applies on next session (no cross-tab
   *  reactivity in v1 — matches existing project-swap precedent). */
  let lanesEnabled = true;

  // ---------------------------------------------------------------------------
  // §10.1 lane gutter — left-edge color strip indicating the active lane.
  // Subscribes to `pty` bus events with kind `lane.changed` published by the
  // Rust LaneClassifier. Until the backend publishes these events, the gutter
  // defaults to SYS (amber-faint) which is correct for a fresh shell prompt.
  // ---------------------------------------------------------------------------
  let currentLane = $state('SYS');
  let unsubscribeLane: (() => Promise<void>) | undefined;
  let laneMounted = true;

  // ---------------------------------------------------------------------------
  // Drag-node-into-terminal (Phase 6.6 — design calls A, C, D)
  // ---------------------------------------------------------------------------

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

  /**
   * Phase 8.7 — manual-gesture vault drop from IndexGraph (SVG `<g>`).
   *
   * IndexGraph dispatches a {@link RIFT_VAULT_DROP_EVENT} CustomEvent on this
   * host element when the user releases a manual mousedown/move/up gesture
   * over the terminal. We can't use HTML5 drag here because SVGElement does
   * not expose the `draggable` IDL property — see dragMime.ts for the full
   * note. The xterm.js paste path is the same as the HTML5 drop path above.
   */
  function onTermVaultDrop(e: Event): void {
    const detail = (e as CustomEvent<RiftVaultDropDetail>).detail;
    if (!detail?.path) return;
    pasteIntoTerminal(detail.path);
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

  // Read terminal settings from RiftConfig before constructing xterm.
  // Defaults survive a missing/old config (serde #[serde(default)]).
  // Failures fall through to the hardcoded defaults so a broken config
  // never prevents a terminal from launching.
  async function loadTerminalSettings(): Promise<{
    fontSize: number;
    lineHeight: number;
    scrollback: number;
    lanesEnabled: boolean;
  }> {
    const fallback = {
      fontSize: TERM_DEFAULT_FONT_SIZE,
      lineHeight: TERM_DEFAULT_LINE_HEIGHT,
      scrollback: TERM_DEFAULT_SCROLLBACK,
      lanesEnabled: true,
    };
    try {
      const cfg = await invoke<import('./riftConfig').RiftConfig>('config_get');
      const t = cfg?.terminal ?? null;
      if (!t) return fallback;
      return {
        fontSize: Math.max(TERM_MIN_FONT_SIZE, Math.min(TERM_MAX_FONT_SIZE, t.font_size)),
        lineHeight: Math.max(1.0, Math.min(2.5, t.line_height)),
        scrollback: Math.max(100, Math.min(100000, t.scrollback)),
        lanesEnabled: t.lanes_enabled,
      };
    } catch {
      return fallback;
    }
  }

  onMount(async () => {
    const settings = await loadTerminalSettings();
    configFontSize = settings.fontSize;
    runtimeFontSize = settings.fontSize;
    lanesEnabled = settings.lanesEnabled;

    term = new XTerm({
      fontFamily: '"JetBrains Mono", monospace',
      fontSize: settings.fontSize,
      lineHeight: settings.lineHeight,
      scrollback: settings.scrollback,
      cursorBlink: true,
      // Phase 8.7g.2 — palette synced with styles.css :root (xterm config
       // doesn't read CSS vars, so values are kept in lockstep manually).
       theme: {
        background: '#080806',
        foreground: '#FFA826',                 /* amber-primary */
        cursor: '#FFC840',                     /* amber-bright */
        cursorAccent: '#080806',
        selectionBackground: 'rgba(255, 168, 38, 0.30)',
        black: '#3A3530',
        red: '#FF4848',
        green: '#4FE855',
        yellow: '#FFC840',                     /* amber-bright */
        blue: '#6CB6FF',
        magenta: '#C58FFF',
        cyan: '#6FE0E0',
        white: '#E8E4D8',
        brightBlack: '#C49A50',                /* amber-faint (brightened for readability) */
        brightRed: '#FF6868',
        brightGreen: '#7FFA85',
        brightYellow: '#FFD968',                /* lifted amber-bright */
        brightBlue: '#9CCEFF',
        brightMagenta: '#DAB1FF',
        brightCyan: '#9FF0F0',
        brightWhite: '#FFFAEC',
      },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    search = new SearchAddon();
    term.loadAddon(search);

    // CRITICAL: await fonts.ready BEFORE term.open(host).
    //
    // xterm's CharSizeService measures glyph dimensions at term.open() time
    // using whatever font is currently rendering. If JetBrains Mono is still
    // loading, xterm caches FALLBACK font metrics and never re-measures —
    // even after fonts.ready resolves later. The cached cell dimensions are
    // then used by FitAddon.fit() to compute rows/cols, and by the renderer
    // to size cursor cells. Result on initial mount (cold font cache):
    // giant cursor 'T' artifact + bogus PTY rows/cols + blank shell prompt.
    // Subsequent terminals (added via "+") work because the font cache is
    // hot — open() measures correctly first time.
    //
    // Earlier attempt (deferredFit awaiting fonts.ready AFTER open) was too
    // late — xterm had already cached metrics. Moving the wait to BEFORE
    // open is the correct fix per xterm CharSizeService internals
    // (xterm.js issues #4101 / #4677). Confirmed by audit-2 frontend
    // specialist 2026-04-28.
    if (typeof document !== 'undefined' && document.fonts && document.fonts.ready) {
      try {
        await Promise.race([
          document.fonts.ready,
          new Promise<void>((r) => setTimeout(r, 2000)),
        ]);
      } catch {
        // jsdom/old browsers — proceed regardless.
      }
    }

    term.open(host);

    // Exposed for MCP pty_read tool — do not remove without updating mcp_host.rs tool_pty_read
    // Exposed for MCP pty_read tool — do not remove without updating mcp_host.rs tool_pty_read
    (window as any).__RIFT_TERM__ = term;

    // Path intelligence — detect file paths in terminal output and show hover previews.
    const PATH_RE = /(?:[A-Za-z]:\\|\/)[^\s:*?"<>|]+\.[a-zA-Z0-9]{1,10}/g;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    term.registerLinkProvider({
      provideLinks(lineNumber: number, callback: (links: any) => void) {
        const line = term?.buffer.active.getLine(lineNumber);
        if (!line) { callback(undefined); return; }
        const text = line.translateToString();
        const links: any[] = [];
        let match: RegExpExecArray | null;
        PATH_RE.lastIndex = 0;
        while ((match = PATH_RE.exec(text)) !== null) {
          const startX = match.index + 1;
          const endX = startX + match[0].length - 1;
          const filePath = match[0];
          links.push({
            text: filePath,
            range: { start: { x: startX, y: lineNumber }, end: { x: endX, y: lineNumber } },
            activate() {
              invoke('file_preview', { path: filePath }).catch(() => {});
            },
            hover(e: MouseEvent) {
              tooltipX = e.clientX + 12;
              tooltipY = e.clientY + 12;
              tooltipFilename = filePath.split(/[\\/]/).pop() || filePath;
              invoke('file_preview', { path: filePath }).then((result) => {
                tooltipPreview = result as FilePreview;
                tooltipVisible = true;
              }).catch(() => {
                tooltipVisible = false;
              });
            },
            leave() {
              tooltipVisible = false;
              tooltipPreview = null;
            },
          });
        }
        callback(links.length > 0 ? links : undefined);
      },
    } as any);

    // Phase 8.7g — Ctrl+C / Ctrl+V clipboard shortcuts.
    //
    // Default xterm behavior on Windows: Ctrl+C sends SIGINT to the PTY
    // (interrupts whatever's running), Ctrl+V is a no-op. That's how
    // shells expect to receive those keys — but it leaves no way to
    // copy/paste. Standard fix: when Ctrl+C is pressed AND the user has
    // a text selection, copy instead of interrupting. Ctrl+V always
    // pastes (Shift+Insert is still available for those who want raw
    // PTY paste). Ctrl+Shift+C / Ctrl+Shift+V are alternates that always
    // copy/paste regardless of selection.
    // Capture term in a non-nullable local so the closure below doesn't
    // hit `'term' is possibly undefined` — TypeScript narrowing doesn't
    // carry across the closure boundary.
    const t = term;

    /** Apply a runtime font-size change: clamp, retune xterm, refit, and
     *  push a `pty_resize` so the spawned shell sees the new geometry.
     *  Caller passes the desired absolute size; clamping + dirty-check
     *  happens here. Returns true if the size actually changed. */
    function applyFontSize(next: number): boolean {
      const clamped = Math.max(TERM_MIN_FONT_SIZE, Math.min(TERM_MAX_FONT_SIZE, Math.round(next)));
      if (clamped === runtimeFontSize) return false;
      runtimeFontSize = clamped;
      t.options.fontSize = clamped;
      // Refit so cell-grid recomputes against the new metrics, then push
      // dimensions to the PTY. This is the same shape ResizeObserver below
      // uses on container changes.
      try {
        fit?.fit();
      } catch {
        /* fit can throw before layout settles — best-effort */
      }
      if (sessionId !== null && alive) {
        invoke('pty_resize', { id: sessionId, rows: t.rows, cols: t.cols }).catch(() => {});
      }
      return true;
    }

    t.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;

      // Ctrl+Shift+F — toggle terminal search overlay.
      if (e.ctrlKey && e.shiftKey && (e.key === 'F' || e.key === 'f')) {
        searchOpen = !searchOpen;
        return false;
      }

      // Zoom keybinds (Phase 8.7m — restoring V1 commit aff0f50 behavior).
      // Plain Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0 only — Shift/Alt combos
      // pass through so the shell can still bind them.
      if (e.ctrlKey && !e.shiftKey && !e.altKey) {
        // Ctrl+= and Ctrl++ both increase (US keyboard '+' = Shift+'='; some
        // layouts emit '+' directly without Shift, e.g. dead-key + numpad).
        if (e.key === '=' || e.key === '+' || e.code === 'NumpadAdd') {
          applyFontSize(runtimeFontSize + 1);
          return false;
        }
        if (e.key === '-' || e.code === 'NumpadSubtract') {
          applyFontSize(runtimeFontSize - 1);
          return false;
        }
        if (e.key === '0' || e.code === 'Numpad0') {
          applyFontSize(configFontSize);
          return false;
        }
      }

      // Always-copy / always-paste with Shift modifier (standard convention).
      if (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'c')) {
        const sel = t.getSelection();
        if (sel) {
          void navigator.clipboard.writeText(sel).catch(() => { /* ignore */ });
        }
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'V' || e.key === 'v')) {
        navigator.clipboard
          .readText()
          .then((text) => { if (text) t.paste(text); })
          .catch(() => { /* ignore */ });
        return false;
      }

      // Plain Ctrl+C: copy IF selection exists, else fall through to SIGINT.
      if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === 'c' || e.key === 'C')) {
        if (t.hasSelection()) {
          const sel = t.getSelection();
          if (sel) {
            void navigator.clipboard.writeText(sel).catch(() => { /* ignore */ });
          }
          t.clearSelection();
          return false;
        }
        return true; // no selection → let xterm send SIGINT
      }

      // Plain Ctrl+V: paste from clipboard. Plays nice with Cmd shells too.
      if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === 'v' || e.key === 'V')) {
        navigator.clipboard
          .readText()
          .then((text) => { if (text) t.paste(text); })
          .catch(() => { /* ignore */ });
        return false;
      }

      return true;
    });

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
    await deferredFit(
      fit.fit.bind(fit),
      tick,
      () => host.getBoundingClientRect(),
    );

    // Force a buffer refresh after the deferred fit. The $effect at line 83
    // does this on visible→true transitions but not on first mount when
    // visible was already true. Without this, xterm's renderer can leave
    // the canvas unpainted if fit() crossed an internal threshold.
    // Cheap when redundant.
    if (term) term.refresh(0, term.rows - 1);

    const onChunk = new Channel<number[]>();
    onChunk.onmessage = (chunk) => {
      term?.write(new Uint8Array(chunk));
    };

    // Guard: start the PTY with at least 80×24 so the shell prompt always
    // has a usable buffer. If fit() couldn't compute real dimensions (host
    // still 0-sized because WebView2 window hasn't unminimized yet, or
    // flex layout hasn't settled), xterm may report tiny rows/cols. Starting
    // a PTY at 2×1 loses the prompt — the shell writes it, but xterm's
    // tiny ring buffer discards the bytes. Clamping here ensures the
    // prompt survives; the recovery timer + ResizeObserver correct to real
    // dimensions once layout settles.
    const startRows = Math.max(term.rows, 24);
    const startCols = Math.max(term.cols, 80);

    // Sync xterm's viewport to match what we'll tell the PTY. Without
    // this, the PTY writes 80-col output but xterm renders in a 2-col
    // viewport, causing garbled wrapping until the recovery timer
    // corrects both sides. Aligning up-front eliminates the flash.
    if (term.rows !== startRows || term.cols !== startCols) {
      term.resize(startCols, startRows);
    }

    try {
      sessionId = await invoke<number>('pty_start', {
        rows: startRows,
        cols: startCols,
        onChunk,
        cwd: projectPath ?? undefined,
      });
      alive = true;
      invoke('terminal_mounted').catch(() => {});
    } catch (err) {
      term.writeln(`\r\n${laneFormatGated('ERR', `pty_start failed: ${err}`, lanesEnabled)}`);
      return;
    }

    // Immediate post-start focus — xterm won't render cursor without it.
    term.focus();

    let lastResizeRows = 0;
    let lastResizeCols = 0;
    const refitAndResize = () => {
      try { fit?.fit(); } catch { /* best-effort */ }
      if (term) {
        term.refresh(0, term.rows - 1);
        term.scrollToBottom();
        term.focus();
        if (sessionId !== null && alive && term.rows > 1 && term.cols > 1) {
          if (term.rows !== lastResizeRows || term.cols !== lastResizeCols) {
            lastResizeRows = term.rows;
            lastResizeCols = term.cols;
            invoke('pty_resize', { id: sessionId, rows: term.rows, cols: term.cols }).catch(() => {});
          }
        }
      }
    };
    requestAnimationFrame(refitAndResize);

    let recoveryAttempts = 0;
    recoveryTimer = setInterval(() => {
      recoveryAttempts++;
      refitAndResize();
      if ((term && term.rows > 1 && term.cols > 1) || recoveryAttempts >= 15) {
        clearInterval(recoveryTimer);
        recoveryTimer = undefined;
        // Last-resort prompt recovery: if the terminal buffer is still
        // empty after all recovery attempts, the shell's initial prompt
        // was lost (written before xterm had a usable buffer). Send
        // Ctrl+L (form feed) — the universal clear-and-redraw signal —
        // to force the shell to re-emit its prompt.
        if (sessionId !== null && alive && term) {
          const line = term.buffer.active.getLine(0);
          const text = line?.translateToString().trim() ?? '';
          if (!text) {
            const ctrlL = Array.from(encoder.encode('\x0c'));
            invoke('pty_write', { id: sessionId, bytes: ctrlL }).catch(() => {});
          }
        }
      }
    }, 200);

    term.onData((data) => {
      if (sessionId === null || !alive) return;
      const bytes = Array.from(encoder.encode(data));
      invoke('pty_write', { id: sessionId, bytes }).catch((err) => {
        term?.writeln(`\r\n${laneFormatGated('ERR', `pty_write failed: ${err}`, lanesEnabled)}`);
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

    // v1: each Terminal filters globally; bounded by terminal count (1-2).
    unlistenExited = await listen<PtyExited>('pty_exited', (event) => {
      if (event.payload.id !== sessionId) return;
      alive = false;
      term?.writeln(
        `\r\n${laneFormatGated(
          'SYS',
          `session ${event.payload.id} exited code=${event.payload.code}`,
          lanesEnabled,
        )}`,
      );
    });

    // Phase 8.7 — vault-drop event listener (manual gesture from IndexGraph).
    host.addEventListener(RIFT_VAULT_DROP_EVENT, onTermVaultDrop);

    // §10.1 lane gutter — subscribe to lane.changed bus events.
    // The Rust LaneClassifier publishes Category::Pty / kind "lane.changed"
    // envelopes when the active lane transitions. We filter on the session_id
    // so multi-tab setups only show lane state for their own PTY.
    try {
      const u = await busSubscribe({ category: 'pty' }, (env: Envelope) => {
        if (env.kind !== 'lane.changed') return;
        const p = env.payload as { lane?: string; session_id?: number } | null;
        if (!p?.lane) return;
        // Filter to our session — ignore lane events from other tabs' PTYs.
        if (p.session_id !== undefined && p.session_id !== sessionId) return;
        currentLane = p.lane;
      });
      if (!laneMounted) {
        void u().catch(() => {});
      } else {
        unsubscribeLane = u;
      }
    } catch (err) {
      console.warn('[Terminal] lane bus subscription failed', err);
    }
  });

  onDestroy(() => {
    laneMounted = false;
    if (sessionId !== null && alive) {
      invoke('pty_kill', { id: sessionId }).catch(() => {});
    }
    unlistenExited?.();
    unsubscribeLane?.().catch(() => {});
    if (recoveryTimer) clearInterval(recoveryTimer);
    resizeObs?.disconnect();
    host?.removeEventListener(RIFT_VAULT_DROP_EVENT, onTermVaultDrop);
    search?.dispose();
    fit?.dispose();
    term?.dispose();
    delete (window as any).__RIFT_TERM__;
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
>
  <LaneGutter terminal={term} hostElement={host} currentLane={currentLane} />
  {#if searchOpen && search}
    <TerminalSearch searchAddon={search} onclose={() => { searchOpen = false; term?.focus(); }} />
  {/if}
</div>

<PathTooltip
  x={tooltipX}
  y={tooltipY}
  visible={tooltipVisible}
  preview={tooltipPreview}
  filename={tooltipFilename}
/>

<style>
  .terminal-host {
    position: relative;
    flex: 1;
    background: var(--bg-base);
    padding: var(--space-8);
    overflow: hidden;
    min-height: 0;
  }
  /* Phase 6.6 — subtle amber inset glow while a tree-path drag hovers */
  .terminal-host.drag-hover {
    box-shadow: inset 0 0 0 2px var(--amber-bright);
  }
</style>
