//! Tauri IPC commands for the Ensemble Router.
//!
//! Phase 2: RouterService sits in the prompt path. @model tags are parsed,
//! the classifier + profile engine select the model, bus events fire on
//! every routing decision + response, and retryable failures escalate
//! through the fallback chain.

use futures_util::StreamExt;
use rift_bus::config::{load_config, save_config, HostingMode, ModelConfig, ProviderType};
use rift_bus::translators::llm::{CompletionRequest, LlmProvider, Message, Role};
use rift_bus::translators::llm_anthropic::AnthropicProvider;
use rift_bus::translators::llm_cli::{
    gemini_auth_status as cli_gemini_auth_status,
    gemini_enable_headless as cli_gemini_enable_headless, CliProvider, GeminiAuthStatus,
};
use rift_bus::translators::llm_gemini::GeminiProvider;
use rift_bus::translators::llm_process::{ApplyOutcome, ProcessManager};
use rift_bus::translators::llm_server::LlamaServerProvider;
use rift_bus::{Category, Envelope, RiftBus};
use rift_router::{RouterService, RoutingDecision, TaskType};
use serde::Serialize;
use serde_json::json;
use tauri::ipc::Channel;
use tauri::State;

#[derive(Serialize)]
pub struct LlmCompleteResult {
    pub content: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub model_used: String,
    pub latency_ms: u64,
    pub task_type: String,
    pub routing_reason: String,
    pub was_overridden: bool,
    pub cost_usd: f64,
    pub escalated: bool,
}

/// Create a provider instance from a model config.
pub fn create_provider(
    model: &rift_bus::config::ModelConfig,
) -> Result<Box<dyn LlmProvider>, String> {
    let api_key = rift_bus::keyring::resolve_api_key(&model.id, model.api_key_ref.as_deref());

    match (&model.hosting, &model.provider) {
        (HostingMode::Cloud, ProviderType::Anthropic) => Ok(Box::new(AnthropicProvider::new(
            &model.endpoint,
            &api_key,
            &model.model_identifier,
        ))),
        (HostingMode::Cloud, ProviderType::Google) => Ok(Box::new(GeminiProvider::new(
            &model.endpoint,
            &api_key,
            &model.model_identifier,
        ))),
        // CLI provider — matches any hosting (the command, not an HTTP host, is
        // what matters). Must precede the `(Cloud, _)` catch-all so a CLI model
        // configured with `hosting = "cloud"` still routes here. The command
        // template lives in `endpoint`; no API key. See `translators::llm_cli`.
        (_, ProviderType::Cli) => Ok(Box::new(CliProvider::new(
            &model.endpoint,
            &model.model_identifier,
        ))),
        (HostingMode::Cloud, _) => Err(format!("unsupported cloud provider: {:?}", model.provider)),
        (HostingMode::Local { process_config }, _) => {
            let endpoint = format!("http://127.0.0.1:{}", process_config.port);
            Ok(Box::new(LlamaServerProvider::new(
                endpoint,
                &model.model_identifier,
            )))
        }
        (HostingMode::Remote { .. }, _) => Ok(Box::new(LlamaServerProvider::new(
            &model.endpoint,
            &model.model_identifier,
        ))),
    }
}

fn compute_cost(model: &rift_bus::config::ModelConfig, tokens_in: u64, tokens_out: u64) -> f64 {
    let cost_in = (tokens_in as f64 / 1_000_000.0) * model.capabilities.cost_per_1m_input;
    let cost_out = (tokens_out as f64 / 1_000_000.0) * model.capabilities.cost_per_1m_output;
    cost_in + cost_out
}

/// Parse a classifier model's free-text reply into a [`TaskType`]. Tolerant
/// of surrounding whitespace/punctuation and stray words — matches the first
/// `snake_case` enum name that appears in the reply. Returns `None` on no
/// match so the caller keeps the keyword result.
fn parse_task_type(raw: &str) -> Option<TaskType> {
    let norm = raw.to_lowercase();
    // Longest / most-specific names first so e.g. "code_generation" wins over
    // a bare "code"; "other" last since it's the safe keyword default anyway.
    const TABLE: &[(&str, TaskType)] = &[
        ("code_generation", TaskType::CodeGeneration),
        ("code_refactoring", TaskType::CodeRefactoring),
        ("large_context_analysis", TaskType::LargeContextAnalysis),
        ("lint_format", TaskType::LintFormat),
        ("documentation", TaskType::Documentation),
        ("quick_query", TaskType::QuickQuery),
        ("architecture", TaskType::Architecture),
        ("debug", TaskType::Debug),
        ("other", TaskType::Other),
    ];
    TABLE
        .iter()
        .find(|(name, _)| norm.contains(name))
        .map(|(_, t)| *t)
}

