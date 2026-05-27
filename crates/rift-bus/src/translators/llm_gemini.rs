//! Google Gemini API translator — direct HTTP client via AI Studio.
//!
//! Uses the `generateContent` / `streamGenerateContent` endpoints.
//! Auth via `key=` query parameter (AI Studio API key).
//!
//! Lives inside the §9 translator boundary.

use std::time::Instant;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::llm::{
    CompletionRequest, CompletionResponse, CompletionStream, LlmError, LlmProvider, ProviderStatus,
    Role, StopReason, StreamChunk,
};

pub struct GeminiProvider {
    client: Client,
    endpoint: String,
    api_key: String,
    model_identifier: String,
}

impl GeminiProvider {
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model_identifier: impl Into<String>,
    ) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("reqwest client"),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model_identifier: model_identifier.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Gemini wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct GenerateRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop_sequences: Vec<String>,
}

#[derive(Deserialize)]
struct GenerateResponse {
    candidates: Option<Vec<Candidate>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsageMetadata>,
    #[serde(rename = "modelVersion")]
    model_version: Option<String>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<GeminiContent>,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u64>,
}

#[derive(Deserialize)]
struct GeminiError {
    error: Option<GeminiErrorBody>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct GeminiErrorBody {
    message: Option<String>,
    code: Option<u16>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn map_role(role: &Role) -> &'static str {
    match role {
        Role::System => "user",
        Role::User => "user",
        Role::Assistant => "model",
    }
}

fn map_finish_reason(reason: &Option<String>) -> StopReason {
    match reason.as_deref() {
        Some("STOP") => StopReason::EndTurn,
        Some("MAX_TOKENS") => StopReason::MaxTokens,
        Some("STOP_SEQUENCE") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

fn build_request(req: &CompletionRequest, stream: bool) -> GenerateRequest {
    let _ = stream;

    let contents: Vec<GeminiContent> = req
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| GeminiContent {
            role: map_role(&m.role).to_string(),
            parts: vec![GeminiPart {
                text: m.content.clone(),
            }],
        })
        .collect();

    let system_instruction = req
        .system_prompt
        .as_ref()
        .or_else(|| {
            req.messages
                .iter()
                .find(|m| m.role == Role::System)
                .map(|m| &m.content)
        })
        .map(|text| GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart { text: text.clone() }],
        });

    let generation_config = if req.max_tokens.is_some()
        || req.temperature.is_some()
        || !req.stop_sequences.is_empty()
    {
        Some(GenerationConfig {
            max_output_tokens: req.max_tokens,
            temperature: req.temperature,
            stop_sequences: req.stop_sequences.clone(),
        })
    } else {
        None
    };

    GenerateRequest {
        contents,
        system_instruction,
        generation_config,
    }
}

