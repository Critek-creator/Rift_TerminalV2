//! Secure API key storage via OS keyring.
//!
//! Keys are stored under service name "rift-terminal" with the model ID
//! as the username. The config file stores only the model ID reference,
//! not the key itself.

const SERVICE_NAME: &str = "rift-terminal";

/// Store an API key in the OS keyring.
pub fn store_api_key(model_id: &str, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE_NAME, model_id)
        .map_err(|e| format!("keyring entry error: {e}"))?;
    entry
        .set_password(key)
        .map_err(|e| format!("keyring store error: {e}"))
}

/// Retrieve an API key from the OS keyring.
pub fn get_api_key(model_id: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(SERVICE_NAME, model_id)
        .map_err(|e| format!("keyring entry error: {e}"))?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring read error: {e}")),
    }
}

/// Delete an API key from the OS keyring.
pub fn delete_api_key(model_id: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE_NAME, model_id)
        .map_err(|e| format!("keyring entry error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete error: {e}")),
    }
}

/// Resolve an API key: try keyring first, fall back to the raw value
/// in api_key_ref (backwards compatible with pre-keyring configs).
pub fn resolve_api_key(model_id: &str, api_key_ref: Option<&str>) -> String {
    // Try keyring first
    if let Ok(Some(key)) = get_api_key(model_id) {
        return key;
    }
    // Fall back to raw value in config (backwards compat)
    api_key_ref.unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_falls_back_to_raw_value() {
        // Don't actually touch the keyring in tests — just verify fallback
        let key = resolve_api_key("nonexistent-test-model", Some("raw-key-123"));
        assert_eq!(key, "raw-key-123");
    }

    #[test]
    fn resolve_returns_empty_when_no_key() {
        let key = resolve_api_key("nonexistent-test-model", None);
        assert_eq!(key, "");
    }
}
