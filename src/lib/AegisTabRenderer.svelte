<script lang="ts">
  // Per-event row renderer for aegis.* envelopes (§10.1 — Aegis lane, amber).
  //
  // Two kinds:
  //   aegis.context     — amber-bordered AEGIS tag + "context refreshed: vX.Y.Z, N hooks"
  //   aegis.invocation  — amber-bordered AEGIS tag + trimmed raw_line (≤120 chars)
  //   (all others)      — amber-bordered AEGIS tag + kind + raw payload

  const MAX_LINE = 120;

  interface Entry {
    ts: number;
    category: string;
    kind: string;
    payload: Record<string, unknown>;
  }

  interface Props {
    entry: Entry;
  }

  let { entry }: Props = $props();

  function formatTs(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour12: false });
  }

  function truncate(s: string, max: number): string {
    if (s.length <= max) return s;
    return s.slice(0, max) + '…';
  }

  const label = $derived.by(() => {
    const p = entry.payload ?? {};
    if (entry.kind === 'aegis.context') {
      const ver = (p.skill_version as string | undefined) ?? '?';
      const hooks = (p.hook_count as number | undefined) ?? 0;
      return `context refreshed: v${ver}, ${hooks} hook${hooks === 1 ? '' : 's'}`;
    }
    if (entry.kind === 'aegis.invocation') {
      const raw = (p.raw_line as string | undefined) ?? '';
      return truncate(raw, MAX_LINE);
    }
    // Fallback for any future aegis.* kinds.
    try {
      return truncate(JSON.stringify(p), MAX_LINE);
    } catch {
      return entry.kind;
    }
  });
</script>

<div class="aegis-row" data-kind={entry.kind}>
  <span class="ts">{formatTs(entry.ts)}</span>
  <span class="tag">AEGIS</span>
  <span class="label">{label}</span>
</div>

<style>
  .aegis-row {
    display: grid;
    grid-template-columns: 70px 52px 1fr;
    gap: var(--space-md);
    align-items: baseline;
    padding: 2px 0;
    font-family: var(--font-family);
    font-size: var(--text-sm);
    line-height: 1.5;
    white-space: nowrap;
  }
  .aegis-row:hover {
    background: rgba(212, 137, 10, 0.06);
  }

  .ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
    flex-shrink: 0;
  }

  /* Amber bordered AEGIS tag — matches §10.1 amber-primary lane */
  .tag {
    display: inline-block;
    padding: 0 4px;
    border: 1px solid var(--amber-primary, #FFA826);
    color: var(--amber-primary, #FFA826);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    line-height: 1.6;
    text-align: center;
    flex-shrink: 0;
  }

  .label {
    color: var(--amber-dim);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Invocation lines are slightly brighter to distinguish from rare context rows */
  .aegis-row[data-kind='aegis.invocation'] .label {
    color: var(--amber-warm);
  }

  .aegis-row[data-kind='aegis.context'] .label {
    color: var(--amber-primary, #FFA826);
    font-weight: 600;
  }
</style>
