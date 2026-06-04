<script lang="ts">
  // BlockActionMenu — N3.3 quick actions for a command block (Warp-style block
  // affordances: copy + bookmark). Opened from a per-command badge; the host
  // (Terminal.svelte) owns the xterm buffer + live markers and does the actual
  // copy, so this component is pure presentation + intent, like TreeContextMenu.
  //
  // A `position: fixed` floating surface clamped to the viewport, dismissed on
  // item click, Escape, outside click, or another right-click.
  import type { CommandBlock } from './commandBlockStore.svelte';

  interface Props {
    block: CommandBlock;
    x: number;
    y: number;
    /** Shown only for a failed command — re-uses the existing explain path. */
    onExplain?: () => void;
    onCopyCommand: () => void;
    onCopyOutput: () => void;
    onCopyBoth: () => void;
    onToggleBookmark: () => void;
    onClose: () => void;
  }
  let {
    block,
    x,
    y,
    onExplain,
    onCopyCommand,
    onCopyOutput,
    onCopyBoth,
    onToggleBookmark,
    onClose,
  }: Props = $props();

  const MENU_W = 224;
  const MENU_H_EST = 200;
  const px = $derived(Math.max(4, Math.min(x, window.innerWidth - MENU_W - 8)));
  const py = $derived(Math.max(4, Math.min(y, window.innerHeight - MENU_H_EST - 8)));

  function run(action: () => void): void {
    action();
    onClose();
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') onClose();
  }}
  onclick={onClose}
  oncontextmenu={onClose}
/>

<div
  class="block-action-menu"
  style="left: {px}px; top: {py}px;"
  role="menu"
  tabindex="-1"
  aria-label="Actions for command: {block.command}"
  onclick={(e) => e.stopPropagation()}
  onkeydown={(e) => e.stopPropagation()}
  oncontextmenu={(e) => {
    e.preventDefault();
    e.stopPropagation();
  }}
>
  <div class="bam-head" title={block.command}>
    <span class="bam-glyph" class:err={block.exitCode !== 0}>
      {block.exitCode === 0 ? '✓' : '✗'} {block.exitCode}
    </span>
    <span class="bam-cmd">{block.command}</span>
  </div>
  <div class="bam-sep" aria-hidden="true"></div>
  {#if onExplain}
    <button class="bam-item" role="menuitem" onclick={() => run(onExplain)}>Explain error</button>
    <div class="bam-sep" aria-hidden="true"></div>
  {/if}
  <button class="bam-item" role="menuitem" onclick={() => run(onCopyCommand)}>Copy command</button>
  <button class="bam-item" role="menuitem" onclick={() => run(onCopyOutput)}>Copy output</button>
  <button class="bam-item" role="menuitem" onclick={() => run(onCopyBoth)}>
    Copy command + output
  </button>
  <div class="bam-sep" aria-hidden="true"></div>
  <button class="bam-item" role="menuitem" onclick={() => run(onToggleBookmark)}>
    {block.bookmarked ? '★ Remove bookmark' : '☆ Bookmark'}
  </button>
</div>

<style>
  .block-action-menu {
    position: fixed;
    z-index: 5000;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elevated, rgba(15, 12, 6, 0.97));
    background-image: var(--grain);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-md);
    box-shadow:
      0 4px 16px rgba(0, 0, 0, 0.55),
      0 0 10px rgba(255, 168, 38, 0.12);
    font-family: var(--font-family);
    display: flex;
    flex-direction: column;
    gap: 1px;
    user-select: none;
  }
  .bam-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px 6px;
    max-width: 320px;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }
  .bam-glyph {
    flex: none;
    font-weight: 700;
    color: var(--term-green);
  }
  .bam-glyph.err {
    color: var(--term-red);
  }
  .bam-cmd {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--amber-faint);
  }
  .bam-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 5px 10px;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--amber-warm, var(--term-white));
    font-family: var(--font-family);
    font-size: var(--text-xs);
    line-height: 1.4;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--duration-fast, 120ms) var(--ease-out, ease);
  }
  .bam-item:hover {
    background: rgba(255, 168, 38, 0.1);
  }
  .bam-item:focus-visible {
    outline: 1px solid var(--amber-bright);
    outline-offset: -1px;
  }
  .bam-sep {
    height: 1px;
    margin: 3px 4px;
    background: var(--amber-faint, rgba(168, 120, 48, 0.3));
  }
</style>
