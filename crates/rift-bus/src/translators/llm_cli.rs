//! CLI provider — drives an external command-line LLM tool as a Rift model.
//!
//! Lets a model be backed by a local CLI instead of an HTTP API. The motivating
//! case: Google's `gemini` CLI authenticates with the user's Google / Gemini-Pro
//! OAuth session, so a Pro subscriber with **no API key** can still have Rift
//! (and, via the `llm_prompt` MCP tool, Claude) hand tasks to Gemini. It works
//! for any one-shot CLI tool, not just Gemini.
//!
//! ## Command template
//!
//! The model's `endpoint` field carries a command template, e.g.
//! `gemini -p {prompt} --model gemini-2.5-pro`. It is split on whitespace into
//! `program + args` (NO shell is invoked). The `{prompt}` token — standalone or
//! embedded in an arg like `--input={prompt}` — is replaced with the composed
//! prompt as a SINGLE argv element, so prompt content can never inject extra
//! flags or be re-parsed by a shell. If the template contains no `{prompt}`
//! token, the prompt is piped on the child's stdin instead.
//!
//! ## §9 boundary
//!
//! All process spawning lives here, inside the `translators/` boundary. Output
//! is captured from stdout; a non-zero exit surfaces stderr as an error.

use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_core::Stream;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use super::llm::{
    CompletionRequest, CompletionResponse, CompletionStream, LlmError, LlmProvider, Message,
    ProviderStatus, StopReason, StreamChunk,
};

/// Windows `CREATE_NO_WINDOW` — suppress the console flash on every spawn.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Default ceiling for one CLI invocation. CLI tools spin up a runtime and do a
/// full model round-trip, so this is deliberately generous.
const DEFAULT_CLI_TIMEOUT: Duration = Duration::from_secs(120);

/// An LLM provider backed by an external command-line tool.
pub struct CliProvider {
    /// Raw command template, e.g. `gemini -p {prompt} --model gemini-2.5-pro`.
    command_template: String,
    /// Reported model identifier (display / `model_id()`), not used to invoke.
    model_identifier: String,
    /// Per-invocation timeout.
    timeout: Duration,
}

impl CliProvider {
    /// Build a CLI provider from a command template and a reported model id.
    pub fn new(command_template: &str, model_identifier: &str) -> Self {
        Self {
            command_template: command_template.to_string(),
            model_identifier: model_identifier.to_string(),
            timeout: DEFAULT_CLI_TIMEOUT,
        }
    }
}

/// A parsed invocation: program + args, plus the prompt to feed on stdin when
/// the template has no `{prompt}` placeholder.
struct Invocation {
    program: String,
    args: Vec<String>,
    stdin_prompt: Option<String>,
}

/// Parse the command template and substitute the prompt.
///
/// No shell is involved: tokens are split on whitespace and `{prompt}` is
/// replaced inside whichever token references it, so the prompt is always a
/// single argv element. If no token references `{prompt}`, the prompt is routed
/// to stdin instead.
fn build_invocation(template: &str, prompt: &str) -> Result<Invocation, LlmError> {
    let mut tokens = template.split_whitespace();
    let program = tokens
        .next()
        .ok_or_else(|| LlmError::InvalidRequest {
            message: "CLI command template is empty".to_string(),
        })?
        .to_string();

    let mut saw_placeholder = false;
    let mut args: Vec<String> = tokens
        .map(|t| {
            if t.contains("{prompt}") {
                saw_placeholder = true;
                t.replace("{prompt}", prompt)
            } else {
                t.to_string()
            }
        })
        .collect();

    // The `gemini` CLI refuses to run headless in an untrusted directory
    // (exit 55) without `--skip-trust`. The generic provider is already
    // gemini-aware (it manages ~/.gemini OAuth settings below), so inject the
    // trust flag for gemini invocations only — scoped to this command, NOT a
    // global trust bypass. No-op if the user already supplied it.
    let is_gemini = std::path::Path::new(&program)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("gemini"))
        .unwrap_or(false);
    if is_gemini && !args.iter().any(|a| a == "--skip-trust") {
        args.push("--skip-trust".to_string());
    }

    Ok(Invocation {
        program,
        args,
        stdin_prompt: if saw_placeholder {
            None
        } else {
            Some(prompt.to_string())
        },
    })
}

