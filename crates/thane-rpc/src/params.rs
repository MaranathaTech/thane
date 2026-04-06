use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Workspace params ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCreateParams {
    pub title: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSelectParams {
    #[serde(default)]
    pub id: Option<Uuid>,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCloseParams {
    pub id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRenameParams {
    pub id: Option<Uuid>,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGetInfoParams {
    pub id: Option<Uuid>,
}

// ─── Workspace results ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: Uuid,
    pub title: String,
    pub cwd: String,
    pub tag: Option<String>,
    pub pane_count: usize,
    pub panel_count: usize,
    pub unread_notifications: usize,
    pub git_branch: Option<String>,
    #[serde(default)]
    pub last_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceListResult {
    pub workspaces: Vec<WorkspaceInfo>,
    pub active_index: usize,
}

// ─── Surface params ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceFocusDirectionParams {
    pub direction: String, // "up", "down", "left", "right"
}

// ─── Notification params ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationListParams {
    pub workspace_id: Option<Uuid>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSendParams {
    pub workspace_id: Option<Uuid>,
    pub title: String,
    pub body: String,
}

// ─── Sidebar params ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarSetStatusParams {
    pub workspace_id: Option<Uuid>,
    pub label: String,
    pub value: String,
    pub style: Option<String>,
}

// ─── Browser params ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserOpenParams {
    pub url: String,
    pub workspace_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserNavigateParams {
    pub panel_id: Uuid,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserEvalJsParams {
    pub panel_id: Uuid,
    pub script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserClickElementParams {
    pub panel_id: Uuid,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTypeTextParams {
    pub panel_id: Uuid,
    pub selector: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserScreenshotParams {
    /// Panel ID of the browser to screenshot. If omitted, uses focused browser panel.
    pub panel_id: Option<Uuid>,
    /// Region to capture: "visible" (default) or "full_document".
    #[serde(default = "default_region")]
    pub region: String,
    /// If true, capture the full document height (alias for region="full_document").
    #[serde(default)]
    pub full_page: bool,
}

fn default_region() -> String {
    "visible".to_string()
}

// ─── Terminal params ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalScreenshotParams {
    /// Panel ID of the terminal to screenshot. If omitted, uses focused terminal panel.
    pub panel_id: Option<Uuid>,
}

// ─── Agent queue params ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueueSubmitParams {
    /// The task content (markdown or structured).
    pub task: String,
    /// Optional workspace to execute in.
    pub workspace_id: Option<Uuid>,
    /// Optional priority (higher = sooner).
    pub priority: Option<i32>,
    /// Optional dependency: task ID that must complete successfully before this task runs.
    pub depends_on: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueueStatusParams {
    pub entry_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueueCancelParams {
    pub entry_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueueEntry {
    pub id: Uuid,
    pub task: String,
    pub status: AgentQueueStatus,
    pub workspace_id: Option<Uuid>,
    pub priority: i32,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
    pub depends_on: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentQueueStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

// ─── System results ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    pub git_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_create_params_roundtrip() {
        let params = WorkspaceCreateParams {
            title: Some("My Workspace".into()),
            cwd: Some("/home/user/project".into()),
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: WorkspaceCreateParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title.unwrap(), "My Workspace");
        assert_eq!(parsed.cwd.unwrap(), "/home/user/project");
    }

    #[test]
    fn test_workspace_select_params_defaults() {
        let id = Uuid::new_v4();
        let params = WorkspaceSelectParams {
            id: Some(id),
            index: Some(2),
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: WorkspaceSelectParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id.unwrap(), id);
        assert_eq!(parsed.index.unwrap(), 2);

        // id and index default to None when missing.
        let parsed: WorkspaceSelectParams = serde_json::from_str("{}").unwrap();
        assert!(parsed.id.is_none());
        assert!(parsed.index.is_none());
    }

    #[test]
    fn test_workspace_close_and_rename_params() {
        let id = Uuid::new_v4();
        let close = WorkspaceCloseParams { id: Some(id) };
        let json = serde_json::to_string(&close).unwrap();
        let parsed: WorkspaceCloseParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id.unwrap(), id);

        let rename = WorkspaceRenameParams {
            id: Some(id),
            title: "New Title".into(),
        };
        let json = serde_json::to_string(&rename).unwrap();
        let parsed: WorkspaceRenameParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "New Title");
    }

    #[test]
    fn test_surface_and_notification_params() {
        let params = SurfaceFocusDirectionParams {
            direction: "left".into(),
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: SurfaceFocusDirectionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.direction, "left");

        let id = Uuid::new_v4();
        let list = NotificationListParams {
            workspace_id: Some(id),
            limit: Some(10),
        };
        let json = serde_json::to_string(&list).unwrap();
        let parsed: NotificationListParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.workspace_id.unwrap(), id);
        assert_eq!(parsed.limit.unwrap(), 10);

        let send = NotificationSendParams {
            workspace_id: None,
            title: "Alert".into(),
            body: "Something happened".into(),
        };
        let json = serde_json::to_string(&send).unwrap();
        let parsed: NotificationSendParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "Alert");
        assert!(parsed.workspace_id.is_none());
    }

    #[test]
    fn test_browser_params_roundtrip() {
        let panel_id = Uuid::new_v4();

        let open = BrowserOpenParams {
            url: "https://example.com".into(),
            workspace_id: None,
        };
        let json = serde_json::to_string(&open).unwrap();
        let parsed: BrowserOpenParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.url, "https://example.com");

        let nav = BrowserNavigateParams {
            panel_id,
            url: "https://rust-lang.org".into(),
        };
        let json = serde_json::to_string(&nav).unwrap();
        let parsed: BrowserNavigateParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.panel_id, panel_id);

        let eval = BrowserEvalJsParams {
            panel_id,
            script: "document.title".into(),
        };
        let json = serde_json::to_string(&eval).unwrap();
        let parsed: BrowserEvalJsParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.script, "document.title");

        let click = BrowserClickElementParams {
            panel_id,
            selector: "#submit".into(),
        };
        let json = serde_json::to_string(&click).unwrap();
        let parsed: BrowserClickElementParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.selector, "#submit");

        let type_text = BrowserTypeTextParams {
            panel_id,
            selector: "input[name=q]".into(),
            text: "hello world".into(),
        };
        let json = serde_json::to_string(&type_text).unwrap();
        let parsed: BrowserTypeTextParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, "hello world");
    }

    #[test]
    fn test_browser_screenshot_params_roundtrip() {
        let panel_id = Uuid::new_v4();

        // With all fields
        let params = BrowserScreenshotParams {
            panel_id: Some(panel_id),
            region: "full_document".into(),
            full_page: true,
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: BrowserScreenshotParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.panel_id.unwrap(), panel_id);
        assert_eq!(parsed.region, "full_document");
        assert!(parsed.full_page);

        // With defaults (empty object)
        let parsed: BrowserScreenshotParams = serde_json::from_str("{}").unwrap();
        assert!(parsed.panel_id.is_none());
        assert_eq!(parsed.region, "visible");
        assert!(!parsed.full_page);
    }

    #[test]
    fn test_terminal_screenshot_params_roundtrip() {
        let panel_id = Uuid::new_v4();

        // With panel_id
        let params = TerminalScreenshotParams {
            panel_id: Some(panel_id),
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: TerminalScreenshotParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.panel_id.unwrap(), panel_id);

        // Without panel_id (empty object)
        let parsed: TerminalScreenshotParams = serde_json::from_str("{}").unwrap();
        assert!(parsed.panel_id.is_none());
    }

    #[test]
    fn test_agent_queue_params_roundtrip() {
        let ws_id = Uuid::new_v4();
        let dep_id = Uuid::new_v4();
        let submit = AgentQueueSubmitParams {
            task: "Run tests".into(),
            workspace_id: Some(ws_id),
            priority: Some(5),
            depends_on: Some(dep_id),
        };
        let json = serde_json::to_string(&submit).unwrap();
        let parsed: AgentQueueSubmitParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.task, "Run tests");
        assert_eq!(parsed.priority.unwrap(), 5);

        let entry_id = Uuid::new_v4();
        let status = AgentQueueStatusParams { entry_id };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: AgentQueueStatusParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entry_id, entry_id);

        let cancel = AgentQueueCancelParams { entry_id };
        let json = serde_json::to_string(&cancel).unwrap();
        let parsed: AgentQueueCancelParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entry_id, entry_id);
    }

    #[test]
    fn test_agent_queue_entry_and_status_variants() {
        let entry = AgentQueueEntry {
            id: Uuid::new_v4(),
            task: "Deploy".into(),
            status: AgentQueueStatus::Running,
            workspace_id: None,
            priority: 10,
            created_at: "2025-01-01T00:00:00Z".into(),
            started_at: Some("2025-01-01T00:01:00Z".into()),
            completed_at: None,
            error: None,
            depends_on: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: AgentQueueEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, AgentQueueStatus::Running);
        assert_eq!(parsed.priority, 10);

        for status in [
            AgentQueueStatus::Queued,
            AgentQueueStatus::Running,
            AgentQueueStatus::Paused,
            AgentQueueStatus::Completed,
            AgentQueueStatus::Failed,
            AgentQueueStatus::Cancelled,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: AgentQueueStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_workspace_info_and_list_result() {
        let info = WorkspaceInfo {
            id: Uuid::new_v4(),
            title: "My Project".into(),
            cwd: "/home/user/proj".into(),
            tag: Some("rust".into()),
            pane_count: 2,
            panel_count: 3,
            unread_notifications: 1,
            git_branch: Some("main".into()),
            last_prompt: Some("Fix the build errors".into()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: WorkspaceInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "My Project");
        assert_eq!(parsed.git_branch.unwrap(), "main");

        let result = WorkspaceListResult {
            workspaces: vec![info],
            active_index: 0,
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: WorkspaceListResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.workspaces.len(), 1);
        assert_eq!(parsed.active_index, 0);
    }

    #[test]
    fn test_version_info_and_sidebar_params() {
        let info = VersionInfo {
            version: "0.1.0".into(),
            git_hash: Some("abc123".into()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: VersionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, "0.1.0");
        assert_eq!(parsed.git_hash.unwrap(), "abc123");

        let params = SidebarSetStatusParams {
            workspace_id: None,
            label: "Status".into(),
            value: "Active".into(),
            style: Some("success".into()),
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: SidebarSetStatusParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.label, "Status");
        assert_eq!(parsed.style.unwrap(), "success");
    }
}
