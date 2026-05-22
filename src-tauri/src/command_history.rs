use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_ENTRIES: usize = 10_000;
const SUGGESTIONS_LIMIT: usize = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandRecord {
    pub command: String,
    pub cwd: String,
    #[serde(default)]
    pub project: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub lane: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandStats {
    pub total_count: usize,
    pub top_commands: Vec<CommandFrequency>,
    pub failure_hotspots: Vec<CommandFrequency>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandFrequency {
    pub command: String,
    pub count: usize,
    pub failure_rate: f64,
}

/// In-memory cache of command history. Lazy-loaded from JSONL on first access.
#[derive(Clone, Default)]
pub struct CommandHistoryStore {
    records: Arc<Mutex<Vec<CommandRecord>>>,
    loaded: Arc<Mutex<bool>>,
}

impl CommandHistoryStore {
    fn ensure_loaded(&self) -> Result<(), String> {
        if *self.loaded.lock() {
            return Ok(());
        }
        let path = history_path()?;
        let mut parsed = Vec::new();
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| format!("command_history: read: {e}"))?;
            for line in raw.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(rec) = serde_json::from_str::<CommandRecord>(line) {
                    parsed.push(rec);
                }
            }
        }
        *self.records.lock() = parsed;
        *self.loaded.lock() = true;
        Ok(())
    }

    fn append_and_save(&self, record: CommandRecord) -> Result<(), String> {
        let path = history_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("command_history: create dir: {e}"))?;
        }

        let mut records = self.records.lock();
        records.push(record.clone());

        let trimmed = if records.len() > MAX_ENTRIES {
            let drain_count = records.len() - MAX_ENTRIES;
            records.drain(..drain_count);
            true
        } else {
            false
        };

        if trimmed {
            let mut out = String::new();
            for rec in records.iter() {
                if let Ok(line) = serde_json::to_string(rec) {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            drop(records);
            std::fs::write(&path, out).map_err(|e| format!("command_history: rewrite: {e}"))?;
        } else {
            drop(records);
            let line = serde_json::to_string(&record)
                .map_err(|e| format!("command_history: serialize: {e}"))?;
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("command_history: open for append: {e}"))?;
            writeln!(file, "{line}").map_err(|e| format!("command_history: write: {e}"))?;
        }

        Ok(())
    }
}

fn history_path() -> Result<PathBuf, String> {
    let dirs = directories::ProjectDirs::from("com", "abyssal", "rift")
        .ok_or_else(|| "command_history: unable to resolve config directory".to_string())?;
    Ok(dirs.config_dir().join("command_history.jsonl"))
}

#[tauri::command]
pub fn command_history_record(
    record: CommandRecord,
    store: tauri::State<'_, CommandHistoryStore>,
) -> Result<(), String> {
    store.ensure_loaded()?;
    store.append_and_save(record)
}

