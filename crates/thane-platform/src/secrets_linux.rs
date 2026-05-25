//! Linux secret store: prefers Secret Service (D-Bus → GNOME Keyring / KWallet /
//! KeePassXC), falls back to an AES-256-GCM-encrypted file under
//! `$XDG_DATA_HOME/thane/secrets/<key>` (mode 0600).
//!
//! ### Fallback details
//!
//! On headless hosts (CI, servers without a D-Bus user session) the Secret Service
//! connection fails. We then write `nonce || ciphertext` to
//! `$XDG_DATA_HOME/thane/secrets/<key>` with file mode `0600`. The AES key is
//! derived via HKDF-SHA256 from `/etc/machine-id` (or `/var/lib/dbus/machine-id`).
//!
//! Threat model: this protects against filesystem snapshots / off-host backups
//! being mounted on a different machine. It does **NOT** protect against an
//! attacker with local read access to both the secrets file and the host's
//! machine-id — that attacker can derive the same key. For higher assurance,
//! ensure Secret Service is available.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

use thane_core::secret_store::{SecretError, SecretStore};

const KEYRING_LABEL_PREFIX: &str = "thane: ";
const KEYRING_ATTR_APP: &str = "application";
const KEYRING_ATTR_APP_VALUE: &str = "com.thane.app";
const KEYRING_ATTR_KEY: &str = "key";

const NONCE_LEN: usize = 12;
const HKDF_INFO: &[u8] = b"thane-secret-store-fallback-v1";

/// Linux secret store. Tries Secret Service first; on failure, falls back to
/// AES-GCM-encrypted file storage under `fallback_root`.
pub struct LinuxSecretStore {
    fallback_root: PathBuf,
}

impl Default for LinuxSecretStore {
    fn default() -> Self {
        Self::with_fallback_root(default_fallback_root())
    }
}

impl LinuxSecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with a caller-provided fallback root. Used by tests so they
    /// don't trample the user's real $XDG_DATA_HOME.
    pub fn with_fallback_root(root: PathBuf) -> Self {
        Self { fallback_root: root }
    }

    fn fallback_path(&self, key: &str) -> PathBuf {
        self.fallback_root.join(sanitize_key(key))
    }
}

impl SecretStore for LinuxSecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError> {
        match secret_service_get(key) {
            Ok(Some(v)) => Ok(Some(v)),
            Ok(None) => {
                // Item not in keychain — fall through to file fallback in case
                // an earlier run wrote it there.
                file_fallback_get(&self.fallback_path(key))
            }
            Err(e) => {
                tracing::debug!("Secret Service unavailable for get({key}): {e}; trying file fallback");
                file_fallback_get(&self.fallback_path(key))
            }
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
        match secret_service_set(key, value) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!(
                    "Secret Service unavailable for set({key}): {e}; using encrypted file fallback at {}",
                    self.fallback_root.display()
                );
                file_fallback_set(&self.fallback_path(key), value)
            }
        }
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        // Best-effort: try both backends so a key written before a backend
        // change is fully removed.
        let ss_err = secret_service_delete(key).err();
        let file_err = file_fallback_delete(&self.fallback_path(key)).err();
        if let (Some(e1), Some(e2)) = (ss_err, file_err) {
            return Err(SecretError::Unavailable(format!(
                "secret-service: {e1}; file fallback: {e2}"
            )));
        }
        Ok(())
    }
}

fn default_fallback_root() -> PathBuf {
    // Mirror the Linux dirs::data_dir logic without depending on the trait —
    // this module is platform-specific and runs before trait selection.
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/var/tmp"))
        .join("thane")
        .join("secrets")
}

/// Sanitize a key for use as a filename. Strips path separators and replaces
/// anything outside `[A-Za-z0-9._-]` with `_`.
fn sanitize_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

// ── Secret Service path ─────────────────────────────────────────────────────

