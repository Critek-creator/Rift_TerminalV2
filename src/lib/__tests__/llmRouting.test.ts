import { describe, it, expect, beforeEach } from 'vitest';
import { llmRouting, handleRouteEvent, handleResponseEvent, resetSession } from '../llmRouting.svelte';

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
});
