# Ensemble Router — Phase 1 Corrected Spec

**Status:** Draft (post-crit review)  
**Date:** 2026-05-27  
**Origin:** RIFT_ENSEMBLE_ROUTER_SPEC.md → /aegis --crit → corrections  
**Blocks resolved:** §9 decoupling, crate structure, node graph removal, Windows process lifecycle, LLM client identity

---

## Product Identity (Resolved)

**Hybrid client model.** Rift manages models AND provides a prompt surface for non-Claude tasks.

- **Rift IS the LLM client for:** local llama-server models, remote server models, explicit `@model` prompts via command palette, cross-critique ensemble flows (Phase 3)
- **Rift is NOT the client for:** Claude Code's normal terminal operation — CC talks directly to Anthropic API as today, untouched
- **Two AI surfaces coexist:** terminal pane (Claude Code) + Rift router (everything else)
- **MCP bridge:** Router exposes MCP tools so Claude Code CAN invoke the router explicitly when desired (e.g., `@local lint this file`)

---

## Corrected Crate Structure

All new Rust code follows the existing multi-crate workspace pattern. Provider HTTP calls live inside the translator boundary (§9 enforced).

```
crates/
├── rift-bus/
│   └── src/
│       ├── translators/
│       │   ├── llm.rs              # NEW — LlmProvider trait + shared types
│       │   ├── llm_anthropic.rs    # NEW — Anthropic API translator
│       │   ├── llm_gemini.rs       # NEW — Google Gemini API translator
│       │   ├── llm_server.rs       # NEW — llama-server OpenAI-compat translator
│       │   └── llm_process.rs      # NEW — llama-server process lifecycle translator
│       ├── config.rs               # EXTEND — add EnsembleConfig, ModelConfig, LlamaServerConfig
│       └── envelope.rs             # EXTEND — add Category::Llm
├── rift-router/                    # NEW CRATE — no external calls, bus-only
│   └── src/
│       ├── lib.rs                  # RouterService
│       ├── rules.rs                # Routing rules engine
│       ├── classifier.rs           # Task type classifier
│       └── profiles.rs             # Built-in routing profiles
├── rift-core/                      # UNCHANGED
├── rift-cli/                       # UNCHANGED
├── rift-mcp/                       # EXTEND — add llm.* tool handlers
└── rift-aegis/                     # UNCHANGED

src-tauri/
└── src/
    ├── llm_commands.rs             # NEW — Tauri IPC commands for frontend
    └── lib.rs                      # EXTEND — register llm_commands, start translators
```

**§9 compliance:** `reqwest::` calls live exclusively in `crates/rift-bus/src/translators/llm_*.rs` — inside the existing CI allowlist. `crates/rift-router/` has NO external calls; it consumes/produces bus envelopes only. The boundary check passes without modification.

**CI allowlist note:** `tools/check-translator-boundary.sh` already allows `crates/rift-bus/src/translators/**/*.rs`. The new `llm_*.rs` files are automatically covered.

---

## Bus Integration

### New Category

```rust
pub enum Category {
    // ... existing variants ...
    /// LLM router events — routing decisions, provider requests/responses,
    /// model health, process lifecycle. Integration-provided (Ensemble Router).
    Llm,
}
```

### Event Types

| Event | Payload (key fields) | Producer |
|-------|---------------------|----------|
| `llm.route` | `model_id`, `task_type`, `profile`, `reason`, `was_override` | rift-router |
| `llm.request` | `model_id`, `prompt_hash`, `estimated_tokens` | llm translators |
| `llm.response` | `model_id`, `tokens_in`, `tokens_out`, `latency_ms`, `cost_usd` | llm translators |
| `llm.stream.start` | `model_id`, `request_id` | llm translators |
| `llm.stream.chunk` | `request_id`, `tokens_so_far` | llm translators (sampled, not per-token) |
| `llm.stream.end` | `request_id`, `total_tokens`, `stop_reason` | llm translators |
| `llm.health` | `model_id`, `status` (ready/loading/error/offline), `latency_ms` | llm_process translator |
| `llm.process.start` | `model_id`, `pid`, `port` | llm_process translator |
| `llm.process.stop` | `model_id`, `pid`, `exit_code` | llm_process translator |
| `llm.process.crash` | `model_id`, `pid`, `stderr_tail` | llm_process translator |

Events flow through the bus like any other category — the notification tab system, session logger, correlation engine, and bus tail all pick them up automatically.

---

## Core Rust Types (Corrected)

### Provider Trait (in `translators/llm.rs`)

