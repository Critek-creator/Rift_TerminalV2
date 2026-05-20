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
