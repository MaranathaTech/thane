use std::path::PathBuf;

use thane_core::session::AppSnapshot;
use thiserror::Error;
use tracing;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Atomic file-based session storage.
///
/// Writes use a temp file + rename pattern to prevent corruption.
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Save a snapshot atomically.
    pub fn save(&self, snapshot: &AppSnapshot) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;

        let json = serde_json::to_string_pretty(snapshot)?;
        let final_path = self.session_file();
        let temp_path = self
            .dir
            .join(format!("session.json.{}.tmp", std::process::id()));

        // Write to temp file first
        std::fs::write(&temp_path, &json)?;

        // Atomic rename
        std::fs::rename(&temp_path, &final_path)?;

        tracing::debug!("Session saved to {}", final_path.display());
        Ok(())
    }

    /// Load the most recent snapshot.
    pub fn load(&self) -> Result<Option<AppSnapshot>, StoreError> {
        let path = self.session_file();
        if !path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&path)?;
        let snapshot: AppSnapshot = serde_json::from_str(&json)?;

        tracing::debug!("Session loaded from {}", path.display());
        Ok(Some(snapshot))
    }

    /// Delete the saved session.
    pub fn clear(&self) -> Result<(), StoreError> {
        let path = self.session_file();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    fn session_file(&self) -> PathBuf {
        self.dir.join("session.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use thane_core::pane::SplitTree;
    use thane_core::panel::PanelInfo;
    use thane_core::sandbox::{EnforcementLevel, SandboxPolicy};
    use thane_core::session::{AppSnapshot, PanelSnapshot, WorkspaceSnapshot};
    use uuid::Uuid;

    static STORE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_store() -> (SessionStore, std::path::PathBuf) {
        let id = STORE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "thane-test-store-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (SessionStore::new(dir.clone()), dir)
    }

    fn make_workspace(title: &str, cwd: &str, panel_count: usize) -> WorkspaceSnapshot {
        let mut panels = Vec::new();
        let mut panel_ids = Vec::new();
        for i in 0..panel_count {
            let info = if i % 2 == 0 {
                PanelInfo::new_terminal("bash", cwd)
            } else {
                PanelInfo::new_browser("Tab", &format!("https://example.com/{i}"))
            };
            panel_ids.push(info.id);
            let snap = if i % 2 == 0 {
                PanelSnapshot::from_terminal(info, Some(format!("$ command {i}\n")))
            } else {
                PanelSnapshot::from_browser(info, format!("https://example.com/{i}"))
            };
            panels.push(snap);
        }

        let first_panel = panel_ids[0];
        let split_tree = SplitTree::new_leaf(first_panel);

        WorkspaceSnapshot {
            id: Uuid::new_v4(),
            title: title.to_string(),
            cwd: cwd.to_string(),
            split_tree,
            panels,
            focused_pane_id: None,
            tag: Some("rust".to_string()),
            sandbox_policy: SandboxPolicy::default(),
        }
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let (store, dir) = test_store();
        let snapshot = AppSnapshot::new(vec![], None);
        store.save(&snapshot).unwrap();

        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.version, AppSnapshot::CURRENT_VERSION);
        assert!(loaded.workspaces.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = std::env::temp_dir().join(format!(
            "thane-test-store-noexist-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SessionStore::new(dir);
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn test_roundtrip_multi_workspace_complex() {
        let (store, dir) = test_store();
        let ws1 = make_workspace("Project Alpha", "/home/user/alpha", 3);
        let ws2 = make_workspace("Project Beta", "/home/user/beta", 5);
        let ws3 = make_workspace("Scratch", "/tmp/scratch", 1);
        let active_id = ws2.id;

        let snapshot = AppSnapshot::new(vec![ws1, ws2, ws3], Some(active_id));
        store.save(&snapshot).unwrap();

        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.workspaces.len(), 3);
        assert_eq!(loaded.active_workspace_id, Some(active_id));
        assert_eq!(loaded.workspaces[0].title, "Project Alpha");
        assert_eq!(loaded.workspaces[0].panels.len(), 3);
        assert_eq!(loaded.workspaces[1].title, "Project Beta");
        assert_eq!(loaded.workspaces[1].panels.len(), 5);
        assert_eq!(loaded.workspaces[2].title, "Scratch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_roundtrip_preserves_panel_content() {
        let (store, dir) = test_store();
        let ws = make_workspace("Content", "/tmp", 2);
        let snapshot = AppSnapshot::new(vec![ws], None);
        store.save(&snapshot).unwrap();

        let loaded = store.load().unwrap().unwrap();
        let panels = &loaded.workspaces[0].panels;
        // First panel is terminal with scrollback.
        assert!(panels[0].scrollback.is_some());
        assert!(panels[0].scrollback.as_ref().unwrap().contains("command 0"));
        assert!(panels[0].url.is_none());
        // Second panel is browser with URL.
        assert!(panels[1].url.is_some());
        assert!(panels[1].url.as_ref().unwrap().contains("example.com"));
        assert!(panels[1].scrollback.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_roundtrip_preserves_sandbox_policy() {
        let (store, dir) = test_store();
        let mut ws = make_workspace("Sandboxed", "/home/user/project", 1);
        ws.sandbox_policy = SandboxPolicy {
            enabled: true,
            root_dir: std::path::PathBuf::from("/home/user/project"),
            read_only_paths: vec![std::path::PathBuf::from("/usr/lib")],
            read_write_paths: vec![std::path::PathBuf::from("/tmp")],
            denied_paths: vec![std::path::PathBuf::from("/etc/passwd")],
            allow_network: false,
            max_open_files: Some(1024),
            max_write_bytes: Some(1_000_000),
            cpu_time_limit: Some(300),
            enforcement: EnforcementLevel::Strict,
        };

        let snapshot = AppSnapshot::new(vec![ws], None);
        store.save(&snapshot).unwrap();

        let loaded = store.load().unwrap().unwrap();
        let policy = &loaded.workspaces[0].sandbox_policy;
        assert!(policy.enabled);
        assert_eq!(policy.root_dir, std::path::PathBuf::from("/home/user/project"));
        assert_eq!(policy.read_only_paths, vec![std::path::PathBuf::from("/usr/lib")]);
        assert_eq!(policy.denied_paths, vec![std::path::PathBuf::from("/etc/passwd")]);
        assert!(!policy.allow_network);
        assert_eq!(policy.max_open_files, Some(1024));
        assert_eq!(policy.max_write_bytes, Some(1_000_000));
        assert_eq!(policy.cpu_time_limit, Some(300));
        assert_eq!(policy.enforcement, EnforcementLevel::Strict);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_roundtrip_preserves_tags() {
        let (store, dir) = test_store();
        let mut ws1 = make_workspace("Tagged", "/tmp", 1);
        ws1.tag = Some("important".to_string());
        let mut ws2 = make_workspace("Untagged", "/tmp", 1);
        ws2.tag = None;

        let snapshot = AppSnapshot::new(vec![ws1, ws2], None);
        store.save(&snapshot).unwrap();

        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.workspaces[0].tag, Some("important".to_string()));
        assert_eq!(loaded.workspaces[1].tag, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_backwards_compat_missing_sandbox_field() {
        let (store, dir) = test_store();
        // Manually write JSON without the sandbox_policy field.
        std::fs::create_dir_all(store.dir.clone()).unwrap();
        let panel_id = Uuid::new_v4();
        let tree = SplitTree::new_leaf(panel_id);
        let json = serde_json::json!({
            "version": 1,
            "timestamp": "2024-01-01T00:00:00Z",
            "workspaces": [{
                "id": Uuid::new_v4(),
                "title": "Old Format",
                "cwd": "/old",
                "split_tree": serde_json::to_value(&tree).unwrap(),
                "panels": [],
                "focused_pane_id": null,
                "tag": null
            }],
            "active_workspace_id": null
        });
        let path = store.dir.join("session.json");
        std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();

        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.workspaces[0].title, "Old Format");
        // sandbox_policy should get the Default value.
        assert!(!loaded.workspaces[0].sandbox_policy.enabled);
        assert_eq!(loaded.workspaces[0].sandbox_policy.enforcement, EnforcementLevel::Enforcing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_corrupt_json() {
        let (store, dir) = test_store();
        std::fs::create_dir_all(store.dir.clone()).unwrap();
        let path = store.dir.join("session.json");
        std::fs::write(&path, "{ invalid json !!!").unwrap();

        let result = store.load();
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_truncated_file() {
        let (store, dir) = test_store();
        std::fs::create_dir_all(store.dir.clone()).unwrap();
        let path = store.dir.join("session.json");
        std::fs::write(&path, "{\"version\":1,\"timestamp\":\"2024-01-01T00:00:00Z\",\"work").unwrap();

        let result = store.load();
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_creates_directory() {
        let (store, dir) = test_store();
        // Dir doesn't exist yet (test_store removes it).
        let snapshot = AppSnapshot::new(vec![], None);
        store.save(&snapshot).unwrap();
        assert!(store.dir.join("session.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_clear_removes_session() {
        let (store, dir) = test_store();
        let snapshot = AppSnapshot::new(vec![], None);
        store.save(&snapshot).unwrap();
        assert!(store.dir.join("session.json").exists());

        store.clear().unwrap();
        assert!(!store.dir.join("session.json").exists());
        assert!(store.load().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_overwrite_existing_session() {
        let (store, dir) = test_store();
        let ws1 = make_workspace("First", "/tmp/1", 1);
        store.save(&AppSnapshot::new(vec![ws1], None)).unwrap();

        let ws2 = make_workspace("Second", "/tmp/2", 2);
        store.save(&AppSnapshot::new(vec![ws2], None)).unwrap();

        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].title, "Second");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
