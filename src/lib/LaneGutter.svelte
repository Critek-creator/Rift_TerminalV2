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

  /** §10.1 lane palette — hex values kept in lockstep with styles.css
   *  :root tokens and laneFormat.ts RGB triples. */
  const LANE_COLORS: Record<string, string> = {
    SYS: '#A87830',
    USER: '#E8E4D8',
    CLAUDE: '#6CB6FF',
    AGENT: '#C58FFF',
    HOOK: '#6FE0E0',
    AEGIS: '#FFA826',
    OK: '#4FE855',
    ERR: '#FF4848',
  };

  const FALLBACK_COLOR = '#A87830'; // SYS as fallback

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
    transition: background-color 150ms ease-out, box-shadow 150ms ease-out;
  }
</style>
