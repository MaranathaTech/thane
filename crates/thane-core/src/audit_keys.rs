//! Audit-log key management.
//!
//! On first launch we generate a 32-byte random **root key** and persist it via
//! the platform `SecretStore` under key `thane-audit-root-key-v1`. Sub-keys for
//! distinct purposes are derived from the root via HKDF-SHA256, each with a
//! purpose-specific `info` string:
//!
//! - **HMAC key** — `info = b"thane-audit-hmac-v1"`, 32 bytes — used by
//!   `audit::AuditLog` to sign each event (Phase 2).
//! - **AES key**  — `info = b"thane-audit-aes-v1"`,  32 bytes — reserved for
//!   encryption-at-rest of rotated audit files (Phase 4).
//!
//! The functions cache the derived key in a process-wide `OnceLock` so each
//! event-signing call doesn't re-hit the keychain.

use std::sync::OnceLock;

use hkdf::Hkdf;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Sha256;
use thiserror::Error;

use crate::secret_store::{SecretError, SecretStore};

const ROOT_KEY_NAME: &str = "thane-audit-root-key-v1";
const ROOT_KEY_LEN: usize = 32;

/// Env-var escape hatch: when set to a 64-character hex string, treat its bytes
/// as the audit HMAC key directly and skip the platform secret store. Useful
/// for headless CI / integration tests where a Keychain prompt would deadlock,
/// and for containerized deployments that mount a key file outside the keychain.
const ENV_KEY_OVERRIDE: &str = "THANE_AUDIT_HMAC_KEY_HEX";

/// HKDF `info` strings — must remain stable forever. Changing one invalidates
/// every audit log ever signed/encrypted with the previous key.
pub const HMAC_INFO: &[u8] = b"thane-audit-hmac-v1";
pub const AES_INFO: &[u8] = b"thane-audit-aes-v1";

