//! Portfolio health collector — periodic background task that gathers
//! vault staleness, sentinel violations, and git status for every Abyssal
//! Arts project, then publishes the aggregate as a `Category::System /
//! kind="health.portfolio"` envelope on the bus.
//!
//! §9 translator boundary: this module uses only `std::fs` and
//! `std::process::Command` — no external crate dependencies beyond what
//! the host crate already has (serde, rift-bus, tokio). No `index-core`,
//! no `git2`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use rift_bus::{Category, Envelope, RiftBus};
use serde::Serialize;
use tokio::sync::Notify;

/// Collection interval — one snapshot every 60 seconds.
const COLLECT_INTERVAL: Duration = Duration::from_secs(60);

/// Windows `CREATE_NO_WINDOW`. Mirrors the constant in `git_status.rs` and
/// `crates/rift-bus/src/translators/status.rs` — every `Command::spawn` of
/// `git.exe` from this crate must apply it, otherwise each status probe
/// flashes a visible console window on Windows.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct ProjectHealth {
    name: String,
    code: String,
    vault_path: String,
    vault_staleness_days: f64,
    sentinel_violations: ViolationCounts,
    git: Option<GitStatus>,
}

#[derive(Debug, Clone, Serialize)]
struct ViolationCounts {
    critical: u32,
    warning: u32,
    info: u32,
}

#[derive(Debug, Clone, Serialize)]
struct GitStatus {
    branch: String,
    ahead: u32,
    behind: u32,
    uncommitted: u32,
}

#[derive(Debug, Clone, Serialize)]
struct HealthPayload {
    projects: Vec<ProjectHealth>,
    collected_at: String,
}

// ---------------------------------------------------------------------------
// Hardcoded project registry (Abyssal Arts portfolio)
// ---------------------------------------------------------------------------

struct ProjectDef {
    name: &'static str,
    code: &'static str,
    repo: &'static str,
    vault: &'static str,
}

const PROJECTS: &[ProjectDef] = &[
    ProjectDef {
        name: "Brain Dump",
        code: "BD",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/Brain_Dump",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p001.md",
    },
    ProjectDef {
        name: "Anchorage",
        code: "ANC",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/Anchor_App_V2",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p002.md",
    },
    ProjectDef {
        name: "Abyssal IDE",
        code: "AIDE",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/Abyssal_ClaudeCode_IDE",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p003.md",
    },
    ProjectDef {
        name: "Aethergard",
        code: "ATH",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/abyss_masters",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p004.md",
    },
    ProjectDef {
        name: "Website",
        code: "WEB",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/AbyssalArts_Website",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p005.md",
    },
    ProjectDef {
        name: "Rift Terminal",
        code: "RIFT",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/Rift_TerminalV2",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p006.md",
    },
    ProjectDef {
        name: "Sentinel",
        code: "SENT",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/Abyssal_Sentinel",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p007.md",
    },
    ProjectDef {
        name: "Index",
        code: "INDEX",
        repo: "C:/Users/Critek/Documents/Abyssal_Arts_main/Projects/abyssal-index",
        vault: "C:/Users/Critek/.claude/abyssal-index/vaults/p008.md",
    },
];

// ---------------------------------------------------------------------------
// Sentinel flags path
// ---------------------------------------------------------------------------