fn secret_service_get(key: &str) -> Result<Option<Vec<u8>>, SecretError> {
    use secret_service::EncryptionType;
    use secret_service::blocking::SecretService;

    let ss = SecretService::connect(EncryptionType::Dh)
        .map_err(|e| SecretError::Unavailable(format!("connect: {e}")))?;
    let coll = ss
        .get_default_collection()
        .map_err(|e| SecretError::Unavailable(format!("default_collection: {e}")))?;

    let attrs = attrs_for(key);
    let items = coll
        .search_items(attrs)
        .map_err(|e| SecretError::Unavailable(format!("search_items: {e}")))?;

    let Some(item) = items.into_iter().next() else {
        return Ok(None);
    };
    let secret = item
        .get_secret()
        .map_err(|e| SecretError::Unavailable(format!("get_secret: {e}")))?;
    Ok(Some(secret))
}

fn secret_service_set(key: &str, value: &[u8]) -> Result<(), SecretError> {
    use secret_service::EncryptionType;
    use secret_service::blocking::SecretService;

    let ss = SecretService::connect(EncryptionType::Dh)
        .map_err(|e| SecretError::Unavailable(format!("connect: {e}")))?;
    let coll = ss
        .get_default_collection()
        .map_err(|e| SecretError::Unavailable(format!("default_collection: {e}")))?;

    let attrs = attrs_for(key);
    let label = format!("{KEYRING_LABEL_PREFIX}{key}");
    coll.create_item(
        &label,
        attrs,
        value,
        true, // replace existing
        "application/octet-stream",
    )
    .map_err(|e| SecretError::Unavailable(format!("create_item: {e}")))?;
    Ok(())
}

fn secret_service_delete(key: &str) -> Result<(), SecretError> {
    use secret_service::EncryptionType;
    use secret_service::blocking::SecretService;

    let ss = SecretService::connect(EncryptionType::Dh)
        .map_err(|e| SecretError::Unavailable(format!("connect: {e}")))?;
    let coll = ss
        .get_default_collection()
        .map_err(|e| SecretError::Unavailable(format!("default_collection: {e}")))?;

    let attrs = attrs_for(key);
    let items = coll
        .search_items(attrs)
        .map_err(|e| SecretError::Unavailable(format!("search_items: {e}")))?;
    for item in items {
        let _ = item.delete();
    }
    Ok(())
}

fn attrs_for(key: &str) -> HashMap<&'static str, String> {
    let mut attrs = HashMap::new();
    attrs.insert(KEYRING_ATTR_APP, KEYRING_ATTR_APP_VALUE.to_string());
    attrs.insert(KEYRING_ATTR_KEY, key.to_string());
    attrs
}

// ── File fallback path ──────────────────────────────────────────────────────

fn file_fallback_get(path: &Path) -> Result<Option<Vec<u8>>, SecretError> {
    if !path.exists() {
        return Ok(None);
    }
    let blob = fs::read(path)?;
    if blob.len() < NONCE_LEN + 16 {
        return Err(SecretError::Crypto(
            "fallback file too small to contain nonce + ciphertext".into(),
        ));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let key = derive_fallback_key()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| SecretError::Crypto(format!("decrypt: {e}")))?;
    Ok(Some(plain))
}

