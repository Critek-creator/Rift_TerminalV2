//! PTY layer — `portable-pty` 0.9 wrapper with split-handle pattern.
//!
//! ## Architecture
//!
//! [`PtySession::spawn`] returns `(PtyOutput, PtyControl)`:
//!
//! * [`PtyOutput`] owns the byte stream. Move it into a drain task
//!   that forwards bytes onto a `tauri::ipc::Channel<Vec<u8>>` per
//!   `decisions/§10.15_real-time_update_mechanism.md` (Tier 1).
//! * [`PtyControl`] owns the writer + master + child handle.
//!   Hold it in a session registry so `pty_write` / `pty_resize` /
//!   `pty_kill` Tauri commands can address sessions by id.
//!
//! ## Threads at runtime
//!
//! Each spawned session creates two OS threads:
//!
//! 1. **Reader thread** — drains the PTY master, forwards 4 KiB chunks to a
//!    tokio `mpsc::UnboundedSender<Vec<u8>>`. Exits when `read` returns 0
//!    (EOF) or errors.
//! 2. **Exit-watcher thread** — polls `child.try_wait()` every 250 ms. When
//!    the child exits, sets the shared `alive` flag false and resolves a
//!    one-shot exit-code receiver. This pattern is required on Windows
//!    because ConPTY's master pipe does not always close cleanly when the
//!    child exits — without the watcher, the reader thread can hang
//!    indefinitely. (Lesson `pty-exit-windows`, ported from V1.)
//!
//! Both threads exit when the session is dropped or killed. Caller is
//! responsible for `await`ing the exit oneshot to learn the exit code.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use std::thread;
use std::time::Duration;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::shell::default_shell;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("pty open failed: {0}")]
    OpenFailed(String),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("write failed: {0}")]
    WriteFailed(#[from] std::io::Error),
    #[error("resize failed: {0}")]
    ResizeFailed(String),
    #[error("kill failed: {0}")]
    KillFailed(String),
}

/// Terminal dimensions in cells + (optional) pixels.
/// `pixel_width` / `pixel_height` may be 0; some kernels do not use them.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct PtyDims {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl Default for PtyDims {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl From<PtyDims> for PtySize {
    fn from(d: PtyDims) -> Self {
        PtySize {
            rows: d.rows,
            cols: d.cols,
            pixel_width: d.pixel_width,
            pixel_height: d.pixel_height,
        }
    }
}

/// Spawn-time options. Use [`PtyOptions::new`] then chain `with_env` to
/// inject environment variables — useful for passing the running Rift
/// instance's IPC socket name into spawned shells so `rift hook ...`
/// works without manual setup.
///
/// Phase 8.7g.4: `cwd` lets the caller pin the spawned shell's working
/// directory. When `None`, falls back to `std::env::current_dir()` (the
/// previous unconditional behavior). Tauri callers should always pass
/// the canonical ProjectRoot — `cargo run` starts the binary from
/// `src-tauri/`, which would otherwise leak into the user's shell prompt.
#[derive(Clone, Debug, Default)]
pub struct PtyOptions {
    pub dims: PtyDims,
    pub env: Vec<(String, String)>,
    pub cwd: Option<std::path::PathBuf>,
    /// Pre-resolved shell binary + args. When `Some`, overrides the
    /// `default_shell()` resolution that `spawn_with_options` would
    /// otherwise apply. Callers feeding a config-driven `ShellPref` resolve
    /// it via `crate::shell::{resolve_auto_shell, resolve_named_shell,
    /// resolve_custom_shell}` and pass the result here.
    pub shell: Option<(std::path::PathBuf, Vec<String>)>,
}

impl PtyOptions {
    pub fn new(dims: PtyDims) -> Self {
        Self {
            dims,
            env: Vec::new(),
            cwd: None,
            shell: None,
        }
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<std::path::PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Override the shell that would otherwise be resolved via
    /// `default_shell()`. Pass an absolute path + args list (args is
    /// usually empty for interactive shells).
    pub fn with_shell(mut self, path: impl Into<std::path::PathBuf>, args: Vec<String>) -> Self {
        self.shell = Some((path.into(), args));
        self
    }
}

/// Drain side: byte stream from the PTY + a one-shot exit notification.
pub struct PtyOutput {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    exit: Option<oneshot::Receiver<u32>>,
}

impl PtyOutput {
    /// Receive the next byte chunk from the PTY. Returns `None` when the
    /// reader thread has terminated (EOF, error, or kill).
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await
    }

