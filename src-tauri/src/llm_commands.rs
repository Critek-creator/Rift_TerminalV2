//! Tauri IPC commands for the Ensemble Router.
//!
//! Frontend calls these via `invoke('llm_complete', ...)` etc.
//! Commands look up the model config, create the appropriate provider,
//! and execute the request.

use rift_bus::config::{load_config, HostingMode};
use rift_bus::translators::llm::{CompletionRequest, LlmProvider, Message, Role};
use rift_bus::translators::llm_server::LlamaServerProvider;
use serde::Serialize;

#[derive(Serialize)]
pub struct LlmCompleteResult {
    pub content: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub model_used: String,
    pub latency_ms: u64,
}

#[tauri::command]
pub async fn llm_complete(model_id: String, prompt: String) -> Result<LlmCompleteResult, String> {
    let config = load_config().map_err(|e| format!("config load error: {e}"))?;

    let model = config
        .ensemble
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("model not found: {model_id}"))?
        .clone();

    let provider: Box<dyn LlmProvider> = match &model.hosting {
        HostingMode::Cloud => {
            return Err(
                "Cloud providers not yet implemented — use a local or remote llama-server model"
                    .into(),
            );
        }
        HostingMode::Local { process_config } => {
            let endpoint = format!("http://127.0.0.1:{}", process_config.port);
            Box::new(LlamaServerProvider::new(endpoint, &model.model_identifier))
        }
        HostingMode::Remote { .. } => Box::new(LlamaServerProvider::new(
            &model.endpoint,
            &model.model_identifier,
        )),
    };

    let request = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: prompt,
        }],
        max_tokens: None,
        temperature: None,
        stop_sequences: vec![],
        system_prompt: None,
        provider_options: None,
    };

    let resp = provider
        .complete(request)
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(LlmCompleteResult {
        content: resp.content,
        tokens_in: resp.tokens_in,
        tokens_out: resp.tokens_out,
        model_used: resp.model_used,
        latency_ms: resp.latency_ms,
    })
}
