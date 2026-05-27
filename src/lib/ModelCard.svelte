<script lang="ts">
  import type { ModelConfig, ProviderType, KvCacheType, LlamaServerConfig } from './riftConfig';
  import { llmModels, type ProcessStatus } from './llmModels.svelte';
  import VramEstimator from './VramEstimator.svelte';

  interface Props {
    model: ModelConfig;
    onremove: () => void;
  }

  let { model, onremove }: Props = $props();

  let expanded = $state(false);

  const PROVIDER_LABELS: Record<ProviderType, string> = {
    anthropic: 'Anthropic',
    google: 'Google Gemini',
    llama_server: 'llama-server',
    open_ai_compat: 'OpenAI-Compatible',
  };

  const KV_CACHE_OPTIONS: KvCacheType[] = [
    'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1',
  ];

  let status: ProcessStatus = $derived(
    llmModels.processStatus[model.id] ?? 'stopped',
  );

  let statusColor = $derived(llmModels.modelStatusColor(model.id));

  let isLocal = $derived(model.hosting.mode === 'local');

  function update<K extends keyof ModelConfig>(key: K, value: ModelConfig[K]) {
    llmModels.updateModel(model.id, { [key]: value });
  }
</script>

<div class="model-card" style="--card-accent: {statusColor}">
  <div class="card-header">
    <span class="status-dot" style="background: {statusColor}" title={status}></span>
    <span class="short-id">{model.short_id || '???'}</span>
    <input
      class="display-name"
      type="text"
      value={model.display_name}
      placeholder="Model name"
      onchange={(e) => update('display_name', (e.target as HTMLInputElement).value)}
    />
    <span class="provider-badge">{PROVIDER_LABELS[model.provider]}</span>
    <button
      type="button"
      class="expand-btn"
      onclick={() => (expanded = !expanded)}
      aria-expanded={expanded}
    >{expanded ? '▾' : '▸'}</button>
  </div>

  {#if expanded}
  <div class="card-body">
    <div class="field">
      <label>Endpoint</label>
      <input
        type="text"
        value={model.endpoint}
        placeholder={isLocal ? 'http://127.0.0.1:8081' : 'https://api.example.com'}
        onchange={(e) => update('endpoint', (e.target as HTMLInputElement).value)}
      />
    </div>

    <div class="field">
      <label>Model ID</label>
      <input
        type="text"
        value={model.model_identifier}
        placeholder="e.g. gemma-4-27b-it-Q4_K_M.gguf"
        onchange={(e) => update('model_identifier', (e.target as HTMLInputElement).value)}
      />
    </div>

    <div class="field">
      <label>Short ID (2-4 chars)</label>
      <input
        type="text"
        value={model.short_id}
        maxlength={4}
        placeholder="LOC"
        onchange={(e) => update('short_id', (e.target as HTMLInputElement).value.toUpperCase())}
      />
    </div>

    {#if !isLocal}
    <div class="field">
      <label>API Key (stored in OS keyring)</label>
      <input
        type="password"
        value={model.api_key_ref ?? ''}
        placeholder="sk-..."
        onchange={(e) => update('api_key_ref', (e.target as HTMLInputElement).value || null)}
      />
    </div>
    {/if}

    {#if isLocal}
    <fieldset class="llama-config">
      <legend>llama-server</legend>

      <div class="field">
        <label>GGUF Model Path</label>
        <input
          type="text"
          value={(model.hosting as any).model_path ?? ''}
          placeholder="C:\Models\model.gguf"
          onchange={(e) => {
            const h = { ...model.hosting, model_path: (e.target as HTMLInputElement).value };
            update('hosting', h);
          }}
        />
      </div>

      <div class="field-row">
        <div class="field">
          <label>Context Size</label>
          <input
            type="number"
            value={(model.hosting as any).ctx_size ?? 32768}
            min={2048}
            max={131072}
            step={1024}
            onchange={(e) => {
              const h = { ...model.hosting, ctx_size: parseInt((e.target as HTMLInputElement).value) };
              update('hosting', h);
            }}
          />
        </div>

        <div class="field">
          <label>GPU Layers</label>
          <input
            type="number"
            value={(model.hosting as any).n_gpu_layers ?? 99}
            min={0}
            max={999}
            onchange={(e) => {
              const h = { ...model.hosting, n_gpu_layers: parseInt((e.target as HTMLInputElement).value) };
              update('hosting', h);
            }}
          />
        </div>

        <div class="field">
          <label>Port</label>
          <input
            type="number"
            value={(model.hosting as any).port ?? 8081}
            min={1024}
            max={65535}
            onchange={(e) => {
              const h = { ...model.hosting, port: parseInt((e.target as HTMLInputElement).value) };
              update('hosting', h);
            }}
          />
        </div>
      </div>

      <div class="field-row">
        <div class="field">
          <label>KV Cache Key</label>
          <select
            value={(model.hosting as any).cache_type_k ?? 'q8_0'}
            onchange={(e) => {
              const h = { ...model.hosting, cache_type_k: (e.target as HTMLSelectElement).value as KvCacheType };
              update('hosting', h);
            }}
          >
            {#each KV_CACHE_OPTIONS as opt}
              <option value={opt}>{opt}</option>
            {/each}
          </select>
        </div>

        <div class="field">
          <label>KV Cache Value</label>
          <select
            value={(model.hosting as any).cache_type_v ?? 'q8_0'}
            onchange={(e) => {
              const h = { ...model.hosting, cache_type_v: (e.target as HTMLSelectElement).value as KvCacheType };
              update('hosting', h);
            }}
          >
            {#each KV_CACHE_OPTIONS as opt}
              <option value={opt}>{opt}</option>
            {/each}
          </select>
        </div>

        <div class="field">
          <label>
            <input
              type="checkbox"
              checked={(model.hosting as any).flash_attention ?? true}
              onchange={(e) => {
                const h = { ...model.hosting, flash_attention: (e.target as HTMLInputElement).checked };
                update('hosting', h);
              }}
            />
            Flash Attention
          </label>
        </div>
      </div>

      <div class="field">
        <label>
          <input
            type="checkbox"
            checked={(model.hosting as any).auto_start ?? false}
            onchange={(e) => {
              const h = { ...model.hosting, auto_start: (e.target as HTMLInputElement).checked };
              update('hosting', h);
            }}
          />
          Auto-start on Rift launch
        </label>
      </div>

      <VramEstimator config={model.hosting as unknown as LlamaServerConfig} />
    </fieldset>
    {/if}

    <div class="card-actions">
      <button type="button" class="rift-btn danger" onclick={onremove}>Remove</button>
    </div>
  </div>
  {/if}
</div>

<style>
  .model-card {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--amber-faint, #A87830);
    border-left: 3px solid var(--card-accent, var(--amber-faint));
    border-radius: var(--radius-md, 4px);
    margin-bottom: 8px;
    overflow: hidden;
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    cursor: pointer;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .short-id {
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    font-weight: 700;
    color: var(--amber-bright, #FFC840);
    min-width: 32px;
  }

  .display-name {
    flex: 1;
    background: transparent;
    border: none;
    border-bottom: 1px solid transparent;
    color: var(--term-white, #E8E4D8);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    padding: 2px 4px;
  }
  .display-name:focus {
    border-bottom-color: var(--amber-faint, #A87830);
    outline: none;
  }

  .provider-badge {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--amber-faint, #A87830);
    border: 1px solid rgba(168, 120, 48, 0.3);
    border-radius: 3px;
    padding: 1px 6px;
    white-space: nowrap;
  }

  .expand-btn {
    background: none;
    border: none;
    color: var(--amber-faint, #A87830);
    cursor: pointer;
    font-size: 12px;
    padding: 2px 6px;
  }

  .card-body {
    padding: 0 12px 12px;
    border-top: 1px solid rgba(168, 120, 48, 0.15);
  }

  .field {
    margin-top: 8px;
  }

  .field label {
    display: block;
    font-size: 10px;
    color: var(--amber-faint, #A87830);
    margin-bottom: 2px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .field input[type="text"],
  .field input[type="password"],
  .field input[type="number"],
  .field select {
    width: 100%;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid rgba(168, 120, 48, 0.25);
    border-radius: var(--radius-md, 4px);
    color: var(--term-white, #E8E4D8);
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    padding: 4px 8px;
    box-sizing: border-box;
  }
  .field input:focus,
  .field select:focus {
    border-color: var(--amber-faint, #A87830);
    outline: none;
  }

  .field-row {
    display: flex;
    gap: 8px;
  }
  .field-row .field {
    flex: 1;
  }

  .llama-config {
    border: 1px solid rgba(168, 120, 48, 0.2);
    border-radius: var(--radius-md, 4px);
    padding: 8px;
    margin-top: 8px;
  }
  .llama-config legend {
    font-size: 10px;
    color: var(--amber-faint, #A87830);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 0 4px;
  }

  .field input[type="checkbox"] {
    accent-color: var(--amber-primary, #FFA826);
    margin-right: 4px;
  }

  .card-actions {
    margin-top: 12px;
    display: flex;
    justify-content: flex-end;
  }
</style>
