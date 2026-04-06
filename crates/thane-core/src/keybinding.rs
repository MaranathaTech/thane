use serde::{Deserialize, Serialize};

/// Actions that can be triggered by keyboard shortcuts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    // Workspace management
    WorkspaceNew,
    WorkspaceClose,
    WorkspaceNext,
    WorkspacePrev,
    WorkspaceSelect(u8), // 1-9
    WorkspaceRename,

    // Split pane management
    SplitRight,
    SplitDown,
    PaneClose,
    PaneNext,
    PanePrev,
    PaneFocusUp,
    PaneFocusDown,
    PaneFocusLeft,
    PaneFocusRight,
    PaneZoomToggle,

    // Panel management
    PanelClose,
    NewTerminal,
    NewBrowser,
    PanelNext,
    PanelPrev,

    // Navigation
    ToggleSidebar,
    ToggleNotifications,
    FocusSidebar,
    FocusContent,

    // Terminal
    Copy,
    Paste,
    FindInTerminal,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,

    // Leader key mode
    EnterLeaderMode,

    // Sandbox
    ToggleSandbox,

    // Audit
    ToggleAuditPanel,

    // Git
    ToggleGitDiff,

    // Font zoom
    FontSizeIncrease,
    FontSizeDecrease,
    FontSizeReset,

    // Panels
    ToggleSettings,
    ToggleTokenUsage,
    ToggleHelp,
    ToggleAgentQueue,
    TogglePlans,

    // Misc
    ReloadConfig,
    ToggleFullscreen,
    Quit,
}

/// Modifier keys for keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        super_key: false,
    };

    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
        super_key: false,
    };

    pub const CTRL_SHIFT: Self = Self {
        ctrl: true,
        alt: false,
        shift: true,
        super_key: false,
    };

    pub const ALT: Self = Self {
        ctrl: false,
        alt: true,
        shift: false,
        super_key: false,
    };

    pub const SUPER: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        super_key: true,
    };
}

/// A keyboard shortcut binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Keybinding {
    pub modifiers: Modifiers,
    pub key: String,
    pub action: KeyAction,
}

/// Parse a keybind string like "ctrl+shift+t=workspace_new".
/// Returns None if the string is malformed.
pub fn parse_keybind(s: &str) -> Option<Keybinding> {
    let (combo, action_str) = s.split_once('=')?;
    let combo = combo.trim();
    let action_str = action_str.trim();

    let action = parse_action(action_str)?;

    let parts: Vec<&str> = combo.split('+').collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::NONE;
    let key = parts.last()?.to_string();

    for &part in &parts[..parts.len() - 1] {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "mod1" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "super" | "mod4" => modifiers.super_key = true,
            _ => {}
        }
    }

    Some(Keybinding {
        modifiers,
        key,
        action,
    })
}

