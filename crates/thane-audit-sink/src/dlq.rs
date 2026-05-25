//! On-disk dead-letter queue.
//!
//! One JSONL file (`audit-dlq.jsonl`) sits next to the active audit log.
//! Writes are append-only and serialized through a single mutex so the file
//! can't be corrupted by concurrent writers from many sink tasks. Entries are
//! the FULL audit event plus enough metadata for an operator to figure out
//! what went wrong and decide whether to retry.

use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use thane_core::audit::AuditEvent;

/// One row written to `audit-dlq.jsonl` when a sink delivery fails for good.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterEntry {
    pub failed_at: DateTime<Utc>,
    pub sink: String,
    pub error: String,
    pub attempts: u32,
    pub event: AuditEvent,
}

/// Append-only DLQ file. All sinks share a single instance so writes are
/// serialized.
pub struct DeadLetterQueue {
    path: PathBuf,
    lock: Mutex<()>,
}

impl DeadLetterQueue {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        let mut path = audit_dir.into();
        path.push("audit-dlq.jsonl");
        Self { path, lock: Mutex::new(()) }
    }

    /// Absolute path of the DLQ file (created lazily on first write).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a regular DLQ entry for a sink failure.
    pub fn write_failure(
        &self,
        sink: &str,
        error: &str,
        attempts: u32,
        event: &AuditEvent,
    ) {
        let entry = DeadLetterEntry {
            failed_at: Utc::now(),
            sink: sink.to_string(),
            error: error.to_string(),
            attempts,
            event: event.clone(),
        };
        self.append(&entry);
    }

    /// Convenience: write an "overflow" entry (event spilled from the
    /// dispatcher channel before any sink even saw it). `attempts = 0` because
    /// no sink was attempted.
    pub fn write_overflow(&self, event: &AuditEvent, reason: &str) {
        let entry = DeadLetterEntry {
            failed_at: Utc::now(),
            sink: "dispatcher".to_string(),
            error: reason.to_string(),
            attempts: 0,
            event: event.clone(),
        };
        self.append(&entry);
    }

    /// Read everything in the DLQ, newest entries last (file order).
    /// Malformed lines are skipped with a warning.
    pub fn read_all(&self) -> io::Result<Vec<DeadLetterEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let f = std::fs::File::open(&self.path)?;
        let reader = BufReader::new(f);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<DeadLetterEntry>(trimmed) {
                Ok(e) => out.push(e),
                Err(e) => tracing::warn!("skipping malformed DLQ line: {e}"),
            }
        }
        Ok(out)
    }

    /// Filter DLQ entries by sink name.
    pub fn read_by_sink(&self, sink: &str) -> io::Result<Vec<DeadLetterEntry>> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|e| e.sink == sink)
            .collect())
    }

    /// Total entries in the DLQ (best-effort; returns 0 on read error).
    pub fn count(&self) -> u64 {
        self.read_all().map(|v| v.len() as u64).unwrap_or(0)
    }

    /// Count entries belonging to a specific sink.
    pub fn count_for_sink(&self, sink: &str) -> u64 {
        self.read_by_sink(sink).map(|v| v.len() as u64).unwrap_or(0)
    }

    /// Truncate the DLQ. Intended for `thane-cli audit dlq clear` — caller
    /// must already have confirmed admin policy allows it (Phase 1).
    pub fn clear(&self) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    fn append(&self, entry: &DeadLetterEntry) {
        let line = match serde_json::to_string(entry) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("DLQ serialize failed for event {}: {e}", entry.event.id);
                return;
            }
        };

        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());

        if let Some(parent) = self.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("DLQ mkdir failed for {}: {e}", parent.display());
            return;
        }

        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("DLQ open failed for {}: {e}", self.path.display());
                return;
            }
        };
        if let Err(e) = writeln!(file, "{line}") {
            tracing::error!("DLQ write failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::{AuditEventType, AuditSeverity};
    use uuid::Uuid;

    fn fake_event(desc: &str) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
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
    fn write_and_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let dlq = DeadLetterQueue::new(dir.path());
        dlq.write_failure("syslog", "host unreachable", 5, &fake_event("one"));
        dlq.write_failure("webhook", "500 Internal", 5, &fake_event("two"));
        let rows = dlq.read_all().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].sink, "syslog");
        assert_eq!(rows[0].attempts, 5);
        assert_eq!(rows[1].event.description, "two");
    }

    #[test]
    fn overflow_entry_has_attempts_zero() {
        let dir = tempfile::tempdir().unwrap();
        let dlq = DeadLetterQueue::new(dir.path());
        dlq.write_overflow(&fake_event("oops"), "queue_overflow");
        let rows = dlq.read_all().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sink, "dispatcher");
        assert_eq!(rows[0].attempts, 0);
        assert_eq!(rows[0].error, "queue_overflow");
    }

    #[test]
    fn filter_by_sink_works() {
        let dir = tempfile::tempdir().unwrap();
        let dlq = DeadLetterQueue::new(dir.path());
        dlq.write_failure("syslog", "x", 1, &fake_event("a"));
        dlq.write_failure("webhook", "y", 1, &fake_event("b"));
        dlq.write_failure("syslog", "z", 1, &fake_event("c"));
        let syslog_only = dlq.read_by_sink("syslog").unwrap();
        assert_eq!(syslog_only.len(), 2);
        assert_eq!(dlq.count_for_sink("syslog"), 2);
        assert_eq!(dlq.count_for_sink("webhook"), 1);
    }

    #[test]
    fn clear_removes_everything() {
        let dir = tempfile::tempdir().unwrap();
        let dlq = DeadLetterQueue::new(dir.path());
        dlq.write_overflow(&fake_event("x"), "queue_overflow");
        assert_eq!(dlq.count(), 1);
        dlq.clear().unwrap();
        assert_eq!(dlq.count(), 0);
    }
}
