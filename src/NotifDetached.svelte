<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow, PhysicalPosition, PhysicalSize, availableMonitors } from '@tauri-apps/api/window';
  import type { Window as TauriWindow } from '@tauri-apps/api/window';
  import NotificationPane from './lib/NotificationPane.svelte';
  import AegisTabContent from './lib/AegisTabContent.svelte';
  import IndexTabContent from './lib/IndexTabContent.svelte';
  import BusTailTabContent from './lib/BusTailTabContent.svelte';
  import TodoTabContent from './lib/TodoTabContent.svelte';
  import GitTabContent from './lib/GitTabContent.svelte';
  import AgentsTabContent from './lib/AgentsTabContent.svelte';
  import FsTabContent from './lib/FsTabContent.svelte';
  import McpTabContent from './lib/McpTabContent.svelte';
  import { signalBusReady, type Category } from './lib/bus';
  import { parseSeverity, type SeverityLevel } from './lib/notifFilter';

  // On-demand window: built fresh each time a tab is detached.
  // signalBusReady at module scope is safe — see CockpitDetached:28-31.
  signalBusReady();

  interface NotifConfig {
    tabId: string;
    category: string;
    title: string;
    icon: string;
    severityThreshold: string;
  }

  let appWindow: TauriWindow;
  let config = $state<NotifConfig | null>(null);

  let severityThreshold = $derived<SeverityLevel>(
    config ? parseSeverity(config.severityThreshold) : 'info',
  );

  function posKey(tabId: string): string {
    return `rift.notif.detached_pos.${tabId}`;
  }

  interface SavedPos {
    x: number;
    y: number;
    width: number;
    height: number;
  }

  function savePosition(x: number, y: number, width: number, height: number): void {
    if (!config) return;
    try {
      localStorage.setItem(posKey(config.tabId), JSON.stringify({ x, y, width, height }));
    } catch {
      // non-fatal
    }
  }

  async function restoreSavedPosition(): Promise<void> {
    if (!config) return;
    let raw: string | null = null;
    try {
      raw = localStorage.getItem(posKey(config.tabId));
    } catch {
      return;
    }
    if (!raw) return;

    let pos: SavedPos;
    try {
      pos = JSON.parse(raw) as SavedPos;
    } catch {
      try { localStorage.removeItem(posKey(config.tabId)); } catch { /* ignore */ }
      return;
    }

    if (
      typeof pos.x !== 'number' ||
      typeof pos.y !== 'number' ||
      typeof pos.width !== 'number' ||
      typeof pos.height !== 'number'
    ) {
      try { localStorage.removeItem(posKey(config.tabId)); } catch { /* ignore */ }
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
        try { localStorage.removeItem(posKey(config.tabId)); } catch { /* ignore */ }
        return;
      }
      await appWindow.setPosition(new PhysicalPosition(pos.x, pos.y));
      await appWindow.setSize(new PhysicalSize(pos.width, pos.height));
    } catch {
      // monitor gone — non-fatal
    }
  }

  // Position tracking with proper cleanup for pool reuse.
  // Unlike CockpitDetached (single window, never reconfigured), pool
  // windows get notif_configure on each detach cycle. Previous listeners
  // must be cleaned up before registering new ones.
  let unlistenMoved: (() => void) | undefined;
  let unlistenResized: (() => void) | undefined;

  function stopPositionTracking(): void {
    unlistenMoved?.();
    unlistenResized?.();
    unlistenMoved = undefined;
    unlistenResized = undefined;
  }

  async function startPositionTracking(): Promise<void> {
    stopPositionTracking();

    try {
      const [p, s] = await Promise.all([
        appWindow.outerPosition(),
        appWindow.outerSize(),
      ]);
      savePosition(p.x, p.y, s.width, s.height);
    } catch { /* non-fatal */ }

    unlistenMoved = await appWindow.onMoved(({ payload: pos }) => {
      appWindow.outerSize().then((size) => {
        savePosition(pos.x, pos.y, size.width, size.height);
      }).catch(() => {});
    });

    unlistenResized = await appWindow.onResized(({ payload: size }) => {
      appWindow.outerPosition().then((pos) => {
        savePosition(pos.x, pos.y, size.width, size.height);
      }).catch(() => {});
    });
  }

  function onTitlebarMouseDown(e: MouseEvent): void {
    if ((e.target as HTMLElement).closest('button')) return;
    appWindow?.startDragging().catch(() => {});
  }

  async function dock(): Promise<void> {
    if (!config) return;
    try {
      await invoke('notif_dock', { tabId: config.tabId });
    } catch (err) {
      console.error('[NotifDetached] notif_dock failed:', err);
    }
  }

  function applyConfig(cfg: NotifConfig): void {
    config = cfg;
    appWindow.setTitle(`Rift — ${cfg.title}`).catch(() => {});
    void restoreSavedPosition().then(() => startPositionTracking()).catch(() => startPositionTracking());
  }

  onMount(() => {
    try {
      appWindow = getCurrentWindow();
    } catch (err) {
      console.error('[NotifDetached] getCurrentWindow() failed:', err);
      return;
    }

    // Pull config from backend — the window is built on-demand so the
    // config is always stored before the window exists. No event race.
    void (async () => {
      const label = appWindow.label;
      try {
        const pending = await invoke<NotifConfig | null>('notif_get_config', { label });
        if (pending) applyConfig(pending);
      } catch (err) {
        console.error('[NotifDetached] notif_get_config failed:', err);
      }
    })();
  });