```rust
use std::pin::Pin;
use futures::Stream;

/// Normalized streaming chunk across all providers.
pub struct StreamChunk {
    pub text: String,
    pub is_final: bool,
    pub token_count: Option<u32>,
    pub stop_reason: Option<StopReason>,
}

pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    Error(String),
}

/// The stream type — pinned, boxed, Send.
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send>>;

/// Normalized request across all providers.
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop_sequences: Vec<String>,
    pub system_prompt: Option<String>,
    /// Provider-specific extensions (Anthropic thinking, Gemini grounding, etc.)
    pub provider_options: Option<serde_json::Value>,
}

pub struct Message {
    pub role: Role,
    pub content: String,
}

pub enum Role { System, User, Assistant }

/// Normalized response.
pub struct CompletionResponse {
    pub content: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub model_used: String,
    pub stop_reason: StopReason,
    pub latency_ms: u64,
}

/// Provider status for health checks.
pub enum ProviderStatus {
    Ready { latency_ms: u64 },
    Loading { progress: Option<f32> },
    Error { message: String, retryable: bool },
    RateLimited { retry_after: Option<Duration> },
    Offline,
}

/// Error hierarchy — routing fallback depends on distinguishing these.
pub enum LlmError {
    /// Auth failed (wrong/expired key) — never retry with same key
    AuthFailed { provider: String },
    /// Rate limited — retry after delay
    RateLimited { retry_after: Option<Duration> },
    /// Network unreachable — retry once, then mark offline
    NetworkError { source: reqwest::Error },
    /// Stream interrupted mid-response — partial content available
    StreamInterrupted { partial_content: String, tokens_delivered: u64 },
    /// Model overloaded — retry with backoff
    Overloaded,
    /// Process not running (local model) — offer restart
    ProcessNotRunning { model_id: String },
    /// All configured models failed — surface to user
    AllProvidersFailed { attempts: Vec<(String, String)> },
    /// Invalid request (bad params) — don't retry
    InvalidRequest { message: String },
    /// Generic/unknown
    Internal { message: String },
}

/// Core trait. Uses async_trait for object safety (Box<dyn LlmProvider>).
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    async fn stream(&self, request: CompletionRequest) -> Result<CompletionStream, LlmError>;
    async fn health_check(&self) -> ProviderStatus;
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
}
```

### Configuration (extends `config.rs`)

```rust
/// Top-level config addition to RiftConfig.
pub struct EnsembleConfig {
    pub enabled: bool,
    pub active_profile: RoutingProfile,
    pub default_model: String,
    pub models: Vec<ModelConfig>,
}

pub struct ModelConfig {
    pub id: String,
    pub display_name: String,
    pub provider: ProviderType,
    pub model_identifier: String,
    pub hosting: HostingMode,
    pub endpoint: String,
    pub api_key_ref: Option<String>,  // keyring service name, not the key itself
    pub color: String,                // CSS variable name, e.g. "--model-claude"
    pub short_id: String,             // 2-4 chars
    pub capabilities: ModelCapabilities,
}

pub enum ProviderType { Anthropic, Google, LlamaServer, OpenAiCompat }

pub enum HostingMode {
    Cloud,
    Local { process_config: LlamaServerConfig },
    Remote { health_check_interval_secs: u64 },
}

pub struct LlamaServerConfig {
    pub model_path: PathBuf,
    pub flash_attention: bool,       // --flash-attn
    pub ctx_size: u32,               // --ctx-size (default: 32768)
    pub cache_type_k: KvCacheType,   // --cache-type-k (default: Q8_0)
    pub cache_type_v: KvCacheType,   // --cache-type-v (default: Q8_0)
    pub n_gpu_layers: i32,           // --n-gpu-layers (default: 99)
    pub threads: Option<u32>,        // --threads (default: auto)
    pub parallel: u32,               // --parallel (default: 1)
    pub port: u16,                   // --port
    pub cuda_visible_devices: Option<String>,
    pub auto_start: bool,
    pub extra_flags: Vec<String>,    // sanitized — validated against known flags
}

pub enum KvCacheType { F32, F16, BF16, Q8_0, Q4_0, Q4_1, IQ4_NL, Q5_0, Q5_1 }
```

---

## Windows Process Lifecycle (Corrected)

The primary workstation is Windows 11. SIGTERM does not exist on Windows.

### Startup

```rust
use std::process::Command;

fn spawn_llama_server(config: &LlamaServerConfig) -> Result<Child, LlmError> {
    let mut cmd = Command::new(&llama_server_path);
    cmd.args(build_cli_flags(config));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }

    cmd.spawn().map_err(|e| LlmError::ProcessNotRunning {
        model_id: config.model_path.display().to_string(),
    })
}
```

### Graceful Shutdown

