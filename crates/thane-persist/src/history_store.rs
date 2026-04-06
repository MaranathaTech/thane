use std::path::PathBuf;

use thane_core::session::WorkspaceHistory;

use crate::store::StoreError;

/// Atomic file-based storage for workspace close history.
///
/// Uses the same temp + rename pattern as `SessionStore`.
pub struct HistoryStore {
    dir: PathBuf,
}

impl HistoryStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Save the workspace history atomically.
    pub fn save(&self, history: &WorkspaceHistory) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;

        let json = serde_json::to_string_pretty(history)?;
        let final_path = self.history_file();
        let temp_path = self
            .dir
            .join(format!("history.json.{}.tmp", std::process::id()));

        std::fs::write(&temp_path, &json)?;
        std::fs::rename(&temp_path, &final_path)?;

        tracing::debug!("History saved to {}", final_path.display());
        Ok(())
    }

    /// Load the workspace history. Returns an empty history if the file doesn't exist.
    pub fn load(&self) -> Result<WorkspaceHistory, StoreError> {
        let path = self.history_file();
        if !path.exists() {
            return Ok(WorkspaceHistory::new());
        }

        let json = std::fs::read_to_string(&path)?;
        let history: WorkspaceHistory = serde_json::from_str(&json)?;

        tracing::debug!("History loaded from {}", path.display());
        Ok(history)
    }

    fn history_file(&self) -> PathBuf {
        self.dir.join("history.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::session::ClosedWorkspaceRecord;
    use thane_core::workspace::Workspace;

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("thane-test-history-store");
        let _ = std::fs::remove_dir_all(&dir);

        let store = HistoryStore::new(dir.clone());

        let mut history = WorkspaceHistory::new();
        let ws = Workspace::new("Closed WS", "/tmp/closed");
        history.push(ClosedWorkspaceRecord::from_workspace(&ws));

        store.save(&history).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].title, "Closed WS");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let dir = std::env::temp_dir().join("thane-test-history-noexist");
        let _ = std::fs::remove_dir_all(&dir);

        let store = HistoryStore::new(dir);
        let history = store.load().unwrap();
        assert!(history.entries.is_empty());
    }
}
