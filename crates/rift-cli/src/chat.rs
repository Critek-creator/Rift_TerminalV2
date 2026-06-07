//! `rift chat` — interactive multi-turn REPL against a local model, routed
//! through Rift's §9-clean gateway (the same path as `rift llm prompt`, but
//! conversational). Intended as an always-available local lifeline: when the
//! cloud assistant is rate-limited or offline, `rift chat` still answers
//! (degraded but functional — a small local model is not a frontier model).
//!
//! **Phase 1a (this module): single-shot per turn.** The full message history
//! is re-sent each turn via the `llm_chat` host tool, which re-uses the
//! router's model selection, fallback chain, and bus events. Token streaming
//! is Phase 1b. No HTTP is made here — only IPC to the running host.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::Result;
use reedline::{
    DefaultPrompt, DefaultPromptSegment, FileBackedHistory, Reedline, Signal, ValidationResult,
    Validator,
};
use rift_bus::Category;
use serde_json::{json, Value};

use crate::llm::{host_call, host_call_streaming};

/// Options parsed from the `rift chat` subcommand.
#[derive(Debug)]
pub struct ChatOptions {
    pub model: Option<String>,
    pub system: Option<String>,
    pub system_file: Option<PathBuf>,
    pub ground: Option<String>,
    pub max_tokens: Option<u32>,
}

/// One message in the in-process conversation history.
#[derive(Clone)]
struct Turn {
    /// `"system"` | `"user"` | `"assistant"`.
    role: &'static str,
    content: String,
}

/// Conservative context-window fallback (tokens) when `llm_models` can't be
/// reached or the chosen model isn't found — matches the smallest local ctx
/// we ship, so the summarization trigger errs toward firing early.
const CTX_FALLBACK: u64 = 65_536;

/// Fraction of the context window at which we summarize older turns.
const SUMMARIZE_AT_PCT: u64 = 70;

/// Conversational turns kept verbatim when summarizing (most-recent N).
const KEEP_RECENT: usize = 8;

/// Index entries selected for grounding (`--ground` / `/ground`).
const GROUND_TOP_K: usize = 5;

/// Char budget for the injected grounding block — bounded so the system prefix
/// stays cache-friendly regardless of whether prefix caching holds on the
/// model (the Mamba/SWA risk flagged in the Phase 2 research).
const GROUND_CHAR_BUDGET: usize = 24_000;

/// Prefix marking an injected grounding block, so `/ground` can replace a
/// prior one without disturbing the rules / other system messages.
const GROUND_MARKER: &str = "[Knowledge base";

/// Rough token estimate (≈4 chars/token). Providers don't report input tokens
/// per turn, so this drives the context-fill gauge and the summarization
/// trigger. Deliberately rounds up.
fn est_tokens(s: &str) -> u64 {
    (s.len() as u64).div_ceil(4)
}

/// Estimated context fill as a percentage of the model's window.
fn ctx_fill_pct(history: &[Turn], max_ctx: u64) -> u64 {
    if max_ctx == 0 {
        return 0;
    }
    let used: u64 = history.iter().map(|t| est_tokens(&t.content)).sum();
    (used.saturating_mul(100) / max_ctx).min(100)
}

/// Validator that keeps the input open while a fenced code block is unclosed
/// (odd count of ``` markers), so users can author/paste multi-line code.
struct FenceValidator;

impl Validator for FenceValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        if line.matches("```").count() % 2 == 1 {
            ValidationResult::Incomplete
        } else {
            ValidationResult::Complete
        }
    }
}

/// Resolve the session system-prefix "rules" (a local CLAUDE.md for the
/// model). Order, first match wins: `--system` inline → `--system-file <path>`
/// → project-local `.rift/rules.md` → global `~/.config/rift/rules.md`.
/// Returns `(content, source-label)`.
fn resolve_rules(opts: &ChatOptions) -> Option<(String, String)> {
    if let Some(s) = &opts.system {
        return Some((s.clone(), "--system".to_string()));
    }
    if let Some(p) = &opts.system_file {
        match std::fs::read_to_string(p) {
            Ok(c) => return Some((c, format!("--system-file {}", p.display()))),
            Err(e) => eprintln!(
                "warning: could not read --system-file {}: {e} (continuing without it)",
                p.display()
            ),
        }
    }
    // Project-local overrides global.
    let project = std::path::Path::new(".rift").join("rules.md");
    if let Ok(c) = std::fs::read_to_string(&project) {
        return Some((c, project.display().to_string()));
    }
    if let Some(home) = home_dir() {
        let global = home.join(".config").join("rift").join("rules.md");
        if let Ok(c) = std::fs::read_to_string(&global) {
            return Some((c, global.display().to_string()));
        }
    }
    None
}

