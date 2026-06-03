// sessionTimeline.ts — pure helpers + types for the session-timeline view.
//
// The view (SessionTimeline.svelte) is mostly rendering; the testable logic —
// the entry shape, the per-source visual metadata, and the replay-seek guard —
// lives here so it can be unit-tested without mounting a component.

/** One merged timeline row, mirroring rift-bus `session_reader::TimelineEntry`.
 *  Field names match the Rust serde output exactly. */
export interface TimelineEntry {
  /** Unix ms — primary sort key. */
  ts: number;
  /** 0-based index into the session-log replay vector for deep-linking.
   *  The backend uses u64::MAX as a sentinel for command-history-only rows
   *  that have no position in the replay vector (see {@link isSeekable}). */
  event_idx: number;
  /** Normalized source tag: command | error | agent | hook | fs | llm | mcp. */
  source: string;
  /** Raw envelope category (lowercase, e.g. "pty"). */
  category: string;
  /** Raw envelope kind (e.g. "command.submitted"). */
  kind: string;
  /** One-line human summary (command text, error message, …). */
  summary: string;
  /** PTY pane id from the command.submitted payload — the join key. */
  pane_session_id: number | null;
  /** Enriched from command_history when joinable; null for live-only rows. */
  exit_code: number | null;
  /** Enriched from command_history when joinable; null for live-only rows. */
  duration_ms: number | null;
}

export interface SourceMeta {
  /** Short uppercase lane tag shown on the row. */
  label: string;
  /** CSS color (design-token var with a literal fallback). */
  color: string;
}

/** Per-source visual metadata — mirrors the §10.1 lane palette so the timeline
 *  reads consistently with the terminal lanes. */
export const SOURCE_META: Record<string, SourceMeta> = {
  command: { label: 'CMD', color: 'var(--term-white, #d8d4c8)' },
  error: { label: 'ERR', color: 'var(--term-red, #cc3333)' },
  agent: { label: 'AGENT', color: 'var(--purple-agent, #b078e8)' },
  hook: { label: 'HOOK', color: 'var(--cyan-hook, #4ad4d4)' },
  fs: { label: 'FS', color: 'var(--blue-claude, #4a9eff)' },
  llm: { label: 'LLM', color: 'var(--amber-warm, #f59e0b)' },
  mcp: { label: 'MCP', color: 'var(--amber-dim, #5a4410)' },
};

/** Visual metadata for a source, with a safe fallback for unknown sources so a
 *  future backend source never renders blank. */
export function sourceMeta(source: string): SourceMeta {
  return SOURCE_META[source] ?? { label: (source || '?').toUpperCase().slice(0, 5), color: 'var(--amber-faint)' };
}

/** Whether a timeline row can deep-link into the replay log. The backend uses
 *  u64::MAX for command-history-only rows; that exceeds JS's safe-integer range,
 *  so anything beyond MAX_SAFE_INTEGER (or negative/non-finite) is not seekable. */
export function isSeekable(eventIdx: number): boolean {
  return Number.isFinite(eventIdx) && eventIdx >= 0 && eventIdx <= Number.MAX_SAFE_INTEGER;
}

/** The active (enabled) source keys from a TimelineConfig-shaped object, with
 *  the `show_` prefix stripped — used to label what the timeline is showing. */
export function activeSourceKeys(cfg: Record<string, boolean>): string[] {
  return Object.entries(cfg)
    .filter(([, on]) => on === true)
    .map(([k]) => k.replace(/^show_/, ''));
}
