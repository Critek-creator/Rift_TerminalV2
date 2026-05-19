<script lang="ts">
  // Per-event row renderer for Category::Index envelopes (§10.1 — Index lane, cyan).
  //
  // Two kinds (Phase 8.1 taxonomy):
  //   vault.update  — cyan-bordered INDEX tag + change_kind + vault_id + path
  //   enrichment    — cyan-bordered INDEX tag + vault_kind + vault_id + fs_path
  //   (all others)  — cyan-bordered INDEX tag + kind + raw payload (future-proof)

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

    if (entry.kind === 'vault.update') {
      const changeKind = (p.change_kind as string | undefined) ?? '?';
      const vaultId = (p.vault_id as string | undefined) ?? '?';
      const path = (p.path as string | undefined) ?? '';
      return `${changeKind} · ${vaultId} · ${truncate(path, MAX_LINE)}`;
    }

    if (entry.kind === 'enrichment') {
      const vaultKind = (p.vault_kind as string | undefined) ?? '?';
      const vaultId = (p.vault_id as string | undefined) ?? '?';
      const fsPath = (p.fs_path as string | undefined) ?? '';
      return `[${vaultKind}] ${vaultId} → ${truncate(fsPath, MAX_LINE)}`;
    }

    // Fallback for any future index.* kinds.
    try {
      return truncate(JSON.stringify(p), MAX_LINE);
    } catch {
      return entry.kind;
    }
  });
</script>

<div class="index-row" data-kind={entry.kind}>
  <span class="ts">{formatTs(entry.ts)}</span>
  <span class="tag">INDEX</span>
  <span class="label">{label}</span>
</div>

<style>
  .index-row {
    display: grid;
    grid-template-columns: 70px 52px 1fr;
    gap: 10px;
    align-items: baseline;
    padding: 2px 0;
    font-family: var(--font-family);
    font-size: 11px;
    line-height: 1.5;
    white-space: nowrap;
  }
  .index-row:hover {
    background: rgba(74, 212, 212, 0.06);
  }

  .ts {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    font-size: 10px;
    flex-shrink: 0;
  }

  /* Cyan-bordered INDEX tag — matches §10.1 cyan lane (#6FE0E0) */
  .tag {
    display: inline-block;
    padding: 0 4px;
    border: 1px solid var(--term-cyan, #6FE0E0);
    color: var(--term-cyan, #6FE0E0);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    line-height: 1.6;
    text-align: center;
    flex-shrink: 0;
  }

  .label {
    color: var(--amber-dim);
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* vault.update rows are slightly brighter — these are the primary signal */
  .index-row[data-kind='vault.update'] .label {
    color: var(--term-cyan, #6FE0E0);
    font-weight: 600;
  }

  /* enrichment rows stay at amber-dim — secondary metadata signal */
  .index-row[data-kind='enrichment'] .label {
    color: var(--amber-warm);
  }
</style>
