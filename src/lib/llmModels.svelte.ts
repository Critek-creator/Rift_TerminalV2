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
  const config = await invoke<import('./riftConfig').RiftConfig>('config_get');
  const next = { ...config, ensemble: buildEnsembleConfig() };
  await invoke('config_save', { cfg: next });
}

// ---------------------------------------------------------------------------
// Local process lifecycle
// ---------------------------------------------------------------------------

// Start/stop a local llama-server via the Tauri commands. Status is set
// optimistically here; the authoritative source is the llm.process.* bus
// events applied by applyProcessEvent (wired in App.svelte).
async function startProcess(id: string): Promise<void> {
  // Persist current settings FIRST: the backend's llm_model_start reads the
  // model config from disk, so unsaved UI edits (GPU layers, ctx size, etc.)
  // would otherwise be ignored and the server would launch with stale flags.
  try {
    await saveEnsemble();
  } catch (err) {
    console.warn('[llmModels] save before start failed, launching with persisted config:', err);
  }
  processStatus = { ...processStatus, [id]: 'starting' };
  try {
    await invoke('llm_model_start', { modelId: id });
    processStatus = { ...processStatus, [id]: 'running' };
  } catch (err) {
    processStatus = { ...processStatus, [id]: 'error' };
    throw err;
  }
}

async function stopProcess(id: string): Promise<void> {
  try {
    await invoke('llm_model_stop', { modelId: id });
  } finally {
    processStatus = { ...processStatus, [id]: 'stopped' };
  }
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
    cpu_moe: false,
    n_cpu_moe: null,
    cache_ram: null,
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
  /** Enabled models only — pickers/routing surfaces should use this so
   *  disabled models don't appear as selectable. */
  get availableModels() { return models.filter((m) => m.enabled !== false); },
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
      enabled: true,
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

  /** Start a local llama-server. Reads persisted config, so save first. */
  startModel(id: string): Promise<void> {
    return startProcess(id);
  },

  /** Stop a running local llama-server. */
  stopModel(id: string): Promise<void> {
    return stopProcess(id);
  },

  /** Activate a model. For local models, free VRAM by stopping other running
   *  local servers first, then start this one (a single GPU rarely holds two).
   *  Cloud models simply become the active route. */
  async activateModel(id: string): Promise<void> {
    const model = models.find((m) => m.id === id);
    if (!model) return;
    if (model.hosting.mode === 'local') {
      // Stop every other running local server first (free VRAM). Use backend
      // truth so a server the store didn't track can't get stacked on top.
      let running: string[] = [];
      try {
        running = await invoke<string[]>('llm_models_running');
      } catch {
        running = models.filter((m) => processStatus[m.id] === 'running').map((m) => m.id);
      }
      for (const rid of running) {
        if (rid !== id) await stopProcess(rid).catch(() => { /* best effort */ });
      }
      await startProcess(id);
    }
    activeModelId = id;
  },

  /** Seed status dots on mount from the set of currently-running processes. */
  async syncRunning(): Promise<void> {
    try {
      const running = await invoke<string[]>('llm_models_running');
      const next = { ...processStatus };
      for (const rid of running) next[rid] = 'running';
      processStatus = next;
    } catch {
      /* non-fatal — leave statuses as-is */
    }
  },

  /** Apply an llm.process.* bus event to the status map (live updates from
   *  auto-start, MCP-initiated starts, crashes, and our own start/stop). */
  applyProcessEvent(kind: string, modelId: string): void {
    if (!modelId) return;
    if (kind === 'llm.process.start') processStatus = { ...processStatus, [modelId]: 'running' };
    else if (kind === 'llm.process.stop') processStatus = { ...processStatus, [modelId]: 'stopped' };
    else if (kind === 'llm.process.crash') processStatus = { ...processStatus, [modelId]: 'error' };
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
