//! Anthropic API translator — direct HTTP client for Claude models.
//!
//! Uses the Messages API (`/v1/messages`) with SSE streaming.
//! Auth via `x-api-key` header. The API key is passed at construction
//! time (resolved from keyring by the caller).
//!
//! Lives inside the §9 translator boundary.

use std::time::Instant;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::llm::{
    CompletionRequest, CompletionResponse, CompletionStream, LlmError, LlmProvider, ProviderStatus,
    Role, StopReason, StreamChunk,
};

const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicProvider {
    client: Client,
    endpoint: String,
    api_key: String,
    model_identifier: String,
}

impl AnthropicProvider {
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model_identifier: impl Into<String>,
    ) -> Self {
        Self {
            // Fall back to a default client rather than panicking the calling
            // (async) task if the builder fails — async-task panics are not
            // caught by the IPC guarded_invoke_handler.
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| Client::new()),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model_identifier: model_identifier.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Anthropic wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop_sequences: Vec<String>,
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<StreamDelta>,
    message: Option<StreamMessageStart>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamMessageStart {
    model: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicError {
    error: Option<AnthropicErrorBody>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AnthropicErrorBody {
    message: Option<String>,
    #[serde(rename = "type")]
    error_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn map_role(role: &Role) -> &'static str {
    match role {
        Role::System => "user",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn map_stop_reason(reason: &Option<String>) -> StopReason {
    match reason.as_deref() {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

fn build_request(req: &CompletionRequest, model: &str, stream: bool) -> MessagesRequest {
    let messages: Vec<AnthropicMessage> = req
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| AnthropicMessage {
            role: map_role(&m.role),
            content: m.content.clone(),
        })
        .collect();

    // Prefer an explicit system_prompt. Otherwise hoist every System message
    // into Anthropic's top-level `system` field — concatenating them, since
    // `build_request` filters all System messages out of `messages` and
    // taking only the first would silently drop the rest.
    let system = req.system_prompt.clone().or_else(|| {
        let parts: Vec<&str> = req
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .filter(|c| !c.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    });

    MessagesRequest {
        model: model.to_string(),
        messages,
        max_tokens: req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        system,
        temperature: req.temperature,
        stop_sequences: req.stop_sequences.clone(),
        stream,
    }
}

async fn handle_error_response(resp: reqwest::Response) -> LlmError {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    let msg = serde_json::from_str::<AnthropicError>(&body)
        .ok()
        .and_then(|e| e.error)
        .and_then(|e| e.message)
        .unwrap_or_else(|| body.chars().take(200).collect());

    match status.as_u16() {
        401 => LlmError::AuthFailed {
            provider: "anthropic".to_string(),
        },
        429 => LlmError::RateLimited { retry_after: None },
        529 | 503 => LlmError::Overloaded,
        _ => LlmError::Internal {
            message: format!("HTTP {status}: {msg}"),
        },
    }
}

fn parse_sse_line(line: &str, tokens_so_far: &mut u32) -> Option<StreamChunk> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') || !line.starts_with("data: ") {
        return None;
    }
    let data = &line["data: ".len()..];

    let event: StreamEvent = serde_json::from_str(data).ok()?;

    match event.event_type.as_str() {
        "content_block_delta" => {
            let text = event.delta.as_ref()?.text.clone().unwrap_or_default();
            if text.is_empty() {
                return None;
            }
            *tokens_so_far += 1;
            Some(StreamChunk {
                text,
                is_final: false,
                token_count: Some(*tokens_so_far),
                stop_reason: None,
            })
        }
        "message_delta" => {
            let stop = event.delta.as_ref().and_then(|d| d.stop_reason.clone());
            // Anthropic reports the authoritative output token count on the
            // message_delta event. Prefer it over our per-delta tally, which
            // counts deltas (each may carry multiple tokens), not tokens.
            if let Some(output) = event.usage.as_ref().and_then(|u| u.output_tokens) {
                *tokens_so_far = output as u32;
            }
            Some(StreamChunk {
                text: String::new(),
                is_final: true,
                token_count: Some(*tokens_so_far),
                stop_reason: Some(map_stop_reason(&stop)),
            })
        }
        "message_stop" => Some(StreamChunk {
            text: String::new(),
            is_final: true,
            token_count: Some(*tokens_so_far),
            stop_reason: Some(StopReason::EndTurn),
        }),
        _ => None,
    }
}

fn spawn_sse_reader(mut resp: reqwest::Response) -> CompletionStream {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk, LlmError>>(32);

    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut tokens_so_far: u32 = 0;

        while let Ok(Some(bytes)) = resp.chunk().await {
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=pos).collect();
                if let Some(chunk) = parse_sse_line(&line, &mut tokens_so_far) {
                    let is_done = chunk.is_final;
                    if tx.send(Ok(chunk)).await.is_err() {
                        return;
                    }
                    if is_done {
                        return;
                    }
                }
            }
        }
    });

    Box::pin(super::llm_server::ReceiverStream(rx))
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let url = format!("{}/v1/messages", self.endpoint);
        let body = build_request(&request, &self.model_identifier, false);
        let start = Instant::now();

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }

        let latency_ms = start.elapsed().as_millis() as u64;
        let msg_resp: MessagesResponse = resp.json().await.map_err(|e| LlmError::Internal {
            message: format!("response parse error: {e}"),
        })?;

        let content = msg_resp
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        let usage = msg_resp.usage.as_ref();

        Ok(CompletionResponse {
            content,
            tokens_in: usage.and_then(|u| u.input_tokens).unwrap_or(0),
            tokens_out: usage.and_then(|u| u.output_tokens).unwrap_or(0),
            model_used: msg_resp
                .model
                .unwrap_or_else(|| self.model_identifier.clone()),
            stop_reason: map_stop_reason(&msg_resp.stop_reason),
            latency_ms,
            tool_calls: None,
            confidence: None,
            mean_logprob: None,
        })
    }

    async fn stream(&self, request: CompletionRequest) -> Result<CompletionStream, LlmError> {
        let url = format!("{}/v1/messages", self.endpoint);
        let body = build_request(&request, &self.model_identifier, true);

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }

        Ok(spawn_sse_reader(resp))
    }

    async fn health_check(&self) -> ProviderStatus {
        let url = format!("{}/v1/messages", self.endpoint);
        let start = Instant::now();

        match self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-haiku-4-5-20251001","max_tokens":1,"messages":[{"role":"user","content":"ping"}]}"#)
            .send()
            .await
        {
            Err(_) => ProviderStatus::Offline,
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                match resp.status().as_u16() {
                    200 => ProviderStatus::Ready { latency_ms },
                    401 => ProviderStatus::Error {
                        message: "invalid API key".to_string(),
                        retryable: false,
                    },
                    429 => ProviderStatus::RateLimited { retry_after: None },
                    529 | 503 => ProviderStatus::Error {
                        message: "overloaded".to_string(),
                        retryable: true,
                    },
                    s => ProviderStatus::Error {
                        message: format!("HTTP {s}"),
                        retryable: true,
                    },
                }
            }
        }
    }

    fn provider_id(&self) -> &str {
        "anthropic"
    }

    fn model_id(&self) -> &str {
        &self.model_identifier
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translators::llm::Message;

    #[test]
    fn build_request_extracts_system() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            max_tokens: Some(100),
            temperature: None,
            stop_sequences: vec![],
            system_prompt: Some("Be helpful.".to_string()),
            provider_options: None,
        };

        let built = build_request(&req, "claude-sonnet-4-6", false);
        assert_eq!(built.system, Some("Be helpful.".to_string()));
        assert_eq!(built.messages.len(), 1);
        assert_eq!(built.messages[0].role, "user");
        assert!(!built.stream);
    }

    #[test]
    fn build_request_system_from_messages() {
        let req = CompletionRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "System msg".to_string(),
                },
                Message {
                    role: Role::User,
                    content: "Hi".to_string(),
                },
            ],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };

        let built = build_request(&req, "model", false);
        assert_eq!(built.system, Some("System msg".to_string()));
        assert_eq!(built.messages.len(), 1);
        assert_eq!(built.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn build_request_concatenates_multiple_system_messages() {
        let req = CompletionRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "First.".to_string(),
                },
                Message {
                    role: Role::System,
                    content: "Second.".to_string(),
                },
                Message {
                    role: Role::User,
                    content: "Hi".to_string(),
                },
            ],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };

        let built = build_request(&req, "model", false);
        // Both system messages survive; only the user turn stays in messages.
        assert_eq!(built.system, Some("First.\n\nSecond.".to_string()));
        assert_eq!(built.messages.len(), 1);
        assert_eq!(built.messages[0].role, "user");
    }

    #[test]
    fn build_request_system_prompt_takes_precedence() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::System,
                content: "Ignored.".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: Some("Explicit.".to_string()),
            provider_options: None,
        };

        let built = build_request(&req, "model", false);
        assert_eq!(built.system, Some("Explicit.".to_string()));
    }

    #[test]
    fn map_stop_reason_variants() {
        assert_eq!(
            map_stop_reason(&Some("end_turn".to_string())),
            StopReason::EndTurn
        );
        assert_eq!(
            map_stop_reason(&Some("max_tokens".to_string())),
            StopReason::MaxTokens
        );
        assert_eq!(
            map_stop_reason(&Some("stop_sequence".to_string())),
            StopReason::StopSequence
        );
    }

    #[test]
    fn provider_ids() {
        let p = AnthropicProvider::new("https://api.anthropic.com", "sk-test", "claude-sonnet-4-6");
        assert_eq!(p.provider_id(), "anthropic");
        assert_eq!(p.model_id(), "claude-sonnet-4-6");
    }

    #[test]
    fn parse_content_block_delta() {
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let mut tokens = 0;
        let chunk = parse_sse_line(line, &mut tokens).unwrap();
        assert_eq!(chunk.text, "Hello");
        assert!(!chunk.is_final);
        assert_eq!(tokens, 1);
    }

    #[test]
    fn parse_message_delta_stop() {
        let line = r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        let mut tokens = 5;
        let chunk = parse_sse_line(line, &mut tokens).unwrap();
        assert!(chunk.is_final);
        assert_eq!(chunk.stop_reason, Some(StopReason::EndTurn));
        // The authoritative usage.output_tokens overrides the per-delta tally.
        assert_eq!(chunk.token_count, Some(42));
        assert_eq!(tokens, 42);
    }

    #[test]
    fn parse_message_delta_without_usage_keeps_running_tally() {
        let line = r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#;
        let mut tokens = 7;
        let chunk = parse_sse_line(line, &mut tokens).unwrap();
        assert!(chunk.is_final);
        assert_eq!(chunk.token_count, Some(7));
    }

    #[test]
    fn messages_response_parses() {
        let json = r#"{
            "content": [{"type": "text", "text": "Hi there!"}],
            "model": "claude-sonnet-4-6",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 3}
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.content[0].text.as_deref(), Some("Hi there!"));
        assert_eq!(resp.usage.as_ref().unwrap().input_tokens, Some(10));
    }
}
