<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { McpCallSpan } from './McpWaterfallStore';
  import { tierCssVar, TIMEOUT_WARN_MS, TIMEOUT_MS } from './McpWaterfallStore';

  interface Props {
    spans: McpCallSpan[];
  }

  let { spans }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let wrapEl: HTMLDivElement | undefined = $state();
  let canvasWidth = $state(600);
  let canvasHeight = $state(300);
  let hovered = $state<McpCallSpan | null>(null);
  let tooltipX = $state(0);
  let tooltipY = $state(0);
  let paused = $state(false);
  let selectedSpan = $state<McpCallSpan | null>(null);
  let animFrame = 0;

  /** Resolved CSS colors — read once on mount from computed style. */
  let colors = {
    read: '#6CB6FF',
    mutate: '#FF4848',
    inspect: '#6FE0E0',
    pending: '#FFC840',
    bg: '#080806',
    grid: '#2a2418',
    text: '#D8A028',
    textFaint: '#A87830',
    warn: '#FF4848',
    surface: '#0F0F0D',
  };

  /** Layout constants. */
  const ROW_H = 22;
  const ROW_GAP = 2;
  const LABEL_W = 120;
  const HEADER_H = 24;
  const MIN_BAR_W = 4;
  const RIGHT_PAD = 16;

  /** Time window in ms shown on screen. */
  const WINDOW_MS = 30_000;

  /** Resolve CSS custom properties to actual color values. */
  function resolveColors(): void {
    if (!canvasEl) return;
    const cs = getComputedStyle(canvasEl);
    const get = (v: string, fb: string) => cs.getPropertyValue(v).trim() || fb;
    colors = {
      read:      get('--term-blue', '#6CB6FF'),
      mutate:    get('--term-red', '#FF4848'),
      inspect:   get('--term-cyan', '#6FE0E0'),
      pending:   get('--amber-bright', '#FFC840'),
      bg:        get('--bg-base', '#080806'),
      grid:      get('--border-subtle', '#2a2418'),
      text:      get('--amber-dim', '#D8A028'),
      textFaint: get('--amber-faint', '#A87830'),
      warn:      get('--term-red', '#FF4848'),
      surface:   get('--bg-surface', '#0F0F0D'),
    };
  }

  function tierColor(tier: string): string {
    switch (tier) {
      case 'read': return colors.read;
      case 'mutate': return colors.mutate;
      case 'inspect': return colors.inspect;
      default: return colors.text;
    }
  }

  /** Build row assignments — concurrent spans get different rows. */
  function assignRows(visibleSpans: McpCallSpan[]): Map<string, number> {
    const rowMap = new Map<string, number>();
    /** Track end times for each row to pack bars tightly. */
    const rowEnds: number[] = [];

    for (const span of visibleSpans) {
      const start = span.requestTime;
      let assigned = -1;
      for (let r = 0; r < rowEnds.length; r++) {
        if (rowEnds[r] <= start) {
          assigned = r;
          break;
        }
      }
      if (assigned === -1) {
        assigned = rowEnds.length;
        rowEnds.push(0);
      }
      const end = span.responseTime ?? (Date.now());
      rowEnds[assigned] = end + 100; // small gap between bars
      rowMap.set(span.id, assigned);
    }
    return rowMap;
  }

  /** Filter spans visible within the current time window. */
  function getVisibleSpans(now: number): McpCallSpan[] {
    const windowStart = now - WINDOW_MS;
    return spans.filter((s) => {
      const end = s.responseTime ?? now;
      return end >= windowStart && s.requestTime <= now;
    });
  }

  /** Format ms duration for display. */
  function fmtDuration(ms: number | null): string {
    if (ms === null) return 'pending';
    if (ms < 1) return '<1ms';
    if (ms < 1000) return `${Math.round(ms)}ms`;
    return `${(ms / 1000).toFixed(2)}s`;
  }

  /** Format a timestamp for the time axis. */
  function fmtAxisTime(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  /** Main render loop. */
  function draw(): void {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = canvasWidth;
    const h = canvasHeight;
    canvasEl.width = w * dpr;
    canvasEl.height = h * dpr;
    ctx.scale(dpr, dpr);

    const now = paused ? (hovered?.requestTime ?? Date.now()) : Date.now();
    const windowStart = now - WINDOW_MS;
    const timeW = w - LABEL_W - RIGHT_PAD;

    // Background
    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, w, h);

    // Get visible spans and assign rows
    const visible = getVisibleSpans(now);
    const rowMap = assignRows(visible);

    // Time axis header
    ctx.fillStyle = colors.surface;
    ctx.fillRect(0, 0, w, HEADER_H);
    ctx.strokeStyle = colors.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, HEADER_H);
    ctx.lineTo(w, HEADER_H);
    ctx.stroke();

    // Time axis labels — every 5 seconds
    ctx.fillStyle = colors.textFaint;
    ctx.font = '9px JetBrains Mono, monospace';
    ctx.textAlign = 'center';
    const stepMs = 5000;
    const firstTick = Math.ceil(windowStart / stepMs) * stepMs;
    for (let t = firstTick; t <= now; t += stepMs) {
      const x = LABEL_W + ((t - windowStart) / WINDOW_MS) * timeW;
      ctx.fillText(fmtAxisTime(t), x, HEADER_H - 6);
      // Vertical grid line
      ctx.strokeStyle = colors.grid;
      ctx.globalAlpha = 0.4;
      ctx.beginPath();
      ctx.moveTo(x, HEADER_H);
      ctx.lineTo(x, h);
      ctx.stroke();
      ctx.globalAlpha = 1.0;
    }

    // Label column header
    ctx.fillStyle = colors.textFaint;
    ctx.font = '9px JetBrains Mono, monospace';
    ctx.textAlign = 'left';
    ctx.fillText('TOOL', 8, HEADER_H - 6);

    // Separator between label col and timeline
    ctx.strokeStyle = colors.grid;
    ctx.beginPath();
    ctx.moveTo(LABEL_W, 0);
    ctx.lineTo(LABEL_W, h);
    ctx.stroke();

    // Draw spans
    for (const span of visible) {
      const row = rowMap.get(span.id) ?? 0;
      const y = HEADER_H + 4 + row * (ROW_H + ROW_GAP);
      if (y + ROW_H > h) continue; // clip if overflows

      const spanEnd = span.responseTime ?? now;
      const startX = LABEL_W + ((span.requestTime - windowStart) / WINDOW_MS) * timeW;
      const endX = LABEL_W + ((spanEnd - windowStart) / WINDOW_MS) * timeW;
      const barW = Math.max(MIN_BAR_W, endX - startX);
      const barX = Math.max(LABEL_W, startX);

      // Tool label (in the label column, clipped)
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, y, LABEL_W - 4, ROW_H);
      ctx.clip();
      ctx.fillStyle = tierColor(span.tier);
      ctx.font = '10px JetBrains Mono, monospace';
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(span.tool, 8, y + ROW_H / 2);
      ctx.restore();

      // Bar
      const base = tierColor(span.tier);
      if (span.status === 'pending') {
        // Pending bar — pulsing opacity
        const elapsed = now - span.requestTime;
        const pulse = 0.5 + 0.3 * Math.sin(elapsed / 300);
        ctx.globalAlpha = pulse;
        ctx.fillStyle = colors.pending;
        ctx.fillRect(barX, y + 2, barW, ROW_H - 4);

        // Timeout warning gradient
        if (elapsed > TIMEOUT_WARN_MS) {
          const warnProgress = Math.min(1, (elapsed - TIMEOUT_WARN_MS) / (TIMEOUT_MS - TIMEOUT_WARN_MS));
          const grad = ctx.createLinearGradient(barX, 0, barX + barW, 0);
          grad.addColorStop(0, base);
          grad.addColorStop(1, colors.warn);
          ctx.globalAlpha = warnProgress * 0.6;
          ctx.fillStyle = grad;
          ctx.fillRect(barX, y + 2, barW, ROW_H - 4);
        }
        ctx.globalAlpha = 1.0;
      } else {
        // Completed bar
        ctx.globalAlpha = span.status === 'error' ? 0.85 : 0.75;
        ctx.fillStyle = base;
        ctx.fillRect(barX, y + 2, barW, ROW_H - 4);

        // Timeout warning gradient for completed bars that were slow
        if (span.durationMs !== null && span.durationMs > TIMEOUT_WARN_MS) {
          const warnProgress = Math.min(1, (span.durationMs - TIMEOUT_WARN_MS) / (TIMEOUT_MS - TIMEOUT_WARN_MS));
          const grad = ctx.createLinearGradient(barX, 0, barX + barW, 0);
          grad.addColorStop(0, base);
          grad.addColorStop(Math.max(0.5, 1 - warnProgress), base);
          grad.addColorStop(1, colors.warn);
          ctx.globalAlpha = 0.75;
          ctx.fillStyle = grad;
          ctx.fillRect(barX, y + 2, barW, ROW_H - 4);
        }
        ctx.globalAlpha = 1.0;

        // Duration label inside bar if wide enough
        if (barW > 40 && span.durationMs !== null) {
          ctx.fillStyle = '#000';
          ctx.font = '9px JetBrains Mono, monospace';
          ctx.textAlign = 'left';
          ctx.textBaseline = 'middle';
          ctx.fillText(fmtDuration(span.durationMs), barX + 4, y + ROW_H / 2);
        }
      }

      // Error indicator — red left border
      if (span.status === 'error') {
        ctx.fillStyle = colors.warn;
        ctx.fillRect(barX, y + 2, 3, ROW_H - 4);
      }

      // Highlight hovered span
      if (hovered && hovered.id === span.id) {
        ctx.strokeStyle = '#fff';
        ctx.lineWidth = 1.5;
        ctx.strokeRect(barX, y + 2, barW, ROW_H - 4);
      }

      // Highlight selected span
      if (selectedSpan && selectedSpan.id === span.id) {
        ctx.strokeStyle = colors.pending;
        ctx.lineWidth = 2;
        ctx.strokeRect(barX - 1, y + 1, barW + 2, ROW_H - 2);
      }
    }

    // Draw dependency arrows for correlated spans
    drawCorrelationArrows(ctx, visible, rowMap, windowStart, timeW, now);

    // "Now" line
    if (!paused) {
      const nowX = LABEL_W + timeW;
      ctx.strokeStyle = colors.pending;
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(nowX, HEADER_H);
      ctx.lineTo(nowX, h);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    animFrame = requestAnimationFrame(draw);
  }

  /** Draw arrows between correlated spans. */
  function drawCorrelationArrows(
    ctx: CanvasRenderingContext2D,
    visible: McpCallSpan[],
    rowMap: Map<string, number>,
    windowStart: number,
    timeW: number,
    _now: number,
  ): void {
    // Group by correlation
    const groups = new Map<string, McpCallSpan[]>();
    for (const s of visible) {
      if (!s.correlationGroup) continue;
      let arr = groups.get(s.correlationGroup);
      if (!arr) {
        arr = [];
        groups.set(s.correlationGroup, arr);
      }
      arr.push(s);
    }

    ctx.strokeStyle = colors.textFaint;
    ctx.globalAlpha = 0.5;
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 3]);

    for (const chain of groups.values()) {
      if (chain.length < 2) continue;
      // Sort by request time
      chain.sort((a, b) => a.requestTime - b.requestTime);
      for (let i = 1; i < chain.length; i++) {
        const prev = chain[i - 1];
        const curr = chain[i];
        const prevRow = rowMap.get(prev.id) ?? 0;
        const currRow = rowMap.get(curr.id) ?? 0;

        const prevEnd = prev.responseTime ?? prev.requestTime;
        const prevX = LABEL_W + ((prevEnd - windowStart) / WINDOW_MS) * timeW;
        const prevY = HEADER_H + 4 + prevRow * (ROW_H + ROW_GAP) + ROW_H / 2;

        const currX = LABEL_W + ((curr.requestTime - windowStart) / WINDOW_MS) * timeW;
        const currY = HEADER_H + 4 + currRow * (ROW_H + ROW_GAP) + ROW_H / 2;

        ctx.beginPath();
        ctx.moveTo(prevX, prevY);
        ctx.lineTo(currX, currY);
        ctx.stroke();

        // Arrowhead
        const angle = Math.atan2(currY - prevY, currX - prevX);
        const headLen = 5;
        ctx.beginPath();
        ctx.moveTo(currX, currY);
        ctx.lineTo(
          currX - headLen * Math.cos(angle - Math.PI / 6),
          currY - headLen * Math.sin(angle - Math.PI / 6),
        );
        ctx.moveTo(currX, currY);
        ctx.lineTo(
          currX - headLen * Math.cos(angle + Math.PI / 6),
          currY - headLen * Math.sin(angle + Math.PI / 6),
        );
        ctx.stroke();
      }
    }

    ctx.setLineDash([]);
    ctx.globalAlpha = 1.0;
  }

  /** Hit-test: find which span the cursor is over. */
  function hitTest(mx: number, my: number): McpCallSpan | null {
    if (!canvasEl) return null;
    const now = paused ? (hovered?.requestTime ?? Date.now()) : Date.now();
    const windowStart = now - WINDOW_MS;
    const timeW = canvasWidth - LABEL_W - RIGHT_PAD;
    const visible = getVisibleSpans(now);
    const rowMap = assignRows(visible);

    for (const span of visible) {
      const row = rowMap.get(span.id) ?? 0;
      const y = HEADER_H + 4 + row * (ROW_H + ROW_GAP);
      const spanEnd = span.responseTime ?? now;
      const startX = LABEL_W + ((span.requestTime - windowStart) / WINDOW_MS) * timeW;
      const endX = LABEL_W + ((spanEnd - windowStart) / WINDOW_MS) * timeW;
      const barW = Math.max(MIN_BAR_W, endX - startX);
      const barX = Math.max(LABEL_W, startX);

      if (mx >= barX && mx <= barX + barW && my >= y + 2 && my <= y + ROW_H - 2) {
        return span;
      }
    }
    return null;
  }

  function onMouseMove(e: MouseEvent): void {
    if (!canvasEl) return;
    const rect = canvasEl.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const hit = hitTest(mx, my);
    hovered = hit;
    tooltipX = e.clientX;
    tooltipY = e.clientY;
    if (hit) {
      paused = true;
    }
  }

  function onMouseLeave(): void {
    hovered = null;
    paused = false;
  }

  function onClick(e: MouseEvent): void {
    if (!canvasEl) return;
    const rect = canvasEl.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const hit = hitTest(mx, my);
    if (hit && selectedSpan?.id === hit.id) {
      selectedSpan = null;
    } else {
      selectedSpan = hit;
    }
  }

  function formatPayloadDetail(payload: unknown): string {
    if (payload === null || payload === undefined) return '(none)';
    if (typeof payload === 'string') return payload;
    try {
      return JSON.stringify(payload, null, 2);
    } catch {
      return String(payload);
    }
  }

  function statusLabel(status: string): string {
    switch (status) {
      case 'pending': return 'PENDING';
      case 'success': return 'OK';
      case 'error': return 'ERR';
      default: return status.toUpperCase();
    }
  }

  function statusColor(status: string): string {
    switch (status) {
      case 'pending': return 'var(--amber-bright)';
      case 'success': return 'var(--term-green)';
      case 'error': return 'var(--term-red)';
      default: return 'var(--amber-dim)';
    }
  }

  let resizeObserver: ResizeObserver | undefined;

  onMount(() => {
    if (wrapEl) {
      const updateSize = () => {
        if (!wrapEl) return;
        canvasWidth = wrapEl.clientWidth;
        canvasHeight = wrapEl.clientHeight;
      };
      resizeObserver = new ResizeObserver(updateSize);
      resizeObserver.observe(wrapEl);
      updateSize();
    }
    resolveColors();
    animFrame = requestAnimationFrame(draw);
  });

  onDestroy(() => {
    cancelAnimationFrame(animFrame);
    resizeObserver?.disconnect();
  });
