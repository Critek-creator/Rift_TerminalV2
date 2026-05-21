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
}
