use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::pane::SplitTree;
use crate::panel::PanelInfo;
use crate::sandbox::SandboxPolicy;
use crate::workspace::Workspace;

/// Serializable snapshot of the entire application state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub active_workspace_id: Option<Uuid>,
    /// Whether the sidebar was collapsed when the session was saved.
    #[serde(default)]
    pub sidebar_collapsed: Option<bool>,
}

impl AppSnapshot {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new(workspaces: Vec<WorkspaceSnapshot>, active_workspace_id: Option<Uuid>) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            timestamp: Utc::now(),
            workspaces,
            active_workspace_id,
            sidebar_collapsed: None,
        }
    }
}

/// Serializable snapshot of a single workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: Uuid,
    pub title: String,
    pub cwd: String,
    pub split_tree: SplitTree,
    pub panels: Vec<PanelSnapshot>,
    pub focused_pane_id: Option<Uuid>,
    pub tag: Option<String>,
    /// Sandbox policy (optional for backwards compat with old snapshots).
    #[serde(default)]
    pub sandbox_policy: SandboxPolicy,
}

/// Serializable snapshot of a panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelSnapshot {
    pub info: PanelInfo,
    /// For terminals: truncated scrollback content (ANSI-safe).
    pub scrollback: Option<String>,
    /// For browsers: current URL.
    pub url: Option<String>,
}

impl PanelSnapshot {
    pub fn from_terminal(info: PanelInfo, scrollback: Option<String>) -> Self {
        Self {
            info,
            scrollback,
            url: None,
        }
    }

    pub fn from_browser(info: PanelInfo, url: String) -> Self {
        Self {
            info,
            scrollback: None,
            url: Some(url),
        }
    }
}

/// Lightweight record of a closed workspace, for the "recently closed" history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedWorkspaceRecord {
    pub original_id: Uuid,
    pub title: String,
    pub cwd: String,
    pub tag: Option<String>,
    pub closed_at: DateTime<Utc>,
}

impl ClosedWorkspaceRecord {
    /// Create a record from a workspace that is about to be closed.
    pub fn from_workspace(ws: &Workspace) -> Self {
        Self {
            original_id: ws.id,
            title: ws.title.clone(),
            cwd: ws.cwd.clone(),
            tag: ws.tag.clone(),
            closed_at: Utc::now(),
        }
    }
}

/// Recently-closed workspace history, stored as an ordered list (most recent first).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHistory {
    pub version: u32,
    pub entries: Vec<ClosedWorkspaceRecord>,
}

impl Default for WorkspaceHistory {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

impl WorkspaceHistory {
    const MAX_ENTRIES: usize = 20;

    pub fn new() -> Self {
        Self::default()
    }

    /// Add a closed workspace record. Deduplicates by `original_id`,
    /// inserts at front, and truncates to 20 entries.
    pub fn push(&mut self, record: ClosedWorkspaceRecord) {
        self.entries.retain(|r| r.original_id != record.original_id);
        self.entries.insert(0, record);
        self.entries.truncate(Self::MAX_ENTRIES);
    }

    /// Remove and return a record by original workspace ID (used on reopen).
    pub fn remove(&mut self, original_id: Uuid) -> Option<ClosedWorkspaceRecord> {
        if let Some(pos) = self.entries.iter().position(|r| r.original_id == original_id) {
            Some(self.entries.remove(pos))
        } else {
            None
        }
    }

    /// Clear all history entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// List all history entries (most recent first).
    pub fn list(&self) -> &[ClosedWorkspaceRecord] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_snapshot_new() {
        let snap = AppSnapshot::new(vec![], None);
        assert_eq!(snap.version, AppSnapshot::CURRENT_VERSION);
        assert!(snap.workspaces.is_empty());
        assert!(snap.active_workspace_id.is_none());
        // Timestamp should be recent (within last second).
        let elapsed = Utc::now() - snap.timestamp;
        assert!(elapsed.num_seconds() < 2);
    }

    #[test]
    fn test_app_snapshot_serde_roundtrip() {
        let panel_id = Uuid::new_v4();
        let ws_id = Uuid::new_v4();
        let ws = WorkspaceSnapshot {
            id: ws_id,
            title: "Test WS".into(),
            cwd: "/tmp/test".into(),
            split_tree: SplitTree::new_leaf(panel_id),
            panels: vec![PanelSnapshot::from_terminal(
                PanelInfo::new_terminal("bash", "/tmp"),
                Some("$ hello\n".into()),
            )],
            focused_pane_id: None,
            tag: Some("rust".into()),
            sandbox_policy: SandboxPolicy::default(),
        };
        let snap = AppSnapshot::new(vec![ws], Some(ws_id));
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: AppSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.workspaces.len(), 1);
        assert_eq!(parsed.workspaces[0].title, "Test WS");
        assert_eq!(parsed.active_workspace_id.unwrap(), ws_id);
    }

