use serde::{Deserialize, Serialize};

/// All RPC methods supported by thane.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    // System
    Ping,
    GetVersion,
    GetConfig,

    // Workspace management
    WorkspaceList,
    WorkspaceCreate,
    WorkspaceSelect,
    WorkspaceClose,
    WorkspaceRename,
    WorkspaceGetInfo,

    // Surface / pane management
    SurfaceSplitRight,
    SurfaceSplitDown,
    SurfaceClose,
    SurfaceFocusNext,
    SurfaceFocusPrev,
    SurfaceFocusDirection,
    SurfaceZoomToggle,

    // Notifications
    NotificationList,
    NotificationMarkRead,
    NotificationClear,
    NotificationSend,

    // Sidebar
    SidebarSetStatus,
    SidebarGetMetadata,

    // Browser
    BrowserOpen,
    BrowserNavigate,
    BrowserEvalJs,
    BrowserGetAccessibilityTree,
    BrowserClickElement,
    BrowserTypeText,
    BrowserScreenshot,

    // Terminal
    TerminalScreenshot,

    // Sandbox
    SandboxStatus,
    SandboxEnable,
    SandboxDisable,
    SandboxAllow,
    SandboxDeny,

    // Agent queue (headless agent support)
    AgentQueueSubmit,
    AgentQueueList,
    AgentQueueStatus,
    AgentQueueCancel,

    // Workspace history (recently closed)
    WorkspaceHistoryList,
    WorkspaceHistoryReopen,
    WorkspaceHistoryClear,

    // Audit trail
    AuditList,
    AuditExport,
    AuditClear,
    AuditSetSensitivePolicy,
}

impl Method {
    /// Parse a method name string into a Method enum.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ping" => Some(Self::Ping),
            "get_version" => Some(Self::GetVersion),
            "get_config" => Some(Self::GetConfig),

            "workspace.list" => Some(Self::WorkspaceList),
            "workspace.create" => Some(Self::WorkspaceCreate),
            "workspace.select" => Some(Self::WorkspaceSelect),
            "workspace.close" => Some(Self::WorkspaceClose),
            "workspace.rename" => Some(Self::WorkspaceRename),
            "workspace.get_info" => Some(Self::WorkspaceGetInfo),

            "surface.split_right" => Some(Self::SurfaceSplitRight),
            "surface.split_down" => Some(Self::SurfaceSplitDown),
            "surface.close" => Some(Self::SurfaceClose),
            "surface.focus_next" => Some(Self::SurfaceFocusNext),
            "surface.focus_prev" => Some(Self::SurfaceFocusPrev),
            "surface.focus_direction" => Some(Self::SurfaceFocusDirection),
            "surface.zoom_toggle" => Some(Self::SurfaceZoomToggle),

            "notification.list" => Some(Self::NotificationList),
            "notification.mark_read" => Some(Self::NotificationMarkRead),
            "notification.clear" => Some(Self::NotificationClear),
            "notification.send" => Some(Self::NotificationSend),

            "sidebar.set_status" => Some(Self::SidebarSetStatus),
            "sidebar.get_metadata" => Some(Self::SidebarGetMetadata),

            "browser.open" => Some(Self::BrowserOpen),
            "browser.navigate" => Some(Self::BrowserNavigate),
            "browser.eval_js" => Some(Self::BrowserEvalJs),
            "browser.get_accessibility_tree" => Some(Self::BrowserGetAccessibilityTree),
            "browser.click_element" => Some(Self::BrowserClickElement),
            "browser.type_text" => Some(Self::BrowserTypeText),
            "browser.screenshot" => Some(Self::BrowserScreenshot),

            "terminal.screenshot" => Some(Self::TerminalScreenshot),

            "sandbox.status" => Some(Self::SandboxStatus),
            "sandbox.enable" => Some(Self::SandboxEnable),
            "sandbox.disable" => Some(Self::SandboxDisable),
            "sandbox.allow" => Some(Self::SandboxAllow),
            "sandbox.deny" => Some(Self::SandboxDeny),

            "agent_queue.submit" => Some(Self::AgentQueueSubmit),
            "agent_queue.list" => Some(Self::AgentQueueList),
            "agent_queue.status" => Some(Self::AgentQueueStatus),
            "agent_queue.cancel" => Some(Self::AgentQueueCancel),

            "workspace.history_list" => Some(Self::WorkspaceHistoryList),
            "workspace.history_reopen" => Some(Self::WorkspaceHistoryReopen),
            "workspace.history_clear" => Some(Self::WorkspaceHistoryClear),

            "audit.list" => Some(Self::AuditList),
            "audit.export" => Some(Self::AuditExport),
            "audit.clear" => Some(Self::AuditClear),
            "audit.set_sensitive_policy" => Some(Self::AuditSetSensitivePolicy),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_parsing() {
        assert_eq!(Method::parse("ping"), Some(Method::Ping));
        assert_eq!(
            Method::parse("workspace.list"),
            Some(Method::WorkspaceList)
        );
        assert_eq!(
            Method::parse("browser.eval_js"),
            Some(Method::BrowserEvalJs)
        );
        assert_eq!(Method::parse("nonexistent"), None);
    }

