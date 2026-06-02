//! Restart-safe session snapshots (Stage 2) — persists the terminal's
//! serialized VT buffer + cwd out-of-band so a re-opened Rift can re-hydrate
//! the terminal after a restart.
//!
//! Companion to [`session_logger`](crate::session_logger) and
//! [`compaction`](crate::compaction): the logger writes the append-only
//! `.jsonl` audit log, compaction writes the `.summary.json` digest, and this
//! module writes the `.snapshot.json` VT snapshot. All three key off the same
//! session id (the newest `.jsonl` stem — see [`compaction::newest_session`]),
//! so one launch's audit log, digest, and snapshot are linked.
//!
//! The mid-flight shell process cannot survive a Rift restart (the ConPTY child
//! dies with the host) — that is the hard boundary. What survives is the
//! terminal *context*: scrollback, cwd, dims, and the compaction digest. On the
//! next launch the frontend re-hydrates the buffer and starts a *fresh* shell.
//!
//! The VT snapshot itself is produced by the frontend via xterm's
//! `@xterm/addon-serialize` (Rift's renderer is xterm.js, so its serializer
//! computes the exact replayable buffer — no Rust-side VT engine needed). This
//! module only persists/loads what the frontend hands it; the Tauri command
//! layer (`src-tauri`) is the thin bridge.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::compaction::newest_session;
use crate::config::{sessions_dir, ConfigError, SessionConfig};

/// One terminal pane's restorable state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaneSnapshot {
    /// Frontend pane id (the `session_id` arg passed to `pty_start`).
    pub pane_id: u32,
    /// Serialized xterm buffer (ANSI) from `@xterm/addon-serialize`. Replayed
    /// verbatim into a fresh terminal on restore.
    pub serialized: String,
    /// Working directory the pane was in (best-effort: the cwd it was spawned
    /// with). Restored as the fresh shell's cwd.
    pub cwd: String,
    /// Terminal rows at snapshot time.
    pub rows: u16,
    /// Terminal cols at snapshot time.
    pub cols: u16,
    /// Project root the pane belonged to, if any (for multi-project restore).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
}

/// On-disk snapshot for one launch: `<sessions_dir>/<session_id>.snapshot.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Session id — the newest `.jsonl` stem at write time. Links this snapshot
    /// to the launch's audit log and compaction digest.
    pub session_id: String,
    /// Unix epoch milliseconds when the snapshot was written.
    pub saved_ms: u64,
    /// Per-pane state. MVP writes the active pane; the schema carries many.
    pub panes: Vec<PaneSnapshot>,
}

/// What [`latest_snapshot`] returns to the frontend on boot: the prior launch's
/// snapshot plus its compaction digest (if one exists), so restore can show a
/// "what you were doing" banner alongside the re-hydrated scrollback.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestorePayload {
    /// Session id of the snapshot being restored.
    pub session_id: String,
    /// When the snapshot was written (epoch ms).
    pub saved_ms: u64,
    /// Per-pane restorable state.
    pub panes: Vec<PaneSnapshot>,
    /// First-line digest from the matching `<id>.summary.json` (Stage 1), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// Max chars of the compaction digest surfaced in the restore banner.
const DIGEST_MAX_CHARS: usize = 240;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn snapshot_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.snapshot.json"))
}

fn summary_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.summary.json"))
}

