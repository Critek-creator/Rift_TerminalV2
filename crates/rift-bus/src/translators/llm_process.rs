//! llama-server process manager — spawn, stop, monitor, and clean up
//! local llama-server child processes.
//!
//! Platform-specific lifecycle:
//! - Windows: `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`, graceful
//!   shutdown via `CTRL_BREAK_EVENT`, fallback to `TerminateProcess`.
//! - Unix: standard `SIGTERM` → wait → `SIGKILL`.
//!
//! Publishes `Category::Llm` bus events for process lifecycle
//! (`llm.process.start`, `llm.process.stop`, `llm.process.crash`).
//!
//! Lives inside the §9 translator boundary.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::config::LlamaServerConfig;
use crate::{Category, Envelope, RiftBus};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

// ---------------------------------------------------------------------------
// PID tracking
// ---------------------------------------------------------------------------

/// On-disk record of managed llama-server processes. Written next to
/// `config.toml` so startup can detect and clean up orphans from crashes.
#[derive(Default, Serialize, Deserialize)]
struct PidFile {
    processes: Vec<PidEntry>,
}

#[derive(Serialize, Deserialize)]
struct PidEntry {
    model_id: String,
    pid: u32,
    port: u16,
}

fn pid_file_path() -> Result<PathBuf, crate::config::ConfigError> {
    let dirs = directories::ProjectDirs::from("com", "abyssal", "rift")
        .ok_or(crate::config::ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().join("llm-pids.json"))
}

fn load_pid_file() -> PidFile {
    pid_file_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pid_file(pf: &PidFile) {
    if let Ok(path) = pid_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, serde_json::to_string_pretty(pf).unwrap_or_default());
    }
}

// ---------------------------------------------------------------------------
// Process alive check (reuses pattern from config.rs)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

// ---------------------------------------------------------------------------
// Graceful shutdown
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn graceful_stop(pid: u32) -> bool {
    extern "system" {
        fn GenerateConsoleCtrlEvent(dw_ctrl_event: u32, dw_process_group_id: u32) -> i32;
    }
    const CTRL_BREAK_EVENT: u32 = 1;
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) != 0 }
}

#[cfg(not(windows))]
fn graceful_stop(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) == 0 }
}

// ---------------------------------------------------------------------------
// Windows Job Object — OS-guaranteed child cleanup on Rift exit
// ---------------------------------------------------------------------------
//
// `std::process::Child` does NOT kill its process on drop, and the exit
// watchdog uses `process::exit(0)` which skips Rust destructors entirely.
// A Job Object configured with KILL_ON_JOB_CLOSE makes Windows terminate
// every assigned process the instant the job handle closes — which the OS
// does automatically when Rift's process dies, for ANY reason (clean exit,
// crash, force-kill, or the watchdog's process::exit). This is the only way
// to guarantee no orphaned llama-server, independent of ExitRequested firing.
#[cfg(windows)]
mod job {
    use std::os::raw::c_void;
    use std::os::windows::io::RawHandle;

    type Handle = *mut c_void;

    #[repr(C)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct ExtendedLimitInformation {
        basic_limit_information: BasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: u32 = 9;

    extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: u32, info: *const c_void, len: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(h: Handle) -> i32;
    }

    /// Owns a kill-on-close Job Object handle.
    pub struct KillOnCloseJob {
        handle: Handle,
    }

    // The handle is owned solely by this struct; the Win32 calls used on it
    // (AssignProcessToJobObject / CloseHandle) are thread-safe.
    unsafe impl Send for KillOnCloseJob {}
    unsafe impl Sync for KillOnCloseJob {}

    impl KillOnCloseJob {
        /// Create a Job Object that kills all assigned processes when closed.
        pub fn new() -> Option<Self> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let mut info: ExtendedLimitInformation = unsafe { std::mem::zeroed() };
            info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = unsafe {
                SetInformationJobObject(
                    handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    &info as *const _ as *const c_void,
                    std::mem::size_of::<ExtendedLimitInformation>() as u32,
                )
            };
            if ok == 0 {
                unsafe { CloseHandle(handle) };
                return None;
            }
            Some(Self { handle })
        }

        /// Assign a spawned child (by raw process handle) to the job. Best-effort.
        pub fn assign(&self, process: RawHandle) -> bool {
            unsafe { AssignProcessToJobObject(self.handle, process as Handle) != 0 }
        }
    }

    impl Drop for KillOnCloseJob {
        fn drop(&mut self) {
            // Closing the last handle fires KILL_ON_JOB_CLOSE. (The OS also
            // does this automatically when the process exits, so cleanup
            // happens even if this Drop never runs.)
            unsafe { CloseHandle(self.handle) };
        }
    }
}

