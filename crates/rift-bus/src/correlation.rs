//! Correlation ID generation for causal chain linking.
//!
//! Each filesystem event starts a new correlation chain. Downstream
//! translators (hooks, agents, MCP) can inherit the correlation_id
//! from a triggering event to build the chain.

use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a compact, unique correlation ID.
///
/// Format: `corr-<epoch_ms_hex>-<counter_hex>`. Unique within a single
/// Rift process lifetime. Not globally unique (no UUID overhead needed
/// since correlation is local to one session).
pub fn new_correlation_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("corr-{ts:x}-{seq:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = new_correlation_id();
        let b = new_correlation_id();
        assert_ne!(a, b);
    }

    #[test]
    fn id_starts_with_prefix() {
        let id = new_correlation_id();
        assert!(id.starts_with("corr-"));
    }
}
