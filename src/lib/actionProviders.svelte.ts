// actionProviders.svelte.ts — in-process control-endpoint provider.
//
// Demonstrates the §9 control-endpoint protocol (candidate 568) end to end:
// it DECLARES a real, invocable action over the bus and FULFILLS invocations,
// publishing results — exactly the contract an external integration (Aegis,
// Sentinel, a git translator) follows over IPC. Because it goes through
// `publish`/`subscribe`, the envelopes genuinely traverse the Rust tokio bus;
// an in-process provider and a remote one are indistinguishable to the
// registry. Additional adopters (git fetch/pull/push, Aegis "run audit",
// Sentinel "acknowledge") declare more actions through these same kinds.

import { subscribe, publish, type Envelope } from './bus';
import { resetSession } from './llmRouting.svelte';

let unsub: (() => Promise<void>) | undefined;
let started = false;

/** Reset the grunt-tier routing ledger — an action the LLM activity tab
 *  otherwise had no control surface for. */
const RESET_LEDGER = 'rift.llm.reset-ledger';

async function handleInvoke(env: Envelope): Promise<void> {
  if (env.kind !== 'action.invoke') return;
  const p = (env.payload ?? {}) as Record<string, unknown>;
  if (p.action_id !== RESET_LEDGER) return;
  const invocation_id = typeof p.invocation_id === 'string' ? p.invocation_id : undefined;
  try {
    resetSession();
    await publish('system', 'action.result', {
      invocation_id,
      action_id: RESET_LEDGER,
      status: 'ok',
      message: 'routing ledger cleared',
    });
  } catch (e) {
    await publish('system', 'action.result', {
      invocation_id,
      action_id: RESET_LEDGER,
      status: 'error',
      message: String(e),
    });
  }
}

export const actionProviders = {
  async start(): Promise<void> {
    if (started) return;
    started = true;
    unsub = await subscribe({ category: 'system' }, handleInvoke);
    await publish('system', 'action.declare', {
      id: RESET_LEDGER,
      target: 'llm-activity',
      label: 'reset ledger',
      description: 'Clear the session routing ledger — cost, tokens, and grunt-tier savings.',
      confirm: true,
      danger: true,
    });
  },
  async stop(): Promise<void> {
    started = false;
    await unsub?.();
    unsub = undefined;
  },
};
