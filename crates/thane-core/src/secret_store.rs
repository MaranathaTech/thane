//! Platform-agnostic secret store trait.
//!
//! Concrete implementations (Keychain on macOS, Secret Service + file fallback
//! on Linux) live in `thane-platform`. `MemorySecretStore` lives here so tests
//! in `thane-core` can construct it without a platform-specific dep.

use std::collections::HashMap;
use std::sync::Mutex;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("secret not found")]
    NotFound,
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
}

/// Trait for storing and retrieving small secret blobs (≤ a few KB).
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError>;
    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// In-memory implementation for tests and ephemeral runs.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    inner: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError> {
        Ok(self.inner.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
        self.inner.lock().unwrap().insert(key.to_string(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.inner.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_roundtrip() {
        let s = MemorySecretStore::new();
        s.set("k", b"v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some(&b"v"[..]));
    }

    #[test]
    fn memory_missing_returns_none() {
        let s = MemorySecretStore::new();
        assert!(s.get("absent").unwrap().is_none());
    }

    #[test]
    fn memory_delete() {
        let s = MemorySecretStore::new();
        s.set("k", b"v").unwrap();
        s.delete("k").unwrap();
        assert!(s.get("k").unwrap().is_none());
    }
}