/// Compose a single prompt string from a request: system prompt first (if any),
/// then each message's content, separated by blank lines.
fn compose_prompt(request: &CompletionRequest) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(sys) = request.system_prompt.as_deref() {
        if !sys.is_empty() {
            parts.push(sys);
        }
    }
    for Message { content, .. } in &request.messages {
        parts.push(content);
    }
    parts.join("\n\n")
}

/// Rough token estimate (~4 chars/token). CLI tools don't report usage, and
/// CLI-backed models cost $0, so an approximation is sufficient for the UI.
fn estimate_tokens(s: &str) -> u64 {
    (s.chars().count() as u64 / 4).max(1)
}

/// Resolve a bare program name to a full path on Windows, honoring `PATHEXT`.
///
/// Rust's `Command` does NOT apply `PATHEXT`, so a bare `gemini` fails to spawn
/// even though npm installed `gemini.cmd` on PATH (there is no `gemini.exe`).
/// We search PATH × PATHEXT and return the first match — typically the `.cmd`
/// shim, which Rust ≥1.77 then executes through cmd.exe with safe argument
/// escaping (the CVE-2024-24576 fix). On non-Windows, or when the name already
/// carries a path/extension, the input is returned unchanged and normal PATH
/// resolution applies. If nothing matches, the original name is returned so the
/// spawn fails with a clear NotFound.
fn resolve_program(program: &str) -> String {
    #[cfg(windows)]
    {
        if program.contains('\\') || program.contains('/') {
            return program.to_string();
        }
        if std::path::Path::new(program).extension().is_some() {
            return program.to_string();
        }
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(';').filter(|d| !d.is_empty()) {
                for ext in pathext.split(';').filter(|e| !e.is_empty()) {
                    let candidate = std::path::Path::new(dir).join(format!("{program}{ext}"));
                    if candidate.is_file() {
                        return candidate.to_string_lossy().into_owned();
                    }
                }
            }
        }
        program.to_string()
    }
    #[cfg(not(windows))]
    {
        program.to_string()
    }
}

// ---------------------------------------------------------------------------
// Gemini CLI auth detection
// ---------------------------------------------------------------------------

/// Authentication / install status for the `gemini` CLI.
///
/// Surfaced to the Settings UI so the Gemini model wizard can show "signed in
/// as X" / "not signed in" without making a network call or spawning the tool.
/// Detection is pure filesystem + PATH inspection — the OAuth credentials the
/// `gemini` CLI writes on first interactive login live under `~/.gemini/`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GeminiAuthStatus {
    /// `true` when a `gemini` binary resolves on PATH (honors Windows PATHEXT).
    pub cli_installed: bool,
    /// `true` when non-empty OAuth credentials exist at `~/.gemini/oauth_creds.json`.
    pub authenticated: bool,
    /// Best-effort signed-in account email, parsed from
    /// `~/.gemini/google_accounts.json` when present.
    pub account: Option<String>,
    /// `true` when `~/.gemini/settings.json` selects an auth method
    /// (`security.auth.selectedType`). Headless `gemini -p` exits 41 without
    /// one even when OAuth creds exist — interactive mode picks it, automated
    /// mode cannot. [`gemini_enable_headless`] sets it to `oauth-personal`.
    pub headless_ready: bool,
}

/// Path to the gemini CLI's user settings file (`~/.gemini/settings.json`).
fn gemini_settings_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().join(".gemini").join("settings.json"))
}

