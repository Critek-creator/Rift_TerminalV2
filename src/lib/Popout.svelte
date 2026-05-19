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
  import ProjectPicker from './ProjectPicker.svelte';
  import Viewer from './Viewer.svelte';
  import NotifManager from './NotifManager.svelte';
  import SettingsPanel from './SettingsPanel.svelte';

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

  // Focus management — move focus into the dialog card when it becomes the
  // top entry. This satisfies WCAG 2.1 SC 2.1.2 (No Keyboard Trap) and the
  // ARIA dialog pattern requirement that focus moves to the dialog on open.
  // tabindex="-1" on the card makes it programmatically focusable without
  // placing it in the natural tab order.
  let cardEl = $state<HTMLElement | null>(null);
  $effect(() => {
    if (isTop && cardEl) {
      // Defer one microtask so the card is fully painted before focus moves.
      const frame = requestAnimationFrame(() => cardEl?.focus());
      return () => cancelAnimationFrame(frame);
    }
  });
  const cardWidth = $derived(entry.width ?? 'min(640px, 80vw)');

  /** Display title for the card header. */
  const cardTitle = $derived(
    entry.content.kind === 'viewer'
      ? (entry.content.path.split('/').at(-1) ?? entry.content.path)
      : entry.content.kind === 'project-picker'
        ? 'Switch Project'
        : entry.content.kind === 'notif-manager'
          ? 'Manage Notification Tabs'
          : entry.content.kind === 'settings'
            ? 'Settings'
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
    class:is-viewer={entry.content.kind === 'viewer'}
    style={entry.content.kind === 'viewer' ? '' : `width: ${cardWidth};`}
    onclick={onCardClick}
    onkeydown={onCardKey}
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-labelledby="popout-title-{entry.id}"
    bind:this={cardEl}
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

    <div
      class="card-body"
      class:card-body-viewer={entry.content.kind === 'viewer'}
      class:card-body-picker={entry.content.kind === 'project-picker'}
      class:card-body-manager={entry.content.kind === 'notif-manager'}
      class:card-body-settings={entry.content.kind === 'settings'}
    >
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
      {:else if entry.content.kind === 'project-picker'}
        <!-- Phase 6.7: ProjectPicker owns its own keyboard handling (Esc
             stopPropagation per pr003 popout-keydown-bubble-collision). -->
        <ProjectPicker popoutId={entry.id} onSelect={entry.content.onSelect} />
      {:else if entry.content.kind === 'notif-manager'}
        <!-- Phase 8.7h: notif tab manager — App.svelte passes a getTabs
             getter + onToggle/onReset callbacks; NotifManager is stateless. -->
        <NotifManager
          popoutId={entry.id}
          getTabs={entry.content.getTabs}
          onToggle={entry.content.onToggle}
          onReset={entry.content.onReset}
        />
      {:else if entry.content.kind === 'settings'}
        <!-- Phase 8.7l: Settings panel — self-contained; reads RiftConfig
             via config_get and saves via config_save per-section. -->
        <SettingsPanel popoutId={entry.id} />
      {/if}
    </div>
  </div>
</div>

<style>
  /* Backdrop: warm-tinted dark overlay with subtle blur for depth */
  .backdrop {
    position: fixed;
    inset: 0;
    /* Slightly warm the black — hint of amber in the tint */
    background: rgba(4, 3, 1, 0.78);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    display: grid;
    place-items: center;
    animation: popout-fade-in 120ms ease-out;
  }

  /* Card: depth shadow + faint amber outer glow + top-edge highlight */
  .card {
    background: var(--bg-elevated);
    /* Top border slightly brighter to create a lifted edge highlight */
    border: 1px solid var(--amber-bright);
    border-top-color: rgba(255, 200, 64, 0.55);
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.6),
      0 0 1px rgba(255, 168, 38, 0.2),
      var(--glow-amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    max-width: 90vw;
    max-height: 90vh;
    min-width: 320px;
    min-height: 200px;
    display: flex;
    flex-direction: column;
    /* Phase 8.7g.5 — resize handle (BR corner). Works with display:flex
       because the browser's native resize widget operates on the box
       layout, not the inner flex layout. overflow:hidden is required for
       the resize widget to render. */
    resize: both;
    overflow: hidden;
    animation: popout-card-in 150ms cubic-bezier(0.18, 0.72, 0.28, 1);
  }
  /* Viewer popout — comfortable default so file content is readable
     without immediate resize. User can still drag the BR corner up/down. */
  .card.is-viewer {
    width: min(1024px, 90vw);
    height: min(720px, 85vh);
    min-width: 480px;
    min-height: 320px;
  }

  /* Header: slightly more prominent separator from body */
  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 18px;
    border-bottom: 1px solid var(--border-active);
    flex-shrink: 0;
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

  /* Close button: 28×28 click target, smooth red hover */
  .card-close {
    background: transparent;
    border: none;
    color: var(--amber-faint);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    padding: 0;
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 2px;
    font-family: inherit;
    transition: color 0.18s, background 0.18s;
    flex-shrink: 0;
  }
  .card-close:hover {
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.12);
  }

  /* Body: consistent 18px padding, smooth scrolling with styled scrollbar */
  .card-body {
    padding: 18px;
    overflow: auto;
    scroll-behavior: smooth;
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
  /* ProjectPicker manages its own padding — strip the card padding. */
  .card-body-picker {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  /* Phase 8.7h NotifManager — same pattern: own padding, fills card body. */
  .card-body-manager {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  /* Phase 8.7l Settings — same pattern: own padding, fills card body. */
  .card-body-settings {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  /* Styled scrollbar — amber-tinted track on the card body */
  .card-body::-webkit-scrollbar { width: 5px; }
  .card-body::-webkit-scrollbar-track { background: var(--bg-base); }
  .card-body::-webkit-scrollbar-thumb {
    background: var(--border-active);
    border-radius: 2px;
  }
  .card-body::-webkit-scrollbar-thumb:hover { background: var(--amber-faint); }

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
    transition: border-color 0.15s, color 0.15s, box-shadow 0.15s;
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
  /* Card entry: scale from 0.97 to 1 with slight upward travel */
  @keyframes popout-card-in {
    from { opacity: 0; transform: scale(0.97) translateY(6px); }
    to   { opacity: 1; transform: scale(1)    translateY(0);   }
  }
</style>
