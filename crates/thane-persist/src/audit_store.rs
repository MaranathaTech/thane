use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use thane_core::audit::{AuditEvent, AuditLog};
use thiserror::Error;

use crate::audit_crypto::{self, CryptoError};

/// Maximum audit log file size before rotation (5 MB).
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// Maximum number of rotated log files to keep.
const MAX_ROTATED_FILES: usize = 5;

/// Default retention window for rotated audit files (days).
const DEFAULT_RETENTION_DAYS: u32 = 90;

#[derive(Debug, Error)]
pub enum AuditStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("audit encryption error: {0}")]
    Crypto(#[from] CryptoError),
    #[error(
        "encountered an encrypted audit file ({}) but no AES key was provided",
        path.display()
    )]
    EncryptedFileButNoKey { path: PathBuf },
}

/// File-based audit log storage using JSONL format (one event per line).
///
/// Supports:
/// - Append-only writing for crash safety
/// - Automatic rotation when file exceeds 5 MB
/// - Loading events from current + rotated files
/// - Export to JSON array format
/// - Time-based purge of rotated files older than `retention_days`
pub struct AuditStore {
    dir: PathBuf,
    /// Number of days to keep rotated audit files. `0` means retain forever.
    retention_days: u32,
    /// AES-256-GCM key for rotated-file encryption (Phase 4). When set,
    /// rotation produces `audit.N.jsonl.enc` files and the read path
    /// transparently decrypts them. When `None`, behaves as before
    /// (plaintext rotated files).
    encryption_key: Option<[u8; 32]>,
}

