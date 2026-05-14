// Canonical TypeScript mirror of the Rust RiftConfig struct.
// Single source of truth for all frontend components calling `config_get`.

export interface ProjectEntry {
  name: string;
  path: string;
  last_used_ms: number;
}

export interface FsConfig {
  ignore_globs: string[];
  max_depth: number;
}

export interface CockpitConfig {
  detached_pos?: { x: number; y: number; width: number; height: number } | null;
}

export interface IndexConfig {
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

export interface SessionConfig {
  enabled: boolean;
  retention_days: number;
  max_file_size_mb: number;
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
}
