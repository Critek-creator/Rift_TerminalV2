//! L3 process-name detection — best-effort CLAUDE lane fallback.
//!
//! Walks the child process tree of a given root PID looking for known
//! Claude Code binaries (`claude`, `node`). Called once per CMD_START
//! sentinel (not per chunk) — the result is cached until CMD_END.
//!
//! Platform support:
//!   - Windows: `CreateToolhelp32Snapshot` + `Process32First/Next`
//!   - Unix: `/proc/<pid>/children` walk (Linux only; macOS = no-op)
//!
//! Returns `true` if a Claude Code process is detected as a descendant
//! of `root_pid`. False negatives are acceptable (best-effort); false
//! positives are tolerable (lane reverts at CMD_END/PROMPT_START).

#[cfg(windows)]
const CLAUDE_BINARY_NAMES: &[&str] = &["claude", "claude.exe"];

/// Returns `true` if any child of `root_pid` appears to be a Claude Code
/// process. Best-effort: returns `false` on any error or unsupported platform.
pub fn is_claude_descendant(root_pid: u32) -> bool {
    detect_claude_child(root_pid)
}

#[cfg(windows)]
fn detect_claude_child(root_pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) == 0 {
            CloseHandle(snapshot);
            return false;
        }

        loop {
            if entry.th32ParentProcessID == root_pid {
                let exe_name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len())],
                );
                let exe_lower = exe_name.to_lowercase();
                for &name in CLAUDE_BINARY_NAMES {
                    if exe_lower == name || exe_lower.starts_with("claude") {
                        CloseHandle(snapshot);
                        return true;
                    }
                }
            }

            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }

        CloseHandle(snapshot);
    }

    false
}

#[cfg(target_os = "linux")]
fn detect_claude_child(root_pid: u32) -> bool {
    let children_path = format!("/proc/{root_pid}/task/{root_pid}/children");
    let Ok(children_str) = std::fs::read_to_string(&children_path) else {
        return false;
    };
    for pid_str in children_str.split_whitespace() {
        let Ok(_pid) = pid_str.parse::<u32>() else {
            continue;
        };
        let comm_path = format!("/proc/{pid_str}/comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            let name = comm.trim().to_lowercase();
            if name == "claude" || name.starts_with("claude") {
                return true;
            }
        }
    }
    false
}

#[cfg(not(any(windows, target_os = "linux")))]
fn detect_claude_child(_root_pid: u32) -> bool {
    // macOS + other platforms: no-op for v1. Users can opt in via
    // RIFT_CLAUDE_SENTINELS=1 which makes CC emit CLAUDE_START/END.
    false
}

// ---------------------------------------------------------------------------
// collect_descendant_pids — full descendant tree (resource-monitor foundation)
// ---------------------------------------------------------------------------

/// Collect `root_pid` plus all its transitive descendants.
///
/// Best-effort, cross-platform. Element 0 is always `root_pid` itself so callers
/// can sample the whole process tree (root + children) uniformly. Returns
/// `vec![root_pid]` on unsupported platforms or on any enumeration error.
///
/// This reuses the same process-table walk that [`is_claude_descendant`] relies
/// on (the Windows toolhelp snapshot / Linux `/proc` children walk), but returns
/// the full descendant set rather than a boolean — the foundation the per-pane
/// resource monitor samples CPU/RSS over.
pub fn collect_descendant_pids(root_pid: u32) -> Vec<u32> {
    collect_descendants_impl(root_pid)
}

#[cfg(windows)]
fn collect_descendants_impl(root_pid: u32) -> Vec<u32> {
    use std::collections::{HashMap, HashSet, VecDeque};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // One snapshot pays for the whole system process table; build a
    // parent -> children adjacency map, then BFS from root_pid.
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return vec![root_pid];
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) == 0 {
            CloseHandle(snapshot);
            return vec![root_pid];
        }
        loop {
            children
                .entry(entry.th32ParentProcessID)
                .or_default()
                .push(entry.th32ProcessID);
            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snapshot);
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(root_pid);
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue; // guard against parent/child cycles (e.g. pid 0)
        }
        out.push(pid);
        if let Some(kids) = children.get(&pid) {
            for &k in kids {
                queue.push_back(k);
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn collect_descendants_impl(root_pid: u32) -> Vec<u32> {
    use std::collections::HashSet;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        out.push(pid);
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(s) = std::fs::read_to_string(&children_path) {
            for c in s.split_whitespace() {
                if let Ok(cpid) = c.parse::<u32>() {
                    stack.push(cpid);
                }
            }
        }
    }
    out
}

#[cfg(not(any(windows, target_os = "linux")))]
fn collect_descendants_impl(root_pid: u32) -> Vec<u32> {
    // macOS + other platforms: sample the root pid only for v1.
    vec![root_pid]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_pid_returns_false() {
        // PID 0 or a very high PID should not have claude children.
        assert!(!is_claude_descendant(u32::MAX));
    }

    #[test]
    fn current_process_no_claude() {
        // Our test process shouldn't have a claude child.
        let pid = std::process::id();
        assert!(!is_claude_descendant(pid));
    }

    #[test]
    fn collect_descendants_includes_root() {
        let pid = std::process::id();
        let pids = collect_descendant_pids(pid);
        assert!(
            pids.contains(&pid),
            "descendant set must always include the root pid itself"
        );
    }

    #[test]
    fn collect_descendants_no_duplicates() {
        let pid = std::process::id();
        let pids = collect_descendant_pids(pid);
        let mut sorted = pids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), pids.len(), "descendant pids must be unique");
    }
}
