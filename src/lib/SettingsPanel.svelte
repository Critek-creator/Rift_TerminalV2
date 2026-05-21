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
  import type { RiftConfig, McpConfig, ShellPref, SeverityLevel, StatusLineConfig, AlertRule, AlertAction } from './riftConfig';
  import { newAlertRuleId } from './alertRules';

  interface Props {
    popoutId: number;
  }

  let { popoutId }: Props = $props();

  type SettingsTab = 'general' | 'terminal' | 'index' | 'tree' | 'mcp' | 'statusline' | 'alerts';
  let activeTab = $state<SettingsTab>('general');

  // ---------------------------------------------------------------------
  // About
  // ---------------------------------------------------------------------

  let appVersion = $state('—');
  let appName = $state('Rift');
  let appIdentifier = $state('com.abyssal.rift');

  // Crash log count — read from localStorage on init.
  let jsCrashCount = $state((() => { try { return (JSON.parse(localStorage.getItem('rift:crash_log') || '[]') as unknown[]).length; } catch { return 0; } })());

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
    section: 'fs' | 'index' | 'mcp' | 'terminal' | 'notif' | 'tree' | 'statusline' | 'alerts';
    ok: boolean;
    msg: string;
  } | null>(null);

  // Terminal — D-018 groundwork (audit close 2026-04-29).
  let termShellKind = $state<ShellKind>('auto');
  let termCustomPath = $state('');
  let termFontSize = $state(13);
  let termFontFamily = $state("'JetBrains Mono', monospace");
  let termLineHeight = $state(1.55);
  let termScrollback = $state(1000);
  let termLanesEnabled = $state(true);

  // Font presets — aesthetic templates
  const FONT_PRESETS: { label: string; value: string }[] = [
    { label: 'Abyssal Classic', value: "'JetBrains Mono', monospace" },
    { label: 'Operator', value: "'Fira Code', monospace" },
    { label: 'Cascade', value: "'Cascadia Code', monospace" },
    { label: 'Plexed', value: "'IBM Plex Mono', monospace" },
    { label: 'Source', value: "'Source Code Pro', monospace" },
  ];
  let fontPresetMode = $state<'preset' | 'custom'>('preset');
  let customFontFamily = $state('');

  // StatusLine — segment visibility toggles.
  let slShowDir = $state(true);
  let slShowGit = $state(true);
  let slShowRepo = $state(true);
  let slShowSession = $state(true);
  let slShowSkill = $state(true);
  let slShowEffort = $state(true);
  let slShowModel = $state(true);
  let slShowCtx = $state(true);
  let slShowSessionUse = $state(true);
  let slShowWeek = $state(true);
  let savingStatusline = $state(false);

  // Alerts — smart tab alerting rules.
  let alertRules = $state<AlertRule[]>([]);
  let savingAlerts = $state(false);

  const alertsDirty = $derived(
    config !== null
    && JSON.stringify(alertRules) !== JSON.stringify(config.alerts?.rules ?? [])
  );

  function addAlertRule() {
    alertRules = [...alertRules, {
      id: newAlertRuleId(),
      tab_id: 'errors',
      severity: 'error' as SeverityLevel,
      threshold: 3,
      window_secs: 10,
      action: 'flash' as AlertAction,
      enabled: true,
    }];
  }

  function removeAlertRule(id: string) {
    alertRules = alertRules.filter((r) => r.id !== id);
  }

  function updateAlertRule(id: string, patch: Partial<AlertRule>) {
    alertRules = alertRules.map((r) => r.id === id ? { ...r, ...patch } : r);
  }

  async function saveAlertRules() {
    if (!config) return;
    savingAlerts = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        alerts: { rules: alertRules },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = { section: 'alerts', ok: true, msg: 'alert rules saved' };
    } catch (err) {
      saveBanner = { section: 'alerts', ok: false, msg: String(err) };
    } finally {
      savingAlerts = false;
    }
  }

  const TAB_ID_OPTIONS = [
    'errors', 'hooks', 'commands', 'aegis', 'index', 'agents',
    'filesystem', 'mcp', 'sentinel', 'bustail', 'todo', 'git', 'sessions',
  ];

  const ALERT_ACTION_OPTIONS: { value: AlertAction; label: string }[] = [
    { value: 'flash', label: 'flash badge' },
    { value: 'promote', label: 'auto-promote' },
    { value: 'tone', label: 'play tone' },
  ];

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
      || termFontFamily !== (config.terminal.font_family ?? "'JetBrains Mono', monospace")
      || Math.abs(termLineHeight - config.terminal.line_height) > 1e-4
      || termScrollback !== config.terminal.scrollback
      || termLanesEnabled !== config.terminal.lanes_enabled
    )
  );

  const statuslineDirty = $derived(
    config !== null
    && (
      slShowDir !== (config.statusline?.show_dir ?? true)
      || slShowGit !== (config.statusline?.show_git ?? true)
      || slShowRepo !== (config.statusline?.show_repo ?? true)
      || slShowSession !== (config.statusline?.show_session ?? true)
      || slShowSkill !== (config.statusline?.show_skill ?? true)
      || slShowEffort !== (config.statusline?.show_effort ?? true)
      || slShowModel !== (config.statusline?.show_model ?? true)
      || slShowCtx !== (config.statusline?.show_ctx ?? true)
      || slShowSessionUse !== (config.statusline?.show_session_use ?? true)
      || slShowWeek !== (config.statusline?.show_week ?? true)
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
    termFontFamily = c.terminal.font_family ?? "'JetBrains Mono', monospace";
    termLineHeight = c.terminal.line_height;
    termScrollback = c.terminal.scrollback;
    termLanesEnabled = c.terminal.lanes_enabled;
    // Font preset mode detection
    const matchedPreset = FONT_PRESETS.find(p => p.value === termFontFamily);
    if (matchedPreset) {
      fontPresetMode = 'preset';
      customFontFamily = '';
    } else {
      fontPresetMode = 'custom';
      customFontFamily = termFontFamily;
    }
    // StatusLine snapshot
    const sl = c.statusline ?? {} as StatusLineConfig;
    slShowDir = sl.show_dir ?? true;
    slShowGit = sl.show_git ?? true;
    slShowRepo = sl.show_repo ?? true;
    slShowSession = sl.show_session ?? true;
    slShowSkill = sl.show_skill ?? true;
    slShowEffort = sl.show_effort ?? true;
    slShowModel = sl.show_model ?? true;
    slShowCtx = sl.show_ctx ?? true;
    slShowSessionUse = sl.show_session_use ?? true;
    slShowWeek = sl.show_week ?? true;
    // Tree snapshot — defaults for old configs are filled by serde-side
    // #[serde(default)], so c.tree is always present at runtime.
    const tree = c.tree ?? { heatmap_enabled: false, heatmap_window_minutes: 15 };
    treeHeatmapEnabled = tree.heatmap_enabled;
    treeHeatmapWindow = tree.heatmap_window_minutes;
    // Alert rules snapshot.
    alertRules = c.alerts?.rules ?? [];
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
          font_family: termFontFamily || "'JetBrains Mono', monospace",
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

  async function saveStatuslineConfig() {
    if (!config) return;
    savingStatusline = true;
    saveBanner = null;
    try {
      const next: RiftConfig = {
        ...config,
        statusline: {
          show_dir: slShowDir,
          show_git: slShowGit,
          show_repo: slShowRepo,
          show_session: slShowSession,
          show_skill: slShowSkill,
          show_effort: slShowEffort,
          show_model: slShowModel,
          show_ctx: slShowCtx,
          show_session_use: slShowSessionUse,
          show_week: slShowWeek,
          color_overrides: config.statusline?.color_overrides ?? {},
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = { section: 'statusline', ok: true, msg: 'status line saved' };
    } catch (err) {
      saveBanner = { section: 'statusline', ok: false, msg: String(err) };
    } finally {
      savingStatusline = false;
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
      { id: 'general',    label: 'GENERAL' },
      { id: 'terminal',   label: 'TERMINAL' },
      { id: 'statusline', label: 'STATUS LINE' },
      { id: 'index',      label: 'INDEX' },
      { id: 'tree',       label: 'TREE' },
      { id: 'mcp',        label: 'MCP' },
      { id: 'alerts',     label: 'ALERTS' },
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
        <div class="v">{appVersion} <span style="background: rgba(255,168,38,0.18); border: 1px solid var(--amber-faint, #A87830); border-radius: 3px; padding: 0 5px; font-size: 8px; font-weight: 700; letter-spacing: 0.1em; color: var(--amber-primary, #FFA826); margin-left: 6px;">BETA</span></div>
      </div>
      <div class="kv">
        <div class="k">identifier</div>
        <div class="v">{appIdentifier}</div>
      </div>
      <div class="row" style="margin-top: 8px; gap: 6px;">
        <button
          type="button"
          class="btn"
          onclick={() => window.dispatchEvent(new Event('rift:show-welcome'))}
        >WELCOME GUIDE</button>
        <button
          type="button"
          class="btn"
          onclick={() => window.open('https://patreon.com/abyssalarts', '_blank')}
        >SUPPORT ON PATREON</button>
      </div>
    </section>

    <!-- CRASH LOGS -->
    <section class="section">
      <div class="section-label">Crash Logs</div>
      {#if jsCrashCount === 0}
        <div class="hint">No JS errors recorded.</div>
      {:else}
        <div class="hint">{jsCrashCount} error{jsCrashCount === 1 ? '' : 's'} recorded.</div>
        <div class="row" style="margin-top: 6px; gap: 6px;">
          <button
            type="button"
            class="btn"
            onclick={() => { navigator.clipboard.writeText(localStorage.getItem('rift:crash_log') || '[]'); }}
          >COPY TO CLIPBOARD</button>
          <button
            type="button"
            class="btn"
            onclick={() => { localStorage.removeItem('rift:crash_log'); jsCrashCount = 0; }}
          >CLEAR</button>
        </div>
      {/if}
      <div class="hint" style="margin-top: 6px;">
        Rust crash dumps are saved to your data directory under crashes/.
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

        <div class="section-label" style="margin-top: 12px;">Font</div>
        <div class="radio-row">
          <label class="radio">
            <input type="radio" name="font-mode" value="preset"
              checked={fontPresetMode === 'preset'}
              onchange={() => (fontPresetMode = 'preset')}
            />
            <span>preset</span>
          </label>
          <label class="radio">
            <input type="radio" name="font-mode" value="custom"
              checked={fontPresetMode === 'custom'}
              onchange={() => { fontPresetMode = 'custom'; if (!customFontFamily) customFontFamily = termFontFamily; }}
            />
            <span>custom</span>
          </label>
        </div>

        {#if fontPresetMode === 'preset'}
          <div class="radio-col">
            {#each FONT_PRESETS as preset}
              <label class="radio">
                <input type="radio" name="font-preset" value={preset.value}
                  checked={termFontFamily === preset.value}
                  onchange={() => (termFontFamily = preset.value)}
                />
                <span style="font-family: {preset.value}">{preset.label}</span>
              </label>
            {/each}
          </div>
        {:else}
          <label class="field">
            <span class="field-label">CSS font-family</span>
            <input
              type="text"
              class="field-input"
              bind:value={customFontFamily}
              oninput={() => (termFontFamily = customFontFamily)}
              placeholder="'My Font', monospace"
            />
          </label>
        {/if}

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

    {#if activeTab === 'statusline'}
    <!-- STATUS LINE -->
    {#if config}
      <section class="section">
        <div class="section-label">Segment Visibility</div>
        <div class="hint">toggle individual status line segments on or off.</div>

        <div class="toggle-grid">
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowDir} /><span>DIR</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowModel} /><span>MODEL</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowCtx} /><span>CTX%</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowSession} /><span>SESSION</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowSkill} /><span>SKILL</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowGit} /><span>GIT</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowRepo} /><span>REPO</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowSessionUse} /><span>USE%</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowWeek} /><span>WEEK%</span></label>
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowEffort} /><span>EFFORT</span></label>
        </div>

        <div class="row">
          <button
            type="button"
            class="btn primary"
            disabled={!statuslineDirty || savingStatusline}
            onclick={saveStatuslineConfig}
          >
            {savingStatusline ? 'saving…' : 'save status line'}
          </button>
          {#if saveBanner && saveBanner.section === 'statusline'}
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

    {#if activeTab === 'alerts'}
    <!-- ALERTS — smart tab alerting rules -->
    {#if config}
      <section class="section">
        <div class="section-label">Alert rules</div>
        <div class="hint">
          trigger visual or audio alerts when event rates exceed a threshold
          within a sliding window. rules evaluate per-envelope in the master
          bus subscription.
        </div>

        {#if alertRules.length === 0}
          <div class="hint" style="font-style: italic; margin: 12px 0;">no rules configured — click add rule to create one.</div>
        {/if}

        {#each alertRules as rule (rule.id)}
          <div class="alert-rule-row">
            <label class="kv-toggle">
              <input
                type="checkbox"
                checked={rule.enabled}
                onchange={() => updateAlertRule(rule.id, { enabled: !rule.enabled })}
              />
              <span class="alert-rule-label">
                <select
                  class="inline-select"
                  value={rule.tab_id}
                  onchange={(e) => updateAlertRule(rule.id, { tab_id: (e.target as HTMLSelectElement).value })}
                >
                  {#each TAB_ID_OPTIONS as tid (tid)}
                    <option value={tid}>{tid}</option>
                  {/each}
                </select>
                <select
                  class="inline-select"
                  value={rule.severity}
                  onchange={(e) => updateAlertRule(rule.id, { severity: (e.target as HTMLSelectElement).value as SeverityLevel })}
                >
                  <option value="debug">debug+</option>
                  <option value="info">info+</option>
                  <option value="warn">warn+</option>
                  <option value="error">error</option>
                </select>
                <span class="alert-rule-op">&ge;</span>
                <input
                  type="number"
                  class="inline-number"
                  min="1"
                  max="100"
                  value={rule.threshold}
                  onchange={(e) => updateAlertRule(rule.id, { threshold: parseInt((e.target as HTMLInputElement).value) || 3 })}
                />
                <span class="alert-rule-op">in</span>
                <input
                  type="number"
                  class="inline-number"
                  min="1"
                  max="60"
                  value={rule.window_secs}
                  onchange={(e) => updateAlertRule(rule.id, { window_secs: parseInt((e.target as HTMLInputElement).value) || 10 })}
                />
                <span class="alert-rule-op">s</span>
                <span class="alert-rule-arrow">&rarr;</span>
                <select
                  class="inline-select"
                  value={rule.action}
                  onchange={(e) => updateAlertRule(rule.id, { action: (e.target as HTMLSelectElement).value as AlertAction })}
                >
                  {#each ALERT_ACTION_OPTIONS as opt (opt.value)}
                    <option value={opt.value}>{opt.label}</option>
                  {/each}
                </select>
              </span>
            </label>
            <button type="button" class="btn btn-danger-sm" onclick={() => removeAlertRule(rule.id)}>
              &times;
            </button>
          </div>
        {/each}

        <div class="row" style="margin-top: 8px;">
          <button type="button" class="btn" onclick={addAlertRule}>+ add rule</button>
          <button
            type="button"
            class="btn primary"
            disabled={!alertsDirty || savingAlerts}
            onclick={saveAlertRules}
          >
            {savingAlerts ? 'saving…' : 'save alerts'}
          </button>
          {#if saveBanner && saveBanner.section === 'alerts'}
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
    font-family: var(--font-family);
    color: var(--amber-warm);
  }

  /* ─── Tab strip ──────────────────────────────────────────────────────── */
  .tab-strip {
    flex-shrink: 0;
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border-subtle);
    padding: 0 var(--space-lg);
    background: var(--bg-panel);
  }
  .tab-btn {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    padding: 10px 14px 8px;
    cursor: pointer;
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out);
    margin-bottom: -1px;
    text-transform: uppercase;
  }
  .tab-btn:hover {
    color: var(--amber-warm);
    background: var(--bg-hover);
  }
  .tab-btn:focus-visible {
    outline: none;
    box-shadow: inset 0 -2px 0 var(--amber-bright), 0 0 0 1px rgba(255, 200, 64, 0.3);
  }
  .tab-btn.active {
    color: var(--amber-bright);
    border-bottom-color: var(--amber-bright);
    background: var(--bg-elevated);
  }

  /* ─── Scrollable body ────────────────────────────────────────────────── */
  .settings-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-lg) var(--space-lg) var(--space-md);
  }

  /* ─── Sections — card containment ────────────────────────────────────── */
  .section {
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-lg);
    margin-bottom: var(--space-md);
    box-shadow: var(--depth-inset);
  }
  .section:last-of-type { margin-bottom: 0; }
  .section-label {
    color: var(--amber-dim);
    font-size: var(--section-header-size);
    letter-spacing: var(--section-header-spacing);
    text-transform: uppercase;
    font-weight: var(--section-header-weight);
    margin-bottom: var(--space-md);
    padding-bottom: var(--space-sm);
    border-bottom: 1px solid var(--border-subtle);
  }

  /* ─── Hints / prose ──────────────────────────────────────────────────── */
  .hint {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    font-style: italic;
    line-height: 1.55;
    margin-bottom: var(--space-md);
  }
  .hint code {
    color: var(--amber-warm);
    font-style: normal;
    background: var(--bg-elevated);
    padding: 1px 4px;
    border-radius: var(--radius-sm);
  }

  /* ─── Key-value display rows ─────────────────────────────────────────── */
  .kv {
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: var(--space-md);
    align-items: baseline;
    padding: 3px 0;
    font-size: var(--text-sm);
  }
  .kv .k {
    color: var(--amber-dim);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: var(--text-2xs);
    font-weight: 600;
  }
  .kv .v {
    color: var(--amber-warm);
    font-weight: 600;
  }
  .kv .v.path {
    word-break: break-all;
    font-size: var(--text-xs);
    color: var(--amber-dim);
  }

  /* ─── Flex utility row (buttons / inline banners) ────────────────────── */
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    margin-top: var(--space-md);
    flex-wrap: wrap;
  }

  /* ─── Checkbox toggle row ────────────────────────────────────────────── */
  .kv-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-md);
    font-size: var(--text-sm);
    color: var(--amber-warm);
    cursor: pointer;
    padding: var(--space-xs) 0;
  }
  .kv-toggle input[type="checkbox"] {
    appearance: none;
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    background: var(--bg-base);
    cursor: pointer;
    flex-shrink: 0;
    position: relative;
    transition: border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out);
  }
  .kv-toggle input[type="checkbox"]:checked {
    background: var(--amber-bright);
    border-color: var(--amber-bright);
  }
  .kv-toggle input[type="checkbox"]:checked::after {
    content: '';
    position: absolute;
    inset: 0;
    background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'%3E%3Cpath d='M1.5 5l2.5 2.5 4.5-4.5' stroke='%23080806' stroke-width='1.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E") center/10px no-repeat;
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
    font-family: var(--font-family);
    word-break: break-all;
    font-size: var(--text-xs);
  }

  /* ─── Buttons — aligned with .rift-btn system ────────────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-weight: 600;
    padding: 0 var(--space-md);
    height: 32px;
    cursor: pointer;
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out),
                box-shadow var(--duration-base) var(--ease-out);
    white-space: nowrap;
    user-select: none;
  }
  .btn:hover:not(:disabled) {
    color: var(--amber-bright);
    border-color: var(--amber-dim);
    background: var(--bg-hover);
    box-shadow: 0 0 4px rgba(255, 168, 38, 0.1);
  }
  .btn:active:not(:disabled) {
    background: rgba(255, 168, 38, 0.1);
  }
  .btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .btn.primary {
    background: var(--amber-bright);
    border-color: var(--amber-bright);
    color: var(--bg-base);
    font-weight: 700;
  }
  .btn.primary:hover:not(:disabled) {
    box-shadow: var(--glow-amber);
    color: var(--bg-base);
    background: var(--amber-bright);
  }
  .btn.primary:active:not(:disabled) {
    box-shadow: var(--glow-amber-faint);
  }

  /* ─── Banners (block — update / error notices) ───────────────────────── */
  .banner {
    margin-top: var(--space-md);
    padding: var(--space-md) var(--space-md);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-size: var(--text-xs);
    line-height: 1.5;
    background: var(--bg-panel);
  }
  .banner-ok {
    border-color: var(--term-green);
    color: var(--term-green);
    background: rgba(79, 232, 85, 0.04);
  }
  .banner-info {
    border-color: var(--amber-bright);
    color: var(--amber-warm);
    background: rgba(255, 200, 64, 0.04);
  }
  .banner-fail {
    border-color: var(--term-red);
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.04);
  }
  .banner-title {
    color: var(--amber-bright);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: var(--space-xs);
  }
  .banner-body {
    color: var(--amber-dim);
    margin-bottom: var(--space-sm);
    white-space: pre-wrap;
  }
  .banner-actions {
    display: flex;
    gap: var(--space-sm);
    margin-top: var(--space-sm);
  }

  .banner-inline {
    display: inline-flex;
    align-items: center;
    height: 32px;
    padding: 0 var(--space-md);
    border: 1px solid var(--term-green);
    border-radius: var(--radius-md);
    background: rgba(79, 232, 85, 0.06);
    color: var(--term-green);
    font-size: var(--text-2xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .banner-inline.fail {
    border-color: var(--term-red);
    background: rgba(255, 72, 72, 0.06);
    color: var(--term-red);
  }

  /* ─── Form fields ────────────────────────────────────────────────────── */
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    margin-bottom: var(--space-md);
  }
  .field-label {
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    letter-spacing: 0.10em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .field-input {
    background: var(--bg-base);
    border: 1px solid var(--border-active);
    border-radius: var(--radius-md);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    padding: 0 var(--space-md);
    height: 34px;
    line-height: 34px;
    resize: none;
    box-sizing: border-box;
    transition: border-color var(--duration-base) var(--ease-out),
                box-shadow var(--duration-base) var(--ease-out);
    caret-color: var(--amber-bright);
  }
  .field-input[rows] {
    height: auto;
    line-height: 1.6;
    padding: var(--space-sm) var(--space-md);
    resize: vertical;
  }
  .field-input:focus {
    outline: none;
    border-color: var(--amber-primary);
    box-shadow: 0 0 0 1px var(--amber-dim), var(--glow-amber);
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

  /* ─── Select — themed dropdown (replaces browser defaults) ───────────── */
  .select {
    appearance: none;
    -webkit-appearance: none;
    background: var(--bg-base);
    border: 1px solid var(--border-active);
    border-radius: var(--radius-md);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    padding: 0 28px 0 var(--space-md);
    height: 34px;
    cursor: pointer;
    transition: border-color var(--duration-base) var(--ease-out),
                box-shadow var(--duration-base) var(--ease-out);
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' stroke='%23A87830' stroke-width='1.5' fill='none' stroke-linecap='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 10px center;
  }
  .select:hover {
    border-color: var(--amber-dim);
  }
  .select:focus {
    outline: none;
    border-color: var(--amber-primary);
    box-shadow: 0 0 0 1px var(--amber-dim), var(--glow-amber);
  }
  .select:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .select option {
    background: var(--bg-elevated);
    color: var(--amber-warm);
  }

  /* ─── Radio groups ───────────────────────────────────────────────────── */
  .radio-row {
    display: flex;
    gap: var(--space-lg);
    margin-bottom: var(--space-sm);
    flex-wrap: wrap;
  }
  .radio-row.radio-wrap {
    row-gap: var(--space-sm);
  }
  .radio-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    margin-bottom: var(--space-sm);
  }
  .toggle-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-xs) var(--space-lg);
    margin-bottom: var(--space-md);
  }
  .radio {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    cursor: pointer;
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--amber-warm);
    font-weight: 600;
    padding: 3px 0;
    transition: color var(--duration-fast) var(--ease-out);
  }
  .radio:hover { color: var(--amber-bright); }
  .radio input[type="radio"] {
    appearance: none;
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-full);
    background: var(--bg-base);
    cursor: pointer;
    flex-shrink: 0;
    position: relative;
    transition: border-color var(--duration-base) var(--ease-out);
  }
  .radio input[type="radio"]:checked {
    border-color: var(--amber-bright);
  }
  .radio input[type="radio"]:checked::after {
    content: '';
    position: absolute;
    inset: 3px;
    border-radius: var(--radius-full);
    background: var(--amber-bright);
  }
  .radio input[type="radio"]:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px rgba(255, 200, 64, 0.4);
  }

  /* ─── Notification per-tab grid ──────────────────────────────────────── */
  .notif-per-tab { margin-bottom: var(--space-md); }
  .notif-tab-row {
    flex-direction: row;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-xs) 0;
  }
  .notif-tab-name {
    min-width: 80px;
    color: var(--amber-warm);
    font-weight: 600;
  }
  .notif-tab-row .select {
    height: 28px;
    font-size: var(--text-xs);
    padding-top: 0;
    padding-bottom: 0;
  }
  .save-row {
    margin-top: var(--space-md);
  }
  .save-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--amber-bright);
    border: 1px solid var(--amber-bright);
    border-radius: var(--radius-md);
    color: var(--bg-base);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 0 var(--space-md);
    height: 32px;
    cursor: pointer;
    transition: box-shadow var(--duration-base) var(--ease-out);
    user-select: none;
  }
  .save-btn:hover:not(:disabled) {
    box-shadow: var(--glow-amber);
  }
  .save-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  /* ─── Footer ─────────────────────────────────────────────────────────── */
  .settings-footer {
    flex-shrink: 0;
    border-top: 1px solid var(--border-subtle);
    padding: var(--space-md) var(--space-lg);
    display: flex;
    justify-content: flex-end;
    background: var(--bg-panel);
  }

  /* ─── Alert rules ────────────────────────────────────────────────────── */
  .alert-rule-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-sm);
    margin-bottom: var(--space-xs);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    transition: border-color var(--duration-base) var(--ease-out);
  }
  .alert-rule-row:hover {
    border-color: var(--border-active);
  }
  .alert-rule-label {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
    flex: 1;
  }
  .alert-rule-op {
    color: var(--amber-faint);
    font-size: var(--text-xs);
  }
  .alert-rule-arrow {
    color: var(--amber-bright);
    font-size: var(--text-sm);
  }
  .inline-select {
    background: var(--bg-elevated);
    color: var(--amber-warm);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    padding: var(--space-xs) var(--space-sm);
    cursor: pointer;
    transition: border-color var(--duration-base) var(--ease-out);
    appearance: none;
    -webkit-appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='5' viewBox='0 0 8 5'%3E%3Cpath d='M1 1l3 3 3-3' stroke='%23A87830' stroke-width='1' fill='none' stroke-linecap='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 4px center;
    padding-right: 18px;
  }
  .inline-select:hover { border-color: var(--amber-dim); }
  .inline-select:focus {
    outline: none;
    border-color: var(--amber-primary);
  }
  .inline-select option {
    background: var(--bg-elevated);
    color: var(--amber-warm);
  }
  .inline-number {
    background: var(--bg-elevated);
    color: var(--amber-warm);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    width: 44px;
    padding: var(--space-xs) var(--space-xs);
    text-align: center;
    transition: border-color var(--duration-base) var(--ease-out);
  }
  .inline-number:focus {
    outline: none;
    border-color: var(--amber-primary);
  }
  .btn-danger-sm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    color: var(--term-red);
    border: 1px solid rgba(255, 72, 72, 0.25);
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
    width: 24px;
    height: 24px;
    cursor: pointer;
    transition: background var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out);
    flex-shrink: 0;
  }
  .btn-danger-sm:hover {
    background: rgba(255, 72, 72, 0.12);
    border-color: rgba(255, 72, 72, 0.5);
  }
</style>