/// Home directory via the platform env var (USERPROFILE on Windows, HOME
/// elsewhere) — avoids a `dirs` dependency for a single lookup.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Build a grounding context block from the Abyssal Index via FTS5 selection
/// (`index_search` → `index_get`). Returns `(injected_text, entry_count)`, or
/// `None` when the index is unavailable (Rift built without `--features index`)
/// or has no matches. Bounded to [`GROUND_CHAR_BUDGET`] so the prefix stays
/// cache-friendly. Selection happens ONCE (session start or `/ground`) — never
/// per turn, which would invalidate the prefix cache.
async fn ground_from_index(
    socket_arg: Option<&str>,
    query: &str,
    top_k: usize,
) -> Option<(String, usize)> {
    let search = host_call(
        socket_arg,
        "index_search",
        json!({ "query": query, "limit": top_k }),
    )
    .await
    .ok()?;
    let nodes = search.as_array()?;
    if nodes.is_empty() {
        return None;
    }

    let mut ctx = String::from(
        "[Knowledge base — relevant Abyssal Index entries for this session. Use them as grounding context.]\n",
    );
    let mut used = 0usize;
    let mut count = 0usize;
    for n in nodes.iter().take(top_k) {
        let Some(id) = n.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let title = n.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let Ok(full) = host_call(socket_arg, "index_get", json!({ "id": id })).await else {
            continue;
        };
        let body = full.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let entry = format!("\n## {title}\n{body}\n");
        if used + entry.len() > GROUND_CHAR_BUDGET {
            break;
        }
        used += entry.len();
        count += 1;
        ctx.push_str(&entry);
    }
    if count == 0 {
        None
    } else {
        Some((ctx, count))
    }
}