/// Read `security.auth.selectedType` from the gemini settings file, if set.
fn gemini_selected_auth() -> Option<String> {
    let raw = std::fs::read_to_string(gemini_settings_path()?).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("security")?
        .get("auth")?
        .get("selectedType")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Ensure headless `gemini -p` can authenticate by selecting the
/// "Login with Google" (`oauth-personal`) auth method in the gemini settings
/// file. Merges into any existing settings (never clobbers other keys); a
/// pre-existing non-empty `selectedType` is left untouched. Returns the value
/// now in effect. Lives in the translator boundary (§9): only this module
/// knows the gemini CLI's on-disk schema.
pub fn gemini_enable_headless() -> Result<String, String> {
    const OAUTH_PERSONAL: &str = "oauth-personal";
    let path = gemini_settings_path().ok_or("could not resolve home directory")?;

    // Start from existing settings (or an empty object) so we never drop keys.
    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(s) if !s.trim().is_empty() => {
            serde_json::from_str(&s).map_err(|e| format!("settings.json parse error: {e}"))?
        }
        _ => serde_json::json!({}),
    };
    if !root.is_object() {
        return Err("settings.json root is not a JSON object".to_string());
    }

    // Respect an already-selected method (e.g. a user on API-key/vertex auth).
    if let Some(existing) = gemini_selected_auth() {
        return Ok(existing);
    }

    root["security"]["auth"]["selectedType"] =
        serde_json::Value::String(OAUTH_PERSONAL.to_string());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create ~/.gemini failed: {e}"))?;
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, pretty).map_err(|e| format!("write settings.json failed: {e}"))?;
    Ok(OAUTH_PERSONAL.to_string())
}

/// Inspect the local `gemini` CLI install + OAuth session state.
///
/// Lives in the translator boundary (§9): all knowledge of the external
/// `gemini` tool's on-disk layout stays here, not in Rift core. Never errors —
/// any missing file / unreadable path degrades to `false` / `None`.
pub fn gemini_auth_status() -> GeminiAuthStatus {
    // PATH check: resolve_program returns the input unchanged when nothing
    // matches, so a successful resolution differs from the bare name (or, on
    // non-Windows, points at an existing file).
    let cli_installed = {
        let resolved = resolve_program("gemini");
        #[cfg(windows)]
        {
            resolved != "gemini"
        }
        #[cfg(not(windows))]
        {
            // resolve_program is a pass-through off-Windows; probe PATH here.
            std::env::var("PATH")
                .ok()
                .map(|path| std::env::split_paths(&path).any(|dir| dir.join("gemini").is_file()))
                .unwrap_or(false)
                || resolved != "gemini"
        }
    };

    let gemini_dir = directories::BaseDirs::new().map(|d| d.home_dir().join(".gemini"));

    let authenticated = gemini_dir
        .as_ref()
        .map(|dir| {
            std::fs::metadata(dir.join("oauth_creds.json"))
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let account = gemini_dir.as_ref().and_then(|dir| {
        let raw = std::fs::read_to_string(dir.join("google_accounts.json")).ok()?;
        let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
        // Shape is small and undocumented; accept the common keys defensively.
        json.get("active")
            .and_then(|v| v.as_str())
            .or_else(|| json.get("email").and_then(|v| v.as_str()))
            .map(str::to_string)
    });

    let headless_ready = gemini_selected_auth().is_some();

    GeminiAuthStatus {
        cli_installed,
        authenticated,
        account,
        headless_ready,
    }
}

/// A `Stream` that yields exactly one already-computed item, then ends. Used to
/// adapt the one-shot CLI result to the streaming trait method without pulling
/// in `futures-util` (mirrors `llm_server`'s hand-rolled stream approach).
struct OnceStream(Option<Result<StreamChunk, LlmError>>);

impl Stream for OnceStream {
    type Item = Result<StreamChunk, LlmError>;
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().0.take())
    }
}

