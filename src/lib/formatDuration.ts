/**
 * Compact millisecond-duration formatter shared across surfaces (terminal
 * command badges, agent run-timeline gantt): `<1s` → `Nms`, `<1min` → `N.Ns`,
 * else `MmSSs`. Negative inputs clamp to 0.
 */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return `${m}m${s.toString().padStart(2, '0')}s`;
}
