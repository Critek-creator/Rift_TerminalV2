//! LLM provider abstraction — trait, types, and error hierarchy.
//!
//! This module defines the contract every LLM translator implements.
//! Cloud APIs (Anthropic, Gemini), local llama-server, and remote
//! llama-server all conform to [`LlmProvider`]. The router crate
//! dispatches through `Box<dyn LlmProvider>` — hence `async_trait`
//! for object safety.
//!
//! Concrete translator implementations (`llm_anthropic.rs`, `llm_server.rs`,
//! etc.) live as sibling modules in this `translators/` directory, inside
//! the §9 boundary.

use std::pin::Pin;
use std::time::Duration;

use futures_core::Stream;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Streaming types
// ---------------------------------------------------------------------------

/// A single chunk from a streaming LLM response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamChunk {
    /// The text content of this chunk.
    pub text: String,
    /// Whether this is the final chunk in the stream.
    pub is_final: bool,
    /// Cumulative token count so far (if the provider reports it).
    pub token_count: Option<u32>,
    /// Reason the stream stopped (present only on the final chunk).
    pub stop_reason: Option<StopReason>,
}

/// Why a completion stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model reached a natural stopping point.
    EndTurn,
    /// Hit the configured max_tokens limit.
    MaxTokens,
    /// Hit one of the configured stop sequences.
    StopSequence,
    /// Model emitted one or more tool calls and is awaiting their results.
    /// Local llama-server (`--jinja`) sets `finish_reason = "tool_calls"`.
    ToolUse,
    /// Provider returned an error mid-stream.
    Error,
}

/// Pinned, boxed, Send-safe stream of completion chunks.
///
/// Every provider normalizes its wire format (SSE, chunked JSON, etc.)
/// into this common stream shape. Consumers drive it with
/// `StreamExt::next()` from `futures-util` or `tokio-stream`.
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send>>;

// ---------------------------------------------------------------------------
// Tool calling (local-provider capability; spike scope — see plan Path C)
// ---------------------------------------------------------------------------

/// A tool the model may call. Mirrors the OpenAI `tools[].function` shape.
///
/// For the single-forced-tool-call spike, tool definitions are passed to the
/// llama-server translator via [`CompletionRequest::provider_options`] (the
/// sanctioned provider-extension bag) rather than a first-class request field,
/// because tool calling is currently a single-provider capability. This type
/// is the shared shape the executor builds and the translator consumes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name — the identifier the model emits to call it.
    pub name: String,
    /// Description shown to the model of what the tool does.
    pub description: String,
    /// JSON Schema for the tool's arguments object.
    pub parameters: serde_json::Value,
}

/// A tool call the model emitted. Mirrors one OpenAI `tool_calls[]` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned id, echoed back when the result is returned.
    pub id: String,
    /// Name of the tool to invoke.
    pub name: String,
    /// Parsed arguments object. llama-server's lazy GBNF grammar guarantees
    /// this is syntactically valid JSON matching the tool's schema.
    pub arguments: serde_json::Value,
}

/// How the model should choose among offered tools (OpenAI `tool_choice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoice {
    /// Model decides whether to call a tool.
    Auto,
    /// Model must call a tool this turn.
    Required,
    /// Model must not call a tool.
    None,
}

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

/// Normalized completion request across all providers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Conversation messages (system, user, assistant turns).
    pub messages: Vec<Message>,
    /// Maximum tokens to generate. `None` = provider default.
    pub max_tokens: Option<u32>,
    /// Sampling temperature. `None` = provider default.
    pub temperature: Option<f32>,
    /// Stop sequences. Empty = none.
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    /// System prompt (prepended by the provider if supported).
    pub system_prompt: Option<String>,
    /// Provider-specific extensions (Anthropic thinking blocks, Gemini
    /// grounding config, llama-server grammar, etc.). Opaque JSON passed
    /// through to the provider translator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