    /// Take the one-shot exit-code receiver. Resolves once the child has
    /// exited (and the watcher has noticed). Returns `None` after the
    /// receiver has been taken once.
    pub fn take_exit(&mut self) -> Option<oneshot::Receiver<u32>> {
        self.exit.take()
    }
}

/// Control side: write to stdin, resize the TTY, kill the child.
/// Cheap to clone via `Arc`; safe to share across tasks.
#[derive(Clone)]
pub struct PtyControl {
    inner: Arc<PtyControlInner>,
}

struct PtyControlInner {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    alive: Arc<AtomicBool>,
}

impl PtyControl {
    /// Write bytes to the PTY's stdin. Errors propagate from the underlying
    /// writer; `flush` is invoked on every call to ensure interactive shells
    /// see input promptly.
    pub fn write(&self, bytes: &[u8]) -> Result<(), PtyError> {
        let mut guard = self.inner.writer.lock();
        guard.write_all(bytes)?;
        guard.flush()?;
        Ok(())
    }

    /// Resize the PTY. Sends `SIGWINCH` on Unix, `ResizePty` on Windows.
    pub fn resize(&self, dims: PtyDims) -> Result<(), PtyError> {
        self.inner
            .master
            .lock()
            .resize(dims.into())
            .map_err(|e| PtyError::ResizeFailed(e.to_string()))
    }

