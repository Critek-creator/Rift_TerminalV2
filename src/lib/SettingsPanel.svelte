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
  import { PALETTES, CUSTOM_PALETTE_KEYS, PALETTE_KEY_LABELS, getDefaultCustomColors } from './terminalPalettes';
  import { llmModels, loadFromConfig as loadLlmFromConfig } from './llmModels.svelte';
  import ModelCard from './ModelCard.svelte';

  interface Props {
    popoutId: number;
  }

  let { popoutId }: Props = $props();

  type SettingsTab = 'general' | 'terminal' | 'integrations' | 'index' | 'tree' | 'mcp' | 'statusline' | 'alerts' | 'models';
  let activeTab = $state<SettingsTab>('general');

  const SETTINGS_TABS: { id: SettingsTab; label: string }[] = [
    { id: 'general',      label: 'GENERAL' },
    { id: 'terminal',     label: 'TERMINAL' },
    { id: 'integrations', label: 'INTEGRATIONS' },
    { id: 'statusline',   label: 'STATUS LINE' },
    { id: 'index',        label: 'INDEX' },
    { id: 'tree',         label: 'TREE' },
    { id: 'mcp',          label: 'MCP' },
    { id: 'alerts',       label: 'ALERTS' },
    { id: 'models',       label: 'MODELS' },
  ];

  let tabStripEl = $state<HTMLElement | null>(null);

  /** Roving-tabindex keyboard nav for the WAI-ARIA tablist pattern:
   *  Left/Right move + activate the adjacent tab, Home/End jump to ends. */
  function onTabKeydown(e: KeyboardEvent) {
    const idx = SETTINGS_TABS.findIndex((t) => t.id === activeTab);
    if (idx === -1) return;
    let next = idx;
    switch (e.key) {
      case 'ArrowRight': case 'ArrowDown': next = (idx + 1) % SETTINGS_TABS.length; break;
      case 'ArrowLeft':  case 'ArrowUp':   next = (idx - 1 + SETTINGS_TABS.length) % SETTINGS_TABS.length; break;
      case 'Home': next = 0; break;
      case 'End':  next = SETTINGS_TABS.length - 1; break;
      default: return;
    }
    e.preventDefault();
    activeTab = SETTINGS_TABS[next].id;
    // Move focus to the newly-activated tab button (roving tabindex).
    const btn = tabStripEl?.querySelectorAll<HTMLButtonElement>('.tab-btn')[next];
    btn?.focus();
  }

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
  // Index settings — old graph controls (sync_mode, label_visibility, density)
  // removed in D-019 (IndexGraph → vault browser list). Config fields kept
  // for serde backwards compatibility but no UI controls remain.

  let savingFs = $state(false);
  let savingMcp = $state(false);
  let savingTerminal = $state(false);
  let savingTree = $state(false);
  let saveBanner = $state<{
    section: 'fs' | 'index' | 'mcp' | 'terminal' | 'notif' | 'tree' | 'statusline' | 'alerts' | 'models';
    ok: boolean;
    msg: string;
  } | null>(null);
  let classifierBanner = $state<{ ok: boolean; msg: string } | null>(null);
  let registeringClassifier = $state(false);

  // Terminal — D-018 groundwork (audit close 2026-04-29).
  let termShellKind = $state<ShellKind>('auto');
  let termCustomPath = $state('');
  let termFontSize = $state(13);
  let termFontFamily = $state("'JetBrains Mono', monospace");
  let termLineHeight = $state(1.55);
  let termScrollback = $state(1000);
  let termLanesEnabled = $state(true);
  let termColorPalette = $state('amber');
  let customColors = $state<Record<string, string>>({});
  let customEditorOpen = $state(false);

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
  let termAdvancedOpen = $state(false);

  // StatusLine — segment visibility toggles.
  let slShowDir = $state(true);
  let slShowGit = $state(true);
  let slShowRepo = $state(true);
  let slShowSession = $state(true);
  let slShowSkill = $state(true);
  let slShowThinking = $state(true);
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
    const prevRules = [...alertRules];
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
      alertRules = prevRules;
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
    const prevHeatmapEnabled = treeHeatmapEnabled;
    const prevHeatmapWindow = treeHeatmapWindow;
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
      treeHeatmapEnabled = prevHeatmapEnabled;
      treeHeatmapWindow = prevHeatmapWindow;
      saveBanner = { section: 'tree', ok: false, msg: String(err) };
    } finally {
      savingTree = false;
    }
  }

  // Notification filters
  const SEVERITY_OPTIONS: SeverityLevel[] = ['debug', 'info', 'warn', 'error'];
  const NOTIF_TAB_IDS = ['errors', 'hooks', 'commands', 'aegis', 'index', 'bustail', 'todo', 'git', 'agents', 'sentinel', 'filesystem', 'mcp', 'health', 'llm-activity'];
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
    const prevThreshold = notifDefaultThreshold;
    const prevPerTab = { ...notifPerTab };
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
      notifDefaultThreshold = prevThreshold;
      notifPerTab = prevPerTab;
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
  function previewPalette(paletteId: string | null): void {
    window.dispatchEvent(new CustomEvent('rift:palette-preview', { detail: paletteId }));
  }

  function broadcastConfigChanged(): void {
    window.dispatchEvent(new CustomEvent('rift:config-changed'));
  }

  async function onToggleMcp(value: boolean, key: keyof McpConfig) {
    if (!config) return;
    savingMcp = true;
    saveBanner = null;
    const prevMcp = { ...config.mcp };
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
      // Rollback config.mcp to pre-toggle state so checkbox reverts.
      if (config) config = { ...config, mcp: prevMcp };
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
      || termColorPalette !== (config.terminal.color_palette ?? 'amber')
      || JSON.stringify(customColors) !== JSON.stringify(config.terminal.custom_palette ?? {})
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
      || slShowThinking !== (config.statusline?.show_thinking ?? true)
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
    // Terminal snapshot — defaults for old configs are filled by serde-side
    // #[serde(default)], so c.terminal is always present at runtime.
    termShellKind = c.terminal.shell.kind;
    termCustomPath = c.terminal.shell.kind === 'custom' ? c.terminal.shell.path : '';
    termFontSize = c.terminal.font_size;
    termFontFamily = c.terminal.font_family ?? "'JetBrains Mono', monospace";
    termLineHeight = c.terminal.line_height;
    termScrollback = c.terminal.scrollback;
    termLanesEnabled = c.terminal.lanes_enabled;
    termColorPalette = c.terminal.color_palette ?? 'amber';
    customColors = c.terminal.custom_palette ?? {};
    if (termColorPalette === 'custom' && Object.keys(customColors).length === 0) {
      customColors = getDefaultCustomColors();
    }
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
    slShowThinking = sl.show_thinking ?? true;
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
    // LLM models snapshot.
    loadLlmFromConfig(c.ensemble);
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
    const prevIgnoreText = fsIgnoreText;
    const prevMaxDepth = fsMaxDepth;
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
      fsIgnoreText = prevIgnoreText;
      fsMaxDepth = prevMaxDepth;
      saveBanner = { section: 'fs', ok: false, msg: String(err) };
    } finally {
      savingFs = false;
    }
  }

  async function saveTerminalConfig() {
    if (!config) return;
    savingTerminal = true;
    saveBanner = null;
    const prevShellKind = termShellKind;
    const prevCustomPath = termCustomPath;
    const prevFontSize = termFontSize;
    const prevFontFamily = termFontFamily;
    const prevLineHeight = termLineHeight;
    const prevScrollback = termScrollback;
    const prevLanesEnabled = termLanesEnabled;
    const prevColorPalette = termColorPalette;
    const prevCustomColors = { ...customColors };
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
          color_palette: termColorPalette,
          custom_palette: customColors,
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = {
        section: 'terminal',
        ok: true,
        msg: 'terminal saved',
      };
    } catch (err) {
      termShellKind = prevShellKind;
      termCustomPath = prevCustomPath;
      termFontSize = prevFontSize;
      termFontFamily = prevFontFamily;
      termLineHeight = prevLineHeight;
      termScrollback = prevScrollback;
      termLanesEnabled = prevLanesEnabled;
      termColorPalette = prevColorPalette;
      customColors = prevCustomColors;
      saveBanner = { section: 'terminal', ok: false, msg: String(err) };
    } finally {
      savingTerminal = false;
    }
  }

  async function saveStatuslineConfig() {
    if (!config) return;
    savingStatusline = true;
    saveBanner = null;
    const prevSl = {
      dir: slShowDir, git: slShowGit, repo: slShowRepo,
      session: slShowSession, skill: slShowSkill, thinking: slShowThinking,
      effort: slShowEffort, model: slShowModel, ctx: slShowCtx, sessionUse: slShowSessionUse,
      week: slShowWeek,
    };
    try {
      const next: RiftConfig = {
        ...config,
        statusline: {
          show_dir: slShowDir,
          show_git: slShowGit,
          show_repo: slShowRepo,
          show_session: slShowSession,
          show_skill: slShowSkill,
          show_thinking: slShowThinking,
          show_effort: slShowEffort,
          show_model: slShowModel,
          show_ctx: slShowCtx,
          show_session_use: slShowSessionUse,
          show_week: slShowWeek,
          show_cost: config.statusline?.show_cost ?? true,
          color_overrides: config.statusline?.color_overrides ?? {},
        },
      };
      await invoke('config_save', { cfg: next });
      config = next;
      broadcastConfigChanged();
      saveBanner = { section: 'statusline', ok: true, msg: 'status line saved' };
    } catch (err) {
      slShowDir = prevSl.dir; slShowGit = prevSl.git; slShowRepo = prevSl.repo;
      slShowSession = prevSl.session; slShowSkill = prevSl.skill; slShowThinking = prevSl.thinking;
      slShowEffort = prevSl.effort; slShowModel = prevSl.model; slShowCtx = prevSl.ctx; slShowSessionUse = prevSl.sessionUse;
      slShowWeek = prevSl.week;
      saveBanner = { section: 'statusline', ok: false, msg: String(err) };
    } finally {
      savingStatusline = false;
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
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <!-- WAI-ARIA tabs pattern: focus rolls across the tabs (roving tabindex),
       the tablist itself stays out of the tab sequence. -->
  <div class="tab-strip" role="tablist" aria-label="settings sections"
       bind:this={tabStripEl} onkeydown={onTabKeydown}>
    {#each SETTINGS_TABS as tab (tab.id)}
      <button type="button"
        role="tab"
        id="settings-tab-{tab.id}"
        aria-selected={activeTab === tab.id}
        aria-controls="settings-panel-{tab.id}"
        tabindex={activeTab === tab.id ? 0 : -1}
        class="tab-btn"
        class:active={activeTab === tab.id}
        onclick={() => (activeTab = tab.id)}
      >{tab.label}</button>
    {/each}
  </div>

  <div class="settings-body"
       role="tabpanel"
       id="settings-panel-{activeTab}"
       aria-labelledby="settings-tab-{activeTab}"
  >

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
        <div class="v">{appVersion} <span class="beta-badge">BETA</span></div>
      </div>
      <div class="kv">
        <div class="k">identifier</div>
        <div class="v">{appIdentifier}</div>
      </div>
      <div class="row" style="margin-top: 8px; gap: var(--space-sm);">
        <button
          type="button"
          class="btn"
          onclick={() => window.dispatchEvent(new Event('rift:show-welcome'))}
        >WELCOME GUIDE</button>
        <button
          type="button"
          class="btn"
          onclick={() => invoke('open_url', { url: 'https://www.patreon.com/cw/AbyssalArtsDev' })}
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
        <div class="row" style="margin-top: 6px; gap: var(--space-sm);">
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
        <div class="banner banner-fail" role="alert">{configError}</div>
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
            <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
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

      <div class="row">
        <button type="button"
          class="btn primary"
          disabled={!notifDirty || savingNotif}
          onclick={saveNotifFilters}
        >{savingNotif ? 'saving…' : 'save filters'}</button>
        {#if saveBanner && saveBanner.section === 'notif'}
          <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
            {saveBanner.msg}
          </span>
        {/if}
      </div>
    </section>
    {/if}

    {#if activeTab === 'integrations'}
    <!-- INTEGRATIONS -->
    {#await invoke<{claude_dir_exists: boolean, node_available: boolean, node_version: string | null, aegis: {installed: boolean, enabled: boolean, path: string}, index: {installed: boolean, enabled: boolean, path: string}}>('integration_detect')}
      <section class="section">
        <div class="section-label">Integrations</div>
        <div class="hint">Detecting integrations…</div>
      </section>
    {:then status}
      <section class="section">
        <div class="section-label">Integrations</div>
        <div class="hint">
          Optional subsystems that extend Rift's cockpit. Tabs light up
          automatically when an integration is detected and enabled.
        </div>
        <div class="field-row">
          <div class="field-row-main">
            <label class="field-row-label" for="aegis-toggle">Aegis</label>
            <span class="field-row-desc">agent observability</span>
          </div>
          {#if status.aegis.installed}
            <span class="rift-badge rift-badge--ok">installed</span>
          {:else}
            <span class="rift-badge rift-badge--warn">not installed</span>
          {/if}
          {#if config}
          <label class="rift-switch">
            <input
              id="aegis-toggle"
              type="checkbox"
              checked={config.integrations.aegis_enabled}
              disabled={!status.aegis.installed}
              onchange={(e) => {
                if (config) {
                  config.integrations.aegis_enabled = e.currentTarget.checked;
                  invoke('config_save', { cfg: config });
                }
              }}
            />
            <span class="rift-switch-track"></span>
          </label>
          {/if}
        </div>
        <div class="field-row">
          <div class="field-row-main">
            <label class="field-row-label" for="index-toggle">Abyssal Index</label>
            <span class="field-row-desc">knowledge cockpit</span>
          </div>
          {#if status.index.installed}
            <span class="rift-badge rift-badge--ok">installed</span>
          {:else}
            <span class="rift-badge rift-badge--warn">not installed</span>
          {/if}
          {#if config}
          <label class="rift-switch">
            <input
              id="index-toggle"
              type="checkbox"
              checked={config.integrations.index_enabled}
              disabled={!status.index.installed}
              onchange={(e) => {
                if (config) {
                  config.integrations.index_enabled = e.currentTarget.checked;
                  invoke('config_save', { cfg: config });
                }
              }}
            />
            <span class="rift-switch-track"></span>
          </label>
          {/if}
        </div>
        <div class="field-row">
          <div class="field-row-main">
            <span class="field-row-label">Node.js</span>
            <span class="field-row-desc">required for maintenance scripts (18+)</span>
          </div>
          {#if status.node_available}
            <span class="rift-badge rift-badge--ok">{status.node_version}</span>
          {:else}
            <span class="rift-badge rift-badge--warn">not found</span>
          {/if}
        </div>
        {#if !status.claude_dir_exists}
          <div class="banner banner-fail" role="alert">
            Claude Code directory (~/.claude/) not found. Install Claude Code to use integrations.
          </div>
        {/if}
      </section>
    {:catch}
      <section class="section">
        <div class="section-label">Integrations</div>
        <div class="hint">Detection failed — try restarting Rift.</div>
      </section>
    {/await}
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

        <div class="section-label" style="margin-top: 12px;">Color Palette</div>
        <div class="radio-col">
          {#each PALETTES as palette (palette.id)}
            <label class="radio palette-radio"
              onmouseenter={() => previewPalette(palette.id)}
              onmouseleave={() => previewPalette(null)}
            >
              <input type="radio" name="color-palette" value={palette.id}
                checked={termColorPalette === palette.id}
                onchange={() => (termColorPalette = palette.id)}
              />
              <span class="palette-option">
                <span class="palette-name">{palette.label}</span>
                <span class="palette-desc">{palette.description}</span>
                <span class="palette-preview">
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.foreground}">Aa</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.red}">Er</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.green}">Ok</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.yellow}">Yl</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.blue}">Cl</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.magenta}">Ag</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.cyan}">Hk</span>
                  <span class="palette-swatch" style="background: {palette.theme.background}; color: {palette.theme.white}">Wh</span>
                </span>
              </span>
            </label>
          {/each}
          <label class="radio palette-radio"
            onmouseenter={() => previewPalette('custom')}
            onmouseleave={() => previewPalette(null)}
          >
            <input type="radio" name="color-palette" value="custom"
              checked={termColorPalette === 'custom'}
              onchange={() => {
                termColorPalette = 'custom';
                if (Object.keys(customColors).length === 0) {
                  customColors = getDefaultCustomColors();
                }
                customEditorOpen = true;
              }}
            />
            <span class="palette-option">
              <span class="palette-name">Custom</span>
              <span class="palette-desc">Define your own color scheme</span>
              {#if Object.keys(customColors).length > 0}
                {@const bg = customColors.background ?? '#080806'}
                <span class="palette-preview">
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.foreground ?? '#FFA826'}">Aa</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.red ?? '#FF4848'}">Er</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.green ?? '#4FE855'}">Ok</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.yellow ?? '#FFC840'}">Yl</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.blue ?? '#6CB6FF'}">Cl</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.magenta ?? '#C58FFF'}">Ag</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.cyan ?? '#6FE0E0'}">Hk</span>
                  <span class="palette-swatch" style="background: {bg}; color: {customColors.white ?? '#E8E4D8'}">Wh</span>
                </span>
              {/if}
            </span>
          </label>
        </div>

        {#if termColorPalette === 'custom'}
          <button type="button" class="disclosure-toggle" onclick={() => (customEditorOpen = !customEditorOpen)}>
            <span class="disclosure-caret">{customEditorOpen ? '▾' : '▸'}</span>
            Custom Colors
          </button>

          {#if customEditorOpen}
            <div class="custom-palette-editor">
              <div class="custom-palette-actions">
                <span class="field-label">base on preset:</span>
                {#each PALETTES.slice(0, 4) as preset (preset.id)}
                  <button type="button" class="btn" onclick={() => {
                    const fresh: Record<string, string> = {};
                    for (const key of CUSTOM_PALETTE_KEYS) {
                      const val = preset.theme[key as keyof typeof preset.theme];
                      if (typeof val === 'string') fresh[key] = val;
                    }
                    customColors = fresh;
                    previewPalette('custom');
                  }}>{preset.label}</button>
                {/each}
              </div>
              <div class="custom-palette-grid">
                {#each CUSTOM_PALETTE_KEYS as key (key)}
                  <label class="custom-color-field">
                    <span class="custom-color-label">{PALETTE_KEY_LABELS[key]}</span>
                    <span class="custom-color-input-row">
                      <input
                        type="color"
                        class="custom-color-picker"
                        value={customColors[key]?.startsWith('rgba') ? '#000000' : (customColors[key] ?? '#000000')}
                        oninput={(e) => {
                          customColors = { ...customColors, [key]: (e.target as HTMLInputElement).value };
                        }}
                      />
                      <input
                        type="text"
                        class="custom-color-hex"
                        value={customColors[key] ?? ''}
                        placeholder="#000000"
                        spellcheck="false"
                        oninput={(e) => {
                          customColors = { ...customColors, [key]: (e.target as HTMLInputElement).value };
                        }}
                      />
                    </span>
                  </label>
                {/each}
              </div>
            </div>
          {/if}
        {/if}

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

        <button type="button" class="disclosure-toggle" onclick={() => (termAdvancedOpen = !termAdvancedOpen)}>
          <span class="disclosure-caret">{termAdvancedOpen ? '▾' : '▸'}</span>
          Advanced
        </button>

        {#if termAdvancedOpen}
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
            <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
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
          <label class="kv-toggle"><input type="checkbox" bind:checked={slShowThinking} /><span>THINKING</span></label>
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
            <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
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
          The index tab streams vault activity events from the Abyssal Index.
          Graph-view settings (density, labels, sync mode) were removed when
          the node graph was replaced by the vault browser list.
        </div>
        <div class="hint" style="margin-top: 8px; color: var(--amber-dim);">
          No configurable settings for the vault event stream yet.
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
            <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
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
          <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
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
            <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
              {saveBanner.msg}
            </span>
          {/if}
        </div>
      </section>
    {:else}
      <div class="hint">loading config…</div>
    {/if}
    {/if}

    {#if activeTab === 'models'}
    <section class="section">
      <div class="section-label">Ensemble Router</div>
      <div class="hint">
        Route prompts to multiple LLM providers — local models, Anthropic, Google Gemini,
        or any OpenAI-compatible endpoint. Enable the router to configure models and choose
        a routing strategy.
      </div>
      <label class="kv-toggle">
        <input
          type="checkbox"
          checked={llmModels.enabled}
          onchange={(e) => llmModels.setEnabled((e.target as HTMLInputElement).checked)}
        />
        Enable Ensemble Router
      </label>

      {#if llmModels.enabled}
      <div class="field" style="margin-top: var(--space-md);">
        <span class="field-label">Routing Strategy</span>
        <select
          value={llmModels.activeProfile}
          onchange={(e) => llmModels.setActiveProfile((e.target as HTMLSelectElement).value as any)}
          class="select"
        >
          <option value="manual">Manual — you pick the model per request</option>
          <option value="cost_optimized">Cost Optimized — cheapest model that fits the task</option>
          <option value="quality_first">Quality First — best model available, cost secondary</option>
          <option value="balanced">Balanced — weighs quality and cost equally</option>
        </select>
      </div>
      {/if}
    </section>

    {#if llmModels.enabled}
    <section class="section">
      <div class="section-label">Configured Models</div>
      {#if llmModels.models.length === 0}
        <div class="hint" style="padding: var(--space-sm) 0;">
          No models configured yet. Add a provider below to get started.
        </div>
      {/if}
      {#each llmModels.models as model (model.id)}
        <ModelCard
          {model}
          isDefault={llmModels.defaultModel === model.id}
          onremove={() => llmModels.removeModel(model.id)}
          onsetdefault={() => llmModels.setDefaultModel(model.id)}
        />
      {/each}

      <div class="add-model-section">
        <span class="field-label" style="margin-bottom: var(--space-sm); display: block;">Add Model</span>
        <div class="provider-cards">
          <button type="button" class="provider-card" onclick={() => llmModels.addModel('llama_server')}>
            <span class="provider-card-name" style="color: var(--term-green)">Local</span>
            <span class="provider-card-desc">Run a GGUF model locally via llama-server</span>
          </button>
          <button type="button" class="provider-card" onclick={() => llmModels.addModel('anthropic')}>
            <span class="provider-card-name" style="color: var(--term-blue)">Anthropic</span>
            <span class="provider-card-desc">Claude models via the Anthropic API</span>
          </button>
          <button type="button" class="provider-card" onclick={() => llmModels.addGeminiCliModel()}>
            <span class="provider-card-name" style="color: var(--model-gemini)">Gemini</span>
            <span class="provider-card-desc">Sign in with Google — no API key. Pick a model and go.</span>
          </button>
          <button type="button" class="provider-card" onclick={() => llmModels.addModel('open_ai_compat')}>
            <span class="provider-card-name" style="color: var(--term-purple)">OpenAI-compat</span>
            <span class="provider-card-desc">Any OpenAI-compatible API endpoint</span>
          </button>
          <button type="button" class="provider-card" onclick={() => llmModels.addModel('cli')}>
            <span class="provider-card-name" style="color: var(--model-custom)">CLI tool</span>
            <span class="provider-card-desc">Drive any external CLI tool as a model</span>
          </button>
        </div>
        <button
          type="button"
          class="provider-advanced-link"
          onclick={() => llmModels.addModel('google')}
        >Advanced: use a Gemini API key instead →</button>
      </div>

      <div class="models-save-row">
        <button type="button" class="btn primary" onclick={async () => {
          saveBanner = null;
          try {
            await llmModels.save();
            broadcastConfigChanged();
            saveBanner = { section: 'models', ok: true, msg: 'models saved' };
          } catch (err) {
            saveBanner = { section: 'models', ok: false, msg: String(err) };
          }
        }}>Save Models</button>
        {#if saveBanner && saveBanner.section === 'models'}
          <span class="banner-inline" class:fail={!saveBanner.ok} role="status">
            {saveBanner.msg}
          </span>
        {/if}
      </div>
    </section>

    <section class="section">
      <div class="section-label">Task Classifier</div>
      <div class="hint">
        A tiny local model (Llama&nbsp;3.2&nbsp;1B) that refines the router's
        ambiguous <em>Other</em> bucket — long, keyword-less prompts that would
        otherwise default to the cloud. It fires <strong>only</strong> on that
        bucket under an auto profile; keyword-matched prompts are untouched and
        pay zero extra latency. Keeps grunt work off the paid API.
      </div>

      <div class="field" style="margin-top: var(--space-md);">
        <span class="field-label">Active classifier</span>
        <select
          value={llmModels.classifierModelId ?? ''}
          onchange={(e) => llmModels.setClassifier((e.target as HTMLSelectElement).value || null)}
          class="select"
        >
          <option value="">None — keyword routing only</option>
          {#each llmModels.models as m (m.id)}
            <option value={m.id}>{m.display_name || m.id}</option>
          {/each}
        </select>
        <span class="hint" style="display: block; margin-top: var(--space-sm);">
          Selecting here is an edit — click <em>Save Models</em> above to persist.
          The one-click button below registers + persists in one step.
        </span>
      </div>

      <div class="models-save-row" style="margin-top: var(--space-sm);">
        <button
          type="button"
          class="btn"
          disabled={registeringClassifier}
          onclick={async () => {
            classifierBanner = null;
            registeringClassifier = true;
            try {
              const res = await llmModels.registerClassifier();
              broadcastConfigChanged();
              classifierBanner = { ok: res.file_present, msg: res.message };
            } catch (err) {
              classifierBanner = { ok: false, msg: String(err) };
            } finally {
              registeringClassifier = false;
            }
          }}
        >
          {registeringClassifier ? 'Registering…' : 'Register Llama 3.2 1B classifier'}
        </button>
        {#if classifierBanner}
          <span class="banner-inline" class:fail={!classifierBanner.ok} role="status">
            {classifierBanner.msg}
          </span>
        {/if}
      </div>
    </section>
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
    box-shadow: var(--sep-glow);
    padding: 0 var(--space-lg);
    background: var(--bg-panel);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab-strip::-webkit-scrollbar { height: 0; }
  .tab-btn {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    padding: var(--space-md) var(--space-14) var(--space-8);
    cursor: pointer;
    transition: color var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out);
    margin-bottom: -1px;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .tab-btn:hover {
    color: var(--amber-warm);
    background: var(--bg-hover);
  }
  .tab-btn:focus-visible {
    outline: 2px solid transparent;
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
    border-radius: var(--radius-md);
    padding: var(--space-lg);
    margin-bottom: var(--space-md);
    box-shadow: var(--depth-inset);
  }
  .section:last-of-type { margin-bottom: 0; }
  .section-label {
    color: var(--amber-bright);
    font-size: var(--type-title-size);
    font-weight: var(--type-title-weight);
    letter-spacing: var(--type-title-spacing);
    text-shadow: var(--glow-amber-faint);
    margin-bottom: var(--space-md);
    padding-bottom: var(--space-sm);
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
    padding: 1px var(--space-xs);
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
    font-size: var(--type-caption-size);
    letter-spacing: var(--type-caption-spacing);
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

  /* ─── BETA badge (About → version) ───────────────────────────────────── */
  .beta-badge {
    display: inline-block;
    margin-left: var(--space-sm);
    padding: 0 var(--space-xs);
    background: rgba(255, 168, 38, 0.18);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-primary);
    vertical-align: middle;
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
    outline: 2px solid transparent;
    box-shadow: 0 0 0 2px rgba(255, 200, 64, 0.4);
  }
  .kv-toggle input[type="checkbox"]:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ─── Integration field rows (label + badge + switch) ───────────────── */
  .field-row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-8) 0;
  }
  .field-row + .field-row {
    border-top: 1px solid var(--border-subtle);
  }
  .field-row-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .field-row-label {
    color: var(--amber-warm);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .field-row-desc {
    color: var(--amber-faint);
    font-size: var(--text-2xs);
    letter-spacing: 0.02em;
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
    height: var(--control-lg);
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
    background: rgba(79, 232, 85, 0.05);
  }
  .banner-info {
    border-color: var(--amber-bright);
    color: var(--amber-warm);
    background: rgba(255, 200, 64, 0.05);
  }
  .banner-fail {
    border-color: var(--term-red);
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.05);
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
    height: var(--control-lg);
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
    outline: 2px solid transparent;
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
    outline: 2px solid transparent;
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
    outline: 2px solid transparent;
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
    outline: 2px solid transparent;
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
    outline: 2px solid transparent;
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
    height: var(--space-24);
    cursor: pointer;
    transition: background var(--duration-base) var(--ease-out),
                border-color var(--duration-base) var(--ease-out);
    flex-shrink: 0;
  }
  .btn-danger-sm:hover {
    background: rgba(255, 72, 72, 0.12);
    border-color: rgba(255, 72, 72, 0.5);
  }

  .palette-option {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .palette-name {
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .palette-desc {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
  }
  .palette-preview {
    display: flex;
    gap: var(--space-xs);
    margin-top: var(--space-xs);
  }
  .palette-swatch {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 20px;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 700;
    font-family: var(--font-family);
    letter-spacing: 0.04em;
  }
  .disclosure-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    background: none;
    border: none;
    color: var(--amber-dim);
    font-family: var(--font-family);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    cursor: pointer;
    padding: var(--space-sm) 0;
    margin-top: var(--space-md);
    transition: color var(--duration-fast) var(--ease-out);
  }
  .disclosure-toggle:hover {
    color: var(--amber-warm);
  }
  .disclosure-caret {
    font-size: var(--text-xs);
    color: var(--amber-faint);
  }

  /* ─── Custom palette editor ──────────────────────────────────────────── */
  .custom-palette-editor {
    margin-top: var(--space-sm);
    padding: var(--space-md);
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }
  .custom-palette-actions {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-bottom: var(--space-md);
    flex-wrap: wrap;
  }
  .custom-palette-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-sm) var(--space-lg);
  }
  .custom-color-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .custom-color-label {
    color: var(--amber-dim);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .custom-color-input-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .custom-color-picker {
    appearance: none;
    -webkit-appearance: none;
    width: 28px;
    height: 24px;
    border: 1px solid var(--border-active);
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .custom-color-picker::-webkit-color-swatch-wrapper {
    padding: 2px;
  }
  .custom-color-picker::-webkit-color-swatch {
    border: none;
    border-radius: 2px;
  }
  .custom-color-hex {
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    padding: 2px var(--space-sm);
    height: 24px;
    width: 82px;
    transition: border-color var(--duration-base) var(--ease-out);
  }
  .custom-color-hex:focus {
    outline: 2px solid transparent;
    border-color: var(--amber-primary);
  }
  .custom-color-hex::placeholder {
    color: var(--amber-faint);
    font-style: italic;
  }

  /* ─── Models — provider cards ────────────────────────────────────────── */
  .add-model-section {
    margin-top: var(--space-lg);
    padding-top: var(--space-md);
    border-top: 1px solid var(--border-subtle);
  }
  .provider-cards {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-sm);
  }
  .provider-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-md) var(--space-md);
    cursor: pointer;
    text-align: left;
    transition: border-color var(--duration-base) var(--ease-out),
                background var(--duration-base) var(--ease-out),
                box-shadow var(--duration-base) var(--ease-out);
  }
  .provider-card:hover {
    border-color: var(--amber-dim);
    background: var(--bg-hover);
    box-shadow: 0 0 6px rgba(255, 168, 38, 0.06);
  }
  .provider-card:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .provider-card-name {
    font-family: var(--font-family);
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .provider-card-desc {
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    line-height: 1.4;
  }
  .provider-advanced-link {
    margin-top: var(--space-sm);
    background: none;
    border: none;
    padding: 2px 0;
    color: var(--amber-faint);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    cursor: pointer;
    transition: color var(--duration-base) var(--ease-out);
  }
  .provider-advanced-link:hover {
    color: var(--amber-warm);
    text-decoration: underline;
  }
  .provider-advanced-link:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 2px;
  }

  /* ─── Models — save row ──────────────────────────────────────────────── */
  .models-save-row {
    margin-top: var(--space-lg);
    display: flex;
    align-items: center;
    gap: var(--space-md);
    justify-content: flex-end;
  }
</style>
