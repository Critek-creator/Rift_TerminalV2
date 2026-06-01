<script lang="ts">
  import { onDestroy } from 'svelte';
  import { invoke, Channel } from '@tauri-apps/api/core';
  import { llmModels } from './llmModels.svelte';
  import { popouts } from './popouts.svelte';

  interface Props {
    popoutId: number;
    initialModelA?: string;
    initialModelB?: string;
  }

  let { popoutId, initialModelA, initialModelB }: Props = $props();

  // ---------------------------------------------------------------------------
  // Model selection state
  // ---------------------------------------------------------------------------

  // svelte-ignore state_referenced_locally
  let modelAId = $state<string | undefined>(initialModelA);
  // svelte-ignore state_referenced_locally
  let modelBId = $state<string | undefined>(initialModelB);
  let pickerOpenA = $state(false);
  let pickerOpenB = $state(false);

  let modelA = $derived(modelAId ? llmModels.getModel(modelAId) : null);
  let modelB = $derived(modelBId ? llmModels.getModel(modelBId) : null);

  // ---------------------------------------------------------------------------
  // Chat state
  // ---------------------------------------------------------------------------

  let inputText = $state('');
  let sending = $state(false);
  let error = $state('');
  let critiqueEnabled = $state(false);

  /** Streaming content for pane A */
  let streamA = $state('');
  /** Streaming content for pane B */
  let streamB = $state('');
  /** Token count during streaming */
  let streamTokensA = $state(0);
  let streamTokensB = $state(0);

  /** Elapsed time counters (ms) */
  let elapsedA = $state(0);
  let elapsedB = $state(0);
  let timerA: ReturnType<typeof setInterval> | null = null;
  let timerB: ReturnType<typeof setInterval> | null = null;
  let startTime = $state(0);

  /** Whether each pane has finished streaming */
  let doneA = $state(false);
  let doneB = $state(false);

  /** Final metadata after invoke resolves */
  interface PaneResult {
    model_id: string;
    model_short_id: string;
    content: string;
    tokens_in: number;
    tokens_out: number;
    latency_ms: number;
    cost_usd: number;
    error: string | null;
  }

  let resultA = $state<PaneResult | null>(null);
  let resultB = $state<PaneResult | null>(null);
  let critiqueText = $state<string | null>(null);
  let totalCost = $state(0);

  /** Critique panel expanded */
  let critiqueExpanded = $state(true);

  /** History of past exchanges */
  interface HistoryEntry {
    prompt: string;
    resultA: PaneResult | null;
    resultB: PaneResult | null;
    critique: string | null;
    totalCost: number;
  }

  let history = $state<HistoryEntry[]>([]);

  let paneAEl: HTMLDivElement = $state(undefined!);
  let paneBEl: HTMLDivElement = $state(undefined!);

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function scrollPaneA() {
    requestAnimationFrame(() => { if (paneAEl) paneAEl.scrollTop = paneAEl.scrollHeight; });
  }
  function scrollPaneB() {
    requestAnimationFrame(() => { if (paneBEl) paneBEl.scrollTop = paneBEl.scrollHeight; });
  }

  function formatCost(usd: number): string {
    if (usd === 0) return '$0';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    if (usd < 1) return `$${usd.toFixed(3)}`;
    return `$${usd.toFixed(2)}`;
  }

  function clearTimers() {
    if (timerA) { clearInterval(timerA); timerA = null; }
    if (timerB) { clearInterval(timerB); timerB = null; }
  }

  onDestroy(() => { clearTimers(); });

  // ---------------------------------------------------------------------------
  // Send
  // ---------------------------------------------------------------------------

  async function send() {
    const text = inputText.trim();
    if (!text || sending) return;
    if (!modelAId || !modelBId) {
      error = 'Select a model for both panes before sending.';
      return;
    }
    if (modelAId === modelBId) {
      // Allow same model — user might want to see variance, but warn once
    }

    error = '';
    sending = true;

    // Archive previous exchange if any
    if (resultA || resultB) {
      history = [...history, {
        prompt: inputText,
        resultA,
        resultB,
        critique: critiqueText,
        totalCost,
      }];
    }

    // Reset pane state
    streamA = '';
    streamB = '';
    streamTokensA = 0;
    streamTokensB = 0;
    elapsedA = 0;
    elapsedB = 0;
    doneA = false;
    doneB = false;
    resultA = null;
    resultB = null;
    critiqueText = null;
    totalCost = 0;

    const prompt = text;
    inputText = '';

    // Start latency timers
    startTime = Date.now();
    timerA = setInterval(() => { if (!doneA) elapsedA = Date.now() - startTime; }, 250);
    timerB = setInterval(() => { if (!doneB) elapsedB = Date.now() - startTime; }, 250);

    try {
      type StreamChunk = { text: string; is_final: boolean; tokens_so_far: number };

      const onChunkA: Channel<StreamChunk> = new Channel();
      const onChunkB: Channel<StreamChunk> = new Channel();

      onChunkA.onmessage = (chunk: StreamChunk) => {
        streamA += chunk.text;
        streamTokensA = chunk.tokens_so_far;
        if (chunk.is_final) {
          doneA = true;
          elapsedA = Date.now() - startTime;
          if (timerA) { clearInterval(timerA); timerA = null; }
        }
        scrollPaneA();
      };

      onChunkB.onmessage = (chunk: StreamChunk) => {
        streamB += chunk.text;
        streamTokensB = chunk.tokens_so_far;
        if (chunk.is_final) {
          doneB = true;
          elapsedB = Date.now() - startTime;
          if (timerB) { clearInterval(timerB); timerB = null; }
        }
        scrollPaneB();
      };

      const result = await invoke<{
        results: PaneResult[];
        task_type: string;
        critique: string | null;
        total_cost_usd: number;
      }>('llm_ensemble', {
        modelIds: [modelAId, modelBId],
        prompt,
        critique: critiqueEnabled,
        onChunkA,
        onChunkB,
      });

      // Populate final results
      if (result.results.length >= 1) {
        resultA = result.results[0];
        streamA = result.results[0].content;
      }
      if (result.results.length >= 2) {
        resultB = result.results[1];
        streamB = result.results[1].content;
      }
      critiqueText = result.critique;
      totalCost = result.total_cost_usd;

    } catch (err) {
      error = String(err);
    } finally {
      sending = false;
      doneA = true;
      doneB = true;
      clearTimers();
      scrollPaneA();
      scrollPaneB();
    }
  }

  // ---------------------------------------------------------------------------
  // Pickers
  // ---------------------------------------------------------------------------

  function pickModelA(id: string) {
    modelAId = id;
    pickerOpenA = false;
  }

  function pickModelB(id: string) {
    modelBId = id;
    pickerOpenB = false;
  }

  function onPickerBackdropA(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('picker-backdrop')) {
      pickerOpenA = false;
    }
  }

  function onPickerBackdropB(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('picker-backdrop')) {
      pickerOpenB = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Keyboard
  // ---------------------------------------------------------------------------

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
    if (e.key === 'Escape') {
      if (pickerOpenA) { pickerOpenA = false; e.stopPropagation(); return; }
      if (pickerOpenB) { pickerOpenB = false; e.stopPropagation(); return; }
      e.stopPropagation();
      popouts.dismiss(popoutId);
    }
  }
