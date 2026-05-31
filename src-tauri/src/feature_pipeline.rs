// Wave 6c — Feature Pipeline tab backend.
//
// Read-only scan of the Abyssal Feature Agent idea store so the cockpit can
// surface the studio's feature pipeline (per-project counts, tier breakdown,
// leverage ranking) at a glance. §9-compliant: this is a plain `std::fs` read
// of a LOCAL store, not a call to any external system — the same boundary
// posture as `todo_scan.rs` and `index_bridge.rs` (the translator-boundary
// check only forbids tokio::net / reqwest / SDK-prefixes outside translators).
//
// Source of truth mirrors render-report.sh: candidates/ holds the authoritative
// originals stamped with `curated_tier` (null = untriaged); curated/graveyard/
// holds buried entries. We scan both so every project's full funnel is visible.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;

/// Hard cap so a huge store can't flood the IPC channel.
const MAX_ENTRIES: usize = 4000;

static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Resolve the feature-agent store root. Mirrors index_bridge.rs::vault_path —
/// env override first, then known candidate locations.
fn store_path() -> &'static PathBuf {
    STORE_PATH.get_or_init(|| {
        if let Ok(p) = std::env::var("FEATURE_AGENT_STORE") {
            return PathBuf::from(p);
        }
        let candidates = [directories::BaseDirs::new().map(|b| {
            b.home_dir()
                .join("Documents/Abyssal_Arts_main/idea-forge/abyssal-feature-agent")
        })];
        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                return candidate;
            }
        }
        PathBuf::from("abyssal-feature-agent")
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineEntry {
    pub id: String,
    pub title: String,
    /// Project slug (`p001`, `p006`, `aegis`, …).
    pub target: String,
    /// Funnel stage: `untriaged` | `low` | `medium` | `high` | `critical` | `buried`.
    pub stage: String,
    /// Wave-4 decision-support fields — None on legacy candidates.
    pub value_class: Option<String>,
    pub effort: Option<String>,
    pub impact: Option<String>,
    pub occurrence: u32,
    /// Whether the candidate carries a `promoted_to_plan` stamp.
    pub promoted: bool,
    /// `first_seen` ISO date (yyyy-mm-dd slice), best-effort.
    pub first_seen: Option<String>,
}

/// Scan the store. Returns at most [`MAX_ENTRIES`] entries.
pub fn scan_store(root: &Path) -> Vec<PipelineEntry> {
    let mut out: Vec<PipelineEntry> = Vec::new();
    // candidates/ — originals stamped with curated_tier (null → untriaged).
    scan_dir(&root.join("candidates"), None, &mut out);
    // curated/graveyard/ — buried.
    scan_dir(
        &root.join("curated").join("graveyard"),
        Some("buried"),
        &mut out,
    );
    out.sort_by(|a, b| a.target.cmp(&b.target).then(a.id.cmp(&b.id)));
    out
}

fn scan_dir(dir: &Path, force_stage: Option<&str>, out: &mut Vec<PipelineEntry>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return, // store or subdir absent — fine, empty contribution
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c.replace("\r\n", "\n"),
            Err(_) => continue,
        };
        if let Some(e) = parse_entry(&content, &path, force_stage) {
            out.push(e);
        }
    }
}

/// Parse a candidate's leading YAML frontmatter into a PipelineEntry.
fn parse_entry(content: &str, path: &Path, force_stage: Option<&str>) -> Option<PipelineEntry> {
    let fm = frontmatter(content)?;
    let id = field(&fm, "id").unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });
    let title = field(&fm, "title").unwrap_or_else(|| "(untitled)".to_string());
    let target = field(&fm, "target").unwrap_or_else(|| "unknown".to_string());

    let tier = field(&fm, "curated_tier");
    let stage = match force_stage {
        Some(s) => s.to_string(),
        None => match tier.as_deref() {
            Some("critical") => "critical",
            Some("high") => "high",
            Some("medium") => "medium",
            Some("low") => "low",
            _ => "untriaged", // null / absent
        }
        .to_string(),
    };

    let occurrence = field(&fm, "occurrence_count")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    let promoted = matches!(field(&fm, "promoted_to_plan").as_deref(), Some(v) if v != "null" && !v.is_empty());
    let first_seen =
        field(&fm, "first_seen").map(|s| s.split('T').next().unwrap_or(&s).to_string());

    Some(PipelineEntry {
        id,
        title,
        target,
        stage,
        value_class: nonnull(field(&fm, "value_class")),
        effort: nonnull(field(&fm, "effort")),
        impact: nonnull(field(&fm, "impact")),
        occurrence,
        promoted,
        first_seen,
    })
}

/// Treat the literal YAML `null` (and empty) as None.
fn nonnull(v: Option<String>) -> Option<String> {
    match v {
        Some(s) if s != "null" && !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Return the leading `---\n … \n---` frontmatter block, or None.
fn frontmatter(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(rest[..end].to_string())
}

/// Read a top-level scalar `key: value` from the frontmatter block. Strips
/// wrapping quotes. Ignores nested/indented lines (so block maps like
/// reason_counts don't leak through).
fn field(fm: &str, key: &str) -> Option<String> {
    for line in fm.lines() {
        // top-level only — no leading whitespace
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix(':') {
                let val = val.trim();
                let val = val.trim_matches('"').trim_matches('\'').trim();
                if val.is_empty() {
                    return None;
                }
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Tauri command — async + spawn_blocking so the fs walk never blocks the
/// command thread. Returns the full entry list; the frontend aggregates.
#[tauri::command]
pub async fn feature_pipeline_scan() -> Result<Vec<PipelineEntry>, String> {
    tokio::task::spawn_blocking(|| Ok(scan_store(store_path())))
        .await
        .map_err(|e| format!("feature_pipeline_scan: task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_candidate() {
        let md = "---\nid: 2026-05-31-x-abc\ntitle: \"A feature\"\ntarget: p006\ncurated_tier: high\noccurrence_count: 2\nvalue_class: quick-win\neffort: S\nimpact: high\npromoted_to_plan: null\nfirst_seen: 2026-05-31T12:00:00Z\nreason_counts:\n  \"r\": 1\n---\n## Description\n";
        let e = parse_entry(md, Path::new("x.md"), None).expect("parses");
        assert_eq!(e.target, "p006");
        assert_eq!(e.stage, "high");
        assert_eq!(e.occurrence, 2);
        assert_eq!(e.value_class.as_deref(), Some("quick-win"));
        assert_eq!(e.impact.as_deref(), Some("high"));
        assert!(!e.promoted);
        assert_eq!(e.first_seen.as_deref(), Some("2026-05-31"));
    }

    #[test]
    fn null_fields_become_none() {
        let md =
            "---\nid: y\ntitle: t\ntarget: p001\ncurated_tier: null\nvalue_class: null\n---\nbody";
        let e = parse_entry(md, Path::new("y.md"), None).expect("parses");
        assert_eq!(e.stage, "untriaged");
        assert!(e.value_class.is_none());
    }

    #[test]
    fn force_stage_overrides() {
        let md = "---\nid: z\ntitle: t\ntarget: p001\ncurated_tier: low\n---\nb";
        let e = parse_entry(md, Path::new("z.md"), Some("buried")).expect("parses");
        assert_eq!(e.stage, "buried");
    }
}