/// Ask the configured tiny classifier model to label a prompt. Returns `None`
/// on any failure (HTTP error, unparseable reply) — the caller then keeps the
/// keyword classification. §9: the HTTP call is the existing llm_server
/// translator behind `provider.complete`.
async fn classify_via_llm(provider: &dyn LlmProvider, prompt: &str) -> Option<TaskType> {
    // Per-token definitions matter more than model size: bare token names let a
    // small model pattern-match "code" words to code_refactoring (4/8 on a test
    // battery); these definitions took gemma-4-E4B to 8/8.
    const SYSTEM: &str =
        "You are a task-type classifier. Reply with EXACTLY ONE token. Definitions:\n\
        - code_generation: write NEW code/functions from scratch\n\
        - code_refactoring: restructure/rename/extract EXISTING code\n\
        - lint_format: linting, formatting, style (clippy, prettier, rustfmt)\n\
        - large_context_analysis: analyze/review/audit across the WHOLE codebase\n\
        - documentation: write docs, comments, docstrings, summaries, explanations\n\
        - quick_query: a short factual question\n\
        - architecture: high-level design / system decisions\n\
        - debug: diagnose or fix a bug, error, crash, panic, or failing test\n\
        - other: anything else";
    // GBNF grammar: forces the model to emit exactly one valid TaskType token —
    // no prose, no punctuation. A weak 1B can't wander off-format. llama.cpp
    // extension, passed through the llm_server translator via provider_options.
    const GRAMMAR: &str = r#"root ::= "code_generation" | "code_refactoring" | "lint_format" | "large_context_analysis" | "documentation" | "quick_query" | "architecture" | "debug" | "other""#;
    // Cap input — a classifier needs the gist, not the whole payload.
    let truncated: String = prompt.chars().take(2000).collect();
    let request = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: truncated,
        }],
        max_tokens: Some(8),
        temperature: Some(0.0),
        stop_sequences: vec![],
        system_prompt: Some(SYSTEM.to_string()),
        provider_options: Some(serde_json::json!({ "grammar": GRAMMAR })),
    };
    let resp = provider.complete(request).await.ok()?;
    parse_task_type(&resp.content)
}

/// Phase 2 refinement: when the keyword router lands on the ambiguous
/// `TaskType::Other` bucket under an auto profile, ask the tiny classifier
/// model to relabel and re-route. Falls back to the original decision on any
/// failure or when no classifier is configured — so the keyword result always
/// stands as the floor. Never fires on overridden/`@tag`/Manual routes.
pub(crate) async fn maybe_refine_with_classifier(
    router: &RouterService,
    ensemble: &rift_bus::config::EnsembleConfig,
    prompt: &str,
    decision: RoutingDecision,
) -> RoutingDecision {
    use rift_bus::config::RoutingProfile;

    if decision.task_type != TaskType::Other
        || decision.was_overridden
        || matches!(ensemble.active_profile, RoutingProfile::Manual)
    {
        return decision;
    }

    // Classifier must be configured, available, resolvable — and must not be
    // asked to classify a route that already picked the classifier itself.
    let Some(cid) = ensemble.classifier_model_id.as_deref() else {
        return decision;
    };
    if !router.is_available(cid) {
        return decision;
    }
    let model = match router.find_model(cid) {
        Ok(m) => m.clone(),
        Err(_) => return decision,
    };
    let provider = match create_provider(&model) {
        Ok(p) => p,
        Err(_) => return decision,
    };

    // Classify the clean (tag-stripped) prompt.
    let clean = rift_router::parse_model_tag(prompt).clean_prompt;
    let refined = match classify_via_llm(provider.as_ref(), &clean).await {
        // A confirmed `Other` is identical to the keyword result — no re-route.
        Some(t) if t != TaskType::Other => t,
        _ => return decision,
    };

    match router.route_with_hint(prompt, None, Some(refined)) {
        Ok(mut refined_decision) => {
            refined_decision.reason =
                format!("{} (classifier → {:?})", refined_decision.reason, refined);
            refined_decision
        }
        Err(_) => decision,
    }
}

fn publish_route_event(bus: &RiftBus, decision: &RoutingDecision) {
    let mut env = Envelope::new(Category::Llm, "llm.route");
    env.payload = json!({
        "model_id": decision.model_id,
        "task_type": decision.task_type,
        "profile": decision.profile,
        "reason": decision.reason,
        "was_overridden": decision.was_overridden,
        "fallback_count": decision.fallback_chain.len(),
    });
    bus.publish(env);
}

fn publish_response_event(
    bus: &RiftBus,
    model_id: &str,
    tokens_in: u64,
    tokens_out: u64,
    latency_ms: u64,
    cost_usd: f64,
    escalated: bool,
) {
    let mut env = Envelope::new(Category::Llm, "llm.response");
    env.payload = json!({
        "model_id": model_id,
        "tokens_in": tokens_in,
        "tokens_out": tokens_out,
        "latency_ms": latency_ms,
        "cost_usd": cost_usd,
        "escalated": escalated,
    });
    bus.publish(env);
}

