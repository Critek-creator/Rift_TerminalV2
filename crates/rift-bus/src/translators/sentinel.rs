//! Sentinel translator — watches `~/.sentinel/events.jsonl` and publishes
//! `Category::Agent` / `kind="sentinel.*"` envelopes on the Rift bus.
//!
//! Pattern: file-tail translator (same family as `vault_walker` and the
//! aegis.log live-tail in rift-aegis). Reads new lines appended by the
//! Abyssal Sentinel CC hooks, parses each as JSON, wraps in an Envelope.
//!
//! Non-fatal on all paths — if the file doesn't exist or can't be watched,
//! the translator logs a warning and exits cleanly. Sentinel is optional.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Notify;

use crate::{Category, Envelope, RiftBus};

fn sentinel_events_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".sentinel").join("events.jsonl"))
}

/// Spawn the sentinel file-tail translator.
///
/// Watches `~/.sentinel/events.jsonl`. On each append, reads new lines from
/// the last-known byte offset, parses JSON, publishes `Category::Agent`
/// envelopes with `kind` taken from the event's `kind` field.
///
/// Shuts down when `shutdown` is notified (same pattern as status translator).
pub async fn spawn_sentinel_translator(bus: RiftBus, shutdown: Arc<Notify>) {
    let events_path = match sentinel_events_path() {
        Some(p) => p,
        None => {
            tracing::warn!("sentinel_translator: could not resolve home directory — skipped");
            return;
        }
    };

    if !events_path.parent().is_some_and(|p| p.exists()) {
        tracing::info!(
            "sentinel_translator: '{}' directory does not exist — sentinel not installed, skipping",
            events_path.parent().unwrap().display()
        );
        return;
    }

    let mut last_offset: u64 = match tokio::fs::metadata(&events_path).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    tracing::info!(
        "sentinel_translator: watching '{}' (offset {})",
        events_path.display(),
        last_offset
    );

    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("sentinel_translator: shutdown signal received");
                return;
            }
            _ = tokio::time::sleep(poll_interval) => {
                last_offset = read_and_publish(&bus, &events_path, last_offset).await;
            }
        }
    }
}

async fn read_and_publish(bus: &RiftBus, path: &PathBuf, offset: u64) -> u64 {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(_) => return offset,
    };

    let current_len = meta.len();
    if current_len <= offset {
        if current_len < offset {
            return current_len;
        }
        return offset;
    }

    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(_) => return offset,
    };

    let new_data = if (offset as usize) < bytes.len() {
        &bytes[offset as usize..]
    } else {
        return current_len;
    };

    let text = match std::str::from_utf8(new_data) {
        Ok(t) => t,
        Err(_) => return current_len,
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("sentinel_translator: skipping malformed line: {e}");
                continue;
            }
        };

        let kind = parsed
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("sentinel.violation");

        match Envelope::new(Category::Agent, kind).with_payload(&parsed) {
            Ok(env) => bus.publish(env),
            Err(e) => {
                tracing::debug!("sentinel_translator: envelope build failed: {e}");
            }
        }
    }

    current_len
}
