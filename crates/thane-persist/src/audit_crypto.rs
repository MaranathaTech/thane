//! AES-256-GCM encryption for rotated audit log files.
//!
//! Phase 4 of the audit hardening plan. The active `audit.jsonl` is appended to
//! constantly and is left as plaintext (protected by 0600 permissions). When a
//! file is rotated out it gets a single-shot encryption pass: the bytes hit
//! disk under `<name>.enc` and the plaintext original is removed.
//!
//! File format (`audit.N.jsonl.enc`):
//! - Bytes 0–7:    magic `b"THANEAUD"`
//! - Byte 8:       version `1`
//! - Bytes 9–20:   12-byte random nonce
//! - Bytes 21–end: AES-256-GCM ciphertext, with the 16-byte tag appended (as
//!   produced by the `aes-gcm` crate)
//!
//! The AES key is the HKDF-derived sub-key from `thane_core::audit_keys::audit_aes_key`,
//! itself derived from the root key persisted in the platform secret store. We
//! never load the root key here — the caller passes the 32-byte sub-key in.

use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::RngCore;
use rand::rngs::OsRng;
use thiserror::Error;

/// 8-byte magic identifying a thane-encrypted audit file.
pub const MAGIC: &[u8; 8] = b"THANEAUD";

/// On-disk file format version. Bump if the layout ever changes.
pub const VERSION: u8 = 1;

/// Length of the GCM nonce in bytes.
pub const NONCE_LEN: usize = 12;

/// Offset of the ciphertext within the encrypted file (magic + version + nonce).
pub const HEADER_LEN: usize = MAGIC.len() + 1 + NONCE_LEN;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ciphertext authentication failed (wrong key or tampered file)")]
    AuthenticationFailed,
    #[error("file is too short to be a thane-encrypted audit log ({len} bytes)")]
    Truncated { len: u64 },
    #[error("bad magic: expected THANEAUD, got {got:?}")]
    BadMagic { got: [u8; 8] },
    #[error("unsupported file format version {got} (this build understands {expected})")]
    UnsupportedVersion { got: u8, expected: u8 },
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
}

/// Encrypt `plaintext_path` to `ciphertext_path` and delete the plaintext on success.
///
/// Steps:
/// 1. Read the plaintext file into memory.
/// 2. Generate a fresh random 96-bit nonce.
/// 3. AES-256-GCM encrypt the bytes (tag appended).
/// 4. Write `magic | version | nonce | ciphertext` to a sibling temp file.
/// 5. Atomically rename the temp file to `ciphertext_path`.
/// 6. Delete the original plaintext file.
///
/// If step 4/5 fails, the temp file is removed and the plaintext is left intact.
pub fn encrypt_file(
    plaintext_path: &Path,
    ciphertext_path: &Path,
    key: &[u8; 32],
) -> Result<(), CryptoError> {
    let plaintext = std::fs::read(plaintext_path)?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let cipher = Aes256Gcm::new(key.into());
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_slice())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let tmp_path = temp_sibling(ciphertext_path);
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        f.write_all(MAGIC)?;
        f.write_all(&[VERSION])?;
        f.write_all(&nonce_bytes)?;
        f.write_all(&ciphertext)?;
        f.sync_all()?;
    }

    if let Err(e) = std::fs::rename(&tmp_path, ciphertext_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(CryptoError::Io(e));
    }

    // Best-effort plaintext removal. If this fails, the encrypted file is in
    // place and the next launch's migration step will retry.
    if let Err(e) = std::fs::remove_file(plaintext_path) {
        tracing::warn!(
            "encrypted {} but failed to remove plaintext {}: {e}",
            ciphertext_path.display(),
            plaintext_path.display()
        );
    }
    Ok(())
}

/// Decrypt a `.enc` file and return the plaintext bytes.
pub fn decrypt_file(ciphertext_path: &Path, key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    let bytes = std::fs::read(ciphertext_path)?;
    if bytes.len() < HEADER_LEN {
        return Err(CryptoError::Truncated {
            len: bytes.len() as u64,
        });
    }
    let magic = &bytes[..MAGIC.len()];
    if magic != MAGIC {
        let mut got = [0u8; 8];
        got.copy_from_slice(magic);
        return Err(CryptoError::BadMagic { got });
    }
    let version = bytes[MAGIC.len()];
    if version != VERSION {
        return Err(CryptoError::UnsupportedVersion {
            got: version,
            expected: VERSION,
        });
    }
    let nonce = Nonce::from_slice(&bytes[MAGIC.len() + 1..HEADER_LEN]);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(nonce, &bytes[HEADER_LEN..])
        .map_err(|_| CryptoError::AuthenticationFailed)
}