fn file_fallback_set(path: &Path, value: &[u8]) -> Result<(), SecretError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        // Tighten directory perms — best effort, only if we own it.
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let key = derive_fallback_key()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, value)
        .map_err(|e| SecretError::Crypto(format!("encrypt: {e}")))?;

    // Atomic write: tmp + rename, mode 0600.
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&nonce_bytes)?;
        f.write_all(&ciphertext)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // Ensure mode is still 0600 after rename (umask could have intervened on create).
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn file_fallback_delete(path: &Path) -> Result<(), SecretError> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn derive_fallback_key() -> Result<[u8; 32], SecretError> {
    let machine_id = read_machine_id()?;
    // HKDF-SHA256(salt=machine-id-context, ikm=machine_id, info=HKDF_INFO).
    let hk = Hkdf::<Sha256>::new(Some(b"thane-secret-store-salt-v1"), machine_id.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(HKDF_INFO, &mut out)
        .map_err(|e| SecretError::Crypto(format!("HKDF expand: {e}")))?;
    Ok(out)
}

fn read_machine_id() -> Result<String, SecretError> {
    for candidate in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(s) = fs::read_to_string(candidate) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    Err(SecretError::Unavailable(
        "no machine-id available (tried /etc/machine-id and /var/lib/dbus/machine-id)".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "thane-secrets-test-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn sanitize_key_strips_separators() {
        assert_eq!(sanitize_key("foo/bar"), "foo_bar");
        assert_eq!(sanitize_key("../../etc/passwd"), ".._..._etc_passwd");
        assert_eq!(sanitize_key("a-b.c_d"), "a-b.c_d");
    }

    /// File fallback round-trip works without a D-Bus session. This is the
    /// scenario CI relies on, so it must always be exercised.
    #[test]
    fn file_fallback_roundtrip() {
        // Skip if no machine-id is readable (extremely rare even in CI containers).
        if read_machine_id().is_err() {
            return;
        }
        let root = temp_root("fallback-roundtrip");
        let _ = fs::remove_dir_all(&root);
        let path = root.join("audit-key");
        file_fallback_set(&path, b"the-quick-brown-fox").unwrap();
        let got = file_fallback_get(&path).unwrap();
        assert_eq!(got.as_deref(), Some(&b"the-quick-brown-fox"[..]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_fallback_get_missing_returns_none() {
        let root = temp_root("missing");
        let _ = fs::remove_dir_all(&root);
        let path = root.join("absent");
        assert!(file_fallback_get(&path).unwrap().is_none());
    }

    #[test]
    fn file_fallback_file_mode_is_0600() {
        if read_machine_id().is_err() {
            return;
        }
        let root = temp_root("mode");
        let _ = fs::remove_dir_all(&root);
        let path = root.join("k");
        file_fallback_set(&path, b"x").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "fallback file must be mode 0600, got {mode:o}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_fallback_overwrites_existing() {
        if read_machine_id().is_err() {
            return;
        }
        let root = temp_root("overwrite");
        let _ = fs::remove_dir_all(&root);
        let path = root.join("k");
        file_fallback_set(&path, b"first").unwrap();
        file_fallback_set(&path, b"second").unwrap();
        assert_eq!(
            file_fallback_get(&path).unwrap().as_deref(),
            Some(&b"second"[..])
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_fallback_detects_tamper() {
        if read_machine_id().is_err() {
            return;
        }
        let root = temp_root("tamper");
        let _ = fs::remove_dir_all(&root);
        let path = root.join("k");
        file_fallback_set(&path, b"secret-payload").unwrap();
        // Flip a byte in the ciphertext (after the 12-byte nonce).
        let mut blob = fs::read(&path).unwrap();
        let idx = NONCE_LEN + 4;
        blob[idx] ^= 0xff;
        fs::write(&path, &blob).unwrap();
        let err = file_fallback_get(&path).unwrap_err();
        assert!(matches!(err, SecretError::Crypto(_)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn linux_store_via_fallback_only() {
        // We don't assume Secret Service is up; LinuxSecretStore::set/get must
        // succeed via the file fallback when D-Bus is unavailable. (If D-Bus IS
        // available, this still passes — secret-service set wins.)
        if read_machine_id().is_err() {
            return;
        }
        let root = temp_root("store-roundtrip");
        let _ = fs::remove_dir_all(&root);
        let store = LinuxSecretStore::with_fallback_root(root.clone());
        let key = "thane-audit-root-key-v1-test";
        store.set(key, b"deadbeef").unwrap();
        let got = store.get(key).unwrap();
        assert_eq!(got.as_deref(), Some(&b"deadbeef"[..]));
        store.delete(key).unwrap();
        // After delete, must be gone from both paths.
        assert!(store.get(key).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
