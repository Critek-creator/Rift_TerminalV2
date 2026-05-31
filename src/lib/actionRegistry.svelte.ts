// actionRegistry.svelte.ts — §9 capability class 2 (control endpoints).
//
// The generic declare-and-render mechanism that was missing (candidate 568).
// Event subscription and data enrichment already had generic plumbing; control
// endpoints were each hand-coded (git_action_command, agent cancel). This
// registry lets ANY integration declare invocable actions over the bus, which
// Rift renders as buttons (via ControlActions.svelte) and round-trips:
//
//   action.declare  (integration → Rift)  { id, target, label, ... }
//   action.invoke   (Rift → integration)  { invocation_id, action_id, target }
//   action.result   (integration → Rift)  { invocation_id|action_id, status, message? }
//   action.revoke   (integration → Rift)  { id, target }
//
// All four ride Category::System (additive kinds — no envelope-version bump,
// per envelope.rs). External integrations use the identical kinds over IPC; an
// in-process provider (see actionProviders.svelte.ts) uses the same bus, so the
// envelopes genuinely traverse the Rust tokio bus either way.

import { subscribe, publish, type Envelope } from './bus';

export interface DeclaredAction {
  /** Stable, integration-namespaced id, e.g. `rift.llm.reset-ledger`. */
  id: string;
  /** Tab/category id this action attaches to (the render target). */
  target: string;
  label: string;
  description?: string;
  /** Require a confirm click before the invoke is published. */
  confirm?: boolean;
  /** Style hint for destructive actions. */
  danger?: boolean;
}

export interface ActionResultState {
  status: 'ok' | 'error';
  message?: string;
  ts: number;
}

let actionsByTarget = $state<Record<string, DeclaredAction[]>>({});
let pending = $state<Record<string, true>>({}); // keyed by action id
let results = $state<Record<string, ActionResultState>>({}); // keyed by action id
const invocations = new Map<string, string>(); // invocation_id → action_id

let unsub: (() => Promise<void>) | undefined;
let started = false;

function upsert(a: DeclaredAction): void {
  const list = (actionsByTarget[a.target] ?? []).filter((x) => x.id !== a.id);
  actionsByTarget = { ...actionsByTarget, [a.target]: [...list, a] };
}

function revoke(target: string, id: string): void {
  const list = (actionsByTarget[target] ?? []).filter((x) => x.id !== id);
  actionsByTarget = { ...actionsByTarget, [target]: list };
}

function handle(env: Envelope): void {
  const p = (env.payload ?? {}) as Record<string, unknown>;
  switch (env.kind) {
    case 'action.declare': {
      const { id, target, label } = p as { id?: string; target?: string; label?: string };
      if (!id || !target || !label) return;
      upsert({
        id,
        target,
        label,
        description: typeof p.description === 'string' ? p.description : undefined,
        confirm: p.confirm === true,
        danger: p.danger === true,
      });
      break;
    }
    case 'action.revoke': {
      if (typeof p.id === 'string' && typeof p.target === 'string') revoke(p.target, p.id);
      break;
    }
    case 'action.result': {
      const invId = typeof p.invocation_id === 'string' ? p.invocation_id : null;
      // Prefer correlation by invocation_id. Fall back to a bare action_id only
      // when that action is actually pending — otherwise a stray or replayed
      // result envelope could fake a success/failure for an action the user
      // never invoked.
      const actionId = invId
        ? invocations.get(invId)
        : typeof p.action_id === 'string' && pending[p.action_id] === true
          ? p.action_id
          : undefined;
      if (!actionId) return;
      if (invId) invocations.delete(invId);
      const { [actionId]: _drop, ...restPending } = pending;
      pending = restPending;
      results = {
        ...results,
        [actionId]: {
          status: p.status === 'error' ? 'error' : 'ok',
          message: typeof p.message === 'string' ? p.message : undefined,
          ts: Date.now(),
        },
      };
      break;
    }
    default:
      break;
  }
}

function newInvocationId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `inv-${Date.now()}-${Math.floor(Math.random() * 1_000_000)}`;
  }
}

export const actionRegistry = {
  /** Subscribe to the action.* channel. Idempotent. The bus replay buffer
   *  means declarations published before this runs are still delivered. */
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

  actionsFor(target: string): DeclaredAction[] {
    return actionsByTarget[target] ?? [];
  },
  isPending(actionId: string): boolean {
    return pending[actionId] === true;
  },
  resultFor(actionId: string): ActionResultState | undefined {
    return results[actionId];
  },

  /** Publish an action.invoke and mark the action pending until its result
   *  arrives. The invocation_id correlates the eventual action.result. */
  async invoke(a: DeclaredAction): Promise<void> {
    const invocation_id = newInvocationId();
    invocations.set(invocation_id, a.id);
    pending = { ...pending, [a.id]: true };
    const { [a.id]: _drop, ...restResults } = results;
    results = restResults;
    await publish('system', 'action.invoke', {
      invocation_id,
      action_id: a.id,
      target: a.target,
    });
  },
};