    #[test]
    fn test_workspace_snapshot_serde_with_sandbox_default() {
        let panel_id = Uuid::new_v4();
        let ws = WorkspaceSnapshot {
            id: Uuid::new_v4(),
            title: "WS".into(),
            cwd: "/home".into(),
            split_tree: SplitTree::new_leaf(panel_id),
            panels: vec![],
            focused_pane_id: None,
            tag: None,
            sandbox_policy: SandboxPolicy::default(),
        };
        let json = serde_json::to_string(&ws).unwrap();
        let parsed: WorkspaceSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sandbox_policy.enabled, ws.sandbox_policy.enabled);
    }

    #[test]
    fn test_workspace_snapshot_backwards_compat_no_sandbox() {
        // Old snapshots won't have the sandbox_policy field — serde(default) handles this.
        let panel_id = Uuid::new_v4();
        let tree = SplitTree::new_leaf(panel_id);
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "title": "Old WS",
            "cwd": "/old",
            "split_tree": serde_json::to_value(&tree).unwrap(),
            "panels": [],
            "focused_pane_id": null,
            "tag": null
        });
        let parsed: WorkspaceSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.title, "Old WS");
        // sandbox_policy should be Default.
        assert!(!parsed.sandbox_policy.enabled);
    }

    #[test]
    fn test_panel_snapshot_from_terminal() {
        let info = PanelInfo::new_terminal("zsh", "/home/user");
        let snap = PanelSnapshot::from_terminal(info, Some("$ ls\nfile.txt\n".into()));
        assert_eq!(snap.scrollback.unwrap(), "$ ls\nfile.txt\n");
        assert!(snap.url.is_none());
    }

    #[test]
    fn test_panel_snapshot_from_browser() {
        let info = PanelInfo::new_browser("Example", "https://example.com");
        let snap = PanelSnapshot::from_browser(info, "https://example.com".into());
        assert!(snap.scrollback.is_none());
        assert_eq!(snap.url.unwrap(), "https://example.com");
    }

    #[test]
    fn test_closed_workspace_record_from_workspace() {
        let ws = Workspace::new("My Project", "/home/user/project");
        let record = ClosedWorkspaceRecord::from_workspace(&ws);
        assert_eq!(record.original_id, ws.id);
        assert_eq!(record.title, "My Project");
        assert_eq!(record.cwd, "/home/user/project");
        assert!(record.tag.is_none());
    }

    #[test]
    fn test_workspace_history_push_dedup() {
        let mut history = WorkspaceHistory::new();
        let ws = Workspace::new("Test", "/tmp");
        let record = ClosedWorkspaceRecord::from_workspace(&ws);
        let id = record.original_id;

        history.push(record.clone());
        history.push(record.clone());

        assert_eq!(history.list().len(), 1);
        assert_eq!(history.list()[0].original_id, id);
    }

    #[test]
    fn test_workspace_history_push_ordering() {
        let mut history = WorkspaceHistory::new();
        let ws1 = Workspace::new("WS1", "/tmp/1");
        let ws2 = Workspace::new("WS2", "/tmp/2");
        history.push(ClosedWorkspaceRecord::from_workspace(&ws1));
        history.push(ClosedWorkspaceRecord::from_workspace(&ws2));

        assert_eq!(history.list()[0].title, "WS2");
        assert_eq!(history.list()[1].title, "WS1");
    }

    #[test]
    fn test_workspace_history_truncates_at_20() {
        let mut history = WorkspaceHistory::new();
        for i in 0..25 {
            let ws = Workspace::new(format!("WS{i}"), "/tmp");
            history.push(ClosedWorkspaceRecord::from_workspace(&ws));
        }
        assert_eq!(history.list().len(), 20);
        assert_eq!(history.list()[0].title, "WS24");
    }

    #[test]
    fn test_workspace_history_remove() {
        let mut history = WorkspaceHistory::new();
        let ws = Workspace::new("Test", "/tmp");
        let id = ws.id;
        history.push(ClosedWorkspaceRecord::from_workspace(&ws));

        let removed = history.remove(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().original_id, id);
        assert!(history.list().is_empty());
    }

    #[test]
    fn test_workspace_history_remove_nonexistent() {
        let mut history = WorkspaceHistory::new();
        assert!(history.remove(Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_workspace_history_clear() {
        let mut history = WorkspaceHistory::new();
        let ws = Workspace::new("Test", "/tmp");
        history.push(ClosedWorkspaceRecord::from_workspace(&ws));
        history.clear();
        assert!(history.list().is_empty());
    }

    #[test]
    fn test_app_snapshot_sidebar_collapsed_serde_roundtrip() {
        let mut snap = AppSnapshot::new(vec![], None);
        snap.sidebar_collapsed = Some(true);
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: AppSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sidebar_collapsed, Some(true));
    }

    #[test]
    fn test_app_snapshot_sidebar_collapsed_backwards_compat() {
        // Old snapshots won't have sidebar_collapsed — serde(default) gives None.
        let json = serde_json::json!({
            "version": 1,
            "timestamp": "2025-01-01T00:00:00Z",
            "workspaces": [],
            "active_workspace_id": null
        });
        let parsed: AppSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.sidebar_collapsed, None);
    }

    #[test]
    fn test_workspace_history_serde_roundtrip() {
        let mut history = WorkspaceHistory::new();
        let ws = Workspace::new("Serde Test", "/home/serde");
        history.push(ClosedWorkspaceRecord::from_workspace(&ws));

        let json = serde_json::to_string(&history).unwrap();
        let parsed: WorkspaceHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].title, "Serde Test");
    }
}