/// Parse an action name string to a KeyAction.
fn parse_action(s: &str) -> Option<KeyAction> {
    match s {
        "workspace_new" => Some(KeyAction::WorkspaceNew),
        "workspace_close" => Some(KeyAction::WorkspaceClose),
        "workspace_next" => Some(KeyAction::WorkspaceNext),
        "workspace_prev" => Some(KeyAction::WorkspacePrev),
        "workspace_rename" => Some(KeyAction::WorkspaceRename),
        "split_right" => Some(KeyAction::SplitRight),
        "split_down" => Some(KeyAction::SplitDown),
        "pane_close" => Some(KeyAction::PaneClose),
        "pane_next" => Some(KeyAction::PaneNext),
        "pane_prev" => Some(KeyAction::PanePrev),
        "pane_focus_up" => Some(KeyAction::PaneFocusUp),
        "pane_focus_down" => Some(KeyAction::PaneFocusDown),
        "pane_focus_left" => Some(KeyAction::PaneFocusLeft),
        "pane_focus_right" => Some(KeyAction::PaneFocusRight),
        "pane_zoom_toggle" => Some(KeyAction::PaneZoomToggle),
        "toggle_sidebar" => Some(KeyAction::ToggleSidebar),
        "toggle_notifications" => Some(KeyAction::ToggleNotifications),
        "copy" => Some(KeyAction::Copy),
        "paste" => Some(KeyAction::Paste),
        "find_in_terminal" => Some(KeyAction::FindInTerminal),
        "leader_mode" => Some(KeyAction::EnterLeaderMode),
        "toggle_sandbox" => Some(KeyAction::ToggleSandbox),
        "toggle_audit_panel" => Some(KeyAction::ToggleAuditPanel),
        "toggle_git_diff" => Some(KeyAction::ToggleGitDiff),
        "font_size_increase" => Some(KeyAction::FontSizeIncrease),
        "font_size_decrease" => Some(KeyAction::FontSizeDecrease),
        "font_size_reset" => Some(KeyAction::FontSizeReset),
        "panel_close" => Some(KeyAction::PanelClose),
        "panel_next" => Some(KeyAction::PanelNext),
        "panel_prev" => Some(KeyAction::PanelPrev),
        "new_terminal" => Some(KeyAction::NewTerminal),
        "new_browser" => Some(KeyAction::NewBrowser),
        "toggle_settings" => Some(KeyAction::ToggleSettings),
        "toggle_token_usage" => Some(KeyAction::ToggleTokenUsage),
        "toggle_help" => Some(KeyAction::ToggleHelp),
        "toggle_agent_queue" => Some(KeyAction::ToggleAgentQueue),
        "toggle_plans" => Some(KeyAction::TogglePlans),
        "reload_config" => Some(KeyAction::ReloadConfig),
        "toggle_fullscreen" => Some(KeyAction::ToggleFullscreen),
        "quit" => Some(KeyAction::Quit),
        s if s.starts_with("workspace_select_") => {
            let n: u8 = s.strip_prefix("workspace_select_")?.parse().ok()?;
            Some(KeyAction::WorkspaceSelect(n))
        }
        _ => None,
    }
}

/// Merge user keybindings with defaults. User bindings override defaults
/// for matching actions.
pub fn merge_keybindings(defaults: Vec<Keybinding>, overrides: &[Keybinding]) -> Vec<Keybinding> {
    let mut result = defaults;
    for user_bind in overrides {
        // Remove any existing binding for this action.
        result.retain(|b| b.action != user_bind.action);
        result.push(user_bind.clone());
    }
    result
}

