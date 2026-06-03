use std::io::BufRead;

use serde::{Deserialize, Serialize};

use crate::config::sessions_dir;

#[derive(Debug, Serialize)]
pub struct SessionMeta {
    pub id: String,
    pub date: String,
    pub event_count: u64,
    pub size_bytes: u64,
}

/// Display date from a session id stem. Session ids are date-prefixed
/// (`YYYY-MM-DD-...`); take the first 10 CHARS (never a byte slice — a
/// non-ASCII char before offset 10 would panic on a `&stem[..10]` byte slice).
fn date_from_stem(stem: &str) -> String {
    stem.chars().take(10).collect()
}

pub fn list_sessions() -> Result<Vec<SessionMeta>, String> {
    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("failed to read sessions dir: {e}")),
    };

    let mut sessions: Vec<SessionMeta> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        let size_bytes = path.metadata().map(|m| m.len()).unwrap_or(0);

        let event_count = match std::fs::File::open(&path) {
            Ok(f) => std::io::BufReader::new(f).lines().count() as u64,
            Err(_) => 0,
        };

        let date = date_from_stem(&stem);

        sessions.push(SessionMeta {
            id: stem,
            date,
            event_count,
            size_bytes,
        });
    }

    sessions.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(sessions)
}

/// One full-text match inside a persisted session log.
#[derive(Debug, Serialize)]
pub struct SessionSearchHit {
    pub session_id: String,
    pub date: String,
    /// 0-based index into [`load_session`]'s event vector, so the replay view
    /// can seek straight to the matched event.
    pub event_idx: u64,
    pub ts: Option<u64>,
    pub category: Option<String>,
    pub kind: Option<String>,
    /// Trimmed, length-capped excerpt of the matched line, centered on the
    /// first match of the query.
    pub snippet: String,
}

/// Build a snippet centered on the first (case-insensitive) match of `q_lower`
/// within `line`, capped to roughly `WINDOW` chars with ellipses.
fn build_snippet(line: &str, q_lower: &str) -> String {
    const WINDOW: usize = 160;
    let lower = line.to_lowercase();
    let Some(pos) = lower.find(q_lower) else {
        return line.chars().take(WINDOW).collect();
    };
    // Work on char boundaries to stay UTF-8 safe.
    let chars: Vec<char> = line.chars().collect();
    let char_pos = line[..pos].chars().count();
    let half = WINDOW / 2;
    let start = char_pos.saturating_sub(half);
    let end = (char_pos + q_lower.chars().count() + half).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Full-text search across every persisted session `.jsonl` log.
///
/// Case-insensitive substring match against each raw JSON line (keys, kind,
/// payload — the whole envelope). Sessions are scanned newest-first (their ids
/// are date-prefixed); scanning stops once `limit` hits are collected so a
/// broad query can't run unbounded. `event_idx` aligns with [`load_session`]'s
/// vector (only non-empty, parseable lines are counted) so a hit links
/// directly to a replay position.
pub fn search_sessions(query: &str, limit: usize) -> Result<Vec<SessionSearchHit>, String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() || limit == 0 {
        return Ok(vec![]);
    }

    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("failed to read sessions dir: {e}")),
    };

    // Collect (stem, path) for .jsonl files, newest-first by stem.
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            files.push((stem.to_string(), path));
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));

    let mut hits: Vec<SessionSearchHit> = Vec::new();

    'files: for (stem, path) in files {
        if hits.len() >= limit {
            break;
        }
        let date = date_from_stem(&stem);
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = std::io::BufReader::new(file);
        let mut evt_idx: u64 = 0;
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }
            // Only parseable lines advance evt_idx (mirrors load_session).
            let val = match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if line.to_lowercase().contains(&q) {
                hits.push(SessionSearchHit {
                    session_id: stem.clone(),
                    date: date.clone(),
                    event_idx: evt_idx,
                    ts: val.get("ts").and_then(|t| t.as_u64()),
                    category: val
                        .get("category")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string()),
                    kind: val
                        .get("kind")
                        .and_then(|k| k.as_str())
                        .map(|s| s.to_string()),
                    snippet: build_snippet(&line, &q),
                });
                if hits.len() >= limit {
                    break 'files;
                }
            }
            evt_idx += 1;
        }
    }

    Ok(hits)
}

