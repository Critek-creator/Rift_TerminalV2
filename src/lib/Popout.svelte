<script lang="ts">
  // Phase 3.5b — overlay shell. Renders one `PopoutEntry` from the
  // global `popouts` store. App.svelte renders the stack via `{#each}`
  // and passes per-entry `isTop` + `stackIndex` so:
  //   - only the top entry responds to Esc / backdrop click
  //   - higher entries paint above lower ones via z-index
  //
  // Visual style matches the rift aesthetic: matte black backdrop,
  // amber-bordered card, JetBrains Mono inherit, soft fade-in.

  import { popouts, type PopoutEntry } from './popouts.svelte';
  import Viewer from './Viewer.svelte';

  interface Props {
    entry: PopoutEntry;
    isTop: boolean;
    stackIndex: number;
  }

  let { entry, isTop, stackIndex }: Props = $props();

  // Esc-listener — only the top dismissible overlay reacts. Re-attaches
  // whenever isTop / dismissible flips so the right entry always owns
  // the keystroke.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isTop && entry.dismissible !== false) {
        e.preventDefault();
        popouts.dismiss(entry.id);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function dismissSelf() {
    popouts.dismiss(entry.id);
  }

  function onBackdropClick() {
    // Only the top overlay reacts to backdrop clicks — clicks on a
    // lower-stack backdrop would otherwise dismiss something the user
    // can't even see. Non-dismissible entries ignore backdrop too.
    if (!isTop) return;
    if (entry.dismissible === false) return;
    popouts.dismiss(entry.id);
  }

  function onCardClick(e: MouseEvent) {
    // Stop the click from bubbling to the backdrop and triggering dismiss.
    e.stopPropagation();
  }

  function onCardKey(e: KeyboardEvent) {
    // Card-level keyboard handler — paired with `onCardClick` so a11y
    // lints accept the interactive role. Esc is owned by the window-level
    // listener (top-only); we stop propagation here to keep the card a
    // self-contained focus boundary (any future focusable children inside
    // the card body still receive their own key events first).
    e.stopPropagation();
  }

  function onConfirm() {
    if (entry.content.kind !== 'confirm') return;
    entry.content.onConfirm?.();
    popouts.dismiss(entry.id);
  }

  function onCancel() {
    if (entry.content.kind !== 'confirm') return;
    entry.content.onCancel?.();
    popouts.dismiss(entry.id);
  }

  // Per-entry z-index — base 1000, +10 per stack level so each new
  // overlay paints above the prior one without colliding with app chrome.
  const zIndex = $derived(1000 + stackIndex * 10);
  const cardWidth = $derived(entry.width ?? 'min(640px, 80vw)');

  /** Display title for the card header — viewer uses the basename of path. */
  const cardTitle = $derived(
    entry.content.kind === 'viewer'
      ? (entry.content.path.split('/').at(-1) ?? entry.content.path)
      : entry.content.title,
  );
</script>

<div
  class="backdrop"
  style="z-index: {zIndex};"
  onclick={onBackdropClick}
  role="presentation"
>
  <div
    class="card"
    style="width: {cardWidth};"
    onclick={onCardClick}
    onkeydown={onCardKey}
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-labelledby="popout-title-{entry.id}"
  >
    <header class="card-header">
      <h2 class="card-title" id="popout-title-{entry.id}">{cardTitle}</h2>
      {#if entry.dismissible !== false}
        <button
          type="button"
          class="card-close"
          onclick={dismissSelf}
          aria-label="close"
        >×</button>
      {/if}
    </header>

    <div class="card-body" class:card-body-viewer={entry.content.kind === 'viewer'}>
      {#if entry.content.kind === 'text'}
        <p class="text-body">{entry.content.body}</p>
      {:else if entry.content.kind === 'confirm'}
        <p class="text-body">{entry.content.body}</p>
        <div class="card-actions">
          <button type="button" class="btn-cancel" onclick={onCancel}>
            {entry.content.cancelLabel ?? 'Cancel'}
          </button>
          <button type="button" class="btn-confirm" onclick={onConfirm}>
            {entry.content.confirmLabel ?? 'Confirm'}
          </button>
        </div>
      {:else if entry.content.kind === 'viewer'}
        <!-- Phase 6.5: Viewer owns its own scrolling, header details, error
             state, and keyboard shortcuts (Ctrl+E / Ctrl+S / Esc). The
             popout chrome provides the backdrop, close-X, and title bar. -->
        <Viewer path={entry.content.path} popoutId={entry.id} />
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: grid;
    place-items: center;
    animation: popout-fade-in 120ms ease-out;
  }

  .card {
    background: var(--bg-elevated);
    border: 1px solid var(--amber-bright);
    box-shadow: var(--glow-amber-faint), 0 8px 32px rgba(0, 0, 0, 0.5);
    color: var(--amber-warm);
    font-family: inherit;
    max-width: 90vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    animation: popout-card-in 160ms cubic-bezier(0.2, 0.7, 0.3, 1);
  }

  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .card-title {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    margin: 0;
  }

  .card-close {
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    padding: 2px 6px;
    font-family: inherit;
  }
  .card-close:hover {
    color: var(--term-red);
  }

  .card-body {
    padding: 16px;
    overflow: auto;
  }
  /* Viewer manages its own padding + scrolling — strip the card padding. */
  .card-body-viewer {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  .card-body::-webkit-scrollbar { width: 5px; }
  .card-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .text-body {
    white-space: pre-wrap;
    line-height: 1.5;
    font-size: 12px;
    color: var(--amber-warm);
    margin: 0;
  }

  .card-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 16px;
  }

  .btn-cancel,
  .btn-confirm {
    padding: 4px 12px;
    font-family: inherit;
    font-size: 11px;
    letter-spacing: 0.08em;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-cancel {
    background: transparent;
    border: 1px solid var(--border-subtle);
    color: var(--amber-dim);
  }
  .btn-cancel:hover {
    border-color: var(--amber-faint);
    color: var(--amber-warm);
  }
  .btn-confirm {
    background: var(--amber-bright);
    border: 1px solid var(--amber-bright);
    color: var(--bg-base);
  }
  .btn-confirm:hover {
    box-shadow: var(--glow-amber-strong);
  }

  @keyframes popout-fade-in {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  @keyframes popout-card-in {
    from { opacity: 0; transform: scale(0.98) translateY(4px); }
    to   { opacity: 1; transform: scale(1) translateY(0); }
  }
</style>
