/**
 * CrossRefHighlight — shared hover state for cross-component highlighting.
 *
 * Bridges IndexGraph (vault browser) ↔ Tree (filesystem tree):
 *   IndexGraph vault-row hover → highlights matching files in Tree (cyan glow)
 *   Tree enrichment-dot hover → highlights matching vaults in IndexGraph
 *
 * Minimal $state store; each component derives its highlight set locally.
 * Follows the enrichmentStore/treeActivity pattern: .svelte.ts for $state,
 * $derived lives in the consuming .svelte component.
 */

class CrossRefHighlightStore {
  /** Set by IndexGraph on vault row mouseenter/mouseleave. */
  hoveredVaultId = $state<string | null>(null);

  /** Set by Tree on enrichment-dot mouseenter/mouseleave. */
  hoveredTreePath = $state<string | null>(null);
}

export const crossRefHighlight = new CrossRefHighlightStore();