#[async_trait]
impl LlmProvider for CliProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = compose_prompt(&request);
        let inv = build_invocation(&self.command_template, &prompt)?;
        let started = Instant::now();

        let mut cmd = Command::new(resolve_program(&inv.program));
        cmd.args(&inv.args);
        cmd.stdin(if inv.stdin_prompt.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        // Kill the child if the timeout future is dropped, so a hung CLI never
        // leaves an orphan process behind.
        cmd.kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                // Surfaced as retryable so the router can fall back to another
                // model — and the message names the missing program.
                LlmError::ProcessNotRunning {
                    model_id: inv.program.clone(),
                }
            } else {
                LlmError::Internal {
                    message: format!("spawn '{}' failed: {e}", inv.program),
                }
            }
        })?;

        if let Some(stdin_prompt) = &inv.stdin_prompt {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(stdin_prompt.as_bytes()).await;
                let _ = stdin.shutdown().await;
            }
        }

        let output = timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| LlmError::NetworkError {
                message: format!(
                    "CLI '{}' timed out after {}s",
                    inv.program,
                    self.timeout.as_secs()
                ),
            })?
            .map_err(|e| LlmError::Internal {
                message: format!("CLI '{}' wait failed: {e}", inv.program),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let code = output.status.code().unwrap_or(-1);
            return Err(LlmError::Internal {
                message: format!(
                    "CLI '{}' exited {code}: {}",
                    inv.program,
                    if stderr.is_empty() {
                        "(no stderr)"
                    } else {
                        &stderr
                    }
                ),
            });
        }

        let content = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if content.is_empty() {
            return Err(LlmError::Internal {
                message: format!("CLI '{}' produced no output", inv.program),
            });
        }

        Ok(CompletionResponse {
            tokens_in: estimate_tokens(&prompt),
            tokens_out: estimate_tokens(&content),
            content,
            model_used: self.model_identifier.clone(),
            stop_reason: StopReason::EndTurn,
            latency_ms: started.elapsed().as_millis() as u64,
            tool_calls: None,
            confidence: None,
            mean_logprob: None,
        })
    }

    async fn stream(&self, request: CompletionRequest) -> Result<CompletionStream, LlmError> {
        // CLI tools are one-shot here: run to completion, then surface the whole
        // result as a single final chunk.
        let resp = self.complete(request).await?;
        let chunk = StreamChunk {
            text: resp.content,
            is_final: true,
            token_count: Some(resp.tokens_out as u32),
            stop_reason: Some(StopReason::EndTurn),
        };
        Ok(Box::pin(OnceStream(Some(Ok(chunk)))))
    }

    async fn health_check(&self) -> ProviderStatus {
        let Some(program) = self.command_template.split_whitespace().next() else {
            return ProviderStatus::Error {
                message: "empty CLI command template".to_string(),
                retryable: false,
            };
        };
        let mut cmd = Command::new(resolve_program(program));
        cmd.arg("--version");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let started = Instant::now();
        match timeout(Duration::from_secs(10), cmd.status()).await {
            // Binary resolved and ran — treat it as reachable regardless of the
            // exit code (some CLIs return non-zero for `--version`).
            Ok(Ok(_)) => ProviderStatus::Ready {
                latency_ms: started.elapsed().as_millis() as u64,
            },
            // Spawn failed (not found / not executable) or timed out.
            Ok(Err(_)) | Err(_) => ProviderStatus::Offline,
        }
    }

    fn provider_id(&self) -> &str {
        "cli"
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
    use crate::translators::llm::Role;

    fn req(user: &str) -> CompletionRequest {
        CompletionRequest {
            messages: vec![Message {
                role: Role::User,
                content: user.to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stop_sequences: vec![],
            system_prompt: None,
            provider_options: None,
        }
    }

    #[test]
    fn substitutes_prompt_as_single_argv_element() {
        let inv = build_invocation("gemini -p {prompt} --model gemini-2.5-pro", "hello world")
            .expect("invocation");
        assert_eq!(inv.program, "gemini");
        // gemini gets `--skip-trust` auto-appended (headless trust — exit 55).
        assert_eq!(
            inv.args,
            vec![
                "-p",
                "hello world",
                "--model",
                "gemini-2.5-pro",
                "--skip-trust"
            ]
        );
        assert!(inv.stdin_prompt.is_none());
    }

    #[test]
    fn substitutes_embedded_placeholder() {
        let inv = build_invocation("tool --input={prompt}", "abc").expect("invocation");
        assert_eq!(inv.program, "tool");
        assert_eq!(inv.args, vec!["--input=abc"]);
        assert!(inv.stdin_prompt.is_none());
    }

    #[test]
    fn no_placeholder_routes_prompt_to_stdin() {
        let inv = build_invocation("gemini chat", "piped prompt").expect("invocation");
        assert_eq!(inv.program, "gemini");
        // `--skip-trust` auto-appended for gemini even on the stdin path.
        assert_eq!(inv.args, vec!["chat", "--skip-trust"]);
        assert_eq!(inv.stdin_prompt.as_deref(), Some("piped prompt"));
    }

    #[test]
    fn gemini_skip_trust_not_duplicated() {
        // User already supplied --skip-trust → we must not add a second one.
        let inv = build_invocation("gemini --skip-trust -p {prompt}", "x").expect("invocation");
        let n = inv.args.iter().filter(|a| *a == "--skip-trust").count();
        assert_eq!(n, 1, "skip-trust should appear exactly once");
    }

    #[test]
    fn non_gemini_gets_no_skip_trust() {
        // Only gemini gets the trust flag — a generic tool is left untouched.
        let inv = build_invocation("tool --input={prompt}", "abc").expect("invocation");
        assert!(!inv.args.iter().any(|a| a == "--skip-trust"));
    }

    #[test]
    fn empty_template_errors() {
        assert!(build_invocation("   ", "x").is_err());
    }

    #[test]
    fn resolve_program_passes_through_unknown() {
        // No PATH match → return the name unchanged so spawn fails NotFound.
        assert_eq!(
            resolve_program("definitely-not-a-real-binary-xyz"),
            "definitely-not-a-real-binary-xyz"
        );
    }

    #[cfg(windows)]
    #[test]
    fn resolve_program_finds_cmd_via_pathext() {
        // `cmd` has no .exe-less spawn problem, but it proves PATHEXT lookup:
        // a bare name resolves to a real on-disk executable path.
        let resolved = resolve_program("cmd");
        assert!(
            resolved.to_lowercase().ends_with("cmd.exe"),
            "expected a cmd.exe path, got {resolved}"
        );
    }

    #[test]
    fn compose_joins_system_and_messages() {
        let mut r = req("question");
        r.system_prompt = Some("be terse".to_string());
        assert_eq!(compose_prompt(&r), "be terse\n\nquestion");
    }

    // Real spawn → capture round-trip via a portable `echo`, proving the
    // template parse + process spawn + stdout capture path end-to-end.
    #[tokio::test]
    async fn echo_round_trip_captures_stdout() {
        #[cfg(windows)]
        let template = "cmd /C echo {prompt}";
        #[cfg(not(windows))]
        let template = "echo {prompt}";

        let provider = CliProvider::new(template, "echo-model");
        let resp = provider
            .complete(req("ping-CLI-12345"))
            .await
            .expect("echo should succeed");
        assert!(
            resp.content.contains("ping-CLI-12345"),
            "captured stdout should contain the prompt, got: {:?}",
            resp.content
        );
        assert_eq!(resp.model_used, "echo-model");
    }

    #[tokio::test]
    async fn missing_program_is_process_not_running() {
        let provider = CliProvider::new("definitely-not-a-real-binary-xyz {prompt}", "m");
        let err = provider.complete(req("hi")).await.expect_err("should fail");
        assert!(matches!(err, LlmError::ProcessNotRunning { .. }));
    }
}
