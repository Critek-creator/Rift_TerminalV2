//! Session event persistence — writes every bus envelope to a per-launch
//! `.jsonl` file under the platform sessions directory.
//!
//! The logger is a plain bus subscriber (not a translator — it touches no
//! external system). Spawned at setup() as a tokio task mirroring the
//! vault-walker pattern.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

use crate::bus::{BusError, RiftBus, SubscribeFilter};
use crate::config::{sessions_dir, SessionConfig};
use crate::envelope::Envelope;

/// Generate a filesystem-safe session identifier from the current UTC time.
///
/// Format: `YYYY-MM-DD_HH-MM-SS` (colons replaced with hyphens for Windows).
fn generate_session_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = now.as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let total_days = secs / 86400;

    // Days since epoch → year/month/day via a civil-calendar algorithm.
    let (y, mo, d) = days_to_ymd(total_days as i64);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{m:02}-{s:02}")
}

/// Convert days since 1970-01-01 to (year, month, day).
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from Howard Hinnant's `chrono`-compatible civil_from_days.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Delete session files older than `retention_days`.
///
/// Reaps all three per-launch artifacts that share the `YYYY-MM-DD_HH-MM-SS`
/// id: the `.jsonl` audit log, the `.summary.json` compaction digest, and the
/// `.snapshot.json` VT snapshot. Reaping only `.jsonl` (the prior behavior)
/// orphaned the sidecars, which then accumulated forever.
///
/// Runs once at startup (sync I/O is fine for a bounded one-shot scan).
/// On per-file errors, logs a warning and continues.
fn cleanup_old_sessions(sessions_dir: &Path, retention_days: u32) {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(retention_days) * 86400))
        .unwrap_or(UNIX_EPOCH);

    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!("session_logger: cleanup read_dir failed: {e}");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        // Match any per-launch session artifact, not just the `.jsonl`. Keying
        // off the file NAME (not `file_stem`, which keeps `.summary`/`.snapshot`
        // for double-extension sidecars) keeps the date offset uniform.
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let is_session_artifact = name.ends_with(".jsonl")
            || name.ends_with(".summary.json")
            || name.ends_with(".snapshot.json");
        if !is_session_artifact {
            continue;
        }
        // Parse date from the first 10 chars: YYYY-MM-DD (leading id segment).
        if name.len() < 10 {
            continue;
        }
        let date_part = &name[..10];
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() != 3 {
            continue;
        }
        let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ) else {
            continue;
        };

        // Reconstruct the file date as days-since-epoch for comparison.
        let file_days = ymd_to_days(y, m, d);
        let cutoff_secs = cutoff
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let cutoff_days = cutoff_secs / 86400;

        if (file_days as u64) < cutoff_days {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(
                    "session_logger: failed to delete old session {}: {e}",
                    path.display()
                );
            } else {
                tracing::info!("session_logger: deleted old session {}", path.display());
            }
        }
    }
}

/// Convert (year, month, day) to days since 1970-01-01.
fn ymd_to_days(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Redact sensitive fields from envelopes before persistence.
///
/// Currently handles `mcp.handshake` which may carry a raw auth token
/// in its payload. Returns a new envelope with `token` replaced by
/// `"[REDACTED]"`.
fn redact_envelope(env: &Envelope) -> Cow<'_, Envelope> {
    if env.kind == "mcp.handshake" {
        let mut redacted = env.clone();
        if let serde_json::Value::Object(ref mut map) = redacted.payload {
            if map.contains_key("token") {
                map.insert(
                    "token".to_string(),
                    serde_json::Value::String("[REDACTED]".to_string()),
                );
            }
        }
        return Cow::Owned(redacted);
    }
    Cow::Borrowed(env)
}