</script>

<div class="ensemble-chat">
  <!-- Header -->
  <div class="ensemble-header">
    <div class="header-left">
      <span class="ensemble-label">ENSEMBLE</span>
      <span class="model-tag" style="border-color: {modelA?.color ? `var(${modelA.color})` : 'var(--amber-faint)'}; color: {modelA?.color ? `var(${modelA.color})` : 'var(--amber-faint)'}">{modelA?.short_id ?? '---'}</span>
      <span class="versus">vs</span>
      <span class="model-tag" style="border-color: {modelB?.color ? `var(${modelB.color})` : 'var(--amber-faint)'}; color: {modelB?.color ? `var(${modelB.color})` : 'var(--amber-faint)'}">{modelB?.short_id ?? '---'}</span>
    </div>
    <div class="header-right">
      {#if totalCost > 0}
        <span class="cost-badge">{formatCost(totalCost)}</span>
      {/if}
      <label class="critique-toggle">
        <input type="checkbox" bind:checked={critiqueEnabled} disabled={sending} />
        <span>Critique</span>
      </label>
      {#if sending}
        <span class="sending-indicator">generating...</span>
      {/if}
    </div>
  </div>

  <!-- Split panes -->
  <div class="split-container">
    <!-- Pane A -->
    <div class="pane" class:pane-error={resultA?.error}>
      <div class="pane-header">
        <div class="model-selector-wrap">
          <button
            type="button"
            class="model-badge"
            style="border-color: {modelA?.color ? `var(${modelA.color})` : 'var(--amber-faint)'}; color: {modelA?.color ? `var(${modelA.color})` : 'var(--amber-faint)'}"
            onclick={() => { pickerOpenA = !pickerOpenA; }}
            title="Select model A"
          >
            {modelA?.short_id ?? 'Model A'}
            <span class="badge-caret">{pickerOpenA ? '▴' : '▾'}</span>
          </button>
          {#if pickerOpenA}
            <div class="picker-backdrop" role="presentation" onclick={onPickerBackdropA}>
              <div class="model-picker">
                {#each llmModels.availableModels as m (m.id)}
                  <button
                    type="button"
                    class="picker-item"
                    class:active={m.id === modelAId}
                    onclick={() => pickModelA(m.id)}
                  >
                    <span class="status-dot" style="background: {llmModels.modelStatusColor(m.id)}"></span>
                    <span class="picker-short" style="color: {m.color ? `var(${m.color})` : 'var(--amber-faint)'}">{m.short_id}</span>
                    <span class="picker-name">{m.display_name}</span>
                  </button>
                {/each}
                {#if llmModels.availableModels.length === 0}
                  <div class="picker-empty">No models configured</div>
                {/if}
              </div>
            </div>
          {/if}
        </div>
        <span class="pane-meta">
          {#if sending && !doneA}
            <span class="latency-live">{elapsedA}ms</span>
            <span class="token-count">{streamTokensA} tok</span>
          {:else if resultA}
            <span class="latency-final">{resultA.latency_ms}ms</span>
            <span class="token-count">{resultA.tokens_in} in / {resultA.tokens_out} out</span>
            {#if resultA.cost_usd > 0}
              <span class="pane-cost">{formatCost(resultA.cost_usd)}</span>
            {/if}
          {/if}
        </span>
      </div>
      <div class="pane-content" bind:this={paneAEl}>
        {#if !streamA && !sending}
          <div class="pane-empty">
            {modelA ? `Ready: ${modelA.display_name}` : 'Select a model'}
          </div>
        {:else}
          <div class="pane-text">{streamA}</div>
          {#if resultA?.error}
            <div class="pane-error-text">{resultA.error}</div>
          {/if}
        {/if}
      </div>
    </div>

    <div class="split-divider"></div>

    <!-- Pane B -->
    <div class="pane" class:pane-error={resultB?.error}>
      <div class="pane-header">
        <div class="model-selector-wrap">
          <button
            type="button"
            class="model-badge"
            style="border-color: {modelB?.color ? `var(${modelB.color})` : 'var(--amber-faint)'}; color: {modelB?.color ? `var(${modelB.color})` : 'var(--amber-faint)'}"
            onclick={() => { pickerOpenB = !pickerOpenB; }}
            title="Select model B"
          >
            {modelB?.short_id ?? 'Model B'}
            <span class="badge-caret">{pickerOpenB ? '▴' : '▾'}</span>
          </button>
          {#if pickerOpenB}
            <div class="picker-backdrop" role="presentation" onclick={onPickerBackdropB}>
              <div class="model-picker">
                {#each llmModels.availableModels as m (m.id)}
                  <button
                    type="button"
                    class="picker-item"
                    class:active={m.id === modelBId}
                    onclick={() => pickModelB(m.id)}
                  >
                    <span class="status-dot" style="background: {llmModels.modelStatusColor(m.id)}"></span>
                    <span class="picker-short" style="color: {m.color ? `var(${m.color})` : 'var(--amber-faint)'}">{m.short_id}</span>
                    <span class="picker-name">{m.display_name}</span>
                  </button>
                {/each}
                {#if llmModels.availableModels.length === 0}
                  <div class="picker-empty">No models configured</div>
                {/if}
              </div>
            </div>
          {/if}
        </div>
        <span class="pane-meta">
          {#if sending && !doneB}
            <span class="latency-live">{elapsedB}ms</span>
            <span class="token-count">{streamTokensB} tok</span>
          {:else if resultB}
            <span class="latency-final">{resultB.latency_ms}ms</span>
            <span class="token-count">{resultB.tokens_in} in / {resultB.tokens_out} out</span>
            {#if resultB.cost_usd > 0}
              <span class="pane-cost">{formatCost(resultB.cost_usd)}</span>
            {/if}
          {/if}
        </span>
      </div>
      <div class="pane-content" bind:this={paneBEl}>
        {#if !streamB && !sending}
          <div class="pane-empty">
            {modelB ? `Ready: ${modelB.display_name}` : 'Select a model'}
          </div>
        {:else}
          <div class="pane-text">{streamB}</div>
          {#if resultB?.error}
            <div class="pane-error-text">{resultB.error}</div>
          {/if}
        {/if}
      </div>
    </div>
  </div>

  <!-- Critique panel -->
  {#if critiqueText}
    <div class="critique-panel">
      <button
        type="button"
        class="critique-header"
        onclick={() => { critiqueExpanded = !critiqueExpanded; }}
      >
        <span class="critique-badge">CRITIQUE</span>
        <span class="critique-caret">{critiqueExpanded ? '▴' : '▾'}</span>
      </button>
      {#if critiqueExpanded}
        <div class="critique-content">{critiqueText}</div>
      {/if}
    </div>
  {/if}

  <!-- Error bar -->
  {#if error}
    <div class="error-bar">{error}</div>
  {/if}

  <!-- Input area -->
  <div class="input-area">
    <textarea
      bind:value={inputText}
      placeholder={modelA && modelB
        ? `Compare ${modelA.short_id} vs ${modelB.short_id}...`
        : 'Select two models, then type a prompt...'}
      disabled={sending}
      onkeydown={onKeydown}
      rows={2}
    ></textarea>
    <button
      type="button"
      class="rift-btn rift-btn--primary send-btn"
      disabled={!modelAId || !modelBId || sending || !inputText.trim()}
      onclick={send}
    >{sending ? '...' : 'Send'}</button>
  </div>
</div>

<style>
  .ensemble-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 400px;
    font-family: var(--font-family);
  }

  /* ── Header ─────────────────────────────────────────────────── */

  .ensemble-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    border-bottom: 1px solid var(--border-subtle);
    font-size: var(--text-sm);
    flex-shrink: 0;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: var(--space-8);
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }

  .ensemble-label {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--term-purple);
    background: rgba(197, 143, 255, 0.1);
    border: 1px solid rgba(197, 143, 255, 0.25);
    border-radius: var(--radius-sm);
    padding: 1px var(--space-sm);
  }

  .model-tag {
    font-size: var(--text-xs);
    font-weight: 700;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 1px 5px;
  }

  .versus {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    opacity: 0.6;
  }

  .cost-badge {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--term-green);
    background: rgba(79, 232, 85, 0.08);
    border: 1px solid rgba(79, 232, 85, 0.2);
    border-radius: var(--radius-sm);
    padding: 1px 5px;
  }

  .critique-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    font-size: var(--text-xs);
    color: var(--term-purple);
    cursor: pointer;
  }

  .critique-toggle input {
    accent-color: var(--term-purple);
    cursor: pointer;
    width: var(--space-12);
    height: var(--space-12);
  }

  .sending-indicator {
    color: var(--amber-bright);
    font-size: var(--text-xs);
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 1; }
  }

  /* ── Split panes ────────────────────────────────────────────── */

  .split-container {
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .pane.pane-error {
    border: 1px solid var(--border-red-strong);
  }

  .split-divider {
    width: 1px;
    background: var(--border-subtle);
    flex-shrink: 0;
  }

  .pane-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
    min-height: var(--control-lg);
  }

  .pane-meta {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    opacity: 0.7;
    flex-shrink: 0;
  }

  .latency-live {
    color: var(--amber-bright);
    font-variant-numeric: tabular-nums;
  }

  .latency-final {
    font-variant-numeric: tabular-nums;
  }

  .token-count {
    font-variant-numeric: tabular-nums;
  }

  .pane-cost {
    color: var(--term-green);
  }

  .pane-content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-md);
    min-height: 0;
  }

  .pane-content::-webkit-scrollbar { width: 4px; }
  .pane-content::-webkit-scrollbar-track { background: transparent; }
  .pane-content::-webkit-scrollbar-thumb {
    background: var(--amber-faint);
    border-radius: var(--radius-sm);
  }

  .pane-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    font-size: var(--text-sm);
    color: var(--amber-faint);
    opacity: 0.5;
  }

  .pane-text {
    font-size: var(--text-base);
    line-height: 1.55;
    color: var(--term-white);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .pane-error-text {
    margin-top: var(--space-8);
    font-size: var(--text-sm);
    color: var(--term-red);
    background: var(--bg-red-tint);
    padding: var(--space-sm) var(--space-8);
    border-radius: var(--radius-md);
    border-left: 2px solid var(--term-red);
  }

  /* ── Model picker (shared with LlmChat pattern) ────────────── */

  .model-selector-wrap {
    position: relative;
  }

  .model-badge {
    background: transparent;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 2px var(--space-sm);
    font-family: var(--font-family);
    font-weight: 700;
    font-size: var(--text-xs);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    transition: background var(--duration-med), box-shadow var(--duration-med);
  }

  .model-badge:hover {
    background: var(--bg-amber-hover);
    box-shadow: var(--glow-amber-faint);
  }

  .badge-caret {
    font-size: var(--text-2xs);
    opacity: 0.6;
  }

  .model-badge:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .picker-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
  }

  .model-picker {
    position: absolute;
    top: calc(100% + var(--space-xs));
    left: 0;
    min-width: 200px;
    max-height: 220px;
    overflow-y: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border-amber-tint);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-flyout), 0 0 1px rgba(255, 168, 38, 0.15);
    z-index: 101;
    padding: var(--space-xs) 0;
    font-family: var(--font-family);
  }

  .model-picker::-webkit-scrollbar { width: 4px; }
  .model-picker::-webkit-scrollbar-track { background: transparent; }
  .model-picker::-webkit-scrollbar-thumb {
    background: var(--amber-faint);
    border-radius: var(--radius-sm);
  }

  .picker-item {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    width: 100%;
    padding: 5px var(--space-md);
    background: transparent;
    border: none;
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-base);
  }

  .picker-item:hover {
    background: var(--bg-amber-hover);
  }

  .picker-item:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .picker-item.active {
    background: var(--bg-amber-selected);
  }

  .status-dot {
    width: var(--space-sm);
    height: var(--space-sm);
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .picker-short {
    font-weight: 700;
    font-size: var(--text-xs);
    min-width: 32px;
  }

  .picker-name {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .picker-empty {
    padding: var(--space-8) var(--space-md);
    font-size: var(--text-xs);
    color: var(--amber-faint);
    opacity: 0.6;
  }

  /* ── Critique panel ─────────────────────────────────────────── */

  .critique-panel {
    border-top: 1px solid rgba(197, 143, 255, 0.25);
    flex-shrink: 0;
  }

  .critique-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--space-sm) var(--space-12);
    background: rgba(197, 143, 255, 0.05);
    border: none;
    cursor: pointer;
    font-family: var(--font-family);
    color: var(--term-purple);
    transition: background var(--duration-base);
  }

  .critique-header:hover {
    background: rgba(197, 143, 255, 0.1);
  }

  .critique-header:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .critique-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    border: 1px solid rgba(197, 143, 255, 0.35);
    border-radius: var(--radius-sm);
    padding: 1px var(--space-sm);
  }

  .critique-caret {
    font-size: var(--text-2xs);
    opacity: 0.6;
  }

  .critique-content {
    padding: var(--space-md) var(--space-12);
    font-size: var(--text-base);
    line-height: 1.55;
    color: var(--term-white);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 200px;
    overflow-y: auto;
    border-left: 2px solid var(--term-purple);
    margin: 0 var(--space-12) var(--space-8) var(--space-12);
    background: rgba(197, 143, 255, 0.03);
    border-radius: 0 var(--radius-md) var(--radius-md) 0;
  }

  .critique-content::-webkit-scrollbar { width: 4px; }
  .critique-content::-webkit-scrollbar-track { background: transparent; }
  .critique-content::-webkit-scrollbar-thumb {
    background: rgba(197, 143, 255, 0.3);
    border-radius: var(--radius-sm);
  }

  /* ── Error bar ──────────────────────────────────────────────── */

  .error-bar {
    padding: var(--space-xs) var(--space-12);
    font-size: var(--text-xs);
    color: var(--term-red);
    background: var(--bg-red-notice);
    border-top: 1px solid rgba(255, 72, 72, 0.2);
    flex-shrink: 0;
  }

  /* ── Input area ─────────────────────────────────────────────── */

  .input-area {
    display: flex;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    border-top: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }

  .input-area textarea {
    flex: 1;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid var(--border-amber-tint);
    border-radius: var(--radius-md);
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-base);
    padding: var(--space-sm) var(--space-8);
    resize: none;
    line-height: 1.4;
  }

  .input-area textarea:focus {
    border-color: var(--amber-faint);
  }
  .input-area textarea:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .input-area textarea:disabled {
    opacity: 0.4;
  }

  .send-btn {
    align-self: flex-end;
    min-width: 60px;
  }
</style>
