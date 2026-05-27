<script lang="ts">
  // CockpitDetached.svelte — Phase 6.4
  //
  // Whole-app for the detached cockpit window. Renders a local titlebar with
  // a DOCK button + close, the FILE TREE pane header, and the Tree component.
  //
  // Position persistence (design C): on mount, reads `rift.cockpit.detached_pos`
  // from localStorage and immediately repositions the window. On every
  // move/resize event, saves the current outer position + size. The Rust side
  // always creates the window at default 480×800; this script overrides
  // immediately after mount so the window lands on the last-used monitor.
  //
  // Reattach paths (design D): DOCK button invokes `cockpit_reattach` which
  // calls `.destroy()` on this window, which fires WindowEvent::Destroyed on
  // the Rust side, which emits `cockpit_reattached` to main. The × button
  // calls `.close()` — same Destroyed path. Neither button needs to emit
  // the event itself.

  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow, PhysicalPosition, PhysicalSize, availableMonitors } from '@tauri-apps/api/window';
  import type { Window as TauriWindow } from '@tauri-apps/api/window';
  import Tree from './lib/Tree.svelte';
  import IndexGraph from './lib/IndexGraph.svelte';
  import Splitter from './lib/Splitter.svelte';
  import { signalBusReady } from './lib/bus';

  // Detached window has its own bus.ts module scope — the ready-gate
  // starts unresolved. Signal immediately since there are no orphan
  // subscriptions to clean up in a freshly-spawned window.
  signalBusReady();

  // Phase 8.7e — graph/tree split is resizable + persisted (separate
  // localStorage key from main cockpit so the user can tune detached layout
  // independently of the docked one).
  let graphHeightPct = $state(55);

  // Lazy-init at onMount instead of module scope.
  //
  // Phase 8.7c finding (2026-04-29): in a freshly-spawned secondary window,
  // Tauri's `__TAURI_INTERNALS__.metadata` is not yet populated at module
  // evaluation time, and `getCurrentWindow()` accesses `metadata.currentWindow`
  // unconditionally — throwing `Cannot read properties of undefined (reading
  // 'metadata')` and aborting the whole component tree. Result: white window.
  //
  // Main window's TitleBar.svelte uses the same pattern at module scope
  // without crashing because the main webview is created with the runtime
  // already initialized; the cockpit webview is spawned later and the runtime
  // setup races the module load. onMount fires after Svelte's hydration,
  // by which time __TAURI_INTERNALS__ is fully populated.
  let appWindow: TauriWindow;

  const POS_KEY = 'rift.cockpit.detached_pos';

  interface SavedPos {
    x: number;
    y: number;
    width: number;
    height: number;
  }

  // Cockpit pane header data — Tree pushes these up via $bindable props.
  let nodeCount = $state(0);
  let watchedPathLabel = $state('…');

  // ---- position persistence ----

  function savePosition(x: number, y: number, width: number, height: number): void {
    try {
      localStorage.setItem(POS_KEY, JSON.stringify({ x, y, width, height }));
    } catch (err) {
      // Quota or private-browsing restriction — non-fatal, just log.
      console.warn('[CockpitDetached] localStorage write failed:', err);
    }
  }

  async function restoreSavedPosition(): Promise<void> {
    let raw: string | null = null;
    try {
      raw = localStorage.getItem(POS_KEY);
    } catch (err) {
      console.warn('[CockpitDetached] localStorage read failed:', err);
      return;
    }
    if (!raw) return;

    let pos: SavedPos;
    try {
      pos = JSON.parse(raw) as SavedPos;
    } catch (err) {
      console.warn('[CockpitDetached] localStorage parse failed, discarding:', err);
      try { localStorage.removeItem(POS_KEY); } catch { /* ignore */ }
      return;
    }

    // Validate the parsed shape before touching the window.
    if (
      typeof pos.x !== 'number' ||
      typeof pos.y !== 'number' ||
      typeof pos.width !== 'number' ||
      typeof pos.height !== 'number'
    ) {
      console.warn('[CockpitDetached] saved position has unexpected shape, discarding');
      try { localStorage.removeItem(POS_KEY); } catch { /* ignore */ }
      return;
    }

    try {
      const monitors = await availableMonitors();
      const onScreen = monitors.some((m) => {
        const mx = m.position.x;
        const my = m.position.y;
        const mw = m.size.width;
        const mh = m.size.height;
        return pos.x + pos.width > mx + 50 && pos.x < mx + mw - 50
            && pos.y > my - 20 && pos.y < my + mh - 50;
      });
      if (!onScreen) {
        try { localStorage.removeItem(POS_KEY); } catch { /* ignore */ }
        return;
      }
      await appWindow.setPosition(new PhysicalPosition(pos.x, pos.y));
      await appWindow.setSize(new PhysicalSize(pos.width, pos.height));
    } catch (err) {
      console.warn('[CockpitDetached] failed to restore window position:', err);
    }
  }

  async function startPositionTracking(): Promise<void> {
    // Save current position immediately so a crash before a move still records something.
    try {
      const [pos, size] = await Promise.all([
        appWindow.outerPosition(),
        appWindow.outerSize(),
      ]);
      savePosition(pos.x, pos.y, size.width, size.height);
    } catch { /* non-fatal */ }

    // Listen for move + resize — both fire position changes.
    appWindow.onMoved(({ payload: pos }) => {
      appWindow.outerSize().then((size) => {
        savePosition(pos.x, pos.y, size.width, size.height);
      }).catch(() => { /* non-fatal */ });
    });

    appWindow.onResized(({ payload: size }) => {
      appWindow.outerPosition().then((pos) => {
        savePosition(pos.x, pos.y, size.width, size.height);
      }).catch(() => { /* non-fatal */ });
    });
  }

  onMount(() => {
    // Phase 8.7d: cockpit window is pre-built at setup() in lib.rs, so the
    // Tauri runtime is injected before this component mounts. No race; no
    // poll needed. getCurrentWindow() works at module load OR onMount.
    try {
      appWindow = getCurrentWindow();
    } catch (err) {
      console.error('[CockpitDetached] getCurrentWindow() failed:', err);
      return;
    }

    // Restore saved position FIRST so it happens before the user sees the window settle.
    restoreSavedPosition().then(() => {
      startPositionTracking();
    }).catch(() => {
      // restoreSavedPosition catches internally; this outer catch is belt-and-suspenders.
      startPositionTracking();
    });
  });

  // ---- window controls ----

  function onTitlebarMouseDown(e: MouseEvent): void {
    if ((e.target as HTMLElement).closest('button')) return;
    appWindow?.startDragging().catch(() => {});
  }

  async function dock(): Promise<void> {
    try {
      await invoke('cockpit_reattach');
      // `cockpit_reattach` calls `.destroy()` → WindowEvent::Destroyed fires
      // on Rust side → `cockpit_reattached` emitted to main. This window closes.
    } catch (err) {
      console.error('[CockpitDetached] cockpit_reattach failed:', err);
    }
  }

  function close(): void {
    // Close goes through OS close path → WindowEvent::Destroyed → same cleanup.
    appWindow.close();
  }
