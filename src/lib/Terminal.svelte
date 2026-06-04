<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { Terminal as XTerm, type IMarker } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { SerializeAddon } from '@xterm/addon-serialize';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { LigaturesAddon } from '@xterm/addon-ligatures';

  /** Local mirror of xterm's non-exported ILink / ILinkProvider interfaces.
   *  Matches @xterm/xterm typings/xterm.d.ts so registerLinkProvider accepts
   *  our object without `as any`. */
  interface RiftLink {
    range: { start: { x: number; y: number }; end: { x: number; y: number } };
    text: string;
    activate(event: MouseEvent, text: string): void;
    hover?(event: MouseEvent, text: string): void;
    leave?(event: MouseEvent, text: string): void;
    dispose?(): void;
  }
  interface RiftLinkProvider {
    provideLinks(bufferLineNumber: number, callback: (links: RiftLink[] | undefined) => void): void;
  }
  import '@xterm/xterm/css/xterm.css';
  import { deferredFit } from './terminal-fit-timing';
  import { TREE_PATH_MIME, RIFT_VAULT_DROP_EVENT, RIFT_EVENT_MIME, type RiftVaultDropDetail } from './dragMime';
  import {
    registerInjector,
    unregisterInjector,
    setActiveInjector,
    registerPtyId,
    unregisterPtyId,
  } from './terminalInject';
  import { laneFormatGated } from './laneFormat';
  import LaneGutter from './LaneGutter.svelte';
  import TerminalSearch from './TerminalSearch.svelte';
  import PathTooltip from './PathTooltip.svelte';
  import { popouts } from './popouts.svelte';
  import { formatDuration } from './formatDuration';
  import { subscribe as busSubscribe, publish as busPublish, type Envelope } from './bus';
  import {
    assembleFailureContext,
    summarizeFailureContext,
    errorActionId,
    failureClusterKey,
    ERROR_EXPLAIN_ACTION,
    ERROR_FIX_ACTION,
    type BufferLike,
    type CommandCapture,
    type FailureContext,
  } from './errorHandoff';
  import { actionRegistry, type DeclaredAction } from './actionRegistry.svelte';
  import { commandBlockStore, type CommandBlock } from './commandBlockStore.svelte';
  import { readBufferRange } from './blockText';
  import { registerScroller, unregisterScroller } from './blockJump';
  import ErrorResultPopout from './ErrorResultPopout.svelte';
  import StickyCommandHeader from './StickyCommandHeader.svelte';
  import BlockActionMenu from './BlockActionMenu.svelte';
  import { getTerminalSettings, invalidateTerminalSettingsCache } from './terminalConfigCache';
  import { resolveTheme } from './terminalPalettes';
  import { LaneTintManager } from './laneTint';
  import { sessionManager } from './sessionManager.svelte';
  import {
    getRestorePayload,
    claimPane,
    registerPaneProvider,
    unregisterPaneProvider,
    getSnapshotConfig,
    setActivePane,
    isActivePane,
    type PaneSnapshot,
  } from './sessionRestore';

  type PtyExited = { id: number; code: number };

  interface Props {
    /** When false the host is `display:none`; xterm needs a refresh + fit
     *  pass on the transition false → true to redraw what arrived while
     *  hidden. */
    visible?: boolean;
    /** Project directory for this session. When set, the PTY spawns with
     *  this cwd instead of the global ProjectRoot. */
    projectPath?: string | null;
    /** Fired when this terminal's PTY process exits. */
    onPtyExited?: () => void;
    /** Pane (leaf) id. Used to consume any one-shot command queued for this
     *  pane via sessionManager (e.g. the Gemini "sign in" launcher). */
    paneId?: number;
  }

  let { visible = true, projectPath = null, onPtyExited, paneId }: Props = $props();

  let host: HTMLDivElement = $state(undefined!);
  let term: XTerm | undefined = $state(undefined);
  let fit: FitAddon | undefined;
  let search = $state<SearchAddon | undefined>(undefined);
  let serializeAddon: SerializeAddon | undefined;
  let snapshotTimer: ReturnType<typeof setInterval> | undefined;
  /** cwd this pane is effectively in — the restored cwd if we re-hydrated from
   *  a snapshot, else the project path. Used as the snapshot's recorded cwd. */
  let effectiveCwd: string | null = null;
  let webgl: WebglAddon | undefined;
  let ligatures: LigaturesAddon | undefined;
  let searchOpen = $state(false);
  let resizeObs: ResizeObserver | undefined;
  let resizeRaf: number | undefined;
  let unlistenExited: UnlistenFn | undefined;
  let unlistenChunk: UnlistenFn | undefined;
  let configChangedCleanup: (() => void) | undefined;
  let tintManager: LaneTintManager | undefined;

  // Path intelligence tooltip state
  type FilePreview = { exists: boolean; size_bytes: number; modified_iso: string; language_hint: string; preview_lines: string[]; is_binary: boolean };
  let tooltipVisible = $state(false);
  let tooltipX = $state(0);
  let tooltipY = $state(0);
  let tooltipPreview = $state<FilePreview | null>(null);
  let tooltipFilename = $state('');
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;
  let hoverPendingPath: string | null = null;
  let recoveryTimer: ReturnType<typeof setInterval> | undefined;
  let contentGuardTimer: ReturnType<typeof setInterval> | undefined;
  let sessionId: number | null = null;
  let alive = false;
  let opened = false;

  // Terminal config defaults (mirror crates/rift-bus/src/config.rs constants).
  const TERM_DEFAULT_FONT_SIZE = 13;
  const TERM_MIN_FONT_SIZE = 8;
  const TERM_MAX_FONT_SIZE = 48;

  /** Per-tab runtime font size — adjusted via Ctrl+= / Ctrl+- / Ctrl+0.
   *  Not persisted across tabs; Settings panel is the persistence point. */
  let runtimeFontSize = $state(TERM_DEFAULT_FONT_SIZE);
  /** Saved-config font size — Ctrl+0 resets to this value. */
  let configFontSize = TERM_DEFAULT_FONT_SIZE;
  /** §10.1 lane tag prefixes for Rift-emitted lines. Snapshot at mount;
   *  the user's Settings change applies on next session (no cross-tab
   *  reactivity in v1 — matches existing project-swap precedent). */
  let lanesEnabled = true;

  // Phase 5 / R0 — error→agent handoff capture cache. On `command.submitted`
  // we stash the command text + cwd + the prompt's buffer row; on a non-zero
  // `command.completed` we pair them with the exit code and read the output
  // region out of the xterm buffer to build a FailureContext. Plain (non-
  // reactive) transient state — it only lives between submit and completion.
  let pendingCapture: CommandCapture | null = null;

  // N3.2 — sticky command header. On command.submitted we drop an xterm marker
  // at the prompt row; the marker's `.line` auto-tracks reflow + scrollback
  // eviction (a raw row integer would drift), so it's the authoritative live
  // anchor for a block's TOP edge. `paneBlocks` maps each recorded block id to
  // its live start marker; `stickyBlock` is the block whose output region spans
  // the current viewport top (null at the live prompt / above the first block).
  let pendingStartMarker: IMarker | null = null;
  // Per-pane live anchors for each recorded block: start + end markers (xterm
  // auto-tracks their `.line` through reflow/eviction) plus the badge element
  // (so a bookmark toggle can flip its star). The cross-pane metadata lives in
  // commandBlockStore; this map is the pane-local xterm binding.
  interface PaneBlock {
    startMarker: IMarker;
    endMarker: IMarker;
    block: CommandBlock;
    badgeEl?: HTMLElement;
  }
  const paneBlocks = new Map<string, PaneBlock>();
  let stickyBlock = $state<CommandBlock | null>(null);
  // N3.3 — the open block-action menu (copy / bookmark), anchored at the badge.
  let blockMenu = $state<{ block: CommandBlock; x: number; y: number } | null>(null);

  /** Recompute which block's output spans the viewport top. Shown only when
   *  scrolled UP (viewportY < baseY) — at the bottom the live prompt is visible
   *  and a sticky reminder is just noise. Picks the block with the greatest
   *  live start row that is still at or above the viewport top. Prunes markers
   *  that xterm disposed when their line aged out of scrollback. Reads the live
   *  store object so the header reflects the current bookmark state. */
  function recomputeSticky(): void {
    const buf = term?.buffer.active;
    if (!buf) {
      stickyBlock = null;
      return;
    }
    const viewportY = buf.viewportY;
    // At (or below) the bottom page → no header.
    if (viewportY >= buf.baseY) {
      stickyBlock = null;
      return;
    }
    let bestId: string | null = null;
    let bestLine = -1;
    for (const [id, entry] of paneBlocks) {
      const line = entry.startMarker.line; // -1 once disposed
      if (line < 0) {
        paneBlocks.delete(id);
        continue;
      }
      if (line <= viewportY && line > bestLine) {
        bestLine = line;
        bestId = id;
      }
    }
    stickyBlock = bestId ? (commandBlockStore.byId(bestId) ?? null) : null;
  }

  // ── N3.3 block actions (copy + bookmark) ──────────────────────────────────

  /** Open the block-action menu for a block, anchored at the click position. */
  function openBlockMenu(blockId: string, ev: MouseEvent): void {
    const block = commandBlockStore.byId(blockId);
    if (!block) return;
    blockMenu = { block, x: ev.clientX, y: ev.clientY };
  }

  /** Read a block's region out of the live buffer. `includeCommand=false` skips
   *  the prompt/command line (first row) to copy just the output. Returns null
   *  when the region has aged out of scrollback (markers disposed). */
  function blockRegionText(blockId: string, includeCommand: boolean): string | null {
    const entry = paneBlocks.get(blockId);
    if (!entry || !term) return null;
    const a = entry.startMarker.line;
    const b = entry.endMarker.line;
    if (a < 0 || b < 0) return null;
    return readBufferRange(term.buffer.active, includeCommand ? a : a + 1, b);
  }

  async function copyToClipboard(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* clipboard permission denied — silent no-op */
    }
  }

  function copyBlockCommand(blockId: string): void {
    const b = commandBlockStore.byId(blockId);
    if (b) void copyToClipboard(b.command);
  }
  function copyBlockOutput(blockId: string): void {
    const t = blockRegionText(blockId, false);
    if (t != null) void copyToClipboard(t);
  }
  function copyBlockBoth(blockId: string): void {
    const b = commandBlockStore.byId(blockId);
    const out = blockRegionText(blockId, false);
    const parts: string[] = [];
    if (b) parts.push(b.command);
    if (out) parts.push(out);
    if (parts.length) void copyToClipboard(parts.join('\n'));
  }
  function toggleBlockBookmark(blockId: string): void {
    const now = commandBlockStore.toggleBookmark(blockId);
    paneBlocks.get(blockId)?.badgeEl?.classList.toggle('bookmarked', now);
    if (stickyBlock?.id === blockId) stickyBlock = commandBlockStore.byId(blockId) ?? null;
  }

  /** Stash the rendered badge element so a later bookmark toggle can flip its
   *  star without re-rendering the decoration. */
  function registerBadgeEl(blockId: string, el: HTMLElement): void {
    const entry = paneBlocks.get(blockId);
    if (entry) entry.badgeEl = el;
  }

  // N3.4 — jump-to-block. A pending target is deferred until the pane is visible
  // (the palette may activate this tab and request the scroll in one turn).
  let pendingJump: string | null = null;

  /** Scroller registered with blockJump: scroll now if visible, else defer to
   *  the next visible→refit (flushed in the $effect below). */
  function scrollToBlockLocal(blockId: string): void {
    if (!visible) {
      pendingJump = blockId;
      return;
    }
    doScrollToBlock(blockId);
  }
  function doScrollToBlock(blockId: string): void {
    const entry = paneBlocks.get(blockId);
    if (!entry || !term) return;
    const line = entry.startMarker.line;
    if (line < 0) return; // region aged out of scrollback
    term.scrollToLine(line);
    recomputeSticky();
  }

  // Phase 5 / R2 — error-handoff mode (off | detect | assist), read live from
  // config. `off` suppresses all surfacing (passive badge only, no issues-list
  // entry); `detect` (default) lights the click-to-explain affordance; `assist`
  // additionally auto-invokes explain on failure (never auto-runs a fix).
  let errorHandoffMode: 'off' | 'detect' | 'assist' = 'detect';
  async function loadErrorHandoffMode(): Promise<void> {
    try {
      const cfg = await invoke<{ error_handoff?: { mode?: string } }>('config_get');
      const m = cfg.error_handoff?.mode;
      errorHandoffMode = m === 'off' || m === 'assist' ? m : 'detect';
    } catch {
      errorHandoffMode = 'detect';
    }
  }

  /** Current absolute buffer row of the cursor (scrollback baseY + cursorY). */
  function currentBufferRow(): number {
    const buf = term?.buffer.active;
    return buf ? buf.baseY + buf.cursorY : 0;
  }

  // Phase 5 / R1 — the active "explain" handoff for this pane. One at a time: a
  // per-failure unique action id (fixes B1 registry-key collisions) + the
  // FailureContext drives the ErrorResultPopout. Clustering: an identical
  // consecutive failure (same command + exit) reuses the live explain instead
  // of firing a second invoke, which kills retry-loop affordance spam.
  let explainSeq = 0;
  let activeExplain = $state<{
    actionId: string;
    failure: FailureContext;
    clusterKey: string;
    /** R3 — the fix invocation id, set once "Propose a fix" is clicked. */
    fixActionId: string | null;
  } | null>(null);

  function startExplain(failure: FailureContext): void {
    const clusterKey = failureClusterKey(failure.command, failure.exitCode);
    // Dedup: identical failure already being explained → keep the open one.
    if (activeExplain && activeExplain.clusterKey === clusterKey) return;
    const seq = ++explainSeq;
    const actionId = errorActionId(ERROR_EXPLAIN_ACTION, paneId ?? -1, seq);
    const action: DeclaredAction = { id: actionId, target: 'terminal', label: 'explain error' };
    activeExplain = { actionId, failure, clusterKey, fixActionId: null };
    void actionRegistry.invoke(action, failure).catch((err) => {
      console.warn('[Terminal] explain invoke failed', err);
    });
  }

  // R3 — invoke the fix action for the currently-open failure. Distinct action
  // id namespace (rift.error.fix::…) so it never collides with the explain.
  function startFix(): void {
    if (!activeExplain || activeExplain.fixActionId) return;
    const seq = ++explainSeq;
    const fixId = errorActionId(ERROR_FIX_ACTION, paneId ?? -1, seq);
    const action: DeclaredAction = { id: fixId, target: 'terminal', label: 'fix error' };
    activeExplain = { ...activeExplain, fixActionId: fixId };
    void actionRegistry.invoke(action, activeExplain.failure).catch((err) => {
      console.warn('[Terminal] fix invoke failed', err);
    });
  }

  function cancelFix(): void {
    if (activeExplain) activeExplain = { ...activeExplain, fixActionId: null };
  }

  // R3 — insert the proposed command WITHOUT a newline (never auto-runs); the
  // user reviews it on the prompt and presses Enter themselves.
  function insertFixCommand(cmd: string): void {
    pasteTextIntoTerminal(cmd);
    dismissExplain();
  }

  function dismissExplain(): void {
    activeExplain = null;
  }

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

  /**
   * Insert raw text at the cursor without path-quoting (used for notification
   * event injection — commands / error messages, not file paths). Trailing
   * space keeps it editable; never appends a newline, so it is never
   * auto-executed. Bracketed-paste mode (xterm `paste`) further guards against
   * embedded control sequences executing.
   */
  function pasteTextIntoTerminal(text: string): void {
    if (!term || !text) return;
    term.paste(text + ' ');
    term.focus();
  }

  /**
   * Map a terminal-detected ABSOLUTE path to a project-RELATIVE one the
   * in-cockpit Viewer (CodeMirror) can open. `fs_read_text` rejects absolute
   * paths and confines reads to the project root, so click-to-edit only
   * applies to files inside the active project. Returns null for the root
   * itself or any path outside it (slash-normalized, case-insensitive to suit
   * Windows). The returned relative path preserves the original casing.
   */
  function toProjectRelative(abs: string, root: string | null): string | null {
    if (!root) return null;
    const norm = (s: string) => s.replace(/\\/g, '/').replace(/\/+$/, '');
    const a = norm(abs);
    const r = norm(root);
    const aCmp = a.toLowerCase();
    const rCmp = r.toLowerCase();
    if (aCmp === rCmp) return null;
    if (!aCmp.startsWith(rCmp + '/')) return null;
    return a.slice(r.length + 1);
  }

  /**
   * Render a per-command status badge (exit code + duration) at the current
   * cursor row — the CMD_END boundary. An xterm marker + decoration keeps the
   * badge anchored to that scrollback line across reflow/scroll/resize, and the
   * decoration is auto-disposed when the line ages out of the scrollback
   * buffer (and on `term.dispose`). Forcing the element to full container
   * width in `onRender` keeps the badge pinned to the right margin even after
   * the terminal is resized narrower than it was at command time.
   */
  function addCommandBadge(
    marker: IMarker | null,
    exitCode: number,
    durationMs: number | null,
    failure: FailureContext | null,
    blockId: string | null,
  ): void {
    if (!term || !marker) return;
    const decoration = term.registerDecoration({ marker, x: 0, width: term.cols });
    if (!decoration) return;
    const ok = exitCode === 0;
    const dur = durationMs != null ? ` · ${formatDuration(durationMs)}` : '';
    const label = `${ok ? '✓' : '✗'} ${exitCode}${dur}`;
    decoration.onRender((el: HTMLElement) => {
      el.style.width = '100%';
      el.style.left = '0';
      el.classList.add('cmd-badge-row');
      if (el.querySelector('.cmd-badge')) return;
      // Phase 5 / R1 — a failed command's badge becomes an interactive
      // affordance: left-click hands the captured FailureContext to the local
      // explain provider (unchanged). N3.3 adds a right-click block menu (copy
      // / bookmark) on it, and turns the success badge into a clickable block
      // menu too. The row is pointer-events:none; the buttons re-enable them.
      if (!ok && failure) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'cmd-badge err interactive';
        btn.innerHTML = `<span class="cmd-badge-label">${label}</span><span class="cmd-badge-cta">explain</span>`;
        btn.setAttribute(
          'aria-label',
          `Command failed with exit ${exitCode}. Explain the error; right-click for actions.`,
        );
        btn.title = 'Explain this error with a local model';
        btn.addEventListener('click', (ev) => {
          ev.stopPropagation();
          startExplain(failure);
        });
        if (blockId) {
          btn.addEventListener('contextmenu', (ev) => {
            ev.preventDefault();
            ev.stopPropagation();
            openBlockMenu(blockId, ev);
          });
          registerBadgeEl(blockId, btn);
        }
        el.appendChild(btn);
      } else if (blockId) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'cmd-badge interactive-block ' + (ok ? 'ok' : 'err');
        btn.textContent = label;
        btn.setAttribute('aria-label', `Command exited ${exitCode}. Open actions (copy, bookmark).`);
        btn.title = 'Command actions — copy, bookmark';
        const open = (ev: MouseEvent) => {
          ev.preventDefault();
          ev.stopPropagation();
          openBlockMenu(blockId, ev);
        };
        btn.addEventListener('click', open);
        btn.addEventListener('contextmenu', open);
        el.appendChild(btn);
        registerBadgeEl(blockId, btn);
      } else {
        // Anonymous CMD_END (no captured command) — passive span, as before.
        const badge = document.createElement('span');
        badge.className = 'cmd-badge' + (ok ? ' ok' : ' err');
        badge.textContent = label;
        el.appendChild(badge);
      }
    });
  }

  /**
   * Phase 5 / R0 — pair the just-completed command with its cached submit-time
   * context and, on a non-zero exit, assemble + publish a `command.failed`
   * event carrying the FailureContext. No agent call and no UI here — this is
   * the isolation-verifiable capture foundation (inspect via bus_history). R1
   * wires the interactive affordance + the explain provider onto this event.
   */
  function captureFailureContext(
    exitCode: number,
    durationMs: number | null,
  ): FailureContext | null {
    const capture = pendingCapture;
    pendingCapture = null; // consume the pairing regardless of outcome
    if (exitCode === 0 || !capture || !term) return null;
    const buffer: BufferLike = term.buffer.active;
    const ctx = assembleFailureContext(capture, {
      exitCode,
      durationMs,
      endRow: currentBufferRow(),
      buffer,
    });
    if (import.meta.env.DEV) {
      console.debug('[Terminal] FailureContext', summarizeFailureContext(ctx));
    }
    // `off` suppresses surfacing entirely — no command.failed, so the issues
    // list never collects it either. pendingCapture is still consumed above.
    if (errorHandoffMode !== 'off') {
      void busPublish('pty', 'command.failed', {
        session_id: sessionId,
        ...ctx,
      }).catch((err) => console.warn('[Terminal] command.failed publish failed', err));
    }
    return ctx;
  }

  function onTermDragOver(e: DragEvent): void {
    // Only claim the drop when the payload is ours — lets other drag sources pass through.
    const types = e.dataTransfer?.types;
    if (!types || (!types.includes(TREE_PATH_MIME) && !types.includes(RIFT_EVENT_MIME))) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'copy';
    dragHover = true;
  }

  function onTermDragLeave(): void {
    dragHover = false;
  }

  function onTermDrop(e: DragEvent): void {
    dragHover = false;
    // Notification-event injection takes the raw-text path; tree paths are quoted.
    const eventText = e.dataTransfer?.getData(RIFT_EVENT_MIME);
    if (eventText) {
      e.preventDefault();
      pasteTextIntoTerminal(eventText);
      return;
    }
    const path = e.dataTransfer?.getData(TREE_PATH_MIME);
    if (!path) return;
    e.preventDefault();
    pasteIntoTerminal(path);
  }

  /**
   * Stable key for the active-terminal inject registry (single terminal = -1).
   * `paneId` is fixed for a given Terminal instance — the grid keys each
   * pane's component by id — so reading it at call time is equivalent to a
   * captured value, without the rune "initial value" caveat.
   */
  function injectRegistryKey(): number {
    return paneId ?? -1;
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

  /** Mark this terminal as the active injection target on focus. */
  function onTermFocusIn(): void {
    setActiveInjector(injectRegistryKey());
    // Stage 2: the focused pane is the one whose buffer we snapshot (so
    // multiple panes don't clobber the single snapshot file). Stays set to the
    // last-focused pane until another takes focus.
    if (paneId !== undefined) setActivePane(paneId);
  }

  /** Human-readable age of a snapshot for the restore divider. */
  function relativeAge(savedMs: number): string {
    const delta = Date.now() - savedMs;
    return delta < 1000 ? 'moments ago' : `${formatDuration(delta)} ago`;
  }

  /** Build this pane's current snapshot (serialized buffer + live cwd + dims).
   *  Returns null until the terminal is serializable. Registered as this pane's
   *  provider so the coordinator can gather the whole active session at once. */
  function buildPaneSnapshot(): PaneSnapshot | null {
    if (!term || !serializeAddon) return null;
    try {
      return {
        pane_id: paneId ?? 0,
        serialized: serializeAddon.serialize({ scrollback: 2000 }),
        cwd: effectiveCwd ?? projectPath ?? '',
        rows: term.rows,
        cols: term.cols,
        project_root: projectPath ?? null,
      };
    } catch {
      return null;
    }
  }

  /** Drive a full active-session snapshot (all panes + layout) via the
   *  coordinator. Focused-pane-only so multiple panes don't each fire a write;
   *  best-effort — never disrupts the live terminal. */
  function captureSnapshot(): void {
    if (!isActivePane(paneId)) return;
    try {
      sessionManager.captureActiveSession();
    } catch {
      /* best-effort */
    }
  }

  const encoder = new TextEncoder();

  $effect(() => {
    if (visible && term && fit && opened) {
      tick().then(() => {
        fit?.fit();
        if (term) term.refresh(0, term.rows - 1);
        // N3.4 — flush a jump requested while this pane was hidden, now that
        // it's visible and refit (so scrollToLine lands on the right row).
        if (pendingJump) {
          const id = pendingJump;
          pendingJump = null;
          doScrollToBlock(id);
        }
      });
    }
  });

  onMount(async () => {
    const settings = await getTerminalSettings();
    configFontSize = settings.fontSize;
    runtimeFontSize = settings.fontSize;
    lanesEnabled = settings.lanesEnabled;
    void loadErrorHandoffMode();

    const initTheme = resolveTheme(settings.colorPalette, settings.customPalette);
    // Sync CSS --term-bg to the palette background so the terminal host and any
    // gaps around the xterm canvas match the active theme. Decoupled from
    // --bg-base (2026-05-30) — chrome panels carry their own warm-graphite
    // ladder and must NOT follow the terminal to vantablack.
    if (initTheme.background) {
      document.documentElement.style.setProperty('--term-bg', initTheme.background);
    }
    term = new XTerm({
      fontFamily: '"JetBrains Mono", monospace',
      fontSize: settings.fontSize,
      lineHeight: settings.lineHeight,
      scrollback: settings.scrollback,
      cursorBlink: settings.cursorBlink,
      cursorStyle: settings.cursorStyle,
      theme: initTheme,
      // Required by @xterm/addon-ligatures: registerCharacterJoiner is a
      // proposed xterm API and throws on activate() without this flag.
      allowProposedApi: true,
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    search = new SearchAddon();
    term.loadAddon(search);
    // Stage 2: serializer used to snapshot the buffer for restart-safe restore.
    serializeAddon = new SerializeAddon();
    term.loadAddon(serializeAddon);
    // Register this pane's snapshot provider so the coordinator can gather the
    // whole active session (all leaves + layout) in one write.
    registerPaneProvider(paneId, buildPaneSnapshot);

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
    opened = true;

    // Programming ligatures (JetBrains Mono: → ⇒ != >= === |>). The addon
    // registers a character joiner synchronously in activate(); its callback
    // returns a built-in fallback ligature set (Iosevka "calt") whenever real
    // font data isn't loaded — so ligatures work in a webview without any
    // system-font access. Loaded BEFORE WebGL so the GPU texture atlas is built
    // with the ligature font-feature-settings already applied (per
    // @xterm/addon-ligatures docs — webgl must be activated after ligatures).
    //
    // WebView2 hygiene: the addon's lazy font-detection probes
    // `window.queryLocalFonts()` on first render, but that call HANGS in an
    // embedded webview (permission-gated, no prompt ever surfaces), leaving a
    // dangling promise. Rift bundles JetBrains Mono and never needs the Local
    // Font Access API, so we remove the property to make the addon skip
    // straight to its fallback set cleanly. This is a cleanup, not the enabler
    // — the fallback joiner is registered and rendering regardless.
    try {
      delete (window as { queryLocalFonts?: () => unknown }).queryLocalFonts;
      const lig = new LigaturesAddon();
      term.loadAddon(lig);
      ligatures = lig;
    } catch (e) {
      console.warn('[rift] ligatures unavailable', e);
    }

    // GPU-accelerated rendering — crisper glyphs on the amber/vantablack
    // palette + headroom for Rift's high-volume lane-tagged output. Loaded
    // AFTER ligatures so the texture atlas includes ligature glyphs. Wrapped
    // because WebGL context creation can fail (no GL, or >~16 live contexts
    // when many sessions are open) — on failure xterm keeps its DOM renderer.
    // onContextLoss disposes the addon so a GPU reset/suspend in WebView2
    // degrades to DOM instead of blanking.
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => {
        gl.dispose();
        webgl = undefined;
      });
      term.loadAddon(gl);
      webgl = gl;
    } catch (e) {
      console.warn('[rift] WebGL renderer unavailable — using DOM renderer', e);
    }

    if (lanesEnabled) {
      tintManager = new LaneTintManager(term);
    }

    // N3.2 — keep the sticky command header in sync with scroll position.
    // xterm owns this listener's lifetime (disposed on term.dispose()).
    term.onScroll(() => recomputeSticky());

    // Exposed for MCP pty_read tool — do not remove without updating mcp_host.rs tool_pty_read
    window.__RIFT_TERM__ = term;

    // Path intelligence — detect file paths in terminal output and show hover previews.
    const PATH_RE = /(?:[A-Za-z]:\\|\/)[^\s:*?"<>|]+\.[a-zA-Z0-9]{1,10}/g;

    const linkProvider: RiftLinkProvider = {
      provideLinks(bufferLineNumber: number, callback: (links: RiftLink[] | undefined) => void) {
        const line = term?.buffer.active.getLine(bufferLineNumber);
        if (!line) { callback(undefined); return; }
        const text = line.translateToString();
        const links: RiftLink[] = [];
        let match: RegExpExecArray | null;
        PATH_RE.lastIndex = 0;
        while ((match = PATH_RE.exec(text)) !== null) {
          const startX = match.index + 1;
          const endX = startX + match[0].length - 1;
          const filePath = match[0];
          links.push({
            text: filePath,
            range: { start: { x: startX, y: bufferLineNumber }, end: { x: endX, y: bufferLineNumber } },
            activate(_event: MouseEvent, _text: string) {
              // Click-to-edit (§11 editor scope): open project files in the
              // in-cockpit CodeMirror Viewer. The Viewer is project-scoped, so
              // relativize first; paths outside the project keep the prior
              // (harmless) preview-fetch fallback rather than a dead click.
              const rel = toProjectRelative(filePath, projectPath);
              if (rel) {
                popouts.summon({ content: { kind: 'viewer', path: rel } });
              } else {
                invoke('file_preview', { path: filePath }).catch(() => {});
              }
            },
            hover(e: MouseEvent, _text: string) {
              tooltipX = e.clientX + 12;
              tooltipY = e.clientY + 12;
              tooltipFilename = filePath.split(/[\\/]/).pop() || filePath;
              // Debounce: cancel any pending fetch, skip if same path already in-flight.
              if (hoverTimer !== null) { clearTimeout(hoverTimer); hoverTimer = null; }
              if (hoverPendingPath === filePath) return;
              hoverPendingPath = filePath;
              hoverTimer = setTimeout(() => {
                hoverTimer = null;
                invoke('file_preview', { path: filePath }).then((result) => {
                  tooltipPreview = result as FilePreview;
                  tooltipVisible = true;
                }).catch(() => {
                  tooltipVisible = false;
                }).finally(() => {
                  hoverPendingPath = null;
                });
              }, 150);
            },
            leave(_event: MouseEvent, _text: string) {
              if (hoverTimer !== null) { clearTimeout(hoverTimer); hoverTimer = null; }
              hoverPendingPath = null;
              tooltipVisible = false;
              tooltipPreview = null;
            },
          });
        }
        callback(links.length > 0 ? links : undefined);
      },
    };
    term.registerLinkProvider(linkProvider);

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

    // PTY output via Tauri events instead of Channel. The Channel mechanism
    // holds a stale WebView2 reference from the initial page load — Vite's
    // dep re-optimization causes an internal webview recreation that
    // permanently breaks Channel callback delivery (runCallback never fires).
    // Events use emit_to("main", ...) which resolves the CURRENT webview by
    // label every time, so they always work.
    //
    // Buffer: events arrive before sessionId is known (PTY starts sending
    // immediately). Buffer them and flush after pty_start returns.
    type PtyChunkPayload = { id: number; b64: string };
    const chunkBuffer: { id: number; data: Uint8Array }[] = [];
    let chunkListenerReady = false;

    unlistenChunk = await listen<PtyChunkPayload>('pty-chunk', (event) => {
      const raw = atob(event.payload.b64);
      const data = new Uint8Array(raw.length);
      for (let i = 0; i < raw.length; i++) data[i] = raw.charCodeAt(i);
      if (sessionId !== null && event.payload.id === sessionId) {
        term?.write(data);
      } else if (!chunkListenerReady) {
        chunkBuffer.push({ id: event.payload.id, data });
      }
    });

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

    let ptyStarted = false;
    const startPty = async () => {
      if (ptyStarted || !term) return;
      ptyStarted = true;

      // No fit() here: deferredFit() already sized the canvas immediately above
      // (no layout-changing await in between), so term.rows/cols are current.
      // A second fit at this point only re-measures the same layout and can
      // re-trigger a canvas resize, adding a launch flash. The post-start
      // refitAndResize() rAF below is the one legitimate settle pass.
      const startRows = Math.max(term.rows, 24);
      const startCols = Math.max(term.cols, 80);
      if (term.rows !== startRows || term.cols !== startCols) {
        term.resize(startCols, startRows);
      }

      // Stage 2 restart-safe restore: re-hydrate THIS pane's scrollback + cwd
      // (and surface the compaction digest) BEFORE the fresh shell prompt, so
      // the restored history sits above it. Each pane matches its own snapshot
      // by id and restores once (Stage 2b multi-pane). The dead shell can't
      // return; its context can. Best-effort — never block a fresh shell.
      let restoreCwd: string | undefined;
      try {
        const payload = await getRestorePayload();
        const snap = payload?.panes.find((p) => p.pane_id === paneId);
        if (payload && snap && claimPane(paneId)) {
          if (snap.serialized) term.write(snap.serialized);
          term.write('\r\n');
          term.writeln(
            laneFormatGated(
              'SYS',
              `─── restored from ${relativeAge(payload.saved_ms)} · fresh shell ───`,
              lanesEnabled,
            ),
          );
          if (payload.digest) {
            term.writeln(laneFormatGated('SYS', `↻ Last session: ${payload.digest}`, lanesEnabled));
          }
          if (snap.cwd) restoreCwd = snap.cwd;
        }
      } catch {
        /* restore is best-effort */
      }
      effectiveCwd = restoreCwd ?? projectPath ?? null;

      try {
        sessionId = await invoke<number>('pty_start', {
          rows: startRows,
          cols: startCols,
          cwd: restoreCwd ?? projectPath ?? undefined,
          // Pass the pane id as the session identity. The backend injects it as
          // $RIFT_SESSION_ID so the CC status bridge tees per-session and the
          // GUI status line tracks the FOCUSED pane (sessionManager keys focus
          // by this same pane id). Distinct from the returned PTY registry id,
          // which is the I/O handle used for pty_write/resize/kill.
          sessionId: paneId,
        });
        alive = true;

        // Surface the PTY registry id so App.svelte can sample this pane's
        // process-tree resources for the StatusLine CPU/RAM segments.
        if (paneId !== undefined && sessionId !== null) {
          registerPtyId(paneId, sessionId);
          // N3.4 — register this pane's block-scroller, keyed by the PTY
          // sessionId that commandBlockStore records, so the palette can jump.
          registerScroller(sessionId, paneId, scrollToBlockLocal);
        }

        // Flush buffered chunks that arrived before sessionId was known
        for (const buffered of chunkBuffer) {
          if (buffered.id === sessionId) {
            term.write(buffered.data);
          }
        }
        chunkBuffer.length = 0;
        chunkListenerReady = true;

        invoke('terminal_mounted').catch(() => {});
      } catch (err) {
        term.writeln(`\r\n${laneFormatGated('ERR', `pty_start failed: ${err}`, lanesEnabled)}`);
        ptyStarted = false;
        return;
      }

      term.focus();
    };

    await startPty();
    requestAnimationFrame(refitAndResize);

    // Stage 2: periodic buffer snapshot so a Rift restart can re-hydrate this
    // pane. Only the focused pane actually writes (see captureSnapshot), so
    // many panes don't clobber the single snapshot file. Gated on the same
    // opt-in flag that enables restore — no point snapshotting if never read.
    void getSnapshotConfig().then((snapCfg) => {
      if (!snapCfg.restoreEnabled || snapCfg.intervalSeconds <= 0) return;
      snapshotTimer = setInterval(captureSnapshot, snapCfg.intervalSeconds * 1000);
    });

    // One-shot launch command for this pane (e.g. Gemini "sign in" opens a tab
    // that auto-runs `gemini`). Sent after a short delay so the shell prompt is
    // ready to read it; guarded by `alive` so a teardown before it fires is a
    // no-op. Consumed (read-and-cleared) so a remount never re-runs it.
    if (paneId !== undefined && sessionId !== null) {
      const launchCmd = sessionManager.consumeInitialCommand(paneId);
      if (launchCmd) {
        const idForLaunch = sessionId;
        setTimeout(() => {
          if (alive && idForLaunch !== null) {
            const bytes = Array.from(encoder.encode(`${launchCmd}\r`));
            invoke('pty_write', { id: idForLaunch, bytes }).catch(() => {});
          }
        }, 700);
      }
    }

    // Post-startup content guarantee — detects "PTY started but nothing
    // visible" and forces recovery. This is the safety net that catches
    // ALL timing/rendering race conditions (canvas zero-size at write time,
    // fit() measuring stale dimensions, shell delayed prompt, etc.)
    // instead of trying to prevent every edge case upfront.
    let contentChecks = 0;
    contentGuardTimer = setInterval(() => {
      contentChecks++;
      if (!term || !alive) {
        if (contentChecks >= 10) { clearInterval(contentGuardTimer); contentGuardTimer = undefined; }
        return;
      }

      let hasContent = false;
      for (let row = 0; row < Math.min(term.rows, 5); row++) {
        const line = term.buffer.active.getLine(row);
        if (line && line.translateToString(true).length > 0) {
          hasContent = true;
          break;
        }
      }

      if (hasContent) {
        clearInterval(contentGuardTimer);
        contentGuardTimer = undefined;
        return;
      }

      // No visible content yet — force refit + refresh
      try { fit?.fit(); } catch { /* best-effort */ }
      term.refresh(0, term.rows - 1);
      term.scrollToBottom();
      term.focus();

      // After a few attempts with no content, send Ctrl+L to force
      // the shell to redraw its prompt.
      if (contentChecks >= 3 && sessionId !== null && alive) {
        invoke('pty_write', { id: sessionId, bytes: Array.from(encoder.encode('\x0c')) }).catch(() => {});
      }

      if (contentChecks >= 10) {
        clearInterval(contentGuardTimer);
        contentGuardTimer = undefined;
      }
    }, 300);

    term.onData((data) => {
      if (sessionId === null || !alive) return;
      const bytes = Array.from(encoder.encode(data));
      invoke('pty_write', { id: sessionId, bytes }).catch((err) => {
        term?.writeln(`\r\n${laneFormatGated('ERR', `pty_write failed: ${err}`, lanesEnabled)}`);
      });
    });

    resizeObs = new ResizeObserver(() => {
      if (resizeRaf !== undefined) cancelAnimationFrame(resizeRaf);
      resizeRaf = requestAnimationFrame(() => {
        resizeRaf = undefined;
        try { fit?.fit(); } catch { /* best-effort */ }
        if (sessionId !== null && alive && term && term.rows > 1 && term.cols > 1) {
          if (term.rows !== lastResizeRows || term.cols !== lastResizeCols) {
            lastResizeRows = term.rows;
            lastResizeCols = term.cols;
            invoke('pty_resize', {
              id: sessionId,
              rows: term.rows,
              cols: term.cols,
            }).catch(() => { /* best-effort */ });
          }
        }
      });
    });
    resizeObs.observe(host);

    // Phase 8.7 — vault-drop event listener (manual gesture from IndexGraph).
    host.addEventListener(RIFT_VAULT_DROP_EVENT, onTermVaultDrop);

    // Notification-event injection: register this terminal's raw-text paste so
    // a click-to-inject from a notification tab targets the active terminal,
    // and mark this terminal active whenever its xterm textarea gains focus
    // (focusin bubbles from the textarea through the host).
    registerInjector(injectRegistryKey(), pasteTextIntoTerminal);
    host.addEventListener('focusin', onTermFocusIn);

    // Live settings application — re-read all terminal settings when config
    // changes and apply them to the running xterm instance immediately.
    const onConfigChanged = async () => {
      invalidateTerminalSettingsCache();
      void loadErrorHandoffMode();
      const fresh = await getTerminalSettings();
      if (!term) return;
      const theme = resolveTheme(fresh.colorPalette, fresh.customPalette);
      term.options.theme = theme;
      if (fresh.fontSize !== runtimeFontSize) {
        runtimeFontSize = fresh.fontSize;
        configFontSize = fresh.fontSize;
        term.options.fontSize = fresh.fontSize;
      }
      term.options.lineHeight = fresh.lineHeight;
      term.options.scrollback = fresh.scrollback;
      term.options.cursorStyle = fresh.cursorStyle;
      term.options.cursorBlink = fresh.cursorBlink;
      lanesEnabled = fresh.lanesEnabled;
      // Sync CSS --term-bg so the terminal host matches the palette (chrome
      // keeps its own ladder — see onMount note).
      if (theme.background) {
        document.documentElement.style.setProperty('--term-bg', theme.background);
      }
      try { fit?.fit(); } catch { /* best-effort */ }
      term.refresh(0, term.rows - 1);
    };
    window.addEventListener('rift:config-changed', onConfigChanged);

    // Live palette hover preview from SettingsPanel.
    let savedPaletteId: string | null = null;
    const onPalettePreview = (e: Event) => {
      const id = (e as CustomEvent<string | null>).detail;
      if (!term) return;
      if (id) {
        if (!savedPaletteId) savedPaletteId = settings.colorPalette;
        const previewTheme = resolveTheme(id, settings.customPalette);
        term.options.theme = previewTheme;
        if (previewTheme.background) {
          document.documentElement.style.setProperty('--term-bg', previewTheme.background);
        }
        term.refresh(0, term.rows - 1);
      } else if (savedPaletteId) {
        const restoredTheme = resolveTheme(savedPaletteId, settings.customPalette);
        term.options.theme = restoredTheme;
        if (restoredTheme.background) {
          document.documentElement.style.setProperty('--term-bg', restoredTheme.background);
        }
        term.refresh(0, term.rows - 1);
        savedPaletteId = null;
      }
    };
    window.addEventListener('rift:palette-preview', onPalettePreview);

    configChangedCleanup = () => {
      window.removeEventListener('rift:config-changed', onConfigChanged);
      window.removeEventListener('rift:palette-preview', onPalettePreview);
    };

    // Run pty_exited listener and lane bus subscription in parallel —
    // they're independent async IPC calls (~5-10ms each, saves one round trip).
    const [exitUnsub, laneResult] = await Promise.all([
      listen<PtyExited>('pty_exited', (event) => {
        if (event.payload.id !== sessionId) return;
        alive = false;
        onPtyExited?.();
        term?.writeln(
          `\r\n${laneFormatGated(
            'SYS',
            `session ${event.payload.id} exited code=${event.payload.code}`,
            lanesEnabled,
          )}`,
        );
      }),
      busSubscribe({ category: 'pty' }, (env: Envelope) => {
        if (env.kind === 'command.submitted') {
          // Phase 5 / R0 — capture the command text (only present on this
          // event) + cwd + the prompt's buffer row, to pair with the exit code
          // when the command completes.
          const s = env.payload as { session_id?: number; command?: string } | null;
          if (!s || typeof s.command !== 'string') return;
          if (s.session_id !== undefined && s.session_id !== sessionId) return;
          pendingCapture = {
            command: s.command,
            cwd: effectiveCwd,
            startRow: currentBufferRow(),
            ts: Date.now(),
          };
          // N3.2 — drop a live marker at the prompt row to anchor this block's
          // top edge. Dispose any orphan from a submit that never completed.
          pendingStartMarker?.dispose();
          pendingStartMarker = term?.registerMarker(0) ?? null;
          return;
        }
        if (env.kind === 'command.completed') {
          const c = env.payload as { session_id?: number; exit_code?: number; duration_ms?: number | null } | null;
          if (!c || typeof c.exit_code !== 'number') return;
          if (c.session_id !== undefined && c.session_id !== sessionId) return;
          // Snapshot the submit-time capture + start marker BEFORE
          // captureFailureContext consumes (nulls) pendingCapture.
          const cap = pendingCapture;
          const startMarker = pendingStartMarker;
          pendingStartMarker = null;
          const endRow = currentBufferRow();
          // Assemble the FailureContext first so a non-zero exit's badge can
          // carry it and become an interactive "explain" affordance. `off`
          // keeps the badge passive; `assist` auto-invokes explain.
          const failure = captureFailureContext(c.exit_code, c.duration_ms ?? null);
          const surfaced = failure && errorHandoffMode !== 'off';
          // End-edge marker — shared by the badge decoration and the block's
          // live region (N3.3 copy). xterm disposes it on scrollback eviction.
          const endMarker = term?.registerMarker(0) ?? null;
          // N3.1/N3.3 — record an addressable command block (the sticky-header /
          // copy / bookmark / jump spine). EVERY command (success + failure)
          // with a captured submit becomes a block; an anonymous CMD_END with
          // no matching submit (no command text) isn't an addressable unit, so
          // it's skipped. Record BEFORE the badge so the badge can carry the id.
          let blockId: string | null = null;
          if (cap) {
            const block = commandBlockStore.record({
              sessionId,
              command: cap.command,
              cwd: cap.cwd,
              exitCode: c.exit_code,
              durationMs: c.duration_ms ?? null,
              startRow: cap.startRow,
              endRow,
              ts: Date.now(),
            });
            blockId = block.id;
            // N3.2/N3.3 — anchor the block's live top + bottom edges. Refresh
            // the header in case the user is scrolled up while it completes.
            if (startMarker && startMarker.line >= 0 && endMarker && endMarker.line >= 0) {
              paneBlocks.set(block.id, { startMarker, endMarker, block });
            } else {
              startMarker?.dispose();
            }
            recomputeSticky();
          } else {
            startMarker?.dispose();
          }
          addCommandBadge(endMarker, c.exit_code, c.duration_ms ?? null, surfaced ? failure : null, blockId);
          if (surfaced && errorHandoffMode === 'assist') startExplain(failure);
          return;
        }
        if (env.kind === 'cwd.changed') {
          // Stage 2b: track the live shell cwd so the next snapshot (and thus
          // the next restore) lands in the directory the user actually cd'd to,
          // not the spawn cwd.
          const w = env.payload as { session_id?: number; cwd?: string } | null;
          if (!w?.cwd) return;
          if (w.session_id !== undefined && w.session_id !== sessionId) return;
          effectiveCwd = w.cwd;
          return;
        }
        if (env.kind !== 'lane.changed') return;
        const p = env.payload as { lane?: string; session_id?: number } | null;
        if (!p?.lane) return;
        if (p.session_id !== undefined && p.session_id !== sessionId) return;
        currentLane = p.lane;
        tintManager?.onLaneChanged(p.lane);
      }).then((u) => {
        if (!laneMounted) { void u().catch(() => {}); return undefined; }
        return u;
      }).catch((err) => {
        console.warn('[Terminal] lane bus subscription failed', err);
        return undefined;
      }),
    ]);
    // Mount-race guard: if onDestroy already ran (fast split-then-close), it
    // called unlistenExited?.() against undefined — so drop this now-orphaned
    // listener immediately rather than stashing it in a dead variable. Mirrors
    // the laneMounted check the lane subscription does at .then() above.
    if (!laneMounted) { exitUnsub(); }
    else { unlistenExited = exitUnsub; }
    if (laneResult) unsubscribeLane = laneResult;
  });

  onDestroy(() => {
    laneMounted = false;
    // Stage 2: one last full-session snapshot before teardown (best-effort, and
    // captured while this pane's provider is still registered), then stop the
    // timer + drop the provider. Periodic capture is the real safety net — a
    // hard window close may not flush this, but at most one interval is lost.
    captureSnapshot();
    unregisterPaneProvider(paneId);
    if (snapshotTimer) clearInterval(snapshotTimer);
    if (sessionId !== null && alive) {
      invoke('pty_kill', { id: sessionId }).catch(() => {});
    }
    unlistenExited?.();
    unlistenChunk?.();
    configChangedCleanup?.();
    tintManager?.dispose();
    unsubscribeLane?.().catch(() => {});
    if (hoverTimer !== null) { clearTimeout(hoverTimer); hoverTimer = null; }
    if (recoveryTimer) clearInterval(recoveryTimer);
    if (contentGuardTimer) clearInterval(contentGuardTimer);
    if (resizeRaf !== undefined) cancelAnimationFrame(resizeRaf);
    resizeObs?.disconnect();
    host?.removeEventListener(RIFT_VAULT_DROP_EVENT, onTermVaultDrop);
    host?.removeEventListener('focusin', onTermFocusIn);
    unregisterInjector(injectRegistryKey());
    unregisterPtyId(injectRegistryKey());
    if (sessionId !== null) unregisterScroller(sessionId);
    ligatures?.dispose();
    webgl?.dispose();
    search?.dispose();
    serializeAddon?.dispose();
    fit?.dispose();
    term?.dispose();
    delete window.__RIFT_TERM__;
  });
</script>

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
  <StickyCommandHeader block={stickyBlock} />
  {#if blockMenu}
    <BlockActionMenu
      block={blockMenu.block}
      x={blockMenu.x}
      y={blockMenu.y}
      onCopyCommand={() => copyBlockCommand(blockMenu!.block.id)}
      onCopyOutput={() => copyBlockOutput(blockMenu!.block.id)}
      onCopyBoth={() => copyBlockBoth(blockMenu!.block.id)}
      onToggleBookmark={() => toggleBlockBookmark(blockMenu!.block.id)}
      onClose={() => (blockMenu = null)}
    />
  {/if}
  {#if searchOpen && search}
    <TerminalSearch searchAddon={search} onclose={() => { searchOpen = false; term?.focus(); }} />
  {/if}
  {#if activeExplain}
    <ErrorResultPopout
      actionId={activeExplain.actionId}
      failure={activeExplain.failure}
      fixActionId={activeExplain.fixActionId}
      onProposeFix={startFix}
      onCancelFix={cancelFix}
      onInsertCommand={insertFixCommand}
      onDismiss={dismissExplain}
    />
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
    background: var(--term-bg);
    padding: var(--space-8);
    overflow: hidden;
    min-height: 0;
  }
  /* Phase 6.6 — subtle amber inset glow while a tree-path drag hovers */
  .terminal-host.drag-hover {
    box-shadow: inset 0 0 0 2px var(--amber-bright);
  }

  /* Per-command status badge (exit code + duration) rendered as an xterm
     decoration at the CMD_END boundary row. Decoration elements live in
     xterm's overlay container, outside this component's scoped styles —
     hence :global. The row box is forced full-width in onRender; the badge
     pins to the right margin and never intercepts pointer events so text
     selection underneath is unaffected. */
  :global(.cmd-badge-row) {
    pointer-events: none;
  }
  :global(.cmd-badge) {
    position: absolute;
    right: 6px;
    top: 0;
    font-family: var(--font-family);
    font-size: 0.78em;
    line-height: 1.35;
    padding: 0 5px;
    border-radius: var(--radius-sm, 3px);
    white-space: nowrap;
    background: rgba(0, 0, 0, 0.55);
    border: 1px solid transparent;
    font-variant-numeric: tabular-nums;
  }
  :global(.cmd-badge.ok) {
    color: var(--term-green);
    border-color: rgba(79, 232, 85, 0.40);
  }
  :global(.cmd-badge.err) {
    color: var(--term-red);
    border-color: rgba(255, 72, 72, 0.50);
  }
  /* Phase 5 / R1 — interactive failure badge. A real button: re-enables
     pointer events on itself (the row stays none), reveals an "explain" call
     to action on hover/focus, and is keyboard-focusable + screen-reader
     labelled. font:inherit keeps it visually identical to the passive badge. */
  :global(button.cmd-badge.interactive) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    pointer-events: auto;
    cursor: pointer;
    font: inherit;
    font-size: 0.78em;
    transition: border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }
  :global(button.cmd-badge.interactive:hover),
  :global(button.cmd-badge.interactive:focus-visible) {
    background: rgba(255, 72, 72, 0.14);
    border-color: var(--term-red);
    outline: none;
  }
  :global(.cmd-badge .cmd-badge-cta) {
    color: var(--amber-warm);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: lowercase;
    opacity: 0;
    max-width: 0;
    overflow: hidden;
    transition: opacity var(--duration-fast) var(--ease-out), max-width var(--duration-fast) var(--ease-out);
  }
  :global(button.cmd-badge.interactive:hover .cmd-badge-cta),
  :global(button.cmd-badge.interactive:focus-visible .cmd-badge-cta) {
    opacity: 1;
    max-width: 6em;
  }
  /* N3.3 — success / non-surfaced badge as a clickable block-menu trigger.
     Same pointer-events re-enable + hover treatment as the failure badge,
     tinted to the badge's own ok/err color. */
  :global(button.cmd-badge.interactive-block) {
    pointer-events: auto;
    cursor: pointer;
    font: inherit;
    font-size: 0.78em;
    transition:
      border-color var(--duration-fast) var(--ease-out),
      background var(--duration-fast) var(--ease-out);
  }
  :global(button.cmd-badge.interactive-block.ok:hover),
  :global(button.cmd-badge.interactive-block.ok:focus-visible) {
    background: rgba(79, 232, 85, 0.14);
    border-color: var(--term-green);
    outline: none;
  }
  :global(button.cmd-badge.interactive-block.err:hover),
  :global(button.cmd-badge.interactive-block.err:focus-visible) {
    background: rgba(255, 72, 72, 0.14);
    border-color: var(--term-red);
    outline: none;
  }
  /* N3.3 — a bookmarked block's badge is marked with a star. */
  :global(.cmd-badge.bookmarked::before) {
    content: '★ ';
    color: var(--amber-bright);
  }
</style>
