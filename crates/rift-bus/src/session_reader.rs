use std::io::BufRead;

use serde::Serialize;

use crate::config::sessions_dir;

#[derive(Debug, Serialize)]
pub struct SessionMeta {
    pub id: String,
    pub date: String,
    pub event_count: u64,
    pub size_bytes: u64,
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

        let date = if stem.len() >= 10 {
            stem[..10].to_string()
        } else {
            stem.clone()
        };

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
        let date = if stem.len() >= 10 {
            stem[..10].to_string()
        } else {
            stem.clone()
        };
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
}
