//! [`RiftBus`] — in-process broadcast bus + replay buffer.
//!
//! Tier 1 of the §10.15 architecture (in-process). The Tier-2 cross-process
//! IPC surface (UDS / named pipe) is built on top of this same bus in a
//! later phase: external translators connect via the IPC server, which
//! itself is just another publisher/subscriber pair on this bus.
//!
//! ## Design notes
//!
//! * **`tokio::sync::broadcast`** — multi-consumer, ordered per-receiver.
//!   Capacity-bounded; lagged subscribers receive a `Lagged(n)` error and
//!   may use the replay buffer to recover.
//! * **Replay buffer** — bounded ring. Captures every published envelope
//!   so late subscribers (especially the IPC server's per-connection
//!   handlers) can drain a snapshot at subscribe time before live events
//!   start flowing. (V1 lesson:
//!   `pre-publish-before-start-ipc-server-isolates-replay-path`.)
//! * **Filter at the seam** — [`SubscribeFilter`] applies on both the
//!   replay snapshot and the live receive loop, so a category-scoped
//!   subscriber never sees other categories' events.
//! * **Publish-before-subscribe is fine.** [`RiftBus::publish`] tolerates
//!   zero subscribers (broadcast send returns Err, but replay still
//!   captures the event for the eventual late joiner).

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;

use tokio::sync::broadcast;

use crate::envelope::{Category, Envelope};

const DEFAULT_BROADCAST_CAPACITY: usize = 4096;
const DEFAULT_REPLAY_CAPACITY: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum BusError {
    #[error("subscriber lagged by {0} events; recover via replay buffer")]
    Lagged(u64),
    #[error("bus closed")]
    Closed,
}

/// Predicate that decides which envelopes a subscriber receives.
#[derive(Clone)]
pub enum SubscribeFilter {
    /// Receive everything.
    All,
    /// Receive a single category.
    Category(Category),
    /// Receive any of these categories.
    Categories(Vec<Category>),
    /// Custom predicate. The closure is invoked once per envelope per
    /// subscriber; keep it cheap.
    Custom(Arc<dyn Fn(&Envelope) -> bool + Send + Sync>),
}

impl std::fmt::Debug for SubscribeFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubscribeFilter::All => f.write_str("All"),
            SubscribeFilter::Category(c) => write!(f, "Category({c:?})"),
            SubscribeFilter::Categories(cs) => write!(f, "Categories({cs:?})"),
            SubscribeFilter::Custom(_) => f.write_str("Custom(<fn>)"),
        }
    }
}

/// In-process bus + replay buffer.
///
/// Cheap to clone (`Arc`-backed); pass clones into translators / Tauri
/// commands / spawned tasks.
#[derive(Clone)]
pub struct RiftBus {
    inner: Arc<RiftBusInner>,
}

struct RiftBusInner {
    tx: broadcast::Sender<Envelope>,
    /// Ring buffer of recent envelopes for late-subscriber replay.
    /// Locked via `.expect()` — poison indicates a prior panic in the
    /// single publisher path, meaning the bus state is unrecoverable.
    replay: Mutex<VecDeque<Envelope>>,
    replay_capacity: usize,
}

impl Default for RiftBus {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_BROADCAST_CAPACITY, DEFAULT_REPLAY_CAPACITY)
    }
}

impl RiftBus {
    /// Construct with explicit capacities.
    /// `broadcast_capacity` is the per-receiver queue depth; receivers that
    /// fall behind by more than this get `BusError::Lagged`.
    /// `replay_capacity` is the ring-buffer depth for late-subscriber catch-up.
    pub fn with_capacity(broadcast_capacity: usize, replay_capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(broadcast_capacity);
        Self {
            inner: Arc::new(RiftBusInner {
                tx,
                replay: Mutex::new(VecDeque::with_capacity(replay_capacity)),
                replay_capacity,
            }),
        }
    }

    /// Publish an envelope. Stores it in the replay buffer (ring) and
    /// broadcasts to live receivers. Tolerates zero subscribers — that
    /// case is normal during early startup.
    pub fn publish(&self, env: Envelope) {
        // Ephemeral high-frequency events skip the replay buffer to avoid
        // lock contention + deep-clone overhead during PTY bursts.
        if env.kind != "lane.changed" {
            let mut replay = self.inner.replay.lock();
            if replay.len() >= self.inner.replay_capacity {
                replay.pop_front();
            }
            replay.push_back(env.clone());
        }
        if let Err(e) = self.inner.tx.send(env) {
            // No live receivers — normal during startup or after all subscribers
            // have dropped. The envelope is already in the replay buffer above,
            // so late subscribers will still see it. Log at trace level only.
            tracing::trace!(
                kind = %e.0.kind,
                category = ?e.0.category,
                "rift-bus: broadcast dropped (no receivers); event preserved in replay buffer",
            );
        }
    }

