// llmRouting.svelte.ts — Ensemble Router Phase 2 routing state.
//
// Tracks routing decisions, accumulated session cost, and last routing
// metadata. Subscribes to Category::Llm bus events.

import type { RoutingProfile } from './riftConfig';

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

const MAX_RECENT = 50;

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
