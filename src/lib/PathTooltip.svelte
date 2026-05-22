<script lang="ts">
  interface FilePreview {
    exists: boolean;
    size_bytes: number;
    modified_iso: string;
    language_hint: string;
    preview_lines: string[];
    is_binary: boolean;
  }

  interface Props {
    x: number;
    y: number;
    visible: boolean;
    preview: FilePreview | null;
    filename?: string;
  }

  let { x, y, visible, preview, filename = '' }: Props = $props();

  function humanizeBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }

  function relativeTime(isoStr: string): string {
    const now = Date.now();
    const then = new Date(isoStr).getTime();
    if (isNaN(then)) return isoStr;
    const diffMs = now - then;
    if (diffMs < 60_000) return 'just now';
    if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
    if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`;
    return `${Math.floor(diffMs / 86_400_000)}d ago`;
  }

  let adjustedPos = $derived.by(() => {
    const maxW = 400;
    const maxH = 250;
    const viewW = typeof window !== 'undefined' ? window.innerWidth : 1920;
    const viewH = typeof window !== 'undefined' ? window.innerHeight : 1080;

    let adjX = x;
    let adjY = y;

    if (x + maxW > viewW - 16) adjX = x - maxW - 8;
    if (adjX < 8) adjX = 8;
    if (y + maxH > viewH - 16) adjY = y - maxH - 8;
    if (adjY < 8) adjY = 8;

    return { x: adjX, y: adjY };
  });
</script>

{#if visible && preview}
  <div
    class="path-tooltip"
    class:visible
    style="left: {adjustedPos.x}px; top: {adjustedPos.y}px;"
    role="tooltip"
  >
    {#if !preview.exists}
      <div class="tooltip-header">
        <span class="tooltip-filename">{filename || 'Unknown'}</span>
      </div>
      <div class="tooltip-not-found">File not found</div>
    {:else}
      <div class="tooltip-header">
        <span class="tooltip-filename">{filename || 'File'}</span>
        {#if preview.language_hint}
          <span class="tooltip-lang">{preview.language_hint}</span>
        {/if}
      </div>

      <div class="tooltip-meta">
        <span>{humanizeBytes(preview.size_bytes)}</span>
        <span class="tooltip-sep">|</span>
        <span>{relativeTime(preview.modified_iso)}</span>
      </div>

      {#if preview.is_binary}
        <div class="tooltip-binary">Binary file</div>
      {:else if preview.preview_lines.length > 0}
        <div class="tooltip-preview">
          {#each preview.preview_lines.slice(0, 5) as line, i}
            <div class="preview-line">
              <span class="line-num">{i + 1}</span>
              <span class="line-text">{line}</span>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .path-tooltip {
    position: fixed;
    z-index: 1000;
    max-width: 400px;
    max-height: 250px;
    background: var(--bg-elevated);
    border: 1px solid var(--amber-dim);
    border-radius: var(--radius-md);
    padding: var(--space-sm);
    font-family: var(--font-family), monospace;
    box-shadow: var(--glow-amber-faint);
    pointer-events: none;
    opacity: 0;
    transition: opacity 150ms var(--ease-out);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .path-tooltip.visible {
    opacity: 1;
  }

  .tooltip-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }

  .tooltip-filename {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--amber-bright);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .tooltip-lang {
    font-size: var(--text-2xs, 9px);
    color: var(--bg-base);
    background: var(--amber-faint);
    padding: 1px 4px;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    flex-shrink: 0;
  }

  .tooltip-meta {
    display: flex;
    gap: var(--space-xs);
    font-size: var(--text-xs);
    color: var(--amber-faint);
  }

  .tooltip-sep {
    opacity: 0.4;
  }

  .tooltip-not-found {
    font-size: var(--text-sm);
    color: var(--term-red);
    font-style: italic;
  }

  .tooltip-binary {
    font-size: var(--text-xs);
    color: var(--amber-faint);
    font-style: italic;
  }

  .tooltip-preview {
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    padding: var(--space-xs);
    overflow: hidden;
    max-height: 130px;
  }

  .preview-line {
    display: flex;
    gap: var(--space-sm);
    line-height: 1.4;
    font-size: var(--text-xs);
  }

  .line-num {
    color: var(--amber-faint);
    opacity: 0.5;
    min-width: 18px;
    text-align: right;
    flex-shrink: 0;
    user-select: none;
  }

  .line-text {
    color: var(--amber-warm, #F0A030);
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
