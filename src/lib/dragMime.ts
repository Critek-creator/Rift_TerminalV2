/**
 * Custom MIME type isolating in-app HTML5 path drags from foreign drag sources.
 * Producer: Tree.svelte (file-tree nodes — Phase 6.6, HTML element).
 * Consumer: Terminal.svelte (drop target — HTML element).
 *
 * NOTE 2026-04-29 (Phase 8.7 finding): SVG `<g>` elements do NOT participate in
 * HTML5 drag-and-drop in WebView2 — the `draggable` IDL property is defined on
 * HTMLElement only. Setting `draggable="true"` as an HTML attribute on `<g>`
 * does NOT make the browser fire `dragstart`. IndexGraph.svelte therefore uses
 * the manual-gesture path below (RIFT_VAULT_DROP_EVENT) instead of HTML5 drag.
 */
export const TREE_PATH_MIME = 'application/x-rift-tree-path';

/**
 * Custom DOM event name for the manual-gesture vault-drop flow (Phase 8.7).
 *
 * IndexGraph.svelte tracks mousedown → mousemove → mouseup itself, then on
 * mouseup over `.terminal-host` it dispatches a `CustomEvent` of this type
 * with `{ detail: { path: string } }`. Terminal.svelte listens on its host
 * element and forwards the path to `pasteIntoTerminal()`.
 *
 * This sidesteps the HTML5-drag-on-SVG limitation; it is NOT a generic drag
 * channel — only IndexGraph dispatches, only Terminal listens.
 */
export const RIFT_VAULT_DROP_EVENT = 'rift-vault-drop';

/** Detail payload type for {@link RIFT_VAULT_DROP_EVENT} CustomEvents. */
export interface RiftVaultDropDetail {
  /** Vault file path (or vault id when no path is available). */
  path: string;
}

/**
 * MIME marker for notification-tab promote/demote drags between TabBar and
 * NotificationPane. Producer: TabBar tab dragstart (tab → pane promote) AND
 * NotificationPane drag-handle dragstart (pane → tab demote). Consumer:
 * TabBar strip drop zone (filters drops by this MIME). Both directions MUST
 * write this MIME or the drop handler silently rejects the gesture.
 */
export const NOTIF_TAB_MIME = 'application/x-rift-notif-tab';

/**
 * MIME marker for notification-event → terminal injection drags.
 * Producer: NotificationPane event-row inject control (HTML element, so HTML5
 * drag works — unlike the SVG vault case). Consumer: Terminal.svelte drop
 * target. The data value is the already-derived inject text (see
 * `eventInject.ts` `envelopeToInjectText`) — a single editable line, NEVER
 * a path to quote. Terminal pastes it verbatim (no path-quoting) + a trailing
 * space, so it lands as editable prompt text and is never auto-executed.
 */
export const RIFT_EVENT_MIME = 'application/x-rift-event';
