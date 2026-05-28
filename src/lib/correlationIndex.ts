import type { Envelope } from './bus';

const MAX_CHAINS = 1000;
const MAX_CHAIN_SIZE = 50;

export class CorrelationIndex {
  private chains = new Map<string, Envelope[]>();
  private insertOrder: string[] = [];

  index(env: Envelope): void {
    if (!env.correlation_id) return;
    const cid = env.correlation_id;
    let chain = this.chains.get(cid);
    if (!chain) {
      chain = [];
      this.chains.set(cid, chain);
      this.insertOrder.push(cid);
      if (this.insertOrder.length > MAX_CHAINS) {
        const evicted = this.insertOrder.shift()!;
        this.chains.delete(evicted);
      }
    }
    chain.push(env);
    if (chain.length > MAX_CHAIN_SIZE) chain.shift();
  }

  getChain(correlationId: string): Envelope[] {
    const chain = this.chains.get(correlationId);
    if (!chain) return [];
    return chain.slice().sort((a, b) => a.ts - b.ts);
  }

  getRelated(env: Envelope): Envelope[] {
    if (!env.correlation_id) return [];
    return this.getChain(env.correlation_id).filter((e) => e !== env);
  }

  chainSize(correlationId: string | undefined): number {
    if (!correlationId) return 0;
    return this.chains.get(correlationId)?.length ?? 0;
  }

  reset(): void {
    this.chains.clear();
    this.insertOrder = [];
  }
}
