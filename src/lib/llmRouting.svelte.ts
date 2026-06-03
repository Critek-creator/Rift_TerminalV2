// llmRouting.svelte.ts — Ensemble Router Phase 2 routing state.
//
// Tracks routing decisions, accumulated session cost, and last routing
// metadata. Subscribes to Category::Llm bus events.
//
// Persistence: ledger is written to localStorage (debounced) on every mutation
// and loaded on module init so cost/routing data survives page reload / app
// restart. resetSession() clears both memory and storage. Absent/corrupt
// storage degrades to empty ledger — no throw.

import type { RoutingProfile } from './riftConfig';
import { llmModels } from './llmModels.svelte';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface RoutingEvent {
  model_id: string;
  task_type: string;
  profile: RoutingProfile;
  reason: string;
  was_overridden: boolean;
  timestamp: number;
}

export interface CostEntry {
  model_id: string;
  tokens_in: number;
  tokens_out: number;
  cost_usd: number;
  timestamp: number;
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let sessionCostUsd = $state(0);
let requestCount = $state(0);
let totalTokensIn = $state(0);
let totalTokensOut = $state(0);
let lastRoute = $state<RoutingEvent | null>(null);
let recentRoutes = $state<RoutingEvent[]>([]);
let costByModel = $state<Record<string, number>>({});
let escalationCount = $state(0);

// Grunt-tier savings ledger (candidate 598) — local (grunt) vs cloud routing.
// Local responses cost ~$0; the savings figure is the counterfactual: what the
// same token volume WOULD have cost on the reference cloud model the user is
// configured to use. Cloud spend is the actual reported cost.
let localRequests = $state(0);
let cloudRequests = $state(0);
let localTokensIn = $state(0);
let localTokensOut = $state(0);
let cloudTokensIn = $state(0);
let cloudTokensOut = $state(0);
let cloudSpendUsd = $state(0);
let savingsUsd = $state(0);
let savingsRefModelId = $state<string | null>(null);

const MAX_RECENT = 50;

// ---------------------------------------------------------------------------
// localStorage persistence
// ---------------------------------------------------------------------------

export const STORAGE_KEY = 'rift:llmRouting:ledger';

/** Shape of the serialised ledger written to localStorage. */
interface PersistedLedger {
  sessionCostUsd: number;
  requestCount: number;
  totalTokensIn: number;
  totalTokensOut: number;
  lastRoute: RoutingEvent | null;
  recentRoutes: RoutingEvent[];
  costByModel: Record<string, number>;
  escalationCount: number;
  localRequests: number;
  cloudRequests: number;
  localTokensIn: number;
  localTokensOut: number;
  cloudTokensIn: number;
  cloudTokensOut: number;
  cloudSpendUsd: number;
  savingsUsd: number;
  savingsRefModelId: string | null;
}

/** Read ledger from localStorage. Returns null on missing or corrupt data. */
function loadFromStorage(): PersistedLedger | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return null;
    return parsed as PersistedLedger;
  } catch {
    return null;
  }
}

