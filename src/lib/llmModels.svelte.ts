// llmModels.svelte.ts — Ensemble Router model state.
//
// Reactive store for configured LLM models, process status, and the
// active routing profile. Reads from RiftConfig.ensemble; writes via
// config_save Tauri command. Process status tracked from bus events.

import { invoke } from '@tauri-apps/api/core';
import type {
  ModelConfig,
  EnsembleConfig,
  RoutingProfile,
  ProviderType,
  LlamaServerConfig,
} from './riftConfig';

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let models = $state<ModelConfig[]>([]);
let enabled = $state(false);
let activeProfile = $state<RoutingProfile>('manual');
let defaultModel = $state('');
let activeModelId = $state<string | null>(null);

export type ProcessStatus = 'stopped' | 'starting' | 'running' | 'error' | 'offline';

let processStatus = $state<Record<string, ProcessStatus>>({});
let healthLatency = $state<Record<string, number>>({});

// ---------------------------------------------------------------------------
// Init from config
// ---------------------------------------------------------------------------

export function loadFromConfig(ensemble: EnsembleConfig | undefined) {
  if (!ensemble) return;
  models = ensemble.models ?? [];
  enabled = ensemble.enabled ?? false;
  activeProfile = ensemble.active_profile ?? 'manual';
  defaultModel = ensemble.default_model ?? '';
  if (!activeModelId && defaultModel) {
    activeModelId = defaultModel;
  }
}

// ---------------------------------------------------------------------------
// Config save
// ---------------------------------------------------------------------------

function buildEnsembleConfig(): EnsembleConfig {
  return {
    enabled,
    active_profile: activeProfile,
    default_model: defaultModel,
    models,
  };
}

async function saveEnsemble(): Promise<void> {
  await invoke('config_save', {
    section: 'ensemble',
    value: JSON.stringify(buildEnsembleConfig()),
  });
}

// ---------------------------------------------------------------------------
// Model CRUD
// ---------------------------------------------------------------------------

function nextPort(): number {
  const used = new Set(
    models
      .filter((m) => m.hosting.mode === 'local')
      .map((m) => (m.hosting as LlamaServerConfig & { mode: 'local' }).port),
  );
  let port = 8081;
  while (used.has(port)) port++;
  return port;
}

function defaultLocalConfig(): LlamaServerConfig & { mode: 'local' } {
  return {
    mode: 'local',
    model_path: '',
    flash_attention: true,
    ctx_size: 32768,
    cache_type_k: 'q8_0',
    cache_type_v: 'q8_0',
    n_gpu_layers: 99,
    threads: null,
    parallel: 1,
    port: nextPort(),
    cuda_visible_devices: null,
    auto_start: false,
    extra_flags: [],
  };
}

function defaultCapabilities(): ModelConfig['capabilities'] {
  return {
    max_context_tokens: 32768,
    supports_streaming: true,
    supports_tool_use: false,
    cost_per_1m_input: 0,
    cost_per_1m_output: 0,
    strength_tags: [],
  };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export const llmModels = {
  get models() { return models; },
  get enabled() { return enabled; },
  get activeProfile() { return activeProfile; },
  get defaultModel() { return defaultModel; },
  get activeModelId() { return activeModelId; },
  get processStatus() { return processStatus; },
  get healthLatency() { return healthLatency; },

  setEnabled(v: boolean) {
    enabled = v;
  },

  setActiveProfile(p: RoutingProfile) {
    activeProfile = p;
  },

  setDefaultModel(id: string) {
    defaultModel = id;
  },

  setActiveModel(id: string | null) {
    activeModelId = id;
  },

  getModel(id: string): ModelConfig | undefined {
    return models.find((m) => m.id === id);
  },

  addModel(provider: ProviderType): ModelConfig {
    const id = `model-${Date.now()}`;
    const isLocal = provider === 'llama_server';
    const model: ModelConfig = {
      id,
      display_name: '',
      provider,
      model_identifier: '',
      hosting: isLocal
        ? defaultLocalConfig()
        : provider === 'anthropic'
          ? { mode: 'cloud' as const }
          : provider === 'google'
            ? { mode: 'cloud' as const }
            : { mode: 'remote' as const, health_check_interval_secs: 30 },
      endpoint: provider === 'anthropic'
        ? 'https://api.anthropic.com/v1/messages'
        : provider === 'google'
          ? 'https://generativelanguage.googleapis.com/v1beta'
          : '',
      api_key_ref: isLocal ? null : '',
      color: '--model-custom',
      short_id: '',
      capabilities: defaultCapabilities(),
    };
    models = [...models, model];
    return model;
  },

  updateModel(id: string, updates: Partial<ModelConfig>) {
    models = models.map((m) =>
      m.id === id ? { ...m, ...updates } : m,
    );
  },

  removeModel(id: string) {
    models = models.filter((m) => m.id !== id);
    if (defaultModel === id) defaultModel = '';
    if (activeModelId === id) activeModelId = null;
  },

  async save(): Promise<void> {
    await saveEnsemble();
  },

  updateProcessStatus(modelId: string, status: ProcessStatus, latencyMs?: number) {
    processStatus = { ...processStatus, [modelId]: status };
    if (latencyMs !== undefined) {
      healthLatency = { ...healthLatency, [modelId]: latencyMs };
    }
  },

  modelStatusColor(modelId: string): string {
    const status = processStatus[modelId];
    switch (status) {
      case 'running': return 'var(--term-green)';
      case 'starting': return 'var(--amber-bright)';
      case 'error': return 'var(--term-red)';
      case 'offline': return 'rgba(168,120,48,0.3)';
      default: return 'rgba(168,120,48,0.3)';
    }
  },
};
