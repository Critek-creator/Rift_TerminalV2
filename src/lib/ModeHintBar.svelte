<script lang="ts">
  // ModeHintBar.svelte — Phase 2 / N4 ambient status chrome.
  //
  // A slim strip that sits directly above the StatusLine. Two jobs, split
  // left/right:
  //   LEFT  — zellij-style mode-hint line: the real keybinds for the CURRENT
  //           surface, shown inline so shortcuts stop being undiscoverable
  //           (the "keybinds for important menus" complaint). Pure text, no
  //           interaction — it teaches keys.
  //   RIGHT — intelligent-terminal-style ambient indicators: AGENT / ERR / MCP
  //           lights driven by the existing notif unread state. Lit (with a
  //           count) when there is unacked activity; click to promote that tab
  //           so the state stops being buried in a cockpit tab. This strip is
  //           also the declared home for the Phase 5 error→agent handoff.
  //
  // Sources its data from props (App owns nm.notifs / sm.active) so this stays
  // a pure render layer — no bus subscriptions of its own.

  type ActiveKind = 'session' | 'empty';

  interface Hint {
    keys: string;
    label: string;
  }

  interface Indicator {
    id: string;        // notif tab id to promote on click
    label: string;     // short ambient label
    count: number;     // unread count; >0 = lit
    tone: 'red' | 'amber' | 'cyan';
  }

  interface Props {
    activeKind: ActiveKind;
    cockpitCollapsed: boolean;
    /** Unacked error events (errors tab unreadCount). */
    errorCount: number;
    /** Unacked agent events (agents tab unreadCount). */
    agentCount: number;
    /** Unacked MCP events (mcp tab unreadCount). */
    mcpCount: number;
    /** Promote a notif tab into the side pane (nm.activateNotif). */
    onPromote: (id: string) => void;
  }

  let {
    activeKind,
    cockpitCollapsed,
    errorCount,
    agentCount,
    mcpCount,
    onPromote,
  }: Props = $props();

  // Context-aware hint set. Keep to ~6 so the line reads at a glance and
  // truncates gracefully on a narrow window. Keys mirror keybindings.ts exactly.
  const hints = $derived.by((): Hint[] => {
    if (activeKind === 'empty') {
      return [
        { keys: 'Ctrl+K', label: 'palette' },
        { keys: 'Ctrl+T', label: 'new terminal' },
        { keys: '?', label: 'keys' },
      ];
    }
    // session
    return [
      { keys: 'Ctrl+K', label: 'palette' },
      { keys: 'Ctrl+T', label: 'new' },
      { keys: 'Ctrl+Shift+E/D', label: 'split' },
      { keys: 'Ctrl+Shift+W', label: 'close' },
      { keys: 'Ctrl+B', label: cockpitCollapsed ? 'show cockpit' : 'cockpit' },
      { keys: '?', label: 'keys' },
    ];
  });

  const indicators = $derived.by((): Indicator[] => [
    { id: 'errors', label: 'ERR', count: errorCount, tone: 'red' },
    { id: 'agents', label: 'AGENT', count: agentCount, tone: 'amber' },
    { id: 'mcp', label: 'MCP', count: mcpCount, tone: 'cyan' },
  ]);
</script>

<div class="mode-bar" role="toolbar" aria-label="Mode hints and ambient status">
  <div class="hints" aria-label="Keyboard shortcuts for the current surface">
    {#each hints as h (h.keys + h.label)}
      <span class="hint">
        <span class="hint-keys">{h.keys}</span>
        <span class="hint-label">{h.label}</span>
      </span>
    {/each}
  </div>

  <div class="indicators" aria-label="Ambient activity indicators">
    {#each indicators as ind (ind.id)}
      <button
        type="button"
        class="indicator {ind.tone}"
        class:lit={ind.count > 0}
        onclick={() => onPromote(ind.id)}
        title={ind.count > 0
          ? `${ind.count} unread ${ind.label} event${ind.count === 1 ? '' : 's'} — click to open`
          : `${ind.label} — no recent activity (click to open)`}
        aria-label={`${ind.label}: ${ind.count} unread`}
      >
        <span class="dot"></span>
        <span class="ind-label">{ind.label}</span>
        {#if ind.count > 0}<span class="ind-count">{ind.count > 99 ? '99+' : ind.count}</span>{/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .mode-bar {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    height: 22px;
    padding: 0 var(--space-md);
    background: var(--bg-base);
    border-top: 1px solid var(--border-subtle);
    font-size: var(--text-2xs);
    line-height: 1;
    user-select: none;
    overflow: hidden;
  }

  /* LEFT — mode-hint line */
  .hints {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .hint-keys {
    font-family: var(--font-family);
    font-weight: 700;
    letter-spacing: 0.02em;
    color: var(--amber-warm);
    background: rgba(255, 200, 64, 0.07);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    padding: 1px 5px;
  }
  .hint-label {
    color: var(--amber-faint);
    letter-spacing: 0.03em;
  }

  /* RIGHT — ambient indicators */
  .indicators {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-shrink: 0;
  }
  .indicator {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 1px 7px;
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--amber-faint);
    background: transparent;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition:
      color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out),
      background var(--duration-fast) var(--ease-out);
  }
  .indicator:hover {
    color: var(--amber-warm);
    border-color: var(--amber-dim);
    background: rgba(255, 200, 64, 0.06);
  }
  .indicator:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--border-active);
    flex-shrink: 0;
    transition: background var(--duration-fast) var(--ease-out);
  }
  .ind-count {
    font-variant-numeric: tabular-nums;
    min-width: 1ch;
    text-align: center;
  }

  /* Lit state — the dot glows in the indicator's tone and the chip brightens.
     A soft pulse signals live, unacked activity without being noisy. */
  .indicator.lit { color: var(--text-primary); }
  .indicator.lit .dot { animation: pulse 1.8s var(--ease-out) infinite; }

  .indicator.red.lit  { border-color: rgba(255, 72, 72, 0.5); }
  .indicator.red.lit .dot   { background: var(--term-red); box-shadow: 0 0 6px rgba(255, 72, 72, 0.7); }
  .indicator.red.lit .ind-count { color: var(--term-red-soft); }

  .indicator.amber.lit { border-color: var(--amber-dim); }
  .indicator.amber.lit .dot { background: var(--amber-bright); box-shadow: 0 0 6px rgba(255, 200, 64, 0.6); }
  .indicator.amber.lit .ind-count { color: var(--amber-warm); }

  .indicator.cyan.lit  { border-color: rgba(111, 224, 224, 0.5); }
  .indicator.cyan.lit .dot  { background: var(--term-cyan); box-shadow: 0 0 6px rgba(111, 224, 224, 0.6); }
  .indicator.cyan.lit .ind-count { color: var(--term-cyan); }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }

  @media (prefers-reduced-motion: reduce) {
    .indicator.lit .dot { animation: none; }
  }
</style>