fn publish_error_event(bus: &RiftBus, model_id: &str, error: &str, retryable: bool) {
    let mut env = Envelope::new(Category::Llm, "llm.error");
    env.payload = json!({
        "model_id": model_id,
        "error": error,
        "retryable": retryable,
    });
    bus.publish(env);
}

/// Streamed token event sent through a [`Channel`] to the frontend.
#[derive(Clone, Serialize)]
pub struct LlmStreamEvent {
    pub text: String,
    pub is_final: bool,
    pub tokens_so_far: u64,
}

/// Phase 3 — result from one model in an ensemble comparison.
#[derive(Clone, Serialize)]
pub struct EnsembleModelResult {
    pub model_id: String,
    pub model_short_id: String,
    pub content: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub latency_ms: u64,
    pub cost_usd: f64,
    pub error: Option<String>,
}

/// Phase 3 — ensemble comparison result.
#[derive(Serialize)]
pub struct EnsembleResult {
    pub results: Vec<EnsembleModelResult>,
    pub task_type: String,
    pub critique: Option<String>,
    pub total_cost_usd: f64,
}

/// Store an API key securely in the OS keyring.
#[tauri::command]
pub async fn llm_key_store(model_id: String, key: String) -> Result<(), String> {
    rift_bus::keyring::store_api_key(&model_id, &key)
}

/// Delete an API key from the OS keyring.
#[tauri::command]
pub async fn llm_key_delete(model_id: String) -> Result<(), String> {
    rift_bus::keyring::delete_api_key(&model_id)
}

/// Phase 2 prompt command: router-driven model selection with escalation.
///
/// - `model_id`: Optional explicit override. If absent, the router decides.
/// - `prompt`: The user's prompt. May contain `@model` tags.
#[tauri::command]
pub async fn llm_complete(
    bus: State<'_, RiftBus>,
    model_id: Option<String>,
    prompt: String,
) -> Result<LlmCompleteResult, String> {
    let config = load_config().map_err(|e| format!("config load error: {e}"))?;
    let mut router = RouterService::new(config.ensemble.clone());

    // Route the prompt (handles @model tags, profile logic, availability)
    let decision = router
        .route(&prompt, model_id.as_deref())
        .map_err(|e| format!("{e}"))?;

    // Phase 2: refine the ambiguous `Other` bucket with the tiny classifier
    // (no-op unless one is configured; only fires on auto, non-overridden routes).
    let decision = maybe_refine_with_classifier(&router, &config.ensemble, &prompt, decision).await;

    publish_route_event(&bus, &decision);

    // Get clean prompt (strip @tag if present)
    let parsed = rift_router::parse_model_tag(&prompt);
    let clean_prompt = parsed.clean_prompt;

    // Try primary model, then escalate on retryable failure
    let mut current_model_id = decision.model_id.clone();
    let mut fallback_chain = decision.fallback_chain.clone();
    let mut escalated = false;

    loop {
        let model = router
            .find_model(&current_model_id)
            .map_err(|e| format!("{e}"))?
            .clone();

        let provider = create_provider(&model)?;

        let request = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: clean_prompt.clone(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };

        match provider.complete(request).await {
            Ok(resp) => {
                let cost = compute_cost(&model, resp.tokens_in, resp.tokens_out);

                publish_response_event(
                    &bus,
                    &model.id,
                    resp.tokens_in,
                    resp.tokens_out,
                    resp.latency_ms,
                    cost,
                    escalated,
                );

                return Ok(LlmCompleteResult {
                    content: resp.content,
                    tokens_in: resp.tokens_in,
                    tokens_out: resp.tokens_out,
                    model_used: resp.model_used,
                    latency_ms: resp.latency_ms,
                    task_type: format!("{:?}", decision.task_type),
                    routing_reason: decision.reason.clone(),
                    was_overridden: decision.was_overridden,
                    cost_usd: cost,
                    escalated,
                });
            }
            Err(err) => {
                let retryable = err.is_retryable();
                publish_error_event(&bus, &current_model_id, &err.to_string(), retryable);

                if !retryable || fallback_chain.is_empty() {
                    return Err(format!("{err}"));
                }

                // Mark failed model and try next in chain
                router.mark_unavailable(&current_model_id);

                if let Some(next) =
                    router.escalate(&current_model_id, &fallback_chain, &clean_prompt)
                {
                    tracing::info!(
                        "llm_complete: escalating from {} to {}",
                        current_model_id,
                        next.model_id
                    );
                    publish_route_event(&bus, &next);
                    current_model_id = next.model_id;
                    fallback_chain = next.fallback_chain;
                    escalated = true;
                } else {
                    return Err(format!(
                        "all models failed — last error from {}: {err}",
                        current_model_id
                    ));
                }
            }
        }
    }
}