// ---------------------------------------------------------------------------
// CLI flag builder
// ---------------------------------------------------------------------------

fn build_cli_args(config: &LlamaServerConfig) -> Vec<String> {
    let mut args = vec![
        "--model".to_string(),
        config.model_path.display().to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        config.port.to_string(),
        "--ctx-size".to_string(),
        config.ctx_size.to_string(),
        "--cache-type-k".to_string(),
        config.cache_type_k.as_flag().to_string(),
        "--cache-type-v".to_string(),
        config.cache_type_v.as_flag().to_string(),
        "--parallel".to_string(),
        config.parallel.to_string(),
    ];

    // A negative n_gpu_layers means "auto": omit the flag so llama-server's
    // device-memory fitter chooses the offload split. A hardcoded value
    // disables that fitter (it aborts with "n_gpu_layers already set by user").
    if config.n_gpu_layers >= 0 {
        args.push("--n-gpu-layers".to_string());
        args.push(config.n_gpu_layers.to_string());
    }

    // MoE expert offload — `--cpu-moe` (all experts to CPU) takes precedence
    // over the finer-grained `--n-cpu-moe N` (first N layers' experts).
    if config.cpu_moe {
        args.push("--cpu-moe".to_string());
    } else if let Some(n) = config.n_cpu_moe {
        args.push("--n-cpu-moe".to_string());
        args.push(n.to_string());
    }

    // `--cache-ram N` — host-RAM prompt-reuse cache (MiB). 0 disables it.
    // Omitted entirely when None so llama-server uses its 8 GiB default.
    if let Some(mib) = config.cache_ram {
        args.push("--cache-ram".to_string());
        args.push(mib.to_string());
    }

    if config.flash_attention {
        // Modern llama-server requires an explicit value: `--flash-attn on|off|auto`.
        // Passing the bare flag fails with "expected value for argument".
        args.push("--flash-attn".to_string());
        args.push("on".to_string());
    }

    if let Some(threads) = config.threads {
        args.push("--threads".to_string());
        args.push(threads.to_string());
    }

    for flag in &config.extra_flags {
        args.push(flag.clone());
    }

    args
}

// ---------------------------------------------------------------------------
// Managed process entry
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct ManagedProcess {
    model_id: String,
    child: Child,
    port: u16,
    config: LlamaServerConfig,
}

// ---------------------------------------------------------------------------
// Auto-restart policy
// ---------------------------------------------------------------------------

/// Max auto-restart attempts allowed within [`RESTART_WINDOW`]. Once a model
/// crashes this many times inside the window, auto-restart gives up and leaves
/// it in an error state rather than restart-looping (e.g. a model that OOMs on
/// every launch). The window resets after `RESTART_WINDOW` of no new attempts.
const MAX_RESTART_ATTEMPTS: u32 = 3;

/// Sliding window over which [`MAX_RESTART_ATTEMPTS`] is counted.
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// Per-model auto-restart bookkeeping. Tracks how many restarts have happened
/// in the current window so a crash-looping model is capped instead of being
/// respawned forever.
struct RestartInfo {
    attempts: u32,
    window_start: Instant,
}

// ---------------------------------------------------------------------------
// ProcessManager
// ---------------------------------------------------------------------------

