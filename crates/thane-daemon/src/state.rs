//! Shared in-memory state for the daemon (queue, audit log, config).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use anyhow::Result;
use thane_audit_sink::{AuditDispatcher, SinkStatusReport};
use thane_core::agent_queue::AgentQueue;
use thane_core::audit::AuditLog;
use thane_core::config::Config;
use thane_persist::audit_store::AuditStore;
use thane_persist::queue_history_store::QueueHistoryStore;
use thane_platform::traits::PlatformDirs;

use crate::platform_dirs;

/// Persistent file name for the live (in-progress) queue snapshot.
const QUEUE_LIVE_FILE: &str = "queue_live.json";

/// All mutable daemon state lives under a single mutex. The daemon is not
/// performance-critical (RPC throughput is tiny), so a single global lock
/// keeps the design simple and free of subtle ordering bugs.
pub struct DaemonState {
    inner: Mutex<Inner>,
    started_at: Instant,
    socket_path: PathBuf,
    /// Phase 5 external-sink dispatcher. Lives outside `Inner` so the RPC
    /// path can take its async status snapshot without holding the global
    /// mutex.
    dispatcher: Option<Arc<AuditDispatcher>>,
}

struct Inner {
    queue: AgentQueue,
    audit: AuditLog,
    /// Number of audit events that have already been flushed to disk.
    audit_flushed_count: usize,
    config: Config,
    config_mtime: Option<SystemTime>,
    audit_store: AuditStore,
    queue_history: QueueHistoryStore,
    queue_live_path: PathBuf,
}

impl DaemonState {
    /// Construct daemon state. Loads persisted config; restores any live queue
    /// snapshot the previous daemon process left behind.
    pub fn new(started_at: Instant, socket_path: PathBuf) -> Result<Self> {
        let dirs = platform_dirs();
        let sessions_dir = dirs.sessions_dir();

        let mut config = Config::load_default();
        let config_mtime = config
            .source_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());

        // Phase 6b: stack any deployed enterprise policy on top of user
        // config. Failure here is non-fatal; we defer the alert event until
        // the audit log is wired up below.
        let policy_load_err = match thane_core::policy::load_for_platform() {
            Ok(Some(policy)) => {
                tracing::info!(
                    "enterprise policy active (issued by {:?}, {} locked keys)",
                    policy.issued_by,
                    policy.locked_keys.len()
                );
                config.apply_policy(Arc::new(policy));
                None
            }
            Ok(None) => None,
            Err(e) => {
                tracing::error!("enterprise policy load failed: {e}; continuing with user config");
                Some(e.to_string())
            }
        };

        let secret_store = thane_platform::default_secret_store();

        let encryption_key = if config.audit_encryption_enabled() {
            match thane_core::audit_keys::try_audit_aes_key(secret_store.as_ref()) {
                Ok(key) => Some(key),
                Err(e) => {
                    tracing::warn!(
                        "audit-encryption-enabled=true but AES key unavailable; rotated audit files will stay plaintext: {e}"
                    );
                    None
                }
            }
        } else {
            tracing::warn!(
                "audit-encryption-enabled=false — rotated audit files will be written in plaintext"
            );
            None
        };

        let audit_store = AuditStore::new(sessions_dir.clone())
            .with_retention_days(config.audit_retention_days())
            .with_encryption_key(encryption_key);
        let mut audit = AuditLog::new(10_000)
            .with_redaction_policy(config.audit_redaction_policy());
        if config.audit_signing_enabled() {
            match thane_core::audit_keys::try_audit_hmac_key(secret_store.as_ref()) {
                Ok(key) => audit.set_signing_key(key),
                Err(e) => tracing::warn!(
                    "audit-signing-enabled=true but HMAC key unavailable; daemon events will be unsigned: {e}"
                ),
            }
        }

        // Phase 5: build the external-sink dispatcher (if any sink enabled)
        // and attach it as the AuditLog's forwarder. Every locally-recorded
        // event (post redaction + signing) is then non-blockingly handed to
        // the dispatcher's MPSC channel.
        let dispatcher = thane_audit_sink::build_dispatcher_from_config(
            &config,
            secret_store.as_ref(),
            sessions_dir.clone(),
        )
        .map(Arc::new);
        if let Some(d) = dispatcher.as_ref() {
            audit.set_forwarder(Arc::new(d.handle()));
            tracing::info!("audit external sinks active");
        }

        // Phase 6b: emit deferred enterprise-policy-load failure now that the
        // log + sinks are up. Alert severity so SIEM picks it up.
        if let Some(err) = policy_load_err {
            audit.log(
                uuid::Uuid::nil(),
                None,
                thane_core::audit::AuditEventType::Custom(
                    "enterprise_policy_load_failed".to_string(),
                ),
                thane_core::audit::AuditSeverity::Alert,
                "Failed to load enterprise policy; falling back to user config",
                serde_json::json!({ "error": err }),
            );
        }