/// Streaming variant of [`llm_complete`].
///
/// Uses `provider.stream()` to deliver tokens token-by-token through a
/// Tauri [`Channel`]. Each chunk is sent as an [`LlmStreamEvent`].
/// The final accumulated text and metadata are returned as the
/// command's return value so the frontend can update token / cost
/// display after the stream finishes.
///
/// Mid-stream errors do NOT escalate through the fallback chain —
/// partial content has already been delivered to the frontend, so
/// retrying on a different model would produce confusing duplicated
/// output. The error is returned as `Err(String)` so the caller can
/// render an inline error indicator.
#[tauri::command]
pub async fn llm_stream(
    bus: State<'_, RiftBus>,
    model_id: Option<String>,
    prompt: String,
    on_chunk: Channel<LlmStreamEvent>,
) -> Result<LlmCompleteResult, String> {
    let config = load_config().map_err(|e| format!("config load error: {e}"))?;
    let router = RouterService::new(config.ensemble.clone());

    // Route the prompt (handles @model tags, profile logic, availability).
    let decision = router
        .route(&prompt, model_id.as_deref())
        .map_err(|e| format!("{e}"))?;

    // Phase 2: refine the ambiguous `Other` bucket with the tiny classifier
    // (no-op unless one is configured; only fires on auto, non-overridden routes).
    let decision = maybe_refine_with_classifier(&router, &config.ensemble, &prompt, decision).await;

    publish_route_event(&bus, &decision);

    // Strip @tag so the provider sees only the clean user text.
    let parsed = rift_router::parse_model_tag(&prompt);
    let clean_prompt = parsed.clean_prompt;

    let chosen_model_id = decision.model_id.clone();

    let model = router
        .find_model(&chosen_model_id)
        .map_err(|e| format!("{e}"))?
        .clone();

    let provider = create_provider(&model)?;

    let request = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: clean_prompt.clone(),
        }],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: None,
        provider_options: None,
    };

    let start = std::time::Instant::now();

    let mut stream = provider.stream(request).await.map_err(|e| format!("{e}"))?;

    let mut full_text = String::new();
    let mut tokens_so_far: u64 = 0;
    // Estimate input tokens from prompt char count (≈4 chars/token).
    // Providers don't report input tokens in stream chunks.
    let estimated_tokens_in: u64 = (clean_prompt.len() as u64).div_ceil(4);
    let mut final_tokens_out: u64 = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                full_text.push_str(&chunk.text);
                tokens_so_far += chunk.text.len() as u64;

                if let Some(tc) = chunk.token_count {
                    final_tokens_out = tc as u64;
                }

                let is_final = chunk.is_final;

                // Best-effort send — if the channel is closed (window
                // dismissed mid-stream) we stop iterating.
                if on_chunk
                    .send(LlmStreamEvent {
                        text: chunk.text,
                        is_final,
                        tokens_so_far,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(err) => {
                // Mid-stream error — do not escalate (partial content sent).
                let retryable = err.is_retryable();
                publish_error_event(&bus, &chosen_model_id, &err.to_string(), retryable);
                return Err(format!("{err}"));
            }
        }
    }

    let latency_ms = start.elapsed().as_millis() as u64;
    let cost = compute_cost(&model, estimated_tokens_in, final_tokens_out);

    publish_response_event(
        &bus,
        &model.id,
        estimated_tokens_in,
        final_tokens_out,
        latency_ms,
        cost,
        false,
    );

    Ok(LlmCompleteResult {
        content: full_text,
        tokens_in: estimated_tokens_in,
        tokens_out: final_tokens_out,
        model_used: model.id.clone(),
        latency_ms,
        task_type: format!("{:?}", decision.task_type),
        routing_reason: decision.reason.clone(),
        was_overridden: decision.was_overridden,
        cost_usd: cost,
        escalated: false,
    })
}

