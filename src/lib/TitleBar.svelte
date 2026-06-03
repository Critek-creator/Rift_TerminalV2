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

  // Visible entry points for the two otherwise keybind-only command surfaces
  // (Ctrl+K palette, Ctrl+? shortcuts). App.svelte owns their open state, so we
  // dispatch window events it listens for — mirrors the rift:show-welcome path.
  function openCommandPalette(): void {
    window.dispatchEvent(new CustomEvent('rift:open-palette'));
  }
  function openShortcuts(): void {
    window.dispatchEvent(new CustomEvent('rift:open-shortcuts'));
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="brand">
    <svg class="logo" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" focusable="false">
      <!-- Diamond crystal outline -->
      <path d="M12 1.5 L22.5 12 L12 22.5 L1.5 12 Z" fill="none"
            stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
      <!-- The rift: a jagged fault line cleaving the crystal -->
      <path d="M12 1.5 L9 10 L13.5 12.5 L9.5 22.5" fill="none"
            stroke="currentColor" stroke-width="1.35" stroke-linecap="round"
            stroke-linejoin="round" />
    </svg>
    <span class="wordmark">RIFT</span>
  </span>
  <span class="spacer" data-tauri-drag-region></span>
  <div class="controls">
    <!-- Command palette + shortcuts — surface the two keybind-only command
         surfaces (Ctrl+K, Ctrl+?) so they're discoverable from the chrome. -->
    <button
      type="button"
      class="btn cmdk"
      aria-label="open command palette"
      onclick={openCommandPalette}
      title="Command palette (Ctrl+K)"
    >
      ⌘ K
    </button>
    <button
      type="button"
      class="btn help"
      aria-label="keyboard shortcuts"
      onclick={openShortcuts}
      title="Keyboard shortcuts (Ctrl+?)"
    >
      ?
    </button>
    <!-- PROJECT button — opens the project-picker popout (Phase 6.7). -->
    <button
      type="button"
      class="btn project"
      aria-label="switch project"
      onclick={openProjectPicker}
      title="Switch project"
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
    height: var(--control-lg);
    /* Matte elevated bar: a thin warm "lit edge" gradient at the top, the
       film-grain texture, then the elevated fill — consistent with the
       cockpit panels so all chrome reads as one matte material. */
    background-color: var(--bg-elevated);
    background-image:
      linear-gradient(to bottom, rgba(255, 200, 64, 0.07) 0%, transparent 55%),
      var(--grain);
    box-shadow: var(--sep-glow);
    display: flex;
    align-items: center;
    padding: 0 var(--space-12);
    user-select: none;
    flex-shrink: 0;
  }

  /* Phase 8.7g.3 — brand + buttons lifted one foreground tier so they
     read as primary surfaces, not subtitles. user feedback this batch. */
  .brand {
    color: var(--amber-bright);
    font-weight: 700;
    font-size: var(--text-base);
    letter-spacing: 0.15em;
    text-shadow: var(--glow-amber-strong);
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
  }
  /* Inline SVG mark — inherits amber via currentColor; drop-shadow mirrors
     the wordmark's text-shadow glow so logo + RIFT read as one lit material. */
  .logo {
    color: var(--amber-bright);
    flex-shrink: 0;
    filter: drop-shadow(0 0 4px rgba(255, 168, 38, 0.55));
  }
  .wordmark {
    line-height: 1;
  }

  .spacer { flex: 1; height: 100%; }

  .controls {
    display: flex;
    gap: var(--space-xs);
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
    font-size: var(--text-xs);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background var(--duration-med) var(--ease-out), color var(--duration-med) var(--ease-out), text-shadow var(--duration-med) var(--ease-out),
                box-shadow var(--duration-med) var(--ease-out);
  }
  .btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  /* ── Action buttons: PROJECT, SETTINGS, DETACH ──────────────────────────── */
  /* Visual separator before the first action button cluster */
  .controls > .btn.cmdk {
    margin-left: var(--space-sm);
  }
  /* Gap between the palette/help cluster and the project/settings cluster */
  .controls > .btn.project {
    margin-left: var(--space-sm);
  }
  /* Thin visual gap before the window-control trio */
  .btn.winctrl-first {
    margin-left: var(--space-md);
  }

  .btn.cmdk,
  .btn.help,
  .btn.project,
  .btn.settings,
  .btn.detach {
    width: auto;
    height: var(--space-24);
    padding: 0 var(--space-md);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    border-radius: var(--radius-md, 4px);
    border: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    box-shadow: inset 0 -1px 0 transparent;
  }
  .btn.cmdk:hover,
  .btn.help:hover,
  .btn.project:hover,
  .btn.settings:hover {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber);
    background: rgba(255, 200, 64, 0.08);
    border-color: var(--border-subtle);
    box-shadow: 0 0 4px rgba(255, 168, 38, 0.1);
  }
  .btn.cmdk:active,
  .btn.help:active,
  .btn.project:active,
  .btn.settings:active {
    background: rgba(255, 200, 64, 0.12);
    border-color: var(--amber-dim);
  }

  .btn.detach:hover {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber);
    background: rgba(255, 200, 64, 0.08);
    border-color: var(--border-subtle);
    box-shadow: 0 0 4px rgba(255, 168, 38, 0.1);
  }
  .btn.detach:active {
    background: rgba(255, 200, 64, 0.12);
    border-color: var(--amber-dim);
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
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
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
    color: var(--term-white);
    background: rgba(255, 72, 72, 0.20);
    text-shadow: none;
  }
  .btn.close:active {
    background: rgba(255, 72, 72, 0.40);
  }
</style>
