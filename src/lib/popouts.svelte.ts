// Phase 3.5b — pop-out infrastructure (§10.5).
// Global rune-aware store that summons ephemeral overlays from anywhere.
// `App.svelte` mounts a `<Popout>` per entry; consumers (rule editor /
// file viewer / agent confirm — Phase 5+) call `popouts.summon(...)`
// without needing to know about the wire-up.
//
// File extension `.svelte.ts` is REQUIRED so the Svelte 5 rune compiler
// processes `$state`. Plain `.ts` will not work.

/** Read-only summary of a notif tab passed to the NotifManager popout.
 *  Mirrors the load-bearing fields of TabBar's NotifTab without coupling the
 *  popouts module to TabBar. */
export interface NotifTabSummary {
  id: string;
  title: string;
  icon: string;
  enabled: boolean;
  detected: boolean;
}

/** Discriminated union of overlay content kinds. v1 ships `text` +
 *  `confirm` + `viewer` + `project-picker` + `notif-manager`; future kinds
 *  (component / snippet) gated to Phase 5+. */
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
    }
  | {
      /** Phase 6.5 — in-cockpit file viewer (§11). */
      kind: 'viewer';
      /** Project-relative path forwarded to fs_read_text / fs_write_text. */
      path: string;
    }
  | {
      /** Phase 6.7 — project picker (switch active project). */
      kind: 'project-picker';
      /** When set, the picker opens a new tab with the selected project
       *  instead of swapping the current tab's project (project-per-tab). */
      title?: string;
      onSelect?: (path: string) => void;
    }
  | {
      /** Phase 8.7h — notif tab manager (§10.7 capability-driven UI made
       *  user-discoverable). Right-click already toggles per-tab enabled,
       *  but the gesture isn't discoverable; this popout makes it explicit. */
      kind: 'notif-manager';
      /** Getter so the popout sees fresh state on every render — App.svelte
       *  reassigns `notifs` immutably on each toggle. */
      getTabs: () => NotifTabSummary[];
      /** Toggle the `enabled` field on the tab with this id. */
      onToggle: (id: string) => void;
      /** Reset all tabs to their defaults (enabled: true, except a
       *  capability-driven detected:false should still hide). */
      onReset: () => void;
    }
  | {
      /** Phase 8.7l — Settings menu. About / Updates / Project / Filesystem
       *  / Notifications sections in one panel. Self-contained; reads
       *  config via `config_get`, writes via `config_save`. The manual
       *  update-check button lives here alongside the auto-startup check
       *  status so users have a discoverable path to "is there a new
       *  version available right now?". */
      kind: 'settings';
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
