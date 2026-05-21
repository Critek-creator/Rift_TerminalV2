//! Cross-session event pattern comparison.
//!
//! Computes structural diffs between two session `.jsonl` files:
//! category frequency deltas, new/missing event types, error rate
//! changes, duration shifts, and per-minute timeline buckets for
//! sparkline overlay rendering.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;

use serde::Serialize;

use crate::config::sessions_dir;

/// Per-category event count.
#[derive(Debug, Clone, Serialize)]
pub struct CategoryCount {
    pub category: String,
    pub count: u64,
}

/// Summary statistics for a single session, computed from its `.jsonl` file.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub event_count: u64,
    pub error_count: u64,
    pub duration_ms: u64,
    pub category_counts: Vec<CategoryCount>,
    /// Distinct `category::kind` strings present in the session.
    pub event_types: Vec<String>,
    /// Per-minute event counts (index 0 = first minute of the session).
    /// Used for sparkline overlay rendering on the frontend.
    pub timeline_buckets: Vec<u64>,
}

/// A frequency delta for a single event category.
#[derive(Debug, Clone, Serialize)]
pub struct FrequencyDelta {
    pub category: String,
    pub baseline_count: u64,
    pub compare_count: u64,
    /// Signed difference: compare - baseline.
    pub delta: i64,
}

/// The full diff between a baseline and a comparison session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionDiff {
    pub baseline: SessionSummary,
    pub compare: SessionSummary,
    /// Event types present in compare but absent in baseline.
    pub new_types: Vec<String>,
    /// Event types present in baseline but absent in compare.
    pub missing_types: Vec<String>,
    /// Per-category frequency deltas, sorted by absolute delta descending.
    pub frequency_deltas: Vec<FrequencyDelta>,
    /// Signed error count difference: compare.error_count - baseline.error_count.
    pub error_delta: i64,
    /// Signed duration difference in ms: compare.duration_ms - baseline.duration_ms.
    pub duration_delta: i64,
}

/// Bucket granularity: 60_000 ms = 1 minute.
const BUCKET_MS: u64 = 60_000;

/// Build a [`SessionSummary`] from a session's `.jsonl` file.
fn summarise_session(session_id: &str) -> Result<SessionSummary, String> {
    let dir = sessions_dir().map_err(|e| format!("sessions dir: {e}"))?;
    let path = dir.join(format!("{session_id}.jsonl"));

    if !path.exists() {
        return Err(format!("session file not found: {session_id}"));
    }

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("failed to open session {session_id}: {e}"))?;

    let reader = std::io::BufReader::new(file);

    let mut event_count: u64 = 0;
    let mut error_count: u64 = 0;
    let mut first_ts: Option<u64> = None;
    let mut last_ts: u64 = 0;
    let mut category_counts: HashMap<String, u64> = HashMap::new();
    let mut event_types: HashSet<String> = HashSet::new();
    let mut timestamps: Vec<u64> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let val: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        event_count += 1;

        let category = val
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let kind = val
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let ts = val.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);

        // Track error events (Category::System with kind containing "error").
        if category == "system" && kind.contains("error") {
            error_count += 1;
        }

        // Track timestamps for timeline + duration.
        if ts > 0 {
            if first_ts.is_none() {
                first_ts = Some(ts);
            }
            if ts > last_ts {
                last_ts = ts;
            }
            timestamps.push(ts);
        }

        *category_counts.entry(category.clone()).or_insert(0) += 1;
        event_types.insert(format!("{category}::{kind}"));
    }

    let duration_ms = match first_ts {
        Some(first) if last_ts >= first => last_ts - first,
        _ => 0,
    };

    // Build per-minute timeline buckets.
    let timeline_buckets = if let Some(base_ts) = first_ts {
        let bucket_count = if duration_ms == 0 {
            1
        } else {
            ((duration_ms / BUCKET_MS) + 1) as usize
        };
        // Cap at 1440 buckets (24 hours) to bound memory.
        let bucket_count = bucket_count.min(1440);
        let mut buckets = vec![0u64; bucket_count];
        for &ts in &timestamps {
            let offset = ts.saturating_sub(base_ts);
            let idx = (offset / BUCKET_MS) as usize;
            if idx < buckets.len() {
                buckets[idx] += 1;
            }
        }
        buckets
    } else {
        vec![]
    };

    let mut cat_vec: Vec<CategoryCount> = category_counts
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect();
    cat_vec.sort_by_key(|a| std::cmp::Reverse(a.count));

    let mut types_vec: Vec<String> = event_types.into_iter().collect();
    types_vec.sort();

    Ok(SessionSummary {
        session_id: session_id.to_string(),
        event_count,
        error_count,
        duration_ms,
        category_counts: cat_vec,
        event_types: types_vec,
        timeline_buckets,
    })
}

/// Compare two sessions and return a [`SessionDiff`].
///
/// `baseline_id` and `compare_id` are session file stems (without `.jsonl`).
pub fn compare_sessions(baseline_id: &str, compare_id: &str) -> Result<SessionDiff, String> {
    let baseline = summarise_session(baseline_id)?;
    let compare = summarise_session(compare_id)?;

    // Compute new/missing event types.
    let baseline_types: HashSet<&str> = baseline.event_types.iter().map(|s| s.as_str()).collect();
    let compare_types: HashSet<&str> = compare.event_types.iter().map(|s| s.as_str()).collect();

    let mut new_types: Vec<String> = compare_types
        .difference(&baseline_types)
        .map(|s| s.to_string())
        .collect();
    new_types.sort();

    let mut missing_types: Vec<String> = baseline_types
        .difference(&compare_types)
        .map(|s| s.to_string())
        .collect();
    missing_types.sort();

    // Build frequency deltas per category.
    let baseline_map: HashMap<&str, u64> = baseline
        .category_counts
        .iter()
        .map(|c| (c.category.as_str(), c.count))
        .collect();
    let compare_map: HashMap<&str, u64> = compare
        .category_counts
        .iter()
        .map(|c| (c.category.as_str(), c.count))
        .collect();

    let mut all_cats: HashSet<&str> = HashSet::new();
    all_cats.extend(baseline_map.keys());
    all_cats.extend(compare_map.keys());

    let mut frequency_deltas: Vec<FrequencyDelta> = all_cats
        .into_iter()
        .map(|cat| {
            let bc = *baseline_map.get(cat).unwrap_or(&0);
            let cc = *compare_map.get(cat).unwrap_or(&0);
            FrequencyDelta {
                category: cat.to_string(),
                baseline_count: bc,
                compare_count: cc,
                delta: cc as i64 - bc as i64,
            }
        })
        .collect();
    frequency_deltas.sort_by_key(|a| std::cmp::Reverse(a.delta.unsigned_abs()));

    let error_delta = compare.error_count as i64 - baseline.error_count as i64;
    let duration_delta = compare.duration_ms as i64 - baseline.duration_ms as i64;

    Ok(SessionDiff {
        baseline,
        compare,
        new_types,
        missing_types,
        frequency_deltas,
        error_delta,
        duration_delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarise_nonexistent_session_errors() {
        let result = summarise_session("nonexistent-session-id-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn compare_nonexistent_sessions_errors() {
        let result = compare_sessions("nonexistent-a", "nonexistent-b");
        assert!(result.is_err());
    }
}
