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
  import LlmChat from './LlmChat.svelte';
  import EnsembleChat from './EnsembleChat.svelte';

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

  // Track where the gesture that produced a backdrop `click` STARTED. A
  // genuine dismiss-click presses and releases on the backdrop itself. A
  // native CSS resize drag (`.card { resize: both }`) presses on the card's
  // BR resize widget and, on release, WebView2/Chromium synthesizes a
  // trailing `click` whose target is the backdrop (the pointer left the
  // card's old geometry) — which would otherwise dismiss the popout
  // mid-resize. Requiring the pointer-DOWN to have landed on the backdrop
  // discriminates the two, and as a bonus stops a text-selection drag that
  // begins in the card and releases over the backdrop from closing it.
  let pointerDownOnBackdrop = false;

  function onBackdropPointerDown(e: PointerEvent) {
    pointerDownOnBackdrop = e.target === e.currentTarget;
  }

  function onBackdropClick() {
    // Only the top overlay reacts to backdrop clicks — clicks on a
    // lower-stack backdrop would otherwise dismiss something the user
    // can't even see. Non-dismissible entries ignore backdrop too.
    if (!isTop) return;
    if (entry.dismissible === false) return;
    // Gesture started on the card (e.g. a resize drag) — not a dismiss.
    if (!pointerDownOnBackdrop) return;
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
      const frame = requestAnimationFrame(() => cardEl?.focus());
      return () => cancelAnimationFrame(frame);
    }
  });

  // Focus trap — Tab/Shift+Tab cycle within the dialog card (WCAG 2.1 §2.1.2).
  function trapFocus(e: KeyboardEvent) {
    if (e.key !== 'Tab' || !cardEl) return;
    const focusable = cardEl.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
  // Per-kind size persistence. The card has `resize: both`; we remember the
  // user's resized width+height in localStorage keyed by popout kind so it
  // sticks across re-opens (otherwise the default width is restored every
  // time). Settings also gets a wider default since the Models tab needs room.
  const SIZE_KEY = (kind: string) => `rift.popout.size.${kind}`;

  function loadSavedSize(kind: string): { w: number; h: number } | null {
    try {
      const raw = localStorage.getItem(SIZE_KEY(kind));
      if (!raw) return null;
      const v = JSON.parse(raw) as { w?: unknown; h?: unknown };
      if (typeof v.w === 'number' && typeof v.h === 'number' && v.w > 0 && v.h > 0) {
        return { w: v.w, h: v.h };
      }
    } catch { /* corrupt entry — ignore */ }
    return null;
  }

  // `userSize` is set only on a real user resize; until then the persisted
  // value (if any) is read from storage. Deriving keeps this reactive on
  // `entry` without the state_referenced_locally pitfall of seeding `$state`
  // from a prop.
  let userSize = $state<{ w: number; h: number } | null>(null);
  const savedSize = $derived(userSize ?? loadSavedSize(entry.content.kind));

  // Snapshot size on card pointer-down; if it changed by pointer-up, the user
  // dragged the resize widget — persist the new size. Guarding on a non-zero
  // start size means content-driven reflows (no preceding card pointer-down)
  // never get persisted.
  let resizeStartW = 0;
  let resizeStartH = 0;

  function onCardPointerDown() {
    if (!cardEl) return;
    resizeStartW = cardEl.offsetWidth;
    resizeStartH = cardEl.offsetHeight;
  }

  function persistSizeIfResized() {
    if (!cardEl || resizeStartW === 0) return;
    const w = cardEl.offsetWidth;
    const h = cardEl.offsetHeight;
    if ((w !== resizeStartW || h !== resizeStartH) && w > 0 && h > 0) {
      userSize = { w, h };
      try {
        localStorage.setItem(SIZE_KEY(entry.content.kind), JSON.stringify(savedSize));
      } catch { /* quota / disabled — non-fatal */ }
    }
    resizeStartW = 0;
    resizeStartH = 0;
  }

  $effect(() => {
    const onUp = () => persistSizeIfResized();
    window.addEventListener('pointerup', onUp);
    return () => window.removeEventListener('pointerup', onUp);
  });

  // Default width by kind. A persisted size, when present, overrides this.
  const cardWidth = $derived(
    entry.width
      ?? (entry.content.kind === 'settings' ? 'min(960px, 92vw)' : 'min(640px, 80vw)'),
  );

  // Card inline style: a persisted size wins (width + height); otherwise
  // width-only so height stays content-driven until the first manual resize.
  // Viewer keeps its CSS-class default size when nothing is persisted.
  const cardStyle = $derived(
    savedSize
      ? `width: ${savedSize.w}px; height: ${savedSize.h}px;`
      : (entry.content.kind === 'viewer' ? '' : `width: ${cardWidth};`),
  );

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
            : entry.content.kind === 'llm-chat'
              ? 'Router Prompt'
              : entry.content.kind === 'llm-ensemble'
                ? 'Ensemble Compare'
            : entry.content.title,
  );
</script>

<div
  class="backdrop"
  style="z-index: {zIndex};"
  onpointerdown={onBackdropPointerDown}
  onclick={onBackdropClick}
  role="presentation"
>
  <div
    class="card"
    class:is-viewer={entry.content.kind === 'viewer'}
    style={cardStyle}
    onpointerdown={onCardPointerDown}
    onclick={onCardClick}
    onkeydown={(e) => { trapFocus(e); onCardKey(e); }}
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
      class:card-body-settings={entry.content.kind === 'settings' || entry.content.kind === 'llm-chat' || entry.content.kind === 'llm-ensemble'}
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
      {:else if entry.content.kind === 'llm-chat'}
        <LlmChat popoutId={entry.id} modelOverride={entry.content.modelId} />
      {:else if entry.content.kind === 'llm-ensemble'}
        <EnsembleChat popoutId={entry.id} initialModelA={entry.content.modelA} initialModelB={entry.content.modelB} />
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
    padding: var(--space-12) var(--space-lg);
    border-bottom: 1px solid var(--border-active);
    flex-shrink: 0;
  }

  .card-title {
    font-size: var(--text-md);
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
    font-size: var(--text-xl);
    line-height: 1;
    cursor: pointer;
    padding: 0;
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    font-family: inherit;
    transition: color var(--duration-med) var(--ease-out), background var(--duration-med) var(--ease-out);
    flex-shrink: 0;
  }
  .card-close:hover {
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.12);
  }
  .card-close:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  /* Body: consistent padding, smooth scrolling with styled scrollbar */
  .card-body {
    padding: var(--space-lg);
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
    border-radius: var(--radius-sm);
  }
  .card-body::-webkit-scrollbar-thumb:hover { background: var(--amber-faint); }

  .text-body {
    white-space: pre-wrap;
    line-height: 1.5;
    font-size: var(--text-base);
    color: var(--amber-warm);
    margin: 0;
  }

  .card-actions {
    display: flex;
    gap: var(--space-8);
    justify-content: flex-end;
    margin-top: var(--space-lg);
  }

  .btn-cancel,
  .btn-confirm {
    padding: var(--space-xs) var(--space-12);
    font-family: inherit;
    font-size: var(--text-sm);
    letter-spacing: 0.08em;
    font-weight: 600;
    cursor: pointer;
    border-radius: var(--radius-md);
    transition: border-color var(--duration-med) var(--ease-out), color var(--duration-med) var(--ease-out), box-shadow var(--duration-med) var(--ease-out);
  }
  .btn-cancel:focus-visible,
  .btn-confirm:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
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