/// Entry point for `rift chat`.
pub async fn run_chat(socket_arg: Option<&str>, opts: ChatOptions) -> Result<()> {
    // Resolve the model label + context window for the fill gauge and the
    // summarization trigger. Best-effort — falls back to conservative values.
    let (model_label, max_ctx) = resolve_model(socket_arg, opts.model.as_deref()).await;

    println!("rift chat — model: {model_label} (ctx {max_ctx} tokens)");
    println!(
        "Slash commands: /clear  /model <id>  /tokens  /ground <topic>  /save <path>  /help  /exit   (Ctrl-D to quit)\n"
    );

    let mut history: Vec<Turn> = Vec::new();
    if let Some((rules, src)) = resolve_rules(&opts) {
        println!("rules: loaded from {src} ({} chars)", rules.len());
        history.push(Turn {
            role: "system",
            content: rules,
        });
    }
    if let Some(q) = opts.ground.clone() {
        match ground_from_index(socket_arg, &q, GROUND_TOP_K).await {
            Some((ctx, n)) => {
                println!(
                    "grounded: {n} Abyssal Index entr{} for {q:?}",
                    if n == 1 { "y" } else { "ies" }
                );
                history.push(Turn {
                    role: "system",
                    content: ctx,
                });
            }
            None => println!(
                "grounding: index unavailable or no matches for {q:?} — continuing without \
                 (needs an index-enabled Rift build)"
            ),
        }
    }

    let mut model_override = opts.model.clone();
    let mut cum_in: u64 = 0;
    let mut cum_out: u64 = 0;

    let mut line_editor = Reedline::create().with_validator(Box::new(FenceValidator));
    // Cross-session history is a nicety, not a requirement — skip silently if
    // the file can't be opened.
    let hist_path = std::env::temp_dir().join("rift-chat-history.txt");
    if let Ok(h) = FileBackedHistory::with_file(1000, hist_path) {
        line_editor = line_editor.with_history(Box::new(h));
    }
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("you".into()),
        DefaultPromptSegment::Empty,
    );

    loop {
        let input = match line_editor.read_line(&prompt) {
            Ok(Signal::Success(buf)) => buf,
            Ok(Signal::CtrlC) => continue, // cancel the current line
            Ok(Signal::CtrlD) => {
                println!("bye.");
                break;
            }
            // `Signal` is #[non_exhaustive] — treat any future variant as a
            // benign no-op rather than failing to compile against new reedline.
            Ok(_) => continue,
            Err(e) => {
                eprintln!("input error: {e}");
                break;
            }
        };
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // ---- Slash commands -------------------------------------------------
        if let Some(rest) = trimmed.strip_prefix('/') {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let cmd = parts.next().unwrap_or("");
            let arg = parts.next().map(str::trim).unwrap_or("");
            match cmd {
                "exit" | "quit" => {
                    println!("bye.");
                    break;
                }
                "clear" => {
                    history.retain(|t| t.role == "system");
                    cum_in = 0;
                    cum_out = 0;
                    println!("(history cleared — system prompt kept)");
                }
                "model" => {
                    if arg.is_empty() {
                        println!(
                            "current model: {}",
                            model_override.as_deref().unwrap_or("auto (router default)")
                        );
                    } else {
                        model_override = Some(arg.to_string());
                        println!("model set to {arg}");
                    }
                }
                "tokens" => {
                    println!(
                        "session tokens — in {cum_in}, out {cum_out}; context fill ~{}%",
                        ctx_fill_pct(&history, max_ctx)
                    );
                }
                "save" => {
                    if arg.is_empty() {
                        println!("usage: /save <path>");
                    } else {
                        match save_transcript(arg, &history) {
                            Ok(()) => println!("saved transcript to {arg}"),
                            Err(e) => println!("save failed: {e}"),
                        }
                    }
                }
                "ground" => {
                    if arg.is_empty() {
                        println!("usage: /ground <topic>");
                    } else {
                        match ground_from_index(socket_arg, arg, GROUND_TOP_K).await {
                            Some((ctx, n)) => {
                                // Replace any prior grounding block; keep it within
                                // the leading system-prefix region.
                                history.retain(|t| {
                                    !(t.role == "system" && t.content.starts_with(GROUND_MARKER))
                                });
                                let at = history.iter().take_while(|t| t.role == "system").count();
                                history.insert(
                                    at,
                                    Turn {
                                        role: "system",
                                        content: ctx,
                                    },
                                );
                                println!("grounded: {n} index entries on {arg:?}");
                            }
                            None => {
                                println!("grounding: index unavailable or no matches for {arg:?}")
                            }
                        }
                    }
                }
                "help" | "" => print_help(),
                other => println!("unknown command: /{other}  (try /help)"),
            }
            continue;
        }

        // ---- Conversational turn -------------------------------------------
        history.push(Turn {
            role: "user",
            content: trimmed.to_string(),
        });

        // Compress older turns BEFORE sending if we're near the window. Never
        // silently truncates — summarization is loud, and on failure the full
        // history is kept (so we degrade to provider-side handling, not loss).
        maybe_summarize(socket_arg, &mut history, max_ctx, model_override.as_deref()).await;

        let messages: Vec<Value> = history
            .iter()
            .map(|t| json!({ "role": t.role, "content": t.content }))
            .collect();
        let mut args = json!({ "messages": messages, "tier": "partner" });
        {
            let map = args.as_object_mut().expect("args is an object");
            if let Some(m) = &model_override {
                map.insert("model_id".into(), json!(m));
            }
            if let Some(mt) = opts.max_tokens {
                map.insert("max_tokens".into(), json!(mt));
            }
        }

        // Estimated input tokens for the gauge (streaming doesn't report them).
        let tin: u64 = history.iter().map(|t| est_tokens(&t.content)).sum();
        let started = std::time::Instant::now();

        // Stream tokens live (Phase 1b). If streaming fails before any text was
        // printed (e.g. an older host without `llm_chat_stream`), fall back to a
        // single-shot `llm_chat`. A mid-stream failure keeps the partial text —
        // it is never silently dropped.
        println!();
        let mut streamed = String::new();
        let stream_res =
            host_call_streaming(socket_arg, "llm_chat_stream", args.clone(), |delta| {
                print!("{delta}");
                let _ = std::io::stdout().flush();
                streamed.push_str(delta);
            })
            .await;

        // Resolve the turn to a common shape: Some((content, tokens_out, model, ms)).
        let outcome: Option<(String, u64, String, u64)> = match stream_res {
            Ok(meta) => {
                println!("\n");
                if streamed.trim().is_empty() {
                    println!("(empty response — try --max-tokens higher or a different model)\n");
                }
                let tout = meta.get("tokens_out").and_then(Value::as_u64).unwrap_or(0);
                let model_used = meta
                    .get("model_used")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&model_label)
                    .to_string();
                let latency = meta
                    .get("latency_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| started.elapsed().as_millis() as u64);
                Some((streamed, tout, model_used, latency))
            }
            // Nothing streamed yet — safe to retry single-shot (no duplicate output).
            Err(_) if streamed.is_empty() => match host_call(socket_arg, "llm_chat", args).await {
                Ok(result) => {
                    let content = result
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tout = result
                        .get("tokens_out")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let model_used = result
                        .get("model_used")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&model_label)
                        .to_string();
                    if content.trim().is_empty() {
                        println!(
                            "(empty response — try --max-tokens higher or a different model)\n"
                        );
                    } else {
                        println!("{content}\n");
                    }
                    Some((
                        content,
                        tout,
                        model_used,
                        started.elapsed().as_millis() as u64,
                    ))
                }
                Err(e2) => {
                    // Drop the user turn we appended so a retry doesn't double-send it.
                    history.pop();
                    eprintln!("chat error: {e2}");
                    eprintln!(
                        "(is a local model running? check `rift llm list` or start one in the cockpit)"
                    );
                    None
                }
            },
            Err(e) => {
                // Partial text already printed — keep it, surface the error.
                println!();
                eprintln!("stream interrupted: {e}");
                Some((
                    streamed,
                    0,
                    model_label.clone(),
                    started.elapsed().as_millis() as u64,
                ))
            }
        };

        if let Some((content, tout, model_used, latency)) = outcome {
            cum_in += tin;
            cum_out += tout;
            history.push(Turn {
                role: "assistant",
                content,
            });
            let fill = ctx_fill_pct(&history, max_ctx);
            // Dim status suffix (never silent about token usage).
            println!(
                "\x1b[2m[{model_used} · in {tin} out {tout} · ctx {fill}% · {latency}ms]\x1b[0m\n"
            );
            publish_chat_turn(socket_arg, &model_used, tin, tout, latency).await;
        }
    }

    Ok(())
}

