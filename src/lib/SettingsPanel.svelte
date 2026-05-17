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
  import type { RiftConfig, McpConfig, ShellPref, SeverityLevel } from './riftConfig';

  interface Props {
    popoutId: number;
  }

  let { popoutId }: Props = $props();

  type SettingsTab = 'general' | 'terminal' | 'index' | 'tree' | 'mcp';
  let activeTab = $state<SettingsTab>('general');

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

  // Config data types imported from ./riftConfig.ts (canonical source).
  // UI-only types (ShellKind, McpStatus) stay local.
  type ShellKind = ShellPref['kind'];
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
  let indexLabelVisibility = $state<'always' | 'hover_only' | 'on_zoom2x'>('always');
  let indexDensity = $state<'compact' | 'standard' | 'spacious'>('standard');

  let savingFs = $state(false);
  let savingIndex = $state(false);
  let savingMcp = $state(false);
  let savingTerminal = $state(false);
  let savingTree = $state(false);
  let saveBanner = $state<{
    section: 'fs' | 'index' | 'mcp' | 'terminal' | 'notif' | 'tree';
    ok: boolean;
    msg: string;
  } | null>(null);

  // Terminal — D-018 groundwork (audit close 2026-04-29).
  let termShellKind = $state<ShellKind>('auto');
  let termCustomPath = $state('');
  let termFontSize = $state(13);
  let termLineHeight = $state(1.55);
  let termScrollback = $state(1000);
  let termLanesEnabled = $state(true);

  // Tree — D-020 heatmap groundwork.
  let treeHeatmapEnabled = $state(false);
  let treeHeatmapWindow = $state(15);

  const HEATMAP_WINDOW_OPTIONS = [
    { value: 5, label: '5 min' },
    { value: 15, label: '15 min' },
    { value: 60, label: '1 hour' },
  ];

  const treeDirty = $derived(
    config !== null
    && (
      treeHeatmapEnabled !== (config.tree?.heatmap_enabled ?? false)
      || treeHeatmapWindow !== (config.tree?.heatmap_window_minutes ?? 15)
    )
  );

  async function saveTreeConfig() {
    if (!config) return;
    savingTree = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        tree: {
          heatmap_enabled: treeHeatmapEnabled,
          heatmap_window_minutes: treeHeatmapWindow,
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = { section: 'tree', ok: true, msg: 'tree settings saved' };
    } catch (err) {
      saveBanner = { section: 'tree', ok: false, msg: String(err) };
    } finally {
      savingTree = false;
    }
  }

  // Notification filters
  const SEVERITY_OPTIONS: SeverityLevel[] = ['debug', 'info', 'warn', 'error'];
  const NOTIF_TAB_IDS = ['errors', 'hooks', 'commands', 'aegis', 'index', 'bustail', 'todo', 'git', 'agents'];
  let notifDefaultThreshold = $state<SeverityLevel>('info');
  let notifPerTab = $state<Record<string, SeverityLevel>>({});
  let savingNotif = $state(false);

  const notifDirty = $derived(
    config !== null
    && (
      notifDefaultThreshold !== (config.notif_filters?.default_threshold ?? 'info')
      || JSON.stringify(notifPerTab) !== JSON.stringify(config.notif_filters?.per_tab ?? {})
    )
  );

  async function saveNotifFilters() {
    if (!config) return;
    savingNotif = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        notif_filters: {
          default_threshold: notifDefaultThreshold,
          per_tab: { ...notifPerTab },
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = { section: 'notif', ok: true, msg: 'notification filters saved' };
    } catch (err) {
      saveBanner = { section: 'notif', ok: false, msg: String(err) };
    } finally {
      savingNotif = false;
    }
  }

  function setPerTabThreshold(tabId: string, value: string) {
    if (value === 'default') {
      const next = { ...notifPerTab };
      delete next[tabId];
      notifPerTab = next;
    } else {
      notifPerTab = { ...notifPerTab, [tabId]: value as SeverityLevel };
    }
  }

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

  /** Phase 8.7q.1 — broadcast a `rift:config-changed` window event so live
   *  surfaces (IndexGraph density + label-visibility, etc.) re-read their
   *  config without needing a remount. Called after every successful save. */
  function broadcastConfigChanged(): void {
    window.dispatchEvent(new CustomEvent('rift:config-changed'));
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
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
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
    config !== null
    && (
      indexSyncMode !== config.index.sync_mode
      || indexLabelVisibility !== (config.index.label_visibility === 'unknown' ? 'always' : config.index.label_visibility)
      || indexDensity !== (config.index.density === 'unknown' ? 'standard' : config.index.density)
    )
  );

  function buildTerminalShellPref(): ShellPref {
    if (termShellKind === 'custom') {
      return { kind: 'custom', path: termCustomPath.trim() };
    }
    return { kind: termShellKind } as ShellPref;
  }

  function shellPrefEquals(a: ShellPref, b: ShellPref): boolean {
    if (a.kind !== b.kind) return false;
    if (a.kind === 'custom' && b.kind === 'custom') return a.path === b.path;
    return true;
  }

  const terminalDirty = $derived(
    config !== null
    && (
      !shellPrefEquals(buildTerminalShellPref(), config.terminal.shell)
      || termFontSize !== config.terminal.font_size
      || Math.abs(termLineHeight - config.terminal.line_height) > 1e-4
      || termScrollback !== config.terminal.scrollback
      || termLanesEnabled !== config.terminal.lanes_enabled
    )
  );

  function snapshotIntoEditState(c: RiftConfig) {
    fsIgnoreText = c.fs.ignore_globs.join('\n');
    fsMaxDepth = c.fs.max_depth;
    indexSyncMode = c.index.sync_mode === 'manual' ? 'manual' : 'live';
    indexLabelVisibility =
      c.index.label_visibility === 'hover_only'
        ? 'hover_only'
        : c.index.label_visibility === 'on_zoom2x'
          ? 'on_zoom2x'
          : 'always';
    indexDensity =
      c.index.density === 'compact'
        ? 'compact'
        : c.index.density === 'spacious'
          ? 'spacious'
          : 'standard';
    // Terminal snapshot — defaults for old configs are filled by serde-side
    // #[serde(default)], so c.terminal is always present at runtime.
    termShellKind = c.terminal.shell.kind;
    termCustomPath = c.terminal.shell.kind === 'custom' ? c.terminal.shell.path : '';
    termFontSize = c.terminal.font_size;
    termLineHeight = c.terminal.line_height;
    termScrollback = c.terminal.scrollback;
    termLanesEnabled = c.terminal.lanes_enabled;
    // Tree snapshot — defaults for old configs are filled by serde-side
    // #[serde(default)], so c.tree is always present at runtime.
    const tree = c.tree ?? { heatmap_enabled: false, heatmap_window_minutes: 15 };
    treeHeatmapEnabled = tree.heatmap_enabled;
    treeHeatmapWindow = tree.heatmap_window_minutes;
    // Notification filters snapshot.
    const nf = c.notif_filters ?? { default_threshold: 'info', per_tab: {} };
    notifDefaultThreshold = (['debug', 'info', 'warn', 'error'].includes(nf.default_threshold)
      ? nf.default_threshold : 'info') as SeverityLevel;
    notifPerTab = { ...(nf.per_tab ?? {}) };
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
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = { section: 'fs', ok: true, msg: 'filesystem settings saved' };
    } catch (err) {
      saveBanner = { section: 'fs', ok: false, msg: String(err) };
    } finally {
      savingFs = false;
    }
  }

  async function saveTerminalConfig() {
    if (!config) return;
    savingTerminal = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        terminal: {
          shell: buildTerminalShellPref(),
          font_size: Math.max(8, Math.min(48, Math.floor(termFontSize || 13))),
          line_height: Math.max(1.0, Math.min(2.5, termLineHeight || 1.55)),
          scrollback: Math.max(100, Math.min(100000, Math.floor(termScrollback || 1000))),
          lanes_enabled: termLanesEnabled,
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = {
        section: 'terminal',
        ok: true,
        msg: 'terminal saved · shell change applies to new sessions',
      };
    } catch (err) {
      saveBanner = { section: 'terminal', ok: false, msg: String(err) };
    } finally {
      savingTerminal = false;
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
          label_visibility: indexLabelVisibility,
          density: indexDensity,
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
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
  <nav class="tab-strip">
    {#each [
      { id: 'general',  label: 'GENERAL' },
      { id: 'terminal', label: 'TERMINAL' },
      { id: 'index',    label: 'INDEX' },
      { id: 'tree',     label: 'TREE' },
      { id: 'mcp',      label: 'MCP' },
    ] as tab (tab.id)}
      <button
        class="tab-btn"
        class:active={activeTab === tab.id}
        onclick={() => (activeTab = tab.id as SettingsTab)}
      >{tab.label}</button>
    {/each}
  </nav>

  <div class="settings-body">

    {#if activeTab === 'general'}
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

    <!-- NOTIFICATIONS -->
    <section class="section">
      <div class="section-label">Notification Filters</div>
      <div class="hint">
        set minimum severity per notification tab. events below the threshold
        are hidden from the tab (still captured by session logger).
      </div>

      <label class="field">
        <span class="field-label">default threshold</span>
        <select
          class="select"
          value={notifDefaultThreshold}
          onchange={(e) => { notifDefaultThreshold = (e.target as HTMLSelectElement).value as SeverityLevel; }}
        >
          {#each SEVERITY_OPTIONS as s (s)}
            <option value={s}>{s}</option>
          {/each}
        </select>
      </label>

      <div class="notif-per-tab">
        <div class="field-label" style="margin-bottom: 6px;">per-tab overrides</div>
        {#each NOTIF_TAB_IDS as tabId (tabId)}
          <label class="field notif-tab-row">
            <span class="field-label notif-tab-name">{tabId}</span>
            <select
              class="select"
              value={notifPerTab[tabId] ?? 'default'}
              onchange={(e) => setPerTabThreshold(tabId, (e.target as HTMLSelectElement).value)}
            >
              <option value="default">default ({notifDefaultThreshold})</option>
              {#each SEVERITY_OPTIONS as s (s)}
                <option value={s}>{s}</option>
              {/each}
            </select>
          </label>
        {/each}
      </div>

      <div class="save-row">
        <button
          class="save-btn"
          disabled={!notifDirty || savingNotif}
          onclick={saveNotifFilters}
        >{savingNotif ? 'saving...' : 'save filters'}</button>
      </div>
    </section>
    {/if}

    {#if activeTab === 'terminal'}
    <!-- TERMINAL -->
    {#if config}
      <section class="section">
        <div class="section-label">Terminal</div>
        <div class="hint">
          shell, font, scrollback, and §10.1 lane prefixes for Rift-emitted
          lines. shell change applies to NEW sessions (close + reopen the
          tab). live PTY-stream lane classification is tracked under D-018
          and is not yet wired.
        </div>

        <label class="field">
          <span class="field-label">shell</span>
          <div class="radio-row radio-wrap">
            {#each ['auto', 'pwsh', 'powershell', 'cmd', 'bash', 'zsh', 'sh', 'custom'] as kind (kind)}
              <label class="radio">
                <input
                  type="radio"
                  name="term-shell"
                  value={kind}
                  checked={termShellKind === kind}
                  onchange={() => (termShellKind = kind as ShellKind)}
                />
                <span>{kind}</span>
              </label>
            {/each}
          </div>
        </label>

        {#if termShellKind === 'custom'}
          <label class="field">
            <span class="field-label">custom executable path</span>
            <input
              type="text"
              class="field-input"
              bind:value={termCustomPath}
              placeholder="C:\Path\To\shell.exe or /usr/local/bin/fish"
              spellcheck="false"
            />
          </label>
        {/if}

        <label class="field">
          <span class="field-label">font size · 8–48 px</span>
          <input
            type="number"
            class="field-input field-narrow"
            bind:value={termFontSize}
            min="8"
            max="48"
          />
        </label>

        <label class="field">
          <span class="field-label">line height · 1.00–2.50</span>
          <input
            type="number"
            class="field-input field-narrow"
            bind:value={termLineHeight}
            min="1.0"
            max="2.5"
            step="0.05"
          />
        </label>

        <label class="field">
          <span class="field-label">scrollback · 100–100000 lines</span>
          <input
            type="number"
            class="field-input field-narrow"
            bind:value={termScrollback}
            min="100"
            max="100000"
            step="100"
          />
        </label>

        <div class="row">
          <label class="kv-toggle">
            <input
              type="checkbox"
              bind:checked={termLanesEnabled}
              disabled={savingTerminal}
            />
            <span>tag-prefix Rift-emitted lines (§10.1)</span>
          </label>
        </div>

        <div class="row">
          <button
            type="button"
            class="btn primary"
            disabled={!terminalDirty || savingTerminal}
            onclick={saveTerminalConfig}
          >
            {savingTerminal ? 'saving…' : 'save terminal'}
          </button>
          {#if saveBanner && saveBanner.section === 'terminal'}
            <span class="banner-inline" class:fail={!saveBanner.ok}>
              {saveBanner.msg}
            </span>
          {/if}
        </div>
      </section>
    {:else}
      <div class="hint">loading config…</div>
    {/if}
    {/if}

    {#if activeTab === 'index'}
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

        <div class="hint" style="margin-top: 12px;">
          graph cluster density — scales the radial spread between vault clusters.
        </div>
        <div class="radio-row">
          {#each ['compact', 'standard', 'spacious'] as d (d)}
            <label class="radio">
              <input
                type="radio"
                name="index-density"
                value={d}
                checked={indexDensity === d}
                onchange={() => (indexDensity = d as 'compact' | 'standard' | 'spacious')}
              />
              <span>{d}</span>
            </label>
          {/each}
        </div>

        <div class="hint" style="margin-top: 8px;">
          label visibility — when to show the id + tagline under each node.
        </div>
        <div class="radio-row">
          {#each [
            { v: 'always', label: 'always' },
            { v: 'hover_only', label: 'hover only' },
            { v: 'on_zoom2x', label: 'on 2× zoom' },
          ] as opt (opt.v)}
            <label class="radio">
              <input
                type="radio"
                name="index-label-vis"
                value={opt.v}
                checked={indexLabelVisibility === opt.v}
                onchange={() => (indexLabelVisibility = opt.v as 'always' | 'hover_only' | 'on_zoom2x')}
              />
              <span>{opt.label}</span>
            </label>
          {/each}
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
    {:else}
      <div class="hint">loading config…</div>
    {/if}
    {/if}

    {#if activeTab === 'tree'}
    <!-- TREE — D-020 heatmap groundwork -->
    {#if config}
      <section class="section">
        <div class="section-label">Tree</div>
        <div class="hint">
          activity heatmap overlays filesystem tree nodes with frequency-based
          coloring. touch events within the sliding window are aggregated to
          determine intensity.
        </div>

        <div class="row">
          <label class="kv-toggle">
            <input
              type="checkbox"
              bind:checked={treeHeatmapEnabled}
              disabled={savingTree}
            />
            <span>activity heatmap</span>
          </label>
        </div>
        <div class="hint" style="margin-top: 2px; margin-bottom: 10px;">
          color-code files by touch frequency over time
        </div>

        {#if treeHeatmapEnabled}
          <label class="field">
            <span class="field-label">aggregation window</span>
            <select
              class="select"
              value={treeHeatmapWindow}
              onchange={(e) => { treeHeatmapWindow = Number((e.target as HTMLSelectElement).value); }}
              disabled={savingTree}
            >
              {#each HEATMAP_WINDOW_OPTIONS as opt (opt.value)}
                <option value={opt.value}>{opt.label}</option>
              {/each}
            </select>
          </label>
        {/if}

        <div class="row">
          <button
            type="button"
            class="btn primary"
            disabled={!treeDirty || savingTree}
            onclick={saveTreeConfig}
          >
            {savingTree ? 'saving…' : 'save tree'}
          </button>
          {#if saveBanner && saveBanner.section === 'tree'}
            <span class="banner-inline" class:fail={!saveBanner.ok}>
              {saveBanner.msg}
            </span>
          {/if}
        </div>
      </section>
    {:else}
      <div class="hint">loading config…</div>
    {/if}
    {/if}

    {#if activeTab === 'mcp'}
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
    {:else}
      <div class="hint">loading config…</div>
    {/if}
    {/if}

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

  /* ─── Tab strip ──────────────────────────────────────────────────────── */
  .tab-strip {
    flex-shrink: 0;
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border-subtle);
    padding: 0 16px;
    background: var(--bg-panel, #0c0c0a);
  }
  .tab-btn {
    background: none;
    border: none;
    border-bottom: 3px solid transparent;
    color: var(--amber-faint);
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    padding: 9px 16px 7px;
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s, background 0.12s;
    /* ensure the bottom border doesn't shift layout */
    margin-bottom: -1px;
  }
  .tab-btn:hover {
    color: var(--amber-warm);
    background: var(--bg-hover, #1a1a14);
  }
  .tab-btn.active {
    color: var(--amber-bright);
    border-bottom-color: var(--amber-bright);
    background: var(--bg-elevated, #14140F);
  }

  /* ─── Scrollable body ────────────────────────────────────────────────── */
  .settings-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 18px 12px;
  }
  .settings-body::-webkit-scrollbar { width: 4px; }
  .settings-body::-webkit-scrollbar-thumb {
    background: var(--amber-faint);
    border-radius: 2px;
  }
  .settings-body::-webkit-scrollbar-track { background: transparent; }

  /* ─── Sections ───────────────────────────────────────────────────────── */
  .section {
    padding: 14px 0 16px;
    border-bottom: 1px solid var(--border-subtle);
  }
  .section:last-of-type { border-bottom: none; }
  .section-label {
    color: var(--amber-faint);
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    font-weight: 700;
    margin-bottom: 10px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--border-subtle);
  }

  /* ─── Hints / prose ──────────────────────────────────────────────────── */
  .hint {
    color: var(--amber-faint);
    font-size: 10px;
    font-style: italic;
    line-height: 1.55;
    margin-bottom: 10px;
  }
  .hint code {
    color: var(--amber-warm);
    font-style: normal;
  }

  /* ─── Key-value display rows ─────────────────────────────────────────── */
  .kv {
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: 12px;
    align-items: baseline;
    padding: 3px 0;
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

  /* ─── Flex utility row (buttons / inline banners) ────────────────────── */
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
    flex-wrap: wrap;
  }

  /* ─── Checkbox toggle row ────────────────────────────────────────────── */
  .kv-toggle {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    color: var(--amber-warm);
    cursor: pointer;
    padding: 4px 0;
  }
  /* Custom checkbox — replaces browser default with amber square */
  .kv-toggle input[type="checkbox"] {
    appearance: none;
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    border: 1px solid var(--amber-faint);
    background: var(--bg-base, #080806);
    cursor: pointer;
    flex-shrink: 0;
    position: relative;
    transition: border-color 0.12s, background 0.12s;
  }
  .kv-toggle input[type="checkbox"]:checked {
    background: var(--amber-bright);
    border-color: var(--amber-bright);
  }
  .kv-toggle input[type="checkbox"]:checked::after {
    content: '';
    position: absolute;
    inset: 0;
    background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'%3E%3Cpath d='M1.5 5l2.5 2.5 4.5-4.5' stroke='%23080806' stroke-width='1.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E") center/8px no-repeat;
  }
  .kv-toggle input[type="checkbox"]:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px rgba(255, 200, 64, 0.4);
  }
  .kv-toggle input[type="checkbox"]:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ─── Mono value display ─────────────────────────────────────────────── */
  .mono-wrap {
    font-family: 'JetBrains Mono', monospace;
    word-break: break-all;
    font-size: 10px;
  }

  /* ─── Buttons ────────────────────────────────────────────────────────── */
  .btn {
    background: transparent;
    border: 1px solid var(--border-active, #4a3818);
    color: var(--amber-dim);
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 700;
    padding: 0 12px;
    height: 32px;
    line-height: 32px;
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s, background 0.12s, box-shadow 0.12s, transform 0.1s;
    white-space: nowrap;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--amber-primary, #FFA826);
    color: var(--amber-warm);
    transform: translateY(-1px);
  }
  .btn:active:not(:disabled) {
    transform: translateY(0);
  }
  .btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .btn.primary {
    background: var(--amber-bright, #FFC840);
    border-color: var(--amber-bright, #FFC840);
    color: var(--bg-base, #080806);
    font-weight: 800;
  }
  .btn.primary:hover:not(:disabled) {
    box-shadow: var(--glow-amber-strong, 0 0 14px rgba(255, 200, 64, 0.85));
    transform: translateY(-1px);
  }
  .btn.primary:active:not(:disabled) {
    transform: translateY(0);
    box-shadow: var(--glow-amber, 0 0 8px rgba(255, 168, 38, 0.55));
  }

  /* ─── Banners (block — update / error notices) ───────────────────────── */
  .banner {
    margin-top: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border-subtle);
    font-size: 10px;
    line-height: 1.5;
    background: var(--bg-panel, #0c0c0a);
  }
  .banner-ok {
    border-color: var(--term-green, #4FE855);
    color: var(--term-green, #4FE855);
    background: rgba(51, 204, 51, 0.05);
  }
  .banner-info {
    border-color: var(--amber-bright, #FFC840);
    color: var(--amber-warm);
    background: rgba(255, 200, 64, 0.05);
  }
  .banner-fail {
    border-color: var(--term-red, #FF4848);
    color: var(--term-red, #FF4848);
    background: rgba(204, 51, 51, 0.05);
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
    margin-bottom: 8px;
    white-space: pre-wrap;
  }
  .banner-actions {
    display: flex;
    gap: 6px;
    margin-top: 6px;
  }

  /* Inline save feedback — sits in the .row next to the save button */
  .banner-inline {
    display: inline-flex;
    align-items: center;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--term-green, #4FE855);
    background: rgba(51, 204, 51, 0.07);
    color: var(--term-green, #4FE855);
    font-size: 9px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .banner-inline.fail {
    border-color: var(--term-red, #FF4848);
    background: rgba(204, 51, 51, 0.07);
    color: var(--term-red, #FF4848);
  }

  /* ─── Form fields ────────────────────────────────────────────────────── */
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 12px;
  }
  .field-label {
    color: var(--amber-dim);
    font-size: 9px;
    letter-spacing: 0.10em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .field-input {
    background: var(--bg-surface, #0F0F0D);
    border: 1px solid var(--border-active, #4a3818);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: 11px;
    padding: 0 10px;
    height: 34px;
    line-height: 34px;
    resize: none;
    box-sizing: border-box;
    transition: border-color 0.12s, box-shadow 0.12s;
    caret-color: var(--amber-bright);
  }
  /* textarea overrides — restore resize and natural height */
  .field-input[rows] {
    height: auto;
    line-height: 1.6;
    padding: 8px 10px;
    resize: vertical;
  }
  .field-input:focus {
    outline: none;
    border-color: var(--amber-primary, #FFA826);
    box-shadow: 0 0 0 1px var(--amber-dim, #D8A028), var(--glow-amber, 0 0 8px rgba(255, 168, 38, 0.55));
  }
  .field-input::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }
  .field-input:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .field-narrow {
    width: 88px;
  }

  /* ─── Radio groups ───────────────────────────────────────────────────── */
  .radio-row {
    display: flex;
    gap: 16px;
    margin-bottom: 6px;
    flex-wrap: wrap;
  }
  /* Terminal section has 8 radios — wrap onto multiple rows on narrow popouts. */
  .radio-row.radio-wrap {
    row-gap: 8px;
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
    padding: 3px 0;
    transition: color 0.1s;
  }
  .radio:hover { color: var(--amber-bright); }
  .radio input[type="radio"] {
    appearance: none;
    -webkit-appearance: none;
    width: 12px;
    height: 12px;
    border: 1px solid var(--amber-faint);
    border-radius: 50%;
    background: var(--bg-base, #080806);
    cursor: pointer;
    flex-shrink: 0;
    position: relative;
    transition: border-color 0.12s;
  }
  .radio input[type="radio"]:checked {
    border-color: var(--amber-bright);
  }
  .radio input[type="radio"]:checked::after {
    content: '';
    position: absolute;
    inset: 2px;
    border-radius: 50%;
    background: var(--amber-bright);
  }
  .radio input[type="radio"]:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px rgba(255, 200, 64, 0.4);
  }

  /* ─── Footer ─────────────────────────────────────────────────────────── */
  .settings-footer {
    flex-shrink: 0;
    border-top: 1px solid var(--border-subtle);
    padding: 10px 18px;
    display: flex;
    justify-content: flex-end;
    background: var(--bg-panel, #0c0c0a);
  }
</style>
