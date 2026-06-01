//! Per-pane process resource sampler.
//!
//! Samples aggregate CPU% and resident memory (RSS) over a PTY pane's
//! foreground process tree. The descendant PID set comes from
//! [`rift_core::process::collect_descendant_pids`] (the same toolhelp-snapshot /
//! `/proc` walk that powers Claude-lane detection); `sysinfo` reads the
//! per-process CPU/RSS for those PIDs.
//!
//! §9 note: this module touches only the local process table (`sysinfo`) — no
//! `tokio::net`, `reqwest`, `claude_*`, or `mcp_*` — so it is outside the
//! translator-boundary concern entirely.

use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::Serialize;
use sysinfo::{Pid, ProcessesToUpdate, System};

/// Persistent `System` instance. CPU usage in `sysinfo` is computed as a delta
/// between consecutive refreshes, so the instance MUST persist across sample
/// calls — a fresh `System` every call would always report 0% CPU. The 1Hz
/// sampling cadence is comfortably above `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`.
static SYS: Lazy<Mutex<System>> = Lazy::new(|| Mutex::new(System::new()));

/// One per-pane resource sample, returned to the frontend StatusLine.
#[derive(Serialize, Clone, Debug)]
pub struct PaneResourceSnapshot {
    /// Aggregate CPU% across the process tree. Can exceed 100 on multi-core
    /// workloads (htop semantics: each core contributes up to 100%).
    pub cpu_pct: f32,
    /// Aggregate resident set size in kilobytes.
    pub rss_kb: u64,
    /// Name of the busiest descendant command, or the shell when no child runs.
    pub foreground_cmd: String,
    /// Number of processes actually sampled (diagnostics; 0 = pane idle/gone).
    pub pid_count: usize,
}

/// Sample aggregate CPU% and RSS over `root_pid` and all its descendants.
///
/// Blocking (syscall-bound) — call from `spawn_blocking`. The first sample of
/// any given PID returns 0% CPU (no prior baseline); subsequent samples report
/// the delta over the elapsed interval.
pub fn sample_resources_for_tree(root_pid: u32) -> PaneResourceSnapshot {
    let pids = rift_core::process::collect_descendant_pids(root_pid);

    // Recover from a poisoned lock rather than panicking the sampler thread —
    // a stale System is still usable for the next sample.
    let mut sys = SYS.lock().unwrap_or_else(|p| p.into_inner());
    // Refresh ALL processes with dead-process eviction (the `true`). A subset
    // refresh (ProcessesToUpdate::Some) would leak: short-lived descendants
    // (e.g. the hundreds of `rustc` workers a `cargo build` spawns) would never
    // be evicted from the persistent System and accumulate unbounded over a
    // session. A full 1Hz refresh is cheap and keeps the table bounded to live
    // processes while preserving correct per-process CPU deltas.
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut cpu_pct = 0.0f32;
    let mut rss_bytes = 0u64;
    let mut counted = 0usize;
    let mut root_name = String::new();
    let mut best_child: Option<(f32, String)> = None;

    for &pid in &pids {
        let Some(proc_) = sys.process(Pid::from_u32(pid)) else {
            continue;
        };
        let c = proc_.cpu_usage();
        cpu_pct += c;
        rss_bytes += proc_.memory();
        counted += 1;
        let name = proc_.name().to_string_lossy().to_string();
        if pid == root_pid {
            root_name = name;
        } else if best_child.as_ref().map(|(bc, _)| c > *bc).unwrap_or(true) {
            // The busiest non-root descendant is the "foreground" command
            // (e.g. `cargo` while a build runs, not the idle shell).
            best_child = Some((c, name));
        }
    }

    PaneResourceSnapshot {
        cpu_pct,
        rss_kb: rss_bytes / 1024,
        foreground_cmd: best_child.map(|(_, n)| n).unwrap_or(root_name),
        pid_count: counted,
    }
}