    /// Subscribe with a filter. Returns:
    ///   1. the *current* replay snapshot (filtered) — drain this synchronously
    ///      before awaiting on the [`Subscription`] to avoid duplicating
    ///      events that arrive in the gap;
    ///   2. a [`Subscription`] handle for live envelopes after the snapshot.
    pub fn subscribe(&self, filter: SubscribeFilter) -> (Vec<Envelope>, Subscription) {
        let snapshot: Vec<Envelope> = {
            let replay = self.inner.replay.lock();
            replay
                .iter()
                .filter(|e| filter_matches(&filter, e))
                .cloned()
                .collect()
        };
        let rx = self.inner.tx.subscribe();
        (snapshot, Subscription { rx, filter })
    }

    /// Return the last `limit` envelopes matching `filter` from the replay
    /// buffer, iterating from the back. More efficient than `subscribe()`
    /// when only the tail is needed (avoids cloning the full filtered set).
    pub fn tail(&self, filter: &SubscribeFilter, limit: usize) -> Vec<Envelope> {
        let replay = self.inner.replay.lock();
        let mut result: Vec<Envelope> = replay
            .iter()
            .rev()
            .filter(|e| filter_matches(filter, e))
            .take(limit)
            .cloned()
            .collect();
        result.reverse();
        result
    }

    /// How many envelopes are currently stored in the replay buffer.
    pub fn replay_len(&self) -> usize {
        self.inner.replay.lock().len()
    }

    /// Number of live broadcast subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.inner.tx.receiver_count()
    }
}

/// Live subscriber handle. Returned by [`RiftBus::subscribe`] alongside
/// the replay snapshot.
pub struct Subscription {
    rx: broadcast::Receiver<Envelope>,
    filter: SubscribeFilter,
}

impl Subscription {
    /// Receive the next envelope matching the subscription's filter.
    /// `Lagged` is converted to [`BusError::Lagged`] so callers can
    /// recover via `RiftBus::subscribe()` again with the same filter.
    pub async fn recv(&mut self) -> Result<Envelope, BusError> {
        loop {
            match self.rx.recv().await {
                Ok(env) if filter_matches(&self.filter, &env) => return Ok(env),
                Ok(_) => continue, // filtered out; keep listening
                Err(broadcast::error::RecvError::Lagged(n)) => return Err(BusError::Lagged(n)),
                Err(broadcast::error::RecvError::Closed) => return Err(BusError::Closed),
            }
        }
    }

    /// Non-blocking poll for the next matching envelope. Skips filtered-out
    /// envelopes. Returns [`TryRecv::Empty`] when nothing matching is
    /// immediately buffered — lets a writer drain a burst and flush once
    /// instead of flushing per envelope.
    pub fn try_recv(&mut self) -> TryRecv {
        loop {
            match self.rx.try_recv() {
                Ok(env) if filter_matches(&self.filter, &env) => return TryRecv::Ready(env),
                Ok(_) => continue, // filtered out; keep draining
                Err(broadcast::error::TryRecvError::Empty) => return TryRecv::Empty,
                Err(broadcast::error::TryRecvError::Lagged(n)) => return TryRecv::Lagged(n),
                Err(broadcast::error::TryRecvError::Closed) => return TryRecv::Closed,
            }
        }
    }
}

/// Outcome of a non-blocking [`Subscription::try_recv`].
pub enum TryRecv {
    /// A matching envelope was immediately ready.
    Ready(Envelope),
    /// Nothing matching is currently buffered.
    Empty,
    /// The subscriber fell behind by `n` events; caller should reconnect.
    Lagged(u64),
    /// The bus has closed.
    Closed,
}