/** Write the current ledger state to localStorage (called debounced). */
function writeToStorage() {
  try {
    const payload: PersistedLedger = {
      sessionCostUsd,
      requestCount,
      totalTokensIn,
      totalTokensOut,
      lastRoute,
      recentRoutes,
      costByModel,
      escalationCount,
      localRequests,
      cloudRequests,
      localTokensIn,
      localTokensOut,
      cloudTokensIn,
      cloudTokensOut,
      cloudSpendUsd,
      savingsUsd,
      savingsRefModelId,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // Storage quota exceeded or unavailable — silently ignore.
  }
}

let _persistTimer: ReturnType<typeof setTimeout> | null = null;

/** Schedule a debounced write to localStorage (500 ms). */
function schedulePersist() {
  if (_persistTimer !== null) clearTimeout(_persistTimer);
  _persistTimer = setTimeout(writeToStorage, 500);
}

/** Hydrate in-memory state from localStorage on module init. */
function initFromStorage() {
  const saved = loadFromStorage();
  if (!saved) return;
  sessionCostUsd = typeof saved.sessionCostUsd === 'number' ? saved.sessionCostUsd : 0;
  requestCount = typeof saved.requestCount === 'number' ? saved.requestCount : 0;
  totalTokensIn = typeof saved.totalTokensIn === 'number' ? saved.totalTokensIn : 0;
  totalTokensOut = typeof saved.totalTokensOut === 'number' ? saved.totalTokensOut : 0;
  lastRoute = saved.lastRoute ?? null;
  recentRoutes = Array.isArray(saved.recentRoutes) ? saved.recentRoutes : [];
  costByModel = typeof saved.costByModel === 'object' && saved.costByModel !== null ? saved.costByModel : {};
  escalationCount = typeof saved.escalationCount === 'number' ? saved.escalationCount : 0;
  localRequests = typeof saved.localRequests === 'number' ? saved.localRequests : 0;
  cloudRequests = typeof saved.cloudRequests === 'number' ? saved.cloudRequests : 0;
  localTokensIn = typeof saved.localTokensIn === 'number' ? saved.localTokensIn : 0;
  localTokensOut = typeof saved.localTokensOut === 'number' ? saved.localTokensOut : 0;
  cloudTokensIn = typeof saved.cloudTokensIn === 'number' ? saved.cloudTokensIn : 0;
  cloudTokensOut = typeof saved.cloudTokensOut === 'number' ? saved.cloudTokensOut : 0;
  cloudSpendUsd = typeof saved.cloudSpendUsd === 'number' ? saved.cloudSpendUsd : 0;
  savingsUsd = typeof saved.savingsUsd === 'number' ? saved.savingsUsd : 0;
  savingsRefModelId = typeof saved.savingsRefModelId === 'string' ? saved.savingsRefModelId : null;
}

/** Force an immediate (non-debounced) write to localStorage. Exported for
 *  testing round-trips without waiting for the debounce interval. */
export function flushPersist() {
  if (_persistTimer !== null) {
    clearTimeout(_persistTimer);
    _persistTimer = null;
  }
  writeToStorage();
}

/** Re-hydrate in-memory state from localStorage. Exported so tests can
 *  simulate a page reload by calling this after seeding localStorage. */
export { initFromStorage };

// ---------------------------------------------------------------------------
// Hosting classification + counterfactual reference rate
// ---------------------------------------------------------------------------

/** Classify a model id as local (grunt) or cloud. Local AND remote
 *  (self-hosted) count as local — neither incurs per-token cloud cost. Unknown
 *  ids are treated as cloud so an unrecognized model never inflates savings. */
function hostingOf(modelId: string): 'local' | 'cloud' {
  const m = llmModels.getModel(modelId);
  if (!m) return 'cloud';
  return m.hosting.mode === 'cloud' ? 'cloud' : 'local';
}

/** The cloud model whose rate stands in for "what you'd have paid". Prefers the
 *  configured default model when it is cloud with a real rate; otherwise the
 *  CHEAPEST priced configured cloud model. Cheapest (not most expensive) is the
 *  deliberate choice: it makes the savings a conservative lower bound — the
 *  realistic floor of what the work would have cost on cloud — rather than
 *  overstating grunt-tier value by pricing against the most expensive model the
 *  user happens to have configured. Returns null when no cloud model carries a
 *  price, so savings show as n/a instead of a fabricated figure. */
function referenceCloudRate(): { modelId: string; input: number; output: number } | null {
  const priced = (c: { cost_per_1m_input: number; cost_per_1m_output: number }) =>
    c.cost_per_1m_input > 0 || c.cost_per_1m_output > 0;

  const def = llmModels.getModel(llmModels.defaultModel ?? '');
  if (def && def.hosting.mode === 'cloud' && priced(def.capabilities)) {
    return {
      modelId: def.id,
      input: def.capabilities.cost_per_1m_input,
      output: def.capabilities.cost_per_1m_output,
    };
  }

  const clouds = llmModels.models.filter(
    (m) => m.hosting.mode === 'cloud' && priced(m.capabilities),
  );
  if (clouds.length === 0) return null;
  const cheapest = clouds.reduce((a, b) =>
    b.capabilities.cost_per_1m_output < a.capabilities.cost_per_1m_output ? b : a,
  );
  return {
    modelId: cheapest.id,
    input: cheapest.capabilities.cost_per_1m_input,
    output: cheapest.capabilities.cost_per_1m_output,
  };
}

// ---------------------------------------------------------------------------
// Bus event handlers
// ---------------------------------------------------------------------------

export function handleRouteEvent(payload: Record<string, unknown>) {
  const event: RoutingEvent = {
    model_id: (payload.model_id as string) ?? '',
    task_type: (payload.task_type as string) ?? '',
    profile: (payload.profile as RoutingProfile) ?? 'manual',
    reason: (payload.reason as string) ?? '',
    was_overridden: (payload.was_overridden as boolean) ?? false,
    timestamp: Date.now(),
  };
  lastRoute = event;
  recentRoutes = [event, ...recentRoutes].slice(0, MAX_RECENT);
  requestCount++;
  schedulePersist();
}

export function handleResponseEvent(payload: Record<string, unknown>) {
  const tokIn = (payload.tokens_in as number) ?? 0;
  const tokOut = (payload.tokens_out as number) ?? 0;
  const cost = (payload.cost_usd as number) ?? 0;
  const modelId = (payload.model_id as string) ?? '';
  const wasEscalated = (payload.escalated as boolean) ?? false;

  totalTokensIn += tokIn;
  totalTokensOut += tokOut;
  sessionCostUsd += cost;

  if (modelId) {
    costByModel = {
      ...costByModel,
      [modelId]: (costByModel[modelId] ?? 0) + cost,
    };

    // Ledger: split by hosting and accrue counterfactual savings for local runs.
    if (hostingOf(modelId) === 'local') {
      localRequests++;
      localTokensIn += tokIn;
      localTokensOut += tokOut;
      const ref = referenceCloudRate();
      if (ref) {
        savingsUsd += (tokIn / 1_000_000) * ref.input + (tokOut / 1_000_000) * ref.output;
        savingsRefModelId = ref.modelId;
      }
    } else {
      cloudRequests++;
      cloudTokensIn += tokIn;
      cloudTokensOut += tokOut;
      cloudSpendUsd += cost;
    }
  }

  if (wasEscalated) {
    escalationCount++;
  }
  schedulePersist();
}

export function resetSession() {
  sessionCostUsd = 0;
  requestCount = 0;
  totalTokensIn = 0;
  totalTokensOut = 0;
  lastRoute = null;
  recentRoutes = [];
  costByModel = {};
  escalationCount = 0;
  localRequests = 0;
  cloudRequests = 0;
  localTokensIn = 0;
  localTokensOut = 0;
  cloudTokensIn = 0;
  cloudTokensOut = 0;
  cloudSpendUsd = 0;
  savingsUsd = 0;
  savingsRefModelId = null;
  // Cancel any pending debounced write and remove the persisted ledger.
  if (_persistTimer !== null) {
    clearTimeout(_persistTimer);
    _persistTimer = null;
  }
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Storage unavailable — silently ignore.
  }
}

