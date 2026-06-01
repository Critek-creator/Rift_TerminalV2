// enrichmentRegistry.svelte.ts — §9 capability class 3 (data enrichment).
//
// The generic declare-and-attach mechanism, mirroring actionRegistry.svelte.ts
// (class 2). Where the action registry lets any integration declare invocable
// actions, this lets any integration attach metadata to filesystem nodes —
// previously only the Index vault-walker could (it was hard-wired to
// Category::Index). Now any provider speaks:
//
//   enrichment.declare  (integration → Rift)  { provider_id, label, description? }
//   enrichment.attach   (integration → Rift)  { provider_id, entry_id, fs_path, label?, tags, data? }
//   enrichment.revoke   (integration → Rift)  { provider_id, fs_path? }
//
// All ride Category::System (additive kinds — no envelope-version bump, per
// envelope.rs). The load-bearing data lands in enrichmentStore via attach;
// declare populates the provider list (e.g. for the Integration Inspector).

import { subscribe, type Envelope } from './bus';
import { enrichmentStore, type EnrichmentEntry } from './enrichmentStore.svelte';

export interface DeclaredEnrichmentProvider {
  provider_id: string;
  label: string;
  description?: string;
}

let providers = $state<Record<string, DeclaredEnrichmentProvider>>({});

let unsub: (() => Promise<void>) | undefined;
let started = false;

function handle(env: Envelope): void {
  const p = (env.payload ?? {}) as Record<string, unknown>;
  switch (env.kind) {
    case 'enrichment.declare': {
      const provider_id = typeof p.provider_id === 'string' ? p.provider_id : null;
      const label = typeof p.label === 'string' ? p.label : null;
      if (!provider_id || !label) return;
      providers = {
        ...providers,
        [provider_id]: {
          provider_id,
          label,
          description: typeof p.description === 'string' ? p.description : undefined,
        },
      };
      break;
    }
    case 'enrichment.attach': {
      const provider_id = typeof p.provider_id === 'string' ? p.provider_id : null;
      const entry_id = typeof p.entry_id === 'string' ? p.entry_id : null;
      const fs_path = typeof p.fs_path === 'string' ? p.fs_path : null;
      if (!provider_id || !entry_id || !fs_path) return;
      const data = p.data;
      const dataObj = (data && typeof data === 'object' ? data : {}) as Record<string, unknown>;
      const entry: EnrichmentEntry & { fs_path: string } = {
        fs_path,
        provider_id,
        entry_id,
        label: typeof p.label === 'string' ? p.label : undefined,
        tags: Array.isArray(p.tags)
          ? (p.tags.filter((t) => typeof t === 'string') as string[])
          : [],
        data,
        // Surface Index conveniences so existing tooltip code keeps working.
        vault_id: typeof dataObj.vault_id === 'string' ? dataObj.vault_id : undefined,
        vault_kind: typeof dataObj.vault_kind === 'string' ? dataObj.vault_kind : undefined,
      };
      enrichmentStore.ingest(entry);
      break;
    }
    case 'enrichment.revoke': {
      const provider_id = typeof p.provider_id === 'string' ? p.provider_id : null;
      if (!provider_id) return;
      // Honor the wire protocol's optional fs_path: a string scopes the revoke
      // to that one path; absent/null evicts the whole provider (e.g. shutdown).
      const fs_path = typeof p.fs_path === 'string' ? p.fs_path : undefined;
      if (fs_path !== undefined) {
        enrichmentStore.removeByProviderAtPath(provider_id, fs_path);
      } else {
        enrichmentStore.removeByProvider(provider_id);
      }
      break;
    }
    default:
      break;
  }
}

export const enrichmentRegistry = {
  /** Subscribe to the enrichment.* channel. Idempotent. The bus replay buffer
   *  means attaches published before this runs are still delivered. */
  async start(): Promise<void> {
    if (started) return;
    started = true;
    unsub = await subscribe({ category: 'system' }, handle);
  },
  async stop(): Promise<void> {
    started = false;
    await unsub?.();
    unsub = undefined;
  },

  /** Declared enrichment providers, for inspector-style UIs. */
  providers(): DeclaredEnrichmentProvider[] {
    return Object.values(providers);
  },
};
