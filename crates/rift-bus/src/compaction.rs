//! Idle session compaction — digests the older prefix of the active session
//! log into a sidecar summary for context re-hydration.
//!
//! Companion to [`session_logger`](crate::session_logger): the logger writes an
//! append-only `.jsonl` of every bus envelope; this task NEVER edits that audit
//! log. On bus idle, it summarizes the older prefix (everything before the most
//! recent `keep_suffix_events`) via an injected LLM provider and writes a
//! sidecar `<id>.summary.json` — a *soft-cursor* (`last_consolidated_offset`
//! marker + summary), leaving the audit trail byte-for-byte intact.
//!
//! Spawned at `setup()` like `session_logger`, with the summarizer provider
//! injected from the app layer — rift-bus does not build providers (the
//! `create_provider` factory lives in `src-tauri`), so the dependency points
//! down, not up.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::bus::{RiftBus, SubscribeFilter};
use crate::config::{sessions_dir, SessionConfig};
use crate::envelope::{Category, Envelope};
use crate::translators::llm::{CompletionRequest, LlmProvider, Message, Role};

/// Factory that resolves + builds a fresh summarizer provider on demand.
///
/// Injected from the app layer (rift-bus does not build providers — the
/// `create_provider` factory lives in `src-tauri`). Called at *each* compaction
/// so the currently-serving grunt-tier model is used — availability-aware,
/// matching the router's resident-aware routing. Returns `None` when no
/// suitable model is available, in which case the compaction cycle is skipped.
pub type SummarizerFactory = Arc<dyn Fn() -> Option<Box<dyn LlmProvider>> + Send + Sync>;

/// Max characters of prefix fed to the summarizer (grunt-tier context safety).
const MAX_PREFIX_CHARS: usize = 16_000;
/// Max tokens for the summary completion.
const SUMMARY_MAX_TOKENS: u32 = 512;

const SUMMARIZE_SYSTEM: &str = "You digest a terminal session activity log into a terse \
context summary for re-hydration after an idle gap. The input is JSONL bus envelopes \
(commands run, files touched, agents dispatched, errors, status snapshots). Produce a \
compact plain-text digest of what happened: key commands, files, agents, and any errors \
or notable state. No preamble, no markdown headers — just the digest.";

/// Sidecar written next to a session `.jsonl`. Survives process restart so a
/// re-attaching client (Stage 2) can re-hydrate context without replaying the
/// full log. The audit `.jsonl` is never modified — this is the soft-cursor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    /// LLM-generated digest of the older prefix.
    pub summary: String,
    /// Number of leading `.jsonl` lines (envelopes) covered by `summary`.
    /// Everything at or after this offset is the retained verbatim suffix.
    pub last_consolidated_offset: u64,
    /// Unix epoch milliseconds when the summary was written.
    pub created_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sidecar path for a session id: `<sessions_dir>/<id>.summary.json`.
fn sidecar_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.summary.json"))
}

/// Find the newest `.jsonl` session file (the active session). Returns its
/// stem (session id) and path. `None` if the dir is empty or unreadable.
fn newest_session(dir: &Path) -> Option<(String, PathBuf)> {
    let mut newest: Option<(SystemTime, String, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let take = match &newest {
            Some((t, _, _)) => modified > *t,
            None => true,
        };
        if take {
            newest = Some((modified, stem, path));
        }
    }
    newest.map(|(_, id, path)| (id, path))
}

/// Read the older prefix (all but the last `keep_suffix` lines) of a session
/// `.jsonl`. Returns `(prefix_text, line_offset)` or `None` when there is not
/// enough history to compact. `prefix_text` is capped to [`MAX_PREFIX_CHARS`],
/// tail-biased (the prefix nearest the suffix is the most relevant context).
fn read_prefix(path: &Path, keep_suffix: usize) -> Option<(String, u64)> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= keep_suffix {
        return None; // not enough history beyond the retained suffix
    }
    let cut = lines.len() - keep_suffix;
    let mut text = lines[..cut].join("\n");
    if text.len() > MAX_PREFIX_CHARS {
        // Keep the tail of the prefix; snap to a char boundary to avoid a
        // panic on a multibyte split.
        let raw_start = text.len() - MAX_PREFIX_CHARS;
        let start = (raw_start..text.len())
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or_else(|| text.len());
        text = format!("[older prefix truncated]\n{}", &text[start..]);
    }
    Some((text, cut as u64))
}