    /// Force-terminate the child. The exit-watcher thread will observe
    /// the exit on its next 250 ms tick and send the code through the
    /// one-shot.
    pub fn kill(&self) -> Result<(), PtyError> {
        self.inner
            .child
            .lock()
            .kill()
            .map_err(|e| PtyError::KillFailed(e.to_string()))?;
        self.inner.alive.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Returns `false` after the exit-watcher has detected child termination
    /// (or after `kill` was called).
    pub fn is_alive(&self) -> bool {
        self.inner.alive.load(Ordering::SeqCst)
    }

    /// Return the PID of the root child process (the shell). Used by L3
    /// process-name detection to walk the child tree looking for known
    /// binaries (e.g. `claude.exe`). Returns `None` if the portable-pty
    /// backend doesn't support process-id retrieval.
    pub fn child_pid(&self) -> Option<u32> {
        self.inner.child.lock().process_id()
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

pub struct PtySession;

impl PtySession {
    /// Open a fresh PTY, spawn the platform default shell, and return the
    /// split (output, control) handle pair. See module docs for thread
    /// lifecycle. Equivalent to [`PtySession::spawn_with_options`] with
    /// no extra environment variables.
    pub fn spawn(dims: PtyDims) -> Result<(PtyOutput, PtyControl), PtyError> {
        Self::spawn_with_options(PtyOptions::new(dims))
    }

    /// Same as [`PtySession::spawn`] but accepts a [`PtyOptions`] so the
    /// caller can inject extra environment variables (e.g. the running
    /// Rift instance's `RIFT_SOCKET_NAME` so `rift hook ...` works
    /// without manual setup).
    pub fn spawn_with_options(opts: PtyOptions) -> Result<(PtyOutput, PtyControl), PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(opts.dims.into())
            .map_err(|e| PtyError::OpenFailed(e.to_string()))?;

        // PtyOptions.shell wins when set (config-driven `ShellPref`); fall
        // back to default_shell() for legacy callers (CLI tests).
        let (shell, args) = opts.shell.clone().unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(shell);
        for arg in args {
            cmd.arg(arg);
        }
        // Phase 8.7g.4 — explicit cwd from PtyOptions takes precedence so
        // `cargo run` (which starts the binary from src-tauri/) doesn't
        // leak the wrong cwd into the spawned shell. Fallback to
        // env::current_dir() preserves prior behavior for callers (CLI,
        // tests) that don't pin the cwd themselves.
        let chosen_cwd = opts.cwd.clone().or_else(|| std::env::current_dir().ok());
        if let Some(cwd) = chosen_cwd {
            cmd.cwd(cwd);
        }
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let alive = Arc::new(AtomicBool::new(true));
        let child = Arc::new(Mutex::new(child));
        let (output_tx, output_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (exit_tx, exit_rx) = oneshot::channel::<u32>();

        // Reader OS thread: drain PTY → mpsc.
        let alive_reader = alive.clone();
        thread::Builder::new()
            .name("rift-pty-reader".into())
            .spawn(move || reader_loop(reader, output_tx, alive_reader))
            .map_err(|e| PtyError::SpawnFailed(format!("spawn reader thread: {e}")))?;

        // Exit-watcher OS thread: poll child.try_wait every 250 ms.
        // Required on Windows (ConPTY pipe may not close cleanly); harmless
        // on Unix. Per pr003 lesson `pty-exit-windows`.
        let alive_watcher = alive.clone();
        let child_watcher = child.clone();
        thread::Builder::new()
            .name("rift-pty-exit-watcher".into())
            .spawn(move || exit_watcher_loop(child_watcher, alive_watcher, exit_tx))
            .map_err(|e| PtyError::SpawnFailed(format!("spawn watcher thread: {e}")))?;

        let control = PtyControl {
            inner: Arc::new(PtyControlInner {
                master: Mutex::new(pair.master),
                writer: Mutex::new(writer),
                child,
                alive,
            }),
        };
        let output = PtyOutput {
            rx: output_rx,
            exit: Some(exit_rx),
        };
        Ok((output, control))
    }
}

// ---------------------------------------------------------------------------
// Worker loops (private)
// ---------------------------------------------------------------------------

fn reader_loop(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    alive: Arc<AtomicBool>,
) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    // Receiver dropped — session is gone.
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    alive.store(false, Ordering::SeqCst);
}

fn exit_watcher_loop(
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    alive: Arc<AtomicBool>,
    exit_tx: oneshot::Sender<u32>,
) {
    let exit_code: u32 = loop {
        thread::sleep(Duration::from_millis(250));
        let status = {
            let mut guard = child.lock();
            match guard.try_wait() {
                Ok(Some(s)) => Some(s),
                Ok(None) => None,
                Err(_) => break u32::MAX,
            }
        };
        if let Some(s) = status {
            break s.exit_code();
        }
    };
    alive.store(false, Ordering::SeqCst);
    // `send` returns Err if the receiver was dropped — we don't care.
    let _ = exit_tx.send(exit_code);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration as TokioDuration};

    /// Spawning the default shell and reading from it produces *some* bytes
    /// within a few seconds — this exercises the full pipe: reader OS thread
    /// → mpsc → tokio task. Kept platform-neutral by not asserting any
    /// particular byte content (`cmd.exe` and POSIX shells differ on prompt
    /// shape, encoding, line endings).
    #[tokio::test]
    async fn shell_produces_output_after_spawn() {
        let (mut output, control) = PtySession::spawn(PtyDims::default()).expect("spawn pty");
        let bytes = timeout(TokioDuration::from_secs(5), output.recv())
            .await
            .expect("first byte chunk should arrive within 5s")
            .expect("rx should be open");
        assert!(!bytes.is_empty(), "first chunk should be non-empty");
        // Clean up: kill, await exit oneshot, drop control.
        control.kill().expect("kill");
        let _ = output.take_exit().unwrap().await;
        drop(control);
    }

    /// Writer accepts bytes without panicking. Behaviour of `cmd.exe` under
    /// ConPTY w.r.t. echoing input back through the master is shell-specific
    /// and flaky to assert in unit tests — the input→output round-trip is
    /// exercised end-to-end via `npm run tauri:dev` (manual acceptance for
    /// Phase 1.4).
    #[tokio::test]
    async fn writer_accepts_bytes() {
        let (mut output, control) = PtySession::spawn(PtyDims::default()).expect("spawn pty");
        let _ = timeout(TokioDuration::from_millis(500), output.recv()).await;
        control.write(b"echo hi\r\n").expect("write does not error");
        control.kill().expect("kill");
        let _ = output.take_exit().unwrap().await;
        drop(control);
    }

    #[tokio::test]
    async fn resize_does_not_panic() {
        let (mut output, control) = PtySession::spawn(PtyDims::default()).expect("spawn pty");
        control
            .resize(PtyDims {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize");
        // Drain whatever the shell wrote, then kill.
        let _ = timeout(TokioDuration::from_millis(300), output.recv()).await;
        control.kill().expect("kill");
        let _ = output.take_exit().unwrap().await;
    }

    #[tokio::test]
    async fn kill_terminates_session() {
        let (mut output, control) = PtySession::spawn(PtyDims::default()).expect("spawn pty");
        control.kill().expect("kill");
        let exit = output.take_exit().unwrap();
        timeout(TokioDuration::from_secs(5), exit)
            .await
            .expect("exit within 5s")
            .expect("oneshot resolves");
        assert!(!control.is_alive());
    }
}
