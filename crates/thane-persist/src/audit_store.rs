use std::io::{BufRead, Write};
use std::path::PathBuf;

use thane_core::audit::{AuditEvent, AuditLog};
use thiserror::Error;

/// Maximum audit log file size before rotation (5 MB).
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// Maximum number of rotated log files to keep.
const MAX_ROTATED_FILES: usize = 5;

#[derive(Debug, Error)]
pub enum AuditStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// File-based audit log storage using JSONL format (one event per line).
///
/// Supports:
/// - Append-only writing for crash safety
/// - Automatic rotation when file exceeds 5 MB
/// - Loading events from current + rotated files
/// - Export to JSON array format
pub struct AuditStore {
    dir: PathBuf,
}

impl AuditStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
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

    /// Flush all events from an in-memory AuditLog to disk.
    pub fn flush(&self, log: &AuditLog) -> Result<(), AuditStoreError> {
        for event in log.all() {
            self.append(event)?;
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
            let path = self.rotated_file(i);
            if path.exists() {
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
            let path = self.rotated_file(i);
            if path.exists() {
                std::fs::remove_file(&path)?;
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
            let path = self.rotated_file(i);
            if let Ok(meta) = path.metadata() {
                size += meta.len();
            }
        }
        size
    }

    fn current_file(&self) -> PathBuf {
        self.dir.join("audit.jsonl")
    }

    fn rotated_file(&self, n: usize) -> PathBuf {
        self.dir.join(format!("audit.{n}.jsonl"))
    }

    fn rotate(&self) -> Result<(), AuditStoreError> {
        // Remove the oldest rotated file.
        let oldest = self.rotated_file(MAX_ROTATED_FILES);
        if oldest.exists() {
            std::fs::remove_file(&oldest)?;
        }

        // Shift existing rotated files: N-1 → N, N-2 → N-1, etc.
        for i in (1..MAX_ROTATED_FILES).rev() {
            let from = self.rotated_file(i);
            let to = self.rotated_file(i + 1);
            if from.exists() {
                std::fs::rename(&from, &to)?;
            }
        }

        // Move current → rotated.1
        let current = self.current_file();
        if current.exists() {
            std::fs::rename(&current, self.rotated_file(1))?;
        }

        tracing::debug!("Audit log rotated");
        Ok(())
    }

    fn load_file(&self, path: &PathBuf) -> Result<Vec<AuditEvent>, AuditStoreError> {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut events = Vec::new();

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
            prev_hash: String::new(),
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
            prev_hash: String::new(),
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
                prev_hash: String::new(),
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
            prev_hash: String::new(),
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
            prev_hash: String::new(),
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
            prev_hash: String::new(),
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
}
