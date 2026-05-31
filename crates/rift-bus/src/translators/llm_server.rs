//! llama-server translator — OpenAI-compatible HTTP client.
//!
//! Works for both local llama-server (managed by Rift) and remote
//! llama-server endpoints over the LAN. Uses the `/v1/chat/completions`
//! endpoint with SSE streaming.
//!
//! Lives inside the §9 translator boundary — `reqwest::` calls are
//! permitted here.

use std::time::Instant;

use reqwest::Client;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::llm::Message;
use super::llm::{
    CompletionRequest, CompletionResponse, CompletionStream, LlmError, LlmProvider, ProviderStatus,
    Role, StopReason, StreamChunk,
};

/// A provider backed by a llama-server (or any OpenAI-compatible) endpoint.
pub struct LlamaServerProvider {
    client: Client,
    endpoint: String,
    model_identifier: String,
    provider_tag: String,
}

impl LlamaServerProvider {
    pub fn new(endpoint: impl Into<String>, model_identifier: impl Into<String>) -> Self {
        Self {
            // Fall back to a default client rather than panicking the calling
            // (async) task if the builder fails — async-task panics are not
            // caught by the IPC guarded_invoke_handler.
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| Client::new()),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            model_identifier: model_identifier.into(),
            provider_tag: "llama_server".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    stream: bool,
    /// GBNF grammar (llama.cpp extension) constraining generation. Sourced from
    /// `CompletionRequest.provider_options["grammar"]`. Omitted for other
    /// providers / unconstrained requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    grammar: Option<String>,
    /// llama.cpp chat-template kwargs (e.g. `{"enable_thinking": false}` to
    /// disable the reasoning channel on thinking models like gemma/gpt-oss).
    /// Sourced from `CompletionRequest.provider_options["chat_template_kwargs"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: Option<ChatChoiceMessage>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct ChatChunk {
    choices: Vec<ChunkChoice>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    delta: Option<ChunkDelta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChunkDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct HealthResponse {
    status: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn map_role(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn map_finish_reason(reason: &Option<String>) -> StopReason {
    match reason.as_deref() {
        Some("stop") | Some("end_turn") => StopReason::EndTurn,
        Some("length") | Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

fn build_chat_request(req: &CompletionRequest, model: &str, stream: bool) -> ChatRequest {
    let mut messages: Vec<ChatMessage> = Vec::new();

    if let Some(sys) = &req.system_prompt {
        messages.push(ChatMessage {
            role: "system",
            content: sys.clone(),
        });
    }

    for m in &req.messages {
        messages.push(ChatMessage {
            role: map_role(&m.role),
            content: m.content.clone(),
        });
    }

    // Pass through a GBNF grammar if the caller supplied one in provider_options.
    let grammar = req
        .provider_options
        .as_ref()
        .and_then(|opts| opts.get("grammar"))
        .and_then(|g| g.as_str())
        .map(str::to_string);

    let chat_template_kwargs = req
        .provider_options
        .as_ref()
        .and_then(|opts| opts.get("chat_template_kwargs"))
        .cloned();

    ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stop: req.stop_sequences.clone(),
        stream,
        grammar,
        chat_template_kwargs,
    }
}

fn map_reqwest_error(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::NetworkError {
            message: "request timed out".to_string(),
        }
    } else if e.is_connect() {
        LlmError::NetworkError {
            message: format!("connection failed: {e}"),
        }
    } else if let Some(status) = e.status() {
        match status.as_u16() {
            401 | 403 => LlmError::AuthFailed {
                provider: "llama_server".to_string(),
            },
            429 => LlmError::RateLimited { retry_after: None },
            503 => LlmError::Overloaded,
            _ => LlmError::Internal {
                message: format!("HTTP {status}: {e}"),
            },
        }
    } else {
        LlmError::NetworkError {
            message: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// SSE parsing (channel-based — no extra deps beyond tokio + reqwest)
// ---------------------------------------------------------------------------

fn parse_sse_line(line: &str, tokens_so_far: &mut u32) -> Option<StreamChunk> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') || !line.starts_with("data: ") {
        return None;
    }

    let data = &line["data: ".len()..];
    if data == "[DONE]" {
        return Some(StreamChunk {
            text: String::new(),
            is_final: true,
            token_count: Some(*tokens_so_far),
            stop_reason: Some(StopReason::EndTurn),
        });
    }

    let chunk: ChatChunk = match serde_json::from_str(data) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(data, "SSE parse error: {e}");
            return None;
        }
    };

    let choice = chunk.choices.first()?;
    let text = choice
        .delta
        .as_ref()
        .and_then(|d| d.content.clone())
        .unwrap_or_default();

    if text.is_empty() && choice.finish_reason.is_none() {
        return None;
    }

    let is_final = choice.finish_reason.is_some();
    if !text.is_empty() {
        *tokens_so_far += 1;
    }

    let stop_reason = if is_final {
        Some(map_finish_reason(&choice.finish_reason))
    } else {
        None
    };

    Some(StreamChunk {
        text,
        is_final,
        token_count: Some(*tokens_so_far),
        stop_reason,
    })
}

/// Spawn a task that reads SSE chunks from a reqwest response and sends
/// parsed `StreamChunk`s through an mpsc channel. Returns the receiver
/// wrapped as a `CompletionStream`.
fn spawn_sse_reader(mut resp: reqwest::Response) -> CompletionStream {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk, LlmError>>(32);

    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut tokens_so_far: u32 = 0;

        while let Ok(Some(bytes)) = resp.chunk().await {
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);

            while let Some(newline_pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=newline_pos).collect();
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

        // Process any remaining data in the buffer
        if !buffer.is_empty() {
            if let Some(chunk) = parse_sse_line(&buffer, &mut tokens_so_far) {
                let _ = tx.send(Ok(chunk)).await;
            }
        }
    });

    Box::pin(ReceiverStream(rx))
}

/// Minimal Stream wrapper around `tokio::sync::mpsc::Receiver`.
/// Avoids pulling in `tokio-stream` for a single adapter.
pub struct ReceiverStream<T>(pub tokio::sync::mpsc::Receiver<T>);

impl<T> futures_core::Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<T>> {
        self.0.poll_recv(cx)
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmProvider for LlamaServerProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let url = format!("{}/v1/chat/completions", self.endpoint);
        let body = build_chat_request(&request, &self.model_identifier, false);
        let start = Instant::now();

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                401 | 403 => LlmError::AuthFailed {
                    provider: self.provider_tag.clone(),
                },
                429 => LlmError::RateLimited { retry_after: None },
                503 => LlmError::Overloaded,
                _ => LlmError::Internal {
                    message: format!("HTTP {status}: {body_text}"),
                },
            });
        }

        let latency_ms = start.elapsed().as_millis() as u64;
        let chat_resp: ChatResponse = resp.json().await.map_err(|e| LlmError::Internal {
            message: format!("response parse error: {e}"),
        })?;

        let choice = chat_resp
            .choices
            .first()
            .ok_or_else(|| LlmError::Internal {
                message: "empty choices array".to_string(),
            })?;

        let content = choice
            .message
            .as_ref()
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let usage = chat_resp.usage.as_ref();

        Ok(CompletionResponse {
            content,
            tokens_in: usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
            tokens_out: usage.and_then(|u| u.completion_tokens).unwrap_or(0),
            model_used: chat_resp
                .model
                .unwrap_or_else(|| self.model_identifier.clone()),
            stop_reason: map_finish_reason(&choice.finish_reason),
            latency_ms,
        })
    }

    async fn stream(&self, request: CompletionRequest) -> Result<CompletionStream, LlmError> {
        let url = format!("{}/v1/chat/completions", self.endpoint);
        let body = build_chat_request(&request, &self.model_identifier, true);

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                401 | 403 => LlmError::AuthFailed {
                    provider: self.provider_tag.clone(),
                },
                429 => LlmError::RateLimited { retry_after: None },
                503 => LlmError::Overloaded,
                _ => LlmError::Internal {
                    message: format!("HTTP {status}: {body_text}"),
                },
            });
        }

        Ok(spawn_sse_reader(resp))
    }

    async fn health_check(&self) -> ProviderStatus {
        let url = format!("{}/health", self.endpoint);
        let start = Instant::now();

        match self.client.get(&url).send().await {
            Err(_) => ProviderStatus::Offline,
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                if !resp.status().is_success() {
                    return ProviderStatus::Error {
                        message: format!("HTTP {}", resp.status()),
                        retryable: true,
                    };
                }
                match resp.json::<HealthResponse>().await {
                    Ok(h) => match h.status.as_deref() {
                        Some("ok") | Some("ready") | None => ProviderStatus::Ready { latency_ms },
                        Some("loading model") | Some("loading") => {
                            ProviderStatus::Loading { progress: None }
                        }
                        Some(other) => ProviderStatus::Error {
                            message: other.to_string(),
                            retryable: true,
                        },
                    },
                    Err(_) => ProviderStatus::Ready { latency_ms },
                }
            }
        }
    }

    fn provider_id(&self) -> &str {
        &self.provider_tag
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

    #[test]
    fn build_chat_request_maps_roles() {
        let req = CompletionRequest {
            messages: vec![
                Message {
                    role: Role::User,
                    content: "Hello".to_string(),
                },
                Message {
                    role: Role::Assistant,
                    content: "Hi".to_string(),
                },
            ],
            max_tokens: Some(100),
            temperature: Some(0.5),
            stop_sequences: vec![],
            system_prompt: Some("Be helpful".to_string()),
            provider_options: None,
        };

        let chat = build_chat_request(&req, "test-model", false);
        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[0].role, "system");
        assert_eq!(chat.messages[0].content, "Be helpful");
        assert_eq!(chat.messages[1].role, "user");
        assert_eq!(chat.messages[2].role, "assistant");
        assert!(!chat.stream);
        assert_eq!(chat.model, "test-model");
    }

    #[test]
    fn build_chat_request_no_system() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "Test".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };

        let chat = build_chat_request(&req, "model", true);
        assert_eq!(chat.messages.len(), 1);
        assert!(chat.stream);
        assert!(chat.grammar.is_none());
    }

    #[test]
    fn build_chat_request_forwards_grammar() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "x".to_string(),
            }],
            max_tokens: Some(8),
            temperature: Some(0.0),
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: Some(serde_json::json!({ "grammar": "root ::= \"other\"" })),
        };
        let chat = build_chat_request(&req, "m", false);
        assert_eq!(chat.grammar.as_deref(), Some("root ::= \"other\""));
        // And it must actually serialize into the wire body.
        let body = serde_json::to_value(&chat).unwrap();
        assert_eq!(body["grammar"], "root ::= \"other\"");
    }

    #[test]
    fn map_finish_reason_variants() {
        assert_eq!(
            map_finish_reason(&Some("stop".to_string())),
            StopReason::EndTurn
        );
        assert_eq!(
            map_finish_reason(&Some("length".to_string())),
            StopReason::MaxTokens
        );
        assert_eq!(
            map_finish_reason(&Some("stop_sequence".to_string())),
            StopReason::StopSequence
        );
        assert_eq!(map_finish_reason(&None), StopReason::EndTurn);
    }

    #[test]
    fn provider_id_and_model_id() {
        let p = LlamaServerProvider::new("http://localhost:8081", "gemma-27b");
        assert_eq!(p.provider_id(), "llama_server");
        assert_eq!(p.model_id(), "gemma-27b");
    }

    #[test]
    fn endpoint_trailing_slash_stripped() {
        let p = LlamaServerProvider::new("http://localhost:8081/", "model");
        assert_eq!(p.endpoint, "http://localhost:8081");
    }

    #[test]
    fn chat_response_parses() {
        let json = r#"{
            "id": "test",
            "object": "chat.completion",
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2}
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content.as_deref(),
            Some("Hello!")
        );
        assert_eq!(resp.usage.as_ref().unwrap().prompt_tokens, Some(5));
    }

    #[test]
    fn chat_chunk_parses() {
        let json = r#"{
            "id": "test",
            "object": "chat.completion.chunk",
            "choices": [{
                "delta": {"content": "He"},
                "finish_reason": null
            }]
        }"#;
        let chunk: ChatChunk = serde_json::from_str(json).unwrap();
        assert_eq!(
            chunk.choices[0].delta.as_ref().unwrap().content.as_deref(),
            Some("He")
        );
        assert!(chunk.choices[0].finish_reason.is_none());
    }

    #[test]
    fn health_response_parses() {
        let json = r#"{"status": "ok"}"#;
        let h: HealthResponse = serde_json::from_str(json).unwrap();
        assert_eq!(h.status.as_deref(), Some("ok"));
    }
}
