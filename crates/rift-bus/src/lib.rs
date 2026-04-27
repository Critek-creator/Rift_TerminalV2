// rift-bus — Rift Integration Protocol bus crate.
// Spec: decisions/§10.15_real-time_update_mechanism.md.
//
// Phase 4.1 ships envelope schema + Category enum.
// Phase 4.2 ships RiftBus (broadcast + replay buffer + SubscribeFilter).
// Phase 4.3 ships IpcServer + IpcClient (UDS + named pipe via interprocess).
// Future: translator-module registry.

pub mod bus;
pub mod envelope;
pub mod ipc;

pub use bus::{BusError, RiftBus, SubscribeFilter, Subscription};
pub use envelope::{Category, Envelope, CURRENT_VERSION};
pub use ipc::{IpcClient, IpcError, IpcServer, MAX_FRAME_BYTES};
