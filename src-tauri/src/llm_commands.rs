//! Tauri IPC commands for the Ensemble Router.
//!
//! Phase 2: RouterService sits in the prompt path. @model tags are parsed,
//! the classifier + profile engine select the model, bus events fire on
//! every routing decision + response, and retryable failures escalate
//! through the fallback chain.

use futures_util::StreamExt;
use rift_bus::config::{load_config, HostingMode, ProviderType};
use rift_bus::translators::llm::{CompletionRequest, LlmProvider, Message, Role};
use rift_bus::translators::llm_anthropic::AnthropicProvider;
use rift_bus::translators::llm_gemini::GeminiProvider;
use rift_bus::translators::llm_server::LlamaServerProvider;
use rift_bus::{Category, Envelope, RiftBus};
use rift_router::{RouterService, RoutingDecision};
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