```rust
fn stop_llama_server(child: &Child) -> Result<(), LlmError> {
    #[cfg(windows)]
    {
        // Send CTRL_BREAK_EVENT — llama-server handles this gracefully
        // Requires CREATE_NEW_PROCESS_GROUP at spawn time
        unsafe {
            windows::Win32::System::Console::GenerateConsoleCtrlEvent(
                windows::Win32::System::Console::CTRL_BREAK_EVENT,
                child.id(),
            );
        }
        // Wait up to 5 seconds for graceful exit
        // If still alive, TerminateProcess as last resort
    }

    #[cfg(unix)]
    {
        // Standard SIGTERM
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGTERM,
        )?;
    }
    Ok(())
}
```

### Orphan Protection

```rust
#[cfg(windows)]
fn create_job_object() -> windows::Win32::System::JobObjects::HANDLE {
    // Create a Job Object that terminates all child processes when Rift exits.
    // Assign every spawned llama-server to this Job.
    // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE ensures cleanup on crash.
}
```

### Startup Cleanup

On Rift boot, before spawning any auto-start models:
1. Read PID file (`~/.rift/llm-pids.json`)
2. Check if any recorded PIDs are still running
3. If running AND not responsive on expected port → kill (orphan from crash)
4. If running AND responsive → adopt (Rift restarted, model still warm)
5. Clear stale PID entries

---

## Frontend (Corrected — Uses Existing Surfaces)

### Model Color Palette (Separate from Lane Colors)

New CSS variables in `styles.css` — distinct from `--term-*` lane colors:

```css
/* Model identity colors — used ONLY on indicators, cards, badges */
/* Deliberately different hue/saturation from lane colors */
--model-claude: #E8A040;      /* warm gold — not amber-primary */
--model-gemini: #7B9CDB;      /* muted steel blue — not term-blue */
--model-local: #7BC47B;       /* sage green — not term-green */
--model-server: #6ABFBF;      /* teal — not term-cyan */
--model-custom: #B89CD9;      /* lavender — not term-purple */
--model-offline: rgba(168, 120, 48, 0.3);  /* dimmed amber */
```

### Surface Assignments (Existing Taxonomy)

| Spec Surface | Classification | Implementation |
|---|---|---|
| Model settings | Section in existing `SettingsPanel.svelte` | Add "Models" tab to existing settings panel |
| Quick switcher | Mode in existing `CommandPalette.svelte` | Add `models` category to command palette |
| Activity log | Notification Tab → **Activity** dropdown group | New `LlmActivityTabContent.svelte` |
| Model indicator | Inline in `StatusLine.svelte` + `LaneGutter.svelte` | Extend existing components |
| VRAM estimator | Inline within Settings model section | Part of `ModelCard.svelte` |
| Cost ticker | StatusLine segment | Extend `StatusLine.svelte` row 2 |

### New Components (9, down from spec's 14)

| Component | Replaces | Notes |
|---|---|---|
| `ModelCard.svelte` | ModelCard + LlamaServerConfig + RemoteModelConfig | Conditional sections by hosting mode |
| `ModelIndicator.svelte` | ModelIndicator | Gutter glyph — shape + short ID primary, color secondary (a11y) |
| `LlmActivityTabContent.svelte` | ActivityLog | Notification tab, follows existing `*TabContent.svelte` pattern |
| `AddModelFlow.svelte` | AddModelFlow | Progressive reveal, NOT wizard steps (matches Rift UX) |
| `VramEstimator.svelte` | VramEstimator | Inline in ModelCard, $derived reactivity |
| `LlmPromptInput.svelte` | (new) | Prompt input surface for Rift-originated LLM requests |
| `LlmResponsePane.svelte` | (new) | Response display pane for router-handled requests |
| `RoutingBadge.svelte` | (new) | Small badge showing active routing profile in status area |
| `ModelHealthDot.svelte` | (new) | Reusable status dot with tooltip — ready/loading/error/offline |

### New Stores (3, down from spec's 4)

| Store | Replaces | Notes |
|---|---|---|
| `llmModels.svelte.ts` | models.ts + processes.ts | Model config + process state (process state is per-model) |
| `llmRouting.svelte.ts` | routing.ts | Active profile, routing state, override state |
| `llmActivity.svelte.ts` | activity.ts | Activity feed derived from bus events (Category::Llm) |

### CommandPalette Extension

```typescript
// In CommandPalette.svelte — add 'models' category
// Triggered by Ctrl+Shift+M (existing keybind infrastructure)
// Shows: configured models grouped by hosting (Local → Server → Cloud)
// Each entry: ModelHealthDot + short_id + display_name + (offline) if applicable
// Selection sets active model for Rift router prompts
```

