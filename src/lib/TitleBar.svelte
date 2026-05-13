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
    <button type="button" class="btn winctrl winctrl-first" aria-label="minimize" onclick={minimize}>−</button>
    <button type="button" class="btn winctrl" aria-label="maximize" onclick={maximize}>▢</button>
    <button type="button" class="btn winctrl close" aria-label="close" onclick={close}>×</button>
  </div>
</header>

<style>
  .titlebar {
    height: 32px;
    /* Subtle two-stop gradient: slightly lighter warm tone at the very top fades
       into the flat bg-elevated — gives the bar a thin "lit edge" without flash. */
    background: linear-gradient(
      to bottom,
      color-mix(in srgb, var(--bg-elevated) 85%, var(--amber-dim) 15%) 0%,
      var(--bg-elevated) 55%
    );
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
    color: var(--amber-bright);
    font-weight: 700;
    font-size: 12px;
    letter-spacing: 0.15em;
    text-shadow: var(--glow-amber-strong);
  }
  .glyph {
    color: var(--amber-bright);
    margin-right: 6px;
    text-shadow: var(--glow-amber-strong);
  }

  .spacer { flex: 1; height: 100%; }

  .controls {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  /* ── Base button reset ──────────────────────────────────────────────────── */
  .btn {
    background: transparent;
    border: none;
    color: var(--amber-warm);
    cursor: pointer;
    padding: 0;
    font-family: inherit;
    font-size: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease, text-shadow 0.15s ease,
                box-shadow 0.15s ease;
  }
  .btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  /* ── Action buttons: PROJECT, SETTINGS, DETACH ──────────────────────────── */
  /* Visual separator before the first action button */
  .controls > .btn.project {
    margin-left: 6px;
  }
  /* Thin visual gap before the window-control trio */
  .btn.winctrl-first {
    margin-left: 10px;
  }

  .btn.project,
  .btn.settings,
  .btn.detach {
    width: auto;
    height: 22px;
    padding: 0 8px;
    font-size: 9px;
    letter-spacing: 0.08em;
    border-radius: 2px;
    /* Hairline bottom border at rest — present but near-invisible, becomes
       the amber underline on hover without a layout shift. */
    box-shadow: inset 0 -1px 0 transparent;
  }
  .btn.project:hover,
  .btn.settings:hover {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber);
    background: rgba(255, 200, 64, 0.06);
    box-shadow: inset 0 -1px 0 var(--amber-warm);
  }
  .btn.project:active,
  .btn.settings:active {
    background: rgba(255, 200, 64, 0.10);
    box-shadow: inset 0 -1px 0 var(--amber-bright);
  }

  .btn.detach:hover {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber);
    background: rgba(255, 200, 64, 0.06);
    box-shadow: inset 0 -1px 0 var(--amber-warm);
  }
  .btn.detach:active {
    background: rgba(255, 200, 64, 0.10);
    box-shadow: inset 0 -1px 0 var(--amber-bright);
  }

  /* DOCK GUI variant — blue accent mirrors the cockpit's local DOCK button */
  .detach.detach--active {
    color: var(--blue-claude, var(--amber-warm));
  }
  .detach.detach--active:hover {
    color: var(--blue-claude, var(--amber-bright));
    text-shadow: 0 0 6px var(--blue-claude, var(--amber-primary));
    box-shadow: inset 0 -1px 0 var(--blue-claude, var(--amber-warm));
    background: rgba(74, 158, 255, 0.06);
  }

  /* ── Window controls: minimize, maximize, close ─────────────────────────── */
  /* These three sit tight together with no borders at rest — background
     fills in on hover exactly like VS Code / modern Electron apps. */
  .btn.winctrl {
    width: 18px;
    height: 22px;
    border-radius: 2px;
    font-size: 12px;
    line-height: 1;
    /* No border, no background — invisible at rest */
  }
  .btn.winctrl:hover {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.12);
  }
  .btn.winctrl:active {
    background: rgba(255, 200, 64, 0.20);
  }

  /* Close button gets the standard red-on-hover treatment */
  .btn.close:hover {
    color: #fff;
    background: rgba(255, 72, 72, 0.20);
    text-shadow: none;
  }
  .btn.close:active {
    background: rgba(255, 72, 72, 0.40);
  }
</style>
