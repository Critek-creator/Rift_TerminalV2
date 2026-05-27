<script lang="ts">
  /**
   * HeatstripTimeline -- canvas-based 60-minute event density heatstrip.
   *
   * Renders one column per bucket (minute). Color intensity maps event count;
   * red tint signals errors. Clicking a segment dispatches a `seek` event
   * with the minute offset so the parent can scroll its event log.
   *
   * Design tokens read at render time via getComputedStyle (same pattern
   * as SparklineChart).
   */

  import { onMount, onDestroy } from 'svelte';
  import type { HeatstripBucket } from './HeatstripBuffer';

  interface Props {
    /** Snapshot of 60 heatstrip buckets (oldest-to-newest). */
    buckets: HeatstripBucket[];
    /** Callback when user clicks a minute segment. Offset 0 = oldest, 59 = now. */
    onseek?: (minuteOffset: number) => void;
  }

  let { buckets, onseek }: Props = $props();

  const HEIGHT = 16;

  let canvasEl: HTMLCanvasElement | undefined = $state(undefined);
  let wrapEl: HTMLDivElement | undefined = $state(undefined);
  let tooltipText = $state('');
  let tooltipX = $state(0);
  let tooltipVisible = $state(false);
  let canvasWidth = $state(200);

  // CSS color cache -- read once per paint from computed styles.
  let colorAmberFaint = '#A87830';
  let colorAmberBright = '#FFC840';
  let colorTermRed = '#FF4848';
  let colorBgBase = '#080806';
  let colorBorderSubtle = '#2a2418';

  function readCssColors(): void {
    if (!canvasEl) return;
    const cs = getComputedStyle(canvasEl);
    colorAmberFaint = cs.getPropertyValue('--amber-faint').trim() || colorAmberFaint;
    colorAmberBright = cs.getPropertyValue('--amber-bright').trim() || colorAmberBright;
    colorTermRed = cs.getPropertyValue('--term-red').trim() || colorTermRed;
    colorBgBase = cs.getPropertyValue('--bg-base').trim() || colorBgBase;
    colorBorderSubtle = cs.getPropertyValue('--border-subtle').trim() || colorBorderSubtle;
  }

  /**
   * Interpolate between two hex colors. t=0 returns `a`, t=1 returns `b`.
   */
  function lerpColor(a: string, b: string, t: number): string {
    const clamp = Math.max(0, Math.min(1, t));
    const parse = (hex: string) => {
      const h = hex.replace('#', '');
      return [
        parseInt(h.substring(0, 2), 16),
        parseInt(h.substring(2, 4), 16),
        parseInt(h.substring(4, 6), 16),
      ];
    };
    const [ar, ag, ab] = parse(a);
    const [br, bg, bb] = parse(b);
    const r = Math.round(ar + (br - ar) * clamp);
    const g = Math.round(ag + (bg - ag) * clamp);
    const bv = Math.round(ab + (bb - ab) * clamp);
    return `rgb(${r},${g},${bv})`;
  }

  function bucketColor(bucket: HeatstripBucket, maxCount: number): string {
    if (bucket.count === 0) return 'transparent';

    // Intensity: how full is this bucket relative to the peak?
    const intensity = maxCount > 0 ? bucket.count / maxCount : 0;

    // Base color ramp: faint amber -> bright amber based on intensity.
    let baseColor = lerpColor(colorAmberFaint, colorAmberBright, intensity);

    // Error tint: blend toward red proportional to error ratio.
    if (bucket.errorCount > 0 && bucket.count > 0) {
      const errorRatio = bucket.errorCount / bucket.count;
      baseColor = lerpColor(baseColor, colorTermRed, errorRatio * 0.7);
    }

    return baseColor;
  }

  function paint(): void {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext('2d');
    if (!ctx) return;

    readCssColors();

    const dpr = window.devicePixelRatio || 1;
    const w = canvasEl.clientWidth;
    const h = HEIGHT;
    canvasWidth = w;

    canvasEl.width = w * dpr;
    canvasEl.height = h * dpr;
    ctx.scale(dpr, dpr);

    // Background
    ctx.fillStyle = colorBgBase;
    ctx.fillRect(0, 0, w, h);

    const bucketCount = buckets.length || 60;
    const colW = w / bucketCount;

    // Find max for normalization.
    let maxCount = 0;
    let totalCount = 0;
    for (const b of buckets) {
      if (b.count > maxCount) maxCount = b.count;
      totalCount += b.count;
    }

    if (totalCount === 0) {
      // Idle state: faint grid lines.
      ctx.strokeStyle = colorBorderSubtle;
      ctx.lineWidth = 0.5;
      const gridStep = w / 6; // 10-minute markers
      for (let i = 1; i < 6; i++) {
        const x = Math.round(gridStep * i) + 0.5;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, h);
        ctx.stroke();
      }

      // "No events" label
      ctx.fillStyle = colorAmberFaint;
      ctx.font = '9px JetBrains Mono, monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('no events (60m)', w / 2, h / 2);
      return;
    }

    // Ensure at least 1 for the normalization denominator.
    if (maxCount < 1) maxCount = 1;

    // Draw each bucket column.
    for (let i = 0; i < bucketCount; i++) {
      const bucket = buckets[i];
      if (!bucket || bucket.count === 0) continue;

      const x = Math.floor(i * colW);
      const nextX = Math.floor((i + 1) * colW);
      const bw = nextX - x;

      const color = bucketColor(bucket, maxCount);
      const intensity = bucket.count / maxCount;

      // Opacity ramp: minimum 0.25 for visibility, max 1.0.
      const alpha = 0.25 + intensity * 0.75;

      ctx.globalAlpha = alpha;
      ctx.fillStyle = color;
      ctx.fillRect(x, 0, bw, h);
    }

    ctx.globalAlpha = 1.0;

    // Subtle 10-minute gridlines on top.
    ctx.strokeStyle = `rgba(42, 36, 24, 0.4)`;
    ctx.lineWidth = 0.5;
    const gridStep = w / 6;
    for (let i = 1; i < 6; i++) {
      const x = Math.round(gridStep * i) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    }
  }

  // Reactive repaint whenever buckets change.
  $effect(() => {
    // Touch `buckets` to create a dependency.
    void buckets;
    paint();
  });

  // Resize observer to handle container width changes.
  let resizeObserver: ResizeObserver | undefined;

  onMount(() => {
    if (wrapEl) {
      resizeObserver = new ResizeObserver(() => {
        paint();
      });
      resizeObserver.observe(wrapEl);
    }
    paint();
  });

  onDestroy(() => {
    resizeObserver?.disconnect();
  });

  function bucketIndexFromX(clientX: number): number {
    if (!canvasEl) return -1;
    const rect = canvasEl.getBoundingClientRect();
    const x = clientX - rect.left;
    const bucketCount = buckets.length || 60;
    const colW = rect.width / bucketCount;
    const idx = Math.floor(x / colW);
    return Math.max(0, Math.min(bucketCount - 1, idx));
  }

  function formatMinuteLabel(minuteOffset: number): string {
    const now = new Date();
    const minutesAgo = (buckets.length - 1) - minuteOffset;
    const target = new Date(now.getTime() - minutesAgo * 60_000);
    return target.toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
  }

  function handleMouseMove(e: MouseEvent): void {
    const idx = bucketIndexFromX(e.clientX);
    if (idx < 0 || idx >= buckets.length) {
      tooltipVisible = false;
      return;
    }
    const bucket = buckets[idx];
    const time = formatMinuteLabel(idx);
    const infoCount = bucket.count - bucket.errorCount;
    const parts: string[] = [];
    if (bucket.errorCount > 0) parts.push(`${bucket.errorCount} error${bucket.errorCount !== 1 ? 's' : ''}`);
    if (infoCount > 0) parts.push(`${infoCount} other`);

    if (bucket.count === 0) {
      tooltipText = `${time} -- idle`;
    } else {
      tooltipText = `${time} -- ${bucket.count} event${bucket.count !== 1 ? 's' : ''} (${parts.join(', ')})`;
    }

    if (canvasEl) {
      const rect = canvasEl.getBoundingClientRect();
      tooltipX = e.clientX - rect.left;
    }
    tooltipVisible = true;
  }

  function handleMouseLeave(): void {
    tooltipVisible = false;
  }

  function handleClick(e: MouseEvent): void {
    if (!onseek) return;
    const idx = bucketIndexFromX(e.clientX);
    if (idx >= 0 && idx < buckets.length) {
      onseek(idx);
    }
  }

  let selectedBucket = $state(-1);

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      selectedBucket = -1;
      tooltipVisible = false;
      return;
    }
    const count = buckets.length || 60;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      if (selectedBucket < 0) {
        selectedBucket = e.key === 'ArrowRight' ? 0 : count - 1;
      } else {
        selectedBucket = e.key === 'ArrowRight'
          ? (selectedBucket + 1) % count
          : (selectedBucket - 1 + count) % count;
      }
      const bucket = buckets[selectedBucket];
      if (bucket) {
        const time = formatMinuteLabel(selectedBucket);
        tooltipText = bucket.count === 0
          ? `${time} -- idle`
          : `${time} -- ${bucket.count} event${bucket.count !== 1 ? 's' : ''}`;
        tooltipX = (selectedBucket + 0.5) * (canvasWidth / count);
        tooltipVisible = true;
      }
      return;
    }
    if (e.key === 'Enter' && selectedBucket >= 0 && onseek) {
      e.preventDefault();
      onseek(selectedBucket);
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -- keyboard-navigable canvas widget -->
<div
  class="heatstrip-wrap"
  bind:this={wrapEl}
  tabindex="0"
  role="img"
  aria-label="Event density heatstrip — arrow keys to navigate buckets"
  onkeydown={handleKeydown}
>
  <canvas
    class="heatstrip-canvas"
    bind:this={canvasEl}
    height={HEIGHT}
    onmousemove={handleMouseMove}
    onmouseleave={handleMouseLeave}
    onclick={handleClick}
  ></canvas>
  {#if tooltipVisible}
    <div
      class="heatstrip-tooltip"
      style="left: {Math.max(60, Math.min(canvasWidth - 60, tooltipX))}px;"
    >
      {tooltipText}
    </div>
  {/if}
</div>

<style>
  .heatstrip-wrap {
    position: relative;
    width: 100%;
    height: var(--space-lg);
    flex-shrink: 0;
    border-radius: var(--radius-md, 4px);
    overflow: hidden;
    border: 1px solid var(--border-subtle, #2a2418);
    background: var(--bg-base, #080806);
    cursor: pointer;
  }
  .heatstrip-wrap:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .heatstrip-canvas {
    display: block;
    width: 100%;
    height: var(--space-lg);
  }

  .heatstrip-tooltip {
    position: absolute;
    top: -26px;
    transform: translateX(-50%);
    background: var(--bg-elevated, #14140F);
    border: 1px solid var(--border-subtle, #2a2418);
    color: var(--amber-warm, #F0A030);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
    padding: 2px var(--space-sm);
    white-space: nowrap;
    pointer-events: none;
    z-index: 10;
    border-radius: var(--radius-sm, 2px);
  }
</style>