/// Manages local llama-server child processes. Thread-safe — holds state
/// behind a `parking_lot::Mutex` for consistency with the rest of the
/// codebase (8 crash paths fixed by the parking_lot migration).
pub struct ProcessManager {
    llama_server_path: PathBuf,
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
    bus: RiftBus,
    /// Per-model auto-restart attempt tracking, keyed by model id. Used by the
    /// health monitor to cap crash-loop restarts within [`RESTART_WINDOW`].
    restart_tracker: Arc<Mutex<HashMap<String, RestartInfo>>>,
    /// Kill-on-close Job Object that all spawned servers are assigned to, so
    /// Windows reaps them when Rift's process exits for any reason. `None` if
    /// job creation failed (falls back to kill_all / orphan cleanup).
    #[cfg(windows)]
    job: Option<job::KillOnCloseJob>,
}

impl ProcessManager {
    pub fn new(llama_server_path: impl Into<PathBuf>, bus: RiftBus) -> Self {
        Self {
            llama_server_path: llama_server_path.into(),
            processes: Arc::new(Mutex::new(HashMap::new())),
            bus,
            restart_tracker: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(windows)]
            job: job::KillOnCloseJob::new(),
        }
    }

    /// Detect llama-server on PATH if no explicit path is configured.
    pub fn detect_llama_server() -> Option<PathBuf> {
        let names = if cfg!(windows) {
            &["llama-server.exe", "llama-server"][..]
        } else {
            &["llama-server"][..]
        };
        let path_var = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Clean up orphaned processes from a previous Rift crash.
    /// Called once on startup before auto-starting any models.
    pub fn cleanup_orphans(&self) {
        let mut pf = load_pid_file();
        let mut cleaned = 0u32;

        pf.processes.retain(|entry| {
            if is_process_alive(entry.pid) {
                tracing::info!(
                    model_id = %entry.model_id,
                    pid = entry.pid,
                    port = entry.port,
                    "llm_process: orphan detected — killing"
                );
                graceful_stop(entry.pid);
                std::thread::sleep(Duration::from_millis(500));
                if is_process_alive(entry.pid) {
                    #[cfg(windows)]
                    {
                        // Last resort — TerminateProcess via taskkill
                        let mut cmd = Command::new("taskkill");
                        cmd.args(["/F", "/PID", &entry.pid.to_string()]);
                        #[cfg(windows)]
                        cmd.creation_flags(CREATE_NO_WINDOW);
                        let _ = cmd.output();
                    }
                    #[cfg(not(windows))]
                    unsafe {
                        libc::kill(entry.pid as i32, libc::SIGKILL);
                    }
                }
                cleaned += 1;
                false
            } else {
                false
            }
        });

        save_pid_file(&pf);

        if cleaned > 0 {
            tracing::info!(cleaned, "llm_process: orphan cleanup complete");
        }
    }

    /// Start a local llama-server process for the given model.
    pub fn start(
        &self,
        model_id: &str,
        config: &LlamaServerConfig,
    ) -> Result<u32, super::llm::LlmError> {
        let mut procs = self.processes.lock();

        if procs.contains_key(model_id) {
            return Err(super::llm::LlmError::Internal {
                message: format!("model {model_id} already running"),
            });
        }

        let args = build_cli_args(config);
        let mut cmd = Command::new(&self.llama_server_path);
        cmd.args(&args);

        if let Some(devices) = &config.cuda_visible_devices {
            cmd.env("CUDA_VISIBLE_DEVICES", devices);
        }

        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);

        let child = cmd
            .spawn()
            .map_err(|_| super::llm::LlmError::ProcessNotRunning {
                model_id: model_id.to_string(),
            })?;

        let pid = child.id();

        // Assign to the kill-on-close Job Object so Windows reaps this server
        // when Rift exits (clean, crash, or watchdog process::exit).
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            if let Some(job) = &self.job {
                if !job.assign(child.as_raw_handle()) {
                    tracing::warn!(
                        model_id,
                        pid,
                        "llm_process: could not assign to job object — exit cleanup falls back to kill_all"
                    );
                }
            }
        }

        tracing::info!(model_id, pid, port = config.port, "llm_process: started");

        publish_process_event(
            &self.bus,
            "llm.process.start",
            model_id,
            pid,
            config.port,
            None,
        );

        // Track in PID file for orphan recovery
        let mut pf = load_pid_file();
        pf.processes.push(PidEntry {
            model_id: model_id.to_string(),
            pid,
            port: config.port,
        });
        save_pid_file(&pf);

        procs.insert(
            model_id.to_string(),
            ManagedProcess {
                model_id: model_id.to_string(),
                child,
                port: config.port,
                config: config.clone(),
            },
        );

        Ok(pid)
    }

    /// Stop a running local llama-server process.
    pub fn stop(&self, model_id: &str) -> Result<(), super::llm::LlmError> {
        // Extract the process from the map and release the lock immediately
        // so concurrent start/list/running_models calls aren't blocked
        // during the shutdown polling loop.
        let (mut child, pid, port) = {
            let mut procs = self.processes.lock();
            let proc =
                procs
                    .remove(model_id)
                    .ok_or_else(|| super::llm::LlmError::ProcessNotRunning {
                        model_id: model_id.to_string(),
                    })?;
            let pid = proc.child.id();
            let port = proc.port;
            (proc.child, pid, port)
        };
        // Lock is released here — polling loop runs without contention.

        // Best-effort graceful signal. NOTE: our children are spawned with
        // CREATE_NO_WINDOW (no console), so CTRL_BREAK can't reach them — this
        // is effectively a no-op on Windows and we proceed straight to a hard
        // kill below. Kept for the Unix SIGTERM path.
        graceful_stop(pid);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut exited = false;
        while std::time::Instant::now() < deadline {
            if !is_process_alive(pid) {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        if !exited {
            // Hard kill. `drop(child)` does NOT terminate on Windows — only
            // `Child::kill` (TerminateProcess) does — and the Job Object only
            // fires at Rift exit, not on a mid-session stop. So kill explicitly.
            let _ = child.kill();
        }
        // Reap so the OS process-handle is released (no zombie / handle leak).
        let _ = child.wait();

        tracing::info!(model_id, pid, "llm_process: stopped");

        publish_process_event(&self.bus, "llm.process.stop", model_id, pid, port, None);

        let mut pf = load_pid_file();
        pf.processes.retain(|e| e.pid != pid);
        save_pid_file(&pf);

        Ok(())
    }

    /// Force-stop every managed process immediately. Exit-path counterpart to
    /// the graceful [`stop`](Self::stop): no 5-second poll, because the app is
    /// shutting down and the exit watchdog will force-exit the process within
    /// seconds. Dropping a `std::process::Child` does NOT kill the OS process,
    /// so without this call llama-server lingers as an orphan after Rift exits.
    /// Returns the number of processes killed.
    pub fn kill_all(&self) -> usize {
        let entries: Vec<(String, u32)> = {
            let mut procs = self.processes.lock();
            procs.drain().map(|(id, p)| (id, p.child.id())).collect()
        };
        let count = entries.len();

        for (model_id, pid) in &entries {
            // Courtesy graceful signal first, then force — we do not wait.
            graceful_stop(*pid);
            #[cfg(windows)]
            {
                let mut cmd = Command::new("taskkill");
                cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
                cmd.creation_flags(CREATE_NO_WINDOW);
                let _ = cmd.output();
            }
            #[cfg(not(windows))]
            unsafe {
                libc::kill(*pid as i32, libc::SIGKILL);
            }
            tracing::info!(
                model_id = model_id.as_str(),
                pid,
                "llm_process: killed on exit"
            );
        }

        // These PIDs are all dead now — drop them from the orphan-recovery file.
        if count > 0 {
            let mut pf = load_pid_file();
            pf.processes
                .retain(|e| !entries.iter().any(|(_, pid)| *pid == e.pid));
            save_pid_file(&pf);
            tracing::info!(count, "llm_process: killed all managed processes on exit");
        }

        count
    }

    /// Check if a model's process is running and responsive.
    pub fn is_running(&self, model_id: &str) -> bool {
        let procs = self.processes.lock();
        procs
            .get(model_id)
            .map(|p| is_process_alive(p.child.id()))
            .unwrap_or(false)
    }

    /// Get the list of currently managed model IDs.
    pub fn running_models(&self) -> Vec<String> {
        self.processes.lock().keys().cloned().collect()
    }

    /// Check all managed processes for crashes and publish events. Crashed
    /// processes are dropped from the map; those whose config has
    /// `auto_restart` are re-spawned, bounded by [`MAX_RESTART_ATTEMPTS`] per
    /// [`RESTART_WINDOW`]. Called periodically by the health monitor loop.
    ///
    /// Detection is PID-death only (via `try_wait`) — a process that is alive
    /// but unresponsive (a hung server) is NOT yet detected here; `/health`
    /// liveness probing is future work.
    pub fn check_for_crashes(&self) {
        // Phase 1 — detect crashes and drop dead processes under the lock.
        // Capture each crashed model's config so it can be re-spawned after the
        // lock is released: start() re-acquires the lock, and restarting while
        // still holding it would deadlock (parking_lot mutexes aren't reentrant).
        let crashed: Vec<(String, LlamaServerConfig)> = {
            let mut procs = self.processes.lock();
            let mut crashed: Vec<(String, LlamaServerConfig)> = Vec::new();

            for (model_id, proc) in procs.iter_mut() {
                match proc.child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::warn!(
                            model_id = model_id.as_str(),
                            pid = proc.child.id(),
                            exit_code = ?status.code(),
                            "llm_process: crash detected"
                        );
                        publish_process_event(
                            &self.bus,
                            "llm.process.crash",
                            model_id,
                            proc.child.id(),
                            proc.port,
                            status.code().map(|c| c.to_string()),
                        );
                        crashed.push((model_id.clone(), proc.config.clone()));
                    }
                    Ok(None) => {} // still running
                    Err(e) => {
                        tracing::warn!(
                            model_id = model_id.as_str(),
                            "llm_process: try_wait error: {e}"
                        );
                    }
                }
            }

            for (id, _) in &crashed {
                procs.remove(id);
            }

            if !crashed.is_empty() {
                // Drop the dead PIDs from the orphan-recovery file.
                let alive_pids: Vec<u32> = procs.values().map(|p| p.child.id()).collect();
                let mut pf = load_pid_file();
                pf.processes.retain(|e| alive_pids.contains(&e.pid));
                save_pid_file(&pf);
            }

            crashed
        };
        // Lock released — safe to call start() (which re-locks) below.

        // Phase 2 — auto-restart eligible crashed models, capped per window so
        // a model that crashes on every launch (e.g. OOM) stops being respawned
        // and is left in an error state instead of looping forever.
        for (model_id, config) in crashed {
            if !config.auto_restart {
                continue;
            }
            if !self.register_restart_attempt(&model_id) {
                tracing::warn!(
                    model_id = model_id.as_str(),
                    "llm_process: auto-restart suppressed — {} attempts within {}s; leaving in error state",
                    MAX_RESTART_ATTEMPTS,
                    RESTART_WINDOW.as_secs()
                );
                continue;
            }
            match self.start(&model_id, &config) {
                Ok(pid) => tracing::info!(
                    model_id = model_id.as_str(),
                    pid,
                    "llm_process: auto-restarted after crash"
                ),
                Err(e) => tracing::warn!(
                    model_id = model_id.as_str(),
                    "llm_process: auto-restart failed: {e}"
                ),
            }
        }
    }

    /// Record an auto-restart attempt for `model_id` and report whether it is
    /// permitted under the per-window cap. The window resets once
    /// [`RESTART_WINDOW`] has elapsed since its first attempt, so a model that
    /// recovers and later crashes again gets a fresh budget.
    fn register_restart_attempt(&self, model_id: &str) -> bool {
        let now = Instant::now();
        let mut tracker = self.restart_tracker.lock();
        let info = tracker.entry(model_id.to_string()).or_insert(RestartInfo {
            attempts: 0,
            window_start: now,
        });
        if now.duration_since(info.window_start) > RESTART_WINDOW {
            info.attempts = 0;
            info.window_start = now;
        }
        if info.attempts >= MAX_RESTART_ATTEMPTS {
            return false;
        }
        info.attempts += 1;
        true
    }
}