/// Pull a short, single-line digest from a `<id>.summary.json` sidecar.
/// `None` when the file is absent, unparseable, or empty.
fn read_digest(dir: &Path, session_id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(summary_path(dir, session_id)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let summary = value.get("summary").and_then(|v| v.as_str())?.trim();
    if summary.is_empty() {
        return None;
    }
    // Collapse to one line, then cap (char-boundary safe).
    let one_line = summary.replace(['\n', '\r'], " ");
    let capped = if one_line.chars().count() > DIGEST_MAX_CHARS {
        let truncated: String = one_line.chars().take(DIGEST_MAX_CHARS).collect();
        format!("{truncated}…")
    } else {
        one_line
    };
    Some(capped)
}

/// Atomically write `content` to `path` via a sibling `.tmp` + rename.
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// Directory-injected cores (testable) + sessions_dir() wrappers
// ---------------------------------------------------------------------------

/// Write a snapshot of the given panes for the *current* launch into `dir`.
/// The session id is the newest `.jsonl` stem (the active session log), so the
/// snapshot links to this launch's audit log + digest. `Err` when there is no
/// active session log to key off (session logging disabled / not yet created).
fn write_snapshot_in(dir: &Path, panes: Vec<PaneSnapshot>) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create sessions dir: {e}"))?;
    let (session_id, _path) =
        newest_session(dir).ok_or("no active session log to key the snapshot to")?;
    let snapshot = SessionSnapshot {
        session_id: session_id.clone(),
        saved_ms: now_ms(),
        panes,
    };
    let json = serde_json::to_string_pretty(&snapshot).map_err(|e| format!("serialize: {e}"))?;
    atomic_write(&snapshot_path(dir, &session_id), &json)
        .map_err(|e| format!("write snapshot: {e}"))?;
    Ok(session_id)
}

/// Find the newest restorable `.snapshot.json` in `dir`, excluding the current
/// launch's own session (so we never restore a session into itself) and any
/// snapshot older than `retention_days`. Attaches the matching compaction
/// digest. `Ok(None)` when restore is disabled or nothing qualifies.
fn latest_snapshot_in(dir: &Path, cfg: &SessionConfig) -> Result<Option<RestorePayload>, String> {
    if !cfg.restore_on_startup {
        return Ok(None);
    }
    // The current launch's session id (newest .jsonl) — skip its own snapshot.
    let current_id = newest_session(dir).map(|(id, _)| id);

    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read sessions dir: {e}")),
    };

    let cutoff_ms = now_ms().saturating_sub(u64::from(cfg.retention_days) * 86_400_000);

    let mut newest: Option<(u64, PathBuf)> = None;
    for entry in read.flatten() {
        let path = entry.path();
        // Match `<id>.snapshot.json` (file_name ends with the double suffix).
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".snapshot.json") => n.to_string(),
            _ => continue,
        };
        let id = name.trim_end_matches(".snapshot.json");
        if current_id.as_deref() == Some(id) {
            continue; // never restore the current session into itself
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            Err(_) => continue,
        };
        let take = match &newest {
            Some((t, _)) => modified > *t,
            None => true,
        };
        if take {
            newest = Some((modified, path));
        }
    }

    let (_mtime, path) = match newest {
        Some(v) => v,
        None => return Ok(None),
    };

    let raw = std::fs::read_to_string(&path).map_err(|e| format!("read snapshot: {e}"))?;
    let snapshot: SessionSnapshot =
        serde_json::from_str(&raw).map_err(|e| format!("parse snapshot: {e}"))?;
    if snapshot.saved_ms < cutoff_ms {
        return Ok(None); // older than retention — let cleanup reap it
    }
    let digest = read_digest(dir, &snapshot.session_id);
    Ok(Some(RestorePayload {
        session_id: snapshot.session_id,
        saved_ms: snapshot.saved_ms,
        panes: snapshot.panes,
        digest,
    }))
}

/// Delete the `<session_id>.snapshot.json` in `dir` (best-effort). Called once
/// a restore has been consumed so it does not replay on every boot.
fn clear_snapshot_in(dir: &Path, session_id: &str) -> Result<(), String> {
    match std::fs::remove_file(snapshot_path(dir, session_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("clear snapshot: {e}")),
    }
}

fn dir_or_err() -> Result<PathBuf, String> {
    sessions_dir().map_err(|e: ConfigError| format!("sessions dir: {e}"))
}

/// Persist a terminal snapshot for the current launch. Returns the session id
/// the snapshot was written under. See [`write_snapshot_in`].
pub fn write_snapshot(panes: Vec<PaneSnapshot>) -> Result<String, String> {
    write_snapshot_in(&dir_or_err()?, panes)
}

/// Most recent restorable snapshot from a *prior* launch, gated on
/// `restore_on_startup`. See [`latest_snapshot_in`].
pub fn latest_snapshot(cfg: &SessionConfig) -> Result<Option<RestorePayload>, String> {
    latest_snapshot_in(&dir_or_err()?, cfg)
}

