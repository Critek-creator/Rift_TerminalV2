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
pub mod translators;

pub use bus::{BusError, RiftBus, SubscribeFilter, Subscription};
pub use envelope::{Category, Envelope, CURRENT_VERSION};
pub use ipc::{IpcClient, IpcError, IpcServer, MAX_FRAME_BYTES};

/// Publish a `Category::System / kind="error"` envelope via the errors translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_error(...)`.
/// Full path also works: `rift_bus::translators::errors::publish(...)`.
pub use translators::errors::publish as publish_error;

/// Publish a `Category::Pty / kind="command.submitted"` envelope via the commands translator.
///
/// Convenience re-export so callers can write `rift_bus::publish_command(...)`.
/// Full path also works: `rift_bus::translators::commands::publish(...)`.
pub use translators::commands::publish as publish_command;

/// Re-export [`CommandBuffer`] so callers can write `rift_bus::CommandBuffer`.
pub use translators::commands::CommandBuffer;
