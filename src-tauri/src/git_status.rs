// Phase 8.7i — git status snapshot for the Git notif tab.
//
// Shells out to `git` once per call. Two invocations:
//   1. `git status --porcelain=v1 -b` — branch line + working-tree changes.
//   2. `git log -1 --format=%H%n%s%n%cn%n%cI` — last commit summary.
//
// If the project root is not inside a git working tree, the `not_a_repo`
// flag is set and the rest of the fields are zeroed; the frontend renders
// an empty-state card instead of erroring.

use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Default)]
pub struct GitStatus {
    /// True when `git status` exits with status 128 ("not a git repository").
    pub not_a_repo: bool,
    /// Local branch name. Empty if detached HEAD.
    pub branch: String,
    /// Tracked upstream (e.g. `origin/main`). Empty if no upstream.
    pub upstream: String,
    /// Commits ahead of upstream.
    pub ahead: u32,
    /// Commits behind upstream.
    pub behind: u32,
    /// Files with index-staged changes.
    pub staged: Vec<GitFileEntry>,
    /// Files with worktree (unstaged) changes.
    pub modified: Vec<GitFileEntry>,
    /// Untracked files.
    pub untracked: Vec<GitFileEntry>,
    /// Last commit on HEAD. None if repo is empty / has no commits yet.
    pub last_commit: Option<GitCommit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitFileEntry {
    pub path: String,
    /// Single-letter porcelain code (`M`, `A`, `D`, `R`, `C`, `?`, etc.).
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitCommit {
    pub hash: String,
    pub short_hash: String,
    pub subject: String,
    pub author: String,
    pub iso_date: String,
}

/// Cap on per-list entries so a stale repo with thousands of untracked
/// files doesn't bloat the IPC payload. Frontend can surface the cap hint.
const MAX_LIST: usize = 200;

/// Run a one-shot status snapshot. Errors that aren't "not a repo" are
/// returned as `Err(String)` so the frontend can show the failure mode.
pub fn snapshot(root: &Path) -> Result<GitStatus, String> {
    let mut status = GitStatus::default();

    let porcelain = run_git(root, &["status", "--porcelain=v1", "-b"])?;

    if porcelain.exit_code == 128 {
        status.not_a_repo = true;
        return Ok(status);
    }
    if !porcelain.success {
        return Err(format!(
            "git status exited {} — stderr: {}",
            porcelain.exit_code,
            porcelain.stderr.trim()
        ));
    }

    parse_porcelain(&porcelain.stdout, &mut status);

    // Last commit — separate invocation so a fresh repo with no commits
    // doesn't poison the porcelain parse. Empty output → no commits yet.
    let log = run_git(root, &["log", "-1", "--format=%H%n%h%n%s%n%cn%n%cI"]);
    if let Ok(out) = log {
        if out.success {
            let mut iter = out.stdout.lines();
            let hash = iter.next().unwrap_or("").trim().to_string();
            let short_hash = iter.next().unwrap_or("").trim().to_string();
            let subject = iter.next().unwrap_or("").trim().to_string();
            let author = iter.next().unwrap_or("").trim().to_string();
            let iso_date = iter.next().unwrap_or("").trim().to_string();
            if !hash.is_empty() {
                status.last_commit = Some(GitCommit {
                    hash,
                    short_hash,
                    subject,
                    author,
                    iso_date,
                });
            }
        }
    }

    Ok(status)
}

struct GitOutput {
    success: bool,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_git(root: &Path, args: &[&str]) -> Result<GitOutput, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root);
    for a in args {
        cmd.arg(a);
    }
    let output = cmd.output().map_err(|e| {
        format!("git_status: failed to spawn `git`: {e} (is git installed and on PATH?)")
    })?;
    Ok(GitOutput {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn parse_porcelain(stdout: &str, status: &mut GitStatus) {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            parse_branch_line(rest, status);
        } else if let Some(path) = line.strip_prefix("?? ") {
            push_capped(&mut status.untracked, path.to_string(), "?".to_string());
        } else if line.len() >= 3 {
            // Two-char XY status + space + path.
            let bytes = line.as_bytes();
            let x = bytes[0] as char;
            let y = bytes[1] as char;
            let path = line[3..].to_string();
            // Strip rename arrows: "old -> new" → keep the `new` half.
            let path = path.split(" -> ").last().unwrap_or(&path).to_string();

            if x != ' ' && x != '?' {
                push_capped(&mut status.staged, path.clone(), x.to_string());
            }
            if y != ' ' && y != '?' {
                push_capped(&mut status.modified, path, y.to_string());
            }
        }
    }
}

fn parse_branch_line(rest: &str, status: &mut GitStatus) {
    // Examples:
    //   "main"                                — no upstream
    //   "main...origin/main"                  — clean
    //   "main...origin/main [ahead 3]"        — ahead only
    //   "main...origin/main [behind 1]"       — behind only
    //   "main...origin/main [ahead 2, behind 1]"
    //   "HEAD (no branch)"                    — detached
    let main = rest.split('[').next().unwrap_or(rest).trim();
    let bracket = rest
        .find('[')
        .and_then(|i| rest.get(i + 1..).and_then(|s| s.strip_suffix(']')));

    if let Some((local, upstream)) = main.split_once("...") {
        status.branch = local.trim().to_string();
        status.upstream = upstream.trim().to_string();
    } else {
        status.branch = main.to_string();
    }

    if let Some(b) = bracket {
        for chunk in b.split(',') {
            let t = chunk.trim();
            if let Some(rest) = t.strip_prefix("ahead ") {
                if let Ok(n) = rest.trim().parse::<u32>() {
                    status.ahead = n;
                }
            } else if let Some(rest) = t.strip_prefix("behind ") {
                if let Ok(n) = rest.trim().parse::<u32>() {
                    status.behind = n;
                }
            }
        }
    }
}

fn push_capped(v: &mut Vec<GitFileEntry>, path: String, status: String) {
    if v.len() >= MAX_LIST {
        return;
    }
    v.push(GitFileEntry { path, status });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_branch_with_ahead_behind() {
        let mut s = GitStatus::default();
        parse_branch_line("main...origin/main [ahead 2, behind 1]", &mut s);
        assert_eq!(s.branch, "main");
        assert_eq!(s.upstream, "origin/main");
        assert_eq!(s.ahead, 2);
        assert_eq!(s.behind, 1);
    }

    #[test]
    fn parse_branch_no_upstream() {
        let mut s = GitStatus::default();
        parse_branch_line("feature/foo", &mut s);
        assert_eq!(s.branch, "feature/foo");
        assert_eq!(s.upstream, "");
        assert_eq!(s.ahead, 0);
        assert_eq!(s.behind, 0);
    }

    #[test]
    fn parse_porcelain_classifies_xy() {
        let mut s = GitStatus::default();
        let stdout = "## main...origin/main\n M src/foo.rs\nM  src/bar.rs\n?? newfile.txt\n";
        parse_porcelain(stdout, &mut s);
        assert_eq!(s.modified.len(), 1);
        assert_eq!(s.modified[0].path, "src/foo.rs");
        assert_eq!(s.staged.len(), 1);
        assert_eq!(s.staged[0].path, "src/bar.rs");
        assert_eq!(s.untracked.len(), 1);
        assert_eq!(s.untracked[0].path, "newfile.txt");
    }
}
