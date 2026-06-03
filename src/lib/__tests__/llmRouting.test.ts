import { describe, it, expect, beforeEach } from 'vitest';
import {
  llmRouting,
  handleRouteEvent,
  handleResponseEvent,
  resetSession,
  flushPersist,
  initFromStorage,
  STORAGE_KEY,
  getLedgerSnapshot,
} from '../llmRouting.svelte';

describe('llmRouting store', () => {
  beforeEach(() => {
    resetSession();
  });

  describe('resetSession', () => {
    it('clears all counters to zero', () => {
      // Mutate state first
      handleRouteEvent({ model_id: 'opus', task_type: 'chat', profile: 'manual', reason: 'test' });
      handleResponseEvent({ model_id: 'opus', tokens_in: 100, tokens_out: 50, cost_usd: 0.01 });

      resetSession();

      expect(llmRouting.sessionCostUsd).toBe(0);
      expect(llmRouting.requestCount).toBe(0);
      expect(llmRouting.totalTokensIn).toBe(0);
      expect(llmRouting.totalTokensOut).toBe(0);
      expect(llmRouting.lastRoute).toBeNull();
      expect(llmRouting.recentRoutes).toEqual([]);
      expect(llmRouting.costByModel).toEqual({});
      expect(llmRouting.escalationCount).toBe(0);
    });
  });

  describe('handleRouteEvent', () => {
    it('increments requestCount and sets lastRoute', () => {
      handleRouteEvent({
        model_id: 'opus',
        task_type: 'chat',
        profile: 'manual',
        reason: 'user override',
        was_overridden: false,
      });

      expect(llmRouting.requestCount).toBe(1);
      expect(llmRouting.lastRoute).not.toBeNull();
      expect(llmRouting.lastRoute!.model_id).toBe('opus');
      expect(llmRouting.lastRoute!.task_type).toBe('chat');
      expect(llmRouting.lastRoute!.reason).toBe('user override');
    });

    it('accumulates recentRoutes on multiple calls', () => {
      handleRouteEvent({ model_id: 'opus', task_type: 'chat' });
      handleRouteEvent({ model_id: 'haiku', task_type: 'classify' });

      expect(llmRouting.requestCount).toBe(2);
      expect(llmRouting.recentRoutes).toHaveLength(2);
      // Most recent first
      expect(llmRouting.recentRoutes[0].model_id).toBe('haiku');
      expect(llmRouting.recentRoutes[1].model_id).toBe('opus');
    });
  });

  describe('handleResponseEvent', () => {
    it('accumulates sessionCostUsd, totalTokensIn, totalTokensOut', () => {
      handleResponseEvent({ model_id: 'opus', tokens_in: 100, tokens_out: 50, cost_usd: 0.01 });
      handleResponseEvent({ model_id: 'opus', tokens_in: 200, tokens_out: 75, cost_usd: 0.02 });

      expect(llmRouting.sessionCostUsd).toBeCloseTo(0.03);
      expect(llmRouting.totalTokensIn).toBe(300);
      expect(llmRouting.totalTokensOut).toBe(125);
    });

    it('increments escalationCount when escalated is true', () => {
      handleResponseEvent({ model_id: 'opus', tokens_in: 10, tokens_out: 5, cost_usd: 0.001, escalated: true });
      handleResponseEvent({ model_id: 'haiku', tokens_in: 10, tokens_out: 5, cost_usd: 0.001, escalated: false });
      handleResponseEvent({ model_id: 'opus', tokens_in: 10, tokens_out: 5, cost_usd: 0.001, escalated: true });

      expect(llmRouting.escalationCount).toBe(2);
    });

    it('tracks costByModel per model_id', () => {
      handleResponseEvent({ model_id: 'opus', tokens_in: 10, tokens_out: 5, cost_usd: 0.05 });
      handleResponseEvent({ model_id: 'haiku', tokens_in: 10, tokens_out: 5, cost_usd: 0.001 });
      handleResponseEvent({ model_id: 'opus', tokens_in: 10, tokens_out: 5, cost_usd: 0.03 });

      expect(llmRouting.costByModel['opus']).toBeCloseTo(0.08);
      expect(llmRouting.costByModel['haiku']).toBeCloseTo(0.001);
    });
  });

  describe('formatCost', () => {
    it('returns "$0" for zero', () => {
      expect(llmRouting.formatCost(0)).toBe('$0');
    });

    it('returns 4 decimals for sub-cent values', () => {
      expect(llmRouting.formatCost(0.001)).toBe('$0.0010');
    });

    it('returns 3 decimals for sub-dollar values', () => {
      expect(llmRouting.formatCost(0.05)).toBe('$0.050');
    });

    it('returns 2 decimals for dollar+ values', () => {
      expect(llmRouting.formatCost(1.5)).toBe('$1.50');
    });
  });

  describe('formatTokens', () => {
    it('returns plain number for sub-1K', () => {
      expect(llmRouting.formatTokens(500)).toBe('500');
    });

    it('returns K suffix for thousands', () => {
      expect(llmRouting.formatTokens(1500)).toBe('1.5K');
    });

    it('returns M suffix for millions', () => {
      expect(llmRouting.formatTokens(1500000)).toBe('1.50M');
    });
  });

  describe('localStorage persistence', () => {
    it('save → reload → load round-trip preserves the full ledger', () => {
      // Build up some state.
      handleRouteEvent({ model_id: 'haiku', task_type: 'classify', profile: 'cost_optimized', reason: 'grunt tier', was_overridden: false });
      handleResponseEvent({ model_id: 'haiku', tokens_in: 200, tokens_out: 80, cost_usd: 0.007 });
      handleResponseEvent({ model_id: 'haiku', tokens_in: 100, tokens_out: 40, cost_usd: 0.003, escalated: true });

      // Flush to localStorage synchronously.
      flushPersist();

      // Capture the serialised snapshot before reset wipes it.
      const snapshot = localStorage.getItem(STORAGE_KEY);
      expect(snapshot).not.toBeNull();

      // Reset memory AND storage (simulates user pressing "Clear session" in a
      // different tab — or just lets us verify re-hydration from scratch).
      resetSession();
      expect(llmRouting.sessionCostUsd).toBe(0);
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull();

      // Re-seed storage with the captured snapshot (simulates page reload with
      // the data still in localStorage from a previous session).
      localStorage.setItem(STORAGE_KEY, snapshot!);

      // Re-hydrate in-memory state.
      initFromStorage();

      // Assert all ledger fields were restored.
      expect(llmRouting.sessionCostUsd).toBeCloseTo(0.01);
      expect(llmRouting.requestCount).toBe(1);   // only handleRouteEvent increments requestCount
      expect(llmRouting.totalTokensIn).toBe(300);
      expect(llmRouting.totalTokensOut).toBe(120);
      expect(llmRouting.escalationCount).toBe(1);
      expect(llmRouting.costByModel['haiku']).toBeCloseTo(0.01);
      expect(llmRouting.recentRoutes).toHaveLength(1);
      expect(llmRouting.recentRoutes[0].model_id).toBe('haiku');
      expect(llmRouting.lastRoute).not.toBeNull();
      expect(llmRouting.lastRoute!.model_id).toBe('haiku');
      // Cloud counters — haiku is unrecognised so defaults to 'cloud'.
      expect(llmRouting.cloudRequests).toBe(2);
      expect(llmRouting.cloudSpendUsd).toBeCloseTo(0.01);
    });

    it('missing localStorage entry degrades to empty ledger without throwing', () => {
      // Ensure nothing is stored.
      localStorage.removeItem(STORAGE_KEY);

      // Should not throw; all counters should remain at their zero defaults.
      expect(() => initFromStorage()).not.toThrow();
      expect(llmRouting.sessionCostUsd).toBe(0);
      expect(llmRouting.requestCount).toBe(0);
      expect(llmRouting.recentRoutes).toEqual([]);
    });

    it('corrupt JSON in localStorage degrades to empty ledger without throwing', () => {
      localStorage.setItem(STORAGE_KEY, '{ not valid json !!!');

      expect(() => initFromStorage()).not.toThrow();
      expect(llmRouting.sessionCostUsd).toBe(0);
      expect(llmRouting.totalTokensIn).toBe(0);
    });

    it('non-object value in localStorage degrades to empty ledger without throwing', () => {
      localStorage.setItem(STORAGE_KEY, '"just a string"');

      expect(() => initFromStorage()).not.toThrow();
      expect(llmRouting.sessionCostUsd).toBe(0);
    });

    it('resetSession removes the key from localStorage', () => {
      handleResponseEvent({ model_id: 'opus', tokens_in: 10, tokens_out: 5, cost_usd: 0.001 });
      flushPersist();
      expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull();

      resetSession();
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    });
  });

  describe('getLedgerSnapshot — profile analytics capture', () => {
    it('returns a valid JSON string containing current ledger fields', () => {
      handleRouteEvent({ model_id: 'sonnet', task_type: 'chat', profile: 'balanced', reason: 'default', was_overridden: false });
      handleResponseEvent({ model_id: 'sonnet', tokens_in: 500, tokens_out: 200, cost_usd: 0.025 });

      const snap = getLedgerSnapshot();
      expect(snap).not.toBeNull();
      expect(typeof snap).toBe('string');

      const parsed = JSON.parse(snap!);
      expect(parsed.sessionCostUsd).toBeCloseTo(0.025);
      expect(parsed.requestCount).toBe(1);
      expect(parsed.totalTokensIn).toBe(500);
      expect(parsed.totalTokensOut).toBe(200);
      expect(parsed.recentRoutes).toHaveLength(1);
      expect(parsed.recentRoutes[0].model_id).toBe('sonnet');
      expect(parsed.lastRoute).not.toBeNull();
      expect(parsed.lastRoute.model_id).toBe('sonnet');
    });

    it('snapshot reflects zero state after resetSession', () => {
      // Build up some data then reset before capturing.
      handleResponseEvent({ model_id: 'opus', tokens_in: 100, tokens_out: 50, cost_usd: 0.01 });
      resetSession();

      const snap = getLedgerSnapshot();
      expect(snap).not.toBeNull();
      const parsed = JSON.parse(snap!);
      expect(parsed.sessionCostUsd).toBe(0);
      expect(parsed.requestCount).toBe(0);
      expect(parsed.totalTokensIn).toBe(0);
      expect(parsed.recentRoutes).toEqual([]);
      expect(parsed.lastRoute).toBeNull();
    });

    it('snapshot is a point-in-time copy: mutating state after capture does not change the snapshot', () => {
      handleResponseEvent({ model_id: 'haiku', tokens_in: 50, tokens_out: 20, cost_usd: 0.002 });

      const snap = getLedgerSnapshot();
      const before = JSON.parse(snap!);

      // Add more data AFTER the snapshot.
      handleResponseEvent({ model_id: 'haiku', tokens_in: 300, tokens_out: 100, cost_usd: 0.015 });

      // The snapshot should still hold the pre-mutation values.
      expect(before.sessionCostUsd).toBeCloseTo(0.002);
      expect(before.totalTokensIn).toBe(50);
      // The live store should have the new values.
      expect(llmRouting.sessionCostUsd).toBeCloseTo(0.017);
    });

    it('snapshot round-trips: parse snapshot, seed localStorage, initFromStorage restores matching state', () => {
      handleRouteEvent({ model_id: 'opus', task_type: 'code', profile: 'quality', reason: 'max', was_overridden: true });
      handleResponseEvent({ model_id: 'opus', tokens_in: 1000, tokens_out: 400, cost_usd: 0.08, escalated: false });

      const snap = getLedgerSnapshot();
      expect(snap).not.toBeNull();

      // Simulate page reload: reset + seed storage with snapshot.
      resetSession();
      localStorage.setItem(STORAGE_KEY, snap!);
      initFromStorage();

      expect(llmRouting.sessionCostUsd).toBeCloseTo(0.08);
      expect(llmRouting.requestCount).toBe(1);
      expect(llmRouting.totalTokensIn).toBe(1000);
      expect(llmRouting.totalTokensOut).toBe(400);
      expect(llmRouting.lastRoute?.model_id).toBe('opus');
      expect(llmRouting.recentRoutes).toHaveLength(1);
    });
  });
});
