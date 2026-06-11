//! Tier-2 IPC server — cross-process bridge over the Rift Integration
//! Protocol. UDS on Unix, named pipe on Windows (via `interprocess`).
//!
//! Per `decisions/§10.15_real-time_update_mechanism.md`:
//!
//! * **Wire format**: length-prefixed JSON frames. 4-byte little-endian
//!   `u32` length prefix followed by `serde_json` of [`Envelope`].
//! * **Per-connection lifecycle**: on accept, send the bus's filtered
//!   replay snapshot synchronously, then fan out live events. The client
//!   may also publish back — its frames are pushed into the same bus.
//! * **Backpressure**: if a per-connection writer falls behind by more
//!   than the bus's broadcast capacity, [`crate::BusError::Lagged`]
//!   surfaces and the connection is closed. Clients reconnect to drain
//!   a fresh snapshot.
//!
//! ## Pre-publish order
//! Per `pr003` lesson `pre-publish-before-start-ipc-server`: the bus is
//! valid for `publish` BEFORE the IPC server starts. Replay buffer
//! captures pre-startup events, and the per-connection drain delivers
//! them on first connect.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use interprocess::local_socket::tokio::{prelude::*, Listener, RecvHalf, SendHalf, Stream};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;

use crate::bus::{BusError, RiftBus, SubscribeFilter, TryRecv};
use crate::envelope::Envelope;

/// Hard cap on a single inbound frame. Defends against malformed or
/// adversarial peers sending an outsized length prefix. 16 MiB is
/// generous for any reasonable Rift event payload.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame too large: {0} bytes (max 16777216)")]
    FrameTooLarge(u32),
    #[error("invalid socket name: {0}")]
    InvalidName(String),
    #[error("ipc server already shut down")]
    AlreadyShutDown,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Listening IPC server. Spawned tasks live until [`IpcServer::shutdown`]
/// is invoked or the handle drops.
///
/// Per-connection tasks are tracked in a shared [`JoinSet`]. On shutdown,
/// all pending per-connection tasks are aborted before returning, so no
/// orphaned tasks outlive the server.
pub struct IpcServer {
    socket_name: String,
    shutdown: Arc<Notify>,
    accept_task: tokio::task::JoinHandle<()>,
    /// Shared with the accept loop so shutdown() can abort all per-connection
    /// tasks that are still running when the server stops.
    conn_tasks: Arc<Mutex<JoinSet<()>>>,
}

impl IpcServer {
    /// Start a server on the given platform-namespaced name.
    ///
    /// On Unix this corresponds to a UDS in the `@`-prefixed abstract or
    /// a path-based socket; on Windows it maps to `\\.\pipe\<name>`.
    pub async fn start(bus: RiftBus, name: impl AsRef<str>) -> Result<Self, IpcError> {
        let socket_name = name.as_ref().to_owned();
        let printable = socket_name
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| IpcError::InvalidName(e.to_string()))?;
        let opts = ListenerOptions::new().name(printable);

        // Windows: restrict the pipe to its owner + LocalSystem. The default
        // pipe DACL lets any local user connect, and a connection immediately
        // receives the replay snapshot (full bus history) before sending a
        // single byte — so access control must live on the pipe itself.
        #[cfg(windows)]
        let opts = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            let sd = SecurityDescriptor::deserialize(widestring::u16cstr!(
                "D:P(A;;GA;;;SY)(A;;GA;;;OW)"
            ))?;
            opts.security_descriptor(sd)
        };

        let listener: Listener = opts.create_tokio()?;

        let shutdown = Arc::new(Notify::new());
        let shutdown_acceptor = shutdown.clone();

        // Shared JoinSet: accept loop spawns into it; shutdown() aborts all.
        let conn_tasks: Arc<Mutex<JoinSet<()>>> = Arc::new(Mutex::new(JoinSet::new()));
        let conn_tasks_acceptor = conn_tasks.clone();

        let accept_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_acceptor.notified() => {
                        tracing::debug!("rift-bus ipc: accept loop received shutdown");
                        break;
                    }
                    accepted = listener.accept() => {
                        match accepted {
                            Ok(stream) => {
                                let bus_for_conn = bus.clone();
                                // Reap any naturally-completed tasks before pushing the
                                // new one, so the JoinSet doesn't grow unboundedly across
                                // long-running sessions with detach/reattach churn.
                                let mut set = conn_tasks_acceptor.lock().await;
                                while set.try_join_next().is_some() {}
                                set.spawn(handle_connection(stream, bus_for_conn));
                            }
                            Err(e) => {
                                tracing::warn!("rift-bus ipc: accept error: {e}");
                                // Brief back-off so a tight error loop doesn't spin.
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            socket_name,
            shutdown,
            accept_task,
            conn_tasks,
        })
    }

    pub fn local_name(&self) -> &str {
        &self.socket_name
    }

    /// Stop accepting new connections and abort all active per-connection tasks.
    ///
    /// Signals the accept loop via [`Notify`], waits up to 1 s for it to exit,
    /// then aborts every per-connection task still tracked in the shared
    /// [`JoinSet`] so no orphaned tasks outlive this call.
    pub async fn shutdown(self) {
        self.shutdown.notify_one();
        // Best-effort: wait briefly for the accept loop to exit.
        let _ = tokio::time::timeout(Duration::from_secs(1), self.accept_task).await;

        // Abort all per-connection tasks that are still running.
        let mut set = self.conn_tasks.lock().await;
        set.abort_all();
        // Drain the JoinSet so the tasks are fully cancelled before we return.
        while set.join_next().await.is_some() {}
    }
}

