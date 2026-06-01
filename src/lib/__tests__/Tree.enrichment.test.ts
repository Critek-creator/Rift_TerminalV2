import { describe, it, expect } from 'vitest';
import { buildEnrichmentTitle, dotX } from '../enrichmentUtils';
import type { EnrichmentEntry } from '../enrichmentStore.svelte';

// Unit tests for Tree.svelte Phase 8.6.2 enrichment indicator helpers.
//
// Component testing (dot renders only for enriched nodes, ARIA label, <title>
// text) would require @testing-library/svelte, which is NOT installed in this
// repo (vitest-only, jsdom env — see vitest.config.ts comment).
// Unblocking event: when @testing-library/svelte is added as a devDependency,
// replace these unit tests with component render assertions.
//
// Instead, the two pure helper functions in enrichmentUtils.ts are exercised:
//   - buildEnrichmentTitle(entries) → <title> text
//   - dotX(nodeX, isDir, name) → dot x position in SVG coordinates

// ---------------------------------------------------------------------------
// buildEnrichmentTitle
// ---------------------------------------------------------------------------

describe('buildEnrichmentTitle', () => {
  it('formats a single entry with no tags as "vault_id (vault_kind)"', () => {
    const entries: EnrichmentEntry[] = [
      { provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] },
    ];
    expect(buildEnrichmentTitle(entries)).toBe('p006 (project)');
  });

  it('appends tags joined by ", " when tags are non-empty', () => {
    const entries: EnrichmentEntry[] = [
      { provider_id: 'index', entry_id: 'pr003', vault_id: 'pr003', vault_kind: 'practices', tags: ['phase8', 'terminal'] },
    ];
    expect(buildEnrichmentTitle(entries)).toBe('pr003 (practices): phase8, terminal');
  });

  it('joins multiple entries with newline and includes all vault_ids', () => {
    const entries: EnrichmentEntry[] = [
      { provider_id: 'index', entry_id: 'p006',  vault_id: 'p006',  vault_kind: 'project',   tags: [] },
      { provider_id: 'index', entry_id: 'pr003', vault_id: 'pr003', vault_kind: 'practices', tags: ['phase8', 'terminal'] },
    ];
    const result = buildEnrichmentTitle(entries);
    // Both vault_ids must appear.
    expect(result).toContain('p006');
    expect(result).toContain('pr003');
    // Newline separator between entries.
    expect(result).toContain('\n');
    // Full expected string.
    expect(result).toBe('p006 (project)\npr003 (practices): phase8, terminal');
  });

  it('renders a non-index provider with a [provider] qualifier and label fallback', () => {
    const entries: EnrichmentEntry[] = [
      { provider_id: 'git', entry_id: 'blame', label: 'main@a1b2c3', tags: ['HEAD'] },
    ];
    expect(buildEnrichmentTitle(entries)).toBe('main@a1b2c3 [git]: HEAD');
  });
});

// ---------------------------------------------------------------------------
// dotX — verifies dot placement formula mirrors the label-start formula
// ---------------------------------------------------------------------------

describe('dotX', () => {
  // Constants from Tree.svelte (mirrored; keep in sync if layout constants change).
  const FILE_R = 4.5;
  const DIR_W  = 10;

  it('places dot to the right of the label for a file node', () => {
    const nodeX = 16; // ROOT_X
    const name = 'main.rs';
    const x = dotX(nodeX, false, name);
    const labelStartX = nodeX + FILE_R + 6;
    const approxLabelWidth = name.length * 6;
    expect(x).toBe(labelStartX + approxLabelWidth + 4);
  });

  it('places dot to the right of the label for a dir node', () => {
    const nodeX = 38; // ROOT_X + 1 * X_STEP
    const name = 'src';
    const x = dotX(nodeX, true, name);
    const labelStartX = nodeX + DIR_W / 2 + 6;
    const approxLabelWidth = name.length * 6;
    expect(x).toBe(labelStartX + approxLabelWidth + 4);
  });

  it('longer names produce larger x (dot moves further right)', () => {
    const nodeX = 16;
    const short = dotX(nodeX, false, 'a.ts');
    const long  = dotX(nodeX, false, 'a-very-long-filename.ts');
    expect(long).toBeGreaterThan(short);
  });
});