/// Query `llm_models` for the chosen (or default) model's display name and
/// context window. Best-effort: returns conservative fallbacks on any failure.
async fn resolve_model(socket_arg: Option<&str>, model_override: Option<&str>) -> (String, u64) {
    let fallback = || (model_override.unwrap_or("auto").to_string(), CTX_FALLBACK);
    let Ok(v) = host_call(socket_arg, "llm_models", json!({})).await else {
        return fallback();
    };
    let models = v.get("models").and_then(|m| m.as_array());
    let Some(models) = models else {
        return fallback();
    };

    // Target id: explicit override, else the configured default_model.
    let target = model_override.map(str::to_string).or_else(|| {
        v.get("default_model")
            .and_then(|d| d.as_str())
            .map(String::from)
    });

    let pick = match &target {
        Some(id) => models
            .iter()
            .find(|m| m.get("id").and_then(|x| x.as_str()) == Some(id.as_str())),
        None => models.first(),
    };

    match pick {
        Some(m) => {
            let label = m
                .get("display_name")
                .and_then(|x| x.as_str())
                .or_else(|| m.get("id").and_then(|x| x.as_str()))
                .unwrap_or("auto")
                .to_string();
            let ctx = m
                .get("capabilities")
                .and_then(|c| c.get("max_context_tokens"))
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .unwrap_or(CTX_FALLBACK);
            (label, ctx)
        }
        None => fallback(),
    }
}

/// Summarize the oldest conversational turns when context fill crosses the
/// threshold, keeping system messages and the most-recent `KEEP_RECENT` turns
/// verbatim. Returns `true` if it compressed anything. Loud, never silent.
async fn maybe_summarize(
    socket_arg: Option<&str>,
    history: &mut Vec<Turn>,
    max_ctx: u64,
    model_override: Option<&str>,
) -> bool {
    if ctx_fill_pct(history, max_ctx) < SUMMARIZE_AT_PCT {
        return false;
    }
    // Indices of non-system (conversational) turns, in order.
    let convo: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, t)| t.role != "system")
        .map(|(i, _)| i)
        .collect();
    if convo.len() <= KEEP_RECENT {
        // Too few turns to compress — the bulk is in single large turns we
        // won't drop. Let the provider handle it; warn once.
        return false;
    }
    let cut = convo.len() - KEEP_RECENT;
    let to_summarize = &convo[..cut];

    let mut transcript = String::new();
    for &i in to_summarize {
        let t = &history[i];
        transcript.push_str(t.role);
        transcript.push_str(": ");
        transcript.push_str(&t.content);
        transcript.push('\n');
    }

    let sum_prompt = format!(
        "Summarize the following conversation excerpt concisely, preserving \
         facts, decisions, names, numbers, and open questions. Output only the \
         summary, no preamble.\n\n{transcript}"
    );
    let mut args = json!({
        "messages": [{ "role": "user", "content": sum_prompt }],
        "tier": "grunt",
    });
    if let Some(m) = model_override {
        args.as_object_mut()
            .expect("args is an object")
            .insert("model_id".into(), json!(m));
    }

    let summary = match host_call(socket_arg, "llm_chat", args).await {
        Ok(r) => r
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        // On failure, keep the full history — degrade to provider handling,
        // not data loss.
        Err(_) => return false,
    };
    if summary.is_empty() {
        return false;
    }

    // Rebuild: existing system turns (order preserved) + the new summary +
    // the most-recent KEEP_RECENT conversational turns.
    let keep: std::collections::HashSet<usize> = convo[cut..].iter().copied().collect();
    let mut rebuilt: Vec<Turn> = history
        .iter()
        .filter(|t| t.role == "system")
        .cloned()
        .collect();
    rebuilt.push(Turn {
        role: "system",
        content: format!("[Summary of earlier conversation]\n{summary}"),
    });
    for (i, t) in history.iter().enumerate() {
        if keep.contains(&i) {
            rebuilt.push(t.clone());
        }
    }
    *history = rebuilt;

    println!("\x1b[2m[summarized {cut} earlier turn(s) to stay within the context window]\x1b[0m");
    true
}