/// Phase 3 — ensemble comparison. Dispatches the same prompt to two models
/// in parallel and optionally runs a critique step (model B reviews model A's
/// output). Returns both results plus the critique.
///
/// - `model_ids`: Exactly 2 model IDs. If omitted, the router picks the top 2.
/// - `prompt`: The user's prompt.
/// - `critique`: If true, after both complete, send model A's output to model B
///   with a critique system prompt.
/// - `on_chunk_a` / `on_chunk_b`: Streaming channels for each model's output.
#[tauri::command]
pub async fn llm_ensemble(
    bus: State<'_, RiftBus>,
    model_ids: Option<Vec<String>>,
    prompt: String,
    critique: Option<bool>,
    on_chunk_a: Channel<LlmStreamEvent>,
    on_chunk_b: Channel<LlmStreamEvent>,
) -> Result<EnsembleResult, String> {
    let config = load_config().map_err(|e| format!("config load error: {e}"))?;
    let router = RouterService::new(config.ensemble.clone());

    let parsed = rift_router::parse_model_tag(&prompt);
    let clean_prompt = parsed.clean_prompt;
    let task_type = rift_router::classifier::classify(&clean_prompt);

    // Resolve the two model IDs
    let (id_a, id_b) = match model_ids {
        Some(ids) if ids.len() >= 2 => (ids[0].clone(), ids[1].clone()),
        _ => pick_two_models(&router, &task_type)?,
    };

    let model_a = router
        .find_model(&id_a)
        .map_err(|e| format!("{e}"))?
        .clone();
    let model_b = router
        .find_model(&id_b)
        .map_err(|e| format!("{e}"))?
        .clone();

    // Publish ensemble start event
    let mut start_env = Envelope::new(Category::Llm, "llm.ensemble.start");
    start_env.payload = json!({
        "model_a": id_a,
        "model_b": id_b,
        "task_type": task_type,
    });
    bus.publish(start_env);

    // Dispatch both models in parallel
    let prompt_a = clean_prompt.clone();
    let prompt_b = clean_prompt.clone();
    let ma = model_a.clone();
    let mb = model_b.clone();

    let (result_a, result_b) = tokio::join!(
        run_streaming_model(&ma, &prompt_a, &on_chunk_a),
        run_streaming_model(&mb, &prompt_b, &on_chunk_b),
    );

    let res_a = build_ensemble_model_result(&model_a, result_a);
    let res_b = build_ensemble_model_result(&model_b, result_b);

    let total_cost = res_a.cost_usd + res_b.cost_usd;

    // Optional critique: send model A's output to model B for review
    let critique_text =
        if critique.unwrap_or(false) && res_a.error.is_none() && res_b.error.is_none() {
            match run_critique(&model_b, &clean_prompt, &res_a.content).await {
                Ok(text) => Some(text),
                Err(e) => Some(format!("Critique failed: {e}")),
            }
        } else {
            None
        };

    // Publish ensemble complete event
    let mut end_env = Envelope::new(Category::Llm, "llm.ensemble.complete");
    end_env.payload = json!({
        "model_a": id_a,
        "model_b": id_b,
        "total_cost_usd": total_cost,
        "has_critique": critique_text.is_some(),
    });
    bus.publish(end_env);

    // Publish cost events for each model
    if res_a.error.is_none() {
        publish_response_event(
            &bus,
            &res_a.model_id,
            res_a.tokens_in,
            res_a.tokens_out,
            res_a.latency_ms,
            res_a.cost_usd,
            false,
        );
    }
    if res_b.error.is_none() {
        publish_response_event(
            &bus,
            &res_b.model_id,
            res_b.tokens_in,
            res_b.tokens_out,
            res_b.latency_ms,
            res_b.cost_usd,
            false,
        );
    }

    Ok(EnsembleResult {
        results: vec![res_a, res_b],
        task_type: format!("{task_type:?}"),
        critique: critique_text,
        total_cost_usd: total_cost,
    })
}

/// Pick two distinct models for ensemble comparison. Prefers models with
/// different providers for maximum diversity of perspective.
fn pick_two_models(
    router: &RouterService,
    task_type: &rift_router::TaskType,
) -> Result<(String, String), String> {
    let models = router.models();
    if models.len() < 2 {
        return Err("ensemble requires at least 2 configured models".into());
    }

    let tag = rift_router::profiles::task_type_tag(task_type);

    // Sort: tag-matched first, then by cost descending (quality first for ensemble)
    let mut ranked: Vec<&rift_bus::config::ModelConfig> = models.iter().collect();
    ranked.sort_by(|a, b| {
        let a_match = a.capabilities.strength_tags.iter().any(|t| t == tag);
        let b_match = b.capabilities.strength_tags.iter().any(|t| t == tag);
        b_match.cmp(&a_match).then_with(|| {
            b.capabilities
                .cost_per_1m_input
                .partial_cmp(&a.capabilities.cost_per_1m_input)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    let first = ranked[0].id.clone();
    // Pick second from a different provider if possible
    let second = ranked[1..]
        .iter()
        .find(|m| m.provider != ranked[0].provider)
        .unwrap_or(&ranked[1])
        .id
        .clone();

    Ok((first, second))
}

/// Stream a single model's response, sending chunks through the channel.
/// Returns (content, tokens_in, tokens_out, latency_ms) or an error.
async fn run_streaming_model(
    model: &rift_bus::config::ModelConfig,
    prompt: &str,
    channel: &Channel<LlmStreamEvent>,
) -> Result<(String, u64, u64, u64), String> {
    let provider = create_provider(model)?;

    let request = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: prompt.to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: None,
        provider_options: None,
    };

    let start = std::time::Instant::now();
    let mut stream = provider.stream(request).await.map_err(|e| format!("{e}"))?;

    let mut full_text = String::new();
    let mut tokens_so_far: u64 = 0;
    let estimated_tokens_in: u64 = (prompt.len() as u64).div_ceil(4);
    let mut final_tokens_out: u64 = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                full_text.push_str(&chunk.text);
                tokens_so_far += chunk.text.len() as u64;
                if let Some(tc) = chunk.token_count {
                    final_tokens_out = tc as u64;
                }
                let _ = channel.send(LlmStreamEvent {
                    text: chunk.text,
                    is_final: chunk.is_final,
                    tokens_so_far,
                });
            }
            Err(e) => return Err(format!("{e}")),
        }
    }

    let latency_ms = start.elapsed().as_millis() as u64;
    Ok((full_text, estimated_tokens_in, final_tokens_out, latency_ms))
}

