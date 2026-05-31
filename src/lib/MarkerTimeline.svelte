<script lang="ts">
  // Candidate 49d — terminal session markers. Horizontal scrubber rendered
  // above the replay event log in SessionsTabContent. Each marker (a
  // `status`/`session.marker` envelope dropped via Ctrl+Shift+K during the
  // live session) shows as a clickable amber pip positioned by its timestamp
  // between the session's first and last event. Clicking seeks the event log
  // to that marker's row. Metadata-only — never touches PTY or terminal state.

  interface SessionMarker {
    /** Index of the marker event within the loaded session's event list. */
    idx: number;
    /** Unix epoch ms of the marker. */
    ts: number;
    /** Human label, e.g. "Mark @ 14:32:07". */
    label: string;
    /** Optional note (post-hoc annotation; null at creation). */
    note: string | null;
  }

  interface Props {
    markers: SessionMarker[];
    /** Timestamp of the session's first event (left edge). */
    startTs: number;
    /** Timestamp of the session's last event (right edge). */
    endTs: number;
    /** Called with the marker's event index when a pip is activated. */
    onSeek: (idx: number) => void;
  }

  let { markers, startTs, endTs, onSeek }: Props = $props();

  /** Left-offset percentage for a marker, clamped to [0, 100]. Guards the
   *  degenerate single-event session (startTs === endTs) by pinning to 0. */
  function offsetPct(ts: number): number {
    const span = endTs - startTs;
    if (span <= 0) return 0;
    const pct = ((ts - startTs) / span) * 100;
    return Math.min(100, Math.max(0, pct));
  }

  function clockOf(ts: number): string {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
  }
</script>

<div class="marker-timeline" role="group" aria-label="Session markers — {markers.length}">
  <span class="ml-label">MARKERS</span>
  <div class="ml-track">
    <div class="ml-rail"></div>
    {#each markers as m (m.idx)}
      <button
        type="button"
        class="ml-pip"
        style="left: {offsetPct(m.ts)}%"
        title="{m.label}{m.note ? ` — ${m.note}` : ''} ({clockOf(m.ts)})"
        aria-label="Jump to {m.label} at {clockOf(m.ts)}"
        onclick={() => onSeek(m.idx)}
      ></button>
    {/each}
  </div>
  <span class="ml-count">{markers.length}</span>
</div>

<style>
  .marker-timeline {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    background: linear-gradient(to bottom, rgba(255, 200, 64, 0.05), transparent);
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .ml-label {
    color: var(--amber-bright);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    text-shadow: var(--glow-amber-faint);
    flex-shrink: 0;
  }
  .ml-track {
    position: relative;
    flex: 1;
    height: 16px;
    min-width: 0;
  }
  .ml-rail {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    background: var(--amber-faint);
    opacity: 0.5;
  }
  .ml-pip {
    position: absolute;
    top: 50%;
    width: 9px;
    height: 9px;
    margin-left: -4px;
    transform: translateY(-50%) rotate(45deg);
    background: var(--amber-bright);
    border: 1px solid var(--amber-bright);
    box-shadow: 0 0 5px var(--glow-amber, rgba(255, 200, 64, 0.5));
    cursor: pointer;
    padding: 0;
    transition: background var(--duration-base) ease-out, box-shadow var(--duration-base) ease-out, transform var(--duration-base) ease-out;
  }
  .ml-pip:hover {
    background: var(--term-white);
    transform: translateY(-50%) rotate(45deg) scale(1.3);
    box-shadow: 0 0 8px var(--amber-bright);
    z-index: 2;
  }
  .ml-pip:focus-visible {
    outline: 1px solid var(--term-white);
    outline-offset: 2px;
    z-index: 2;
  }
  .ml-count {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
</style>
