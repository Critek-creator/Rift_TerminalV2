// errorHandoffProvider.svelte.ts — Phase 5 / R1, decision D1.
//
// The frontend provider that makes the error→agent handoff work in BARE Rift,
// with no external integration present: it answers `rift.error.explain::*`
// invocations by running a LOCAL model and publishing an `action.result`.
//
// Privacy (D2, locked + privacy-critical): the prompt carries the user's
// command + stderr + cwd, so it must NEVER leave the machine. This provider:
//   1. answers ONLY when a resident local server is running, and
//   2. calls `llm_complete` with `localOnly: true`, which pins the primary to a
//      resident-local model and forbids cloud escalation (backend guard).
// With no resident local model it DEGRADES (an `error` result telling the user
// to load one) — it never routes to cloud.
//
// Like actionProviders.svelte.ts, this goes through publish/subscribe so the
// envelopes traverse the Rust tokio bus exactly as a remote integration would.

import { invoke } from '@tauri-apps/api/core';
import { subscribe, publish, type Envelope } from './bus';
import { llmModels } from './llmModels.svelte';
import {
  isErrorExplainAction,
  isErrorFixAction,
  buildExplainPrompt,
  buildFixPrompt,
  parseProposedCommand,
  type FailureContext,
} from './errorHandoff';

interface LlmCompleteResult {
  content: string;
  model_used: string;
  escalated: boolean;
}

let unsub: (() => Promise<void>) | undefined;
let started = false;

/**
 * The id of a RESIDENT LOCAL model to answer with, or null if none is loaded.
 * Backend truth (`llm_models_running`) is authoritative; we intersect it with
 * the configured models that are actually `local`-hosted so a running cloud
 * proxy can't qualify. Prefers a model tagged for grunt/utility work, else the
 * first resident local model.
 */
async function residentLocalModelId(): Promise<string | null> {
  let running: string[] = [];
  try {
    running = await invoke<string[]>('llm_models_running');
  } catch {
    return null;
  }
  if (running.length === 0) return null;
  const runningSet = new Set(running);
  const localResident = llmModels.models.filter(
    (m) => m.hosting.mode === 'local' && runningSet.has(m.id),
  );
  if (localResident.length === 0) return null;
  const grunt = localResident.find((m) =>
    (m.capabilities?.strength_tags ?? []).some((t) =>
      /grunt|util|fast|small/i.test(t),
    ),
  );
  return (grunt ?? localResident[0]).id;
}

async function handleInvoke(env: Envelope): Promise<void> {
  if (env.kind !== 'action.invoke') return;
  const p = (env.payload ?? {}) as Record<string, unknown>;
  const actionId = typeof p.action_id === 'string' ? p.action_id : '';
  const isExplain = isErrorExplainAction(actionId);
  const isFix = isErrorFixAction(actionId);
  if (!isExplain && !isFix) return;

  const invocation_id = typeof p.invocation_id === 'string' ? p.invocation_id : undefined;
  const ctx = p.params as FailureContext | undefined;

  const fail = (message: string) =>
    publish('system', 'action.result', {
      invocation_id,
      action_id: actionId,
      status: 'error',
      message,
    });

  if (!ctx || typeof ctx.command !== 'string') {
    await fail('error-handoff: no failure context attached to the invocation');
    return;
  }

  // D2 — degrade, never escalate. With no resident local model we refuse to
  // call out at all, so the failing command + stderr never leave the machine.
  const modelId = await residentLocalModelId();
  if (!modelId) {
    await fail(
      'No local model loaded — the raw error is shown above. Load a local model in Settings → Models to get offline help (nothing is ever sent to the cloud).',
    );
    return;
  }

  try {
    const res = await invoke<LlmCompleteResult>('llm_complete', {
      modelId,
      prompt: isFix ? buildFixPrompt(ctx) : buildExplainPrompt(ctx),
      localOnly: true,
    });
    // Belt-and-suspenders: the backend guard already forbids cloud escalation,
    // but if anything unexpected escalated off the pinned local model, refuse
    // to surface it rather than imply the result stayed local.
    if (res.escalated) {
      await fail('error-handoff: aborted — completion escalated off the local model.');
      return;
    }

    if (isFix) {
      // R3 — parse a paste-ready candidate command. No command (or NONE) is a
      // valid, non-error outcome: we just have nothing safe to propose.
      const proposed = parseProposedCommand(res.content);
      await publish('system', 'action.result', {
        invocation_id,
        action_id: actionId,
        status: 'ok',
        message: proposed ? '' : 'No safe fix could be proposed for this failure.',
        ...(proposed ? { proposed_command: proposed } : {}),
      });
      return;
    }

    await publish('system', 'action.result', {
      invocation_id,
      action_id: actionId,
      status: 'ok',
      message: res.content.trim() || '(the local model returned an empty explanation)',
    });
  } catch (e) {
    await fail(`error-handoff: local ${isFix ? 'fix' : 'explain'} failed — ${String(e)}`);
  }
}

export const errorHandoffProvider = {
  /** Subscribe to the action.* channel and answer explain invocations.
   *  Idempotent. No `action.declare` here — failures declare their own
   *  per-failure action ids in Terminal.svelte (B1 fix); this provider just
   *  fulfills any `rift.error.explain::*` invoke that arrives. */
  async start(): Promise<void> {
    if (started) return;
    started = true;
    unsub = await subscribe({ category: 'system' }, handleInvoke);
  },
  async stop(): Promise<void> {
    started = false;
    await unsub?.();
    unsub = undefined;
  },
};