fn build_ensemble_model_result(
    model: &rift_bus::config::ModelConfig,
    result: Result<(String, u64, u64, u64), String>,
) -> EnsembleModelResult {
    match result {
        Ok((content, tokens_in, tokens_out, latency_ms)) => {
            let cost = compute_cost(model, tokens_in, tokens_out);
            EnsembleModelResult {
                model_id: model.id.clone(),
                model_short_id: model.short_id.clone(),
                content,
                tokens_in,
                tokens_out,
                latency_ms,
                cost_usd: cost,
                error: None,
            }
        }
        Err(e) => EnsembleModelResult {
            model_id: model.id.clone(),
            model_short_id: model.short_id.clone(),
            content: String::new(),
            tokens_in: 0,
            tokens_out: 0,
            latency_ms: 0,
            cost_usd: 0.0,
            error: Some(e),
        },
    }
}

/// Run the critique step: send model A's output to model B for review.
async fn run_critique(
    critic_model: &rift_bus::config::ModelConfig,
    original_prompt: &str,
    response_a: &str,
) -> Result<String, String> {
    let provider = create_provider(critic_model)?;

    let critique_prompt = format!(
        "The user asked: \"{original_prompt}\"\n\n\
         Another AI model responded:\n\n---\n{response_a}\n---\n\n\
         Critique this response. Identify strengths, weaknesses, factual errors, \
         and missed nuances. Be specific and constructive."
    );

    let request = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: critique_prompt,
        }],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: Some(
            "You are a critical reviewer. Analyze the given AI response for accuracy, \
             completeness, and quality. Be concise but thorough."
                .to_string(),
        ),
        provider_options: None,
    };

    let resp = provider
        .complete(request)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(resp.content)
}

// ---------------------------------------------------------------------------
// Local process lifecycle (UI counterparts to the MCP llm_process_* tools)
// ---------------------------------------------------------------------------

/// Start a local llama-server for `model_id`. UI counterpart to the MCP
/// `llm_process_start` tool — lets the Settings model cards start a server
/// without a Rift restart. Reads the persisted config, so the frontend must
/// save model edits before calling this. Returns the spawned PID.
///
/// Runs the spawn on a blocking thread so the async runtime is never stalled
/// (consistent with the sync→async command migration).
#[tauri::command]
pub async fn llm_model_start(
    pm: State<'_, std::sync::Arc<ProcessManager>>,
    model_id: String,
) -> Result<u32, String> {
    let config = load_config().map_err(|e| e.to_string())?;
    let model = config
        .ensemble
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("model not found: {model_id}"))?;
    let process_config = match &model.hosting {
        HostingMode::Local { process_config } => process_config.clone(),
        _ => return Err(format!("model '{model_id}' is not a local model")),
    };

    let pm = pm.inner().clone();
    tokio::task::spawn_blocking(move || pm.start(&model_id, &process_config))
        .await
        .map_err(|e| format!("start join error: {e}"))?
        .map_err(|e| e.to_string())
}

/// Stop a running local llama-server. UI counterpart to MCP `llm_process_stop`.
/// `stop` polls up to 5s for a graceful exit, so it runs on a blocking thread.
#[tauri::command]
pub async fn llm_model_stop(
    pm: State<'_, std::sync::Arc<ProcessManager>>,
    model_id: String,
) -> Result<(), String> {
    let pm = pm.inner().clone();
    tokio::task::spawn_blocking(move || pm.stop(&model_id))
        .await
        .map_err(|e| format!("stop join error: {e}"))?
        .map_err(|e| e.to_string())
}

/// List the model ids of all currently-running managed processes. The UI calls
/// this on mount to seed status dots (bus events keep them live afterwards).
#[tauri::command]
pub fn llm_models_running(pm: State<'_, std::sync::Arc<ProcessManager>>) -> Vec<String> {
    pm.running_models()
}

/// Apply the persisted config for `model_id` to its running server.
///
/// Fixes the stale-server trap: llama-server reads launch flags (ctx_size,
/// gpu layers, flash-attn, …) only at spawn, so a running model keeps OLD
/// flags after a config edit until restarted. The frontend/caller must save
/// model edits first (this reads the persisted config from disk), then call
/// this to reconcile the running server. Returns `"restarted"` (launch args
/// drifted → stop+start), `"unchanged"` (already matches), or `"not_running"`.
#[tauri::command]
pub async fn llm_model_apply_config(
    pm: State<'_, std::sync::Arc<ProcessManager>>,
    model_id: String,
) -> Result<String, String> {
    let config = load_config().map_err(|e| e.to_string())?;
    let model = config
        .ensemble
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("model not found: {model_id}"))?;
    let process_config = match &model.hosting {
        HostingMode::Local { process_config } => process_config.clone(),
        _ => return Err(format!("model '{model_id}' is not a local model")),
    };

    let pm = pm.inner().clone();
    let outcome = tokio::task::spawn_blocking(move || pm.apply_config(&model_id, &process_config))
        .await
        .map_err(|e| format!("apply_config join error: {e}"))?
        .map_err(|e| e.to_string())?;

    Ok(match outcome {
        ApplyOutcome::Restarted => "restarted",
        ApplyOutcome::Unchanged => "unchanged",
        ApplyOutcome::NotRunning => "not_running",
    }
    .to_string())
}

