<script lang="ts">
  // Phase 8.7l — Settings panel. Lives inside a popout shell.
  //
  // Sections:
  //   - About        — version + identifier (from @tauri-apps/api/app)
  //   - Updates      — status + manual `check()` button (the auto-check on
  //                    session start is in App.svelte; this lets users
  //                    poll on-demand without restarting)
  //   - Project      — current root + Switch button → ProjectPicker popout
  //   - Filesystem   — ignore-globs editor + max-depth (config_save)
  //   - Index        — sync mode toggle (config_save)
  //   - Notifications — link to NotifManager popout
  //
  // Self-contained: reads config via `config_get`, writes via `config_save`.
  // Save buttons are scoped per-section so users can edit one knob without
  // touching others.

  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getVersion, getName } from '@tauri-apps/api/app';
  import { check, type Update } from '@tauri-apps/plugin-updater';
  import { popouts } from './popouts.svelte';

  interface Props {
    popoutId: number;
  }

  let { popoutId }: Props = $props();

  // ---------------------------------------------------------------------
  // About
  // ---------------------------------------------------------------------

  let appVersion = $state('—');
  let appName = $state('Rift');
  let appIdentifier = $state('com.abyssal.rift');

  // ---------------------------------------------------------------------
  // Updates
  // ---------------------------------------------------------------------

  type UpdateState =
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'up-to-date'; checkedAt: number }
    | { kind: 'available'; update: Update }
    | { kind: 'installing' }
    | { kind: 'error'; message: string };

  let updateState = $state<UpdateState>({ kind: 'idle' });

  async function manualCheck() {
    updateState = { kind: 'checking' };
    try {
      const result = await check();
      if (result) {
        updateState = { kind: 'available', update: result };
      } else {
        updateState = { kind: 'up-to-date', checkedAt: Date.now() };
      }
    } catch (err) {
      updateState = { kind: 'error', message: String(err) };
    }
  }

  async function installUpdate() {
    if (updateState.kind !== 'available') return;
    const upd = updateState.update;
    updateState = { kind: 'installing' };
    try {
      await upd.downloadAndInstall();
      // Tauri restarts the app on success — code below the await won't run.
    } catch (err) {
      updateState = { kind: 'error', message: String(err) };
    }
  }

  // ---------------------------------------------------------------------
  // Project + Filesystem + Index (loaded from RiftConfig)
  // ---------------------------------------------------------------------

  interface ProjectEntry {
    name: string;
    path: string;
    last_used_ms: number;
  }
  interface FsConfig {
    ignore_globs: string[];
    max_depth: number;
  }
  interface IndexConfig {
    ignore_globs: string[];
    sync_mode: 'live' | 'manual' | 'unknown';
    camera_transform?: unknown;
    node_positions?: unknown;
  }
  interface CockpitConfig {
    detached_pos?: unknown;
  }
  interface McpConfig {
    enabled: boolean;
    allow_inspection: boolean;
    allow_js_eval: boolean;
    allow_mutations: boolean;
  }
  interface RiftConfig {
    projects: ProjectEntry[];
    fs: FsConfig;
    cockpit: CockpitConfig;
    index: IndexConfig;
    mcp: McpConfig;
  }
  interface McpStatus {
    enabled: boolean;
    token_present: boolean;
    token_path: string;
  }

  let config = $state<RiftConfig | null>(null);
  let configError = $state<string | null>(null);

  // Editable copies — diffed against `config` to enable Save buttons.
  let fsIgnoreText = $state('');
  let fsMaxDepth = $state(0);
  let indexSyncMode = $state<'live' | 'manual'>('live');

  let savingFs = $state(false);
  let savingIndex = $state(false);
  let savingMcp = $state(false);
  let saveBanner = $state<{ section: 'fs' | 'index' | 'mcp'; ok: boolean; msg: string } | null>(
    null,
  );

  // MCP — D-014 Phase A
  let mcpToken = $state<string | null>(null);
  let mcpTokenVisible = $state(false);
  let mcpStatus = $state<McpStatus | null>(null);

  async function refreshMcpStatus() {
    try {
      mcpStatus = await invoke<McpStatus>('mcp_status');
    } catch (err) {
      console.warn('[Settings] mcp_status failed', err);
    }
  }

  async function onToggleMcp(value: boolean, key: keyof McpConfig) {
    if (!config) return;
    savingMcp = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        mcp: { ...config.mcp, [key]: value },
      };
      await invoke('config_save', { config: next });
      config = next;
      // Generating-on-enable so the token is ready when the user copies it.
      if (key === 'enabled' && value && !mcpToken) {
        try {
          mcpToken = await invoke<string>('mcp_token_get');
        } catch (err) {
          console.warn('[Settings] mcp_token_get failed', err);
        }
      }
      await refreshMcpStatus();
      saveBanner = { section: 'mcp', ok: true, msg: 'mcp settings saved' };
    } catch (err) {
      saveBanner = { section: 'mcp', ok: false, msg: String(err) };
    } finally {
      savingMcp = false;
    }
  }

  async function revealMcpToken() {
    if (!mcpToken) {
      try {
        mcpToken = await invoke<string>('mcp_token_get');
      } catch (err) {
        saveBanner = { section: 'mcp', ok: false, msg: String(err) };
        return;
      }
    }
    mcpTokenVisible = !mcpTokenVisible;
  }

  async function copyMcpToken() {
    if (!mcpToken) return;
    try {
      await navigator.clipboard.writeText(mcpToken);
      saveBanner = { section: 'mcp', ok: true, msg: 'token copied to clipboard' };
    } catch (err) {
      saveBanner = { section: 'mcp', ok: false, msg: String(err) };
    }
  }

  async function regenerateMcpToken() {
    try {
      mcpToken = await invoke<string>('mcp_token_regenerate');
      mcpTokenVisible = true;
      await refreshMcpStatus();
      saveBanner = {
        section: 'mcp',
        ok: true,
        msg: 'new token issued — update connected clients',
      };
    } catch (err) {
      saveBanner = { section: 'mcp', ok: false, msg: String(err) };
    }
  }

  const currentProject = $derived(
    config && config.projects.length > 0 ? config.projects[0] : null
  );

  const fsDirty = $derived(
    config !== null
    && (fsIgnoreText.trim().split('\n').map((s) => s.trim()).filter(Boolean).join('\n')
        !== config.fs.ignore_globs.join('\n')
        || fsMaxDepth !== config.fs.max_depth)
  );
  const indexDirty = $derived(
    config !== null && indexSyncMode !== config.index.sync_mode
  );

  function snapshotIntoEditState(c: RiftConfig) {
    fsIgnoreText = c.fs.ignore_globs.join('\n');
    fsMaxDepth = c.fs.max_depth;
    indexSyncMode = c.index.sync_mode === 'manual' ? 'manual' : 'live';
  }

  async function reloadConfig() {
    try {
      const c = await invoke<RiftConfig>('config_get');
      config = c;
      snapshotIntoEditState(c);
      configError = null;
    } catch (err) {
      configError = String(err);
      console.error('[Settings] config_get failed', err);
    }
  }

  async function saveFsConfig() {
    if (!config) return;
    savingFs = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        fs: {
          ...config.fs,
          ignore_globs: fsIgnoreText
            .split('\n')
            .map((s) => s.trim())
            .filter(Boolean),
          max_depth: Math.max(1, Math.min(64, Math.floor(fsMaxDepth || 1))),
        },
      };
      await invoke('config_save', { config: next });
      config = next;
      saveBanner = { section: 'fs', ok: true, msg: 'filesystem settings saved' };
    } catch (err) {
      saveBanner = { section: 'fs', ok: false, msg: String(err) };
    } finally {
      savingFs = false;
    }
  }

  async function saveIndexConfig() {
    if (!config) return;
    savingIndex = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        index: {
          ...config.index,
          sync_mode: indexSyncMode,
        },
      };
      await invoke('config_save', { config: next });
      config = next;
      saveBanner = { section: 'index', ok: true, msg: 'index settings saved' };
    } catch (err) {
      saveBanner = { section: 'index', ok: false, msg: String(err) };
    } finally {
      savingIndex = false;
    }
  }

  function openProjectPicker() {
    popouts.summon({
      content: { kind: 'project-picker' },
      width: 'min(640px, 80vw)',
    });
  }

  function done() {
    popouts.dismiss(popoutId);
  }

  function formatCheckedAt(ts: number): string {
    const ageMs = Date.now() - ts;
    if (ageMs < 60_000) return 'just now';
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  }

  onMount(() => {
    void (async () => {
      try {
        appVersion = await getVersion();
      } catch (err) {
        console.warn('[Settings] getVersion failed', err);
      }
      try {
        appName = await getName();
      } catch {
        // fall back to default
      }
      try {
        // tauri.conf.json identifier — exposed via @tauri-apps/api/app's
        // getIdentifier (Tauri 2.x). Dynamic-call so older Tauri runtimes
        // without the export don't blow up at module load.
        const mod = await import('@tauri-apps/api/app');
        const getId = (mod as Record<string, unknown>).getIdentifier;
        if (typeof getId === 'function') {
          appIdentifier = await (getId as () => Promise<string>)();
        }
      } catch {
        // no-op — fall back to default
      }
      await reloadConfig();
      await refreshMcpStatus();
    })();
  });
