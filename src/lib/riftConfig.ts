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
  color_palette: string;
  custom_palette: Record<string, string>;
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
  show_thinking: boolean;
  show_effort: boolean;
  show_model: boolean;
  show_ctx: boolean;
  show_session_use: boolean;
  show_week: boolean;
  show_cost: boolean;
  /** Per-pane foreground-process CPU% segment (optional; defaults on). */
  show_cpu?: boolean;
  /** Per-pane foreground-process RSS segment (optional; defaults on). */
  show_ram?: boolean;
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

interface IntegrationsConfig {
  aegis_enabled: boolean;
  index_enabled: boolean;
}

export type ProviderType = 'anthropic' | 'google' | 'llama_server' | 'open_ai_compat' | 'cli';
export type RoutingProfile = 'manual' | 'cost_optimized' | 'quality_first' | 'balanced';
export type KvCacheType = 'f32' | 'f16' | 'bf16' | 'q8_0' | 'q4_0' | 'q4_1' | 'iq4_nl' | 'q5_0' | 'q5_1';

export interface LlamaServerConfig {
  model_path: string;
  flash_attention: boolean;
  ctx_size: number;
  cache_type_k: KvCacheType;
  cache_type_v: KvCacheType;
  /** Layers offloaded to GPU. 99 = all; a negative value = auto (lets
   *  llama-server's device-memory fitter choose the split). */
  n_gpu_layers: number;
  /** `--cpu-moe`: keep all MoE expert tensors on CPU (big VRAM saving for
   *  MoE models, modest speed cost). */
  cpu_moe: boolean;
  /** `--n-cpu-moe N`: offload experts for the first N layers to CPU.
   *  Ignored when cpu_moe is true. null = omit. */
  n_cpu_moe: number | null;
  /** `--cache-ram N`: host-RAM prompt-reuse cache in MiB (default 8192).
   *  0 = disable. null = omit (use default). Not model weights. */
  cache_ram: number | null;
  threads: number | null;
  parallel: number;
  port: number;
  cuda_visible_devices: string | null;
  auto_start: boolean;
  /** Auto-restart this local server if it crashes (bounded retry on the
   *  backend health monitor). false = a crash surfaces as an error status
   *  with no recovery. */
  auto_restart: boolean;
  extra_flags: string[];
}

/** Architecture facts read from a GGUF file header by the `gguf_inspect`
 *  command. Every field is nullable — the writer may omit any key, and the
 *  VRAM estimator falls back to its filename heuristic for missing fields. */
export interface GgufMeta {
  architecture: string | null;
  n_layers: number | null;
  n_embd: number | null;
  n_head: number | null;
  n_head_kv: number | null;
  expert_count: number | null;
  parameter_count: number | null;
}

export type HostingMode =
  | { mode: 'cloud' }
  | { mode: 'local' } & LlamaServerConfig
  | { mode: 'remote'; health_check_interval_secs: number };

export interface ModelCapabilities {
  max_context_tokens: number;
  supports_streaming: boolean;
  supports_tool_use: boolean;
  cost_per_1m_input: number;
  cost_per_1m_output: number;
  strength_tags: string[];
}

export interface ModelConfig {
  id: string;
  display_name: string;
  provider: ProviderType;
  model_identifier: string;
  hosting: HostingMode;
  endpoint: string;
  api_key_ref: string | null;
  color: string;
  short_id: string;
  capabilities: ModelCapabilities;
  /** Whether the model is available for use. Disabled models are skipped by
   *  the router and hidden from model pickers. */
  enabled: boolean;
}

export interface EnsembleConfig {
  enabled: boolean;
  active_profile: RoutingProfile;
  default_model: string;
  models: ModelConfig[];
  /** Optional tiny model that refines the router's ambiguous `Other` bucket.
   *  `null`/absent = keyword routing only. */
  classifier_model_id?: string | null;
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
  integrations: IntegrationsConfig;
  ensemble: EnsembleConfig;
}