// ---------------------------------------------------------------------------
// VRAM-estimate inputs (GGUF metadata + GPU detection)
// ---------------------------------------------------------------------------

/// Read a GGUF model file's metadata header (layers, hidden size, head counts,
/// expert count, total params) for an accurate VRAM estimate. Reads only the
/// header KV section, not tensor weights, so it's cheap even for huge models.
/// The frontend falls back to its filename heuristic when a field is absent.
///
/// Runs on a blocking thread — file I/O must not stall the async runtime.
#[tauri::command]
pub async fn gguf_inspect(path: String) -> Result<rift_bus::translators::gguf::GgufMeta, String> {
    tokio::task::spawn_blocking(move || {
        rift_bus::translators::gguf::inspect(std::path::Path::new(&path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("gguf_inspect join error: {e}"))?
}

/// Best-effort total VRAM of the primary GPU, in MiB. Queries `nvidia-smi`
/// (NVIDIA only); returns `None` if the tool is absent or output can't be
/// parsed, in which case the frontend keeps its configured default rather than
/// showing a wrong number. Runs on a blocking thread (spawns a subprocess).
#[tauri::command]
pub async fn gpu_vram_mb() -> Option<u32> {
    tokio::task::spawn_blocking(|| {
        let mut cmd = std::process::Command::new("nvidia-smi");
        cmd.args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        // First line is the primary GPU's total memory, in MiB (nounits).
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<u32>().ok())
    })
    .await
    .ok()
    .flatten()
}

/// Best-effort *currently-used* VRAM of the primary GPU, in MiB. Queries
/// `nvidia-smi` (NVIDIA only); returns `None` if the tool is absent or the
/// output can't be parsed. NOTE: this is GPU-wide usage (desktop + every
/// process), not just our llama-server — the StatusLine tooltip says so. Runs
/// on a blocking thread (spawns a subprocess); polled while a local model runs.
#[tauri::command]
pub async fn gpu_vram_used_mb() -> Option<u32> {
    tokio::task::spawn_blocking(|| {
        let mut cmd = std::process::Command::new("nvidia-smi");
        cmd.args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        // First line is the primary GPU's used memory, in MiB (nounits).
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<u32>().ok())
    })
    .await
    .ok()
    .flatten()
}

/// Register (or refresh) the tiny Llama-3.2-1B classifier used to refine the
/// router's ambiguous `TaskType::Other` bucket. Idempotent: upserts a
/// `llama-classifier` model and points `ensemble.classifier_model_id` at it.
///
/// The model is enabled + auto-started **only when the GGUF is actually
/// present** at `model_path`, so registering against a missing file never
/// spams launch errors and never adds routing latency (a disabled model is
/// dropped by `RouterService`, and refinement falls back to keyword routing).
/// Drop the GGUF and re-run to activate. Defaults: `C:\Models\Llama-3.2-1B-Instruct-Q6_K.gguf`;
/// the port auto-picks the lowest free slot from 8082 up so it never collides with another worker.
#[tauri::command]
pub async fn llm_classifier_register(
    cached: tauri::State<'_, crate::CachedConfig>,
    model_path: Option<String>,
    port: Option<u16>,
) -> Result<serde_json::Value, String> {
    const CLASSIFIER_ID: &str = "llama-classifier";

    let path = std::path::PathBuf::from(
        model_path.unwrap_or_else(|| r"C:\Models\Llama-3.2-1B-Instruct-Q6_K.gguf".to_string()),
    );
    let file_present = path.exists();

    // Base on the in-memory cache (the app's source of truth that `config_get`
    // returns), NOT a fresh disk read — otherwise our write is invisible to the
    // frontend and gets clobbered by the next cache flush.
    let mut cfg = cached.get();

    // Ports already bound by OTHER local models (exclude any prior classifier
    // entry so re-registration can reuse its own slot).
    let used_ports: std::collections::HashSet<u16> = cfg
        .ensemble
        .models
        .iter()
        .filter(|m| m.id != CLASSIFIER_ID)
        .filter_map(|m| match &m.hosting {
            HostingMode::Local { process_config } => Some(process_config.port),
            _ => None,
        })
        .collect();

    // Port priority: explicit arg → existing classifier's port (if still free)
    // → lowest free port from 8082 up. Never collide with another worker.
    let existing_port = cfg
        .ensemble
        .models
        .iter()
        .find(|m| m.id == CLASSIFIER_ID)
        .and_then(|m| match &m.hosting {
            HostingMode::Local { process_config } => Some(process_config.port),
            _ => None,
        });
    let port = match port {
        Some(p) => p,
        None => match existing_port {
            Some(p) if !used_ports.contains(&p) => p,
            _ => {
                let mut p = 8082u16;
                while used_ports.contains(&p) {
                    p += 1;
                }
                p
            }
        },
    };

    let mut model = ModelConfig::llama_classifier(path.clone(), port);
    // Inert unless the file exists — registered-but-disabled is a safe no-op.
    model.enabled = file_present;
    if let HostingMode::Local { process_config } = &mut model.hosting {
        process_config.auto_start = file_present;
    }
    let model_id = model.id.clone();

    // Upsert by id (idempotent across re-runs).
    if let Some(existing) = cfg.ensemble.models.iter_mut().find(|m| m.id == model_id) {
        *existing = model;
    } else {
        cfg.ensemble.models.push(model);
    }
    cfg.ensemble.classifier_model_id = Some(model_id.clone());

    save_config(&cfg).map_err(|e| format!("config save error: {e}"))?;
    // Keep the in-memory cache in lock-step with disk so `config_get` returns
    // the classifier and no later flush clobbers it.
    cached.set(cfg);

    Ok(json!({
        "registered": true,
        "model_id": model_id,
        "port": port,
        "model_path": path.to_string_lossy(),
        "file_present": file_present,
        "active": file_present,
        "message": if file_present {
            "Classifier registered and set to auto-start. Other-bucket prompts now refine via the local model."
        } else {
            "Classifier registered but DISABLED — GGUF not found at that path. Drop the file there and re-run (or enable it in Settings) to activate."
        },
    }))
}

/// Report the local `gemini` CLI install + OAuth login state.
///
/// Backs the Settings → Gemini model wizard: it shows "signed in as X" when the
/// CLI already holds OAuth credentials, or a sign-in prompt otherwise. Pure
/// filesystem + PATH inspection (no network, no process spawn) — the detection
/// logic lives in the `llm_cli` translator to keep external-tool knowledge
/// inside the §9 boundary.
#[tauri::command]
pub fn gemini_auth_status() -> GeminiAuthStatus {
    cli_gemini_auth_status()
}

/// Make the `gemini` CLI usable in headless (`-p`) mode by selecting the
/// "Login with Google" auth method in `~/.gemini/settings.json`. Without a
/// selected auth method, headless `gemini -p` exits 41 even when OAuth
/// credentials are present (interactive mode picks the method; automated mode
/// cannot). Idempotent and non-clobbering — returns the auth method now in
/// effect. Backs the Settings → Gemini "finish setup" action.
#[tauri::command]
pub fn gemini_enable_headless() -> Result<String, String> {
    cli_gemini_enable_headless()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_task_type_exact_tokens() {
        assert_eq!(parse_task_type("quick_query"), Some(TaskType::QuickQuery));
        assert_eq!(parse_task_type("debug"), Some(TaskType::Debug));
        assert_eq!(
            parse_task_type("architecture"),
            Some(TaskType::Architecture)
        );
    }

    #[test]
    fn parse_task_type_tolerates_whitespace_and_case() {
        assert_eq!(parse_task_type("  DEBUG\n"), Some(TaskType::Debug));
        assert_eq!(parse_task_type("Quick_Query"), Some(TaskType::QuickQuery));
    }

    #[test]
    fn parse_task_type_extracts_from_noisy_reply() {
        assert_eq!(
            parse_task_type("route: \"code_generation\""),
            Some(TaskType::CodeGeneration)
        );
        assert_eq!(
            parse_task_type("The task is documentation."),
            Some(TaskType::Documentation)
        );
    }

    #[test]
    fn parse_task_type_specificity() {
        // `code_refactoring` must not be swallowed by the `code_generation` arm.
        assert_eq!(
            parse_task_type("code_refactoring"),
            Some(TaskType::CodeRefactoring)
        );
        assert_eq!(
            parse_task_type("large_context_analysis"),
            Some(TaskType::LargeContextAnalysis)
        );
    }

    #[test]
    fn parse_task_type_unknown_is_none() {
        assert_eq!(parse_task_type("banana"), None);
        assert_eq!(parse_task_type(""), None);
    }

    #[test]
    fn llama_classifier_factory_shape() {
        let m = ModelConfig::llama_classifier(std::path::PathBuf::from("C:/Models/x.gguf"), 8082);
        assert_eq!(m.id, "llama-classifier");
        assert_eq!(m.provider, ProviderType::LlamaServer);
        assert_eq!(m.endpoint, "http://127.0.0.1:8082");
        assert_eq!(m.capabilities.cost_per_1m_input, 0.0);
        match m.hosting {
            HostingMode::Local { process_config } => {
                assert_eq!(process_config.port, 8082);
                assert_eq!(process_config.n_gpu_layers, 99);
                assert_eq!(process_config.ctx_size, 2048);
            }
            _ => panic!("expected Local hosting"),
        }
    }
}