// ---------------------------------------------------------------------------
// Module init — hydrate from storage once on load
// ---------------------------------------------------------------------------

initFromStorage();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export const llmRouting = {
  get sessionCostUsd() { return sessionCostUsd; },
  get requestCount() { return requestCount; },
  get totalTokensIn() { return totalTokensIn; },
  get totalTokensOut() { return totalTokensOut; },
  get lastRoute() { return lastRoute; },
  get recentRoutes() { return recentRoutes; },
  get costByModel() { return costByModel; },
  get escalationCount() { return escalationCount; },

  // Grunt-tier savings ledger (candidate 598).
  get localRequests() { return localRequests; },
  get cloudRequests() { return cloudRequests; },
  get localTokensIn() { return localTokensIn; },
  get localTokensOut() { return localTokensOut; },
  get cloudTokensIn() { return cloudTokensIn; },
  get cloudTokensOut() { return cloudTokensOut; },
  get cloudSpendUsd() { return cloudSpendUsd; },
  /** Counterfactual savings from routing locally instead of to the reference
   *  cloud model. `null` ref id means no priced cloud model is configured. */
  get savingsUsd() { return savingsUsd; },
  get savingsRefModelId() { return savingsRefModelId; },
  /** Fraction of accounted requests served locally (0..1). */
  get localShare() {
    const total = localRequests + cloudRequests;
    return total === 0 ? 0 : localRequests / total;
  },

  formatCost(usd: number): string {
    if (usd === 0) return '$0';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    if (usd < 1) return `$${usd.toFixed(3)}`;
    return `$${usd.toFixed(2)}`;
  },

  formatTokens(count: number): string {
    if (count < 1000) return `${count}`;
    if (count < 1_000_000) return `${(count / 1000).toFixed(1)}K`;
    return `${(count / 1_000_000).toFixed(2)}M`;
  },
};
