<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';

  const appWindow = getCurrentWindow();

  function minimize() { appWindow.minimize(); }
  function maximize() { appWindow.toggleMaximize(); }
  function close()    { appWindow.close(); }
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="brand"><span class="glyph">◆</span>RIFT</span>
  <span class="spacer" data-tauri-drag-region></span>
  <div class="controls">
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
  .spacer { flex: 1; height: 100%; }
  .controls { display: flex; gap: 8px; }
  .btn {
    width: 14px;
    height: 14px;
    background: transparent;
    border: 1px solid var(--amber-dim);
    color: var(--amber-dim);
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
</style>
