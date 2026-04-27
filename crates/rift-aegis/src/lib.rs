//! rift-aegis — Rift ↔ Aegis private translator (public stub form).
//!
//! On public clones, this crate compiles to an empty placeholder. The
//! real implementation lives in additional gitignored files alongside
//! this lib.rs (detect.rs, snapshot.rs) and is conditionally compiled
//! via the `private_modules` feature flag.
//!
//! src-tauri's `aegis` feature enables both `dep:rift-aegis` AND
//! `rift-aegis/private_modules`, so building with `cargo build
//! --features aegis` on private dev machines activates the real impl.
//! Public CI builds default features only and compiles this stub
//! as-is, with all `#[cfg(feature = "private_modules")]` blocks
//! inactive — the file lookup for `pub mod detect;` etc. only fires
//! when the feature is on, so the gitignored files do not need to
//! exist on public clones.
//!
//! Phase 7.1 ships the load-detection probe (decision B-b1).
//! Phase 7.2 adds: snapshot publication (SKILL.md version + settings.json
//! hooks + lesson count) and aegis.log live tail. Phase 7.4 adds the
//! aegis.session.skill_loaded envelope for the live status-line SKILL
//! segment. See PHASE 7 PLAN (commit 749ec91) and DEFERRED.md C-014
//! (D-011 close — public-CI stub mechanism).

// Public stub `detect.rs` and `snapshot.rs` are tracked. When the
// `private_modules` feature is on, the cfg_attr path attribute swaps the
// module's source file to the gitignored real implementation (`detect_
// private.rs` / `snapshot_private.rs`). rustfmt walks `pub mod` regardless
// of cfg, so the public stubs MUST exist on disk — they do.
#[cfg_attr(feature = "private_modules", path = "detect_private.rs")]
pub mod detect;

#[cfg_attr(feature = "private_modules", path = "snapshot_private.rs")]
pub mod snapshot;

// Re-exports stay cfg-gated: the public stubs are empty (no `probe` /
// `publish_snapshot` / `spawn_log_tail` symbols), so re-exporting from
// them on public CI would fail to resolve. The real impl files contain
// these symbols and the re-exports activate only when the feature is on.
#[cfg(feature = "private_modules")]
pub use detect::probe;
#[cfg(feature = "private_modules")]
pub use snapshot::{publish_snapshot, spawn_log_tail};