/// Delete a consumed snapshot so it does not restore again. See
/// [`clear_snapshot_in`].
pub fn clear_snapshot(session_id: &str) -> Result<(), String> {
    clear_snapshot_in(&dir_or_err()?, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_restore(on: bool) -> SessionConfig {
        SessionConfig {
            restore_on_startup: on,
            ..SessionConfig::default()
        }
    }

    fn pane(id: u32) -> PaneSnapshot {
        PaneSnapshot {
            pane_id: id,
            serialized: format!("\x1b[32mhello from {id}\x1b[0m"),
            cwd: "C:/work".into(),
            rows: 40,
            cols: 120,
            project_root: Some("C:/work".into()),
        }
    }

    #[test]
    fn session_snapshot_round_trips_json() {
        let snap = SessionSnapshot {
            session_id: "2026-06-02_01-00-00".into(),
            saved_ms: 1234,
            panes: vec![pane(0)],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, "2026-06-02_01-00-00");
        assert_eq!(back.panes.len(), 1);
        assert_eq!(back.panes[0].cwd, "C:/work");
    }

    #[test]
    fn latest_returns_none_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        // Even with a valid snapshot present, disabled config yields None.
        std::fs::write(dir.path().join("2020-01-01_00-00-00.jsonl"), "{}\n").unwrap();
        write_snapshot_in(dir.path(), vec![pane(0)]).unwrap();
        let got = latest_snapshot_in(dir.path(), &cfg_restore(false)).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn write_then_latest_skips_current_session() {
        let dir = tempfile::tempdir().unwrap();
        // Only one session exists; its snapshot == the current session, so it
        // must be skipped (never restore a session into itself).
        std::fs::write(dir.path().join("2026-06-02_00-00-00.jsonl"), "{}\n").unwrap();
        let id = write_snapshot_in(dir.path(), vec![pane(0)]).unwrap();
        assert_eq!(id, "2026-06-02_00-00-00");
        let got = latest_snapshot_in(dir.path(), &cfg_restore(true)).unwrap();
        assert!(
            got.is_none(),
            "current session's own snapshot must be skipped"
        );
    }

    #[test]
    fn latest_restores_prior_session_with_digest() {
        let dir = tempfile::tempdir().unwrap();
        // Prior launch: its .jsonl, a hand-written .snapshot.json, and a digest.
        let prior = "2026-06-01_00-00-00";
        std::fs::write(dir.path().join(format!("{prior}.jsonl")), "{}\n").unwrap();
        let snap = SessionSnapshot {
            session_id: prior.into(),
            saved_ms: now_ms(),
            panes: vec![pane(7)],
        };
        std::fs::write(
            snapshot_path(dir.path(), prior),
            serde_json::to_string(&snap).unwrap(),
        )
        .unwrap();
        std::fs::write(
            summary_path(dir.path(), prior),
            serde_json::json!({
                "summary": "ran cargo test\nedited compaction.rs",
                "last_consolidated_offset": 10,
                "created_ms": 1
            })
            .to_string(),
        )
        .unwrap();
        // Current launch is a newer .jsonl, so the prior snapshot is restorable.
        std::fs::write(dir.path().join("2026-06-02_00-00-00.jsonl"), "{}\n").unwrap();

        let got = latest_snapshot_in(dir.path(), &cfg_restore(true))
            .unwrap()
            .expect("prior snapshot should restore");
        assert_eq!(got.session_id, prior);
        assert_eq!(got.panes[0].pane_id, 7);
        let digest = got.digest.expect("digest attached from summary sidecar");
        assert!(digest.contains("cargo test"));
        assert!(!digest.contains('\n'), "digest collapsed to one line");
    }

    #[test]
    fn clear_removes_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("2026-06-02_00-00-00.jsonl"), "{}\n").unwrap();
        let id = write_snapshot_in(dir.path(), vec![pane(0)]).unwrap();
        assert!(snapshot_path(dir.path(), &id).exists());
        clear_snapshot_in(dir.path(), &id).unwrap();
        assert!(!snapshot_path(dir.path(), &id).exists());
        // Idempotent — clearing a missing snapshot is Ok.
        clear_snapshot_in(dir.path(), &id).unwrap();
    }
}
