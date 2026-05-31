/**
 * terminalInject.ts — route a click-to-inject gesture to the active terminal.
 *
 * The HTML5-drag path (event row → terminal-host drop) targets a specific
 * terminal by where it is dropped. The click path has no spatial target, so it
 * needs to know which terminal is "active". Each {@link Terminal} registers its
 * raw-text paste function keyed by pane id and reports focus; this module
 * routes {@link injectIntoActiveTerminal} to the focused terminal (or, if focus
 * is unknown, the most-recently-registered one).
 *
 * Plain module state (not Svelte `$state`) — callers are imperative and do not
 * render off this registry.
 */

type Injector = (text: string) => void;

const registry = new Map<number, Injector>();
let activePaneId: number | null = null;

/** Register a terminal's raw-text paste fn. Call on mount. The newest terminal
 *  becomes the default active target — a freshly-opened pane is the most likely
 *  injection target until the user focuses another; the focusin path
 *  (setActiveInjector) then keeps it tracking the actually-focused terminal. */
export function registerInjector(paneId: number, inject: Injector): void {
  registry.set(paneId, inject);
  activePaneId = paneId;
}

/** Remove a terminal's injector. Call on cleanup. */
export function unregisterInjector(paneId: number): void {
  registry.delete(paneId);
  if (activePaneId === paneId) {
    const next = registry.keys().next();
    activePaneId = next.done ? null : next.value;
  }
}

/** Mark a terminal as the active injection target (call on focus). */
export function setActiveInjector(paneId: number): void {
  if (registry.has(paneId)) activePaneId = paneId;
}

/**
 * Inject text into the active terminal. Returns false if no terminal is
 * registered (e.g. the cockpit is detached with no terminal in this window),
 * so callers can surface a no-op rather than silently dropping the gesture.
 */
export function injectIntoActiveTerminal(text: string): boolean {
  let id = activePaneId;
  if (id === null || !registry.has(id)) {
    const next = registry.keys().next();
    id = next.done ? null : next.value;
  }
  if (id === null) return false;
  const fn = registry.get(id);
  if (!fn) return false;
  fn(text);
  return true;
}