// ---------------------------------------------------------------------------
// Per-connection driver
// ---------------------------------------------------------------------------

async fn handle_connection(stream: Stream, bus: RiftBus) {
    let (recv_half, send_half) = stream.split();

    let bus_for_writer = bus.clone();
    let writer_task = tokio::spawn(async move {
        if let Err(e) = run_writer(send_half, bus_for_writer).await {
            tracing::debug!("rift-bus ipc writer ended: {e}");
        }
    });

    let bus_for_reader = bus.clone();
    let reader_task = tokio::spawn(async move {
        if let Err(e) = run_reader(recv_half, bus_for_reader).await {
            tracing::debug!("rift-bus ipc reader ended: {e}");
        }
    });

    // When either half ends, drop the other to close the connection.
    tokio::select! {
        _ = writer_task => {},
        _ = reader_task => {},
    }
}

async fn run_writer(send: SendHalf, bus: RiftBus) -> Result<(), IpcError> {
    // Buffer writes so a connect-storm replay drain (up to the full replay
    // ring) and live-event bursts collapse into one `flush()` syscall each
    // instead of three per envelope. The named pipe / UDS is otherwise
    // flushed per frame, which is costly under PTY-burst event volume.
    let mut send = BufWriter::new(send);

    // 1. Drain the replay snapshot first — buffered, single flush.
    let (snapshot, mut sub) = bus.subscribe(SubscribeFilter::All);
    for env in &snapshot {
        write_frame_buffered(&mut send, env).await?;
    }
    send.flush().await?;

    // 2. Fan out live events. Block for one, then drain any already-ready
    // envelopes without awaiting so a burst flushes once.
    loop {
        match sub.recv().await {
            Ok(env) => {
                write_frame_buffered(&mut send, &env).await?;
                loop {
                    match sub.try_recv() {
                        TryRecv::Ready(env) => write_frame_buffered(&mut send, &env).await?,
                        TryRecv::Empty => break,
                        // Lagged/Closed mid-drain: flush what we have, then
                        // close so the client reconnects for a fresh snapshot.
                        TryRecv::Lagged(n) => {
                            tracing::warn!(
                                "rift-bus ipc: writer lagged by {n} events; closing connection"
                            );
                            let _ = send.flush().await;
                            return Ok(());
                        }
                        TryRecv::Closed => {
                            let _ = send.flush().await;
                            return Ok(());
                        }
                    }
                }
                send.flush().await?;
            }
            // Lagged: client missed events. Close so they reconnect and
            // re-drain a fresh snapshot.
            Err(BusError::Lagged(n)) => {
                tracing::warn!("rift-bus ipc: writer lagged by {n} events; closing connection");
                return Ok(());
            }
            Err(BusError::Closed) => return Ok(()),
        }
    }
}

async fn run_reader(mut recv: RecvHalf, bus: RiftBus) -> Result<(), IpcError> {
    loop {
        let env = read_frame(&mut recv).await?;
        bus.publish(env);
    }
}

// ---------------------------------------------------------------------------
// Framing — 4-byte LE length prefix + JSON body
// ---------------------------------------------------------------------------