fn temp_sibling(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".tmp.{}", std::process::id()));
    target.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thane-audit-crypto-{}-{tag}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrip_recovers_plaintext_and_deletes_original() {
        let dir = temp_dir("roundtrip");
        let pt = dir.join("audit.1.jsonl");
        let ct = dir.join("audit.1.jsonl.enc");
        let payload = b"{\"id\":\"abc\"}\n{\"id\":\"def\"}\n".to_vec();
        std::fs::write(&pt, &payload).unwrap();

        let key = [9u8; 32];
        encrypt_file(&pt, &ct, &key).unwrap();
        assert!(ct.exists(), "ciphertext file must exist");
        assert!(!pt.exists(), "plaintext must be removed after encryption");

        let got = decrypt_file(&ct, &key).unwrap();
        assert_eq!(got, payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_key_returns_authentication_failed() {
        let dir = temp_dir("wrong-key");
        let pt = dir.join("audit.1.jsonl");
        let ct = dir.join("audit.1.jsonl.enc");
        std::fs::write(&pt, b"secret data").unwrap();

        encrypt_file(&pt, &ct, &[1u8; 32]).unwrap();
        let err = decrypt_file(&ct, &[2u8; 32]).unwrap_err();
        assert!(matches!(err, CryptoError::AuthenticationFailed), "got {err:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_ciphertext_returns_authentication_failed() {
        let dir = temp_dir("corrupted");
        let pt = dir.join("audit.1.jsonl");
        let ct = dir.join("audit.1.jsonl.enc");
        std::fs::write(&pt, b"hello world payload").unwrap();

        let key = [7u8; 32];
        encrypt_file(&pt, &ct, &key).unwrap();

        // Flip a single bit inside the ciphertext body.
        let mut bytes = std::fs::read(&ct).unwrap();
        let idx = HEADER_LEN + 2;
        bytes[idx] ^= 0x01;
        std::fs::write(&ct, &bytes).unwrap();

        let err = decrypt_file(&ct, &key).unwrap_err();
        assert!(matches!(err, CryptoError::AuthenticationFailed), "got {err:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_file_returns_graceful_error() {
        let dir = temp_dir("truncated");
        let ct = dir.join("audit.1.jsonl.enc");
        // Way less than HEADER_LEN bytes.
        std::fs::write(&ct, b"short").unwrap();
        let err = decrypt_file(&ct, &[0u8; 32]).unwrap_err();
        assert!(matches!(err, CryptoError::Truncated { .. }), "got {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_magic_returns_bad_magic() {
        let dir = temp_dir("bad-magic");
        let ct = dir.join("audit.1.jsonl.enc");
        let mut bytes = vec![0u8; HEADER_LEN + 32];
        bytes[..8].copy_from_slice(b"NOTTHANE");
        bytes[8] = VERSION;
        std::fs::write(&ct, &bytes).unwrap();

        let err = decrypt_file(&ct, &[0u8; 32]).unwrap_err();
        assert!(matches!(err, CryptoError::BadMagic { .. }), "got {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_version_returns_unsupported_version() {
        let dir = temp_dir("bad-version");
        let ct = dir.join("audit.1.jsonl.enc");
        let mut bytes = vec![0u8; HEADER_LEN + 32];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8] = 99; // unsupported version
        std::fs::write(&ct, &bytes).unwrap();

        let err = decrypt_file(&ct, &[0u8; 32]).unwrap_err();
        assert!(
            matches!(err, CryptoError::UnsupportedVersion { got: 99, expected: 1 }),
            "got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_calls_produce_different_nonces() {
        // GCM determinism relies on unique nonces — confirm we mint a fresh
        // one for each encrypt call.
        let dir = temp_dir("nonce-unique");
        let pt = dir.join("audit.1.jsonl");
        let key = [3u8; 32];

        std::fs::write(&pt, b"same plaintext").unwrap();
        encrypt_file(&pt, &dir.join("a.enc"), &key).unwrap();
        std::fs::write(&pt, b"same plaintext").unwrap();
        encrypt_file(&pt, &dir.join("b.enc"), &key).unwrap();

        let a = std::fs::read(dir.join("a.enc")).unwrap();
        let b = std::fs::read(dir.join("b.enc")).unwrap();
        // Headers differ in their nonce slice — and therefore the ciphertext +
        // tag bytes differ too.
        let nonce_a = &a[MAGIC.len() + 1..HEADER_LEN];
        let nonce_b = &b[MAGIC.len() + 1..HEADER_LEN];
        assert_ne!(nonce_a, nonce_b, "successive encrypts must use distinct nonces");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_plaintext_roundtrips() {
        let dir = temp_dir("empty");
        let pt = dir.join("audit.1.jsonl");
        let ct = dir.join("audit.1.jsonl.enc");
        std::fs::write(&pt, b"").unwrap();
        let key = [0xAA; 32];
        encrypt_file(&pt, &ct, &key).unwrap();
        let got = decrypt_file(&ct, &key).unwrap();
        assert!(got.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
