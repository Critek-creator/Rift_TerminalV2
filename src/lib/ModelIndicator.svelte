<script lang="ts">
  import { llmModels } from './llmModels.svelte';

  let model = $derived(
    llmModels.activeModelId
      ? llmModels.getModel(llmModels.activeModelId)
      : null,
  );

  let status = $derived(
    model ? (llmModels.processStatus[model.id] ?? 'stopped') : 'stopped',
  );

  let glyph = $derived(
    !model ? '—'
      : status === 'running' ? '◆'
        : status === 'starting' ? '◐'
          : status === 'error' ? '✕'
            : '◇',
  );

  let color = $derived(
    model?.color ? `var(${model.color}, var(--amber-faint))` : 'var(--amber-faint)',
  );

  let label = $derived(model?.short_id ?? '—');

  let title = $derived(
    model
      ? `${model.display_name} (${status})`
      : 'No model selected',
  );
</script>

{#if llmModels.enabled}
<span class="model-indicator" {title} style="color: {color}">
  <span class="glyph" aria-hidden="true">{glyph}</span>
  <span class="sid">{label}</span>
</span>
{/if}

<style>
  .model-indicator {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 10px;
    user-select: none;
    cursor: default;
  }

  .glyph {
    font-size: 8px;
    line-height: 1;
  }

  .sid {
    font-weight: 700;
    font-size: 9px;
    letter-spacing: 0.05em;
  }
</style>
