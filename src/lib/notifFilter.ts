// Notification severity filtering — derives severity from Envelope.kind
// strings and gates events against a per-tab threshold.

import type { SeverityLevel } from './riftConfig';
export type { SeverityLevel };

export const SEVERITY_RANK: Record<SeverityLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
};

function severityRank(level: SeverityLevel): number {
  return SEVERITY_RANK[level] ?? 1;
}

export function kindToSeverity(kind: string): SeverityLevel {
  const k = kind.toLowerCase();
  // Some translators encode severity as a domain noun rather than a literal
  // "error"/"warn" token — e.g. `llm.process.crash`, `sentinel.violation`,
  // `mcp.handshake.deny`. Without matching these, those kinds fell through to
  // `info` and were silently hidden whenever a tab's threshold was raised to
  // warn/error — the opposite of intent. (notif-filter audit 2026-05-31.)
  if (k.includes('error') || k.includes('failed') || k.includes('panic') ||
      k.includes('crash') || k.includes('fatal')) return 'error';
  if (k.includes('warn') || k.includes('violation') ||
      k.includes('deny') || k.includes('denied')) return 'warn';
  if (k.includes('debug') || k.includes('trace')) return 'debug';
  return 'info';
}

export function shouldShow(kind: string, threshold: SeverityLevel): boolean {
  return severityRank(kindToSeverity(kind)) >= severityRank(threshold);
}

// Raise a threshold to at least `warn`. The errors tab is fed by the entire
// `system` bus category, which also carries info-level noise (health.portfolio,
// project.changed, notif.window.state). Flooring its effective threshold at
// `warn` keeps that benign traffic out so only genuine errors/warnings surface
// there — for both the unread badge and the rendered list. (audit 2026-05-31.)
export function floorAtWarn(level: SeverityLevel): SeverityLevel {
  return severityRank(level) >= severityRank('warn') ? level : 'warn';
}

export function parseSeverity(s: string | undefined | null): SeverityLevel {
  if (s === 'debug' || s === 'info' || s === 'warn' || s === 'error') return s;
  return 'info';
}
