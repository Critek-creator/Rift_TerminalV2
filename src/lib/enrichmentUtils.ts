/**
 * enrichmentUtils.ts — Phase 8.6.2 pure helper functions for the enrichment
 * indicator render in Tree.svelte.
 *
 * Extracted as a plain TS module (not .svelte.ts) so they can be imported by
 * vitest tests without requiring @testing-library/svelte. Pure functions only —
 * no rune state, no DOM, no Tauri.
 *
 * Tree.svelte layout constants are mirrored here; keep in sync if they change.
 */

import type { EnrichmentEntry } from './enrichmentStore.svelte';

// Mirrors Tree.svelte layout constants — update both if layout changes.
const FILE_R = 4.5;
const DIR_W  = 10;

/**
 * Build the <title> text for the enrichment dot tooltip.
 * Format per plan v3: one line per enrichment: "<vault_id> (<vault_kind>)"
 * followed by ": <tags>" if non-empty.
 * Example: "p006 (project)\npr003 (practices): phase8, terminal"
 */
export function buildEnrichmentTitle(entries: EnrichmentEntry[]): string {
  return entries
    .map((e) => {
      // Index entries keep the original "<vault_id> (<vault_kind>)" form; other
      // providers fall back to label/entry_id with a "[provider]" qualifier.
      const name = e.label ?? e.vault_id ?? e.entry_id;
      const qualifier = e.vault_kind
        ? `(${e.vault_kind})`
        : e.provider_id !== 'index'
          ? `[${e.provider_id}]`
          : '';
      const base = qualifier ? `${name} ${qualifier}` : name;
      return e.tags.length > 0 ? `${base}: ${e.tags.join(', ')}` : base;
    })
    .join('\n');
}

/**
 * Compute the SVG x coordinate for the enrichment dot, positioned right of
 * the node label.
 * JetBrains Mono at 10px — empirical average character width ≈ 6px.
 * Dot lands at: labelStartX + approxLabelWidth + 4px gap.
 */
export function dotX(nodeX: number, isDir: boolean, name: string): number {
  const labelStartX = nodeX + (isDir ? DIR_W / 2 : FILE_R) + 6;
  const approxLabelWidth = name.length * 6;
  return labelStartX + approxLabelWidth + 4;
}