</script>

<div class="waterfall-wrap" bind:this={wrapEl}>
  <canvas
    class="waterfall-canvas"
    bind:this={canvasEl}
    onmousemove={onMouseMove}
    onmouseleave={onMouseLeave}
    onclick={onClick}
    style="width: {canvasWidth}px; height: {canvasHeight}px;"
  ></canvas>

  {#if hovered}
    <div
      class="tooltip"
      style="left: {tooltipX + 12}px; top: {tooltipY - 8}px;"
    >
      <div class="tooltip-tool" style="color: var({tierCssVar(hovered.tier)})">{hovered.tool}</div>
      <div class="tooltip-row">
        <span class="tooltip-k">tier</span>
        <span class="tooltip-v" style="color: var({tierCssVar(hovered.tier)})">{hovered.tier}</span>
      </div>
      <div class="tooltip-row">
        <span class="tooltip-k">duration</span>
        <span class="tooltip-v">{fmtDuration(hovered.status === 'pending' ? Date.now() - hovered.requestTime : hovered.durationMs)}</span>
      </div>
      <div class="tooltip-row">
        <span class="tooltip-k">status</span>
        <span class="tooltip-v" style="color: {statusColor(hovered.status)}">{statusLabel(hovered.status)}</span>
      </div>
      {#if hovered.correlationGroup}
        <div class="tooltip-row">
          <span class="tooltip-k">group</span>
          <span class="tooltip-v corr-id">{hovered.correlationGroup.slice(0, 12)}</span>
        </div>
      {/if}
    </div>
  {/if}

  {#if selectedSpan}
    <div class="detail-panel">
      <div class="detail-header">
        <span class="detail-title" style="color: var({tierCssVar(selectedSpan.tier)})">{selectedSpan.tool}</span>
        <span class="detail-status" style="color: {statusColor(selectedSpan.status)}">{statusLabel(selectedSpan.status)}</span>
        <span class="detail-dur">{fmtDuration(selectedSpan.durationMs)}</span>
        <button type="button" class="detail-close" onclick={() => (selectedSpan = null)} aria-label="close detail">x</button>
      </div>
      <div class="detail-body">
        <div class="detail-section">
          <div class="detail-section-title">REQUEST</div>
          <pre class="detail-payload">{formatPayloadDetail(selectedSpan.requestPayload)}</pre>
        </div>
        {#if selectedSpan.responsePayload}
          <div class="detail-section">
            <div class="detail-section-title">RESPONSE</div>
            <pre class="detail-payload">{formatPayloadDetail(selectedSpan.responsePayload)}</pre>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  {#if spans.length === 0}
    <div class="empty-state">
      <span class="empty-state-icon">▦</span>
      <span class="empty-state-text">Waterfall timeline</span>
      <span class="empty-state-hint">MCP tool call spans will appear as horizontal bars on a shared time axis</span>
    </div>
  {/if}
</div>

<style>
  .waterfall-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
    background: var(--bg-base);
  }

  .waterfall-canvas {
    display: block;
    cursor: crosshair;
  }

  .tooltip {
    position: fixed;
    z-index: 100;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    padding: 8px 10px;
    min-width: 160px;
    max-width: 280px;
    box-shadow: var(--shadow-flyout);
    pointer-events: none;
    font-size: var(--text-xs);
  }

  .tooltip-tool {
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.06em;
    margin-bottom: 4px;
    padding-bottom: 4px;
    box-shadow: var(--sep-depth);
  }

  .tooltip-row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-12);
    padding: 1px 0;
  }

  .tooltip-k {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .tooltip-v {
    color: var(--amber-warm);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .corr-id {
    color: var(--term-purple);
    font-size: var(--text-2xs);
    font-family: var(--font-family);
  }

  .detail-panel {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    max-height: 45%;
    background: var(--bg-elevated);
    border-top: 1px solid var(--border-active);
    box-shadow: var(--depth-lift);
    display: flex;
    flex-direction: column;
    z-index: 50;
  }

  .detail-header {
    display: flex;
    align-items: center;
    gap: var(--space-12);
    padding: 6px 14px;
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }

  .detail-title {
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.06em;
  }

  .detail-status {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    padding: 1px 6px;
    border: 1px solid currentColor;
  }

  .detail-dur {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .detail-close {
    margin-left: auto;
    background: none;
    border: 1px solid var(--border-subtle);
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    padding: 1px 6px;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: color var(--duration-base) ease-out, border-color var(--duration-base) ease-out;
  }
  .detail-close:hover {
    color: var(--term-red);
    border-color: var(--term-red);
  }
  .detail-close:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .detail-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 14px 12px;
    display: flex;
    flex-direction: column;
    gap: var(--space-8);
  }

  .detail-section-title {
    color: var(--amber-warm);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    margin-bottom: 4px;
  }

  .detail-payload {
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    padding: 8px 10px;
    font-family: var(--font-family);
    font-size: var(--text-xs);
    color: var(--amber-dim);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 120px;
    overflow-y: auto;
    line-height: 1.5;
  }

  .empty-state {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    pointer-events: none;
  }

</style>