/// Summarize the prefix via the injected provider. `None` on any provider error
/// or an empty completion.
async fn summarize(provider: &dyn LlmProvider, prefix: &str) -> Option<String> {
    let request = CompletionRequest {
        messages: vec![Message {
            role: Role::User,
            content: prefix.to_string(),
        }],
        max_tokens: Some(SUMMARY_MAX_TOKENS),
        temperature: Some(0.2),
        stop_sequences: vec![],
        system_prompt: Some(SUMMARIZE_SYSTEM.to_string()),
        provider_options: None,
    };
    match provider.complete(request).await {
        Ok(resp) => {
            let s = resp.content.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        Err(e) => {
            tracing::warn!("compaction: summarize failed: {e}");
            None
        }
    }
}

/// Compact one session: read prefix → summarize → write sidecar. Returns the
/// summary on success (for the completion envelope). Never touches the `.jsonl`.
async fn compact_session(
    dir: &Path,
    session_id: &str,
    path: &Path,
    cfg: &SessionConfig,
    provider: &dyn LlmProvider,
) -> Option<String> {
    let (prefix, offset) = read_prefix(path, cfg.keep_suffix_events)?;
    let summary = summarize(provider, &prefix).await?;
    let sidecar = SessionSummary {
        summary: summary.clone(),
        last_consolidated_offset: offset,
        created_ms: now_ms(),
    };
    let json = serde_json::to_string_pretty(&sidecar).ok()?;
    if let Err(e) = std::fs::write(sidecar_path(dir, session_id), json) {
        tracing::warn!("compaction: failed to write sidecar for {session_id}: {e}");
        return None;
    }
    tracing::info!(
        "compaction: digested {offset} envelopes of session {session_id} ({} summary chars)",
        summary.len()
    );
    Some(summary)
}

/// Shared one-shot core: resolve a summarizer, find the newest session,
/// compact its prefix. Returns `(session_id, summary)`. `Err` carries a
/// human-readable reason (no session / no summarizer serving / nothing to
/// compact / summarize failed) — surfaced to the on-demand caller, logged at
/// debug for the idle watcher.
async fn run_once(
    dir: &Path,
    cfg: &SessionConfig,
    summarizer: &SummarizerFactory,
) -> Result<(String, String), String> {
    let (id, path) = newest_session(dir).ok_or("no session log found")?;
    let provider = summarizer().ok_or("no local summarizer model serving")?;
    let summary = compact_session(dir, &id, &path, cfg, provider.as_ref())
        .await
        .ok_or("nothing to compact (session <= keep_suffix_events) or summarize failed")?;
    Ok((id, summary))
}

/// On-demand compaction of the newest session, IGNORING the idle/enabled gate
/// (still honors `keep_suffix_events`). Backs the `rift session compact`
/// trigger, so a manual compaction works even when auto-compaction is off.
pub async fn compact_now(
    cfg: &SessionConfig,
    summarizer: &SummarizerFactory,
) -> Result<(String, String), String> {
    let dir = sessions_dir().map_err(|e| format!("sessions dir: {e}"))?;
    run_once(&dir, cfg, summarizer).await
}

/// Run the idle-compaction watcher. Subscribes to the bus; when no envelope
/// arrives for `idle_compact_after_minutes`, compacts the newest session log
/// once and publishes a `system` completion envelope, then re-arms on the next
/// activity. Returns immediately if the session logger is disabled or
/// compaction is off (`idle_compact_after_minutes == 0`).
///
/// Callers wrap this in `tauri::async_runtime::spawn`, injecting a summarizer
/// factory built at the app layer (resolves the live grunt-tier provider per
/// call):
/// ```ignore
/// tauri::async_runtime::spawn(async move {
///     spawn_compaction(bus, cfg, shutdown, factory).await;
/// });
/// ```
pub async fn spawn_compaction(
    bus: RiftBus,
    cfg: SessionConfig,
    shutdown: Arc<Notify>,
    summarizer: SummarizerFactory,
) {
    if !cfg.enabled || cfg.idle_compact_after_minutes == 0 {
        tracing::debug!("compaction: disabled by config");
        return;
    }
    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("compaction: could not resolve sessions dir: {e}");
            return;
        }
    };
    let idle = Duration::from_secs(u64::from(cfg.idle_compact_after_minutes) * 60);
    let (_replay, mut sub) = bus.subscribe(SubscribeFilter::All);
    // True once we've compacted for the current idle stretch; reset on activity
    // so one long idle period triggers a single compaction, not one per interval.
    let mut compacted_this_idle = false;

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                tracing::debug!("compaction: shutdown");
                break;
            }
            recv = tokio::time::timeout(idle, sub.recv()) => {
                match recv {
                    // Bus activity — re-arm the idle window.
                    Ok(Ok(_env)) => {
                        compacted_this_idle = false;
                    }
                    // Bus closed (or unrecoverable receive error) — exit cleanly.
                    Ok(Err(e)) => {
                        tracing::debug!("compaction: bus recv ended ({e}) — watcher stopping");
                        break;
                    }
                    // Idle threshold elapsed with no traffic — compact once.
                    Err(_elapsed) => {
                        if compacted_this_idle {
                            continue;
                        }
                        compacted_this_idle = true;
                        match run_once(&dir, &cfg, &summarizer).await {
                            Ok((id, summary)) => {
                                let mut env =
                                    Envelope::new(Category::System, "session.compaction.complete");
                                env.payload = serde_json::json!({
                                    "session_id": id,
                                    "summary_chars": summary.len(),
                                });
                                bus.publish(env);
                            }
                            Err(e) => {
                                tracing::debug!("compaction: idle cycle skipped — {e}");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_prefix_none_when_below_suffix() {
        // 3 lines, keep 100 → nothing to compact.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rift-compaction-test-{}.jsonl", now_ms()));
        std::fs::write(&path, "a\nb\nc\n").unwrap();
        assert!(read_prefix(&path, 100).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_prefix_splits_at_suffix() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rift-compaction-test2-{}.jsonl", now_ms()));
        // 10 lines, keep last 3 → prefix is the first 7, offset = 7.
        let body: String = (0..10).map(|i| format!("line{i}\n")).collect();
        std::fs::write(&path, body).unwrap();
        let (prefix, offset) = read_prefix(&path, 3).unwrap();
        assert_eq!(offset, 7);
        assert!(prefix.contains("line0"));
        assert!(prefix.contains("line6"));
        assert!(!prefix.contains("line7")); // line7..9 are the retained suffix
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn session_summary_round_trips_json() {
        let s = SessionSummary {
            summary: "did stuff".into(),
            last_consolidated_offset: 42,
            created_ms: 1234,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.last_consolidated_offset, 42);
        assert_eq!(back.summary, "did stuff");
    }
}
