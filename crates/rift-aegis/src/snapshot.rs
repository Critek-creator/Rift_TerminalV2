//! Public stub for the `snapshot` module. Real implementation lives in
//! `snapshot_private.rs` (gitignored) and is loaded by lib.rs's
//! `#[cfg_attr(feature = "private_modules", path = "snapshot_private.rs")]`
//! when the feature flag is on. See DEFERRED.md C-014.
//!
//! On public clones (no `private_modules` feature), `pub mod snapshot;`
//! resolves to this empty stub so `cargo fmt` and `cargo build` succeed
//! without the gitignored real impl present. The cfg-gated `pub use`
//! lines in lib.rs are inactive in this configuration, so no symbols
//! from the empty stub leak into rift-aegis's public surface.
