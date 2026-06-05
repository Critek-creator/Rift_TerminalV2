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
    Role, StopReason, StreamChunk, ToolCall,
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
    /// JSON Schema (llama.cpp top-level `json_schema` extension) constraining
    /// generation to a schema-valid JSON value. The server compiles it to an
    /// internal constraint that — unlike a hand-written GBNF with negated char
    /// classes (`[^...]`, which this build silently drops) — reliably constrains
    /// strings. Sourced from `CompletionRequest.provider_options["json_schema"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    json_schema: Option<serde_json::Value>,
    /// llama.cpp chat-template kwargs (e.g. `{"enable_thinking": false}` to
    /// disable the reasoning channel on thinking models like gemma/gpt-oss).
    /// Sourced from `CompletionRequest.provider_options["chat_template_kwargs"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
    /// OpenAI `tools` array. llama-server only honors this when launched with
    /// `--jinja` (the server then renders the schema into the chat template
    /// and auto-applies a lazy GBNF grammar that guarantees syntactically
    /// valid tool-call JSON). Sourced from `provider_options["tools"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<serde_json::Value>,
    /// OpenAI `tool_choice` (`"auto"` | `"required"` | `"none"` | object).
    /// Sourced from `provider_options["tool_choice"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    /// Request token-level log-probabilities from llama-server
    /// (`/v1/chat/completions` OpenAI extension). Always set to `true` for
    /// non-streaming completions so the confidence-gated escalation path has
    /// a signal. Has no effect on providers that ignore unknown fields.
    logprobs: bool,
    /// Number of top alternative log-prob candidates per token. Set to 1
    /// (we only need the chosen-token prob) to minimise response size.
    top_logprobs: u32,
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
    /// Present when `logprobs:true` was sent in the request.
    #[serde(default)]
    logprobs: Option<ChatLogprobs>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
}

/// One OpenAI `tool_calls[]` entry as returned by llama-server (`--jinja`).
#[derive(Deserialize)]
struct ChatToolCall {
    #[serde(default)]
    id: Option<String>,
    function: ChatToolCallFunction,
}

#[derive(Deserialize)]
struct ChatToolCallFunction {
    name: String,
    /// Per the OpenAI spec this is a JSON-encoded *string*; some servers emit
    /// a bare object instead. [`ChatToolCall::to_tool_call`] handles both.
    arguments: serde_json::Value,
}