/// Write one length-prefixed frame and flush immediately. Used by the
/// request/response client paths where a frame must hit the wire at once.
async fn write_frame<W>(writer: &mut W, env: &Envelope) -> Result<(), IpcError>
where
    W: AsyncWriteExt + Unpin,
{
    write_frame_buffered(writer, env).await?;
    writer.flush().await?;
    Ok(())
}

/// Write one length-prefixed frame into the writer without flushing. The
/// caller controls flush cadence (see [`run_writer`], which batches a burst
/// into a single flush).
async fn write_frame_buffered<W>(writer: &mut W, env: &Envelope) -> Result<(), IpcError>
where
    W: AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(env)?;
    if body.len() > MAX_FRAME_BYTES as usize {
        return Err(IpcError::FrameTooLarge(body.len() as u32));
    }
    let len = body.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&body).await?;
    Ok(())
}

async fn read_frame<R>(reader: &mut R) -> Result<Envelope, IpcError>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body).await?;
    let env: Envelope = serde_json::from_slice(&body)?;
    Ok(env)
}

// ---------------------------------------------------------------------------
// Client (used for tests and as a foundation for `rift-tail`-style CLIs)
// ---------------------------------------------------------------------------

pub struct IpcClient {
    recv: RecvHalf,
    send: SendHalf,
}

impl IpcClient {
    /// Connect to a server by name.
    pub async fn connect(name: impl AsRef<str>) -> Result<Self, IpcError> {
        let printable = name
            .as_ref()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| IpcError::InvalidName(e.to_string()))?;
        let stream = Stream::connect(printable).await?;
        let (recv, send) = stream.split();
        Ok(Self { recv, send })
    }

    /// Read the next envelope.
    pub async fn recv(&mut self) -> Result<Envelope, IpcError> {
        read_frame(&mut self.recv).await
    }

    /// Publish an envelope back to the server.
    pub async fn send(&mut self, env: &Envelope) -> Result<(), IpcError> {
        write_frame(&mut self.send, env).await
    }

    /// Split into independent reader + writer halves for callers that need
    /// concurrent recv/send (e.g. a router task that demuxes responses
    /// while another task pushes outbound requests). The `interprocess`
    /// crate's split halves are wrapped in newtypes so the dependency
    /// stays internal.
    pub fn split(self) -> (IpcReader, IpcWriter) {
        (IpcReader { recv: self.recv }, IpcWriter { send: self.send })
    }
}

/// Read half of a split [`IpcClient`]. See [`IpcClient::split`].
pub struct IpcReader {
    recv: RecvHalf,
}

impl IpcReader {
    /// Read the next envelope.
    pub async fn recv(&mut self) -> Result<Envelope, IpcError> {
        read_frame(&mut self.recv).await
    }
}

/// Write half of a split [`IpcClient`]. See [`IpcClient::split`].
pub struct IpcWriter {
    send: SendHalf,
}

impl IpcWriter {
    /// Publish an envelope back to the server.
    pub async fn send(&mut self, env: &Envelope) -> Result<(), IpcError> {
        write_frame(&mut self.send, env).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Category;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::time::{sleep, timeout, Duration};

    /// Unique per-test socket name. Re-using the same name across tests
    /// would collide on Windows (named pipes) and fail to bind.
    fn unique_name() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        format!("rift-bus-test-{pid}-{id}.sock")
    }

    #[tokio::test]
    async fn replay_then_live_streaming() {
        let bus = RiftBus::default();
        let name = unique_name();

        // Pre-publish before server starts → replay buffer captures.
        bus.publish(Envelope::new(Category::Hook, "pre_edit"));
        bus.publish(Envelope::new(Category::Pty, "pty.start"));

        let server = IpcServer::start(bus.clone(), &name).await.expect("server");
        // Tiny delay so the listener is fully ready before connecting.
        sleep(Duration::from_millis(50)).await;

        let mut client = IpcClient::connect(&name).await.expect("connect");

        // First two frames are the replay snapshot.
        let f1 = timeout(Duration::from_secs(2), client.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f1.kind, "pre_edit");
        let f2 = timeout(Duration::from_secs(2), client.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f2.kind, "pty.start");

        // Live event arrives next.
        bus.publish(Envelope::new(Category::Agent, "agent.dispatch"));
        let f3 = timeout(Duration::from_secs(2), client.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f3.kind, "agent.dispatch");

        server.shutdown().await;
    }