</script>

<div class="detached-shell" data-tauri-drag-region>
  <!-- Local titlebar (design F — minimal brand + DOCK + close) -->
  <header class="titlebar" role="toolbar" tabindex={-1} data-tauri-drag-region onmousedown={onTitlebarMouseDown}>
    <span class="brand"><span class="glyph">◆</span>RIFT <span class="sub">COCKPIT</span></span>
    <span class="spacer" data-tauri-drag-region></span>
    <div class="controls">
      <button type="button" class="btn dock" aria-label="dock cockpit" onclick={dock}>
        ↙ DOCK
      </button>
      <button type="button" class="btn close" aria-label="close" onclick={close}>×</button>
    </div>
  </header>

  <!-- Phase 8.7d — mirrors App.svelte's cockpit-right split:
       IndexGraph (top) + horizontal Splitter + File Tree (bottom).
       Phase 8.7e — split ratio is resizable + persisted. -->
  <div class="graph-pane" style="flex: 0 0 {graphHeightPct}%;">
    <div class="pane-header">
      <span>INDEX</span>
      <span class="meta">vault graph · fixture</span>
    </div>
    <div class="graph-body">
      <IndexGraph />
    </div>
  </div>

  <Splitter
    direction="horizontal"
    storageKey="rift.cockpit.detached_graph_height_pct"
    unit="percent"
    bind:size={graphHeightPct}
    min={20}
    max={80}
    onDblClick={() => (graphHeightPct = 55)}
  />

  <div class="tree-pane">
    <div class="pane-header">
      <span>FILE TREE</span>
      <span class="meta">{nodeCount} files · {watchedPathLabel}</span>
    </div>
    <div class="tree-body">
      <Tree bind:nodeCount bind:watchedPathLabel />
    </div>
  </div>
</div>

<style>
  .detached-shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg-panel);
    overflow: hidden;
  }

  /* ---- local titlebar ---- */
  .titlebar {
    height: var(--control-lg);
    background: var(--bg-elevated);
    box-shadow: var(--sep-glow);
    display: flex;
    align-items: center;
    padding: 0 var(--space-12);
    user-select: none;
    flex-shrink: 0;
  }

  .brand {
    color: var(--amber-primary);
    font-weight: 700;
    font-size: var(--text-base);
    letter-spacing: 0.15em;
    text-shadow: var(--glow-amber);
  }

  .glyph {
    color: var(--amber-bright);
    margin-right: var(--space-sm);
  }

  .sub {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    font-weight: 400;
    letter-spacing: 0.12em;
    margin-left: var(--space-xs);
  }

  .spacer {
    flex: 1;
    height: 100%;
  }

  .controls {
    display: flex;
    gap: var(--space-8);
    align-items: center;
  }

  .btn {
    height: 14px;
    background: transparent;
    border: 1px solid var(--amber-dim);
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    line-height: 12px;
    text-align: center;
    cursor: pointer;
    padding: 0 5px;
    font-family: inherit;
    letter-spacing: 0.08em;
  }

  .btn:hover {
    color: var(--amber-primary);
    border-color: var(--amber-primary);
  }

  .btn.close {
    width: 14px;
    padding: 0;
    font-size: var(--text-xs);
  }

  .btn.close:hover {
    color: var(--term-red);
    border-color: var(--term-red);
  }

  .btn.dock:hover {
    color: var(--blue-claude, #6CB6FF);
    border-color: var(--blue-claude, #6CB6FF);
  }

  /* ---- graph + tree split — Phase 8.7d (mirrors App.svelte cockpit-right);
     Phase 8.7e: graph-pane flex-basis controlled by Splitter via inline style. ---- */
  .graph-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-base);
  }
  .graph-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .tree-pane {
    flex: 1 1 45%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  /* ---- pane header — matches App.svelte .pane-header exactly ---- */
  .pane-header {
    height: var(--space-24);
    padding: 0 var(--space-md);
    background: var(--bg-elevated);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    flex-shrink: 0;
    user-select: none;
  }

  .pane-header .meta {
    color: var(--amber-faint);
    font-weight: 400;
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
  }

  /* ---- tree body — matches App.svelte .tree-body exactly ---- */
  .tree-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-xs) 0;
  }

  .tree-body::-webkit-scrollbar {
    width: 5px;
  }

  .tree-body::-webkit-scrollbar-thumb {
    background: var(--amber-faint);
  }
</style>