/// Run the session logger. Subscribes to the bus and writes every envelope
/// as a JSON line to `<sessions_dir>/<session_id>.jsonl`.
///
/// Returns immediately if `cfg.enabled` is `false`.
///
/// Callers wrap this in `tauri::async_runtime::spawn`:
/// ```ignore
/// tauri::async_runtime::spawn(async move {
///     spawn_session_logger(bus, cfg, shutdown).await;
/// });
/// ```
pub async fn spawn_session_logger(bus: RiftBus, cfg: SessionConfig, shutdown: Arc<Notify>) {
    if !cfg.enabled {
        tracing::debug!("session_logger: disabled by config");
        return;
    }

    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("session_logger: could not resolve sessions dir: {e}");
            return;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("session_logger: failed to create sessions dir: {e}");
        return;
    }

    cleanup_old_sessions(&dir, cfg.retention_days);

    let session_id = generate_session_id();
    let file_path = dir.join(format!("{session_id}.jsonl"));

    let file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                "session_logger: failed to open {}: {e}",
                file_path.display()
            );
            return;
        }
    };

    let mut writer = tokio::io::BufWriter::new(file);
    let max_bytes = u64::from(cfg.max_file_size_mb) * 1024 * 1024;
    let mut written_bytes: u64 = 0;
    let mut capped = false;

    let (_replay, mut sub) = bus.subscribe(SubscribeFilter::All);

    // Write replay snapshot first — these events happened before subscribe.
    for env in &_replay {
        if capped {
            break;
        }
        let env = redact_envelope(env);
        if let Ok(line) = serde_json::to_string(&env) {
            let line_bytes = line.len() as u64 + 1; // +1 for newline
            if written_bytes + line_bytes > max_bytes {
                capped = true;
                tracing::warn!(
                    "session_logger: file size cap ({} MiB) reached — stopping writes",
                    cfg.max_file_size_mb
                );
                break;
            }
            if let Err(e) = writer.write_all(line.as_bytes()).await {
                tracing::warn!("session_logger: replay write failed, stopping: {e}");
                break;
            }
            if let Err(e) = writer.write_all(b"\n").await {
                tracing::warn!("session_logger: replay write failed, stopping: {e}");
                break;
            }
            written_bytes += line_bytes;
        }
    }
    let _ = writer.flush().await;

    loop {
        tokio::select! {
            result = sub.recv() => {
                match result {
                    Ok(env) => {
                        if capped {
                            continue;
                        }
                        let env = redact_envelope(&env);
                        let line = match serde_json::to_string(&env) {
                            Ok(l) => l,
                            Err(_) => continue,
                        };
                        let line_bytes = line.len() as u64 + 1;
                        if written_bytes + line_bytes > max_bytes {
                            capped = true;
                            tracing::warn!(
                                "session_logger: file size cap ({} MiB) reached — stopping writes",
                                cfg.max_file_size_mb
                            );
                            continue;
                        }
                        if let Err(e) = writer.write_all(line.as_bytes()).await {
                            tracing::warn!("session_logger: write failed, stopping: {e}");
                            break;
                        }
                        if let Err(e) = writer.write_all(b"\n").await {
                            tracing::warn!("session_logger: write failed, stopping: {e}");
                            break;
                        }
                        written_bytes += line_bytes;
                        // Flush periodically — every 64 KiB of accumulated writes.
                        if written_bytes % (64 * 1024) < line_bytes {
                            let _ = writer.flush().await;
                        }
                    }
                    Err(BusError::Lagged(n)) => {
                        tracing::warn!("session_logger: lagged by {n} events — re-subscribing");
                        let (snap, new_sub) = bus.subscribe(SubscribeFilter::All);
                        sub = new_sub;
                        // Drain the replay snapshot so events between
                        // the lag point and re-subscribe are not lost.
                        if !capped {
                            for env in &snap {
                                let env = redact_envelope(env);
                                let line = match serde_json::to_string(&env) {
                                    Ok(l) => l,
                                    Err(_) => continue,
                                };
                                let line_bytes = line.len() as u64 + 1;
                                if written_bytes + line_bytes > max_bytes {
                                    capped = true;
                                    tracing::warn!(
                                        "session_logger: file size cap ({} MiB) reached during lag recovery",
                                        cfg.max_file_size_mb
                                    );
                                    break;
                                }
                                if writer.write_all(line.as_bytes()).await.is_err()
                                    || writer.write_all(b"\n").await.is_err()
                                {
                                    break;
                                }
                                written_bytes += line_bytes;
                            }
                            let _ = writer.flush().await;
                        }
                    }
                    Err(BusError::Closed) => break,
                }
            }
            _ = shutdown.notified() => {
                let _ = writer.flush().await;
                tracing::debug!("session_logger: shutdown — flushed and exiting");
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Category, Envelope};

    #[test]
    fn session_id_format_is_filesystem_safe() {
        let id = generate_session_id();
        assert!(!id.contains(':'), "colons not allowed in filenames: {id}");
        assert!(id.len() >= 19, "expected YYYY-MM-DD_HH-MM-SS: {id}");
        assert_eq!(&id[4..5], "-");
        assert_eq!(&id[7..8], "-");
        assert_eq!(&id[10..11], "_");
    }

    #[test]
    fn days_to_ymd_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_ymd_round_trip() {
        for days in [0i64, 1, 365, 730, 10000, 20000] {
            let (y, m, d) = days_to_ymd(days);
            let back = ymd_to_days(y, m, d);
            assert_eq!(
                days, back,
                "round-trip failed for days={days} → ({y},{m},{d})"
            );
        }
    }

    #[test]
    fn cleanup_skips_non_jsonl_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("not-a-session.txt"), "hello").unwrap();
        cleanup_old_sessions(dir.path(), 0);
        assert!(dir.path().join("not-a-session.txt").exists());
    }

    #[test]
    fn cleanup_deletes_old_sessions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("2020-01-01_00-00-00.jsonl"), "{}").unwrap();
        std::fs::write(dir.path().join("2099-12-31_23-59-59.jsonl"), "{}").unwrap();
        cleanup_old_sessions(dir.path(), 1);
        assert!(!dir.path().join("2020-01-01_00-00-00.jsonl").exists());
        assert!(dir.path().join("2099-12-31_23-59-59.jsonl").exists());
    }

    #[test]
    fn cleanup_reaps_summary_and_snapshot_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        // An old launch's full artifact set — all three must be reaped.
        for ext in ["jsonl", "summary.json", "snapshot.json"] {
            std::fs::write(dir.path().join(format!("2020-01-01_00-00-00.{ext}")), "{}").unwrap();
        }
        // A fresh launch's sidecars must survive.
        std::fs::write(dir.path().join("2099-12-31_23-59-59.summary.json"), "{}").unwrap();
        std::fs::write(dir.path().join("2099-12-31_23-59-59.snapshot.json"), "{}").unwrap();
        cleanup_old_sessions(dir.path(), 1);
        assert!(!dir.path().join("2020-01-01_00-00-00.summary.json").exists());
        assert!(!dir
            .path()
            .join("2020-01-01_00-00-00.snapshot.json")
            .exists());
        assert!(dir.path().join("2099-12-31_23-59-59.summary.json").exists());
        assert!(dir
            .path()
            .join("2099-12-31_23-59-59.snapshot.json")
            .exists());
    }

    #[test]
    fn redact_mcp_handshake_token() {
        let env = Envelope::new(Category::Mcp, "mcp.handshake")
            .with_payload(&serde_json::json!({
                "server": "test-server",
                "token": "super-secret-token-12345"
            }))
            .unwrap();
        let redacted = redact_envelope(&env);
        assert_eq!(redacted.payload["token"], "[REDACTED]");
        assert_eq!(redacted.payload["server"], "test-server");
    }

    #[test]
    fn redact_leaves_non_handshake_unchanged() {
        let env = Envelope::new(Category::Mcp, "mcp.invoke")
            .with_payload(&serde_json::json!({
                "tool": "read_file",
                "token": "should-stay"
            }))
            .unwrap();
        let redacted = redact_envelope(&env);
        assert_eq!(redacted.payload["token"], "should-stay");
    }

    #[tokio::test]
    async fn logger_writes_envelopes_to_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let bus = RiftBus::default();

        let session_id = generate_session_id();
        let file_path = dir.path().join(format!("{session_id}.jsonl"));

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .unwrap();

        let mut writer = tokio::io::BufWriter::new(file);
        let (_replay, mut sub) = bus.subscribe(SubscribeFilter::All);

        // Publish 3 envelopes.
        for i in 0..3 {
            bus.publish(Envelope::new(Category::System, format!("test.{i}")));
        }

        // Read them from the subscription and write to the file.
        for _ in 0..3 {
            let env = sub.recv().await.unwrap();
            let line = serde_json::to_string(&env).unwrap();
            writer.write_all(line.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
        }
        writer.flush().await.unwrap();

        // Verify the file has 3 lines, each a valid Envelope.
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            let env: Envelope = serde_json::from_str(line).unwrap();
            assert!(env.kind.starts_with("test."));
        }
    }
}