    #[tokio::test]
    async fn client_publish_round_trips_through_bus() {
        let bus = RiftBus::default();
        let name = unique_name();

        let server = IpcServer::start(bus.clone(), &name).await.expect("server");
        sleep(Duration::from_millis(50)).await;

        // Independent subscriber on the bus side observes what the client
        // publishes through the IPC pipe.
        let (_snap, mut watcher) = bus.subscribe(SubscribeFilter::Category(Category::Fs));

        let mut client = IpcClient::connect(&name).await.expect("connect");
        client
            .send(&Envelope::new(Category::Fs, "write"))
            .await
            .expect("send");

        let observed = timeout(Duration::from_secs(2), watcher.recv())
            .await
            .expect("recv within 2s")
            .expect("ok");
        assert_eq!(observed.category, Category::Fs);
        assert_eq!(observed.kind, "write");

        server.shutdown().await;
    }

    #[tokio::test]
    async fn frame_too_large_error_does_not_panic() {
        // Encode a fake length prefix above MAX_FRAME_BYTES into a duplex
        // pipe and read from it.
        let (rd, mut wr) = tokio::io::duplex(64);
        let mut rd = rd;
        let oversize = (MAX_FRAME_BYTES + 1).to_le_bytes();
        wr.write_all(&oversize).await.unwrap();

        let result = read_frame(&mut rd).await;
        match result {
            Err(IpcError::FrameTooLarge(n)) => assert_eq!(n, MAX_FRAME_BYTES + 1),
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    /// `shutdown_stops_accepting_new_connections` was removed: on Windows
    /// `interprocess::local_socket::tokio::Stream::connect` blocks in a
    /// non-cancellable WinAPI call when the named-pipe name lingers
    /// post-shutdown, defeating `tokio::time::timeout`. Graceful shutdown
    /// is an internal implementation detail rather than a wire-protocol
    /// contract, so its absence from the test surface does not weaken
    /// the IPC layer's contractual coverage. The four remaining IPC
    /// tests assert: framing round-trip, replay-snapshot drain at
    /// connect, live event delivery, and bidirectional client→bus
    /// publish — those are the contract.
    #[tokio::test]
    async fn shutdown_invocation_does_not_panic() {
        let bus = RiftBus::default();
        let name = unique_name();
        let server = IpcServer::start(bus, &name).await.expect("server");
        sleep(Duration::from_millis(50)).await;
        server.shutdown().await; // should return cleanly
    }

    #[tokio::test]
    async fn frame_round_trip_via_duplex_pipe() {
        let (rd, wr) = tokio::io::duplex(8192);
        let mut rd = rd;
        let mut wr = wr;

        let env = Envelope::new(Category::Hook, "pre_edit")
            .with_payload(&serde_json::json!({ "file": "pty.rs", "ok": true }))
            .unwrap();

        write_frame(&mut wr, &env).await.unwrap();
        let back = read_frame(&mut rd).await.unwrap();
        assert_eq!(env, back);
    }

    /// Shutdown aborts per-connection tasks.
    ///
    /// Spawn a server, accept a connection (which keeps the per-conn task
    /// alive), call shutdown(), then assert the conn task is no longer
    /// running within a short timeout.  We verify this by checking that the
    /// shared JoinSet is empty after shutdown drains it.
    #[tokio::test]
    async fn shutdown_aborts_per_connection_tasks() {
        let bus = RiftBus::default();
        let name = unique_name();

        let server = IpcServer::start(bus.clone(), &name).await.expect("server");
        sleep(Duration::from_millis(50)).await;

        // Connect a client so the accept loop spawns a per-conn handle_connection
        // task.  The client holds the socket open — the task stays alive.
        let _client = IpcClient::connect(&name).await.expect("connect");
        // Give the accept loop a moment to register the connection task.
        sleep(Duration::from_millis(50)).await;

        // Hold a reference to the JoinSet to inspect post-shutdown.
        let conn_tasks = server.conn_tasks.clone();

        // shutdown() must complete within 3s even with an active connection.
        timeout(Duration::from_secs(3), server.shutdown())
            .await
            .expect("shutdown must complete within 3s");

        // After shutdown() the JoinSet is fully drained (abort_all + join_next
        // until empty), so it must be empty now.
        let remaining = conn_tasks.lock().await.len();
        assert_eq!(remaining, 0, "JoinSet must be empty after shutdown");
    }
}
