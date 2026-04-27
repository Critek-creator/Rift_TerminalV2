// Phase 3.5b — pop-out infrastructure (§10.5).
// Global rune-aware store that summons ephemeral overlays from anywhere.
// `App.svelte` mounts a `<Popout>` per entry; consumers (rule editor /
// file viewer / agent confirm — Phase 5+) call `popouts.summon(...)`
// without needing to know about the wire-up.
//
// File extension `.svelte.ts` is REQUIRED so the Svelte 5 rune compiler
// processes `$state`. Plain `.ts` will not work.

/** Discriminated union of overlay content kinds. v1 ships `text` +
 *  `confirm`; future kinds (component / snippet) gated to Phase 5+. */
export type PopoutContent =
  | {
      kind: 'text';
      title: string;
      body: string;
    }
  | {
      kind: 'confirm';
      title: string;
      body: string;
      confirmLabel?: string;
      cancelLabel?: string;
      onConfirm?: () => void;
      onCancel?: () => void;
    };

export interface PopoutEntry {
  /** Monotonic id assigned by the store; used as the `{#each}` key
   *  and as the dismissal handle. */
  id: number;
  content: PopoutContent;
  /** CSS width applied to the card. Default `min(640px, 80vw)`. */
  width?: string;
  /** When `false`, only `dismiss(id)` / `dismissAll()` can close the
   *  overlay — Esc + backdrop + close-X all become no-ops. Default `true`. */
  dismissible?: boolean;
  /** Fired on every dismiss path (programmatic, Esc, backdrop, close-X,
   *  confirm/cancel). Useful for callers that need to free resources
   *  regardless of how the overlay closed. */
  onDismiss?: () => void;
}

class PopoutStore {
  /** Ordered stack — last entry is the top of the visual stack and the
   *  only entry that responds to Esc + backdrop click. */
  entries = $state<PopoutEntry[]>([]);
  #nextId = 1;

  /** Append a new overlay to the stack and return its assigned id.
   *  Reactivity uses immutable update (spread) to match the rest of the
   *  codebase's `$state` pattern. */
  summon(opts: Omit<PopoutEntry, 'id'>): number {
    const id = this.#nextId++;
    const entry: PopoutEntry = { id, ...opts };
    this.entries = [...this.entries, entry];
    return id;
  }

  /** Remove the entry with this id and fire its `onDismiss`.
   *  No-op if the id is unknown. */
  dismiss(id: number): void {
    const entry = this.entries.find((e) => e.id === id);
    if (!entry) return;
    this.entries = this.entries.filter((e) => e.id !== id);
    entry.onDismiss?.();
  }

  /** Dismiss the topmost (last) entry. No-op if the stack is empty. */
  dismissTop(): void {
    if (this.entries.length === 0) return;
    const top = this.entries[this.entries.length - 1];
    this.dismiss(top.id);
  }

  /** Clear the entire stack and fire each entry's `onDismiss` in order
   *  (bottom → top). Snapshot first so `onDismiss` callbacks that
   *  re-summon don't observe a partially-cleared stack. */
  dismissAll(): void {
    const snapshot = this.entries;
    this.entries = [];
    for (const entry of snapshot) {
      entry.onDismiss?.();
    }
  }
}

export const popouts = new PopoutStore();