fn extract_text(resp: &GenerateResponse) -> String {
    resp.candidates
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.content.as_ref())
        .map(|content| {
            content
                .parts
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

async fn handle_error_response(resp: reqwest::Response) -> LlmError {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    let msg = serde_json::from_str::<GeminiError>(&body)
        .ok()
        .and_then(|e| e.error)
        .and_then(|e| e.message)
        .unwrap_or_else(|| body.chars().take(200).collect());

    match status.as_u16() {
        400 => LlmError::InvalidRequest { message: msg },
        401 | 403 => LlmError::AuthFailed {
            provider: "google".to_string(),
        },
        429 => LlmError::RateLimited { retry_after: None },
        503 => LlmError::Overloaded,
        _ => LlmError::Internal {
            message: format!("HTTP {status}: {msg}"),
        },
    }
}

fn parse_stream_line(line: &str, tokens_so_far: &mut u32) -> Option<StreamChunk> {
    let line = line.trim();
    if line.is_empty() || line == "[" || line == "]" || line == "," {
        return None;
    }
    let trimmed = line.trim_start_matches(',');

    let resp: GenerateResponse = serde_json::from_str(trimmed).ok()?;
    let text = extract_text(&resp);
    let finish = resp
        .candidates
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.finish_reason.clone());

    if text.is_empty() && finish.is_none() {
        return None;
    }

    let is_final = finish.is_some();
    if !text.is_empty() {
        *tokens_so_far += 1;
    }

    Some(StreamChunk {
        text,
        is_final,
        token_count: Some(*tokens_so_far),
        stop_reason: if is_final {
            Some(map_finish_reason(&finish))
        } else {
            None
        },
    })
}

fn spawn_stream_reader(mut resp: reqwest::Response) -> CompletionStream {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk, LlmError>>(32);

    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut tokens_so_far: u32 = 0;

        while let Ok(Some(bytes)) = resp.chunk().await {
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=pos).collect();
                if let Some(chunk) = parse_stream_line(&line, &mut tokens_so_far) {
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

        if !buffer.is_empty() {
            if let Some(chunk) = parse_stream_line(&buffer, &mut tokens_so_far) {
                let _ = tx.send(Ok(chunk)).await;
            }
        }
    });

    Box::pin(super::llm_server::ReceiverStream(rx))
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmProvider for GeminiProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.endpoint, self.model_identifier, self.api_key
        );
        let body = build_request(&request, false);
        let start = Instant::now();

        let resp = self
            .client
            .post(&url)
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
        let gen_resp: GenerateResponse = resp.json().await.map_err(|e| LlmError::Internal {
            message: format!("response parse error: {e}"),
        })?;

        let content = extract_text(&gen_resp);
        let finish = gen_resp
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.finish_reason.clone());
        let usage = gen_resp.usage_metadata.as_ref();

        Ok(CompletionResponse {
            content,
            tokens_in: usage.and_then(|u| u.prompt_token_count).unwrap_or(0),
            tokens_out: usage.and_then(|u| u.candidates_token_count).unwrap_or(0),
            model_used: gen_resp
                .model_version
                .unwrap_or_else(|| self.model_identifier.clone()),
            stop_reason: map_finish_reason(&finish),
            latency_ms,
        })
    }

    async fn stream(&self, request: CompletionRequest) -> Result<CompletionStream, LlmError> {
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.endpoint, self.model_identifier, self.api_key
        );
        let body = build_request(&request, true);

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }

        Ok(spawn_stream_reader(resp))
    }

    async fn health_check(&self) -> ProviderStatus {
        let url = format!(
            "{}/models/{}?key={}",
            self.endpoint, self.model_identifier, self.api_key
        );
        let start = Instant::now();

        match self.client.get(&url).send().await {
            Err(_) => ProviderStatus::Offline,
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                match resp.status().as_u16() {
                    200 => ProviderStatus::Ready { latency_ms },
                    401 | 403 => ProviderStatus::Error {
                        message: "invalid API key".to_string(),
                        retryable: false,
                    },
                    429 => ProviderStatus::RateLimited { retry_after: None },
                    _ => ProviderStatus::Error {
                        message: format!("HTTP {}", resp.status()),
                        retryable: true,
                    },
                }
            }
        }
    }

    fn provider_id(&self) -> &str {
        "google"
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
    fn build_request_basic() {
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

        let built = build_request(&req, false);
        assert_eq!(built.contents.len(), 1);
        assert_eq!(built.contents[0].role, "user");
        assert!(built.system_instruction.is_some());
        assert_eq!(
            built.generation_config.as_ref().unwrap().max_output_tokens,
            Some(100)
        );
    }

    #[test]
    fn build_request_no_system() {
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

        let built = build_request(&req, false);
        assert!(built.system_instruction.is_none());
        assert!(built.generation_config.is_none());
    }

    #[test]
    fn map_finish_reason_variants() {
        assert_eq!(
            map_finish_reason(&Some("STOP".to_string())),
            StopReason::EndTurn
        );
        assert_eq!(
            map_finish_reason(&Some("MAX_TOKENS".to_string())),
            StopReason::MaxTokens
        );
    }

    #[test]
    fn extract_text_from_response() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "Hello!"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 2
            }
        }"#;
        let resp: GenerateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(extract_text(&resp), "Hello!");
        assert_eq!(
            resp.usage_metadata.as_ref().unwrap().prompt_token_count,
            Some(5)
        );
    }

    #[test]
    fn provider_ids() {
        let p = GeminiProvider::new(
            "https://generativelanguage.googleapis.com/v1beta",
            "test-key",
            "gemini-2.5-flash",
        );
        assert_eq!(p.provider_id(), "google");
        assert_eq!(p.model_id(), "gemini-2.5-flash");
    }

    #[test]
    fn parse_stream_chunk() {
        let line = r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"Hi"}]}}]}"#;
        let mut tokens = 0;
        let chunk = parse_stream_line(line, &mut tokens).unwrap();
        assert_eq!(chunk.text, "Hi");
        assert!(!chunk.is_final);
        assert_eq!(tokens, 1);
    }
}
