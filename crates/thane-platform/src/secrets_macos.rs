//! macOS Keychain-backed secret store using `security-framework`.

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use thane_core::secret_store::{SecretError, SecretStore};

/// Keychain service name for all thane secrets. Items are stored under this
/// service with the caller-supplied `key` as the account.
const SERVICE: &str = "com.thane.app";

/// Keychain-backed implementation. Items are Generic Password entries in the
/// user's default keychain under service `com.thane.app`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacosSecretStore;

impl MacosSecretStore {
    pub fn new() -> Self {
        Self
    }
}

impl SecretStore for MacosSecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError> {
        match get_generic_password(SERVICE, key) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) => {
                // security-framework returns "ItemNotFound" with status code -25300.
                // Anything else is a real failure (locked keychain, perms, etc.).
                if err.code() == -25300 {
                    Ok(None)
                } else {
                    Err(SecretError::Unavailable(format!("Keychain read failed: {err}")))
                }
            }
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
        set_generic_password(SERVICE, key, value)
            .map_err(|e| SecretError::Unavailable(format!("Keychain write failed: {e}")))
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match delete_generic_password(SERVICE, key) {
            Ok(()) => Ok(()),
            Err(err) => {
                if err.code() == -25300 {
                    Ok(()) // Already gone — treat as success.
                } else {
                    Err(SecretError::Unavailable(format!("Keychain delete failed: {err}")))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keychain access is gated on user/Touch ID consent on real machines and
    /// is unavailable in headless CI; we exercise the round-trip behind a
    /// `THANE_KEYCHAIN_TESTS=1` env opt-in only.
    #[test]
    fn keychain_roundtrip_opt_in() {
        if std::env::var("THANE_KEYCHAIN_TESTS").ok().as_deref() != Some("1") {
            return;
        }
        let store = MacosSecretStore::new();
        let key = "thane-test-keychain-roundtrip";
        // Best-effort cleanup before the test.
        let _ = store.delete(key);
        assert!(store.get(key).unwrap().is_none());
        store.set(key, b"hello").unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some(b"hello".as_ref()));
        store.delete(key).unwrap();
        assert!(store.get(key).unwrap().is_none());
    }
}
