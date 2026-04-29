<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { popouts } from './popouts.svelte';

  const appWindow = getCurrentWindow();

  function minimize() { appWindow.minimize(); }
  function maximize() { appWindow.toggleMaximize(); }
  function close()    { appWindow.close(); }

  // Whether the cockpit window is currently detached.
  // Polled once on mount (design E: reload recovery), then kept current via events.
  let detached = $state(false);

  onMount(() => {
    // Svelte 5's onMount expects a sync callback whose optional return is the
    // cleanup. Async init runs in a self-invoked IIFE; the cleanup closure
    // captures the unlisten handles via mutable refs that the IIFE populates.
    let unlistenDetached: (() => void) | undefined;
    let unlistenReattached: (() => void) | undefined;

    void (async () => {
      try {
        detached = await invoke<boolean>('cockpit_status');
      } catch (err) {
        console.warn('[TitleBar] cockpit_status failed:', err);
      }

      // Keep state current via Tauri events emitted by cockpit_window.rs.
      unlistenDetached = await listen('cockpit_detached', () => {
        detached = true;
      });
      unlistenReattached = await listen('cockpit_reattached', () => {
        detached = false;
      });
    })();

    return () => {
      unlistenDetached?.();
      unlistenReattached?.();
    };
  });

  async function detachGui(): Promise<void> {
    try {
      await invoke('cockpit_detach');
    } catch (err) {
      console.error('[TitleBar] cockpit_detach failed:', err);
    }
  }

  async function reattachGui(): Promise<void> {
    try {
      await invoke('cockpit_reattach');
    } catch (err) {
      console.error('[TitleBar] cockpit_reattach failed:', err);
    }
  }

  function openProjectPicker(): void {
    popouts.summon({
      content: { kind: 'project-picker' },
      width: 'min(640px, 80vw)',
    });
  }

  function openSettings(): void {
    popouts.summon({
      content: { kind: 'settings' },
      width: 'min(680px, 86vw)',
    });
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="brand"><span class="glyph">◆</span>RIFT</span>
  <span class="spacer" data-tauri-drag-region></span>
  <div class="controls">
    <!-- PROJECT button — opens the project-picker popout (Phase 6.7). -->
    <button
      type="button"
      class="btn project"
      aria-label="switch project"
      onclick={openProjectPicker}
      title="switch project (Ctrl+P later — Phase 6.x)"
    >
      ▦ PROJECT
    </button>
    <!-- SETTINGS button — Phase 8.7l. About / Updates (manual check) /
         Project / Filesystem / Index / Notifications. -->
    <button
      type="button"
      class="btn settings"
      aria-label="settings"
      onclick={openSettings}
      title="settings"
    >
      ⚙ SETTINGS
    </button>
    <!-- DETACH/DOCK GUI chip (mockup line 656). Phase 8.7d: while detached
         the cockpit-right panel is hidden entirely, so the only main-window
         path back is this button. Swap label + handler instead of locking
         the user out. -->
    {#if !detached}
      <button type="button" class="btn detach" aria-label="detach cockpit to second window" onclick={detachGui}>
        ↗ DETACH GUI
      </button>
    {:else}
      <button type="button" class="btn detach detach--active" aria-label="dock cockpit back to main window" onclick={reattachGui}>
        ↙ DOCK GUI
      </button>
    {/if}
    <button type="button" class="btn" aria-label="minimize" onclick={minimize}>−</button>
    <button type="button" class="btn" aria-label="maximize" onclick={maximize}>▢</button>
    <button type="button" class="btn close" aria-label="close" onclick={close}>×</button>
  </div>
</header>

<style>
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
  /* Phase 8.7g.3 — brand + buttons lifted one foreground tier so they
     read as primary surfaces, not subtitles. user feedback this batch. */
  .brand {
    color: var(--amber-bright);              /* was amber-primary */
    font-weight: 700;
    font-size: 12px;
    letter-spacing: 0.15em;
    text-shadow: var(--glow-amber-strong);   /* stronger glow */
  }
  .glyph {
    color: var(--amber-bright);
    margin-right: 6px;
    text-shadow: var(--glow-amber-strong);
  }
  .spacer { flex: 1; height: 100%; }
  .controls { display: flex; gap: 8px; align-items: center; }
  .btn {
    width: 14px;
    height: 14px;
    background: transparent;
    border: 1px solid var(--amber-warm);     /* was amber-dim */
    color: var(--amber-warm);                /* was amber-dim */
    font-size: 10px;
    line-height: 12px;
    text-align: center;
    cursor: pointer;
    padding: 0;
    font-family: inherit;
  }
  .btn:hover {
    color: var(--amber-primary);
    border-color: var(--amber-primary);
  }
  .btn.close:hover {
    color: var(--term-red);
    border-color: var(--term-red);
  }
  /* PROJECT button — same shape as DETACH GUI, same border vocabulary */
  .btn.project {
    width: auto;
    padding: 0 6px;
    font-size: 9px;
    letter-spacing: 0.08em;
  }
  .btn.project:hover {
    color: var(--amber-bright);
    border-color: var(--amber-bright);
    text-shadow: var(--glow-amber);
  }
  /* DETACH GUI button — wider than the window controls, same border vocabulary */
  .btn.detach {
    width: auto;
    padding: 0 6px;
    font-size: 9px;
    letter-spacing: 0.08em;
  }
  .btn.detach:hover {
    color: var(--amber-bright);
    border-color: var(--amber-bright);
    text-shadow: var(--glow-amber);
  }
  /* DOCK GUI variant — same as DETACH but uses blue accent like the cockpit's
     local DOCK button so the visual matches across windows (Phase 8.7d). */
  .detach.detach--active {
    color: var(--blue-claude, var(--amber-warm));
    border-color: var(--blue-claude, var(--amber-primary));
  }
  .detach.detach--active:hover {
    text-shadow: 0 0 6px var(--blue-claude, var(--amber-primary));
  }
</style>