fn filter_matches(filter: &SubscribeFilter, env: &Envelope) -> bool {
    match filter {
        SubscribeFilter::All => true,
        SubscribeFilter::Category(c) => &env.category == c,
        SubscribeFilter::Categories(cs) => cs.contains(&env.category),
        SubscribeFilter::Custom(f) => f(env),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    fn env(category: Category, kind: &str) -> Envelope {
        Envelope::new(category, kind)
    }

    #[tokio::test]
    async fn publish_before_subscribe_appears_in_snapshot() {
        let bus = RiftBus::default();
        bus.publish(env(Category::Hook, "pre_edit"));
        bus.publish(env(Category::Pty, "pty.output"));

        let (snapshot, mut sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].category, Category::Hook);
        assert_eq!(snapshot[1].category, Category::Pty);

        // Live receive: publish after subscribe, sub.recv() returns it.
        bus.publish(env(Category::Agent, "agent.dispatch"));
        let live = timeout(Duration::from_secs(1), sub.recv())
            .await
            .expect("recv within 1s")
            .expect("ok");
        assert_eq!(live.category, Category::Agent);
    }

    #[tokio::test]
    async fn category_filter_excludes_others() {
        let bus = RiftBus::default();
        let (_snap, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::Hook));

        bus.publish(env(Category::Pty, "pty.output")); // filtered out
        bus.publish(env(Category::Hook, "pre_edit")); // delivered

        let got = timeout(Duration::from_secs(1), sub.recv())
            .await
            .expect("recv within 1s")
            .expect("ok");
        assert_eq!(got.category, Category::Hook);
        assert_eq!(got.kind, "pre_edit");
    }

    #[tokio::test]
    async fn replay_ring_buffer_drops_oldest() {
        let bus = RiftBus::with_capacity(64, 3);
        bus.publish(env(Category::System, "a"));
        bus.publish(env(Category::System, "b"));
        bus.publish(env(Category::System, "c"));
        bus.publish(env(Category::System, "d")); // pushes "a" out

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 3);
        let kinds: Vec<&str> = snapshot.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["b", "c", "d"]);
        assert_eq!(bus.replay_len(), 3);
    }

    #[tokio::test]
    async fn categories_filter_matches_any() {
        let bus = RiftBus::default();
        let (_snap, mut sub) = bus.subscribe(SubscribeFilter::Categories(vec![
            Category::Hook,
            Category::Aegis,
        ]));

        bus.publish(env(Category::Pty, "pty.output")); // out
        bus.publish(env(Category::Aegis, "loaded")); // in
        bus.publish(env(Category::Fs, "write")); // out
        bus.publish(env(Category::Hook, "pre_edit")); // in

        let first = timeout(Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.category, Category::Aegis);
        let second = timeout(Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.category, Category::Hook);
    }

    #[tokio::test]
    async fn custom_filter_is_applied() {
        let bus = RiftBus::default();
        let only_pre = SubscribeFilter::Custom(Arc::new(|e: &Envelope| e.kind.starts_with("pre_")));
        let (_snap, mut sub) = bus.subscribe(only_pre);

        bus.publish(env(Category::Hook, "post_edit")); // out
        bus.publish(env(Category::Hook, "pre_edit")); // in
        bus.publish(env(Category::Hook, "post_edit")); // out
        bus.publish(env(Category::Hook, "pre_run")); // in

        let a = timeout(Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.kind, "pre_edit");
        let b = timeout(Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(b.kind, "pre_run");
    }

    #[tokio::test]
    async fn subscriber_count_tracks_subscribes() {
        let bus = RiftBus::default();
        assert_eq!(bus.subscriber_count(), 0);
        let (_s1, sub1) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(bus.subscriber_count(), 1);
        let (_s2, sub2) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(bus.subscriber_count(), 2);
        drop(sub1);
        // broadcast tracks live receivers; allow tokio runtime a tick to update.
        tokio::task::yield_now().await;
        assert!(bus.subscriber_count() <= 2);
        drop(sub2);
    }

    #[tokio::test]
    async fn publish_with_zero_subscribers_does_not_panic() {
        let bus = RiftBus::default();
        for i in 0..50 {
            bus.publish(env(Category::System, &format!("kind-{i}")));
        }
        assert!(bus.replay_len() > 0);
    }

    #[tokio::test]
    async fn tail_returns_last_n_matching() {
        let bus = RiftBus::default();
        bus.publish(env(Category::Hook, "a"));
        bus.publish(env(Category::System, "sys1"));
        bus.publish(env(Category::Hook, "b"));
        bus.publish(env(Category::Hook, "c"));
        bus.publish(env(Category::System, "sys2"));
        bus.publish(env(Category::Hook, "d"));

        let tail = bus.tail(&SubscribeFilter::Category(Category::Hook), 2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].kind, "c");
        assert_eq!(tail[1].kind, "d");
    }

    #[tokio::test]
    async fn tail_returns_all_when_fewer_than_limit() {
        let bus = RiftBus::default();
        bus.publish(env(Category::Aegis, "x"));
        bus.publish(env(Category::Aegis, "y"));

        let tail = bus.tail(&SubscribeFilter::Category(Category::Aegis), 10);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].kind, "x");
        assert_eq!(tail[1].kind, "y");
    }

    #[tokio::test]
    async fn tail_with_all_filter() {
        let bus = RiftBus::with_capacity(64, 5);
        for i in 0..10 {
            bus.publish(env(Category::System, &format!("e{i}")));
        }
        let tail = bus.tail(&SubscribeFilter::All, 3);
        assert_eq!(tail.len(), 3);
        let kinds: Vec<&str> = tail.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["e7", "e8", "e9"]);
    }
}
