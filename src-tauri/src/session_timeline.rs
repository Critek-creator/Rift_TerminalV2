//! Tauri command wrapper for the cross-store session timeline.
//!
//! The pure merge/filter/sort/enrich logic lives in
//! `rift_bus::session_reader::build_timeline` (stateless, unit-tested).
//! This module owns only the Tauri boundary: resolving `CommandHistoryStore`
//! state, narrowing history records to the pane ids seen in the session log,
//! and bridging into `spawn_blocking`.
//!
//! § Boundary rule: no external calls outside crates/rift-bus/src/translators/.
//! This file is pure Tauri-state → pure-fn plumbing; all I/O is in rift-bus.

use rift_bus::session_reader::{HistoryRecord, TimelineEntry, TimelineSources};

use crate::command_history::CommandHistoryStore;

/// Parse an ISO 8601 / RFC 3339 datetime string (UTC) into unix milliseconds.
///
/// Handles the two forms written by the command-history hook:
/// - `2026-05-22T10:00:00Z`
/// - `2026-05-22T10:00:00+00:00`
///
/// Returns `None` on any parse failure so callers can degrade gracefully.
/// We avoid a `chrono` dependency by doing the arithmetic directly.
fn parse_iso8601_to_ms(s: &str) -> Option<u64> {
    // Strip trailing offset — accept "Z", "+00:00", or no offset (assume UTC).
    let core = s
        .trim_end_matches('Z')
        .trim_end_matches("+00:00")
        .trim_end_matches("-00:00");

    // Expected format after stripping: "YYYY-MM-DDTHH:MM:SS"
    // Allow either 'T' or ' ' as the date/time separator.
    let (date_part, time_part) = if let Some(t_pos) = core.find('T') {
        (&core[..t_pos], &core[t_pos + 1..])
    } else if let Some(sp_pos) = core.find(' ') {
        (&core[..sp_pos], &core[sp_pos + 1..])
    } else {
        return None;
    };

    let mut date_parts = date_part.splitn(3, '-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u64 = date_parts.next()?.parse().ok()?;
    let day: u64 = date_parts.next()?.parse().ok()?;

    let mut time_parts = time_part.splitn(3, ':');
    let hour: u64 = time_parts.next()?.parse().ok()?;
    let minute: u64 = time_parts.next()?.parse().ok()?;
    // Seconds may have a fractional part — take only the integer portion.
    let sec_str = time_parts.next().unwrap_or("0");
    let sec: u64 = sec_str.split('.').next()?.parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Days since Unix epoch (1970-01-01). Simple Gregorian calculation — good
    // enough for timestamps in the 2020s; no leap-second handling needed.
    let days = days_since_epoch(year, month, day)?;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + sec;
    Some(secs * 1_000)
}

/// Days from 1970-01-01 to the given (year, month, day) in the proleptic
/// Gregorian calendar. Returns `None` if the date looks implausible.
fn days_since_epoch(year: i64, month: u64, day: u64) -> Option<u64> {
    // Month lengths (non-leap year).
    const MLEN: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let feb_days: u64 = if is_leap { 29 } else { 28 };

    let mut days_in_year: u64 = 0;
    for m in 1..(month as usize) {
        days_in_year += if m == 2 { feb_days } else { MLEN[m - 1] };
    }
    days_in_year += day - 1;

    // Days from 1970 to start of `year`.
    let y = year - 1970;
    if y < 0 {
        return None; // Pre-epoch timestamps not expected in command history.
    }
    let leap_years = (y / 4) - (y / 100) + (y / 400);
    let days_from_epoch = (y as u64) * 365 + leap_years as u64 + days_in_year;
    Some(days_from_epoch)
}

/// Async Tauri command — merges command_history + per-session .jsonl events into
/// a chronological, source-filtered, exit-code-enriched timeline.
///
/// The `sources` parameter is forwarded directly from the frontend's
/// `config.timeline` field — shapes are identical (same 7 snake_case bools)
/// so it deserializes 1:1. This keeps `build_timeline` stateless and
/// config-free; the frontend is the natural holder of the live config.
///
/// `limit` caps the returned entries (default 2000, most-recent after sort).
#[tauri::command]
pub async fn session_timeline(
    session_id: String,
    sources: TimelineSources,
    limit: Option<usize>,
    store: tauri::State<'_, CommandHistoryStore>,
) -> Result<Vec<TimelineEntry>, String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        // Ensure the history store is loaded from disk.
        store.ensure_loaded()?;

        // Pre-scan the session log once to collect the distinct pane ids seen
        // in command.submitted envelopes — we only need history for those panes.
        // This avoids scanning all 10 k records per command by narrowing up-front.
        let envelopes = rift_bus::session_reader::load_session(&session_id)?;

        let mut pane_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for env in &envelopes {
            let cat = env.get("category").and_then(|v| v.as_str()).unwrap_or("");
            let kind = env.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if cat == "pty" && kind == "command.submitted" {
                if let Some(pid) = env
                    .get("payload")
                    .and_then(|p| p.get("session_id"))
                    .and_then(|v| v.as_i64())
                {
                    pane_ids.insert(pid);
                }
            }
        }

        // Gather command-history records for only the relevant pane ids.
        let history_records: Vec<HistoryRecord> = {
            let records = store.records_snapshot();
            records
                .into_iter()
                .filter(|r| {
                    r.session_id
                        .map(|pid| pane_ids.contains(&pid))
                        .unwrap_or(false)
                })
                .map(|r| {
                    // Parse started_at (ISO 8601 / RFC 3339) into unix ms for
                    // the proximity join. We don't depend on `chrono` in this
                    // crate, so use a lightweight hand-rolled parser that handles
                    // the common `YYYY-MM-DDTHH:MM:SSZ` and
                    // `YYYY-MM-DDTHH:MM:SS+00:00` forms written by the hook.
                    // On parse failure we fall back to `None`; the join still
                    // works (it picks the first matching record rather than the
                    // temporally closest one).
                    let started_at_ms = parse_iso8601_to_ms(&r.started_at);
                    HistoryRecord {
                        session_id: r.session_id,
                        command: r.command,
                        started_at_ms,
                        exit_code: r.exit_code,
                        duration_ms: r.duration_ms,
                    }
                })
                .collect()
        };

        rift_bus::session_reader::build_timeline(&session_id, &sources, &history_records, limit)
    })
    .await
    .map_err(|e| format!("session_timeline: task join error: {e}"))?
}