    #[test]
    fn test_all_methods_parse_exhaustive() {
        let all: Vec<(&str, Method)> = vec![
            ("ping", Method::Ping),
            ("get_version", Method::GetVersion),
            ("get_config", Method::GetConfig),
            ("workspace.list", Method::WorkspaceList),
            ("workspace.create", Method::WorkspaceCreate),
            ("workspace.select", Method::WorkspaceSelect),
            ("workspace.close", Method::WorkspaceClose),
            ("workspace.rename", Method::WorkspaceRename),
            ("workspace.get_info", Method::WorkspaceGetInfo),
            ("surface.split_right", Method::SurfaceSplitRight),
            ("surface.split_down", Method::SurfaceSplitDown),
            ("surface.close", Method::SurfaceClose),
            ("surface.focus_next", Method::SurfaceFocusNext),
            ("surface.focus_prev", Method::SurfaceFocusPrev),
            ("surface.focus_direction", Method::SurfaceFocusDirection),
            ("surface.zoom_toggle", Method::SurfaceZoomToggle),
            ("notification.list", Method::NotificationList),
            ("notification.mark_read", Method::NotificationMarkRead),
            ("notification.clear", Method::NotificationClear),
            ("notification.send", Method::NotificationSend),
            ("sidebar.set_status", Method::SidebarSetStatus),
            ("sidebar.get_metadata", Method::SidebarGetMetadata),
            ("browser.open", Method::BrowserOpen),
            ("browser.navigate", Method::BrowserNavigate),
            ("browser.eval_js", Method::BrowserEvalJs),
            ("browser.get_accessibility_tree", Method::BrowserGetAccessibilityTree),
            ("browser.click_element", Method::BrowserClickElement),
            ("browser.type_text", Method::BrowserTypeText),
            ("browser.screenshot", Method::BrowserScreenshot),
            ("terminal.screenshot", Method::TerminalScreenshot),
            ("sandbox.status", Method::SandboxStatus),
            ("sandbox.enable", Method::SandboxEnable),
            ("sandbox.disable", Method::SandboxDisable),
            ("sandbox.allow", Method::SandboxAllow),
            ("sandbox.deny", Method::SandboxDeny),
            ("agent_queue.submit", Method::AgentQueueSubmit),
            ("agent_queue.list", Method::AgentQueueList),
            ("agent_queue.status", Method::AgentQueueStatus),
            ("agent_queue.cancel", Method::AgentQueueCancel),
            ("workspace.history_list", Method::WorkspaceHistoryList),
            ("workspace.history_reopen", Method::WorkspaceHistoryReopen),
            ("workspace.history_clear", Method::WorkspaceHistoryClear),
            ("audit.list", Method::AuditList),
            ("audit.export", Method::AuditExport),
            ("audit.clear", Method::AuditClear),
            ("audit.set_sensitive_policy", Method::AuditSetSensitivePolicy),
        ];

        for (name, expected) in &all {
            assert_eq!(
                Method::parse(name),
                Some(expected.clone()),
                "Method::parse({name:?}) should return {expected:?}"
            );
        }
        // Verify we tested every variant (46 total).
        assert_eq!(all.len(), 46);
    }

    #[test]
    fn test_serde_roundtrip_matches_parse() {
        // Verify that serde rename_all snake_case serialization is consistent.
        let method = Method::WorkspaceGetInfo;
        let json = serde_json::to_string(&method).unwrap();
        // serde produces "workspace_get_info" (snake_case of the variant name).
        assert_eq!(json, "\"workspace_get_info\"");

        let method = Method::AgentQueueSubmit;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"agent_queue_submit\"");

        // Roundtrip all variants.
        let all_methods = vec![
            Method::Ping,
            Method::GetVersion,
            Method::GetConfig,
            Method::WorkspaceList,
            Method::BrowserEvalJs,
            Method::SandboxStatus,
            Method::AgentQueueCancel,
            Method::BrowserScreenshot,
            Method::TerminalScreenshot,
            Method::AuditSetSensitivePolicy,
        ];
        for method in all_methods {
            let json = serde_json::to_string(&method).unwrap();
            let parsed: Method = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, method);
        }
    }

    #[test]
    fn test_invalid_method_names() {
        assert_eq!(Method::parse(""), None);
        assert_eq!(Method::parse("Ping"), None); // Case-sensitive
        assert_eq!(Method::parse("PING"), None);
        assert_eq!(Method::parse("workspace_list"), None); // Dot not underscore
        assert_eq!(Method::parse("workspace."), None);
        assert_eq!(Method::parse(".list"), None);
        assert_eq!(Method::parse("workspace.list.extra"), None);
        assert_eq!(Method::parse(" ping"), None); // Leading space
        assert_eq!(Method::parse("ping "), None); // Trailing space
    }
}
