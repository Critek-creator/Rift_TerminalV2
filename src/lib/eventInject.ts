/**
 * eventInject.ts — derive the prompt-injection text for a notification event.
 *
 * Feature: "Drag-or-click a notification event into the terminal" (candidate
 * 2026-05-31-...-547). The notification tabs already hold structured event
 * payloads (failed commands, errors, fs paths); this maps an {@link Envelope}
 * to the single editable line a user would want to act on next.
 *
 * Design contract: the returned text is ALWAYS a single line (newlines
 * collapsed to spaces). It is inserted as editable prompt text plus a trailing
 * space — never auto-executed (see Terminal.svelte `pasteTextIntoTerminal`).
 */

import type { Envelope } from './bus';

/** Collapse any run of whitespace containing a newline into a single space,
 *  and trim. Keeps multi-line errors / stack traces injectable as one line. */
function oneLine(s: string): string {
  return s.replace(/\s*[\r\n]+\s*/g, ' ').replace(/[ \t]{2,}/g, ' ').trim();
}

/**
 * Map a bus envelope to the text to inject at the terminal prompt.
 *
 * Per kind:
 *  - `command.submitted` (Category::Pty, payload `{ command, raw_len }`) → the
 *    command string, ready to re-run.
 *  - `error` (Category::System, payload `{ source, message, context }`) → the
 *    human-readable message.
 *  - filesystem events (payload `{ path }`) → the path.
 *  - otherwise → the first of `command` / `message` / `text` present, else a
 *    compact `kind + json` summary so the gesture is never a silent no-op.
 */
export function envelopeToInjectText(env: Envelope): string {
  const p =
    env.payload && typeof env.payload === 'object'
      ? (env.payload as Record<string, unknown>)
      : {};

  if (env.kind === 'command.submitted' && typeof p.command === 'string') {
    return oneLine(p.command);
  }
  if (env.kind === 'error' && typeof p.message === 'string') {
    return oneLine(p.message);
  }
  if (typeof p.path === 'string') return oneLine(p.path);

  if (typeof p.command === 'string') return oneLine(p.command);
  if (typeof p.message === 'string') return oneLine(p.message);
  if (typeof p.text === 'string') return oneLine(p.text);
  if (typeof env.payload === 'string') return oneLine(env.payload);

  try {
    return oneLine(`${env.kind} ${JSON.stringify(env.payload)}`);
  } catch {
    return env.kind;
  }
}
