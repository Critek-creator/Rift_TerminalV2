<script lang="ts">
  // ModelIndicator.svelte — LOCAL (ensemble-router) model status chip.
  //
  // Lives in the TitleBar (top chrome), deliberately separate from the
  // StatusLine's core Claude-model info. Shows an at-a-glance status LIGHT
  // (green = a local model is loaded + serving · amber = starting · red = none
  // loaded), the active local model's short id, and measured GPU VRAM. Clicking
  // it opens the model-swap palette so load/swap is one click from the top bar.

  import { invoke } from '@tauri-apps/api/core';
  import { llmModels } from './llmModels.svelte';
  import { popouts } from './popouts.svelte';

  let model = $derived(
    llmModels.activeModelId
      ? llmModels.getModel(llmModels.activeModelId)
      : null,
  );

  let status = $derived(
    model ? (llmModels.processStatus[model.id] ?? 'stopped') : 'stopped',
  );

  let isLocalRunning = $derived(
    model?.hosting?.mode === 'local' && status === 'running',
  );

  // Status light: green when a local model is up, amber while it spins up, red
  // when nothing local is loaded (the "disabled" state the user wants to see).
  let lightClass = $derived(
    isLocalRunning ? 'on' : status === 'starting' ? 'starting' : 'off',
  );

  // --- Measured VRAM (gpu_vram_used_mb) ------------------------------------
  // Poll the GPU's actual used VRAM only while a local model is the active route
  // AND running. Total capacity never changes, so it's fetched once and cached.
  // Values are GPU-wide (desktop + every process), per the readout's title.
  let vramUsedMb = $state<number | null>(null);
  let vramTotalMb = $state<number | null>(null);

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

  let label = $derived(model?.short_id || model?.display_name || '—');

  let statusWord = $derived(
    !model ? 'no local model'
      : status === 'running' ? 'running'
        : status === 'starting' ? 'starting'
          : status === 'error' ? 'error'
            : 'stopped',
  );

  let title = $derived(
    model
      ? `Local model: ${model.display_name} — ${statusWord}. Click to load / swap.`
      : 'No local model loaded. Click to load / swap.',
  );

  function openModelSwap(): void {
    window.dispatchEvent(new CustomEvent('rift:open-model-swap'));
  }

  function openChat(): void {
    popouts.summon({ content: { kind: 'llm-chat' }, width: 'min(720px, 85vw)' });
  }
</script>

{#if llmModels.enabled}
  <span class="model-indicator-group">
    <button type="button" class="model-chip" {title} onclick={openModelSwap} aria-label={title}>
      <span class="light {lightClass}" aria-hidden="true"></span>
      <span class="sid">{label}</span>
      {#if vramLabel}
        <span class="vram" title="GPU VRAM in use — whole GPU, not just this model">{vramLabel}</span>
      {/if}
      <span class="swap" aria-hidden="true">⇄</span>
    </button>
    <button
      type="button"
      class="chat-btn"
      onclick={openChat}
      title="LLM Chat"
      aria-label="LLM Chat"
    >💬</button>
  </span>
{:else}
  <span
    class="model-chip model-chip--disabled"
    title="Configure a local model in Settings to enable LLM chat"
    aria-label="Configure a local model in Settings to enable LLM chat"
    aria-disabled="true"
  >
    <span class="light off" aria-hidden="true"></span>
    <span class="sid">NO LOCAL MODEL</span>
  </span>
{/if}

<style>
  .model-indicator-group {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }

  .model-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--space-24);
    padding: 0 var(--space-sm);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    color: var(--amber-warm);
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    cursor: pointer;
    user-select: none;
    transition: background var(--duration-med) var(--ease-out), border-color var(--duration-med) var(--ease-out), color var(--duration-med) var(--ease-out);
  }
  .model-chip:hover {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.08);
    border-color: var(--amber-dim);
  }
  .model-chip:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  /* Empty-state chip — shown when llmModels.enabled is false.
     Non-interactive: pointer-events off, muted amber-faint palette,
     same geometry as the active chip so the TitleBar slot never collapses. */
  .model-chip--disabled {
    cursor: default;
    pointer-events: none;
    color: var(--amber-faint);
    opacity: 0.55;
  }

  /* Status light — the at-a-glance load indicator. */
  .light {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--term-red);
    box-shadow: 0 0 5px rgba(255, 72, 72, 0.7);
  }
  .light.on {
    background: var(--term-green);
    box-shadow: 0 0 6px rgba(79, 232, 85, 0.8);
  }
  .light.starting {
    background: var(--amber-bright);
    box-shadow: 0 0 6px rgba(255, 200, 64, 0.8);
    animation: light-pulse 1.2s var(--ease-out) infinite;
  }
  .light.off {
    background: var(--term-red);
    box-shadow: 0 0 5px rgba(255, 72, 72, 0.6);
  }
  @keyframes light-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .sid {
    font-weight: 700;
    letter-spacing: 0.05em;
  }
  .vram {
    color: var(--amber-faint);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.03em;
    padding-left: var(--space-xs);
    border-left: 1px solid rgba(168, 120, 48, 0.25);
  }
  .swap {
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    padding-left: 2px;
  }
  .model-chip:hover .swap { color: var(--amber-warm); }

  .chat-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--space-24);
    width: var(--space-24);
    font-size: var(--text-2xs);
    color: var(--amber-warm);
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md, 4px);
    cursor: pointer;
    user-select: none;
    transition: background var(--duration-med) var(--ease-out), border-color var(--duration-med) var(--ease-out), color var(--duration-med) var(--ease-out);
  }
  .chat-btn:hover {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.08);
    border-color: var(--amber-dim);
  }
  .chat-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .light.starting { animation: none; }
  }
</style>
