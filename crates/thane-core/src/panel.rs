use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a panel (terminal, browser, or other content).
pub type PanelId = Uuid;

/// The type of content a panel displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelType {
    Terminal,
    Browser,
}

/// Metadata about a panel, independent of its UI representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelInfo {
    pub id: PanelId,
    pub panel_type: PanelType,
    /// Human-readable title (e.g. shell program name, page title).
    pub title: String,
    /// Working directory for terminal panels, URL for browser panels.
    pub location: String,
    /// Whether this panel has unread notifications.
    pub has_unread: bool,
}

impl PanelInfo {
    pub fn new_terminal(title: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            panel_type: PanelType::Terminal,
            title: title.into(),
            location: cwd.into(),
            has_unread: false,
        }
    }

    pub fn new_browser(title: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            panel_type: PanelType::Browser,
            title: title.into(),
            location: url.into(),
            has_unread: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_terminal_panel() {
        let panel = PanelInfo::new_terminal("zsh", "/home/user");
        assert_eq!(panel.panel_type, PanelType::Terminal);
        assert_eq!(panel.title, "zsh");
        assert_eq!(panel.location, "/home/user");
        assert!(!panel.has_unread);
        assert!(!panel.id.is_nil());
    }

    #[test]
    fn test_new_browser_panel() {
        let panel = PanelInfo::new_browser("Docs", "https://example.com");
        assert_eq!(panel.panel_type, PanelType::Browser);
        assert_eq!(panel.title, "Docs");
        assert_eq!(panel.location, "https://example.com");
        assert!(!panel.has_unread);
        assert!(!panel.id.is_nil());
    }

    #[test]
    fn test_panel_ids_are_unique() {
        let p1 = PanelInfo::new_terminal("a", "/tmp");
        let p2 = PanelInfo::new_terminal("b", "/tmp");
        assert_ne!(p1.id, p2.id);
    }
}
