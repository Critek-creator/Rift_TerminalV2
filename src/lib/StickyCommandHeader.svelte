<script lang="ts">
  // StickyCommandHeader.svelte — N3.2: the sticky command header (principle N3,
  // "blocks, not scroll"). Presentation-only: it renders whichever CommandBlock
  // Terminal.svelte computes as spanning the viewport top, so when you scroll up
  // into old output you always know which command produced what you're looking
  // at. Terminal owns the xterm wiring (markers + scroll math); this component
  // owns nothing but the look. Null block → nothing rendered (at the live prompt
  // / above the first block). pointer-events:none so it never eats terminal
  // clicks — N3.3/N3.4 add interactivity deliberately.
  import { formatDuration } from './formatDuration';
  import type { CommandBlock } from './commandBlockStore.svelte';

  interface Props {
    block: CommandBlock | null;
  }
  let { block }: Props = $props();
</script>

{#if block}
  {@const ok = block.exitCode === 0}
  <div
    class="sticky-cmd-header"
    role="status"
    aria-live="off"
    title={`${block.command}\nexit ${block.exitCode}${
      block.durationMs != null ? ' · ' + formatDuration(block.durationMs) : ''
    }`}
  >
    {#if block.bookmarked}<span class="sch-star" title="Bookmarked">★</span>{/if}
    <span class="sch-status" class:ok class:err={!ok}>{ok ? '✓' : '✗'} {block.exitCode}</span>
    {#if block.durationMs != null}
      <span class="sch-dur">· {formatDuration(block.durationMs)}</span>
    {/if}
    <span class="sch-cmd">{block.command}</span>
  </div>
{/if}

<style>
  .sticky-cmd-header {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 22px;
    /* clear the 3px lane gutter + roughly align with the 8px host padding */
    padding: 0 12px 0 14px;
    font-family: var(--font-family);
    font-size: 0.78rem;
    line-height: 22px;
    background: linear-gradient(rgba(12, 10, 6, 0.94), rgba(12, 10, 6, 0.88));
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.35);
    pointer-events: none;
    user-select: none;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
  }
  .sch-star {
    flex: none;
    color: var(--amber-bright);
  }
  .sch-status {
    flex: none;
    font-weight: 700;
  }
  .sch-status.ok {
    color: var(--term-green);
  }
  .sch-status.err {
    color: var(--term-red);
  }
  .sch-dur {
    flex: none;
    color: var(--amber-faint);
  }
  .sch-cmd {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--term-white);
  }
</style>
