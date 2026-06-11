<script module lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  // Detect the GPU's real VRAM once per session, shared across every ModelCard
  // (the promise is cached at module scope so N cards don't each spawn
  // nvidia-smi). Returns total MiB, or null when detection isn't available.
  let gpuVramPromise: Promise<number | null> | undefined;
  function detectGpuVramMb(): Promise<number | null> {
    if (!gpuVramPromise) {
      gpuVramPromise = invoke<number | null>('gpu_vram_mb').catch(() => null);
    }
    return gpuVramPromise;
  }
</script>

<script lang="ts">
  import type { ModelConfig, ProviderType, KvCacheType, LlamaServerConfig, GgufMeta } from './riftConfig';
  import { llmModels, type ProcessStatus } from './llmModels.svelte';
  import VramEstimator from './VramEstimator.svelte';
  import { fitToGpu, classifyConfig } from './vramModel';
  import { sessionManager } from './sessionManager.svelte';
  import { popouts } from './popouts.svelte';

  interface Props {
    model: ModelConfig;
    isDefault?: boolean;
    onremove: () => void;
    onsetdefault?: () => void;
  }

  let { model, isDefault = false, onremove, onsetdefault }: Props = $props();

  let showAdvanced = $state(false);

  const PROVIDER_META: Record<ProviderType, { label: string; desc: string; color: string }> = {
    anthropic:      { label: 'Anthropic',        desc: 'Claude models via Anthropic API',        color: 'var(--model-claude)' },
    google:         { label: 'Google Gemini',     desc: 'Gemini models via Google AI API',        color: 'var(--model-gemini)' },
    llama_server:   { label: 'Local (llama.cpp)', desc: 'Self-hosted GGUF model via llama-server', color: 'var(--model-local)' },
    open_ai_compat: { label: 'OpenAI-Compatible', desc: 'Any OpenAI-compatible endpoint',          color: 'var(--model-custom)' },
    cli:            { label: 'CLI tool',          desc: 'External CLI (e.g. gemini) — endpoint holds the command, no API key', color: 'var(--model-custom)' },
  };

  const KNOWN_MODELS: Record<ProviderType, string[]> = {
    anthropic: [
      'claude-opus-4-6', 'claude-sonnet-4-6', 'claude-haiku-4-5-20251001',
      'claude-opus-4-5-20250414', 'claude-sonnet-4-5-20250414',
    ],
    // Verified against the installed gemini CLI bundle (v0.44.1, 2026-05-31) —
    // newest first. There is no gemini-3.5; the current flagship line is 3.1.
    google: [
      'gemini-3.1-pro', 'gemini-3.1-flash', 'gemini-3.1-flash-lite',
      'gemini-2.5-pro', 'gemini-2.5-flash', 'gemini-2.5-flash-lite',
      'gemini-2.0-flash',
    ],
    llama_server: [
      'gemma-4-27b-it-Q4_K_M.gguf', 'gemma-4-12b-it-Q6_K.gguf',
      'llama-3.3-70b-instruct-Q4_K_M.gguf', 'Qwen3-32B-Q4_K_M.gguf',
      'Qwen3-14B-Q6_K.gguf', 'mistral-small-3.2-24b-instruct-2503-Q4_K_M.gguf',
    ],
    open_ai_compat: [
      'gpt-4o', 'gpt-4o-mini', 'gpt-4-turbo', 'gpt-3.5-turbo',
      'o3-mini', 'o4-mini',
    ],
    cli: [
      'gemini-3.1-pro', 'gemini-3.1-flash', 'gemini-3.1-flash-lite',
      'gemini-2.5-pro', 'gemini-2.5-flash', 'gemini-2.5-flash-lite',
      'gemini-2.0-flash',
    ],
  };

  /** Gemini models offered in the CLI sign-in panel picker (newest first). */
  const GEMINI_CLI_MODELS = KNOWN_MODELS.cli;

  const KV_CACHE_OPTIONS: KvCacheType[] = [
    'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1',
  ];

  let status: ProcessStatus = $derived(
    llmModels.processStatus[model.id] ?? 'stopped',
  );

  let statusColor = $derived(llmModels.modelStatusColor(model.id));

  let statusLabel = $derived(
    status === 'running' ? 'Running' :
    status === 'starting' ? 'Starting…' :
    status === 'error' ? 'Error' :
    status === 'offline' ? 'Offline' : 'Stopped'
  );

  let isLocal = $derived(model.hosting.mode === 'local');

  // ─── CLI-backed (Gemini) provider state ──────────────────────────────────
  // A CLI model drives an external tool (the `gemini` CLI) via its own OAuth
  // session — no API key. The command template lives in `endpoint`; the model
  // picker rewrites its --model/-m token.
  let isCli = $derived(model.provider === 'cli');

  type GeminiAuth = {
    cli_installed: boolean;
    authenticated: boolean;
    account: string | null;
    headless_ready: boolean;
  };
  let geminiAuth = $state<GeminiAuth | null>(null);
  let geminiAuthLoading = $state(false);
  // One-shot guard so the auto-probe effect fires exactly once per card even
  // when the probe FAILS (on error geminiAuth stays null — without this flag
  // the effect's `=== null` guard would re-fire in a loop). The Recheck button
  // calls refreshGeminiAuth directly and is unaffected.
  let geminiAuthProbed = $state(false);

  async function refreshGeminiAuth() {
    geminiAuthLoading = true;
    geminiAuthProbed = true;
    try {
      geminiAuth = await invoke<GeminiAuth>('gemini_auth_status');
    } catch {
      geminiAuth = null;
    } finally {
      geminiAuthLoading = false;
    }
  }

  // Probe auth state once when this is a CLI card (and re-probe is available
  // via the button). Cheap: pure filesystem + PATH check on the backend.
  $effect(() => {
    if (isCli && !geminiAuthProbed) {
      void refreshGeminiAuth();
    }
  });

  /** Open a Rift terminal tab running `gemini` (triggers the browser OAuth on
   *  first use), and close Settings so the user lands in that terminal. */
  function signInToGemini() {
    sessionManager.openTerminalWithCommand('gemini');
    popouts.dismissAll();
  }

  /** Select the OAuth method in ~/.gemini/settings.json so headless `gemini -p`
   *  works (it exits 41 without one). Then re-probe to update the panel. */
  let enablingHeadless = $state(false);
  async function finishGeminiSetup() {
    enablingHeadless = true;
    try {
      await invoke('gemini_enable_headless');
      await refreshGeminiAuth();
    } catch (e) {
      console.error('[ModelCard] gemini_enable_headless failed', e);
    } finally {
      enablingHeadless = false;
    }
  }

  /** Extract the model from a `gemini … --model X` / `-m X` command template,
   *  falling back to the stored model_identifier. */
  function cliModelFromTemplate(template: string): string {
    const m = template.match(/(?:--model|-m)[ =]([^\s]+)/);
    return m ? m[1] : model.model_identifier;
  }

  let currentCliModel = $derived(isCli ? cliModelFromTemplate(model.endpoint) : '');

  /** Set the Gemini model: rewrite the --model/-m token in the command
   *  template in place (or append one if absent), and keep model_identifier in
   *  sync for routing/display. */
  function setCliModel(next: string) {
    let template = model.endpoint;
    if (/(?:--model|-m)[ =][^\s]+/.test(template)) {
      template = template.replace(/((?:--model|-m)[ =])[^\s]+/, `$1${next}`);
    } else {
      template = `${template.trim()} --model ${next}`;
    }
    llmModels.updateModel(model.id, { endpoint: template, model_identifier: next });
  }

  let providerMeta = $derived(PROVIDER_META[model.provider] ?? { label: model.provider, desc: '', color: 'var(--amber-faint)' });

  let latencyMs = $derived(llmModels.healthLatency[model.id]);

  const DEFAULT_CAPS: ModelConfig['capabilities'] = {
    max_context_tokens: 32768,
    supports_streaming: true,
    supports_tool_use: false,
    cost_per_1m_input: 0,
    cost_per_1m_output: 0,
    strength_tags: [],
  };

  let caps = $derived(model.capabilities ?? DEFAULT_CAPS);

  function update<K extends keyof ModelConfig>(key: K, value: ModelConfig[K]) {
    llmModels.updateModel(model.id, { [key]: value });
  }

  function updateCapability<K extends keyof ModelConfig['capabilities']>(
    key: K,
    value: ModelConfig['capabilities'][K],
  ) {
    const updated = { ...caps, [key]: value };
    update('capabilities', updated);
  }

  let confirmRemove = $state(false);
  let busy = $state(false);

  // --- VRAM-estimate inputs (candidate #157) -------------------------------
  // Real GPU VRAM + GGUF header metadata, both best-effort. VramEstimator falls
  // back to its filename/arch heuristics when these are null/unset.
  let gpuVramGb = $state(16);
  let ggufMeta = $state<GgufMeta | null>(null);

  // Detect GPU VRAM once (cached at module scope across all cards).
  $effect(() => {
    void (async () => {
      const mb = await detectGpuVramMb();
      if (mb && mb > 0) gpuVramGb = Math.round((mb / 1024) * 10) / 10;
    })();
  });

  // Read GGUF metadata whenever a local model's path changes. A missing or
  // unreadable file clears meta so the estimator silently uses heuristics.
  $effect(() => {
    const path = isLocal ? ((model.hosting as any).model_path as string) : '';
    if (!path) {
      ggufMeta = null;
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const meta = await invoke<GgufMeta>('gguf_inspect', { path });
        if (!cancelled) ggufMeta = meta;
      } catch {
        if (!cancelled) ggufMeta = null;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  // Collapsed state — persisted per model so the list stays at-a-glance.
  // model.id read inside closures (not at script top level) to avoid Svelte's
  // state_referenced_locally warning; the id is stable per card anyway.
  const collapseKey = () => `rift:mc-collapsed:${model.id}`;
  function readCollapsed(): boolean {
    try {
      return localStorage.getItem(collapseKey()) === '1';
    } catch {
      return false;
    }
  }
  let collapsed = $state(readCollapsed());
  function toggleCollapsed() {
    collapsed = !collapsed;
    try {
      localStorage.setItem(collapseKey(), collapsed ? '1' : '0');
    } catch {
      /* localStorage unavailable — non-fatal */
    }
  }

  // --- "Fit to my GPU" auto-tuner (candidate #789) -------------------------
  // The verdict chip is LIVE: it classifies the CURRENT hosting config (incl.
  // hand edits) against detected VRAM via the same math as the VRAM readout, so
  // it's always accurate without needing the solver to have run. The button runs
  // the solver and applies the winning n_gpu_layers / cpu_moe / n_cpu_moe combo
  // through the existing update('hosting', …) path; ctx_size and KV-quant are
  // never auto-touched (they encode quality intent — the chip names them when
  // they're the only remaining lever). `changedFlash` lists what the last click
  // altered, and clears on the next manual edit.
  let fitVerdict = $derived(
    isLocal
      ? classifyConfig(
          model.hosting as unknown as LlamaServerConfig,
          model.model_identifier,
          ggufMeta,
          gpuVramGb,
        )
      : null,
  );

  let fitChipColor = $derived(
    !fitVerdict ? ''
      : fitVerdict.verdict === 'fits' ? 'var(--model-local)'
        : fitVerdict.verdict === 'wont-fit' ? 'var(--term-red)'
          : 'var(--amber-bright)',
  );

  let changedFlash = $state<string | null>(null);
  // Set true for the duration of a solver write so the clearing effect below
  // skips the change the solver itself made (otherwise the "Applied: …" flash
  // would be wiped the instant it's set).
  let applyingFit = false;

  function handleFitToGpu() {
    if (!isLocal) return;
    const h = model.hosting as unknown as LlamaServerConfig;
    const res = fitToGpu(h, model.model_identifier, ggufMeta, gpuVramGb);
    if (res.patch) {
      const changed: string[] = [];
      if (res.patch.n_gpu_layers !== h.n_gpu_layers) changed.push(`GPU layers→${res.patch.n_gpu_layers}`);
      if (res.patch.cpu_moe !== h.cpu_moe) changed.push(`all experts on CPU ${res.patch.cpu_moe ? 'on' : 'off'}`);
      if ((res.patch.n_cpu_moe ?? null) !== (h.n_cpu_moe ?? null)) {
        changed.push(res.patch.n_cpu_moe == null ? 'CPU-MoE layers cleared' : `CPU-MoE layers→${res.patch.n_cpu_moe}`);
      }
      applyingFit = true;
      // Spread model.hosting (not the mode-stripped `h`) so the 'local'
      // discriminant survives — update() expects a full HostingMode.
      update('hosting', { ...model.hosting, ...res.patch });
      changedFlash = changed.length ? `Applied: ${changed.join(', ')}` : 'Already optimal — no change';
    } else {
      changedFlash = res.message;
    }
  }

  // A manual edit to any solver-owned/read knob clears the last "changed" flash
  // (it described the pre-edit click). The live chip recomputes on its own.
  // Skips exactly one run after the solver writes, so the solver's own patch
  // doesn't wipe the flash it just produced.
  $effect(() => {
    void (model.hosting as any).n_gpu_layers;
    void (model.hosting as any).cpu_moe;
    void (model.hosting as any).n_cpu_moe;
    void (model.hosting as any).ctx_size;
    if (applyingFit) {
      applyingFit = false;
      return;
    }
    changedFlash = null;
  });

  // Start = hot-swap activate: stops other running local servers (frees VRAM
  // on a single GPU), starts this one, and makes it the active route.
  // Failures are surfaced to the card UI (not just console) — on a single-GPU
  // box a failed start (VRAM OOM, port bind, stale server) is common and the
  // user must see why the spinner stopped.
  let actionError = $state<string | null>(null);

  async function handleStart() {
    busy = true;
    actionError = null;
    try {
      await llmModels.activateModel(model.id);
    } catch (e) {
      console.error('[ModelCard] start failed', e);
      actionError = `Start failed: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      busy = false;
    }
  }

  async function handleStop() {
    busy = true;
    actionError = null;
    try {
      await llmModels.stopModel(model.id);
    } catch (e) {
      console.error('[ModelCard] stop failed', e);
      actionError = `Stop failed: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      busy = false;
    }
  }
</script>

<div class="model-card" class:is-default={isDefault} class:collapsed class:disabled={model.enabled === false} style="--card-accent: {providerMeta.color}">
  <!-- ─── Header ──────────────────────────────────────────── -->
  <div class="card-header">
    <div class="header-left">
      <button
        type="button"
        class="collapse-btn"
        onclick={toggleCollapsed}
        aria-expanded={!collapsed}
        title={collapsed ? 'Expand model settings' : 'Collapse model settings'}
      >{collapsed ? '▸' : '▾'}</button>
      <span class="status-indicator" style="background: {statusColor}" title={statusLabel}></span>
      <div class="header-info">
        <div class="header-top-row">
          <input
            class="display-name"
            type="text"
            value={model.display_name}
            placeholder="Model name"
            onchange={(e) => update('display_name', (e.target as HTMLInputElement).value)}
          />
          {#if isDefault}
            <span class="default-badge">DEFAULT</span>
          {/if}
        </div>
        <div class="header-meta">
          <span class="provider-tag" style="border-color: {providerMeta.color}; color: {providerMeta.color}">
            {providerMeta.label}
          </span>
          <span class="model-id-display" title={model.model_identifier || 'No model ID set'}>
            {model.model_identifier || '(no model selected)'}
          </span>
          {#if status !== 'stopped'}
            <span class="status-text" style="color: {statusColor}">
              {statusLabel}
              {#if latencyMs !== undefined}
                · {latencyMs}ms
              {/if}
            </span>
          {/if}
        </div>
      </div>
    </div>
    <div class="header-actions">
      <button
        type="button"
        class="card-action-btn"
        class:off={model.enabled === false}
        onclick={() => update('enabled', model.enabled === false)}
        title={model.enabled === false
          ? 'Model is disabled — enable to make it available'
          : 'Disable this model (hide from routing and pickers)'}
      >{model.enabled === false ? '⊘ Disabled' : '⏻ Enabled'}</button>
      {#if isLocal}
        {#if status === 'running'}
          <button
            type="button"
            class="card-action-btn"
            onclick={handleStop}
            disabled={busy}
            title="Stop this local server"
          >■ Stop</button>
        {:else}
          <button
            type="button"
            class="card-action-btn"
            onclick={handleStart}
            disabled={busy || status === 'starting'}
            title="Start this model (stops other running local models to free VRAM)"
          >{status === 'starting' ? '… Starting' : '▶ Start'}</button>
        {/if}
      {/if}
      {#if !isDefault && onsetdefault}
        <button
          type="button"
          class="card-action-btn"
          onclick={onsetdefault}
          title="Set as default model"
        >★ Default</button>
      {/if}
    </div>
  </div>

  {#if actionError}
    <div class="card-action-error" role="alert">{actionError}</div>
  {/if}

  <!-- ─── Core settings (always visible) ─────────────────── -->
  <div class="card-body">
    <div class="core-fields">
      {#if isCli}
        <!-- ─── Gemini sign-in panel (CLI provider) ──────────────── -->
        <div class="gemini-signin">
          <div class="gemini-signin-status">
            {#if geminiAuthLoading && geminiAuth === null}
              <span class="gemini-dot checking"></span>
              <span class="gemini-status-text">Checking sign-in…</span>
            {:else if geminiAuth && !geminiAuth.cli_installed}
              <span class="gemini-dot err"></span>
              <span class="gemini-status-text">
                <code>gemini</code> CLI not found —
                <code>npm install -g @google/gemini-cli</code>
              </span>
            {:else if geminiAuth && geminiAuth.authenticated}
              <span class="gemini-dot {geminiAuth.headless_ready ? 'ok' : 'checking'}"></span>
              <span class="gemini-status-text">
                Signed in{#if geminiAuth.account} as <strong>{geminiAuth.account}</strong>{/if}
                {#if !geminiAuth.headless_ready}
                  — <span class="gemini-warn">one-time setup needed for headless use</span>
                {/if}
              </span>
            {:else}
              <span class="gemini-dot off"></span>
              <span class="gemini-status-text">Not signed in</span>
            {/if}
          </div>
          <div class="gemini-signin-actions">
            {#if geminiAuth && geminiAuth.cli_installed && !geminiAuth.authenticated}
              <button type="button" class="rift-btn" onclick={signInToGemini}>
                ▸ Sign in to Gemini
              </button>
            {/if}
            {#if geminiAuth && geminiAuth.authenticated && !geminiAuth.headless_ready}
              <button
                type="button"
                class="rift-btn"
                onclick={finishGeminiSetup}
                disabled={enablingHeadless}
                title="Select the Google login auth method so Rift/Claude can use Gemini non-interactively"
              >{enablingHeadless ? '… Finishing' : '✓ Finish setup'}</button>
            {/if}
            <button
              type="button"
              class="rift-btn ghost"
              onclick={refreshGeminiAuth}
              disabled={geminiAuthLoading}
              title="Re-check sign-in status (after completing the browser login)"
            >↻ Recheck</button>
          </div>
          <span class="field-hint" style="margin-left: 0;">
            Uses your Google login via the <code>gemini</code> CLI — no API key. Claude
            can also route prompts here via the <code>llm_prompt</code> MCP tool.
          </span>
        </div>

        <!-- ─── Gemini model picker ──────────────────────────────── -->
        <div class="field-group">
          <label class="field-lbl" for="mc-gemini-model-{model.id}">
            Model
            <span class="field-hint">Which Gemini model the CLI runs</span>
          </label>
          <select
            id="mc-gemini-model-{model.id}"
            class="field-select"
            value={currentCliModel}
            onchange={(e) => setCliModel((e.target as HTMLSelectElement).value)}
          >
            {#each GEMINI_CLI_MODELS as m}
              <option value={m}>{m}</option>
            {/each}
            {#if currentCliModel && !GEMINI_CLI_MODELS.includes(currentCliModel)}
              <option value={currentCliModel}>{currentCliModel} (custom)</option>
            {/if}
          </select>
        </div>
      {/if}

      {#if !isCli}
      <div class="field-group">
        <label class="field-lbl" for="mc-modelid-{model.id}">
          Model ID
          <span class="field-hint">The model identifier sent to the API</span>
        </label>
        <div class="model-id-input-wrap">
          <input
            id="mc-modelid-{model.id}"
            class="field-input"
            type="text"
            list="models-{model.id}"
            value={model.model_identifier}
            placeholder={isLocal ? 'e.g. gemma-4-27b-it-Q4_K_M.gguf' : 'e.g. claude-sonnet-4-6'}
            onchange={(e) => update('model_identifier', (e.target as HTMLInputElement).value)}
          />
          <datalist id="models-{model.id}">
            {#each (KNOWN_MODELS[model.provider] ?? []) as m}
              <option value={m}>{m}</option>
            {/each}
          </datalist>
        </div>
      </div>
      {/if}

      <div class="field-group">
        <label class="field-lbl" for="mc-endpoint-{model.id}">
          {isCli ? 'Command' : 'Endpoint'}
          <span class="field-hint">
            {isCli
              ? 'Command template — {prompt} is substituted; the Model picker rewrites --model'
              : 'API base URL for requests'}
          </span>
        </label>
        <input
          id="mc-endpoint-{model.id}"
          class="field-input"
          type="text"
          value={model.endpoint}
          placeholder={isLocal
            ? 'http://127.0.0.1:8081'
            : isCli
              ? 'gemini -p {prompt} --model gemini-2.5-flash --skip-trust'
              : 'https://api.example.com'}
          onchange={(e) => update('endpoint', (e.target as HTMLInputElement).value)}
        />
      </div>

      <div class="field-row-2">
        <div class="field-group">
          <label class="field-lbl" for="mc-shortid-{model.id}">
            Short ID
            <span class="field-hint">2-4 char tag for the status line</span>
          </label>
          <input
            id="mc-shortid-{model.id}"
            class="field-input field-narrow"
            type="text"
            value={model.short_id}
            maxlength={4}
            placeholder="LOC"
            onchange={(e) => update('short_id', (e.target as HTMLInputElement).value.toUpperCase())}
          />
        </div>

        {#if !isLocal && !isCli}
          <div class="field-group">
            <label class="field-lbl" for="mc-apikey-{model.id}">
              API Key
              <span class="field-hint">Stored in OS keyring, not config file</span>
            </label>
            <input
              id="mc-apikey-{model.id}"
              class="field-input"
              type="password"
              value={model.api_key_ref ?? ''}
              placeholder="sk-..."
              onchange={(e) => update('api_key_ref', (e.target as HTMLInputElement).value || null)}
            />
          </div>
        {/if}
      </div>
    </div>

    <!-- ─── Capabilities (inline, editable) ────────────────── -->
    <div class="caps-row">
      <div class="cap-item">
        <label class="field-lbl" for="mc-ctx-cap-{model.id}">Context</label>
        <div class="cap-value">
          <input
            id="mc-ctx-cap-{model.id}"
            class="field-input field-narrow"
            type="number"
            value={caps.max_context_tokens}
            min={1024}
            step={1024}
            onchange={(e) => updateCapability('max_context_tokens', parseInt((e.target as HTMLInputElement).value))}
          />
          <span class="cap-unit">tokens</span>
        </div>
      </div>
      <div class="cap-item">
        <label class="field-lbl cap-toggle-lbl">
          <input
            type="checkbox"
            checked={caps.supports_streaming}
            onchange={(e) => updateCapability('supports_streaming', (e.target as HTMLInputElement).checked)}
          />
          Streaming
        </label>
      </div>
      <div class="cap-item">
        <label class="field-lbl cap-toggle-lbl">
          <input
            type="checkbox"
            checked={caps.supports_tool_use}
            onchange={(e) => updateCapability('supports_tool_use', (e.target as HTMLInputElement).checked)}
          />
          Tool Use
        </label>
      </div>
      {#if caps.cost_per_1m_input > 0 || caps.cost_per_1m_output > 0}
        <div class="cap-item">
          <span class="cap-cost">
            ${caps.cost_per_1m_input}/{caps.cost_per_1m_output} per 1M
          </span>
        </div>
      {/if}
    </div>

    <!-- ─── Local server config ─────────────────────────────── -->
    {#if isLocal}
      <fieldset class="llama-config">
        <legend>llama-server Configuration</legend>

        <div class="field-group">
          <label class="field-lbl" for="mc-gguf-{model.id}">
            GGUF Model Path
            <span class="field-hint">Full path to the .gguf model file on disk</span>
          </label>
          <input
            id="mc-gguf-{model.id}"
            class="field-input"
            type="text"
            value={(model.hosting as any).model_path ?? ''}
            placeholder="C:\Models\model.gguf"
            onchange={(e) => {
              const h = { ...model.hosting, model_path: (e.target as HTMLInputElement).value };
              update('hosting', h);
            }}
          />
        </div>

        <div class="field-row-3">
          <div class="field-group">
            <label class="field-lbl" for="mc-ctx-{model.id}">
              Context Size
              <span class="field-hint">Max tokens in context window</span>
            </label>
            <input
              id="mc-ctx-{model.id}"
              class="field-input"
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

          <div class="field-group">
            <label class="field-lbl" for="mc-gpu-{model.id}">
              GPU Layers
              <span class="field-hint">99 = all · -1 = auto-fit to free VRAM</span>
            </label>
            <input
              id="mc-gpu-{model.id}"
              class="field-input"
              type="number"
              value={(model.hosting as any).n_gpu_layers ?? 99}
              min={-1}
              max={999}
              onchange={(e) => {
                const h = { ...model.hosting, n_gpu_layers: parseInt((e.target as HTMLInputElement).value) };
                update('hosting', h);
              }}
            />
          </div>

          <div class="field-group">
            <label class="field-lbl" for="mc-port-{model.id}">
              Port
              <span class="field-hint">HTTP port for the server</span>
            </label>
            <input
              id="mc-port-{model.id}"
              class="field-input"
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

        <div class="field-row-3">
          <div class="field-group">
            <label class="field-lbl" for="mc-kvk-{model.id}">
              KV Cache Key
              <span class="field-hint">Quantization type for key cache</span>
            </label>
            <select
              id="mc-kvk-{model.id}"
              class="field-select"
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

          <div class="field-group">
            <label class="field-lbl" for="mc-kvv-{model.id}">
              KV Cache Value
              <span class="field-hint">Quantization type for value cache</span>
            </label>
            <select
              id="mc-kvv-{model.id}"
              class="field-select"
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

          <div class="field-group">
            <label class="field-lbl cap-toggle-lbl">
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

        <!-- ─── MoE expert offload (VRAM savings for MoE models) ─── -->
        <div class="field-row-2" style="margin-top: var(--space-md)">
          <div class="field-group">
            <label class="field-lbl cap-toggle-lbl">
              <input
                type="checkbox"
                checked={(model.hosting as any).cpu_moe ?? false}
                onchange={(e) => {
                  const h = { ...model.hosting, cpu_moe: (e.target as HTMLInputElement).checked };
                  update('hosting', h);
                }}
              />
              Offload MoE experts to CPU
            </label>
            <span class="field-hint">Forces ALL experts to RAM regardless of free VRAM — use only when the model won't fit. To fill VRAM, leave this off and tune CPU MoE Layers.</span>
          </div>

          {#if !((model.hosting as any).cpu_moe ?? false)}
            <div class="field-group">
              <label class="field-lbl" for="mc-ncpumoe-{model.id}">
                CPU MoE Layers
                <span class="field-hint">Offload experts for first N layers (blank = none)</span>
              </label>
              <input
                id="mc-ncpumoe-{model.id}"
                class="field-input field-narrow"
                type="number"
                value={(model.hosting as any).n_cpu_moe ?? ''}
                min={0}
                max={999}
                placeholder="—"
                onchange={(e) => {
                  const raw = (e.target as HTMLInputElement).value;
                  const n = raw === '' ? null : parseInt(raw);
                  const h = { ...model.hosting, n_cpu_moe: Number.isNaN(n as number) ? null : n };
                  update('hosting', h);
                }}
              />
            </div>
          {/if}
        </div>

        <div class="field-group" style="margin-top: var(--space-md)">
          <label class="field-lbl" for="mc-cacheram-{model.id}">
            Prompt Cache RAM (MiB)
            <span class="field-hint">Host-RAM cache for prompt reuse (default 8192 = 8 GB). 0 disables it. Not model weights — lower it to reclaim system RAM.</span>
          </label>
          <input
            id="mc-cacheram-{model.id}"
            class="field-input field-narrow"
            type="number"
            value={(model.hosting as any).cache_ram ?? ''}
            min={0}
            max={65536}
            step={512}
            placeholder="8192"
            onchange={(e) => {
              const raw = (e.target as HTMLInputElement).value;
              const n = raw === '' ? null : parseInt(raw);
              const h = { ...model.hosting, cache_ram: Number.isNaN(n as number) ? null : n };
              update('hosting', h);
            }}
          />
        </div>

        <label class="field-lbl cap-toggle-lbl" style="margin-top: var(--space-md)">
          <input
            type="checkbox"
            checked={(model.hosting as any).auto_start ?? false}
            onchange={(e) => {
              const h = { ...model.hosting, auto_start: (e.target as HTMLInputElement).checked };
              update('hosting', h);
            }}
          />
          Auto-start when Rift launches
        </label>

        <label class="field-lbl cap-toggle-lbl" style="margin-top: var(--space-sm)">
          <input
            type="checkbox"
            checked={(model.hosting as any).auto_restart ?? false}
            onchange={(e) => {
              const h = { ...model.hosting, auto_restart: (e.target as HTMLInputElement).checked };
              update('hosting', h);
            }}
          />
          Auto-restart if it crashes
          <span class="field-hint">Re-launches this server on crash (up to 3 times/min, then stops to avoid loops).</span>
        </label>

        <!-- ─── "Fit to my GPU" auto-tuner (candidate #789) ─── -->
        <div class="fit-row">
          <button
            type="button"
            class="rift-btn fit-btn"
            onclick={handleFitToGpu}
            title="Compute the best GPU-layer / expert-offload split to fit this model in detected VRAM, and apply it"
          >▣ Fit to my GPU</button>
          {#if fitVerdict}
            <span
              class="fit-chip"
              style:color={fitChipColor}
              style:border-color={fitChipColor}
              title={fitVerdict.message}
            >
              {fitVerdict.verdict === 'fits' ? '✓ fits'
                : fitVerdict.verdict === 'fits-experts-cpu' ? '◐ fits (experts on CPU)'
                : fitVerdict.verdict === 'fits-partial-layers' ? '◐ fits (partial layers)'
                : '✕ won’t fit'}
            </span>
          {/if}
        </div>
        {#if changedFlash}
          <div class="fit-flash">{changedFlash}</div>
        {/if}

        <VramEstimator
          config={model.hosting as unknown as LlamaServerConfig}
          modelName={model.model_identifier}
          meta={ggufMeta}
          {gpuVramGb}
        />
      </fieldset>
    {/if}

    <!-- ─── Advanced toggle (strength tags, cost) ──────────── -->
    <button
      type="button"
      class="advanced-toggle"
      onclick={() => (showAdvanced = !showAdvanced)}
      aria-expanded={showAdvanced}
    >
      {showAdvanced ? '▾' : '▸'} Advanced
    </button>

    {#if showAdvanced}
      <div class="advanced-section">
        <div class="field-row-2">
          <div class="field-group">
            <label class="field-lbl" for="mc-cost-in-{model.id}">
              Cost / 1M input tokens
              <span class="field-hint">For cost-optimized routing</span>
            </label>
            <input
              id="mc-cost-in-{model.id}"
              class="field-input field-narrow"
              type="number"
              value={caps.cost_per_1m_input}
              min={0}
              step={0.1}
              onchange={(e) => updateCapability('cost_per_1m_input', parseFloat((e.target as HTMLInputElement).value))}
            />
          </div>
          <div class="field-group">
            <label class="field-lbl" for="mc-cost-out-{model.id}">
              Cost / 1M output tokens
              <span class="field-hint">For cost-optimized routing</span>
            </label>
            <input
              id="mc-cost-out-{model.id}"
              class="field-input field-narrow"
              type="number"
              value={caps.cost_per_1m_output}
              min={0}
              step={0.1}
              onchange={(e) => updateCapability('cost_per_1m_output', parseFloat((e.target as HTMLInputElement).value))}
            />
          </div>
        </div>

        <div class="field-group">
          <label class="field-lbl" for="mc-tags-{model.id}">
            Strength Tags
            <span class="field-hint">Comma-separated tags like "code, reasoning, creative"</span>
          </label>
          <input
            id="mc-tags-{model.id}"
            class="field-input"
            type="text"
            value={(caps.strength_tags ?? []).join(', ')}
            placeholder="code, reasoning, creative"
            onchange={(e) => {
              const tags = (e.target as HTMLInputElement).value
                .split(',')
                .map(t => t.trim())
                .filter(Boolean);
              updateCapability('strength_tags', tags);
            }}
          />
        </div>
      </div>
    {/if}

    <!-- ─── Card footer ─────────────────────────────────────── -->
    <div class="card-footer">
      {#if !confirmRemove}
        <button type="button" class="remove-btn" onclick={() => (confirmRemove = true)}>
          Remove Model
        </button>
      {:else}
        <span class="confirm-prompt">Remove this model?</span>
        <button type="button" class="remove-btn confirm" onclick={() => { confirmRemove = false; onremove(); }}>
          Yes, remove
        </button>
        <button type="button" class="cancel-btn" onclick={() => (confirmRemove = false)}>
          Cancel
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .model-card {
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-left: 3px solid var(--card-accent, var(--amber-faint));
    border-radius: var(--radius-md);
    margin-bottom: var(--space-md);
    overflow: hidden;
  }
  .model-card.is-default {
    border-color: var(--amber-bright);
    border-left-color: var(--amber-bright);
    box-shadow: 0 0 8px rgba(255, 200, 64, 0.08);
  }
  /* Collapsed: header only, body hidden for an at-a-glance list. */
  .model-card.collapsed .card-body {
    display: none;
  }
  /* Disabled: dimmed + muted accent; still editable, just not routable. */
  .model-card.disabled {
    opacity: 0.55;
    border-left-color: var(--border-subtle);
  }
  .model-card.disabled .status-indicator {
    filter: grayscale(1);
  }
  .card-action-btn.off {
    color: var(--term-red);
    border-color: var(--border-red-strong);
  }

  .collapse-btn {
    flex-shrink: 0;
    background: transparent;
    border: none;
    color: var(--amber-faint);
    cursor: pointer;
    font-size: var(--text-sm);
    line-height: 1;
    padding: 2px var(--space-xs);
    margin-top: 2px;
    border-radius: var(--radius-sm);
  }
  .collapse-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .collapse-btn:hover {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.08);
  }

  /* ─── Header ──────────────────────────────── */
  .card-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-md) var(--space-lg);
    background: rgba(0, 0, 0, 0.15);
  }
  .header-left {
    display: flex;
    align-items: flex-start;
    gap: var(--space-md);
    flex: 1;
    min-width: 0;
  }
  .status-indicator {
    width: 10px;
    height: 10px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
    margin-top: 5px;
  }
  .header-info {
    flex: 1;
    min-width: 0;
  }
  .header-top-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .display-name {
    flex: 1;
    background: transparent;
    border: none;
    border-bottom: 1px solid transparent;
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    font-weight: 700;
    padding: 2px 4px;
    min-width: 0;
  }
  .display-name:hover {
    border-bottom-color: var(--border-subtle);
  }
  .display-name:focus {
    border-bottom-color: var(--amber-faint);
  }
  .display-name:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .default-badge {
    font-size: var(--text-2xs, 9px);
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--bg-base);
    background: var(--amber-bright);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .header-meta {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-top: 4px;
    flex-wrap: wrap;
  }
  .provider-tag {
    font-size: var(--text-2xs, 9px);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 1px 6px;
    white-space: nowrap;
  }
  .model-id-display {
    font-size: var(--text-2xs, 9px);
    color: var(--amber-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }
  .status-text {
    font-size: var(--text-2xs, 9px);
    font-weight: 600;
    letter-spacing: 0.03em;
  }
  .header-actions {
    flex-shrink: 0;
  }
  /* Inline failure surface for start/stop — mirrors .rift-badge--error tints. */
  .card-action-error {
    margin: var(--space-8) var(--space-12) 0;
    padding: var(--space-xs) var(--space-8);
    background: var(--bg-red-notice);
    border: 1px solid var(--border-red-strong);
    border-radius: var(--radius-sm);
    color: var(--term-red);
    font-size: var(--text-2xs, 9px);
    letter-spacing: 0.03em;
  }
  .card-action-btn {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-2xs, 9px);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 4px 10px;
    cursor: pointer;
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out);
    white-space: nowrap;
  }
  .card-action-btn:hover {
    color: var(--amber-bright);
    border-color: var(--amber-dim);
    background: var(--bg-hover);
  }
  .card-action-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .card-action-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ─── Body ────────────────────────────────── */
  .card-body {
    padding: var(--space-md) var(--space-lg) var(--space-lg);
  }
  .core-fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  /* ─── Field system ────────────────────────── */
  .field-group {
    margin-bottom: var(--space-sm);
  }
  .field-lbl {
    display: block;
    font-size: var(--text-2xs, 9px);
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 700;
    margin-bottom: 4px;
  }
  .field-hint {
    display: inline;
    font-weight: 400;
    text-transform: none;
    letter-spacing: normal;
    color: var(--amber-faint);
    font-style: italic;
    margin-left: 6px;
  }
  .field-input {
    width: 100%;
    background: rgba(0, 0, 0, 0.35);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    padding: 6px 10px;
    box-sizing: border-box;
    transition: border-color var(--duration-base) var(--ease-out);
  }
  .field-input:hover {
    border-color: var(--amber-faint);
  }
  .field-input:focus {
    border-color: var(--amber-primary);
    outline: none;
    box-shadow: 0 0 0 1px var(--amber-dim);
  }
  .field-input:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .field-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .field-narrow {
    width: 100px;
  }
  .field-select {
    appearance: none;
    -webkit-appearance: none;
    width: 100%;
    background: rgba(0, 0, 0, 0.35);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    padding: 6px 28px 6px 10px;
    cursor: pointer;
    transition: border-color var(--duration-base) var(--ease-out);
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' stroke='%23A87830' stroke-width='1.5' fill='none' stroke-linecap='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 10px center;
  }
  .field-select:hover {
    border-color: var(--amber-faint);
  }
  .field-select:focus {
    border-color: var(--amber-primary);
    outline: none;
    box-shadow: 0 0 0 1px var(--amber-dim);
  }
  .field-select:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }
  .field-select option {
    background: var(--bg-elevated);
    color: var(--amber-warm);
  }

  .model-id-input-wrap {
    position: relative;
  }

  .field-row-2 {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-md);
  }
  .field-row-3 {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: var(--space-md);
  }

  /* ─── Capabilities row ────────────────────── */
  .caps-row {
    display: flex;
    align-items: center;
    gap: var(--space-lg);
    flex-wrap: wrap;
    padding: var(--space-sm) 0;
    margin-top: var(--space-xs);
    border-top: 1px solid rgba(168, 120, 48, 0.1);
    border-bottom: 1px solid rgba(168, 120, 48, 0.1);
  }
  .cap-item {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .cap-item .field-input {
    width: 80px;
    padding: 3px 6px;
    font-size: var(--text-2xs, 9px);
    height: auto;
  }
  .cap-unit {
    font-size: var(--text-2xs, 9px);
    color: var(--amber-faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .cap-value {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .cap-toggle-lbl {
    display: inline-flex !important;
    align-items: center;
    gap: var(--space-sm);
    cursor: pointer;
    color: var(--amber-warm);
    text-transform: uppercase;
    font-weight: 600;
  }
  .cap-toggle-lbl input[type="checkbox"] {
    accent-color: var(--amber-primary);
    margin: 0;
  }
  .cap-cost {
    font-size: var(--text-2xs, 9px);
    color: var(--amber-faint);
    letter-spacing: 0.03em;
  }

  /* ─── llama-server fieldset ───────────────── */
  .llama-config {
    border: 1px solid rgba(168, 120, 48, 0.2);
    border-radius: var(--radius-md);
    padding: var(--space-md);
    margin-top: var(--space-md);
  }
  .llama-config legend {
    font-size: var(--text-2xs, 9px);
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 700;
    padding: 0 6px;
  }

  /* ─── Fit-to-GPU row ─────────────────────── */
  .fit-row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    margin-top: var(--space-md);
    flex-wrap: wrap;
  }
  .fit-chip {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 1px var(--space-sm);
    white-space: nowrap;
  }
  .fit-flash {
    margin-top: var(--space-xs);
    font-size: var(--text-2xs);
    color: var(--amber-warm);
    font-style: italic;
  }

  /* ─── Gemini sign-in panel (CLI provider) ─── */
  .gemini-signin {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    margin-bottom: var(--space-sm);
    background: rgba(255, 168, 38, 0.05);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-md);
  }
  .gemini-signin-status {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .gemini-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }
  .gemini-dot.ok { background: var(--term-green); }
  .gemini-dot.off { background: var(--amber-faint); }
  .gemini-dot.err { background: var(--term-red); }
  .gemini-dot.checking { background: var(--amber-bright); }
  .gemini-status-text {
    font-size: var(--text-xs);
    color: var(--term-white);
  }
  .gemini-status-text code {
    font-size: var(--text-2xs);
    color: var(--amber-warm);
    background: rgba(0, 0, 0, 0.3);
    padding: 1px 4px;
    border-radius: var(--radius-sm);
  }
  .gemini-status-text strong { color: var(--amber-bright); }
  .gemini-warn { color: var(--amber-bright); }
  .gemini-signin-actions {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }
  .rift-btn.ghost {
    background: transparent;
  }

  /* ─── Advanced toggle ─────────────────────── */
  .advanced-toggle {
    background: none;
    border: none;
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-2xs, 9px);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    padding: var(--space-sm) 0;
    margin-top: var(--space-sm);
    transition: color var(--duration-base) var(--ease-out);
  }
  .advanced-toggle:hover {
    color: var(--amber-warm);
  }
  .advanced-toggle:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 2px;
  }
  .advanced-section {
    padding-top: var(--space-sm);
  }

  /* ─── Footer ──────────────────────────────── */
  .card-footer {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-top: var(--space-md);
    padding-top: var(--space-sm);
    border-top: 1px solid rgba(168, 120, 48, 0.08);
  }
  .remove-btn {
    background: none;
    border: 1px solid var(--border-red-tint);
    border-radius: var(--radius-md);
    color: var(--term-red);
    font-family: var(--font-family);
    font-size: var(--text-2xs, 9px);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 4px 10px;
    cursor: pointer;
    transition: background var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out);
  }
  .remove-btn:hover {
    background: var(--bg-red-notice);
    border-color: rgba(255, 72, 72, 0.5);
  }
  .remove-btn.confirm {
    background: rgba(255, 72, 72, 0.15);
    border-color: var(--term-red);
  }
  .remove-btn:focus-visible {
    outline: 1px solid var(--term-red);
    outline-offset: 1px;
  }
  .cancel-btn {
    background: none;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-2xs, 9px);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 4px 10px;
    cursor: pointer;
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out);
  }
  .cancel-btn:hover {
    color: var(--amber-warm);
    border-color: var(--amber-faint);
  }
  .cancel-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .confirm-prompt {
    font-size: var(--text-2xs, 9px);
    color: var(--term-red);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
</style>
