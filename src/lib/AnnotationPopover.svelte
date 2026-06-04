<script lang="ts">
  /**
   * AnnotationPopover — small popover for annotating a bus event.
   * Positioned near the clicked event row. Provides a text input
   * for a note and a tag chip selector.
   */

  import type { Envelope } from './bus';
  import {
    annotationStore,
    envelopeKey,
    ANNOTATION_TAGS,
    TAG_META,
    type AnnotationTag,
  } from './busAnnotations';

  interface Props {
    envelope: Envelope;
    /** Pixel offset from the top of the scroll container. */
    anchorY: number;
    onClose: () => void;
  }

  let { envelope, anchorY, onClose }: Props = $props();

  // Read existing annotation once on mount (popover is remounted per open).
  const initialAnnotation = (() => annotationStore.getForEnvelope(envelope))();
  let note = $state(initialAnnotation?.note ?? '');
  let selectedTags = $state<Set<AnnotationTag>>(
    new Set(initialAnnotation?.tags ?? [])
  );
  const existing = initialAnnotation;

  function toggleTag(tag: AnnotationTag): void {
    const next = new Set(selectedTags);
    if (next.has(tag)) next.delete(tag);
    else next.add(tag);
    selectedTags = next;
  }

  function handleSave(): void {
    const trimmed = note.trim();
    if (trimmed.length === 0 && selectedTags.size === 0) {
      // If both empty, remove annotation if it exists.
      annotationStore.remove(envelopeKey(envelope));
    } else {
      annotationStore.annotate(envelope, trimmed, Array.from(selectedTags));
    }
    onClose();
  }

  function handleRemove(): void {
    annotationStore.remove(envelopeKey(envelope));
    onClose();
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose();
    }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      handleSave();
    }
  }
</script>

<div class="popover-backdrop" role="presentation" onclick={onClose} onkeydown={handleKeydown}>
  <div
    class="popover"
    role="dialog"
    aria-label="Annotate event"
    aria-modal="true"
    tabindex="-1"
    style="top: {Math.max(8, anchorY - 40)}px"
    onclick={(e) => e.stopPropagation()}
    onkeydown={handleKeydown}
  >
    <div class="popover-header">
      <span class="popover-title">ANNOTATE EVENT</span>
      <span class="popover-kind">{envelope.category} / {envelope.kind}</span>
    </div>

    <div class="popover-body">
      <!-- svelte-ignore a11y_autofocus — modal opened by explicit user action,
           not page-load; focusing the note input is the expected landing point. -->
      <textarea
        class="note-input"
        placeholder="Add a note..."
        bind:value={note}
        rows={3}
        autofocus
      ></textarea>

      <div class="tag-section">
        <span class="tag-label">TAGS</span>
        <div class="tag-chips">
          {#each ANNOTATION_TAGS as tag (tag)}
            {@const meta = TAG_META[tag]}
            {@const active = selectedTags.has(tag)}
            <button
              type="button"
              class="tag-chip"
              class:active
              aria-pressed={active}
              style="--chip-color: {meta.cssVar}"
              onclick={() => toggleTag(tag)}
            >
              {meta.label}
            </button>
          {/each}
        </div>
      </div>
    </div>

    <div class="popover-footer">
      {#if existing}
        <button type="button" class="rift-btn rift-btn--danger rift-btn--sm" onclick={handleRemove}>
          REMOVE
        </button>
      {/if}
      <span class="spacer"></span>
      <button type="button" class="rift-btn rift-btn--ghost rift-btn--sm" onclick={onClose}>
        CANCEL
      </button>
      <button type="button" class="rift-btn rift-btn--primary rift-btn--sm" onclick={handleSave}>
        SAVE
      </button>
    </div>
  </div>
</div>

<style>
  .popover-backdrop {
    position: absolute;
    inset: 0;
    z-index: 50;
    background: rgba(0, 0, 0, 0.3);
  }

  .popover {
    position: absolute;
    right: 16px;
    left: 16px;
    max-width: 380px;
    margin-left: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border-active);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-flyout);
    z-index: 51;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .popover-header {
    padding: var(--space-8) var(--space-12);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .popover-title {
    color: var(--amber-bright);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.1em;
  }
  .popover-kind {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .popover-body {
    padding: var(--space-md) var(--space-12);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  .note-input {
    width: 100%;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    padding: var(--space-8);
    resize: vertical;
    min-height: 48px;
    line-height: 1.45;
  }
  .note-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .note-input:focus {
    outline: 2px solid transparent;
    border-color: var(--amber-dim);
  }
  .note-input:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -1px;
    border-color: var(--amber-dim);
  }

  .tag-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  .tag-label {
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
  }
  .tag-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px var(--space-8);
    background: transparent;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    cursor: pointer;
    transition: color var(--duration-base) ease-out, border-color var(--duration-base) ease-out, background var(--duration-base) ease-out;
  }
  .tag-chip:hover {
    border-color: var(--chip-color);
    color: var(--chip-color);
  }
  .tag-chip.active {
    border-color: var(--chip-color);
    color: var(--chip-color);
    background: var(--bg-amber-hover);
  }
  .tag-chip:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .popover-footer {
    padding: var(--space-8) var(--space-12);
    border-top: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .spacer {
    flex: 1;
  }
</style>