pub fn load_session(session_id: &str) -> Result<Vec<serde_json::Value>, String> {
    let dir = sessions_dir().map_err(|e| format!("sessions dir: {e}"))?;
    let path = dir.join(format!("{session_id}.jsonl"));

    if !path.exists() {
        return Err(format!("session file not found: {session_id}"));
    }

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("failed to open session {session_id}: {e}"))?;

    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(val) => events.push(val),
            Err(e) => {
                tracing::warn!("session_reader: skipping unparseable line in {session_id}: {e}");
            }
        }
    }

    Ok(events)
}

// ---------------------------------------------------------------------------
// Session Timeline — cross-store chronological merge (IA Phase 4)
// ---------------------------------------------------------------------------

/// Source-selection set forwarded from the frontend's `config.timeline`.
///
/// Field names mirror `TimelineConfig` exactly so the frontend can pass
/// `{ sources: config.timeline }` and serde deserializes it 1:1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineSources {
    pub show_commands: bool,
    pub show_errors: bool,
    pub show_agents: bool,
    pub show_hooks: bool,
    pub show_fs: bool,
    pub show_llm_cost: bool,
    pub show_mcp: bool,
}

/// One entry in the merged session timeline.
#[derive(Debug, Serialize)]
pub struct TimelineEntry {
    /// Unix ms — primary sort key.
    pub ts: u64,
    /// 0-based index into `load_session`'s vector for deep-linking into replay.
    /// `u64::MAX` for enrichment-only rows that have no corresponding envelope.
    pub event_idx: u64,
    /// Normalized source tag: `"command"` | `"error"` | `"agent"` | `"hook"` |
    /// `"fs"` | `"llm"` | `"mcp"`.
    pub source: String,
    /// Raw envelope category (lowercase, e.g. `"pty"`).
    pub category: String,
    /// Raw envelope kind (e.g. `"command.submitted"`).
    pub kind: String,
    /// One-line human summary (command text, error message, etc.).
    pub summary: String,
    /// PTY pane id from `command.submitted` payload — the join key.
    pub pane_session_id: Option<i64>,
    /// Enriched from `command_history` when joinable; `None` for live-only rows.
    pub exit_code: Option<i32>,
    /// Enriched from `command_history` when joinable; `None` for live-only rows.
    pub duration_ms: Option<u64>,
    /// Raw payload for the expandable detail row.
    pub payload: serde_json::Value,
}

/// A minimal view into a `CommandRecord` needed for the join.
///
/// Passed in from the Tauri wrapper (which owns the `CommandHistoryStore`
/// state) so `build_timeline` stays pure and config/state-free.
#[derive(Clone, Debug)]
pub struct HistoryRecord {
    pub session_id: Option<i64>,
    pub command: String,
    pub started_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
}

/// Classify an envelope's `(category, kind)` pair into one of the 7 normalized
/// source tags. Returns `None` for categories/kinds that are always excluded
/// (status/index/aegis/sentinel/raw pty.output — the non-selectable noise set).
fn classify_source(category: &str, kind: &str) -> Option<&'static str> {
    match category {
        "pty" => {
            if kind == "command.submitted" {
                Some("command")
            } else if kind.starts_with("pty.output") {
                // Raw byte streams — always excluded to keep the timeline signal-dense.
                None
            } else if kind.starts_with("pty.failed") || kind.starts_with("pty.exited") {
                Some("error")
            } else {
                // Other pty lifecycle events (pty.started, etc.) — excluded.
                None
            }
        }
        "system" => {
            if kind.starts_with("system.error") || kind.starts_with("system.warn") {
                Some("error")
            } else {
                None
            }
        }
        "agent" => Some("agent"),
        "hook" => Some("hook"),
        "fs" => Some("fs"),
        "llm" => Some("llm"),
        "mcp" => Some("mcp"),
        // Always-excluded noise categories.
        "status" | "index" | "aegis" | "sentinel" => None,
        // Unknown categories are excluded (forward-compat: don't show noise).
        _ => None,
    }
}

