//! Agent translator — tails `%TEMP%/rift-agent-events.jsonl` and publishes
//! `Category::Agent` envelopes on the Rift bus.
//!
//! Pattern: file-tail translator (same family as `sentinel`). A CC hook
//! script (`tools/cc-agent-hook.mjs`) appends agent lifecycle events
//! (agent.start, agent.end) to the JSONL file. This translator polls
//! for new lines and publishes them as typed bus envelopes.
//!
//! Non-fatal on all paths — if the file doesn't exist, the translator
//! creates it so the hook script doesn't need to worry about ordering.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Notify;

use crate::{Category, Envelope, RiftBus};

fn agent_events_path() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("rift-agent-events.jsonl");
    dir
}

/// Spawn the agent file-tail translator.
///
/// Watches `%TEMP%/rift-agent-events.jsonl`. On each append, reads new
/// lines from the last-known byte offset, parses JSON, publishes
/// `Category::Agent` envelopes with `kind` taken from the event's
/// `kind` field.
///
/// Shuts down when `shutdown` is notified.
pub async fn spawn_agent_translator(bus: RiftBus, shutdown: Arc<Notify>) {
    let events_path = agent_events_path();

    // Create the file if it doesn't exist so the hook script can always
    // append without worrying about ordering.
    if !events_path.exists() {
        if let Err(e) = tokio::fs::write(&events_path, "").await {
            tracing::warn!(
                "agent_translator: failed to create '{}': {e}",
                events_path.display()
            );
        }
    }

    let mut last_offset: u64 = match tokio::fs::metadata(&events_path).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    tracing::info!(
        "agent_translator: watching '{}' (offset {})",
        events_path.display(),
        last_offset
    );

    let poll_interval = std::time::Duration::from_millis(300);

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("agent_translator: shutdown signal received");
                return;
            }
            _ = tokio::time::sleep(poll_interval) => {
                last_offset = read_and_publish(&bus, &events_path, last_offset).await;
            }
        }
    }
}

async fn read_and_publish(bus: &RiftBus, path: &std::path::Path, offset: u64) -> u64 {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(_) => return offset,
    };

    let current_len = meta.len();
    if current_len <= offset {
        if current_len < offset {
            // File was truncated — reset offset.
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
                tracing::debug!("agent_translator: skipping malformed line: {e}");
                continue;
            }
        };

        let kind = parsed
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("agent.activity");

        let payload = parsed.get("payload").cloned().unwrap_or(parsed.clone());

        match Envelope::new(Category::Agent, kind).with_payload(&payload) {
            Ok(env) => bus.publish(env),
            Err(e) => {
                tracing::debug!("agent_translator: envelope build failed: {e}");
            }
        }
    }

    current_len
}
