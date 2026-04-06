use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Per-panel location info (CWD + git status) for sidebar display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelLocationInfo {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub git_dirty: bool,
}

/// Metadata displayed in the sidebar for a workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SidebarMetadata {
    /// Short status entries (e.g. "running", "idle", "error").
    pub status_entries: Vec<StatusEntry>,
    /// Git branch name, if detected.
    pub git_branch: Option<String>,
    /// Whether the git working tree has uncommitted changes.
    pub git_dirty: bool,
    /// Listening ports detected in the workspace.
    pub ports: Vec<u16>,
    /// Agent activity status.
    pub agent_status: AgentStatus,
    /// Estimated token cost for the current session (USD).
    pub session_cost: Option<f64>,
    /// All-time project cost (USD), across all Claude Code sessions.
    #[serde(default)]
    pub all_time_cost: Option<f64>,
    /// Per-panel CWD and git info, keyed by panel UUID.
    #[serde(default)]
    pub panel_locations: HashMap<Uuid, PanelLocationInfo>,
    /// Last user prompt sent to an AI agent (e.g. Claude) in this workspace.
    #[serde(default)]
    pub last_prompt: Option<String>,
}

/// A single status entry in the sidebar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEntry {
    pub label: String,
    pub value: String,
    pub style: StatusStyle,
}

/// Visual style for a status entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusStyle {
    Normal,
    Success,
    Warning,
    Error,
    Muted,
}

/// Agent activity status for a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AgentStatus {
    /// No agent detected.
    #[default]
    Inactive,
    /// Agent is actively producing output.
    Active,
    /// Agent hasn't produced output for a while (possible stuck).
    Stalled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_location_info_serde() {
        let info = PanelLocationInfo {
            cwd: "/home/user/project".to_string(),
            git_branch: Some("main".to_string()),
            git_dirty: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PanelLocationInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.cwd, "/home/user/project");
        assert_eq!(deserialized.git_branch.as_deref(), Some("main"));
        assert!(deserialized.git_dirty);
    }

    #[test]
    fn test_panel_location_info_no_git() {
        let info = PanelLocationInfo {
            cwd: "/tmp".to_string(),
            git_branch: None,
            git_dirty: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PanelLocationInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.cwd, "/tmp");
        assert!(deserialized.git_branch.is_none());
        assert!(!deserialized.git_dirty);
    }

    #[test]
    fn test_sidebar_metadata_with_panel_locations() {
        let mut meta = SidebarMetadata::default();
        let panel_id = Uuid::new_v4();
        meta.panel_locations.insert(
            panel_id,
            PanelLocationInfo {
                cwd: "/home/user/project".to_string(),
                git_branch: Some("feature".to_string()),
                git_dirty: false,
            },
        );

        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: SidebarMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.panel_locations.len(), 1);
        let loc = deserialized.panel_locations.get(&panel_id).unwrap();
        assert_eq!(loc.cwd, "/home/user/project");
        assert_eq!(loc.git_branch.as_deref(), Some("feature"));
    }

    #[test]
    fn test_sidebar_metadata_backward_compat_no_panel_locations() {
        // Simulate old JSON without panel_locations field.
        let json = r#"{
            "status_entries": [],
            "git_branch": "main",
            "git_dirty": false,
            "ports": [],
            "agent_status": "inactive",
            "session_cost": null
        }"#;
        let meta: SidebarMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.panel_locations.is_empty());
        assert_eq!(meta.git_branch.as_deref(), Some("main"));
    }
}