/// Resolve the sentinel flags log path. Uses `directories::BaseDirs` for
/// cross-platform home resolution (same pattern as the sentinel translator
/// in `crates/rift-bus/src/translators/sentinel.rs`).
fn sentinel_flags_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| {
        b.home_dir()
            .join(".claude")
            .join("sentinel")
            .join("flags.log")
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Spawn the periodic health collector as a background tokio task.
///
/// Runs every [`COLLECT_INTERVAL`] seconds. Publishes a
/// `Category::System / kind="health.portfolio"` envelope with the
/// aggregated [`HealthPayload`]. Respects the `shutdown` notify so it
/// exits promptly on app teardown (same pattern as
/// `spawn_status_translator` and `spawn_sentinel_translator`).
///
/// All I/O errors are logged to `tracing::warn` and swallowed — this is
/// a background enhancer, never critical path.
pub async fn spawn_health_collector(bus: RiftBus, shutdown: Arc<Notify>) {
    // Initial delay — let the app finish booting before the first
    // collection pass. Matches the pattern used by the status and
    // sentinel translators (500ms–1s boot delay).
    tokio::time::sleep(Duration::from_secs(2)).await;

    loop {
        // Collect on a blocking thread — git subprocess spawns and
        // filesystem stat calls can block for hundreds of ms.
        let payload = match tokio::task::spawn_blocking(collect_health).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("health_collector: spawn_blocking join error: {e}");
                tokio::time::sleep(COLLECT_INTERVAL).await;
                continue;
            }
        };

        match Envelope::new(Category::System, "health.portfolio").with_payload(&payload) {
            Ok(env) => bus.publish(env),
            Err(e) => tracing::warn!("health_collector: envelope build failed: {e}"),
        }

        // Sleep until next collection, or exit on shutdown.
        tokio::select! {
            _ = tokio::time::sleep(COLLECT_INTERVAL) => {},
            _ = shutdown.notified() => {
                tracing::info!("health_collector: shutdown signal received");
                break;
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Collection logic (runs on a blocking thread)
// ---------------------------------------------------------------------------

fn collect_health() -> HealthPayload {
    let sentinel_counts = sentinel_flags_path()
        .map(|p| sentinel_violations(&p))
        .unwrap_or(ViolationCounts {
            critical: 0,
            warning: 0,
            info: 0,
        });

    let projects: Vec<ProjectHealth> = PROJECTS
        .iter()
        .map(|def| {
            let vault_path = PathBuf::from(def.vault);
            let repo_path = PathBuf::from(def.repo);

            ProjectHealth {
                name: def.name.to_string(),
                code: def.code.to_string(),
                vault_path: def.vault.to_string(),
                vault_staleness_days: vault_staleness(&vault_path),
                // All projects share the single sentinel flags file —
                // per-project filtering would require parsing log content
                // for project identifiers, which v1 does not do. Each
                // project gets the global counts; the UI can note this.
                sentinel_violations: sentinel_counts.clone(),
                git: git_status(&repo_path),
            }
        })
        .collect();

    // Use SystemTime for the timestamp (no chrono dependency in this crate).
    let collected_at = system_time_to_rfc3339(SystemTime::now());

    HealthPayload {
        projects,
        collected_at,
    }
}

/// Compute days since the vault file was last modified.
///
/// Returns `-1.0` on any error (file missing, permission denied, clock
/// skew). The frontend can treat negative values as "unknown".
fn vault_staleness(vault_path: &Path) -> f64 {
    let meta = match std::fs::metadata(vault_path) {
        Ok(m) => m,
        Err(_) => return -1.0,
    };
    let modified = match meta.modified() {
        Ok(t) => t,
        Err(_) => return -1.0,
    };
    let elapsed = match SystemTime::now().duration_since(modified) {
        Ok(d) => d,
        Err(_) => return -1.0, // clock skew — modified is in the future
    };
    elapsed.as_secs_f64() / 86_400.0
}

/// Count critical / warning / info lines in the sentinel flags log.
///
/// Simple line-by-line case-insensitive substring search. Returns all
/// zeros if the file does not exist or cannot be read.
fn sentinel_violations(flags_path: &Path) -> ViolationCounts {
    let content = match std::fs::read_to_string(flags_path) {
        Ok(c) => c,
        Err(_) => {
            return ViolationCounts {
                critical: 0,
                warning: 0,
                info: 0,
            }
        }
    };

    let mut critical = 0u32;
    let mut warning = 0u32;
    let mut info = 0u32;

    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.contains("critical") {
            critical += 1;
        } else if lower.contains("warning") {
            warning += 1;
        } else if lower.contains("info") {
            info += 1;
        }
    }

    ViolationCounts {
        critical,
        warning,
        info,
    }
}

/// Gather git status for a repository path.
///
/// Returns `None` if the directory does not exist or any git command fails
/// (e.g. not a git repo). Never panics.
fn git_status(repo_path: &Path) -> Option<GitStatus> {
    if !repo_path.is_dir() {
        return None;
    }

    let branch = run_git(repo_path, &["branch", "--show-current"]).unwrap_or_default();

    let uncommitted = run_git(repo_path, &["status", "--porcelain"])
        .map(|out| out.lines().filter(|l| !l.is_empty()).count() as u32)
        .unwrap_or(0);

    let (ahead, behind) = run_git(
        repo_path,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    )
    .and_then(|out| parse_ahead_behind(&out))
    .unwrap_or((0, 0));

    Some(GitStatus {
        branch: branch.trim().to_string(),
        ahead,
        behind,
        uncommitted,
    })
}

/// Run a git command with `-C <repo_path>` and return stdout as a String.
///
/// Applies `CREATE_NO_WINDOW` on Windows to suppress console flashes
/// (same pattern as `src-tauri/src/git_status.rs::run_git`).
fn run_git(repo_path: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_path);
    for a in args {
        cmd.arg(a);
    }

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Parse `git rev-list --left-right --count HEAD...@{u}` output.
///
/// Expected format: `<ahead>\t<behind>\n`.
fn parse_ahead_behind(raw: &str) -> Option<(u32, u32)> {
    let trimmed = raw.trim();
    let mut parts = trimmed.split('\t');
    let ahead = parts.next()?.parse::<u32>().ok()?;
    let behind = parts.next()?.parse::<u32>().ok()?;
    Some((ahead, behind))
}

/// Format a `SystemTime` as an RFC 3339 timestamp without pulling in chrono.
///
/// Produces a UTC string like `2026-05-26T14:30:00Z`. Sub-second precision
/// is omitted for readability. On platforms where `UNIX_EPOCH` arithmetic
/// fails, returns `"unknown"`.
fn system_time_to_rfc3339(t: SystemTime) -> String {
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return "unknown".to_string(),
    };

    // Manual UTC calendar arithmetic (no leap seconds — matches libc/gmtime).
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01 → year/month/day.
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since Unix epoch to (year, month, day).
///
/// Uses the civil calendar algorithm (proleptic Gregorian). Matches the
/// POSIX `gmtime` decomposition used by chrono/time crates internally.
fn days_to_ymd(days_since_epoch: u64) -> (u32, u32, u32) {
    // Algorithm from Howard Hinnant's `civil_from_days`.
    let z = days_since_epoch as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_staleness_missing_file() {
        let result = vault_staleness(Path::new("C:/nonexistent/vault.md"));
        assert!(
            result < 0.0,
            "missing file should return -1.0, got {result}"
        );
    }

    #[test]
    fn sentinel_violations_missing_file() {
        let result = sentinel_violations(Path::new("C:/nonexistent/flags.log"));
        assert_eq!(result.critical, 0);
        assert_eq!(result.warning, 0);
        assert_eq!(result.info, 0);
    }

    #[test]
    fn parse_ahead_behind_valid() {
        assert_eq!(parse_ahead_behind("3\t5\n"), Some((3, 5)));
        assert_eq!(parse_ahead_behind("0\t0\n"), Some((0, 0)));
        assert_eq!(parse_ahead_behind("12\t7"), Some((12, 7)));
    }

    #[test]
    fn parse_ahead_behind_invalid() {
        assert_eq!(parse_ahead_behind(""), None);
        assert_eq!(parse_ahead_behind("abc"), None);
        assert_eq!(parse_ahead_behind("1"), None);
    }

    #[test]
    fn git_status_nonexistent_dir() {
        let result = git_status(Path::new("C:/nonexistent_repo_dir"));
        assert!(result.is_none(), "nonexistent dir should return None");
    }

    #[test]
    fn system_time_to_rfc3339_known_epoch() {
        // 2026-01-01T00:00:00Z = 1_767_225_600 seconds since epoch
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_225_600);
        let s = system_time_to_rfc3339(t);
        assert_eq!(s, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn system_time_to_rfc3339_epoch() {
        let s = system_time_to_rfc3339(SystemTime::UNIX_EPOCH);
        assert_eq!(s, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn days_to_ymd_epoch() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_known_date() {
        // 2026-05-26 is day 20_599 since epoch.
        // 56 full years (1970–2025) = 56*365 + 14 leap days = 20454.
        // Jan 31 + Feb 28 + Mar 31 + Apr 30 + May 26 = 146 days into year,
        // but Jan 1 is day 0, so offset = 145.  20454 + 145 = 20599.
        assert_eq!(days_to_ymd(20_599), (2026, 5, 26));
    }

    #[test]
    fn days_to_ymd_leap_year() {
        // 2024-02-29 (leap day)
        // Days from 1970-01-01 to 2024-02-29:
        // 54 years (1970-2023), 13 leap years in [1972..2020 step 4]
        // = 54*365 + 13 = 19710 + 13 = 19723
        // + Jan 31 + Feb 29 = 60 → 19723 + 59 = 19782
        assert_eq!(days_to_ymd(19_782), (2024, 2, 29));
    }

    #[test]
    fn collect_health_does_not_panic() {
        // Smoke test: the collection function should never panic,
        // regardless of whether the paths exist on this machine.
        let payload = collect_health();
        assert_eq!(payload.projects.len(), PROJECTS.len());
        assert!(!payload.collected_at.is_empty());
        for p in &payload.projects {
            assert!(!p.name.is_empty());
            assert!(!p.code.is_empty());
        }
    }
}
