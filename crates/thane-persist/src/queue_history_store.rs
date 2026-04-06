use std::path::PathBuf;

use chrono::{Duration, Utc};
use thane_core::agent_queue::QueueEntry;

use crate::store::StoreError;

/// Lightweight summary of queue history for status bar display.
///
/// Avoids deserializing the full entry list on every status bar refresh.
#[derive(Debug, Clone, Copy)]
pub struct QueueHistorySummary {
    pub entry_count: usize,
    pub alltime_cost: f64,
}

/// Maximum number of entries to keep in the history file.
const MAX_ENTRIES: usize = 500;

/// Maximum age (in days) for completed entries before pruning.
const MAX_AGE_DAYS: i64 = 90;

/// Atomic file-based storage for completed queue entries.
///
/// Uses the same temp + rename pattern as `HistoryStore`.
/// Maintains an in-memory summary cache to avoid redundant disk I/O
/// on hot paths like `refresh_status_bar()`.
pub struct QueueHistoryStore {
    dir: PathBuf,
    cached_summary: Option<QueueHistorySummary>,
}

impl QueueHistoryStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            cached_summary: None,
        }
    }

    /// Save a list of queue entries atomically.
    pub fn save(&self, entries: &[QueueEntry]) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;

        let json = serde_json::to_string_pretty(entries)?;
        let final_path = self.history_file();
        let temp_path = self
            .dir
            .join(format!("queue_history.json.{}.tmp", std::process::id()));

        std::fs::write(&temp_path, &json)?;
        std::fs::rename(&temp_path, &final_path)?;

        tracing::debug!("Queue history saved to {}", final_path.display());
        Ok(())
    }

    /// Load queue history. Returns an empty vec if the file doesn't exist.
    pub fn load(&self) -> Result<Vec<QueueEntry>, StoreError> {
        let path = self.history_file();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let json = std::fs::read_to_string(&path)?;
        let entries: Vec<QueueEntry> = serde_json::from_str(&json)?;

        tracing::debug!("Queue history loaded from {}", path.display());
        Ok(entries)
    }

    /// Return a cached summary (entry count + total cost).
    ///
    /// On the first call, reads from disk and caches the result. Subsequent
    /// calls return the cached value without any I/O. The cache is updated
    /// atomically by `append()` and can be cleared with `invalidate_cache()`.
    pub fn summary(&mut self) -> Result<QueueHistorySummary, StoreError> {
        if let Some(cached) = self.cached_summary {
            return Ok(cached);
        }

        let entries = self.load()?;
        let summary = QueueHistorySummary {
            entry_count: entries.len(),
            alltime_cost: entries.iter().map(|e| e.tokens_used.estimated_cost_usd).sum(),
        };
        self.cached_summary = Some(summary);
        Ok(summary)
    }

    /// Clear the cached summary so the next `summary()` call re-reads from disk.
    pub fn invalidate_cache(&mut self) {
        self.cached_summary = None;
    }

    /// Append a single entry to the history file (load + prune + append + save).
    ///
    /// Prunes old/excess entries before saving, and updates the summary cache
    /// atomically after a successful write.
    pub fn append(&mut self, entry: &QueueEntry) -> Result<(), StoreError> {
        let mut entries = self.load()?;
        entries.push(entry.clone());

        Self::prune_entries(&mut entries);

        self.save(&entries)?;

        // Update cache atomically after successful save.
        self.cached_summary = Some(QueueHistorySummary {
            entry_count: entries.len(),
            alltime_cost: entries.iter().map(|e| e.tokens_used.estimated_cost_usd).sum(),
        });

        Ok(())
    }

    /// Remove entries that exceed the count cap or are older than the age limit.
    fn prune_entries(entries: &mut Vec<QueueEntry>) {
        let cutoff = Utc::now() - Duration::days(MAX_AGE_DAYS);

        // Remove entries older than the age limit.
        entries.retain(|e| e.completed_at.is_none_or(|t| t > cutoff));

        // If still over the count cap, drop the oldest entries.
        if entries.len() > MAX_ENTRIES {
            let excess = entries.len() - MAX_ENTRIES;
            entries.drain(..excess);
        }
    }

    fn history_file(&self) -> PathBuf {
        self.dir.join("queue_history.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::agent_queue::{AgentQueue, QueueEntryStatus, QueueTokenUsage};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thane-test-qhs-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn make_entry(queue: &mut AgentQueue, content: &str, cost: f64) -> QueueEntry {
        let id = queue.submit(content.into(), None, 0);
        queue.start(id);
        queue.update_tokens(id, QueueTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: cost,
            model: None,
        });
        queue.complete(id);
        queue.get(id).unwrap().clone()
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = temp_dir("roundtrip");

        let store = QueueHistoryStore::new(dir.clone());

        let mut queue = AgentQueue::new();
        let id = queue.submit("Test task".into(), None, 0);
        queue.start(id);
        queue.complete(id);

        let entry = queue.get(id).unwrap().clone();
        store.save(&[entry]).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "Test task");
        assert_eq!(loaded[0].status, QueueEntryStatus::Completed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_append() {
        let dir = temp_dir("append");

        let mut store = QueueHistoryStore::new(dir.clone());

        let mut queue = AgentQueue::new();
        let id1 = queue.submit("Task A".into(), None, 0);
        queue.start(id1);
        queue.complete(id1);
        let entry1 = queue.get(id1).unwrap().clone();

        let id2 = queue.submit("Task B".into(), None, 0);
        queue.start(id2);
        queue.fail(id2, "error".into());
        let entry2 = queue.get(id2).unwrap().clone();

        store.append(&entry1).unwrap();
        store.append(&entry2).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "Task A");
        assert_eq!(loaded[1].content, "Task B");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let dir = temp_dir("noexist");

        let store = QueueHistoryStore::new(dir);
        let entries = store.load().unwrap();
        assert!(entries.is_empty());
    }

    // --- New tests for caching and pruning ---

    #[test]
    fn test_summary_values() {
        let dir = temp_dir("summary-values");
        let mut store = QueueHistoryStore::new(dir.clone());

        let mut queue = AgentQueue::new();
        let e1 = make_entry(&mut queue, "Task 1", 0.50);
        let e2 = make_entry(&mut queue, "Task 2", 1.25);

        store.append(&e1).unwrap();
        store.append(&e2).unwrap();

        let summary = store.summary().unwrap();
        assert_eq!(summary.entry_count, 2);
        assert!((summary.alltime_cost - 1.75).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_summary_caching_avoids_repeated_load() {
        let dir = temp_dir("summary-cache");
        let mut store = QueueHistoryStore::new(dir.clone());

        let mut queue = AgentQueue::new();
        let e1 = make_entry(&mut queue, "Task 1", 0.10);
        store.append(&e1).unwrap();

        // First call populates cache.
        let s1 = store.summary().unwrap();
        assert_eq!(s1.entry_count, 1);

        // Tamper with file on disk — cache should shield us.
        let e2 = make_entry(&mut queue, "Task 2", 0.20);
        // Write directly to disk (bypassing append, so cache is stale).
        store.save(&[e1.clone(), e2]).unwrap();

        // Should still return cached value (1 entry, not 2).
        let s2 = store.summary().unwrap();
        assert_eq!(s2.entry_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_invalidate_cache_forces_reload() {
        let dir = temp_dir("invalidate-cache");
        let mut store = QueueHistoryStore::new(dir.clone());

        let mut queue = AgentQueue::new();
        let e1 = make_entry(&mut queue, "Task 1", 0.10);
        store.append(&e1).unwrap();

        let s1 = store.summary().unwrap();
        assert_eq!(s1.entry_count, 1);

        // Write a second entry directly to disk.
        let e2 = make_entry(&mut queue, "Task 2", 0.20);
        store.save(&[e1, e2]).unwrap();

        // Invalidate and re-read.
        store.invalidate_cache();
        let s2 = store.summary().unwrap();
        assert_eq!(s2.entry_count, 2);
        assert!((s2.alltime_cost - 0.30).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_by_count() {
        let mut entries: Vec<QueueEntry> = Vec::new();
        let mut queue = AgentQueue::new();

        // Create MAX_ENTRIES + 50 entries.
        for i in 0..(MAX_ENTRIES + 50) {
            entries.push(make_entry(&mut queue, &format!("Task {i}"), 0.01));
        }

        QueueHistoryStore::prune_entries(&mut entries);
        assert_eq!(entries.len(), MAX_ENTRIES);

        // The oldest 50 should have been dropped — first remaining is "Task 50".
        assert_eq!(entries[0].content, "Task 50");
    }

    #[test]
    fn test_prune_by_age() {
        let mut queue = AgentQueue::new();
        let mut entries = Vec::new();

        // Create an entry with a very old completed_at.
        let mut old_entry = make_entry(&mut queue, "Old task", 0.50);
        old_entry.completed_at = Some(Utc::now() - Duration::days(MAX_AGE_DAYS + 1));
        entries.push(old_entry);

        // Create a recent entry.
        let recent_entry = make_entry(&mut queue, "Recent task", 0.25);
        entries.push(recent_entry);

        QueueHistoryStore::prune_entries(&mut entries);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Recent task");
    }

    #[test]
    fn test_summary_empty_store() {
        let dir = temp_dir("summary-empty");
        let mut store = QueueHistoryStore::new(dir.clone());

        let summary = store.summary().unwrap();
        assert_eq!(summary.entry_count, 0);
        assert!((summary.alltime_cost).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_append_updates_cache_atomically() {
        let dir = temp_dir("append-cache");
        let mut store = QueueHistoryStore::new(dir.clone());

        let mut queue = AgentQueue::new();
        let e1 = make_entry(&mut queue, "Task 1", 0.10);
        store.append(&e1).unwrap();

        // Cache should be populated after append — no disk read needed.
        let s1 = store.summary().unwrap();
        assert_eq!(s1.entry_count, 1);
        assert!((s1.alltime_cost - 0.10).abs() < f64::EPSILON);

        let e2 = make_entry(&mut queue, "Task 2", 0.30);
        store.append(&e2).unwrap();

        let s2 = store.summary().unwrap();
        assert_eq!(s2.entry_count, 2);
        assert!((s2.alltime_cost - 0.40).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
