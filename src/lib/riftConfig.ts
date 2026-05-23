// Canonical TypeScript mirror of the Rust RiftConfig struct.
// Single source of truth for all frontend components calling `config_get`.

export interface ProjectEntry {
  name: string;
  path: string;
  last_used_ms: number;
}

interface FsConfig {
  ignore_globs: string[];
  max_depth: number;
}

interface CockpitConfig {
  detached_pos?: { x: number; y: number; width: number; height: number } | null;
}

interface IndexConfig {
  ignore_globs: string[];
  sync_mode: string;
  camera_transform?: unknown;
  node_positions?: unknown;
  label_visibility: string;
  density: string;
}

export interface McpConfig {
  enabled: boolean;
  allow_inspection: boolean;
  allow_js_eval: boolean;
  allow_mutations: boolean;
}

export interface TerminalConfig {
  shell: ShellPref;
  font_size: number;
  font_family: string;
  line_height: number;
  scrollback: number;
  lanes_enabled: boolean;
}

export type ShellPref =
  | { kind: 'auto' }
  | { kind: 'pwsh' }
  | { kind: 'powershell' }
  | { kind: 'cmd' }
  | { kind: 'bash' }
  | { kind: 'zsh' }
  | { kind: 'sh' }
  | { kind: 'custom'; path: string }
  | { kind: 'unknown' };

export type SeverityLevel = 'debug' | 'info' | 'warn' | 'error';

export interface NotifFilterConfig {
  default_threshold: SeverityLevel;
  per_tab: Record<string, SeverityLevel>;
}

interface SessionConfig {
  enabled: boolean;
  retention_days: number;
  max_file_size_mb: number;
}

interface TreeConfig {
  heatmap_enabled: boolean;
  heatmap_window_minutes: number;
}

export interface StatusLineConfig {
  show_dir: boolean;
  show_git: boolean;
  show_repo: boolean;
  show_session: boolean;
  show_skill: boolean;
  show_effort: boolean;
  show_model: boolean;
  show_ctx: boolean;
  show_session_use: boolean;
  show_week: boolean;
  color_overrides: Record<string, string>;
}

export type AlertAction = 'flash' | 'promote' | 'tone';

export interface AlertRule {
  id: string;
  tab_id: string;
  severity: SeverityLevel;
  threshold: number;
  window_secs: number;
  action: AlertAction;
  enabled: boolean;
}

interface AlertsConfig {
  rules: AlertRule[];
}

export interface RiftConfig {
  projects: ProjectEntry[];
  fs: FsConfig;
  cockpit: CockpitConfig;
  index: IndexConfig;
  mcp: McpConfig;
  terminal: TerminalConfig;
  session: SessionConfig;
  notif_filters: NotifFilterConfig;
  tree: TreeConfig;
  statusline: StatusLineConfig;
  alerts: AlertsConfig;
  first_run_completed: boolean;
}
