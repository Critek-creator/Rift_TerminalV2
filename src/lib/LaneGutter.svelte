<script lang="ts">
  // §10.1 lane gutter — colored left strip indicating the active lane.
  //
  // Strategy: absolutely-positioned <div> overlay on the left edge of the
  // terminal host. 3px wide, full height, pointer-events: none so it never
  // intercepts terminal input. Color changes animate via CSS transition
  // (150ms ease-out).
  //
  // This component does NOT affect xterm's column count or trigger FitAddon
  // recalculation — it sits on top of the terminal canvas purely as a
  // visual indicator.

  import type { Terminal as XTerm } from '@xterm/xterm';

  interface Props {
    /** The xterm instance (used only for lifecycle gating — gutter renders
     *  only when the terminal is mounted). */
    terminal: XTerm | undefined;
    /** The terminal host div — gutter positions relative to this. */
    hostElement: HTMLDivElement | undefined;
    /** Current lane identifier. Must match the Rust Lane::Display strings:
     *  SYS, USER, CLAUDE, AGENT, HOOK, AEGIS, OK, ERR. */
    currentLane: string;
  }

  let { terminal, hostElement, currentLane }: Props = $props();

  const LANE_COLORS: Record<string, string> = {
    SYS: 'var(--amber-faint)',
    USER: 'var(--term-white)',
    CLAUDE: 'var(--term-blue)',
    AGENT: 'var(--term-purple)',
    HOOK: 'var(--term-cyan)',
    AEGIS: 'var(--amber-primary)',
    OK: 'var(--term-green)',
    ERR: 'var(--term-red)',
  };

  const FALLBACK_COLOR = 'var(--amber-faint)';

  let gutterColor = $derived(LANE_COLORS[currentLane] ?? FALLBACK_COLOR);
  let visible = $derived(!!terminal && !!hostElement);
</script>

{#if visible}
  <div
    class="lane-gutter"
    style:background-color={gutterColor}
    style:box-shadow="0 0 4px {gutterColor}"
  ></div>
{/if}

<style>
  .lane-gutter {
    position: absolute;
    top: 0;
    left: 0;
    width: 3px;
    height: 100%;
    pointer-events: none;
    z-index: 10;
    /* box-shadow set inline via style: directive so it transitions
       in sync with background-color (both use the reactive gutterColor). */
    transition: background-color var(--duration-med) var(--ease-out), box-shadow var(--duration-med) var(--ease-out);
  }
</style>
