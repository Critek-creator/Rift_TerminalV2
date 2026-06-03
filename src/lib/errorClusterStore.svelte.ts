/**
 * errorClusterStore — persistent singleton that owns clustered error state
 * across the whole session.
 *
 * Audit debt #6 fix: the previous design kept cluster state (counts,
 * first/last seen) inside NotificationPane as component-local $state.
 * Every tab switch / component unmount destroyed the clusters, so badge
 * counts reset and history was lost.
 *
 * This store lifts that state out of the component:
 *   • Accepts envelopes pushed by callers (e.g. NotificationPane, or a
 *     future bus subscriber).
 *   • Keeps a flat rolling buffer of every received envelope.
 *   • Re-derives the cluster list reactively via clusterEvents() whenever
 *     the buffer changes.
 *   • Survives unmount — the singleton lives in module scope.
 *
 * §9 boundary: the store does NOT subscribe to the bus itself. Bus
 * subscription stays in the component / App.svelte layer so that the
 * category filter and pause/clear controls remain component-owned. The
 * store is a pure recipient of `push()` calls.
 *
 * Wiring NotificationPane to consume the store (replacing its local
 * `events` array and `clusters` derived) is DESIGN-DEFERRED — it needs
 * visual verification and is a near one-liner once the component is
 * opened for editing.
 */

import { clusterEvents, type EventCluster } from './errorClustering';
import type { Envelope } from './bus';

/** Upper bound on the raw envelope buffer kept in memory. */
const MAX_EVENTS = 2000;

export class ErrorClusterStore {
  /**
   * Flat buffer of every received envelope, oldest → newest.
   * Assign-replace on every mutation so $derived consumers re-run.
   * Capped at MAX_EVENTS: when the buffer overflows, the oldest half is
   * dropped (same trim heuristic as NotificationPane's rolling window).
   */
  events = $state<Envelope[]>([]);

  /**
   * Cluster list computed from the full event buffer.
   * Recomputed by clusterEvents() on every read; because `this.events` is a
   * $state field, Svelte 5 tracks this getter as a reactive dependency when
   * accessed from inside a $derived expression in a component.
   *
   * Intentionally NOT declared as a class-field `$derived(...)` — that form
   * is not yet used elsewhere in this codebase and its interaction with the
   * class field initializer order is untested here. Callers that need
   * reactivity wrap the read in their own `$derived`:
   *   const clusters = $derived(errorClusterStore.clusters);
   */
  get clusters(): EventCluster[] {
    return clusterEvents(this.events);
  }

  /** Total raw event count (before clustering). */
  get totalCount(): number {
    return this.events.length;
  }

  /** Number of distinct clusters (= number of distinct problems). */
  get clusterCount(): number {
    return this.clusters.length;
  }

  /**
   * Accept a new envelope and fold it into the buffer.
   * Callers are responsible for any severity/filter checks before pushing;
   * the store is policy-free — every pushed envelope is kept.
   */
  push(env: Envelope): void {
    let next = [...this.events, env];
    if (next.length > MAX_EVENTS) {
      // Drop the oldest half; keeps memory bounded while retaining recency.
      next = next.slice(-Math.floor(MAX_EVENTS / 2));
    }
    this.events = next;
  }

  /**
   * Push multiple envelopes at once (e.g. replay snapshots).
   * Applies the same cap logic as push() after appending all at once.
   */
  pushAll(envs: Envelope[]): void {
    if (envs.length === 0) return;
    let next = [...this.events, ...envs];
    if (next.length > MAX_EVENTS) {
      next = next.slice(-Math.floor(MAX_EVENTS / 2));
    }
    this.events = next;
  }

  /**
   * Clear all events and clusters.
   * Matches the clearEvents() action in NotificationPane so the component
   * can delegate clearing to the store once wired.
   */
  clear(): void {
    this.events = [];
  }
}

/**
 * Module-level singleton — survives component mount/unmount cycles.
 * Import this wherever cluster state needs to be read or written.
 *
 * Usage (read):
 *   import { errorClusterStore } from './errorClusterStore.svelte';
 *   // In a Svelte 5 component:
 *   const clusters = $derived(errorClusterStore.clusters);
 *
 * Usage (write — from NotificationPane or App.svelte):
 *   errorClusterStore.push(env);        // single event
 *   errorClusterStore.pushAll(events);  // replay batch
 *   errorClusterStore.clear();          // user-triggered clear
 */
export const errorClusterStore = new ErrorClusterStore();