/// A single message in a conversation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// Conversation role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Normalized completion response (non-streaming).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The generated text content.
    pub content: String,
    /// Input token count (from provider response or local estimate).
    pub tokens_in: u64,
    /// Output token count.
    pub tokens_out: u64,
    /// The model that actually handled the request (may differ from
    /// the configured model if the provider auto-selects).
    pub model_used: String,
    /// Why generation stopped.
    pub stop_reason: StopReason,
    /// Wall-clock latency in milliseconds (first token for streaming,
    /// full response for non-streaming).
    pub latency_ms: u64,
    /// Tool calls the model requested this turn. `Some` only when the
    /// provider supports tool calling and the model emitted calls
    /// (`stop_reason == ToolUse`); `None` for plain completions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Mean per-token probability (0..1), derived from token logprobs.
    /// `Some` only when the provider returns per-token logprobs (llama-server
    /// with `logprobs:true`). `None` for providers that do not expose logprobs
    /// (Anthropic, Gemini, CLI). Used by the confidence-gated escalation path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Raw mean token logprob (≤ 0). Retained alongside `confidence` for
    /// calibration and debug display. `None` when logprobs are unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_logprob: Option<f32>,
}

// ---------------------------------------------------------------------------
// Provider status
// ---------------------------------------------------------------------------

/// Health-check result for a provider endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Model loaded and accepting requests.
    Ready { latency_ms: u64 },
    /// Model is loading into memory (llama-server VRAM load).
    Loading { progress: Option<f32> },
    /// Provider returned an error.
    Error { message: String, retryable: bool },
    /// Provider returned 429 — back off.
    RateLimited {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "option_duration_secs"
        )]
        retry_after: Option<Duration>,
    },
    /// Endpoint unreachable (network down, process not running).
    Offline,
}

mod option_duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(d) => s.serialize_u64(d.as_secs()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let v: Option<u64> = Option::deserialize(d)?;
        Ok(v.map(Duration::from_secs))
    }
}

// ---------------------------------------------------------------------------
// Error hierarchy
// ---------------------------------------------------------------------------