/// Publish an `llm.chat.turn` envelope so the cockpit LLM-activity tab reflects
/// chat sessions alongside one-shot prompts. Best-effort — never blocks chat.
async fn publish_chat_turn(
    socket_arg: Option<&str>,
    model_used: &str,
    tokens_in: u64,
    tokens_out: u64,
    latency_ms: u64,
) {
    let Ok(socket) = crate::resolve_socket(socket_arg) else {
        return;
    };
    let payload = json!({
        "model_id": model_used,
        "tokens_in": tokens_in,
        "tokens_out": tokens_out,
        "latency_ms": latency_ms,
        "source": "rift-chat",
        "tier": "partner",
    });
    let _ = crate::publish_event(&socket, Category::Llm, "llm.chat.turn", payload).await;
}

/// Write the transcript to a file as `role: content` lines.
fn save_transcript(path: &str, history: &[Turn]) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for t in history {
        let _ = writeln!(out, "{}: {}\n", t.role, t.content);
    }
    std::fs::write(path, out)
}

fn print_help() {
    println!(
        "commands:\n  \
         /clear         reset the conversation (keeps the system prompt)\n  \
         /model <id>    switch the model for the next turn (no arg = show current)\n  \
         /tokens        show session token usage + context fill\n  \
         /ground <topic> re-select Abyssal Index context for the topic\n  \
         /save <path>   write the transcript to a file\n  \
         /help          this help\n  \
         /exit          quit (or Ctrl-D)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &'static str, content: &str) -> Turn {
        Turn {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn resolve_rules_prefers_inline_system() {
        // Inline `--system` short-circuits before any file lookup, so this is
        // deterministic regardless of the test machine's rules.md files.
        let opts = ChatOptions {
            model: None,
            system: Some("be terse".to_string()),
            system_file: None,
            ground: None,
            max_tokens: None,
        };
        let (content, src) = resolve_rules(&opts).expect("inline system should resolve");
        assert_eq!(content, "be terse");
        assert_eq!(src, "--system");
    }

    #[test]
    fn ground_marker_matches_injected_prefix() {
        // The /ground replacement logic relies on this prefix relationship.
        let injected = "[Knowledge base — relevant Abyssal Index entries for this session.";
        assert!(injected.starts_with(GROUND_MARKER));
    }

    #[test]
    fn est_tokens_rounds_up() {
        assert_eq!(est_tokens(""), 0);
        assert_eq!(est_tokens("abc"), 1); // 3 chars → ceil(3/4) = 1
        assert_eq!(est_tokens("abcd"), 1);
        assert_eq!(est_tokens("abcde"), 2);
    }

    #[test]
    fn ctx_fill_is_bounded_and_proportional() {
        let h = vec![turn("user", &"x".repeat(400))]; // ~100 tokens
        assert_eq!(ctx_fill_pct(&h, 1000), 10);
        // Never exceeds 100 even when over budget.
        let big = vec![turn("user", &"x".repeat(40_000))];
        assert_eq!(ctx_fill_pct(&big, 1000), 100);
        // Zero ctx is safe.
        assert_eq!(ctx_fill_pct(&h, 0), 0);
    }

    #[test]
    fn fence_validator_holds_open_on_unclosed_fence() {
        let v = FenceValidator;
        assert!(matches!(
            v.validate("```rust"),
            ValidationResult::Incomplete
        ));
        assert!(matches!(
            v.validate("```rust\ncode\n```"),
            ValidationResult::Complete
        ));
        assert!(matches!(
            v.validate("plain text"),
            ValidationResult::Complete
        ));
    }
}
