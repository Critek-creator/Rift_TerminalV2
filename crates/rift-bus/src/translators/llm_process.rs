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
use std::time::Duration;

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
        "--n-gpu-layers".to_string(),
        config.n_gpu_layers.to_string(),
        "--cache-type-k".to_string(),
        config.cache_type_k.as_flag().to_string(),
        "--cache-type-v".to_string(),
        config.cache_type_v.as_flag().to_string(),
        "--parallel".to_string(),
        config.parallel.to_string(),
    ];

    if config.flash_attention {
        args.push("--flash-attn".to_string());
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
// ProcessManager
// ---------------------------------------------------------------------------

/// Manages local llama-server child processes. Thread-safe — holds state
/// behind a `parking_lot::Mutex` for consistency with the rest of the
/// codebase (8 crash paths fixed by the parking_lot migration).
pub struct ProcessManager {
    llama_server_path: PathBuf,
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
    bus: RiftBus,
}

impl ProcessManager {
    pub fn new(llama_server_path: impl Into<PathBuf>, bus: RiftBus) -> Self {
        Self {
            llama_server_path: llama_server_path.into(),
            processes: Arc::new(Mutex::new(HashMap::new())),
            bus,
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
        let mut procs = self.processes.lock();

        let proc =
            procs
                .remove(model_id)
                .ok_or_else(|| super::llm::LlmError::ProcessNotRunning {
                    model_id: model_id.to_string(),
                })?;

        let pid = proc.child.id();
        let port = proc.port;

        // Attempt graceful shutdown
        graceful_stop(pid);

        // Wait up to 5 seconds for exit
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut exited = false;
        while std::time::Instant::now() < deadline {
            if !is_process_alive(pid) {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        // Force kill if still alive
        if !exited {
            tracing::warn!(
                model_id,
                pid,
                "llm_process: graceful stop timed out, force killing"
            );
            drop(proc.child);
            // Child::drop calls TerminateProcess on Windows
        }

        tracing::info!(model_id, pid, "llm_process: stopped");

        publish_process_event(&self.bus, "llm.process.stop", model_id, pid, port, None);

        // Remove from PID file
        let mut pf = load_pid_file();
        pf.processes.retain(|e| e.pid != pid);
        save_pid_file(&pf);

        Ok(())
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

    /// Check all managed processes for crashes and publish events.
    /// Called periodically by the health monitor loop.
    pub fn check_for_crashes(&self) {
        let mut procs = self.processes.lock();
        let mut crashed: Vec<String> = Vec::new();

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
                    crashed.push(model_id.clone());
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

        for id in &crashed {
            procs.remove(id);
        }

        if !crashed.is_empty() {
            // Update PID file
            let alive_pids: Vec<u32> = procs.values().map(|p| p.child.id()).collect();
            let mut pf = load_pid_file();
            pf.processes.retain(|e| alive_pids.contains(&e.pid));
            save_pid_file(&pf);
        }
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
            threads: Some(8),
            parallel: 2,
            port: 8081,
            cuda_visible_devices: None,
            auto_start: false,
            extra_flags: vec!["--verbose".to_string()],
        };

        let args = build_cli_args(&config);

        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"/models/test.gguf".to_string()));
        assert!(args.contains(&"--flash-attn".to_string()));
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
}