/// LLM provider errors — typed so the routing fallback logic can
/// distinguish retryable from fatal failures.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// Authentication failed (wrong or expired API key). Never retry
    /// with the same credentials.
    #[error("auth failed for provider {provider}")]
    AuthFailed { provider: String },

    /// Rate limited (429). Retry after the indicated delay, or fall
    /// back to the next model immediately.
    #[error("rate limited (retry after {retry_after:?})")]
    RateLimited { retry_after: Option<Duration> },

    /// Network error — endpoint unreachable. Retry once, then mark
    /// offline.
    #[error("network error: {message}")]
    NetworkError { message: String },

    /// Stream interrupted mid-response. Partial content may be usable.
    #[error("stream interrupted after {tokens_delivered} tokens")]
    StreamInterrupted {
        partial_content: String,
        tokens_delivered: u64,
    },

    /// Provider overloaded (503). Retry with exponential backoff.
    #[error("provider overloaded")]
    Overloaded,

    /// Local llama-server process not running. Offer restart.
    #[error("process not running for model {model_id}")]
    ProcessNotRunning { model_id: String },

    /// All configured models failed for this request.
    #[error("all providers failed: {attempts:?}")]
    AllProvidersFailed { attempts: Vec<(String, String)> },

    /// Invalid request parameters. Don't retry.
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    /// Catch-all for unexpected errors.
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl LlmError {
    /// Whether the router should attempt automatic fallback to another model.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::NetworkError { .. }
                | Self::Overloaded
                | Self::ProcessNotRunning { .. }
        )
    }

    /// Whether partial content is available from this error.
    pub fn partial_content(&self) -> Option<&str> {
        match self {
            Self::StreamInterrupted {
                partial_content, ..
            } => Some(partial_content),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// Core abstraction for LLM providers. Cloud APIs, local llama-server,
/// and remote llama-server all implement this.
///
/// Uses `async_trait` for object safety — the router holds providers as
/// `Box<dyn LlmProvider>` for dynamic dispatch.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a completion request and wait for the full response.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Send a completion request and return a streaming response.
    async fn stream(&self, request: CompletionRequest) -> Result<CompletionStream, LlmError>;

    /// Check whether the provider endpoint is healthy and ready.
    async fn health_check(&self) -> ProviderStatus;

    /// The provider identifier (e.g. `"anthropic"`, `"llama_server"`).
    fn provider_id(&self) -> &str;

    /// The specific model identifier this provider instance serves.
    fn model_id(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_round_trips_json() {
        for variant in [
            StopReason::EndTurn,
            StopReason::MaxTokens,
            StopReason::StopSequence,
            StopReason::ToolUse,
            StopReason::Error,
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let back: StopReason = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn role_round_trips_json() {
        for variant in [Role::System, Role::User, Role::Assistant] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let back: Role = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn completion_request_round_trips_json() {
        let req = CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            stop_sequences: vec![],
            system_prompt: Some("You are helpful.".to_string()),
            provider_options: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let back: CompletionRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.messages.len(), 1);
        assert_eq!(back.messages[0].role, Role::User);
        assert_eq!(back.max_tokens, Some(1024));
    }

    #[test]
    fn completion_response_round_trips_json() {
        let resp = CompletionResponse {
            content: "Hello!".to_string(),
            tokens_in: 5,
            tokens_out: 2,
            model_used: "gemma-4-27b".to_string(),
            stop_reason: StopReason::EndTurn,
            latency_ms: 42,
            tool_calls: None,
            confidence: None,
            mean_logprob: None,
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let back: CompletionResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.content, "Hello!");
        assert_eq!(back.tokens_in, 5);
        assert_eq!(back.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn provider_status_variants_round_trip_json() {
        let cases: Vec<ProviderStatus> = vec![
            ProviderStatus::Ready { latency_ms: 12 },
            ProviderStatus::Loading {
                progress: Some(0.5),
            },
            ProviderStatus::Error {
                message: "timeout".to_string(),
                retryable: true,
            },
            ProviderStatus::RateLimited {
                retry_after: Some(Duration::from_secs(30)),
            },
            ProviderStatus::Offline,
        ];
        for status in cases {
            let json = serde_json::to_string(&status).expect("serialize");
            let back: ProviderStatus = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&back).expect("re-serialize");
            assert_eq!(json, json2, "round-trip failed for {status:?}");
        }
    }

    #[test]
    fn llm_error_retryable() {
        assert!(LlmError::RateLimited { retry_after: None }.is_retryable());
        assert!(LlmError::NetworkError {
            message: "timeout".into()
        }
        .is_retryable());
        assert!(LlmError::Overloaded.is_retryable());
        assert!(LlmError::ProcessNotRunning {
            model_id: "local".into()
        }
        .is_retryable());
        assert!(!LlmError::AuthFailed {
            provider: "anthropic".into()
        }
        .is_retryable());
        assert!(!LlmError::InvalidRequest {
            message: "bad".into()
        }
        .is_retryable());
    }

    #[test]
    fn llm_error_partial_content() {
        let err = LlmError::StreamInterrupted {
            partial_content: "Hello wor".to_string(),
            tokens_delivered: 3,
        };
        assert_eq!(err.partial_content(), Some("Hello wor"));
        assert_eq!(LlmError::Overloaded.partial_content(), None);
    }

    #[test]
    fn stream_chunk_round_trips_json() {
        let chunk = StreamChunk {
            text: "Hello".to_string(),
            is_final: false,
            token_count: Some(1),
            stop_reason: None,
        };
        let json = serde_json::to_string(&chunk).expect("serialize");
        let back: StreamChunk = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.text, "Hello");
        assert!(!back.is_final);
        assert_eq!(back.token_count, Some(1));
    }

    #[test]
    fn tool_call_round_trips_json() {
        let call = ToolCall {
            id: "call_1".to_string(),
            name: "fs_read".to_string(),
            arguments: serde_json::json!({ "path": "src/lib.rs" }),
        };
        let json = serde_json::to_string(&call).expect("serialize");
        let back: ToolCall = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, call);
        assert_eq!(back.name, "fs_read");
    }

    #[test]
    fn tool_choice_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ToolChoice::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&ToolChoice::Required).unwrap(),
            "\"required\""
        );
        assert_eq!(
            serde_json::to_string(&ToolChoice::None).unwrap(),
            "\"none\""
        );
    }

    #[test]
    fn tool_calls_absent_when_none() {
        // skip_serializing_if = "Option::is_none" keeps existing wire output
        // byte-identical when no tool calls are present.
        let resp = CompletionResponse {
            content: "hi".to_string(),
            tokens_in: 1,
            tokens_out: 1,
            model_used: "local".to_string(),
            stop_reason: StopReason::EndTurn,
            latency_ms: 1,
            tool_calls: None,
            confidence: None,
            mean_logprob: None,
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        assert!(
            !json.contains("tool_calls"),
            "tool_calls must be omitted when None: {json}"
        );
        assert!(
            !json.contains("confidence"),
            "confidence must be omitted when None: {json}"
        );
        assert!(
            !json.contains("mean_logprob"),
            "mean_logprob must be omitted when None: {json}"
        );
    }
}