impl AuditStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            retention_days: DEFAULT_RETENTION_DAYS,
            encryption_key: None,
        }
    }

    /// Override the retention window. `0` disables time-based purging entirely.
    pub fn with_retention_days(mut self, days: u32) -> Self {
        self.retention_days = days;
        self
    }

    /// Current retention window, in days.
    pub fn retention_days(&self) -> u32 {
        self.retention_days
    }

    /// Enable AES-256-GCM encryption for rotated audit files.
    ///
    /// `None` disables encryption (rotation keeps plaintext `audit.N.jsonl`),
    /// `Some(key)` enables it (rotation produces `audit.N.jsonl.enc` and removes
    /// the plaintext). Caller derives the key via
    /// [`thane_core::audit_keys::audit_aes_key`] from the platform secret store.
    pub fn with_encryption_key(mut self, key: Option<[u8; 32]>) -> Self {
        self.encryption_key = key;
        self
    }

    /// Whether at-rest encryption is currently enabled on this store.
    pub fn encryption_enabled(&self) -> bool {
        self.encryption_key.is_some()
    }

    /// Append a single audit event to the current log file.
    pub fn append(&self, event: &AuditEvent) -> Result<(), AuditStoreError> {
        std::fs::create_dir_all(&self.dir)?;

        // Rotate if the current file is too large.
        let current = self.current_file();
        if current.exists()
            && let Ok(meta) = current.metadata()
            && meta.len() >= MAX_FILE_SIZE
        {
            self.rotate()?;
        }

        let line = serde_json::to_string(event)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Flush all events from an in-memory AuditLog to disk and apply the
    /// time-based retention policy to rotated files.
    pub fn flush(&self, log: &AuditLog) -> Result<(), AuditStoreError> {
        for event in log.all() {
            self.append(event)?;
        }
        self.purge_expired(Utc::now())?;
        Ok(())
    }

    /// Delete rotated files whose newest event is older than the retention window.
    ///
    /// `now` is taken as a parameter so tests can inject a fixed clock instead of
    /// depending on real time (see CLAUDE.md testing guidance).
    ///
    /// The active `audit.jsonl` is never purged — only rotated files
    /// (`audit.N.jsonl` or `audit.N.jsonl.enc`) past the window are removed, and
    /// only if they have at least one parseable event (corrupt/empty files are
    /// left alone for forensics).
    pub fn purge_expired(&self, now: DateTime<Utc>) -> Result<(), AuditStoreError> {
        if self.retention_days == 0 {
            return Ok(());
        }
        let cutoff = now - Duration::days(self.retention_days as i64);
        for i in 1..=MAX_ROTATED_FILES {
            let Some(path) = self.find_rotated_file(i) else {
                continue;
            };
            match self.newest_event_timestamp(&path)? {
                Some(newest) if newest < cutoff => {
                    tracing::info!(
                        "Purging audit file {} (newest event {newest} < cutoff {cutoff})",
                        path.display()
                    );
                    if let Err(e) = std::fs::remove_file(&path) {
                        tracing::warn!("Failed to remove expired audit file {}: {e}", path.display());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Load events from the current log file into an AuditLog.
    pub fn load_current(&self) -> Result<Vec<AuditEvent>, AuditStoreError> {
        self.load_file(&self.current_file())
    }

    /// Load events from all log files (current + rotated), newest first.
    pub fn load_all(&self) -> Result<Vec<AuditEvent>, AuditStoreError> {
        let mut events = Vec::new();

        // Load rotated files (oldest first).
        for i in (1..=MAX_ROTATED_FILES).rev() {
            if let Some(path) = self.find_rotated_file(i) {
                events.extend(self.load_file(&path)?);
            }
        }

        // Load current file (newest).
        events.extend(self.load_current()?);

        Ok(events)
    }

    /// Export all events as a JSON array string.
    pub fn export_json(&self) -> Result<String, AuditStoreError> {
        let events = self.load_all()?;
        Ok(serde_json::to_string_pretty(&events)?)
    }

    /// Clear all audit log files.
    pub fn clear(&self) -> Result<(), AuditStoreError> {
        let current = self.current_file();
        if current.exists() {
            std::fs::remove_file(&current)?;
        }
        for i in 1..=MAX_ROTATED_FILES {
            // Remove both possible names at each slot to cover mixed states.
            for path in self.both_rotated_paths(i) {
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
            }
        }
        Ok(())
    }

    /// Get the total size of all audit log files in bytes.
    pub fn total_size(&self) -> u64 {
        let mut size = 0;
        let current = self.current_file();
        if let Ok(meta) = current.metadata() {
            size += meta.len();
        }
        for i in 1..=MAX_ROTATED_FILES {
            if let Some(path) = self.find_rotated_file(i)
                && let Ok(meta) = path.metadata()
            {
                size += meta.len();
            }
        }
        size
    }

    fn current_file(&self) -> PathBuf {
        self.dir.join("audit.jsonl")
    }

    /// Plaintext rotated path (legacy when encryption is disabled).
    fn plaintext_rotated_file(&self, n: usize) -> PathBuf {
        self.dir.join(format!("audit.{n}.jsonl"))
    }

    /// Encrypted rotated path.
    fn encrypted_rotated_file(&self, n: usize) -> PathBuf {
        self.dir.join(format!("audit.{n}.jsonl.enc"))
    }

    /// Both possible paths at slot `n` (plaintext + encrypted).
    fn both_rotated_paths(&self, n: usize) -> [PathBuf; 2] {
        [
            self.encrypted_rotated_file(n),
            self.plaintext_rotated_file(n),
        ]
    }

    /// The rotated file actually on disk at slot `n`, preferring `.enc`.
    fn find_rotated_file(&self, n: usize) -> Option<PathBuf> {
        let enc = self.encrypted_rotated_file(n);
        if enc.exists() {
            return Some(enc);
        }
        let plain = self.plaintext_rotated_file(n);
        if plain.exists() {
            return Some(plain);
        }
        None
    }

    fn rotate(&self) -> Result<(), AuditStoreError> {
        // Remove the oldest rotated file (whichever flavor is present).
        if let Some(oldest) = self.find_rotated_file(MAX_ROTATED_FILES) {
            std::fs::remove_file(&oldest)?;
        }

        // Shift existing rotated files: N-1 → N, N-2 → N-1, etc. We preserve
        // each file's flavor (`.enc` stays `.enc`, `.jsonl` stays `.jsonl`) so
        // that a mid-flight migration doesn't silently rename things.
        for i in (1..MAX_ROTATED_FILES).rev() {
            if let Some(from) = self.find_rotated_file(i) {
                let to = if from.extension().and_then(|s| s.to_str()) == Some("enc") {
                    self.encrypted_rotated_file(i + 1)
                } else {
                    self.plaintext_rotated_file(i + 1)
                };
                std::fs::rename(&from, &to)?;
            }
        }

        // Move current → rotated.1, encrypting if a key is set.
        let current = self.current_file();
        if current.exists() {
            if let Some(key) = self.encryption_key.as_ref() {
                let dest = self.encrypted_rotated_file(1);
                audit_crypto::encrypt_file(&current, &dest, key)?;
            } else {
                std::fs::rename(&current, self.plaintext_rotated_file(1))?;
            }
        }

        tracing::debug!("Audit log rotated");
        Ok(())
    }

    fn load_file(&self, path: &Path) -> Result<Vec<AuditEvent>, AuditStoreError> {
        let bytes = read_audit_file(path, self.encryption_key.as_ref())?;
        parse_jsonl_events(&bytes)
    }

    /// Return the timestamp of the newest parseable event in a rotated file,
    /// decrypting first if the path ends in `.enc`.
    fn newest_event_timestamp(
        &self,
        path: &Path,
    ) -> Result<Option<DateTime<Utc>>, AuditStoreError> {
        let bytes = read_audit_file(path, self.encryption_key.as_ref())?;
        let events = parse_jsonl_events(&bytes)?;
        Ok(events.iter().map(|e| e.timestamp).max())
    }

    /// Encrypt any plaintext rotated `audit.N.jsonl` files in this store's
    /// directory to `audit.N.jsonl.enc` and remove the originals. Returns the
    /// number of files converted.
    ///
    /// Idempotent: returns 0 if nothing needs migrating, or if encryption is
    /// disabled on this store. Failures on individual files are logged but
    /// do not abort the run — the next launch will retry.
    pub fn migrate_plaintext_rotated_files(&self) -> Result<usize, AuditStoreError> {
        let Some(key) = self.encryption_key.as_ref() else {
            return Ok(0);
        };
        if !self.dir.exists() {
            return Ok(0);
        }
        let mut converted = 0usize;
        for i in 1..=MAX_ROTATED_FILES {
            let plain = self.plaintext_rotated_file(i);
            let enc = self.encrypted_rotated_file(i);
            if !plain.exists() {
                continue;
            }
            if enc.exists() {
                // Both forms exist at this slot (e.g. crash between encrypt and
                // delete). The encrypted file is authoritative; remove the
                // straggler so the slot is unambiguous.
                tracing::warn!(
                    "audit migration: both {} and {} exist; deleting plaintext as ciphertext is authoritative",
                    plain.display(),
                    enc.display()
                );
                if let Err(e) = std::fs::remove_file(&plain) {
                    tracing::warn!("failed to remove duplicate plaintext {}: {e}", plain.display());
                }
                continue;
            }
            match audit_crypto::encrypt_file(&plain, &enc, key) {
                Ok(()) => {
                    tracing::info!("audit migration: encrypted {} → {}", plain.display(), enc.display());
                    converted += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        "audit migration: failed to encrypt {}: {e}",
                        plain.display()
                    );
                }
            }
        }
        Ok(converted)
    }
}

/// Load every audit event reachable from `path`.
///
/// - If `path` is a directory, treats it like an `AuditStore` root: returns
///   rotated files (oldest first) + the active `audit.jsonl` (newest last).
/// - If `path` is a regular file, parses it as a single JSONL log
///   (transparently decrypting if `path` ends in `.enc`).
///
/// `key` is the AES-256-GCM sub-key from
/// [`thane_core::audit_keys::audit_aes_key`]. Pass `None` when verifying logs
/// that you know are plaintext; passing `None` while pointed at `.enc` files
/// returns [`AuditStoreError::EncryptedFileButNoKey`].
///
/// Designed for the offline verifier in `thane-cli audit verify`, where the
/// auditor might point us at either a hand-copied JSONL or a full audit dir.
pub fn load_events_from_path(
    path: &Path,
    key: Option<&[u8; 32]>,
) -> Result<Vec<AuditEvent>, AuditStoreError> {
    if path.is_dir() {
        let store = AuditStore::new(path.to_path_buf()).with_encryption_key(key.copied());
        store.load_all()
    } else {
        let bytes = read_audit_file(path, key)?;
        parse_jsonl_events(&bytes)
    }
}

/// Read a JSONL audit file from disk, skipping malformed lines.
///
/// Transparently decrypts the file if `path` ends in `.enc` and a `key` is
/// provided. Plaintext files do not require a key.
pub fn load_jsonl_events(
    path: &Path,
    key: Option<&[u8; 32]>,
) -> Result<Vec<AuditEvent>, AuditStoreError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = read_audit_file(path, key)?;
    parse_jsonl_events(&bytes)
}

/// Read an audit log file from disk, returning the plaintext bytes.
///
/// - If `path` ends in `.enc`, decrypts with `key` (returns
///   [`AuditStoreError::EncryptedFileButNoKey`] if `key` is `None`).
/// - Otherwise, returns the raw file contents.
pub fn read_audit_file(
    path: &Path,
    key: Option<&[u8; 32]>,
) -> Result<Vec<u8>, AuditStoreError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    if is_encrypted_path(path) {
        let Some(k) = key else {
            return Err(AuditStoreError::EncryptedFileButNoKey {
                path: path.to_path_buf(),
            });
        };
        Ok(audit_crypto::decrypt_file(path, k)?)
    } else {
        Ok(std::fs::read(path)?)
    }
}

fn is_encrypted_path(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("enc")
}

/// Parse a JSONL byte buffer into audit events, skipping malformed lines.
fn parse_jsonl_events(bytes: &[u8]) -> Result<Vec<AuditEvent>, AuditStoreError> {
    let mut events = Vec::new();
    let reader = std::io::BufReader::new(bytes);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<AuditEvent>(trimmed) {
            Ok(event) => events.push(event),
            Err(e) => {
                tracing::warn!("Skipping malformed audit line: {e}");
            }
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::{AuditEventType, AuditSeverity};
    use uuid::Uuid;

    fn make_event(desc: &str) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: desc.to_string(),
            metadata: serde_json::json!({}),
            agent_name: None,
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        }
    }

    #[test]
    fn test_append_and_load() {
        let dir = std::env::temp_dir().join("thane-audit-test-1");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        store.append(&make_event("event 1")).unwrap();
        store.append(&make_event("event 2")).unwrap();

        let events = store.load_current().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].description, "event 1");
        assert_eq!(events[1].description, "event 2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_clear() {
        let dir = std::env::temp_dir().join("thane-audit-test-2");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        store.append(&make_event("event")).unwrap();
        assert!(!store.load_current().unwrap().is_empty());

        store.clear().unwrap();
        assert!(store.load_current().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_json() {
        let dir = std::env::temp_dir().join("thane-audit-test-3");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        store.append(&make_event("export me")).unwrap();

        let json = store.export_json().unwrap();
        assert!(json.contains("export me"));

        let parsed: Vec<AuditEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flush_from_log() {
        let dir = std::env::temp_dir().join("thane-audit-test-4");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        let mut log = AuditLog::new(100);
        let ws_id = Uuid::new_v4();
        log.log(ws_id, None, AuditEventType::SecretAccess, AuditSeverity::Alert,
            "Secret accessed", serde_json::json!({}));
        log.log(ws_id, None, AuditEventType::PiiDetected, AuditSeverity::Alert,
            "PII found", serde_json::json!({}));

        store.flush(&log).unwrap();

        let events = store.load_current().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].description, "Secret accessed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_agent_name_persists_through_store() {
        let dir = std::env::temp_dir().join("thane-audit-test-agent-persist");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());

        let event_with_agent = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::AgentInvocation,
            severity: AuditSeverity::Info,
            description: "claude invoked".to_string(),
            metadata: serde_json::json!({"prompt": "fix bug"}),
            agent_name: Some("claude".to_string()),
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        };

        let event_without_agent = make_event("unattributed command");

        store.append(&event_with_agent).unwrap();
        store.append(&event_without_agent).unwrap();

        let loaded = store.load_current().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].agent_name.as_deref(), Some("claude"));
        assert_eq!(loaded[0].description, "claude invoked");
        assert_eq!(loaded[1].agent_name, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_multiple_agents_persist_through_store() {
        let dir = std::env::temp_dir().join("thane-audit-test-multi-agent");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        let ws = Uuid::new_v4();

        for (agent, desc) in &[("claude", "claude cmd"), ("codex", "codex cmd"), ("aider", "aider cmd")] {
            let event = AuditEvent {
                id: Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                workspace_id: ws,
                panel_id: None,
                event_type: AuditEventType::CommandExecuted,
                severity: AuditSeverity::Info,
                description: desc.to_string(),
                metadata: serde_json::json!({}),
                agent_name: Some(agent.to_string()),
                system_user: None,
                system_uid: None,
                prev_hash: String::new(),
                hmac: None,
            };
            store.append(&event).unwrap();
        }

        let loaded = store.load_current().unwrap();
        assert_eq!(loaded.len(), 3);

        let agents: Vec<_> = loaded.iter().map(|e| e.agent_name.as_deref().unwrap()).collect();
        assert_eq!(agents, vec!["claude", "codex", "aider"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rotation_preserves_events() {
        let dir = std::env::temp_dir().join("thane-audit-test-rotation");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());

        // Write enough data to trigger rotation by manually creating a large file
        // then appending more events
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join("audit.jsonl");

        // Write a file just under 5MB, then append to trigger rotation
        {
            let padding_event = make_event("padding");
            let line = serde_json::to_string(&padding_event).unwrap();
            let line_len = line.len() + 1; // +1 for newline
            let lines_needed = (MAX_FILE_SIZE as usize / line_len) + 1;

            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .open(&current)
                .unwrap();
            for _ in 0..lines_needed {
                writeln!(file, "{line}").unwrap();
            }
        }

        // This append should trigger rotation
        let post_rotation = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: "after rotation".to_string(),
            metadata: serde_json::json!({}),
            agent_name: Some("claude".to_string()),
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        };
        store.append(&post_rotation).unwrap();

        // The rotated file should exist
        assert!(dir.join("audit.1.jsonl").exists(), "rotation should create audit.1.jsonl");

        // Current file should have the new event
        let current_events = store.load_current().unwrap();
        assert_eq!(current_events.len(), 1);
        assert_eq!(current_events[0].description, "after rotation");
        assert_eq!(current_events[0].agent_name.as_deref(), Some("claude"));

        // load_all should return everything
        let all_events = store.load_all().unwrap();
        assert!(all_events.len() > 1, "load_all should include rotated + current events");
        assert_eq!(all_events.last().unwrap().description, "after rotation");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_all_ordering() {
        let dir = std::env::temp_dir().join("thane-audit-test-load-order");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();

        // Manually create a rotated file and a current file
        let old_event = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: "old event from rotation".to_string(),
            metadata: serde_json::json!({}),
            agent_name: Some("codex".to_string()),
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        };
        let rotated_file = dir.join("audit.1.jsonl");
        let line = serde_json::to_string(&old_event).unwrap();
        std::fs::write(&rotated_file, format!("{line}\n")).unwrap();

        // Append new event to current file
        let new_event = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::FileWrite,
            severity: AuditSeverity::Warning,
            description: "new event in current".to_string(),
            metadata: serde_json::json!({}),
            agent_name: Some("claude".to_string()),
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        };
        store.append(&new_event).unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 2);
        // Rotated (older) events come first, current (newer) events last
        assert_eq!(all[0].description, "old event from rotation");
        assert_eq!(all[0].agent_name.as_deref(), Some("codex"));
        assert_eq!(all[1].description, "new event in current");
        assert_eq!(all[1].agent_name.as_deref(), Some("claude"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_event_at(desc: &str, ts: chrono::DateTime<chrono::Utc>) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: ts,
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: desc.to_string(),
            metadata: serde_json::json!({}),
            agent_name: None,
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        }
    }

    fn write_jsonl(path: &std::path::Path, events: &[AuditEvent]) {
        let lines: Vec<String> = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn test_retention_deletes_rotated_file_past_window() {
        let dir = std::env::temp_dir().join(format!("thane-audit-retention-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let store = AuditStore::new(dir.clone()).with_retention_days(7);
        // Write a rotated file whose newest event is 30 days old.
        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(30);
        write_jsonl(&dir.join("audit.1.jsonl"), &[make_event_at("old", old_ts)]);

        // And a "recent" rotated file whose newest event is 2 days old.
        let recent_ts = now - chrono::Duration::days(2);
        write_jsonl(&dir.join("audit.2.jsonl"), &[make_event_at("recent", recent_ts)]);

        store.purge_expired(now).unwrap();

        assert!(!dir.join("audit.1.jsonl").exists(),
            "expired rotated file should be deleted");
        assert!(dir.join("audit.2.jsonl").exists(),
            "in-window rotated file must survive");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_retention_zero_means_keep_forever() {
        let dir = std::env::temp_dir().join(format!("thane-audit-retention-zero-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let store = AuditStore::new(dir.clone()).with_retention_days(0);
        let now = chrono::Utc::now();
        let ancient = now - chrono::Duration::days(10_000);
        write_jsonl(&dir.join("audit.1.jsonl"), &[make_event_at("ancient", ancient)]);

        store.purge_expired(now).unwrap();
        assert!(dir.join("audit.1.jsonl").exists(),
            "retention_days = 0 must not purge anything");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_retention_uses_newest_event_in_file() {
        // A file containing both ancient and recent events must NOT be deleted,
        // because the newest event is in-window.
        let dir = std::env::temp_dir().join(format!("thane-audit-retention-mixed-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let store = AuditStore::new(dir.clone()).with_retention_days(7);
        let now = chrono::Utc::now();
        let ancient = now - chrono::Duration::days(60);
        let recent = now - chrono::Duration::days(1);
        write_jsonl(&dir.join("audit.1.jsonl"), &[
            make_event_at("ancient", ancient),
            make_event_at("recent", recent),
        ]);

        store.purge_expired(now).unwrap();
        assert!(dir.join("audit.1.jsonl").exists(),
            "file must survive when its newest event is in-window");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_retention_does_not_touch_active_file() {
        // Even if the active audit.jsonl is full of ancient events, it must
        // never be deleted — only rotated files are subject to purge.
        let dir = std::env::temp_dir().join(format!("thane-audit-retention-active-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let store = AuditStore::new(dir.clone()).with_retention_days(1);
        let now = chrono::Utc::now();
        let ancient = now - chrono::Duration::days(1000);
        write_jsonl(&dir.join("audit.jsonl"), &[make_event_at("ancient active", ancient)]);

        store.purge_expired(now).unwrap();
        assert!(dir.join("audit.jsonl").exists(),
            "active audit.jsonl must never be purged");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flush_invokes_purge() {
        // flush() should opportunistically run the purge so retention applies
        // without a separate scheduled job.
        let dir = std::env::temp_dir().join(format!("thane-audit-retention-flush-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let store = AuditStore::new(dir.clone()).with_retention_days(7);
        let now = chrono::Utc::now();
        write_jsonl(&dir.join("audit.1.jsonl"), &[
            make_event_at("ancient", now - chrono::Duration::days(60)),
        ]);

        // flush an empty log — should still trigger purge.
        let log = AuditLog::new(10);
        store.flush(&log).unwrap();

        assert!(!dir.join("audit.1.jsonl").exists(),
            "flush should purge expired rotated files");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_with_retention_days_changes_returned_value() {
        let dir = std::env::temp_dir().join(format!("thane-audit-retention-ctor-{}", Uuid::new_v4()));
        let store = AuditStore::new(dir).with_retention_days(30);
        assert_eq!(store.retention_days(), 30);
    }

    #[test]
    fn test_total_size_accounts_for_rotated_files() {
        let dir = std::env::temp_dir().join("thane-audit-test-total-size");
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();

        // Create a rotated file with some content
        std::fs::write(dir.join("audit.1.jsonl"), "old data\n").unwrap();

        // Append to current
        store.append(&make_event("current")).unwrap();

        let size = store.total_size();
        assert!(size > 0);

        // Size should include both files
        let rotated_size = std::fs::metadata(dir.join("audit.1.jsonl")).unwrap().len();
        let current_size = std::fs::metadata(dir.join("audit.jsonl")).unwrap().len();
        assert_eq!(size, rotated_size + current_size);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Phase 4: encryption-at-rest ─────────────────────────────────────────

    /// Build a deterministic AES-256-GCM key for tests so failures are
    /// reproducible.
    fn test_aes_key() -> [u8; 32] {
        [0x42; 32]
    }

    /// Fill `dir`'s audit.jsonl with enough padding events to exceed MAX_FILE_SIZE.
    /// Returns the number of padding lines written.
    fn fill_to_force_rotation(dir: &std::path::Path) -> usize {
        std::fs::create_dir_all(dir).unwrap();
        let current = dir.join("audit.jsonl");
        let padding_event = make_event("padding");
        let line = serde_json::to_string(&padding_event).unwrap();
        let line_len = line.len() + 1;
        let lines_needed = (MAX_FILE_SIZE as usize / line_len) + 1;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&current)
            .unwrap();
        for _ in 0..lines_needed {
            writeln!(file, "{line}").unwrap();
        }
        lines_needed
    }

    #[test]
    fn rotation_produces_enc_files_only_when_encryption_enabled() {
        let dir = std::env::temp_dir().join(format!("thane-audit-encrypt-rot-{}", Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone()).with_encryption_key(Some(test_aes_key()));

        // Force the active file to exceed MAX_FILE_SIZE.
        let _padding = fill_to_force_rotation(&dir);

        // The next append triggers rotation.
        store.append(&make_event("after rotation")).unwrap();

        assert!(
            dir.join("audit.1.jsonl.enc").exists(),
            "rotation under encryption must produce audit.1.jsonl.enc"
        );
        assert!(
            !dir.join("audit.1.jsonl").exists(),
            "no plaintext rotated file should remain at slot 1"
        );
        // All slots: nothing should be plaintext.
        for i in 1..=MAX_ROTATED_FILES {
            assert!(
                !dir.join(format!("audit.{i}.jsonl")).exists(),
                "plaintext rotated file at slot {i} must not exist"
            );
        }
        // Active file is the new one — and it stays plaintext.
        assert!(dir.join("audit.jsonl").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_all_decrypts_rotated_enc_files_transparently() {
        let dir = std::env::temp_dir().join(format!("thane-audit-encrypt-load-{}", Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&dir);

        let store = AuditStore::new(dir.clone()).with_encryption_key(Some(test_aes_key()));

        fill_to_force_rotation(&dir);
        store.append(&make_event("post-rotation")).unwrap();

        // load_all must return events from BOTH the encrypted rotated file and
        // the plaintext current file, in the right order.
        let events = store.load_all().unwrap();
        assert!(
            events.len() > 1,
            "load_all must include rotated + current events"
        );
        assert_eq!(events.last().unwrap().description, "post-rotation");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_encrypts_pre_existing_plaintext_files() {
        let dir = std::env::temp_dir().join(format!("thane-audit-migrate-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        // Pre-populate two plaintext rotated files (the pre-Phase-4 layout).
        let e1 = make_event("rotated 1");
        let e2 = make_event("rotated 2");
        write_jsonl(&dir.join("audit.1.jsonl"), std::slice::from_ref(&e1));
        write_jsonl(&dir.join("audit.2.jsonl"), std::slice::from_ref(&e2));

        let store = AuditStore::new(dir.clone()).with_encryption_key(Some(test_aes_key()));
        let converted = store.migrate_plaintext_rotated_files().unwrap();
        assert_eq!(converted, 2, "both plaintext rotated files should convert");

        // No plaintext rotated files remain.
        assert!(!dir.join("audit.1.jsonl").exists());
        assert!(!dir.join("audit.2.jsonl").exists());
        // The .enc files are in place.
        assert!(dir.join("audit.1.jsonl.enc").exists());
        assert!(dir.join("audit.2.jsonl.enc").exists());

        // Events still readable via the store.
        let events = store.load_all().unwrap();
        let descs: Vec<&str> = events.iter().map(|e| e.description.as_str()).collect();
        assert!(descs.contains(&"rotated 1"));
        assert!(descs.contains(&"rotated 2"));

        // Idempotency: running it again is a no-op.
        let again = store.migrate_plaintext_rotated_files().unwrap();
        assert_eq!(again, 0, "second migration call should encrypt nothing");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_is_noop_when_encryption_disabled() {
        let dir = std::env::temp_dir().join(format!("thane-audit-migrate-off-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        write_jsonl(&dir.join("audit.1.jsonl"), &[make_event("rotated 1")]);

        let store = AuditStore::new(dir.clone()); // no encryption key
        let converted = store.migrate_plaintext_rotated_files().unwrap();
        assert_eq!(converted, 0);
        // Plaintext file must be left in place.
        assert!(dir.join("audit.1.jsonl").exists());
        assert!(!dir.join("audit.1.jsonl.enc").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_prefers_existing_encrypted_file_when_both_present() {
        // Simulates a crash between encrypt-success and plaintext-delete: both
        // forms exist at the same slot. The encrypted file wins; the plaintext
        // is cleaned up.
        let dir = std::env::temp_dir().join(format!("thane-audit-migrate-both-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = test_aes_key();

        // Authoritative encrypted file:
        write_jsonl(&dir.join("audit.1.jsonl.tmp_plain"), &[make_event("authoritative")]);
        audit_crypto::encrypt_file(
            &dir.join("audit.1.jsonl.tmp_plain"),
            &dir.join("audit.1.jsonl.enc"),
            &key,
        )
        .unwrap();

        // Straggler plaintext at the same slot:
        write_jsonl(&dir.join("audit.1.jsonl"), &[make_event("straggler")]);

        let store = AuditStore::new(dir.clone()).with_encryption_key(Some(key));
        let converted = store.migrate_plaintext_rotated_files().unwrap();
        assert_eq!(converted, 0, "must NOT overwrite the existing .enc");
        assert!(!dir.join("audit.1.jsonl").exists(), "straggler must be removed");
        assert!(dir.join("audit.1.jsonl.enc").exists());
        // Loading sees only the authoritative event.
        let events = store.load_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].description, "authoritative");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_shifts_existing_enc_files_when_encryption_enabled() {
        let dir = std::env::temp_dir().join(format!("thane-audit-shift-enc-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = test_aes_key();

        // Pre-populate audit.1.jsonl.enc with a known event.
        write_jsonl(&dir.join("audit.1.jsonl.staging"), &[make_event("slot-1-original")]);
        audit_crypto::encrypt_file(
            &dir.join("audit.1.jsonl.staging"),
            &dir.join("audit.1.jsonl.enc"),
            &key,
        )
        .unwrap();

        let store = AuditStore::new(dir.clone()).with_encryption_key(Some(key));
        fill_to_force_rotation(&dir);
        store.append(&make_event("post-rotation")).unwrap();

        // Slot 1 now holds the freshly encrypted previous-active file.
        // The old slot-1 file should have shifted to slot 2.
        assert!(dir.join("audit.1.jsonl.enc").exists());
        assert!(dir.join("audit.2.jsonl.enc").exists());

        // Slot 2's plaintext should match what was originally in slot 1.
        let slot2 = audit_crypto::decrypt_file(&dir.join("audit.2.jsonl.enc"), &key).unwrap();
        let events = parse_jsonl_events(&slot2).unwrap();
        assert_eq!(events[0].description, "slot-1-original");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_audit_file_errors_when_enc_file_has_no_key() {
        let dir = std::env::temp_dir().join(format!("thane-audit-no-key-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = test_aes_key();

        write_jsonl(&dir.join("audit.1.jsonl.staging"), &[make_event("encrypted")]);
        audit_crypto::encrypt_file(
            &dir.join("audit.1.jsonl.staging"),
            &dir.join("audit.1.jsonl.enc"),
            &key,
        )
        .unwrap();

        let err = read_audit_file(&dir.join("audit.1.jsonl.enc"), None).unwrap_err();
        assert!(matches!(err, AuditStoreError::EncryptedFileButNoKey { .. }));

        // With the key, it works.
        let bytes = read_audit_file(&dir.join("audit.1.jsonl.enc"), Some(&key)).unwrap();
        assert!(!bytes.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_events_from_path_handles_enc_directory() {
        // The CLI verify command points us at a directory. It must transparently
        // handle a mix of `.enc` rotated files and the plaintext active file.
        let dir = std::env::temp_dir().join(format!(
            "thane-audit-load-from-path-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let key = test_aes_key();

        // Encrypted rotated file.
        write_jsonl(&dir.join("audit.1.jsonl.tmp"), &[make_event("rotated-enc")]);
        audit_crypto::encrypt_file(
            &dir.join("audit.1.jsonl.tmp"),
            &dir.join("audit.1.jsonl.enc"),
            &key,
        )
        .unwrap();

        // Plaintext active file.
        write_jsonl(&dir.join("audit.jsonl"), &[make_event("active")]);

        let events = load_events_from_path(&dir, Some(&key)).unwrap();
        let descs: Vec<&str> = events.iter().map(|e| e.description.as_str()).collect();
        assert!(descs.contains(&"rotated-enc"));
        assert!(descs.contains(&"active"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encryption_enabled_reports_state() {
        let dir = std::env::temp_dir().join(format!("thane-audit-enc-state-{}", Uuid::new_v4()));
        let off = AuditStore::new(dir.clone());
        assert!(!off.encryption_enabled());
        let on = AuditStore::new(dir).with_encryption_key(Some(test_aes_key()));
        assert!(on.encryption_enabled());
    }
}