</script>

<div class="settings"
     onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); done(); } }}
     role="dialog"
     tabindex="-1"
>
  <div class="settings-body">

    <!-- ABOUT -->
    <section class="section">
      <div class="section-label">About</div>
      <div class="kv">
        <div class="k">app</div>
        <div class="v">{appName}</div>
      </div>
      <div class="kv">
        <div class="k">version</div>
        <div class="v">{appVersion}</div>
      </div>
      <div class="kv">
        <div class="k">identifier</div>
        <div class="v">{appIdentifier}</div>
      </div>
    </section>

    <!-- UPDATES -->
    <section class="section">
      <div class="section-label">Updates</div>
      <div class="hint">
        auto-check runs on session start; manual check below for on-demand polling.
      </div>
      <div class="row">
        <button
          type="button"
          class="btn"
          onclick={manualCheck}
          disabled={updateState.kind === 'checking' || updateState.kind === 'installing'}
        >
          {#if updateState.kind === 'checking'}
            checking…
          {:else}
            check for updates now
          {/if}
        </button>
      </div>

      {#if updateState.kind === 'up-to-date'}
        <div class="banner banner-ok">
          you're on the latest version · checked {formatCheckedAt(updateState.checkedAt)}
        </div>
      {:else if updateState.kind === 'available'}
        <div class="banner banner-info">
          <div class="banner-title">update available — v{updateState.update.version}</div>
          {#if updateState.update.body}
            <div class="banner-body">{updateState.update.body.slice(0, 240)}{updateState.update.body.length > 240 ? '…' : ''}</div>
          {/if}
          <div class="banner-actions">
            <button type="button" class="btn primary" onclick={installUpdate}>
              install + restart
            </button>
          </div>
        </div>
      {:else if updateState.kind === 'installing'}
        <div class="banner banner-info">installing… app will restart automatically.</div>
      {:else if updateState.kind === 'error'}
        <div class="banner banner-fail">{updateState.message}</div>
      {/if}
    </section>

    <!-- PROJECT -->
    <section class="section">
      <div class="section-label">Project</div>
      {#if currentProject}
        <div class="kv">
          <div class="k">name</div>
          <div class="v">{currentProject.name}</div>
        </div>
        <div class="kv">
          <div class="k">path</div>
          <div class="v path">{currentProject.path}</div>
        </div>
      {:else}
        <div class="hint">no project active</div>
      {/if}
      <div class="row">
        <button type="button" class="btn" onclick={openProjectPicker}>
          switch project…
        </button>
      </div>
    </section>

    <!-- FILESYSTEM -->
    <section class="section">
      <div class="section-label">Filesystem</div>
      {#if configError}
        <div class="banner banner-fail">{configError}</div>
      {:else if !config}
        <div class="hint">loading…</div>
      {:else}
        <label class="field">
          <span class="field-label">ignore globs · one per line</span>
          <textarea
            class="field-input"
            bind:value={fsIgnoreText}
            rows="6"
            spellcheck="false"
          ></textarea>
        </label>
        <label class="field">
          <span class="field-label">max walk depth · 1–64</span>
          <input
            type="number"
            class="field-input field-narrow"
            bind:value={fsMaxDepth}
            min="1"
            max="64"
          />
        </label>
        <div class="row">
          <button
            type="button"
            class="btn primary"
            disabled={!fsDirty || savingFs}
            onclick={saveFsConfig}
          >
            {savingFs ? 'saving…' : 'save filesystem'}
          </button>
          {#if saveBanner && saveBanner.section === 'fs'}
            <span class="banner-inline" class:fail={!saveBanner.ok}>
              {saveBanner.msg}
            </span>
          {/if}
        </div>
      {/if}
    </section>

    <!-- INDEX -->
    {#if config}
      <section class="section">
        <div class="section-label">Index</div>
        <div class="hint">
          how the vault graph stays in sync with disk. live = filesystem watcher pushes updates as they happen; manual = only on rescan.
        </div>
        <div class="radio-row">
          <label class="radio">
            <input
              type="radio"
              name="sync-mode"
              value="live"
              checked={indexSyncMode === 'live'}
              onchange={() => (indexSyncMode = 'live')}
            />
            <span>live</span>
          </label>
          <label class="radio">
            <input
              type="radio"
              name="sync-mode"
              value="manual"
              checked={indexSyncMode === 'manual'}
              onchange={() => (indexSyncMode = 'manual')}
            />
            <span>manual</span>
          </label>
        </div>
        <div class="row">
          <button
            type="button"
            class="btn primary"
            disabled={!indexDirty || savingIndex}
            onclick={saveIndexConfig}
          >
            {savingIndex ? 'saving…' : 'save index'}
          </button>
          {#if saveBanner && saveBanner.section === 'index'}
            <span class="banner-inline" class:fail={!saveBanner.ok}>
              {saveBanner.msg}
            </span>
          {/if}
        </div>
      </section>
    {/if}

    <!-- MCP — D-014 Phase A -->
    {#if config}
      <section class="section">
        <div class="section-label">MCP server</div>
        <div class="hint">
          expose this Rift instance to MCP-aware clients (Claude Code, automation
          harnesses) over stdio JSON-RPC. localhost-only · off by default ·
          audit-logged via <code>Category::Mcp</code> on the bus.
        </div>

        <div class="row">
          <label class="kv-toggle">
            <input
              type="checkbox"
              checked={config.mcp.enabled}
              onchange={(e) => onToggleMcp((e.target as HTMLInputElement).checked, 'enabled')}
              disabled={savingMcp}
            />
            <span>enable MCP server</span>
          </label>
        </div>

        {#if config.mcp.enabled}
          <div class="row">
            <label class="kv-toggle">
              <input
                type="checkbox"
                checked={config.mcp.allow_inspection}
                onchange={(e) => onToggleMcp((e.target as HTMLInputElement).checked, 'allow_inspection')}
                disabled={savingMcp}
              />
              <span>allow DOM snapshot + screenshot (Phase C)</span>
            </label>
          </div>
          <div class="row">
            <label class="kv-toggle">
              <input
                type="checkbox"
                checked={config.mcp.allow_js_eval}
                onchange={(e) => onToggleMcp((e.target as HTMLInputElement).checked, 'allow_js_eval')}
                disabled={savingMcp}
              />
              <span>allow <code>js_eval</code> (Phase C)</span>
            </label>
          </div>
          <div class="row">
            <label class="kv-toggle">
              <input
                type="checkbox"
                checked={config.mcp.allow_mutations}
                onchange={(e) => onToggleMcp((e.target as HTMLInputElement).checked, 'allow_mutations')}
                disabled={savingMcp}
              />
              <span>allow mutating tools — pty_input, fs_write, git_action (Phase D)</span>
            </label>
          </div>

          <div class="kv">
            <div class="k">token</div>
            <div class="v mono-wrap">
              {#if mcpTokenVisible && mcpToken}
                {mcpToken}
              {:else}
                ••••••••••••••••••••••••••••••••
              {/if}
            </div>
          </div>
          <div class="row">
            <button type="button" class="btn" onclick={revealMcpToken}>
              {mcpTokenVisible ? 'hide token' : 'reveal token'}
            </button>
            <button type="button" class="btn" onclick={copyMcpToken} disabled={!mcpToken}>
              copy
            </button>
            <button type="button" class="btn" onclick={regenerateMcpToken}>
              regenerate
            </button>
          </div>
          {#if mcpStatus}
            <div class="kv">
              <div class="k">token path</div>
              <div class="v mono-wrap">{mcpStatus.token_path}</div>
            </div>
          {/if}
          <div class="hint">
            client config: pass <code>--token &lt;value&gt;</code> or set
            <code>RIFT_MCP_TOKEN</code> when launching <code>rift-mcp</code>.
            Audit trail: filter <code>bus tail</code> to <code>Category::Mcp</code>.
          </div>
        {/if}

        {#if saveBanner && saveBanner.section === 'mcp'}
          <span class="banner-inline" class:fail={!saveBanner.ok}>
            {saveBanner.msg}
          </span>
        {/if}
      </section>
    {/if}

    <!-- NOTIFICATIONS -->
    <section class="section">
      <div class="section-label">Notifications</div>
      <div class="hint">
        notification tabs are managed from a dedicated popout — opens via the
        <code>⋯</code> button on the right edge of the tab strip, or here.
      </div>
    </section>

  </div>

  <div class="settings-footer">
    <button type="button" class="btn primary" onclick={done}>done</button>
  </div>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-family: 'JetBrains Mono', monospace;
    color: var(--amber-warm);
  }

  .settings-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 18px 8px;
  }
  .settings-body::-webkit-scrollbar { width: 5px; }
  .settings-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .section {
    padding: 10px 0;
    border-bottom: 1px solid var(--border-subtle);
  }
  .section:last-of-type { border-bottom: none; }
  .section-label {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    font-weight: 700;
    margin-bottom: 8px;
  }

  .hint {
    color: var(--amber-faint);
    font-size: 10px;
    font-style: italic;
    line-height: 1.5;
    margin-bottom: 8px;
  }
  .hint code {
    color: var(--amber-warm);
    font-style: normal;
  }

  .kv {
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: 12px;
    align-items: baseline;
    padding: 2px 0;
    font-size: 11px;
  }
  .kv .k {
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 9px;
  }
  .kv .v {
    color: var(--amber-warm);
    font-weight: 600;
  }
  .kv .v.path {
    word-break: break-all;
    font-size: 10px;
    color: var(--amber-dim);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
    flex-wrap: wrap;
  }

  .kv-toggle {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--amber-warm);
    cursor: pointer;
  }
  .kv-toggle input { cursor: pointer; }

  .mono-wrap {
    font-family: 'JetBrains Mono', monospace;
    word-break: break-all;
    font-size: 10px;
  }

  .btn {
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 700;
    padding: 4px 10px;
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s, background 0.12s;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .btn.primary {
    background: var(--amber-bright);
    border-color: var(--amber-bright);
    color: var(--bg-base);
  }
  .btn.primary:hover:not(:disabled) {
    box-shadow: var(--glow-amber-faint);
  }

  .banner {
    margin-top: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border-subtle);
    font-size: 10px;
    line-height: 1.5;
  }
  .banner-ok {
    border-color: var(--term-green, #33CC33);
    color: var(--term-green, #33CC33);
  }
  .banner-info {
    border-color: var(--amber-bright);
    color: var(--amber-warm);
  }
  .banner-fail {
    border-color: var(--term-red);
    color: var(--term-red);
  }
  .banner-title {
    color: var(--amber-bright);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 4px;
  }
  .banner-body {
    color: var(--amber-dim);
    margin-bottom: 6px;
    white-space: pre-wrap;
  }
  .banner-actions {
    display: flex;
    gap: 6px;
  }
  .banner-inline {
    color: var(--term-green, #33CC33);
    font-size: 9px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .banner-inline.fail { color: var(--term-red); }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 8px;
  }
  .field-label {
    color: var(--amber-dim);
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .field-input {
    background: var(--bg-base);
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 11px;
    padding: 6px 8px;
    resize: vertical;
  }
  .field-input:focus {
    outline: none;
    border-color: var(--amber-bright);
    box-shadow: var(--glow-amber-faint);
  }
  .field-narrow {
    width: 80px;
    resize: none;
  }

  .radio-row {
    display: flex;
    gap: 14px;
    margin-bottom: 4px;
  }
  .radio {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--amber-warm);
    font-weight: 600;
  }
  .radio input[type="radio"] {
    accent-color: var(--amber-bright);
  }

  .settings-footer {
    flex-shrink: 0;
    border-top: 1px solid var(--border-subtle);
    padding: 10px 18px;
    display: flex;
    justify-content: flex-end;
  }
</style>