impl ChatToolCall {
    /// Normalize a wire tool-call into the shared [`ToolCall`] shape, parsing
    /// the OpenAI string-encoded `arguments` into a JSON object. `idx` provides
    /// a synthetic id when the server omits one.
    fn to_tool_call(&self, idx: usize) -> ToolCall {
        let arguments = match &self.function.arguments {
            serde_json::Value::String(s) => {
                serde_json::from_str(s).unwrap_or_else(|_| serde_json::Value::String(s.clone()))
            }
            other => other.clone(),
        };
        ToolCall {
            id: self.id.clone().unwrap_or_else(|| format!("call_{idx}")),
            name: self.function.name.clone(),
            arguments,
        }
    }
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Logprob response types (OpenAI extension, returned by llama-server)
// ---------------------------------------------------------------------------

/// Top-level logprob container on a `ChatChoice` (present when `logprobs:true`).
#[derive(Deserialize)]
struct ChatLogprobs {
    #[serde(default)]
    content: Vec<TokenLogprob>,
}

/// Per-token logprob entry.
#[derive(Deserialize)]
struct TokenLogprob {
    logprob: f32,
}

// ---------------------------------------------------------------------------
// Confidence computation
// ---------------------------------------------------------------------------

/// Reduce a slice of per-token logprobs into:
/// - `mean_logprob`: arithmetic mean of the raw logprob values (≤ 0)
/// - `confidence`: mean per-token probability (exp of mean_logprob), in 0..1
///
/// Returns `(None, None)` when `logprobs` is empty (e.g. empty completion or
/// provider did not return logprob data), keeping `None` as the safe sentinel
/// that disables confidence-gated escalation.
fn compute_confidence(logprobs: &[TokenLogprob]) -> (Option<f32>, Option<f32>) {
    if logprobs.is_empty() {
        return (None, None);
    }
    let mean_lp = logprobs.iter().map(|t| t.logprob).sum::<f32>() / logprobs.len() as f32;
    // exp() of the mean logprob gives the geometric-mean probability — a
    // human-readable 0..1 scalar that collapses the per-token distribution
    // into a single confidence signal suitable for threshold comparisons.
    let confidence = mean_lp.exp().clamp(0.0, 1.0);
    (Some(confidence), Some(mean_lp))
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
        Some("tool_calls") => StopReason::ToolUse,
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

    // Pass through a JSON Schema if supplied — the robust path for structured
    // output (the server's schema→constraint handles strings, where raw GBNF
    // negated classes are dropped on this build).
    let json_schema = req
        .provider_options
        .as_ref()
        .and_then(|opts| opts.get("json_schema"))
        .cloned();

    let chat_template_kwargs = req
        .provider_options
        .as_ref()
        .and_then(|opts| opts.get("chat_template_kwargs"))
        .cloned();

    // Tool calling (passed through provider_options — a single-provider
    // capability today, so it stays out of the shared CompletionRequest).
    let tools = req
        .provider_options
        .as_ref()
        .and_then(|opts| opts.get("tools"))
        .cloned();

    let tool_choice = req
        .provider_options
        .as_ref()
        .and_then(|opts| opts.get("tool_choice"))
        .cloned();

    // Request token logprobs only for non-streaming completions. Streaming
    // does not carry per-token logprob data in the chunk wire format and the
    // confidence signal is unused there (no escalation path on streams).
    // `top_logprobs: 1` — we only need the chosen-token probability.
    let (logprobs, top_logprobs) = if stream { (false, 0) } else { (true, 1) };

    ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stop: req.stop_sequences.clone(),
        stream,
        grammar,
        json_schema,
        chat_template_kwargs,
        tools,
        tool_choice,
        logprobs,
        top_logprobs,
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

        let message = choice.message.as_ref();
        let content = message.and_then(|m| m.content.clone()).unwrap_or_default();

        // Normalize any tool calls the model emitted. `None` (not empty Vec)
        // when the model produced a plain text turn.
        let tool_calls: Option<Vec<ToolCall>> = message
            .and_then(|m| m.tool_calls.as_ref())
            .map(|calls| {
                calls
                    .iter()
                    .enumerate()
                    .map(|(i, c)| c.to_tool_call(i))
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());

        // Some models/servers report finish_reason="stop" even while emitting
        // tool calls (feasibility note on Qwen3/Gemma parsers). Treat any
        // present tool calls as ToolUse so the executor loop detects them.
        let stop_reason = if tool_calls.is_some() {
            StopReason::ToolUse
        } else {
            map_finish_reason(&choice.finish_reason)
        };

        let usage = chat_resp.usage.as_ref();
        let tokens_in = usage.and_then(|u| u.prompt_tokens).unwrap_or(0);
        let tokens_out = usage.and_then(|u| u.completion_tokens).unwrap_or(0);

        // Extract per-token logprobs from the first choice (present only when
        // `logprobs:true` was honoured by the server). Collapse to confidence
        // scalars; missing/empty → None (feature disabled at default).
        let (confidence, mean_logprob) = choice
            .logprobs
            .as_ref()
            .map(|lp| compute_confidence(&lp.content))
            .unwrap_or((None, None));

        Ok(CompletionResponse {
            content,
            tokens_in,
            tokens_out,
            model_used: chat_resp
                .model
                .unwrap_or_else(|| self.model_identifier.clone()),
            stop_reason,
            latency_ms,
            tool_calls,
            confidence,
            mean_logprob,
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
    fn build_chat_request_forwards_json_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"]
        });
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "x".to_string(),
            }],
            max_tokens: Some(8),
            temperature: Some(0.0),
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: Some(serde_json::json!({ "json_schema": schema })),
        };
        let chat = build_chat_request(&req, "m", false);
        assert!(chat.json_schema.is_some());
        // Must serialize into the wire body as the llama-server `json_schema` field.
        let body = serde_json::to_value(&chat).unwrap();
        assert_eq!(body["json_schema"]["type"], "object");
        assert_eq!(body["json_schema"]["required"][0], "name");
        // Absent when not supplied (skip_serializing_if).
        let req2 = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "x".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };
        let body2 = serde_json::to_value(build_chat_request(&req2, "m", false)).unwrap();
        assert!(body2.get("json_schema").is_none());
    }

    #[test]
    fn build_chat_request_forwards_tools() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "list files".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: Some(serde_json::json!({
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "fs_read",
                        "description": "read a file",
                        "parameters": { "type": "object" }
                    }
                }],
                "tool_choice": "auto"
            })),
        };
        let chat = build_chat_request(&req, "m", false);
        let body = serde_json::to_value(&chat).unwrap();
        assert_eq!(body["tools"][0]["function"]["name"], "fs_read");
        assert_eq!(body["tool_choice"], "auto");
        // Absent when not requested — wire body stays clean for plain calls.
        let plain = build_chat_request(
            &CompletionRequest {
                messages: vec![Message {
                    role: Role::User,
                    content: "hi".to_string(),
                }],
                max_tokens: None,
                temperature: None,
                stop_sequences: vec![],
                system_prompt: None,
                provider_options: None,
            },
            "m",
            false,
        );
        let plain_body = serde_json::to_value(&plain).unwrap();
        assert!(plain_body.get("tools").is_none());
        assert!(plain_body.get("tool_choice").is_none());
    }

    #[test]
    fn chat_response_with_tool_calls_parses() {
        // OpenAI/llama-server shape: arguments is a JSON-encoded STRING.
        let json = r#"{
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_0",
                        "type": "function",
                        "function": { "name": "fs_read", "arguments": "{\"path\":\"a.rs\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "model": "local"
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).expect("parse");
        let choice = resp.choices.first().expect("choice");
        let calls = choice
            .message
            .as_ref()
            .and_then(|m| m.tool_calls.as_ref())
            .expect("tool_calls");
        assert_eq!(calls.len(), 1);
        let tc = calls[0].to_tool_call(0);
        assert_eq!(tc.name, "fs_read");
        assert_eq!(tc.id, "call_0");
        // The string-encoded arguments were parsed into a real object.
        assert_eq!(tc.arguments["path"], "a.rs");
        assert_eq!(
            map_finish_reason(&choice.finish_reason),
            StopReason::ToolUse
        );
    }

    #[test]
    fn tool_call_arguments_object_form_also_parses() {
        // Some servers emit arguments as a bare object instead of a string.
        let call: ChatToolCall = serde_json::from_str(
            r#"{ "id": "c1", "function": { "name": "git_status", "arguments": { "x": 1 } } }"#,
        )
        .expect("parse");
        let tc = call.to_tool_call(2);
        assert_eq!(tc.arguments["x"], 1);
        assert_eq!(tc.id, "c1");
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
        assert_eq!(
            map_finish_reason(&Some("tool_calls".to_string())),
            StopReason::ToolUse
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

    /// Canned llama-server response with known logprobs: verify computed confidence.
    ///
    /// Logprobs: [-0.5, -1.0, -0.5]
    /// mean_logprob = (-0.5 + -1.0 + -0.5) / 3 = -0.6667
    /// confidence = exp(-0.6667) ≈ 0.5134
    #[test]
    fn logprob_confidence_computed_from_canned_response() {
        let json = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop",
                "logprobs": {
                    "content": [
                        {"token": "ok", "logprob": -0.5, "bytes": null, "top_logprobs": []},
                        {"token": " is", "logprob": -1.0, "bytes": null, "top_logprobs": []},
                        {"token": " fine", "logprob": -0.5, "bytes": null, "top_logprobs": []}
                    ]
                }
            }],
            "model": "granite-4.1-8b",
            "usage": {"prompt_tokens": 10, "completion_tokens": 3}
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).expect("parse");
        let choice = resp.choices.first().expect("choice");
        let lp = choice.logprobs.as_ref().expect("logprobs");
        let (confidence, mean_logprob) = compute_confidence(&lp.content);
        let conf = confidence.expect("confidence Some");
        let mlp = mean_logprob.expect("mean_logprob Some");
        // mean logprob ≈ -0.6667
        assert!((mlp - (-2.0 / 3.0)).abs() < 0.001, "mean_logprob={mlp}");
        // confidence = exp(mean_logprob) ≈ 0.5134
        assert!((conf - 0.5134_f32).abs() < 0.001, "confidence={conf}");
    }

    #[test]
    fn logprob_confidence_empty_returns_none() {
        let (confidence, mean_logprob) = compute_confidence(&[]);
        assert!(confidence.is_none());
        assert!(mean_logprob.is_none());
    }

    #[test]
    fn chat_request_non_streaming_sets_logprobs_true() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };
        let chat = build_chat_request(&req, "m", false);
        assert!(
            chat.logprobs,
            "non-streaming request must set logprobs=true"
        );
        assert_eq!(chat.top_logprobs, 1);
        // Verify it actually serializes into the wire body.
        let body = serde_json::to_value(&chat).unwrap();
        assert_eq!(body["logprobs"], true);
        assert_eq!(body["top_logprobs"], 1);
    }

    #[test]
    fn chat_request_streaming_does_not_set_logprobs() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        };
        let chat = build_chat_request(&req, "m", true);
        assert!(
            !chat.logprobs,
            "streaming request must not set logprobs=true"
        );
        assert_eq!(chat.top_logprobs, 0);
    }

    #[test]
    fn chat_response_without_logprobs_gives_none_confidence() {
        // A response from a server that does not return logprobs must not
        // cause a panic or error — confidence fields remain None.
        let json = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2}
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).expect("parse");
        let choice = resp.choices.first().expect("choice");
        let (confidence, mean_logprob) = choice
            .logprobs
            .as_ref()
            .map(|lp| compute_confidence(&lp.content))
            .unwrap_or((None, None));
        assert!(confidence.is_none());
        assert!(mean_logprob.is_none());
    }
}
