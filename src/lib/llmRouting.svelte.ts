// llmRouting.svelte.ts — Ensemble Router Phase 2 routing state.
//
// Tracks routing decisions, accumulated session cost, and last routing
// metadata. Subscribes to Category::Llm bus events.

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
}

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
