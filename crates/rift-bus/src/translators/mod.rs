//! Translator modules — boundary layer between Rift core and external systems.
//!
//! Each translator owns its [`crate::Category`] and publishes envelopes on
//! behalf of one concern. Rift core MUST NOT call external systems directly
//! (§9 Integration Decoupling Principle — translators are the enforced
//! boundary).

pub mod commands;
pub mod errors;