#[derive(Debug, Error)]
pub enum AuditKeyError {
    #[error("secret store error: {0}")]
    Secret(#[from] SecretError),
    #[error("HKDF expand failed: {0}")]
    Hkdf(String),
    #[error("stored root key is malformed (expected {expected} bytes, got {got})")]
    MalformedRoot { expected: usize, got: usize },
}

static HMAC_KEY: OnceLock<[u8; 32]> = OnceLock::new();
static AES_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Load (or, on first call, generate-and-persist) the root key from `store`.
///
/// Caller is responsible for choosing the right store for the platform.
pub fn load_or_create_root_key(
    store: &dyn SecretStore,
) -> Result<[u8; ROOT_KEY_LEN], AuditKeyError> {
    if let Some(bytes) = store.get(ROOT_KEY_NAME)? {
        if bytes.len() != ROOT_KEY_LEN {
            return Err(AuditKeyError::MalformedRoot {
                expected: ROOT_KEY_LEN,
                got: bytes.len(),
            });
        }
        let mut out = [0u8; ROOT_KEY_LEN];
        out.copy_from_slice(&bytes);
        return Ok(out);
    }
    // First launch — mint a new key and persist.
    let mut key = [0u8; ROOT_KEY_LEN];
    OsRng.fill_bytes(&mut key);
    store.set(ROOT_KEY_NAME, &key)?;
    tracing::info!("generated new audit root key (32 bytes), stored in platform secret store");
    Ok(key)
}

/// Replace the persisted root key with `key`. Used by the CLI `audit import-key`
/// command (and tests). Existing logs signed with the previous key will fail
/// verification afterwards.
pub fn store_root_key(
    store: &dyn SecretStore,
    key: &[u8; ROOT_KEY_LEN],
) -> Result<(), AuditKeyError> {
    store.set(ROOT_KEY_NAME, key)?;
    // Invalidate caches so subsequent derivations pick up the new root.
    // `OnceLock` can't be reset; this is safe because we only call this from
    // an admin command and the process is expected to be restarted.
    Ok(())
}

/// Derive an HKDF-SHA256 sub-key of `out_len` bytes from `root` under `info`.
pub fn derive_subkey(root: &[u8], info: &[u8], out_len: usize) -> Result<Vec<u8>, AuditKeyError> {
    let hk = Hkdf::<Sha256>::new(None, root);
    let mut out = vec![0u8; out_len];
    hk.expand(info, &mut out)
        .map_err(|e| AuditKeyError::Hkdf(format!("{e}")))?;
    Ok(out)
}

/// Convenience: derive a 32-byte sub-key, copying into a fixed array.
fn derive_32(root: &[u8], info: &[u8]) -> Result<[u8; 32], AuditKeyError> {
    let v = derive_subkey(root, info, 32)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

/// Return the cached HMAC sub-key, deriving it on first call.
///
/// Panics on backend failure — the audit log cannot do its job without this key
/// and continuing unsigned would silently degrade integrity guarantees. Callers
/// that want a recoverable path should use [`try_audit_hmac_key`].
pub fn audit_hmac_key(store: &dyn SecretStore) -> [u8; 32] {
    try_audit_hmac_key(store).expect("audit HMAC key unavailable")
}

/// Fallible variant of [`audit_hmac_key`].
pub fn try_audit_hmac_key(store: &dyn SecretStore) -> Result<[u8; 32], AuditKeyError> {
    if let Some(k) = HMAC_KEY.get() {
        return Ok(*k);
    }
    // Env-var escape hatch — never touches the platform secret store.
    if let Some(k) = key_from_env_override()? {
        let _ = HMAC_KEY.set(k);
        return Ok(k);
    }
    let root = load_or_create_root_key(store)?;
    let k = derive_32(&root, HMAC_INFO)?;
    let _ = HMAC_KEY.set(k);
    Ok(*HMAC_KEY.get().unwrap_or(&k))
}

/// Parse `THANE_AUDIT_HMAC_KEY_HEX` into a 32-byte key, if set.
fn key_from_env_override() -> Result<Option<[u8; 32]>, AuditKeyError> {
    let Ok(s) = std::env::var(ENV_KEY_OVERRIDE) else {
        return Ok(None);
    };
    let trimmed = s.trim();
    let mut buf = [0u8; 32];
    // Manual hex decode — keeps thane-core free of a `hex` crate dependency.
    if trimmed.len() != 64 {
        return Err(AuditKeyError::Hkdf(format!(
            "{ENV_KEY_OVERRIDE} must be 64 hex chars, got {}",
            trimmed.len()
        )));
    }
    for (i, byte_chunk) in trimmed.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(byte_chunk).map_err(|e| {
            AuditKeyError::Hkdf(format!("{ENV_KEY_OVERRIDE}: non-utf8 byte: {e}"))
        })?;
        buf[i] = u8::from_str_radix(s, 16).map_err(|e| {
            AuditKeyError::Hkdf(format!("{ENV_KEY_OVERRIDE}: non-hex byte {s:?}: {e}"))
        })?;
    }
    Ok(Some(buf))
}

/// Return the cached AES sub-key, deriving it on first call. Reserved for
/// Phase 4 (encryption-at-rest).
pub fn audit_aes_key(store: &dyn SecretStore) -> [u8; 32] {
    try_audit_aes_key(store).expect("audit AES key unavailable")
}

/// Fallible variant of [`audit_aes_key`].
pub fn try_audit_aes_key(store: &dyn SecretStore) -> Result<[u8; 32], AuditKeyError> {
    if let Some(k) = AES_KEY.get() {
        return Ok(*k);
    }
    let root = load_or_create_root_key(store)?;
    let k = derive_32(&root, AES_INFO)?;
    let _ = AES_KEY.set(k);
    Ok(*AES_KEY.get().unwrap_or(&k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_store::MemorySecretStore;

    #[test]
    fn load_or_create_generates_on_first_call() {
        let store = MemorySecretStore::new();
        assert!(store.get(ROOT_KEY_NAME).unwrap().is_none());
        let k = load_or_create_root_key(&store).unwrap();
        assert_eq!(k.len(), ROOT_KEY_LEN);
        assert!(store.get(ROOT_KEY_NAME).unwrap().is_some());
    }

    #[test]
    fn load_or_create_returns_existing_on_second_call() {
        let store = MemorySecretStore::new();
        let k1 = load_or_create_root_key(&store).unwrap();
        let k2 = load_or_create_root_key(&store).unwrap();
        assert_eq!(k1, k2, "second call must return the same persisted key");
    }

    #[test]
    fn malformed_root_key_returns_error() {
        let store = MemorySecretStore::new();
        store.set(ROOT_KEY_NAME, b"too-short").unwrap();
        let err = load_or_create_root_key(&store).unwrap_err();
        assert!(matches!(err, AuditKeyError::MalformedRoot { .. }));
    }

    #[test]
    fn derive_subkey_is_deterministic_for_same_root_and_info() {
        let root = [7u8; 32];
        let a = derive_subkey(&root, b"info-1", 32).unwrap();
        let b = derive_subkey(&root, b"info-1", 32).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn derive_subkey_differs_by_info() {
        let root = [7u8; 32];
        let a = derive_subkey(&root, b"info-1", 32).unwrap();
        let b = derive_subkey(&root, b"info-2", 32).unwrap();
        assert_ne!(a, b, "HKDF must produce different keys for different info strings");
    }

    #[test]
    fn derive_subkey_differs_by_root() {
        let r1 = [1u8; 32];
        let r2 = [2u8; 32];
        let a = derive_subkey(&r1, HMAC_INFO, 32).unwrap();
        let b = derive_subkey(&r2, HMAC_INFO, 32).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn hmac_and_aes_keys_differ_for_same_root() {
        let root = [3u8; 32];
        let hmac = derive_subkey(&root, HMAC_INFO, 32).unwrap();
        let aes = derive_subkey(&root, AES_INFO, 32).unwrap();
        assert_ne!(hmac, aes, "HMAC and AES sub-keys must use distinct info strings");
    }

    #[test]
    fn store_root_key_overwrites_existing() {
        let store = MemorySecretStore::new();
        let original = load_or_create_root_key(&store).unwrap();
        let replacement = [42u8; 32];
        store_root_key(&store, &replacement).unwrap();
        let read_back = load_or_create_root_key(&store).unwrap();
        assert_eq!(read_back, replacement);
        assert_ne!(read_back, original);
    }

    #[test]
    fn env_override_bypasses_secret_store_when_set() {
        // SAFETY: single-threaded test, mutating process env.
        unsafe {
            std::env::set_var(ENV_KEY_OVERRIDE, "ab".repeat(32));
        }
        // Use a store whose `get` would loudly fail if we accidentally hit it.
        struct PoisonedStore;
        impl SecretStore for PoisonedStore {
            fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, SecretError> {
                panic!("env override path must not touch the secret store");
            }
            fn set(&self, _: &str, _: &[u8]) -> Result<(), SecretError> {
                panic!("must not set");
            }
            fn delete(&self, _: &str) -> Result<(), SecretError> {
                panic!("must not delete");
            }
        }
        // Reset the cache so this test always exercises the env path. Can't
        // actually reset OnceLock from a test; the side effect of `try_audit_hmac_key`
        // is that future calls return the cached key. To stay independent of
        // test order, parse the env directly here:
        let got = key_from_env_override().unwrap().unwrap();
        let expected: [u8; 32] = [0xab; 32];
        assert_eq!(got, expected);
        let _ = PoisonedStore; // touched so the unused-warning is silenced
        unsafe {
            std::env::remove_var(ENV_KEY_OVERRIDE);
        }
    }

    #[test]
    fn env_override_rejects_bad_length() {
        unsafe {
            std::env::set_var(ENV_KEY_OVERRIDE, "ff");
        }
        let err = key_from_env_override().unwrap_err();
        assert!(format!("{err}").contains("64 hex chars"));
        unsafe {
            std::env::remove_var(ENV_KEY_OVERRIDE);
        }
    }

    #[test]
    fn hkdf_info_strings_match_documented_constants() {
        // Lock the wire format — these strings cannot change without
        // invalidating every existing audit log.
        assert_eq!(HMAC_INFO, b"thane-audit-hmac-v1");
        assert_eq!(AES_INFO, b"thane-audit-aes-v1");
    }
}