/// Return the default set of keybindings.
pub fn default_keybindings() -> Vec<Keybinding> {
    vec![
        // Workspace management
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "t".to_string(),
            action: KeyAction::WorkspaceNew,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "w".to_string(),
            action: KeyAction::PanelClose,
        },
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "Tab".to_string(),
            action: KeyAction::WorkspaceNext,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "Tab".to_string(),
            action: KeyAction::WorkspacePrev,
        },
        // Split pane management
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "d".to_string(),
            action: KeyAction::SplitRight,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "e".to_string(),
            action: KeyAction::SplitDown,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "x".to_string(),
            action: KeyAction::PaneClose,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "bracketright".to_string(),
            action: KeyAction::PaneNext,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "bracketleft".to_string(),
            action: KeyAction::PanePrev,
        },
        Keybinding {
            modifiers: Modifiers::ALT,
            key: "k".to_string(),
            action: KeyAction::PaneFocusUp,
        },
        Keybinding {
            modifiers: Modifiers::ALT,
            key: "j".to_string(),
            action: KeyAction::PaneFocusDown,
        },
        Keybinding {
            modifiers: Modifiers::ALT,
            key: "h".to_string(),
            action: KeyAction::PaneFocusLeft,
        },
        Keybinding {
            modifiers: Modifiers::ALT,
            key: "l".to_string(),
            action: KeyAction::PaneFocusRight,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "z".to_string(),
            action: KeyAction::PaneZoomToggle,
        },
        // Navigation
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "b".to_string(),
            action: KeyAction::ToggleSidebar,
        },
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "i".to_string(),
            action: KeyAction::ToggleNotifications,
        },
        // Terminal
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "c".to_string(),
            action: KeyAction::Copy,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "v".to_string(),
            action: KeyAction::Paste,
        },
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "f".to_string(),
            action: KeyAction::FindInTerminal,
        },
        // Audit
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "a".to_string(),
            action: KeyAction::ToggleAuditPanel,
        },
        // Git
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "g".to_string(),
            action: KeyAction::ToggleGitDiff,
        },
        // Font zoom (Ctrl+=/Ctrl+-/Ctrl+0)
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "equal".to_string(),
            action: KeyAction::FontSizeIncrease,
        },
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "minus".to_string(),
            action: KeyAction::FontSizeDecrease,
        },
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "0".to_string(),
            action: KeyAction::FontSizeReset,
        },
        // Settings
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "comma".to_string(),
            action: KeyAction::ToggleSettings,
        },
        // Token usage
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "u".to_string(),
            action: KeyAction::ToggleTokenUsage,
        },
        // Panel tab cycling
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "Page_Down".to_string(),
            action: KeyAction::PanelNext,
        },
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "Page_Up".to_string(),
            action: KeyAction::PanelPrev,
        },
        // Agent queue
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "p".to_string(),
            action: KeyAction::ToggleAgentQueue,
        },
        // Plans
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "l".to_string(),
            action: KeyAction::TogglePlans,
        },
        // Leader key
        Keybinding {
            modifiers: Modifiers::CTRL,
            key: "b".to_string(),
            action: KeyAction::EnterLeaderMode,
        },
        // Misc
        Keybinding {
            modifiers: Modifiers::CTRL_SHIFT,
            key: "r".to_string(),
            action: KeyAction::ReloadConfig,
        },
        Keybinding {
            modifiers: Modifiers::NONE,
            key: "F11".to_string(),
            action: KeyAction::ToggleFullscreen,
        },
        // Help
        Keybinding {
            modifiers: Modifiers::NONE,
            key: "F1".to_string(),
            action: KeyAction::ToggleHelp,
        },
        // Rename workspace
        Keybinding {
            modifiers: Modifiers::NONE,
            key: "F2".to_string(),
            action: KeyAction::WorkspaceRename,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keybind() {
        let binding = parse_keybind("ctrl+shift+t=workspace_new").unwrap();
        assert!(binding.modifiers.ctrl);
        assert!(binding.modifiers.shift);
        assert!(!binding.modifiers.alt);
        assert_eq!(binding.key, "t");
        assert_eq!(binding.action, KeyAction::WorkspaceNew);
    }

    #[test]
    fn test_parse_keybind_alt() {
        let binding = parse_keybind("alt+h=pane_focus_left").unwrap();
        assert!(binding.modifiers.alt);
        assert!(!binding.modifiers.ctrl);
        assert_eq!(binding.key, "h");
        assert_eq!(binding.action, KeyAction::PaneFocusLeft);
    }

    #[test]
    fn test_parse_keybind_no_modifier() {
        let binding = parse_keybind("F11=toggle_fullscreen").unwrap();
        assert_eq!(binding.modifiers, Modifiers::NONE);
        assert_eq!(binding.key, "F11");
        assert_eq!(binding.action, KeyAction::ToggleFullscreen);
    }

    #[test]
    fn test_parse_keybind_invalid() {
        assert!(parse_keybind("invalid").is_none());
        assert!(parse_keybind("ctrl+t=nonexistent_action").is_none());
    }

    #[test]
    fn test_parse_workspace_select() {
        let binding = parse_keybind("ctrl+3=workspace_select_3").unwrap();
        assert_eq!(binding.action, KeyAction::WorkspaceSelect(3));
    }

    #[test]
    fn test_merge_keybindings() {
        let defaults = vec![
            Keybinding {
                modifiers: Modifiers::CTRL_SHIFT,
                key: "t".to_string(),
                action: KeyAction::WorkspaceNew,
            },
            Keybinding {
                modifiers: Modifiers::CTRL_SHIFT,
                key: "w".to_string(),
                action: KeyAction::WorkspaceClose,
            },
        ];

        let overrides = vec![Keybinding {
            modifiers: Modifiers::CTRL,
            key: "n".to_string(),
            action: KeyAction::WorkspaceNew,
        }];

        let merged = merge_keybindings(defaults, &overrides);
        assert_eq!(merged.len(), 2);

        // WorkspaceNew should now be Ctrl+N, not Ctrl+Shift+T.
        let ws_new = merged.iter().find(|b| b.action == KeyAction::WorkspaceNew).unwrap();
        assert_eq!(ws_new.key, "n");
        assert!(ws_new.modifiers.ctrl);
        assert!(!ws_new.modifiers.shift);
    }

    #[test]
    fn test_parse_toggle_agent_queue() {
        let binding = parse_keybind("ctrl+shift+p=toggle_agent_queue").unwrap();
        assert_eq!(binding.action, KeyAction::ToggleAgentQueue);
        assert!(binding.modifiers.ctrl);
        assert!(binding.modifiers.shift);
        assert_eq!(binding.key, "p");
    }

    #[test]
    fn test_parse_case_insensitive_modifiers() {
        let b1 = parse_keybind("Ctrl+Shift+t=workspace_new").unwrap();
        assert!(b1.modifiers.ctrl);
        assert!(b1.modifiers.shift);

        let b2 = parse_keybind("CTRL+SHIFT+t=workspace_new").unwrap();
        assert!(b2.modifiers.ctrl);
        assert!(b2.modifiers.shift);

        let b3 = parse_keybind("Control+t=workspace_new").unwrap();
        assert!(b3.modifiers.ctrl);
    }

    #[test]
    fn test_parse_special_keys() {
        let f1 = parse_keybind("F1=toggle_help").unwrap();
        assert_eq!(f1.key, "F1");
        assert_eq!(f1.action, KeyAction::ToggleHelp);

        let f11 = parse_keybind("F11=toggle_fullscreen").unwrap();
        assert_eq!(f11.key, "F11");

        let pg = parse_keybind("ctrl+Page_Down=panel_next").unwrap();
        assert_eq!(pg.key, "Page_Down");
        assert_eq!(pg.action, KeyAction::PanelNext);
    }

    #[test]
    fn test_parse_unknown_action_returns_none() {
        assert!(parse_keybind("ctrl+t=does_not_exist").is_none());
        assert!(parse_keybind("ctrl+t=").is_none());
    }

    #[test]
    fn test_parse_workspace_rename() {
        let binding = parse_keybind("F2=workspace_rename").unwrap();
        assert_eq!(binding.modifiers, Modifiers::NONE);
        assert_eq!(binding.key, "F2");
        assert_eq!(binding.action, KeyAction::WorkspaceRename);
    }

    #[test]
    fn test_default_keybindings_contains_f2_rename() {
        let defaults = default_keybindings();
        let f2 = defaults
            .iter()
            .find(|b| b.action == KeyAction::WorkspaceRename)
            .expect("F2 WorkspaceRename should be in defaults");
        assert_eq!(f2.modifiers, Modifiers::NONE);
        assert_eq!(f2.key, "F2");
    }

    #[test]
    fn test_parse_mod1_and_mod4_aliases() {
        let b1 = parse_keybind("mod1+h=pane_focus_left").unwrap();
        assert!(b1.modifiers.alt);

        let b2 = parse_keybind("mod4+t=workspace_new").unwrap();
        assert!(b2.modifiers.super_key);
    }
}