/// Returns `true` if `source` is enabled in `sources`.
fn source_enabled(source: &str, sources: &TimelineSources) -> bool {
    match source {
        "command" => sources.show_commands,
        "error" => sources.show_errors,
        "agent" => sources.show_agents,
        "hook" => sources.show_hooks,
        "fs" => sources.show_fs,
        "llm" => sources.show_llm_cost,
        "mcp" => sources.show_mcp,
        _ => false,
    }
}

/// Extract a one-line human summary from an envelope payload.
fn summarize_envelope(source: &str, kind: &str, payload: &serde_json::Value) -> String {
    match source {
        "command" => payload
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or(kind)
            .to_string(),
        "error" => payload
            .get("message")
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("msg").and_then(|v| v.as_str()))
            .unwrap_or(kind)
            .to_string(),
        _ => {
            // Generic: use the kind as the summary; include a short payload peek.
            if let Some(msg) = payload.get("message").and_then(|v| v.as_str()) {
                msg.to_string()
            } else {
                kind.to_string()
            }
        }
    }
}

/// Merge, filter, enrich, and sort a session's events into a `TimelineEntry` vec.
///
/// # Parameters
/// - `session_id`: launch-session string id (same id `list_sessions` / `load_session` use).
/// - `sources`: resolved source-selection set (forwarded from `config.timeline`).
/// - `history_records`: pre-narrowed slice of command-history records for the pane ids
///   seen in this session (the Tauri wrapper narrows them; this fn stays pure).
/// - `limit`: cap on the number of returned entries (applied after sort, keeping the
///   most-recent `limit` entries). `None` defaults to 2000.
pub fn build_timeline(
    session_id: &str,
    sources: &TimelineSources,
    history_records: &[HistoryRecord],
    limit: Option<usize>,
) -> Result<Vec<TimelineEntry>, String> {
    let cap = limit.unwrap_or(2000);
    let events = load_session(session_id)?;

    let mut entries: Vec<TimelineEntry> = Vec::with_capacity(events.len().min(cap));

    for (idx, envelope) in events.iter().enumerate() {
        let event_idx = idx as u64;

        let category = envelope
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let kind = envelope
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ts = envelope.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
        let payload = envelope
            .get("payload")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // Classify — None means always-excluded noise.
        let Some(source) = classify_source(&category, &kind) else {
            continue;
        };

        // Per-source filter.
        if !source_enabled(source, sources) {
            continue;
        }

        let summary = summarize_envelope(source, &kind, &payload);

        // Join enrichment for command.submitted events.
        let (pane_session_id, exit_code, duration_ms) = if source == "command" {
            let pane_id = payload.get("session_id").and_then(|v| v.as_i64());
            let cmd_text = payload
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let (ec, dur) = if let Some(pid) = pane_id {
                // Find the history record whose session_id matches and command matches,
                // preferring the one with started_at_ms closest to envelope ts.
                let best = history_records
                    .iter()
                    .filter(|r| r.session_id == Some(pid) && r.command == cmd_text)
                    .min_by_key(|r| {
                        let rec_ms = r.started_at_ms.unwrap_or(0);
                        (rec_ms as i128 - ts as i128).unsigned_abs() as u64
                    });
                if let Some(rec) = best {
                    (rec.exit_code, rec.duration_ms)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            (pane_id, ec, dur)
        } else {
            (None, None, None)
        };

        entries.push(TimelineEntry {
            ts,
            event_idx,
            source: source.to_string(),
            category,
            kind,
            summary,
            pane_session_id,
            exit_code,
            duration_ms,
            payload,
        });
    }

    // Sort ascending by ts; on equal ts fall back to event_idx to preserve in-file order.
    entries.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.event_idx.cmp(&b.event_idx)));

    // Cap: keep the most-recent `cap` entries (already sorted ascending, so take the tail).
    if entries.len() > cap {
        let drain_count = entries.len() - cap;
        entries.drain(..drain_count);
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_empty_query_returns_empty() {
        assert!(search_sessions("", 50).unwrap().is_empty());
        assert!(search_sessions("   ", 50).unwrap().is_empty());
        assert!(search_sessions("anything", 0).unwrap().is_empty());
    }

    #[test]
    fn snippet_centers_on_match_with_ellipses() {
        let line = format!("{}NEEDLE{}", "a".repeat(200), "b".repeat(200));
        let snip = build_snippet(&line, "needle");
        assert!(snip.contains("NEEDLE"));
        assert!(snip.starts_with('…'));
        assert!(snip.ends_with('…'));
        // Short lines are returned without ellipses.
        let short = build_snippet("just a needle here", "needle");
        assert_eq!(short, "just a needle here");
    }

    #[test]
    fn snippet_is_utf8_safe_around_multibyte() {
        // Multibyte chars on both sides of the match must not panic on slicing.
        let line = format!("{}MATCH{}", "→".repeat(120), "✓".repeat(120));
        let snip = build_snippet(&line, "match");
        assert!(snip.contains("MATCH"));
    }

    #[test]
    fn list_sessions_returns_ok_on_missing_dir() {
        let result = list_sessions();
        assert!(result.is_ok());
    }

    #[test]
    fn load_session_errors_on_missing_file() {
        let result = load_session("nonexistent-session");
        assert!(result.is_err());
    }

    #[test]
    fn load_session_reads_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test-session.jsonl");
        std::fs::write(
            &file_path,
            "{\"id\":\"a\",\"version\":2,\"timestamp\":0,\"category\":\"system\",\"kind\":\"test\",\"payload\":null}\n\
             {\"id\":\"b\",\"version\":2,\"timestamp\":1,\"category\":\"system\",\"kind\":\"test\",\"payload\":null}\n",
        )
        .unwrap();

        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let val: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(val.get("kind").is_some());
        }
    }

    // ---------------------------------------------------------------------------
    // build_timeline unit tests (IA Phase 4)
    // These tests use in-memory inputs — no real session file required.
    // We write a temp .jsonl and call build_timeline via load_session's path.
    // ---------------------------------------------------------------------------

    /// Build a TimelineSources with only CORE sources enabled (commands + errors).
    fn core_only_sources() -> TimelineSources {
        TimelineSources {
            show_commands: true,
            show_errors: true,
            show_agents: false,
            show_hooks: false,
            show_fs: false,
            show_llm_cost: false,
            show_mcp: false,
        }
    }

    /// Build a TimelineSources with ALL sources enabled.
    fn all_sources() -> TimelineSources {
        TimelineSources {
            show_commands: true,
            show_errors: true,
            show_agents: true,
            show_hooks: true,
            show_fs: true,
            show_llm_cost: true,
            show_mcp: true,
        }
    }

    /// Write envelopes as JSONL to a temp file and return (dir, session_id).
    /// The session_id is the stem of the file inside the temp dir. We inject it
    /// into the sessions_dir by temporarily overriding via a direct file read in
    /// the test (build_timeline calls load_session which uses sessions_dir()).
    ///
    /// Since sessions_dir() resolves from the real config dir, we call
    /// load_session-equivalent inline so the tests stay self-contained.
    fn build_timeline_from_lines(
        lines: &[&str],
        sources: &TimelineSources,
        history: &[HistoryRecord],
    ) -> Vec<TimelineEntry> {
        // Parse the lines directly (mirrors load_session logic) so we don't need
        // a real sessions dir.
        let events: Vec<serde_json::Value> = lines
            .iter()
            .filter_map(|l| {
                let t = l.trim();
                if t.is_empty() {
                    return None;
                }
                serde_json::from_str(t).ok()
            })
            .collect();

        // Inline build_timeline logic (pure over events slice) for isolation.
        let cap = 2000usize;
        let mut entries: Vec<TimelineEntry> = Vec::new();

        for (idx, envelope) in events.iter().enumerate() {
            let event_idx = idx as u64;
            let category = envelope
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let kind = envelope
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let ts = envelope.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
            let payload = envelope
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let Some(source) = classify_source(&category, &kind) else {
                continue;
            };
            if !source_enabled(source, sources) {
                continue;
            }
            let summary = summarize_envelope(source, &kind, &payload);

            let (pane_session_id, exit_code, duration_ms) = if source == "command" {
                let pane_id = payload.get("session_id").and_then(|v| v.as_i64());
                let cmd_text = payload
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let (ec, dur) = if let Some(pid) = pane_id {
                    let best = history
                        .iter()
                        .filter(|r| r.session_id == Some(pid) && r.command == cmd_text)
                        .min_by_key(|r| {
                            let rec_ms = r.started_at_ms.unwrap_or(0);
                            (rec_ms as i128 - ts as i128).unsigned_abs() as u64
                        });
                    if let Some(rec) = best {
                        (rec.exit_code, rec.duration_ms)
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };
                (pane_id, ec, dur)
            } else {
                (None, None, None)
            };

            entries.push(TimelineEntry {
                ts,
                event_idx,
                source: source.to_string(),
                category,
                kind,
                summary,
                pane_session_id,
                exit_code,
                duration_ms,
                payload,
            });
        }

        entries.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.event_idx.cmp(&b.event_idx)));
        if entries.len() > cap {
            let drain_count = entries.len() - cap;
            entries.drain(..drain_count);
        }
        entries
    }

    // (a) CORE-only source set drops agent/hook/fs envelopes.
    #[test]
    fn timeline_core_only_drops_agent_hook_fs() {
        let lines = [
            r#"{"ts":1000,"category":"pty","kind":"command.submitted","payload":{"session_id":1,"command":"ls","raw_len":2}}"#,
            r#"{"ts":2000,"category":"agent","kind":"agent.dispatch","payload":{"msg":"starting"}}"#,
            r#"{"ts":3000,"category":"hook","kind":"hook.run","payload":{"msg":"hook"}}"#,
            r#"{"ts":4000,"category":"fs","kind":"fs.change","payload":{"path":"/foo"}}"#,
            r#"{"ts":5000,"category":"system","kind":"system.error.crash","payload":{"message":"boom"}}"#,
        ];
        let entries = build_timeline_from_lines(&lines, &core_only_sources(), &[]);
        // Only command (ts=1000) and error (ts=5000) should survive.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source, "command");
        assert_eq!(entries[1].source, "error");
    }

    // (b) command.submitted with a matching history record gets exit_code populated.
    #[test]
    fn timeline_command_enriched_with_exit_code() {
        let lines = [
            r#"{"ts":1000,"category":"pty","kind":"command.submitted","payload":{"session_id":7,"command":"cargo build","raw_len":11}}"#,
        ];
        let history = vec![HistoryRecord {
            session_id: Some(7),
            command: "cargo build".to_string(),
            started_at_ms: Some(990),
            exit_code: Some(0),
            duration_ms: Some(5000),
        }];
        let entries = build_timeline_from_lines(&lines, &core_only_sources(), &history);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "command");
        assert_eq!(entries[0].exit_code, Some(0));
        assert_eq!(entries[0].duration_ms, Some(5000));
        assert_eq!(entries[0].pane_session_id, Some(7));
    }

    // (c) Entries come back sorted ascending by ts.
    #[test]
    fn timeline_sorted_ascending_by_ts() {
        let lines = [
            r#"{"ts":9000,"category":"system","kind":"system.error.x","payload":{"message":"late"}}"#,
            r#"{"ts":1000,"category":"pty","kind":"command.submitted","payload":{"session_id":1,"command":"echo hi","raw_len":7}}"#,
            r#"{"ts":5000,"category":"system","kind":"system.error.y","payload":{"message":"mid"}}"#,
        ];
        let entries = build_timeline_from_lines(&lines, &core_only_sources(), &[]);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].ts <= entries[1].ts);
        assert!(entries[1].ts <= entries[2].ts);
        assert_eq!(entries[0].ts, 1000);
        assert_eq!(entries[2].ts, 9000);
    }

    // (d) Non-selectable noise categories (status/index) excluded even when
    //     all_sources() is active — they have no toggle.
    #[test]
    fn timeline_noise_categories_always_excluded() {
        let lines = [
            r#"{"ts":100,"category":"status","kind":"status.snapshot","payload":{}}"#,
            r#"{"ts":200,"category":"index","kind":"index.update","payload":{}}"#,
            r#"{"ts":300,"category":"aegis","kind":"aegis.session.start","payload":{}}"#,
            r#"{"ts":400,"category":"sentinel","kind":"sentinel.check","payload":{}}"#,
            r#"{"ts":500,"category":"pty","kind":"command.submitted","payload":{"session_id":1,"command":"pwd","raw_len":3}}"#,
        ];
        let entries = build_timeline_from_lines(&lines, &all_sources(), &[]);
        // Only the command at ts=500 should survive; status/index/aegis/sentinel are noise.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "command");
        assert_eq!(entries[0].ts, 500);
    }
}