// ---------------------------------------------------------------------------
// Bus event publishing
// ---------------------------------------------------------------------------

fn publish_process_event(
    bus: &RiftBus,
    kind: &str,
    model_id: &str,
    pid: u32,
    port: u16,
    exit_info: Option<String>,
) {
    #[derive(Serialize)]
    struct ProcessPayload<'a> {
        model_id: &'a str,
        pid: u32,
        port: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_info: Option<String>,
    }

    let payload = ProcessPayload {
        model_id,
        pid,
        port,
        exit_info,
    };

    match Envelope::new(Category::Llm, kind).with_payload(&payload) {
        Ok(env) => bus.publish(env),
        Err(e) => tracing::warn!("llm_process: failed to serialize {kind} envelope: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Health monitor loop
// ---------------------------------------------------------------------------

/// Spawn a background task that periodically checks all managed processes
/// for crashes and publishes health status. Runs until `shutdown` fires.
pub async fn spawn_health_monitor(
    manager: Arc<ProcessManager>,
    shutdown: Arc<Notify>,
    interval: Duration,
) {
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("llm_process: health monitor shutting down");
                break;
            }
            _ = tick.tick() => {
                manager.check_for_crashes();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_cli_args_basic() {
        let config = LlamaServerConfig {
            model_path: PathBuf::from("/models/test.gguf"),
            flash_attention: true,
            ctx_size: 4096,
            cache_type_k: crate::config::KvCacheType::Q8_0,
            cache_type_v: crate::config::KvCacheType::Q4_0,
            n_gpu_layers: 99,
            cpu_moe: false,
            n_cpu_moe: None,
            cache_ram: None,
            threads: Some(8),
            parallel: 2,
            port: 8081,
            cuda_visible_devices: None,
            auto_start: false,
            auto_restart: false,
            extra_flags: vec!["--verbose".to_string()],
        };

        let args = build_cli_args(&config);

        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"/models/test.gguf".to_string()));
        assert!(args.contains(&"--flash-attn".to_string()));
        // `--flash-attn` must be followed by an explicit value (on/off/auto);
        // the bare flag is rejected by modern llama-server.
        let fa = args.iter().position(|a| a == "--flash-attn").unwrap();
        assert_eq!(args[fa + 1], "on");
        assert!(args.contains(&"--ctx-size".to_string()));
        assert!(args.contains(&"4096".to_string()));
        assert!(args.contains(&"--cache-type-k".to_string()));
        assert!(args.contains(&"q8_0".to_string()));
        assert!(args.contains(&"--cache-type-v".to_string()));
        assert!(args.contains(&"q4_0".to_string()));
        assert!(args.contains(&"--n-gpu-layers".to_string()));
        assert!(args.contains(&"99".to_string()));
        assert!(args.contains(&"--threads".to_string()));
        assert!(args.contains(&"8".to_string()));
        assert!(args.contains(&"--parallel".to_string()));
        assert!(args.contains(&"2".to_string()));
        assert!(args.contains(&"--port".to_string()));
        assert!(args.contains(&"8081".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn build_cli_args_no_flash_no_threads() {
        let config = LlamaServerConfig {
            flash_attention: false,
            threads: None,
            extra_flags: vec![],
            ..Default::default()
        };

        let args = build_cli_args(&config);
        assert!(!args.contains(&"--flash-attn".to_string()));
        assert!(!args.contains(&"--threads".to_string()));
    }

    #[test]
    fn build_cli_args_auto_gpu_layers_omits_flag() {
        // Negative n_gpu_layers = auto: the flag must be omitted so
        // llama-server's device-memory fitter runs.
        let config = LlamaServerConfig {
            n_gpu_layers: -1,
            ..Default::default()
        };
        let args = build_cli_args(&config);
        assert!(!args.contains(&"--n-gpu-layers".to_string()));
    }

    #[test]
    fn build_cli_args_cpu_moe_flags() {
        // `--cpu-moe` (all experts) takes precedence over `--n-cpu-moe`.
        let both = LlamaServerConfig {
            cpu_moe: true,
            n_cpu_moe: Some(20),
            ..Default::default()
        };
        let args = build_cli_args(&both);
        assert!(args.contains(&"--cpu-moe".to_string()));
        assert!(!args.contains(&"--n-cpu-moe".to_string()));

        // `--n-cpu-moe N` only when cpu_moe is false.
        let partial = LlamaServerConfig {
            cpu_moe: false,
            n_cpu_moe: Some(20),
            ..Default::default()
        };
        let args = build_cli_args(&partial);
        assert!(!args.contains(&"--cpu-moe".to_string()));
        let pos = args.iter().position(|a| a == "--n-cpu-moe").unwrap();
        assert_eq!(args[pos + 1], "20");

        // Neither flag when both are unset (default).
        let none = LlamaServerConfig::default();
        let args = build_cli_args(&none);
        assert!(!args.contains(&"--cpu-moe".to_string()));
        assert!(!args.contains(&"--n-cpu-moe".to_string()));
    }

    #[test]
    fn build_cli_args_cache_ram() {
        // None → flag omitted (llama-server default 8 GiB).
        assert!(!build_cli_args(&LlamaServerConfig::default()).contains(&"--cache-ram".to_string()));

        // Some(0) → `--cache-ram 0` (disable the prompt cache).
        let off = LlamaServerConfig {
            cache_ram: Some(0),
            ..Default::default()
        };
        let args = build_cli_args(&off);
        let pos = args.iter().position(|a| a == "--cache-ram").unwrap();
        assert_eq!(args[pos + 1], "0");
    }

    #[test]
    fn pid_file_round_trips() {
        let pf = PidFile {
            processes: vec![
                PidEntry {
                    model_id: "local-gemma".to_string(),
                    pid: 12345,
                    port: 8081,
                },
                PidEntry {
                    model_id: "server-fast".to_string(),
                    pid: 67890,
                    port: 8082,
                },
            ],
        };

        let json = serde_json::to_string(&pf).expect("serialize");
        let back: PidFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.processes.len(), 2);
        assert_eq!(back.processes[0].model_id, "local-gemma");
        assert_eq!(back.processes[0].pid, 12345);
        assert_eq!(back.processes[1].port, 8082);
    }

    #[test]
    fn process_payload_serializes() {
        #[derive(Serialize)]
        struct ProcessPayload {
            model_id: String,
            pid: u32,
            port: u16,
            #[serde(skip_serializing_if = "Option::is_none")]
            exit_info: Option<String>,
        }

        let payload = ProcessPayload {
            model_id: "test".to_string(),
            pid: 1234,
            port: 8081,
            exit_info: None,
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("\"model_id\":\"test\""));
        assert!(json.contains("\"pid\":1234"));
        assert!(!json.contains("exit_info"));
    }

    #[test]
    fn auto_restart_capped_per_window() {
        let pm = ProcessManager::new("llama-server", RiftBus::default());
        // The first MAX_RESTART_ATTEMPTS attempts within a window are permitted...
        for _ in 0..MAX_RESTART_ATTEMPTS {
            assert!(pm.register_restart_attempt("m1"));
        }
        // ...and the next is suppressed, so a model that crashes on every launch
        // stops being respawned instead of looping forever.
        assert!(!pm.register_restart_attempt("m1"));
        // A different model tracks its own independent restart budget.
        assert!(pm.register_restart_attempt("m2"));
    }
}
