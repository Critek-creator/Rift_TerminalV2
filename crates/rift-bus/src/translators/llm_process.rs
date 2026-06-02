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
use std::process::{Child, Command, Stdio};
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

/// Per-model log path under `<config_dir>/logs/llama-<model>-<port>.log`.
/// Captures the llama-server's stdout+stderr so crash causes (CUDA OOM,
/// asserts) are diagnosable instead of being discarded by `CREATE_NO_WINDOW`.
fn llm_log_path(model_id: &str, port: u16) -> Result<PathBuf, crate::config::ConfigError> {
    let dirs = directories::ProjectDirs::from("com", "abyssal", "rift")
        .ok_or(crate::config::ConfigError::NoConfigDir)?;
    let safe: String = model_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(dirs
        .config_dir()
        .join("logs")
        .join(format!("llama-{safe}-{port}.log")))
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
        // `--jinja` activates llama-server's OpenAI-compatible tool-calling layer:
        // it renders the request's `tools` array into the model's chat template
        // and auto-applies a lazy GBNF grammar so tool-call JSON is always
        // syntactically valid. REQUIRED for any tool calling (without it the
        // `tools` parameter is silently ignored). Also the modern-recommended
        // templating mode. Behavior note: all launches now use the GGUF's
        // embedded jinja chat template instead of llama.cpp's legacy built-in.
        "--jinja".to_string(),
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
// VRAM footprint estimation
// ---------------------------------------------------------------------------

/// Bytes-per-element of a KV cache quant, scaled ×100 so fractional widths
/// (q8_0 ≈ 1.06 B, q4_0 ≈ 0.56 B) stay in integer math. Unknown → f16.
fn cache_type_bytes_x100(flag: &str) -> u64 {
    match flag {
        "f32" => 400,
        "f16" | "bf16" => 200,
        "q8_0" => 106,
        "q5_0" | "q5_1" => 69,
        "q4_0" | "q4_1" | "iq4_nl" => 56,
        _ => 200,
    }
}

/// Estimate the GPU VRAM (MiB) a model will occupy once started with `config`.
///
/// Two terms:
/// - **weights** ≈ the GGUF file size on disk (full-offload assumption; with
///   partial offload or `--cpu-moe` the real GPU share is smaller, so this
///   over-estimates — the safe direction for an OOM guard).
/// - **KV cache** = n_layers · n_head_kv · head_dim · ctx · (k+v bytes), read
///   from the GGUF architecture header. This is the term that scales with
///   `ctx_size`. Sliding-window / MoE models allocate less than this in
///   practice, so the estimate runs high there too — again the safe direction.
///
/// Plus a fixed CUDA-context overhead. Best-effort: an unreadable file or GGUF
/// header degrades to weights-only (+ a flat KV fraction).
fn estimate_vram_mb(config: &LlamaServerConfig) -> u64 {
    const MIB: u64 = 1024 * 1024;
    let path = config.model_path.as_path();
    let weights_mb = std::fs::metadata(path).map(|m| m.len() / MIB).unwrap_or(0);

    let kv_mb = match super::gguf::inspect(path) {
        Ok(meta) => {
            let n_layers = meta.n_layers.unwrap_or(0) as u64;
            let n_head_u32 = meta.n_head.unwrap_or(0);
            let n_head = n_head_u32 as u64;
            let n_head_kv = meta.n_head_kv.unwrap_or(n_head_u32).max(1) as u64;
            let n_embd = meta.n_embd.unwrap_or(0) as u64;
            if n_layers > 0 && n_head > 0 && n_embd > 0 {
                let head_dim = n_embd / n_head;
                let ctx = config.ctx_size as u64;
                let bk = cache_type_bytes_x100(config.cache_type_k.as_flag());
                let bv = cache_type_bytes_x100(config.cache_type_v.as_flag());
                // K and V tensors: per token, per layer, n_head_kv · head_dim
                // elements each. Scaled ×100 widths divided back out at the end.
                let elems = n_layers
                    .saturating_mul(n_head_kv)
                    .saturating_mul(head_dim)
                    .saturating_mul(ctx);
                elems.saturating_mul(bk + bv) / 100 / MIB
            } else {
                // Header present but missing the dims we need — rough fraction.
                weights_mb / 8
            }
        }
        Err(_) => weights_mb / 8,
    };

    weights_mb + kv_mb + VRAM_CUDA_OVERHEAD_MB
}

/// Real free VRAM on the primary GPU via `nvidia-smi`, in MiB. `None` when the
/// tool is absent or errors (non-NVIDIA hosts), signalling the fallback budget.
fn query_gpu_free_mb() -> Option<u64> {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args(["--query-gpu=memory.free", "--format=csv,noheader,nounits"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .parse::<u64>()
        .ok()
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

/// Outcome of [`ProcessManager::apply_config`] — whether reconciling a running
/// server with new config required a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Model is not currently running — nothing to apply.
    NotRunning,
    /// Running launch args already match the new config — no restart needed.
    Unchanged,
    /// Launch args drifted — the server was stopped and restarted on new config.
    Restarted,
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

// ---------------------------------------------------------------------------
// VRAM admission guard
// ---------------------------------------------------------------------------
//
// A 16 GB card holds exactly one large local model at a time; manually
// enabling a second while the first is still resident OOMs llama-server at
// load. The guard (see [`ProcessManager::enforce_vram_budget`]) measures real
// free VRAM, estimates the incoming model's footprint, and evicts the largest
// co-resident managed model(s) until it fits — erring toward eviction, never
// refusing.

/// Master switch for the VRAM admission guard. On by default; flip to disable
/// the evict-to-fit behavior (e.g. once a second GPU makes co-residence safe).
const VRAM_GUARD_ENABLED: bool = true;

/// Safety margin kept free above the incoming model's estimate, in MiB.
const VRAM_HEADROOM_MB: u64 = 512;

/// Assumed usable VRAM (MiB) when `nvidia-smi` can't be queried (non-NVIDIA, or
/// the tool is absent). The guard then budgets against estimated resident
/// footprints instead of measured free memory.
const VRAM_FALLBACK_BUDGET_MB: u64 = 15360;

/// Fixed driver/CUDA-context allocation beyond weights + KV, in MiB.
const VRAM_CUDA_OVERHEAD_MB: u64 = 400;

/// Upper bound on evictions per start, so a mis-estimate can't loop forever.
const VRAM_MAX_EVICTIONS: u32 = 6;

/// Pause after a stop so the driver reclaims the freed VRAM before re-measuring.
const VRAM_SETTLE: Duration = Duration::from_millis(700);

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
        // Fast pre-check, lock released immediately — skip the VRAM guard (and
        // its nvidia-smi probe) for an already-running model.
        if self.processes.lock().contains_key(model_id) {
            return Err(super::llm::LlmError::Internal {
                message: format!("model {model_id} already running"),
            });
        }

        // Admission control: evict co-resident models if this start would not
        // fit in VRAM. Runs WITHOUT the process lock held (it may call stop()).
        self.enforce_vram_budget(model_id, config);

        let mut procs = self.processes.lock();

        // Re-check under the lock: another start could have raced in during the
        // guard's lock-free window above.
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

        // Capture stdout+stderr to a per-model log so a crash's actual cause
        // (CUDA OOM, llama.cpp assert) is recoverable instead of being silently
        // discarded by CREATE_NO_WINDOW. Best-effort — the spawn proceeds even
        // if the log cannot be opened. Append (not truncate) so an auto-restart
        // after a crash does not erase the crashing run's output.
        if let Ok(log_path) = llm_log_path(model_id, config.port) {
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                match file.try_clone() {
                    Ok(err_handle) => {
                        cmd.stdout(Stdio::from(file));
                        cmd.stderr(Stdio::from(err_handle));
                    }
                    Err(_) => {
                        cmd.stderr(Stdio::from(file));
                    }
                }
            }
        }

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

    /// VRAM admission control, run before a start. Ensures enough GPU memory is
    /// free for `config` by evicting the largest *other* managed model(s) until
    /// it fits — or until nothing else is managed. Prefers real `nvidia-smi`
    /// free VRAM; falls back to a fixed budget minus estimated resident
    /// footprints when `nvidia-smi` is unavailable.
    ///
    /// Never refuses: the worst case clears every co-resident and lets
    /// llama-server try, matching the operating model "enable one, evict the
    /// others unless they're certain to both fit." Acquires/releases the process
    /// lock internally (it calls [`stop`](Self::stop)), so callers must NOT hold
    /// it.
    fn enforce_vram_budget(&self, model_id: &str, config: &LlamaServerConfig) {
        if !VRAM_GUARD_ENABLED {
            return;
        }
        let need = estimate_vram_mb(config);

        for _ in 0..VRAM_MAX_EVICTIONS {
            let free = self.available_vram_mb(model_id);
            if need + VRAM_HEADROOM_MB <= free {
                return; // fits as-is — co-residence is safe
            }

            // Evict the largest other managed resident and re-measure.
            let victim = {
                let procs = self.processes.lock();
                procs
                    .values()
                    .filter(|p| p.model_id != model_id)
                    .max_by_key(|p| estimate_vram_mb(&p.config))
                    .map(|p| p.model_id.clone())
            };

            match victim {
                Some(v) => {
                    tracing::warn!(
                        model = %model_id,
                        evicting = %v,
                        need_mb = need,
                        free_mb = free,
                        "vram guard: evicting co-resident model to make room"
                    );
                    publish_process_event(
                        &self.bus,
                        "llm.process.evicted",
                        &v,
                        0,
                        0,
                        Some(format!(
                            "evicted to free VRAM for {model_id} (need ~{need} MiB, free ~{free} MiB)"
                        )),
                    );
                    if self.stop(&v).is_err() {
                        return; // can't evict — proceed best-effort
                    }
                    std::thread::sleep(VRAM_SETTLE);
                }
                None => return, // nothing left to evict — proceed best-effort
            }
        }
    }

    /// Free VRAM (MiB) available for a new start. Real `nvidia-smi` free when
    /// available; otherwise [`VRAM_FALLBACK_BUDGET_MB`] minus the estimated
    /// footprint of currently-managed residents (excluding `exclude_id`).
    fn available_vram_mb(&self, exclude_id: &str) -> u64 {
        if let Some(free) = query_gpu_free_mb() {
            return free;
        }
        let used: u64 = {
            let procs = self.processes.lock();
            procs
                .values()
                .filter(|p| p.model_id != exclude_id)
                .map(|p| estimate_vram_mb(&p.config))
                .sum()
        };
        VRAM_FALLBACK_BUDGET_MB.saturating_sub(used)
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

    /// Reconcile a *running* server with `new_config`.
    ///
    /// llama-server reads launch flags (`--ctx-size`, `--n-gpu-layers`,
    /// `--flash-attn`, cache types, port, model path, …) only at spawn time, so
    /// a model left running across a config edit keeps its OLD flags until an
    /// explicit restart — the stale-server trap (a server pinned an old
    /// `ctx_size` after the config was changed). This method compares the
    /// running process's launch args against what `new_config` would produce
    /// (via the same [`build_cli_args`] used to spawn it) and:
    ///
    /// - [`ApplyOutcome::NotRunning`] — model isn't running; nothing to apply
    ///   (the next [`start`](Self::start) reads the persisted config anyway).
    /// - [`ApplyOutcome::Unchanged`] — launch args identical; no restart.
    /// - [`ApplyOutcome::Restarted`] — args drifted; the server was stopped and
    ///   restarted on `new_config` so the change takes effect.
    pub fn apply_config(
        &self,
        model_id: &str,
        new_config: &LlamaServerConfig,
    ) -> Result<ApplyOutcome, super::llm::LlmError> {
        // Snapshot the running config's launch args under a brief lock, then
        // release before calling stop()/start() (each takes the lock itself).
        let current_args = {
            let procs = self.processes.lock();
            match procs.get(model_id) {
                Some(p) => build_cli_args(&p.config),
                None => return Ok(ApplyOutcome::NotRunning),
            }
        };

        if current_args == build_cli_args(new_config) {
            return Ok(ApplyOutcome::Unchanged);
        }

        tracing::info!(
            model_id,
            "llm_process: launch args drifted — restarting to apply new config"
        );
        self.stop(model_id)?;
        self.start(model_id, new_config)?;
        Ok(ApplyOutcome::Restarted)
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

    /// Model IDs whose process is currently ALIVE (PID responsive), not just
    /// present in the managed map. Unlike [`running_models`](Self::running_models)
    /// this filters by liveness, so a crashed-but-not-yet-reaped entry is
    /// excluded. Used to seed router availability so auto-routing never targets
    /// a stopped llama-server (the connection-fail cascade). One lock acquire.
    pub fn live_models(&self) -> Vec<String> {
        let procs = self.processes.lock();
        procs
            .iter()
            .filter(|(_, p)| is_process_alive(p.child.id()))
            .map(|(id, _)| id.clone())
            .collect()
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
    fn cache_type_bytes_known_and_unknown() {
        assert_eq!(cache_type_bytes_x100("f16"), 200);
        assert_eq!(cache_type_bytes_x100("bf16"), 200);
        assert_eq!(cache_type_bytes_x100("f32"), 400);
        assert_eq!(cache_type_bytes_x100("q8_0"), 106);
        assert_eq!(cache_type_bytes_x100("q4_0"), 56);
        assert_eq!(cache_type_bytes_x100("iq4_nl"), 56);
        // Unknown flag falls back to f16 width, never zero.
        assert_eq!(cache_type_bytes_x100("mystery"), 200);
    }

    #[test]
    fn estimate_vram_missing_file_is_overhead_only() {
        // No weights readable, no GGUF header → just the fixed CUDA overhead.
        let config = LlamaServerConfig {
            model_path: PathBuf::from("/nonexistent/model-does-not-exist.gguf"),
            flash_attention: true,
            ctx_size: 32768,
            cache_type_k: crate::config::KvCacheType::Q8_0,
            cache_type_v: crate::config::KvCacheType::Q8_0,
            n_gpu_layers: 99,
            cpu_moe: false,
            n_cpu_moe: None,
            cache_ram: None,
            threads: None,
            parallel: 1,
            port: 8086,
            cuda_visible_devices: None,
            auto_start: false,
            auto_restart: false,
            extra_flags: vec![],
        };
        assert_eq!(estimate_vram_mb(&config), VRAM_CUDA_OVERHEAD_MB);
    }

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
        // `--jinja` must always be present — it activates OpenAI tool calling.
        assert!(args.contains(&"--jinja".to_string()));
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