        // One-shot migration: encrypt any leftover plaintext rotated files.
        if encryption_key.is_some() {
            match audit_store.migrate_plaintext_rotated_files() {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!("audit migration: encrypted {n} plaintext rotated file(s)");
                    audit.log(
                        uuid::Uuid::nil(),
                        None,
                        thane_core::audit::AuditEventType::Custom(
                            "audit_migration_encrypted".to_string(),
                        ),
                        thane_core::audit::AuditSeverity::Info,
                        format!("Encrypted {n} pre-existing plaintext audit file(s) at rest"),
                        serde_json::json!({ "files_converted": n }),
                    );
                }
                Err(e) => tracing::warn!("audit migration failed: {e}"),
            }
        }

        let queue_history = QueueHistoryStore::new(sessions_dir.clone());
        let queue_live_path = sessions_dir.join(QUEUE_LIVE_FILE);
        let queue = restore_queue(&queue_live_path);

        Ok(Self {
            inner: Mutex::new(Inner {
                queue,
                audit,
                audit_flushed_count: 0,
                config,
                config_mtime,
                audit_store,
                queue_history,
                queue_live_path,
            }),
            started_at,
            socket_path,
            dispatcher,
        })
    }

    /// Borrow the external-sink dispatcher, if one is configured. Used by the
    /// RPC layer to answer `audit.sink_status` and by the DLQ retry path.
    pub fn dispatcher(&self) -> Option<&Arc<AuditDispatcher>> {
        self.dispatcher.as_ref()
    }

    /// Synchronous status snapshot. `None` when no dispatcher is configured.
    pub fn sink_status(&self) -> Option<SinkStatusReport> {
        self.dispatcher
            .as_ref()
            .map(|d| d.status_snapshot_blocking())
    }

    /// Path of the DLQ file, derived from the sessions dir. Returns the path
    /// even when no dispatcher is configured — operators may still inspect
    /// historical entries from a previous run.
    pub fn dlq(&self) -> thane_audit_sink::DeadLetterQueue {
        let dirs = platform_dirs();
        thane_audit_sink::DeadLetterQueue::new(dirs.sessions_dir())
    }

    /// Time the daemon has been running, in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// The socket path the daemon bound to.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Borrow the queue for a short critical section.
    pub fn with_queue<R>(&self, f: impl FnOnce(&mut AgentQueue) -> R) -> R {
        let mut inner = self.inner.lock().expect("daemon state poisoned");
        f(&mut inner.queue)
    }

    /// Borrow the audit log for a short critical section.
    pub fn with_audit<R>(&self, f: impl FnOnce(&mut AuditLog) -> R) -> R {
        let mut inner = self.inner.lock().expect("daemon state poisoned");
        f(&mut inner.audit)
    }

    /// Persist any not-yet-flushed audit events to disk.
    ///
    /// Maintains a cursor (`audit_flushed_count`) so events are written at
    /// most once. We do NOT call `AuditLog::clear()` because that records a
    /// "cleared" marker event, which would write an entry every flush cycle.
    pub fn flush_audit(&self) {
        let mut inner = self.inner.lock().expect("daemon state poisoned");
        let all = inner.audit.all();
        let already = inner.audit_flushed_count;
        if all.len() <= already {
            return;
        }
        // Collect the slice we still need to write before releasing immutable
        // borrow of `inner.audit`.
        let to_write: Vec<_> = all[already..].to_vec();
        let new_total = all.len();
        for event in &to_write {
            if let Err(e) = inner.audit_store.append(event) {
                tracing::warn!("audit append failed: {e}");
                return; // Don't advance cursor; retry next tick.
            }
        }
        inner.audit_flushed_count = new_total;
    }

    /// Save the live queue snapshot atomically.
    pub fn save_queue(&self) {
        let mut inner = self.inner.lock().expect("daemon state poisoned");
        let queue_json = match serde_json::to_vec_pretty(&inner.queue) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("queue serialize failed: {e}");
                return;
            }
        };
        let path = inner.queue_live_path.clone();
        if let Err(e) = write_atomic(&path, &queue_json) {
            tracing::warn!("queue snapshot save failed: {e}");
        }

        // Also append any newly terminal entries to the queue-history file
        // so they survive a daemon restart for the user-facing history view.
        let completed: Vec<_> = inner.queue.completed_entries().into_iter().cloned().collect();
        for entry in completed {
            if let Err(e) = inner.queue_history.append(&entry) {
                tracing::warn!("queue history append failed: {e}");
            }
        }
    }

    /// Re-read the config file from disk if its mtime changed.
    pub fn reload_config_if_changed(&self) {
        let mut inner = self.inner.lock().expect("daemon state poisoned");
        let Some(path) = inner.config.source_path.clone() else { return };
        let Ok(meta) = std::fs::metadata(&path) else { return };
        let Ok(mtime) = meta.modified() else { return };
        if inner.config_mtime == Some(mtime) {
            return;
        }
        match Config::load(&path) {
            Ok(new_config) => {
                inner.config = new_config;
                inner.config_mtime = Some(mtime);
                tracing::info!("config reloaded from {}", path.display());
            }
            Err(e) => tracing::warn!("config reload failed: {e}"),
        }
    }

    /// Snapshot the config in a short critical section.
    pub fn config_snapshot(&self) -> Config {
        self.inner.lock().expect("daemon state poisoned").config.clone()
    }
}

fn restore_queue(path: &std::path::Path) -> AgentQueue {
    if !path.exists() {
        return AgentQueue::new();
    }
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!("queue snapshot parse failed ({e}), starting fresh");
            AgentQueue::new()
        }),
        Err(e) => {
            tracing::warn!("queue snapshot read failed ({e}), starting fresh");
            AgentQueue::new()
        }
    }
}

fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
