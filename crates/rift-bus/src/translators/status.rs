//! Status translator — publishes `Category::Status` envelopes every 5 seconds.
//!
//! D-012: `DIR`, `GIT`, and `REPO` segments are live via git polling.
//! `MODEL`, `CTX%`, `SESSION USE%`, `WEEK%` are sourced from Claude Code's
//! StatusJSON, written to `$TEMP/rift-cc-status.json` by the bridge script
//! `tools/cc-status-bridge.mjs`.
//!
//! ## §9 Boundary note
//!
//! ALL git subprocess invocations, home-dir resolution, and CC status file
//! reads live exclusively in this file. Nothing leaks to rift-bus core.
//! `check-translator-boundary.sh` enforces the boundary.
//!
//! ## Kind taxonomy
//!
//! One kind is emitted under `Category::Status`:
//!
//! | kind      | trigger                    |
//! |-----------|----------------------------|
//! | `"usage"` | 5-second polling tick      |
//!
//! Adding further kinds is additive and does NOT bump `CURRENT_VERSION`
//! (per `envelope-version-additive-categories-no-bump`).
//!
//! ## Payload shape
//!
//! ```json
//! {
//!   "dir":  "~/Documents/Abyssal_Arts_main/Projects/Rift_TerminalV2",
//!   "git":  "main*",
//!   "repo": "Rift_TerminalV2",
//!   "ts":   1714262400000,
//!   "model": "claude-opus-4-6[1m]",
//!   "ctx_pct": 42,
//!   "session_use_pct": 30,
//!   "week_pct": 65,
//!   "github_owner": "Critek-creator",
//!   "github_repo": "Rift_TerminalV2",
//!   "skill": "aegis",
//!   "thinking": "high"
//! }
//! ```
//!
//! `dir` is tilde-collapsed (home dir replaced with `~`). `git` appends `*`
//! when the working tree is dirty. Falls back to `—` on any failure so
//! callers never need to handle absent fields — the segments always have a
//! displayable value. CC-sourced fields are absent when the bridge has not
//! yet written a file or the file is stale (>30 s).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde_json::json;
use tokio::sync::Notify;
use tokio::time::{interval, Duration};

use crate::{Category, Envelope, RiftBus};

/// Windows `CREATE_NO_WINDOW` process-creation flag. Suppresses the console
/// window that would otherwise flash on every `Command::spawn` of a console
/// subsystem child (`git.exe`, `cmd.exe`). Without this flag, the 5-second
/// status-translator tick paints a visible terminal flash on every poll.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// Public spawn entry point
// ---------------------------------------------------------------------------

