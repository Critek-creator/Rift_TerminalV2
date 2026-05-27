import { describe, it, expect, beforeEach } from 'vitest';
import { llmModels, loadFromConfig } from '../llmModels.svelte';
import type { EnsembleConfig, ModelConfig } from '../riftConfig';

/** Build a minimal valid ModelConfig for testing. */
function makeModel(overrides: Partial<ModelConfig> = {}): ModelConfig {
  return {
    id: overrides.id ?? 'test-model',
    display_name: overrides.display_name ?? 'Test Model',
    provider: overrides.provider ?? 'llama_server',
    model_identifier: overrides.model_identifier ?? 'test/model.gguf',
    hosting: overrides.hosting ?? { mode: 'local' as const, model_path: '', flash_attention: true, ctx_size: 32768, cache_type_k: 'q8_0', cache_type_v: 'q8_0', n_gpu_layers: 99, threads: null, parallel: 1, port: 8081, cuda_visible_devices: null, auto_start: false, extra_flags: [] },
    endpoint: overrides.endpoint ?? '',
    api_key_ref: overrides.api_key_ref ?? null,
    color: overrides.color ?? '--model-custom',
    short_id: overrides.short_id ?? 'TST',
    capabilities: overrides.capabilities ?? { max_context_tokens: 32768, supports_streaming: true, supports_tool_use: false, cost_per_1m_input: 0, cost_per_1m_output: 0, strength_tags: [] },
  };
}

function resetModels() {
  // Load an empty config to clear state
  loadFromConfig({
    enabled: false,
    active_profile: 'manual',
    default_model: '',
    models: [],
  });
}

describe('llmModels store', () => {
  beforeEach(() => {
    resetModels();
  });

  describe('loadFromConfig', () => {
    it('populates models, enabled, activeProfile, defaultModel from valid config', () => {
      const m = makeModel({ id: 'opus', short_id: 'OPU' });
      const config: EnsembleConfig = {
        enabled: true,
        active_profile: 'quality_first',
        default_model: 'opus',
        models: [m],
      };

      loadFromConfig(config);

      expect(llmModels.models).toHaveLength(1);
      expect(llmModels.models[0].id).toBe('opus');
      expect(llmModels.enabled).toBe(true);
      expect(llmModels.activeProfile).toBe('quality_first');
      expect(llmModels.defaultModel).toBe('opus');
    });

    it('is a no-op when called with undefined', () => {
      const m = makeModel({ id: 'existing' });
      loadFromConfig({
        enabled: true,
        active_profile: 'balanced',
        default_model: 'existing',
        models: [m],
      });

      loadFromConfig(undefined);

      // State unchanged
      expect(llmModels.models).toHaveLength(1);
      expect(llmModels.enabled).toBe(true);
    });
  });

  describe('addModel', () => {
    it('creates a model with local hosting defaults for llama_server', () => {
      const model = llmModels.addModel('llama_server');

      expect(model.provider).toBe('llama_server');
      expect(model.hosting.mode).toBe('local');
      expect(model.endpoint).toBe('');
      expect(model.api_key_ref).toBeNull();
      expect(llmModels.models).toHaveLength(1);
    });

    it('creates a model with cloud hosting and anthropic endpoint', () => {
      const model = llmModels.addModel('anthropic');

      expect(model.provider).toBe('anthropic');
      expect(model.hosting.mode).toBe('cloud');
      expect(model.endpoint).toBe('https://api.anthropic.com/v1/messages');
      expect(model.api_key_ref).toBe('');
      expect(llmModels.models).toHaveLength(1);
    });

    it('creates a model with cloud hosting and google endpoint', () => {
      const model = llmModels.addModel('google');

      expect(model.provider).toBe('google');
      expect(model.hosting.mode).toBe('cloud');
      expect(model.endpoint).toBe('https://generativelanguage.googleapis.com/v1beta');
    });

    it('creates a model with remote hosting for open_ai_compat', () => {
      const model = llmModels.addModel('open_ai_compat');

      expect(model.provider).toBe('open_ai_compat');
      expect(model.hosting.mode).toBe('remote');
    });
  });

  describe('removeModel', () => {
    it('removes the model from the list', () => {
      const model = llmModels.addModel('llama_server');
      expect(llmModels.models).toHaveLength(1);

      llmModels.removeModel(model.id);
      expect(llmModels.models).toHaveLength(0);
    });

    it('clears defaultModel if it referenced the removed model', () => {
      const model = llmModels.addModel('llama_server');
      llmModels.setDefaultModel(model.id);
      expect(llmModels.defaultModel).toBe(model.id);

      llmModels.removeModel(model.id);
      expect(llmModels.defaultModel).toBe('');
    });

    it('clears activeModelId if it referenced the removed model', () => {
      const model = llmModels.addModel('llama_server');
      llmModels.setActiveModel(model.id);
      expect(llmModels.activeModelId).toBe(model.id);

      llmModels.removeModel(model.id);
      expect(llmModels.activeModelId).toBeNull();
    });
  });

  describe('updateModel', () => {
    it('merges partial updates into the target model', () => {
      const model = llmModels.addModel('llama_server');
      llmModels.updateModel(model.id, { display_name: 'Updated Name', short_id: 'UPD' });

      const updated = llmModels.getModel(model.id);
      expect(updated?.display_name).toBe('Updated Name');
      expect(updated?.short_id).toBe('UPD');
      // Other fields remain
      expect(updated?.provider).toBe('llama_server');
    });
  });

  describe('setActiveModel / setEnabled / setActiveProfile', () => {
    it('setActiveModel updates activeModelId', () => {
      llmModels.setActiveModel('some-model');
      expect(llmModels.activeModelId).toBe('some-model');

      llmModels.setActiveModel(null);
      expect(llmModels.activeModelId).toBeNull();
    });

    it('setEnabled updates enabled flag', () => {
      llmModels.setEnabled(true);
      expect(llmModels.enabled).toBe(true);

      llmModels.setEnabled(false);
      expect(llmModels.enabled).toBe(false);
    });

    it('setActiveProfile updates activeProfile', () => {
      llmModels.setActiveProfile('cost_optimized');
      expect(llmModels.activeProfile).toBe('cost_optimized');
    });
  });

  describe('modelStatusColor', () => {
    it('returns green for running', () => {
      llmModels.updateProcessStatus('m1', 'running');
      expect(llmModels.modelStatusColor('m1')).toBe('var(--term-green)');
    });

    it('returns amber-bright for starting', () => {
      llmModels.updateProcessStatus('m1', 'starting');
      expect(llmModels.modelStatusColor('m1')).toBe('var(--amber-bright)');
    });

    it('returns red for error', () => {
      llmModels.updateProcessStatus('m1', 'error');
      expect(llmModels.modelStatusColor('m1')).toBe('var(--term-red)');
    });

    it('returns faint amber for offline', () => {
      llmModels.updateProcessStatus('m1', 'offline');
      expect(llmModels.modelStatusColor('m1')).toBe('rgba(168,120,48,0.3)');
    });

    it('returns faint amber for stopped (default)', () => {
      llmModels.updateProcessStatus('m1', 'stopped');
      expect(llmModels.modelStatusColor('m1')).toBe('rgba(168,120,48,0.3)');
    });
  });

  describe('getModel', () => {
    it('returns undefined for unknown ID', () => {
      expect(llmModels.getModel('nonexistent')).toBeUndefined();
    });

    it('returns the model when it exists', () => {
      const model = llmModels.addModel('anthropic');
      const found = llmModels.getModel(model.id);
      expect(found).toBeDefined();
      expect(found?.id).toBe(model.id);
    });
  });
});
