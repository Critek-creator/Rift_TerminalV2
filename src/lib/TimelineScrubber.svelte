<script lang="ts">
  interface TimelineEvent {
    id: string;
    command: string;
    startTs: number;
    endTs: number;
    lane: string;
    exitCode: number | null;
  }

  interface Props {
    events: TimelineEvent[];
    currentTs: number;
    onSeek: (ts: number) => void;
    viewportMinutes?: number;
  }

  let { events, currentTs, onSeek, viewportMinutes: initialViewport = 5 }: Props = $props();
  let activeViewportMinutes = $state(initialViewport);

  const LANE_COLORS: Record<string, string> = {
    user: '#E8E4D8',
    claude: '#6CB6FF',
    agent: '#C58FFF',
    hook: '#6FE0E0',
    system: '#A87830',
    SYS: '#A87830',
  };

  const ZOOM_LEVELS = [1, 5, 15, 60];

  let svgWidth = $state(600);
  let svgHeight = 48;
  let scrollOffset = $state(0);
  let hoveredEvent = $state<TimelineEvent | null>(null);
  let hoverPos = $state({ x: 0, y: 0 });
  let containerEl: HTMLDivElement | undefined = $state(undefined);

  let viewportMs = $derived(activeViewportMinutes * 60 * 1000);

  let viewportStart = $derived.by(() => {
    if (events.length === 0) return currentTs - viewportMs;
    return currentTs - viewportMs + scrollOffset;
  });

  let viewportEnd = $derived(viewportStart + viewportMs);

  let visibleEvents = $derived(
    events.filter((e) => e.endTs >= viewportStart && e.startTs <= viewportEnd)
  );

  function tsToX(ts: number): number {
    return ((ts - viewportStart) / viewportMs) * svgWidth;
  }

  function durationToWidth(start: number, end: number): number {
    const w = ((end - start) / viewportMs) * svgWidth;
    return Math.max(w, 3);
  }

  function laneColor(lane: string): string {
    return LANE_COLORS[lane] ?? '#A87830';
  }

  function formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    return `${(ms / 60000).toFixed(1)}m`;
  }

  function handleBlockClick(e: MouseEvent, evt: TimelineEvent) {
    e.stopPropagation();
    onSeek(evt.startTs);
  }

  function handleBlockHover(e: MouseEvent, evt: TimelineEvent) {
    hoveredEvent = evt;
    hoverPos = { x: e.clientX, y: e.clientY };
  }

  function handleBlockLeave() {
    hoveredEvent = null;
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    scrollOffset += e.deltaX > 0 ? viewportMs * 0.1 : -viewportMs * 0.1;
  }

  function setZoom(minutes: number) {
    activeViewportMinutes = minutes;
    scrollOffset = 0;
  }

  function scrollLeft() {
    scrollOffset -= viewportMs * 0.25;
  }

  function scrollRight() {
    scrollOffset += viewportMs * 0.25;
  }

  $effect(() => {
    if (!containerEl) return;
    const obs = new ResizeObserver((entries) => {
      for (const entry of entries) {
        svgWidth = entry.contentRect.width;
      }
    });
    obs.observe(containerEl);
    return () => obs.disconnect();
  });

  let currentMarkerX = $derived(tsToX(currentTs));
</script>

<div class="timeline-scrubber" bind:this={containerEl}>
  <div class="timeline-controls">
    <button class="timeline-btn" onclick={scrollLeft} aria-label="Scroll left">◂</button>
    {#each ZOOM_LEVELS as level}
      <button
        class="timeline-btn"
        class:active={activeViewportMinutes === level}
        onclick={() => setZoom(level)}
      >
        {level}m
      </button>
    {/each}
    <button class="timeline-btn" onclick={scrollRight} aria-label="Scroll right">▸</button>
  </div>

  <div class="timeline-track" onwheel={handleWheel}>
    <svg width={svgWidth} height={svgHeight} xmlns="http://www.w3.org/2000/svg">
      {#each visibleEvents as evt (evt.id)}
        {@const x = tsToX(evt.startTs)}
        {@const w = durationToWidth(evt.startTs, evt.endTs)}
        <rect
          {x}
          y={8}
          width={w}
          height={32}
          rx={2}
          fill={laneColor(evt.lane)}
          opacity={0.8}
          class="event-block"
          role="button"
          tabindex="0"
          onclick={(e) => handleBlockClick(e, evt)}
          onmouseenter={(e) => handleBlockHover(e, evt)}
          onmouseleave={handleBlockLeave}
          onkeydown={(e) => { if (e.key === 'Enter') onSeek(evt.startTs); }}
        />
        {#if evt.exitCode !== null && evt.exitCode !== 0}
          <circle
            cx={x + w - 4}
            cy={12}
            r={3}
            fill="var(--term-red)"
          />
        {/if}
      {/each}

      {#if currentMarkerX >= 0 && currentMarkerX <= svgWidth}
        <line
          x1={currentMarkerX}
          y1={0}
          x2={currentMarkerX}
          y2={svgHeight}
          stroke="var(--amber-bright)"
          stroke-width={2}
          opacity={0.9}
        />
      {/if}
    </svg>
  </div>

  {#if hoveredEvent}
    <div
      class="timeline-tooltip"
      style="left: {hoverPos.x}px; top: {hoverPos.y - 60}px;"
    >
      <span class="tooltip-cmd">{hoveredEvent.command.slice(0, 60)}{hoveredEvent.command.length > 60 ? '...' : ''}</span>
      <span class="tooltip-dur">{formatDuration(hoveredEvent.endTs - hoveredEvent.startTs)}</span>
      {#if hoveredEvent.exitCode !== null && hoveredEvent.exitCode !== 0}
        <span class="tooltip-exit">exit {hoveredEvent.exitCode}</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .timeline-scrubber {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-family: var(--font-family), monospace;
    user-select: none;
  }

  .timeline-controls {
    display: flex;
    gap: var(--space-xs);
    align-items: center;
  }

  .timeline-btn {
    padding: 2px 6px;
    font-size: var(--text-xs);
    font-family: inherit;
    background: var(--bg-elevated);
    color: var(--amber-dim);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out),
                border-color var(--duration-fast) var(--ease-out);
  }

  .timeline-btn:hover {
    background: var(--bg-hover);
    border-color: var(--amber-faint);
  }

  .timeline-btn.active {
    background: var(--amber-primary);
    color: var(--bg-base);
    border-color: var(--amber-primary);
    font-weight: 600;
  }

  .timeline-track {
    overflow: hidden;
    border-radius: var(--radius-sm);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
  }

  .timeline-track svg {
    display: block;
  }

  .event-block {
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .event-block:hover {
    opacity: 1 !important;
    filter: brightness(1.2);
  }

  .event-block:focus-visible {
    outline: 2px solid var(--amber-bright);
    outline-offset: 1px;
  }

  .timeline-tooltip {
    position: fixed;
    z-index: 1000;
    background: var(--bg-elevated);
    border: 1px solid var(--amber-dim);
    border-radius: var(--radius-md);
    padding: var(--space-xs) var(--space-sm);
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: 300px;
    pointer-events: none;
    box-shadow: var(--glow-amber-faint);
  }

  .tooltip-cmd {
    font-size: var(--text-xs);
    color: var(--amber-warm, #F0A030);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tooltip-dur {
    font-size: var(--text-2xs, 9px);
    color: var(--amber-faint);
  }

  .tooltip-exit {
    font-size: var(--text-2xs, 9px);
    color: var(--term-red);
    font-weight: 600;
  }
</style>