#[tauri::command]
pub fn command_stats(
    project: Option<String>,
    cwd: Option<String>,
    store: tauri::State<'_, CommandHistoryStore>,
) -> Result<CommandStats, String> {
    store.ensure_loaded()?;
    let records = store.records.lock();

    let filtered: Vec<&CommandRecord> = records
        .iter()
        .filter(|r| {
            if let Some(ref p) = project {
                if r.project.as_deref() != Some(p.as_str()) {
                    return false;
                }
            }
            if let Some(ref c) = cwd {
                if r.cwd != *c {
                    return false;
                }
            }
            true
        })
        .collect();

    let total_count = filtered.len();

    // Frequency + failure counting.
    let mut freq: HashMap<&str, (usize, usize)> = HashMap::new();
    for rec in &filtered {
        let entry = freq.entry(rec.command.as_str()).or_insert((0, 0));
        entry.0 += 1;
        if rec.exit_code.map(|c| c != 0).unwrap_or(false) {
            entry.1 += 1;
        }
    }

    let mut top_commands: Vec<CommandFrequency> = freq
        .iter()
        .map(|(cmd, (count, failures))| CommandFrequency {
            command: cmd.to_string(),
            count: *count,
            failure_rate: if *count > 0 {
                *failures as f64 / *count as f64
            } else {
                0.0
            },
        })
        .collect();
    top_commands.sort_by_key(|a| std::cmp::Reverse(a.count));
    top_commands.truncate(10);

    let mut failure_hotspots: Vec<CommandFrequency> = freq
        .iter()
        .filter(|(_, (count, failures))| *count >= 3 && (*failures as f64 / *count as f64) > 0.2)
        .map(|(cmd, (count, failures))| CommandFrequency {
            command: cmd.to_string(),
            count: *count,
            failure_rate: *failures as f64 / *count as f64,
        })
        .collect();
    failure_hotspots.sort_by(|a, b| {
        b.failure_rate
            .partial_cmp(&a.failure_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    failure_hotspots.truncate(10);

    Ok(CommandStats {
        total_count,
        top_commands,
        failure_hotspots,
    })
}

#[tauri::command]
pub fn command_suggestions(
    prefix: String,
    cwd: String,
    store: tauri::State<'_, CommandHistoryStore>,
) -> Result<Vec<String>, String> {
    store.ensure_loaded()?;
    let records = store.records.lock();

    let prefix_lower = prefix.to_lowercase();
    let mut freq: HashMap<&str, usize> = HashMap::new();

    for rec in records.iter().rev() {
        if rec.cwd == cwd && rec.command.to_lowercase().starts_with(&prefix_lower) {
            *freq.entry(rec.command.as_str()).or_insert(0) += 1;
        }
    }

    let mut suggestions: Vec<(&str, usize)> = freq.into_iter().collect();
    suggestions.sort_by_key(|a| std::cmp::Reverse(a.1));
    suggestions.truncate(SUGGESTIONS_LIMIT);

    Ok(suggestions
        .into_iter()
        .map(|(cmd, _)| cmd.to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<CommandRecord> {
        vec![
            CommandRecord {
                command: "cargo build".to_string(),
                cwd: "/project".to_string(),
                project: Some("rift".to_string()),
                started_at: "2026-05-22T10:00:00Z".to_string(),
                duration_ms: Some(5000),
                exit_code: Some(0),
                lane: Some("user".to_string()),
            },
            CommandRecord {
                command: "cargo test".to_string(),
                cwd: "/project".to_string(),
                project: Some("rift".to_string()),
                started_at: "2026-05-22T10:01:00Z".to_string(),
                duration_ms: Some(8000),
                exit_code: Some(1),
                lane: Some("user".to_string()),
            },
            CommandRecord {
                command: "cargo build".to_string(),
                cwd: "/project".to_string(),
                project: Some("rift".to_string()),
                started_at: "2026-05-22T10:02:00Z".to_string(),
                duration_ms: Some(4000),
                exit_code: Some(0),
                lane: Some("user".to_string()),
            },
        ]
    }

    #[test]
    fn stats_counts_frequency() {
        let store = CommandHistoryStore::default();
        *store.loaded.lock() = true;
        *store.records.lock() = sample_records();

        // Direct computation (bypassing Tauri state).
        let records = store.records.lock();
        let mut freq: HashMap<&str, usize> = HashMap::new();
        for rec in records.iter() {
            *freq.entry(rec.command.as_str()).or_insert(0) += 1;
        }
        assert_eq!(freq.get("cargo build"), Some(&2));
        assert_eq!(freq.get("cargo test"), Some(&1));
    }

    #[test]
    fn suggestions_prefix_match() {
        let store = CommandHistoryStore::default();
        *store.loaded.lock() = true;
        *store.records.lock() = sample_records();

        let records = store.records.lock();
        let prefix = "cargo b";
        let matches: Vec<&str> = records
            .iter()
            .filter(|r| r.cwd == "/project" && r.command.to_lowercase().starts_with(prefix))
            .map(|r| r.command.as_str())
            .collect();
        assert!(matches.contains(&"cargo build"));
        assert!(!matches.contains(&"cargo test"));
    }
}
