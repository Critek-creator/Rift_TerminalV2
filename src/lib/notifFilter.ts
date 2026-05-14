// Notification severity filtering — derives severity from Envelope.kind
// strings and gates events against a per-tab threshold.

export type SeverityLevel = 'debug' | 'info' | 'warn' | 'error';

const SEVERITY_RANK: Record<SeverityLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
};

export function severityRank(level: SeverityLevel): number {
  return SEVERITY_RANK[level] ?? 1;
}

export function kindToSeverity(kind: string): SeverityLevel {
  const k = kind.toLowerCase();
  if (k.includes('error') || k.includes('failed') || k.includes('panic')) return 'error';
  if (k.includes('warn')) return 'warn';
  if (k.includes('debug') || k.includes('trace')) return 'debug';
  return 'info';
}

export function shouldShow(kind: string, threshold: SeverityLevel): boolean {
  return severityRank(kindToSeverity(kind)) >= severityRank(threshold);
}

export function parseSeverity(s: string | undefined | null): SeverityLevel {
  if (s === 'debug' || s === 'info' || s === 'warn' || s === 'error') return s;
  return 'info';
}