/// Run the status translator loop.
///
/// Publishes a `Category::Status / kind="usage"` envelope to `bus` every 5
/// seconds. The envelope payload carries `{ dir, git, repo, ts }` computed
/// from `project_root` — see the module-level docs for the exact semantics.
///
/// This is an `async fn` — callers MUST wrap it in `tauri::async_runtime::spawn`
/// (or equivalent) per the Phase 7.1 setup() pattern. Tauri 2 owns its async
/// runtime; calling `tokio::spawn` from inside this crate would panic
/// (`there is no reactor running`) since rift-bus runs inside the Tauri main
/// thread, which does not have a freestanding tokio reactor active.
///
/// The loop runs until the Tauri process exits, the spawned task is aborted,
/// or `shutdown.notify_waiters()` is called by the host's `RunEvent::ExitRequested`
/// handler — whichever comes first. The shutdown signal is what stops the loop
/// from continuing to spawn `git.exe` children after the main window closes
/// (which would otherwise paint visible terminal flashes until the process is
/// force-killed; see the Windows-only comments around `CREATE_NO_WINDOW`).
///
/// # §9 boundary
///
/// All git subprocess invocations and home-dir lookups are confined to this
/// function and its private helpers. No external-system calls escape to
/// rift-bus core.
pub async fn spawn_status_translator(
    bus: RiftBus,
    project_root: Arc<Mutex<PathBuf>>,
    shutdown: Arc<Notify>,
) {
    let mut tick = interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let bus_clone = bus.clone();
                let root_clone = project_root.lock().clone();
                let _ = tokio::task::spawn_blocking(move || {
                    publish_status_snapshot(&bus_clone, &root_clone);
                }).await;
            }
            _ = shutdown.notified() => {
                tracing::info!("status-translator: shutdown signal received, exiting loop");
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot publisher (pub(crate) for tests)
// ---------------------------------------------------------------------------

/// Compute and publish one `Category::Status / kind="usage"` envelope.
///
/// Called by the polling loop. Also called directly from unit tests to verify
/// the published shape without a real timer.
pub fn publish_status_snapshot(bus: &RiftBus, project_root: &Path) {
    let ts = now_unix_ms();
    let canonical_root = resolve_repo_root(project_root);
    let root_ref: &Path = canonical_root.as_deref().unwrap_or(project_root);

    let dir = compute_dir(root_ref);
    let git = compute_git(project_root);
    let repo = compute_repo(root_ref);

    let mut payload = json!({
        "dir":  dir,
        "git":  git,
        "repo": repo,
        "ts":   ts,
    });

    // Merge CC StatusJSON data (model, context, usage) when available.
    if let Some(cc) = read_cc_status() {
        let map = payload.as_object_mut().expect("just built");
        if let Some(m) = cc.model {
            map.insert("model".into(), json!(m));
        }
        if let Some(v) = cc.ctx_pct {
            map.insert("ctx_pct".into(), json!(v));
        }
        if let Some(v) = cc.session_use_pct {
            map.insert("session_use_pct".into(), json!(v));
        }
        if let Some(v) = cc.week_pct {
            map.insert("week_pct".into(), json!(v));
        }
        if let Some(v) = cc.github_owner {
            map.insert("github_owner".into(), json!(v));
        }
        if let Some(v) = cc.github_repo {
            map.insert("github_repo".into(), json!(v));
        }
        if let Some(v) = cc.skill {
            map.insert("skill".into(), json!(v));
        }
        if let Some(v) = cc.thinking {
            map.insert("thinking".into(), json!(v));
        }
        if let Some(v) = cc.effort {
            map.insert("effort".into(), json!(v));
        }
    }

    let mut env = Envelope::new(Category::Status, "usage");
    env.payload = payload;
    bus.publish(env);
}

// ---------------------------------------------------------------------------
// CC StatusJSON reader — reads the bridge-teed temp file
// ---------------------------------------------------------------------------

/// Parsed subset of Claude Code's StatusJSON relevant to Rift's status line.
struct CcStatus {
    model: Option<String>,
    ctx_pct: Option<u32>,
    session_use_pct: Option<u32>,
    week_pct: Option<u32>,
    github_owner: Option<String>,
    github_repo: Option<String>,
    skill: Option<String>,
    thinking: Option<String>,
    effort: Option<String>,
}

/// Read and parse `$TEMP/rift-cc-status.json`. Returns `None` if the file
/// is missing, unreadable, older than 30 seconds, or unparseable.
fn read_cc_status() -> Option<CcStatus> {
    let path = std::env::temp_dir().join("rift-cc-status.json");
    let metadata = std::fs::metadata(&path).ok()?;

    // Staleness guard — ignore files older than 30 seconds.
    let age = metadata
        .modified()
        .ok()?
        .elapsed()
        .unwrap_or(Duration::from_secs(999));
    if age > Duration::from_secs(30) {
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    // model — string or { id, display_name }
    let model = v.get("model").and_then(|m| {
        if let Some(s) = m.as_str() {
            Some(s.to_string())
        } else {
            m.get("display_name")
                .or_else(|| m.get("id"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        }
    });

    // ctx_pct — computed from context_window tokens
    let ctx_pct = v.get("context_window").and_then(|cw| {
        let window = cw.get("context_window_size").and_then(as_f64)?;
        if window <= 0.0 {
            return None;
        }
        let usage = cw.get("current_usage").and_then(|cu| {
            // current_usage can be a number or an object with token fields
            if let Some(n) = as_f64(cu) {
                Some(n)
            } else {
                let input = as_f64(cu.get("input_tokens")?).unwrap_or(0.0);
                let output = as_f64(cu.get("output_tokens")?).unwrap_or(0.0);
                Some(input + output)
            }
        })?;
        Some(((usage / window) * 100.0).round() as u32)
    });

    // session_use_pct — from rate_limits.five_hour.used_percentage
    let session_use_pct = v
        .get("rate_limits")
        .and_then(|rl| rl.get("five_hour"))
        .and_then(|fh| fh.get("used_percentage"))
        .and_then(as_f64)
        .map(|v| v.round() as u32);

    // week_pct — from rate_limits.seven_day.used_percentage
    let week_pct = v
        .get("rate_limits")
        .and_then(|rl| rl.get("seven_day"))
        .and_then(|sd| sd.get("used_percentage"))
        .and_then(as_f64)
        .map(|v| v.round() as u32);

    // GitHub repo info from workspace.repo (v2.1.145+)
    let github_owner = v
        .get("workspace")
        .and_then(|ws| ws.get("repo"))
        .and_then(|r| r.get("owner"))
        .and_then(|o| o.as_str())
        .map(|s| s.to_string());

    let github_repo = v
        .get("workspace")
        .and_then(|ws| ws.get("repo"))
        .and_then(|r| r.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());

    // skill — extract from session_name (e.g. "aegis:auto" → "aegis"),
    // or primary_skill, or session.active_skill (CC version dependent)
    let skill = v
        .get("session_name")
        .and_then(|s| s.as_str())
        .and_then(|s| s.split(':').next())
        .filter(|s| !s.is_empty() && *s != "default")
        .map(|s| s.to_string())
        .or_else(|| {
            v.get("primary_skill")
                .or_else(|| v.get("session").and_then(|s| s.get("active_skill")))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });

    // thinking — "on"/"off" based on whether extended thinking is enabled
    let thinking = v
        .get("thinking")
        .and_then(|t| t.get("type").and_then(|ty| ty.as_str()))
        .map(|ty| if ty == "enabled" { "on" } else { "off" })
        .or_else(|| {
            v.get("effort")
                .and_then(|e| e.get("level"))
                .and_then(|l| l.as_str())
                .map(|_| "on")
        })
        .map(|s| s.to_string());

    // effort — the active effort level (low/medium/high/max)
    let effort = v
        .get("effort")
        .and_then(|e| e.get("level"))
        .and_then(|l| l.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            v.get("thinking").and_then(|t| {
                if let Some(e) = t.get("effort").and_then(|e| e.as_str()) {
                    return Some(e.to_string());
                }
                if t.get("type").and_then(|ty| ty.as_str()) == Some("enabled") {
                    let budget = t.get("budget_tokens").and_then(as_f64);
                    Some(
                        match budget {
                            Some(b) if b >= 32000.0 => "max",
                            Some(b) if b >= 16000.0 => "high",
                            Some(b) if b >= 8000.0 => "medium",
                            Some(_) => "low",
                            None => "medium",
                        }
                        .to_string(),
                    )
                } else {
                    None
                }
            })
        });

    Some(CcStatus {
        model,
        ctx_pct,
        session_use_pct,
        week_pct,
        github_owner,
        github_repo,
        skill,
        thinking,
        effort,
    })
}

/// Coerce a JSON value to f64 — handles both number and numeric-string.
fn as_f64(v: &serde_json::Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        Some(n)
    } else {
        v.as_str()?.trim().parse::<f64>().ok()
    }
}

// ---------------------------------------------------------------------------
// dir — tilde-collapsed project root path
// ---------------------------------------------------------------------------

/// Return the project root as a tilde-collapsed string.
///
/// Home dir is resolved via the `directories` crate (`directories::BaseDirs`)
/// — already a workspace dep. Falls back to `std::env::var("HOME")` then
/// `std::env::var("USERPROFILE")` if `BaseDirs::new()` fails (very rare:
/// means no home dir was resolvable by the OS).
///
/// If `project_root` is not under the home dir, the full canonical path is
/// returned unchanged. Forward-slash separators are used on all platforms.
fn compute_dir(project_root: &Path) -> String {
    // Attempt to resolve the home directory from the `directories` crate first
    // (cross-platform, already in the workspace). Fall back to env vars for
    // environments where BaseDirs fails (e.g., containers without /etc/passwd).
    let home_opt: Option<PathBuf> = directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .or_else(|| {
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .ok()
                .map(PathBuf::from)
        });

    let root_str = project_root.to_string_lossy().replace('\\', "/");

    match home_opt {
        Some(home) => {
            let home_str = home.to_string_lossy().replace('\\', "/");
            if root_str.starts_with(&home_str) {
                // Replace the home prefix with `~`.
                format!("~{}", &root_str[home_str.len()..])
            } else {
                root_str
            }
        }
        None => root_str,
    }
}

// ---------------------------------------------------------------------------
// git — current branch or commit sha, with dirty marker
// ---------------------------------------------------------------------------

/// Return the current git branch (or short SHA on detached HEAD).
///
/// Appends `*` when `git status --porcelain` is non-empty (dirty tree).
/// Returns `"—"` when:
/// - `project_root` is not in a git repo.
/// - `git` is not on PATH.
/// - Any git invocation produces a non-zero exit with no useful stdout.
///
/// Errors are logged via `tracing::warn!`. The translator loop continues
/// running — a non-git directory is not a fatal condition.
///
/// # §9 boundary
///
/// All `std::process::Command` invocations to `git` are in this function only.
fn compute_git(project_root: &Path) -> String {
    // R-08: run branch resolution and dirty-check in parallel — they're
    // independent git calls (~20-50ms each on Windows).
    let (branch_name, dirty) = std::thread::scope(|s| {
        let dirty_handle = s.spawn(|| match git_cmd(project_root, &["status", "--porcelain"]) {
            Ok(output) => !output.is_empty(),
            Err(_) => false,
        });

        let branch_name = {
            let branch = git_cmd(project_root, &["symbolic-ref", "--short", "HEAD"]);
            match branch {
                Ok(name) if !name.is_empty() => Some(name),
                _ => match git_cmd(project_root, &["rev-parse", "--short", "HEAD"]) {
                    Ok(sha) if !sha.is_empty() => Some(sha),
                    Ok(_) | Err(_) => None,
                },
            }
        };

        let dirty = dirty_handle.join().unwrap_or(false);
        (branch_name, dirty)
    });

    let Some(branch_name) = branch_name else {
        return "\u{2014}".to_string(); // em-dash — not a git repo
    };

    if dirty {
        format!("{branch_name}*")
    } else {
        branch_name
    }
}

/// Resolve the canonical git repository root via `git rev-parse --show-toplevel`.
///
/// Returns `Some(PathBuf)` when `project_root` lies inside a git tree, even
/// when it is several directories deep (e.g., the binary's CWD is the
/// `src-tauri/` workspace child but the repo root is one level up). Returns
/// `None` outside a git tree — callers should fall back to `project_root`.
///
/// # §9 boundary
///
/// All `std::process::Command` invocations to `git` are confined to this
/// translator. `git_cmd` does the actual subprocess work.
fn resolve_repo_root(project_root: &Path) -> Option<PathBuf> {
    let toplevel = git_cmd(project_root, &["rev-parse", "--show-toplevel"]).ok()?;
    if toplevel.is_empty() {
        return None;
    }
    Some(PathBuf::from(toplevel))
}

/// Run `git -C <root> <args...>` and return trimmed stdout on exit 0.
///
/// On non-zero exit or spawn failure, logs `tracing::warn!` and returns `Err`.
fn git_cmd(root: &Path, args: &[&str]) -> Result<String, ()> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(root);
    for arg in args {
        cmd.arg(arg);
    }
    // Suppress stderr so git's "not a git repo" messages don't pollute output.
    cmd.stderr(std::process::Stdio::null());

    // Windows: suppress the visible console window that flashes on every
    // `Command::spawn` of a console-subsystem child. Without this flag,
    // the 5-second polling tick paints a `git.exe` flash on screen each
    // tick — the user-visible "spamming flashes of terminals" symptom.
    // No-op on macOS/Linux (compiled out via cfg).
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.output() {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        Ok(out) => {
            tracing::warn!(
                "status-translator: git {:?} exited {:?}",
                args,
                out.status.code()
            );
            Err(())
        }
        Err(e) => {
            tracing::warn!("status-translator: git spawn failed: {e}");
            Err(())
        }
    }
}

// ---------------------------------------------------------------------------
// repo — basename of the project root
// ---------------------------------------------------------------------------

/// Return the basename of the project root path (v1: path-basename only).
///
/// For example `/home/user/projects/Rift_TerminalV2` → `"Rift_TerminalV2"`.
/// Falls back to the full path string if `file_name()` returns `None` (e.g.
/// the root is `/`).
fn compute_repo(project_root: &Path) -> String {
    project_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| project_root.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Timestamp helper
// ---------------------------------------------------------------------------

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RiftBus, SubscribeFilter};

    // T1 — status_envelope_shape
    // Verify Category, kind, and payload field presence for a usage envelope.
    #[test]
    fn status_envelope_shape() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();

        let bus = RiftBus::default();
        publish_status_snapshot(&bus, &root);

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1, "expected exactly one envelope");

        let env = &snapshot[0];
        assert_eq!(env.category, Category::Status, "category must be Status");
        assert_eq!(env.kind, "usage", "kind must be 'usage'");

        // All three fields must be present (even if em-dash).
        assert!(
            env.payload.get("dir").is_some(),
            "payload must contain 'dir'"
        );
        assert!(
            env.payload.get("git").is_some(),
            "payload must contain 'git'"
        );
        assert!(
            env.payload.get("repo").is_some(),
            "payload must contain 'repo'"
        );
        assert!(env.payload.get("ts").is_some(), "payload must contain 'ts'");
    }

    // T2 — git_degrades_on_non_git_dir
    // When project_root is NOT a git repo, compute_git returns em-dash, no panic.
    #[test]
    fn git_degrades_on_non_git_dir() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        // tempdir is guaranteed to NOT be a git repo.
        let result = compute_git(root);
        assert_eq!(
            result, "\u{2014}",
            "non-git dir must yield em-dash, got: {result:?}"
        );
    }

    // T3 — repo_is_basename
    // compute_repo returns the last path component.
    #[test]
    fn repo_is_basename() {
        let path = PathBuf::from("/some/deep/path/MyProject");
        assert_eq!(compute_repo(&path), "MyProject");
    }

    // T4 — dir_tilde_collapse
    // compute_dir collapses the home prefix to `~`.
    #[test]
    fn dir_tilde_collapse() {
        // Resolve the actual home dir to build a synthetic path under it.
        let home_opt = directories::BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .or_else(|| {
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .ok()
                    .map(PathBuf::from)
            });

        if let Some(home) = home_opt {
            let synthetic = home.join("projects").join("MyProject");
            let result = compute_dir(&synthetic);
            assert!(
                result.starts_with('~'),
                "dir should start with ~ for paths under home; got: {result}"
            );
            assert!(
                result.contains("MyProject"),
                "dir should contain the project name; got: {result}"
            );
        }
        // If no home is resolvable (very rare in CI), skip assertion gracefully.
    }

    // T5 — version_not_bumped
    // Status envelopes carry CURRENT_VERSION (additive kind, no version bump).
    #[test]
    fn version_not_bumped() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let bus = RiftBus::default();
        publish_status_snapshot(&bus, dir.path());
        let (snapshot, _) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].version, crate::CURRENT_VERSION);
    }
}