</script>

<div class="detached-shell" data-tauri-drag-region>
  <header class="titlebar" role="toolbar" tabindex={-1} data-tauri-drag-region onmousedown={onTitlebarMouseDown}>
    <span class="brand">
      <span class="glyph">◆</span>RIFT
      {#if config}
        <span class="sub">{config.title.toUpperCase()}</span>
      {/if}
    </span>
    <span class="spacer" data-tauri-drag-region></span>
    <div class="controls">
      <button type="button" class="btn dock" aria-label="dock notification tab" onclick={dock}>
        ↙ DOCK
      </button>
      <button type="button" class="btn close" aria-label="close" onclick={dock}>×</button>
    </div>
  </header>

  <div class="content">
    {#if config}
      {#key config.tabId}
        {#if config.tabId === 'aegis'}
          <AegisTabContent {severityThreshold} />
        {:else if config.tabId === 'index'}
          <IndexTabContent />
        {:else if config.tabId === 'bustail'}
          <BusTailTabContent {severityThreshold} />
        {:else if config.tabId === 'todo'}
          <TodoTabContent />
        {:else if config.tabId === 'git'}
          <GitTabContent />
        {:else if config.tabId === 'agents'}
          <AgentsTabContent {severityThreshold} />
        {:else if config.tabId === 'filesystem'}
          <FsTabContent {severityThreshold} />
        {:else if config.tabId === 'mcp'}
          <McpTabContent {severityThreshold} />
        {:else}
          <NotificationPane
            title={config.title}
            icon={config.icon}
            categoryFilter={config.category as Category}
            {severityThreshold}
          />
        {/if}
      {/key}
    {:else}
      <div class="waiting">
        <span class="waiting-icon">◇</span>
        <span class="waiting-label">AWAITING TAB</span>
      </div>
    {/if}
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

  .titlebar {
    height: 32px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    padding: 0 12px;
    user-select: none;
    flex-shrink: 0;
  }

  .brand {
    color: var(--amber-primary);
    font-weight: 700;
    font-size: 12px;
    letter-spacing: 0.15em;
    text-shadow: var(--glow-amber);
  }

  .glyph {
    color: var(--amber-bright);
    margin-right: 6px;
  }

  .sub {
    color: var(--amber-dim);
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0.12em;
    margin-left: 4px;
  }

  .spacer {
    flex: 1;
    height: 100%;
  }

  .controls {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .btn {
    height: 14px;
    background: transparent;
    border: 1px solid var(--amber-dim);
    color: var(--amber-dim);
    font-size: 9px;
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
    font-size: 10px;
  }

  .btn.close:hover {
    color: var(--term-red);
    border-color: var(--term-red);
  }

  .btn.dock:hover {
    color: var(--blue-claude, #6CB6FF);
    border-color: var(--blue-claude, #6CB6FF);
  }

  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  .waiting {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--amber-faint);
    user-select: none;
  }

  .waiting-icon {
    font-size: 32px;
    opacity: 0.4;
  }

  .waiting-label {
    font-size: 10px;
    letter-spacing: 0.2em;
    opacity: 0.5;
  }
</style>