### StatusLine Integration

Row 1 `MODEL` segment: shows the **Rift router's active model** (e.g., `LOC` or `CLD`).  
This is separate from Claude Code's model — the StatusLine already tracks CC model via hooks.  
When no router model is selected (manual profile, no prompt pending): show `—`.

---

## MCP Tool Exposure (via rift-mcp)

Phase 1 adds these tools to the existing MCP server:

| Tool | Description |
|---|---|
| `llm_models` | List configured models with health status |
| `llm_switch` | Set active model for Rift router |
| `llm_health` | Health check results for all models |
| `llm_prompt` | Send a prompt through the router (enables CC → router bridge) |
| `llm_process_start` | Start a local llama-server model |
| `llm_process_stop` | Stop a local llama-server model |

The `llm_prompt` tool is the key integration point — Claude Code can explicitly route tasks to other models: `@local lint this` in a prompt triggers the MCP tool.

---

## Phase 1 Deliverables (Corrected)

1. `EnsembleConfig` + `ModelConfig` + `LlamaServerConfig` in `config.rs` with validation
2. `Category::Llm` bus variant + event type constants
3. `LlmProvider` trait + `LlmError` hierarchy in `translators/llm.rs`
4. `llm_server.rs` translator — OpenAI-compatible HTTP client (works for local + remote llama-server)
5. `llm_process.rs` translator — spawn/stop/monitor/health with Windows Job objects + PID cleanup
6. `llm_anthropic.rs` translator — Anthropic API client (direct HTTP, SSE streaming)
7. `llm_gemini.rs` translator — Google Gemini API client (AI Studio key)
8. `rift-router` crate — RouterService with manual profile only (no auto-routing in Phase 1)
9. Settings panel "Models" section — add/edit/remove models, test connection, VRAM estimator
10. CommandPalette `models` mode (Ctrl+Shift+M) — manual model selection
11. `ModelIndicator.svelte` — gutter glyph with health state
12. `LlmActivityTabContent.svelte` — notification tab in Activity group
13. `LlmPromptInput.svelte` + `LlmResponsePane.svelte` — basic prompt surface
14. StatusLine MODEL segment extension
15. MCP tools: `llm_models`, `llm_switch`, `llm_health`, `llm_prompt`, `llm_process_*`
16. `--model-*` CSS variables in design token system
17. API key storage via `keyring` crate (with file-based fallback for headless Linux)

**Explicitly deferred to Phase 2:** Auto-routing profiles, task classifier, @model tag parsing, cost ticker, escalation logic.  
**Explicitly deferred to Phase 3:** Cross-critique ensemble mode, split view.  
**Dropped from spec:** Node graph visualization (substrate removed in D-019), LiteLLM sidecar (native Rust clients sufficient for 3 providers).

---

## Implementation Order (Suggested)

```
1. Bus: Add Category::Llm + event type constants
2. Config: EnsembleConfig + ModelConfig + LlamaServerConfig in config.rs
3. Types: LlmProvider trait + LlmError + streaming types in translators/llm.rs
4. Spike: llm_server.rs translator (simplest — talk to local llama-server)
5. Process: llm_process.rs with Windows Job objects + health monitoring
6. Frontend: llmModels.svelte.ts store + ModelCard.svelte in SettingsPanel
7. Frontend: CommandPalette models mode + ModelIndicator glyph
8. Frontend: LlmPromptInput + LlmResponsePane (basic prompt surface)
9. Frontend: LlmActivityTabContent notification tab
10. Cloud: llm_anthropic.rs + llm_gemini.rs translators
11. Router: rift-router crate with manual profile
12. MCP: llm_* tool handlers in rift-mcp
13. Polish: StatusLine integration, VRAM estimator, health dots, a11y pass
```

Steps 1-5 are backend foundation. Steps 6-9 are frontend MVP. Steps 10-12 are cloud + routing. Step 13 is polish before ship.

---

## Open Questions (Reduced)

1. **llama-server binary management:** Bundle/download, or require user install? Recommendation: detect on PATH, offer to download correct release binary if missing.
2. **Per-pane vs global model selection:** Phase 1 = global (one active model for all Rift prompts). Per-pane deferred to Phase 2 with terminal session model.
3. **Streaming UX:** Phase 1 streams responses token-by-token in LlmResponsePane. Ensemble streaming deferred to Phase 3.
4. **VRAM estimation formula:** `model_file_size + (ctx_size × kv_bytes_per_token × num_layers × 2) + cuda_overhead`. Need model-architecture-dependent constants — parse from GGUF metadata or maintain a lookup table.
