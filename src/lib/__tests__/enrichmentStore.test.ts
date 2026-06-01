import { describe, it, expect, beforeEach } from 'vitest';
import { EnrichmentStore } from '../enrichmentStore.svelte';

// Unit tests for EnrichmentStore.
// Tests the store's public API in isolation. App.svelte subscription wiring
// is integration territory and is NOT tested here.
//
// The store is now provider-agnostic (§9 class-3 generalization): entries dedup
// on (provider_id, entry_id) at an fs_path. The Index provider dogfoods it with
// provider_id="index", entry_id=<vault_id>, so vault_id/vault_kind remain
// surfaced for the tooltip.
//
// A fresh EnrichmentStore instance is created per test via beforeEach to
// guarantee no cross-test state leakage (avoids touching the singleton).

let store: EnrichmentStore;

beforeEach(() => {
  store = new EnrichmentStore();
});

describe('EnrichmentStore.ingest', () => {
  it('puts entry under fs_path key', () => {
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(1);
    expect(result![0]).toEqual({
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });
  });

  it('with duplicate (provider_id, entry_id)+fs_path replaces entry, no duplication', () => {
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['first'],
    });
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['second'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(1);
    expect(result![0].tags).toEqual(['second']);
  });

  it('stacks entries with different entry_ids at same fs_path', () => {
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift'],
    });
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'pr003',
      vault_id: 'pr003',
      vault_kind: 'practices',
      tags: ['gotchas'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(2);
    const ids = result!.map((e) => e.vault_id);
    expect(ids).toContain('p006');
    expect(ids).toContain('pr003');
  });

  it('lets two different providers coexist at the same fs_path', () => {
    store.ingest({
      fs_path: '/a',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: [],
    });
    store.ingest({
      fs_path: '/a',
      provider_id: 'git',
      entry_id: 'blame',
      label: 'main@a1b2c3',
      tags: ['HEAD'],
    });

    const result = store.get('/a');
    expect(result).toHaveLength(2);
    const providers = result!.map((e) => e.provider_id);
    expect(providers).toContain('index');
    expect(providers).toContain('git');
  });
});

describe('EnrichmentStore.removeByVaultId (Index back-compat)', () => {
  it('drops entries across all fs_paths for the given vault_id', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/b', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });

    store.removeByVaultId('p006');

    // Both paths should return undefined — empty arrays are pruned (key removed).
    expect(store.get('/a')).toBeUndefined();
    expect(store.get('/b')).toBeUndefined();
  });

  it('leaves other vault_ids intact when removing one vault_id', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006',  vault_id: 'p006',  vault_kind: 'project',   tags: ['rift'] });
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'pr003', vault_id: 'pr003', vault_kind: 'practices', tags: ['gotchas'] });

    store.removeByVaultId('p006');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].vault_id).toBe('pr003');
    expect(result![0].tags).toEqual(['gotchas']);
  });
});

describe('EnrichmentStore.removeByProvider', () => {
  it('drops only the named provider, leaving others intact', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });

    store.removeByProvider('git');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].provider_id).toBe('index');
  });
});

describe('EnrichmentStore.removeByProviderAtPath', () => {
  it('removes a provider only at the named path, leaving the same provider elsewhere', () => {
    store.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });
    store.ingest({ fs_path: '/b', provider_id: 'git', entry_id: 'blame', label: 'y', tags: [] });

    store.removeByProviderAtPath('git', '/a');

    expect(store.get('/a')).toBeUndefined();
    expect(store.get('/b')).toHaveLength(1);
  });

  it('leaves other providers at the same path intact', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });

    store.removeByProviderAtPath('git', '/a');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].provider_id).toBe('index');
  });
});

describe('EnrichmentStore reactivity contract', () => {
  it('map identity changes on ingest (Svelte 5 assign-replace)', () => {
    const before = store.map;
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    expect(store.map).not.toBe(before);
  });
});

describe('EnrichmentStore.loaded flag', () => {
  it('defaults to false', () => {
    expect(store.loaded).toBe(false);
  });

  it('can be set to true (App.svelte sets this on walk.complete)', () => {
    store.loaded = true;
    expect(store.loaded).toBe(true);
  });
});
