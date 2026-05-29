<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { llmModels } from './llmModels.svelte';

  let model = $derived(
    llmModels.activeModelId
      ? llmModels.getModel(llmModels.activeModelId)
      : null,
  );

  let status = $derived(
    model ? (llmModels.processStatus[model.id] ?? 'stopped') : 'stopped',
  );

  // --- Measured VRAM (gpu_vram_used_mb) ------------------------------------
  // Poll the GPU's actual used VRAM (nvidia-smi memory.used) only while a local
  // model is the active route AND running. Total capacity changes never, so it's
  // fetched once and cached. Values are GPU-wide (desktop + every process), not
  // attributable solely to this model — the readout's title says so.
  let vramUsedMb = $state<number | null>(null);
  let vramTotalMb = $state<number | null>(null);

  let isLocalRunning = $derived(
    model?.hosting?.mode === 'local' && status === 'running',
  );

  $effect(() => {
    if (!isLocalRunning) {
      vramUsedMb = null;
      return;
    }
    let cancelled = false;
    let inFlight = false;
    async function poll() {
      if (inFlight) return;
      inFlight = true;
      try {
        if (vramTotalMb == null) {
          const t = await invoke<number | null>('gpu_vram_mb').catch(() => null);
          if (!cancelled && t && t > 0) vramTotalMb = t;
        }
        const u = await invoke<number | null>('gpu_vram_used_mb').catch(() => null);
        if (!cancelled) vramUsedMb = u && u > 0 ? u : null;
      } finally {
        inFlight = false;
      }
    }
    void poll();
    const id = setInterval(() => void poll(), 5000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  });

  let vramLabel = $derived(
    vramUsedMb == null
      ? ''
      : vramTotalMb != null
        ? `${(vramUsedMb / 1024).toFixed(1)}/${(vramTotalMb / 1024).toFixed(0)}G`
        : `${(vramUsedMb / 1024).toFixed(1)}G`,
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
  {#if vramLabel}
    <span class="vram" title="GPU VRAM in use — whole GPU, not just this model">{vramLabel}</span>
  {/if}
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

  .vram {
    color: var(--amber-faint, #A87830);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.03em;
    padding-left: 3px;
    border-left: 1px solid rgba(168, 120, 48, 0.25);
  }
</style>
