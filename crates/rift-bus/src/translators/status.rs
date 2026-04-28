//! Status translator — publishes `Category::Status` envelopes every 5 seconds.
//!
//! Closes the unblocked slice of D-012: `DIR`, `GIT`, and `REPO` StatusLine
//! segments. The remaining segments (`CTX`, `SESSION`, `WEEK`, `MODEL`) stay
//! as em-dash placeholders — they are upstream-blocked on a Claude Code usage
//! hook that does not yet exist. See `DEFERRED.md` D-012 for the tracking entry.
//!
//! ## §9 Boundary note
//!
//! ALL git subprocess invocations and home-dir resolution live exclusively in
//! this file. Nothing leaks to rift-bus core. `check-translator-boundary.sh`
//! enforces the `std::process::Command` + `tokio::net::` pattern — git via
//! `std::process::Command` is in the allowlist for translators.
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
//!   "ts":   1714262400000
//! }
//! ```
//!
//! `dir` is tilde-collapsed (home dir replaced with `~`). `git` appends `*`
//! when the working tree is dirty. Falls back to `—` on any failure so
//! callers never need to handle absent fields — the segments always have a
//! displayable value.

use std::path::{Path, PathBuf};

use serde_json::json;
use tokio::time::{interval, Duration};

use crate::{Category, Envelope, RiftBus};

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
/// The loop runs until the Tauri process exits or the spawned task is aborted.
///
/// # §9 boundary
///
/// All git subprocess invocations and home-dir lookups are confined to this
/// function and its private helpers. No external-system calls escape to
/// rift-bus core.
pub async fn spawn_status_translator(bus: RiftBus, project_root: PathBuf) {
    let mut tick = interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        publish_status_snapshot(&bus, &project_root);
    }
}

// ---------------------------------------------------------------------------
// Snapshot publisher (pub(crate) for tests)
// ---------------------------------------------------------------------------

/// Compute and publish one `Category::Status / kind="usage"` envelope.
///
/// Called by the polling loop. Also called directly from unit tests to verify
/// the published shape without a real timer.
pub(crate) fn publish_status_snapshot(bus: &RiftBus, project_root: &Path) {
    let ts = now_unix_ms();
    // Resolve the repo root via `git rev-parse --show-toplevel` so DIR/REPO
    // reflect the actual repository even when the binary's CWD is a
    // subdirectory (e.g., `tauri dev` runs rift.exe with CWD=src-tauri/).
    // Falls back to `project_root` when not in a git tree.
    let canonical_root = resolve_repo_root(project_root);
    let root_ref: &Path = canonical_root.as_deref().unwrap_or(project_root);

    let dir = compute_dir(root_ref);
    let git = compute_git(project_root); // git_cmd already takes any path inside the repo
    let repo = compute_repo(root_ref);

    let mut env = Envelope::new(Category::Status, "usage");
    env.payload = json!({
        "dir":  dir,
        "git":  git,
        "repo": repo,
        "ts":   ts,
    });
    bus.publish(env);
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
    // Try `symbolic-ref --short HEAD` for a branch name.
    let branch = git_cmd(project_root, &["symbolic-ref", "--short", "HEAD"]);

    let branch_name = match branch {
        Ok(name) if !name.is_empty() => name,
        _ => {
            // Detached HEAD — fall back to short commit sha.
            match git_cmd(project_root, &["rev-parse", "--short", "HEAD"]) {
                Ok(sha) if !sha.is_empty() => sha,
                Ok(_) | Err(_) => {
                    // Not a git repo or git not found — silent dash.
                    return "\u{2014}".to_string(); // em-dash
                }
            }
        }
    };

    // Check for dirty working tree.
    let dirty = match git_cmd(project_root, &["status", "--porcelain"]) {
        Ok(output) => !output.is_empty(),
        Err(_) => false,
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
