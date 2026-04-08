use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use thane_core::audit::{AuditEventType, AuditLog, AuditSeverity, scan_queue_output_log};
use thane_core::config::Config;
use thane_core::cost_tracker::{Plan, TokenLimitInfo, fetch_oauth_usage, read_oauth_token, read_subscription_type};
use thane_core::git::GitInfo;
use thane_core::keybinding::KeyAction;
use thane_core::notification::{Notification, parse_osc_notification};
use thane_core::pane::Orientation as PaneOrientation;
use thane_core::panel::{PanelId, PanelType};
use thane_core::queue_executor;
use thane_core::agent_queue::{AgentQueue, QueueProcessingMode, ScheduleEntry, parse_schedule};
use thane_core::port_scanner;
use thane_core::session::{AppSnapshot, PanelSnapshot, WorkspaceHistory, WorkspaceSnapshot};
use thane_core::workspace::WorkspaceManager;
use thane_persist::audit_store::AuditStore;
use thane_persist::history_store::HistoryStore;
use thane_persist::queue_history_store::QueueHistoryStore;
use thane_persist::policy::PersistPolicy;
use thane_persist::store::SessionStore;
use thane_platform::linux::LinuxNotifier;
use thane_platform::traits::{DesktopNotifier, NotifyUrgency, PlatformDirs};
use thane_browser::webkit_backend::WebKitEngine;
use thane_terminal::traits::TerminalSurface;
use thane_terminal::vte_backend::VteEngine;

use crate::browser::browser_surface::BrowserPanel;
use gdk4::Key;
use gtk4::prelude::*;
use uuid::Uuid;

use crate::shortcuts;
use crate::sidebar::audit_panel::{AuditPanel, show_audit_detail_dialog};
use crate::sidebar::git_diff_panel::GitDiffPanel;
use crate::sidebar::help_panel::HelpPanel;
use crate::sidebar::notification_panel::NotificationPanel;
use crate::sidebar::agent_queue_panel::AgentQueuePanel;
use crate::sidebar::plans_panel::{PlansPanel, show_plan_detail_dialog};
use crate::sidebar::sandbox_panel::SandboxPanel;
use crate::sidebar::settings_panel::SettingsPanel;
use crate::sidebar::sidebar_view::SidebarView;
use crate::sidebar::status_bar::StatusBar;
use crate::sidebar::token_panel::TokenPanel;
use crate::split::split_container::SplitContainer;
use crate::terminal::terminal_surface::TerminalPanel;

/// Public type alias for the shared app state (used by RPC handler).
pub(crate) type AppStateHandle = AppState;

/// Shared application state, accessible from closures.
pub(crate) struct AppState {
    workspace_mgr: WorkspaceManager,
    engine: VteEngine,
    browser_engine: WebKitEngine,
    sidebar: SidebarView,
    notification_panel: NotificationPanel,
    workspace_stack: gtk4::Stack,
    /// Per-workspace split containers (keyed by workspace UUID).
    split_containers: HashMap<Uuid, SplitContainer>,
    /// Per-workspace terminal panels (keyed by PanelId).
    terminal_panels: HashMap<PanelId, TerminalPanel>,
    /// Per-workspace browser panels (keyed by PanelId).
    browser_panels: HashMap<PanelId, BrowserPanel>,
    /// Maps panel IDs to their workspace IDs (for routing notifications).
    panel_workspace_map: HashMap<PanelId, Uuid>,
    /// Desktop notification sender.
    notifier: LinuxNotifier,
    /// Whether we're currently inside a leader key sequence (Ctrl+B prefix).
    leader_mode: bool,
    /// Whether the notification panel is visible.
    notification_panel_visible: bool,
    /// Audit trail panel.
    audit_panel: AuditPanel,
    /// Whether the audit panel is visible.
    audit_panel_visible: bool,
    /// Git diff panel.
    git_diff_panel: GitDiffPanel,
    /// Whether the git diff panel is visible.
    git_diff_panel_visible: bool,
    /// Settings panel.
    settings_panel: SettingsPanel,
    /// Whether the settings panel is visible.
    settings_panel_visible: bool,
    /// Token usage panel.
    token_panel: TokenPanel,
    /// Whether the token panel is visible.
    token_panel_visible: bool,
    /// Help panel.
    help_panel: HelpPanel,
    /// Whether the help panel is visible.
    help_panel_visible: bool,
    /// Agent queue panel.
    agent_queue_panel: AgentQueuePanel,
    /// Whether the agent queue panel is visible.
    agent_queue_panel_visible: bool,
    /// Plans panel (completed queue tasks).
    plans_panel: PlansPanel,
    /// Whether the plans panel is visible.
    plans_panel_visible: bool,
    /// Sandbox configuration panel.
    sandbox_panel: SandboxPanel,
    /// Whether the sandbox panel is visible.
    sandbox_panel_visible: bool,
    /// Bottom status bar.
    status_bar: StatusBar,
    /// Current font size for terminals.
    current_font_size: f64,
    /// Base font size from config (for reset).
    base_font_size: f64,
    /// Current font family.
    current_font_family: String,
    /// Pane zoom: if Some, the zoomed pane's workspace and pane id.
    zoomed_pane: Option<(Uuid, thane_core::pane::PaneId)>,
    /// Last known config file modification time (for hot-reload).
    config_mtime: Option<std::time::SystemTime>,
    /// Agent execution queue.
    agent_queue: AgentQueue,
    /// Currently running headless queue entry: (entry_id, child process).
    running_queue_entry: Option<(Uuid, std::process::Child)>,
    /// Audit log for security monitoring.
    audit_log: AuditLog,
    /// Weak self-reference for tab bar callbacks.
    self_ref: Option<std::rc::Weak<RefCell<AppState>>>,
    /// Weak reference to the application window (for confirmation dialogs).
    window_ref: Option<glib::WeakRef<gtk4::ApplicationWindow>>,
    /// Current UI font size for sidebar/panels.
    current_ui_font_size: f64,
    /// CSS provider for dynamic UI font size.
    ui_css_provider: gtk4::CssProvider,
    /// Whether to show confirmation dialogs on close.
    confirm_close: bool,
    /// Whether clicking a URL opens in the embedded browser.
    link_url_in_app: bool,
    /// Whether Shift+clicking a URL opens in the system browser.
    link_url_in_browser: bool,
    /// Loaded configuration.
    config: Config,
    /// Per-panel command block trackers (keyed by PanelId).
    block_trackers: HashMap<PanelId, thane_core::command_block::BlockTracker>,
    /// Recently-closed workspace history.
    workspace_history: WorkspaceHistory,
    /// UUIDs of Claude prompts already logged to audit (for dedup).
    seen_prompt_uuids: std::collections::HashSet<String>,
    /// Per-panel detected agent name (e.g. "claude", "codex"). Updated each metadata refresh.
    panel_agents: HashMap<PanelId, String>,
    /// Per-panel last terminal output time (for agent stall detection).
    last_output_times: HashMap<PanelId, std::time::Instant>,
    /// CWD currently shown in the git diff panel (None = workspace CWD).
    git_diff_cwd: Option<String>,
    /// When this app instance started (for filtering session costs).
    started_at: std::time::SystemTime,
    /// Cached parsed queue schedule entries.
    queue_schedule: Vec<ScheduleEntry>,
    /// Queue history store for persisting completed entries.
    queue_history_store: QueueHistoryStore,
    /// Cached JSONL cost scanner (avoids re-parsing unchanged files).
    cost_cache: thane_core::cost_tracker::CostCache,
    /// Cached OAuth usage data for token limits (fetched periodically).
    cached_limit_info: Option<TokenLimitInfo>,
    /// When the OAuth usage data was last fetched.
    limit_info_fetched_at: Option<std::time::Instant>,
    /// Whether the Claude CLI binary is available on this system.
    has_claude: bool,
}

impl AppState {
    /// Get a reference to the workspace manager.
    pub(crate) fn workspace_mgr(&self) -> &WorkspaceManager {
        &self.workspace_mgr
    }

    /// Get a mutable reference to the workspace manager.
    pub(crate) fn workspace_mgr_mut(&mut self) -> &mut WorkspaceManager {
        &mut self.workspace_mgr
    }

    /// Get a reference to the agent queue.
    pub(crate) fn agent_queue(&self) -> &AgentQueue {
        &self.agent_queue
    }

    /// Get a mutable reference to the agent queue.
    pub(crate) fn agent_queue_mut(&mut self) -> &mut AgentQueue {
        &mut self.agent_queue
    }

    /// Get a reference to the loaded configuration.
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    /// Get a reference to the audit log.
    #[allow(dead_code)]
    pub(crate) fn audit_log(&self) -> &AuditLog {
        &self.audit_log
    }

    /// Get a mutable reference to the audit log.
    pub(crate) fn audit_log_mut(&mut self) -> &mut AuditLog {
        &mut self.audit_log
    }

    /// Get a reference to the workspace history.
    pub(crate) fn workspace_history(&self) -> &WorkspaceHistory {
        &self.workspace_history
    }

    /// Get a mutable reference to the workspace history.
    pub(crate) fn workspace_history_mut(&mut self) -> &mut WorkspaceHistory {
        &mut self.workspace_history
    }

    /// Save the workspace history to disk immediately.
    pub(crate) fn save_history(&self) {
        let sessions_dir = thane_platform::dirs::LinuxDirs.sessions_dir();
        let store = HistoryStore::new(sessions_dir);
        if let Err(e) = store.save(&self.workspace_history) {
            tracing::error!("Failed to save workspace history: {e}");
        }
    }

    /// Reopen a previously closed workspace from history.
    /// Returns the new workspace ID and initial panel ID, or None if not found.
    pub(crate) fn reopen_from_history(&mut self, original_id: Uuid) -> Option<(Uuid, PanelId)> {
        let record = self.workspace_history.remove(original_id)?;
        self.save_history();

        // Validate CWD exists, fall back to home directory.
        let cwd = if std::path::Path::new(&record.cwd).is_dir() {
            record.cwd
        } else {
            dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string())
        };

        let (ws_id, panel_id) = self.create_workspace(&record.title, &cwd);

        // Restore the tag if it was set.
        if let Some(tag) = record.tag
            && let Some(ws) = self.workspace_mgr.get_mut(ws_id)
        {
            ws.tag = Some(tag);
        }

        self.refresh_sidebar();
        Some((ws_id, panel_id))
    }

    /// Refresh the sidebar to reflect current workspace state.
    pub(crate) fn refresh_sidebar(&mut self) {
        let workspaces = self.workspace_mgr.list();
        let active_index = self.workspace_mgr.active_index();
        let (cost_mode, util_pct, sub_cost) = self
            .cached_limit_info
            .as_ref()
            .map(|info| {
                (
                    info.display_mode_with_override(self.config.enterprise_monthly_cost()),
                    info.primary_utilization(),
                    info.derived_subscription_cost_with_override(self.config.enterprise_monthly_cost()),
                )
            })
            .unwrap_or((thane_core::cost_tracker::CostDisplayMode::Dollar, None, None));
        let use_alltime = self.config.cost_display_scope() == "all-time";
        self.sidebar
            .update_workspaces(workspaces, active_index, cost_mode, util_pct, sub_cost, use_alltime);
        self.sidebar.update_history(self.workspace_history.list());
    }

    /// Switch the visible stack child to the active workspace.
    pub(crate) fn switch_to_active_workspace(&self) {
        if let Some(ws) = self.workspace_mgr.active() {
            let name = ws.id.to_string();
            self.workspace_stack.set_visible_child_name(&name);
            self.focus_active_terminal();
        }
    }

    /// Focus the panel in the active workspace's focused pane.
    fn focus_active_terminal(&self) {
        if let Some(ws) = self.workspace_mgr.active()
            && let Some(panel) = ws.focused_panel()
        {
            if let Some(tp) = self.terminal_panels.get(&panel.id) {
                tp.grab_focus();
            } else if let Some(bp) = self.browser_panels.get(&panel.id) {
                bp.grab_focus();
            }
        }
    }

    /// Rebuild the split container for the active workspace.
    /// Now includes tab bars for panes with multiple panels.
    fn rebuild_active_splits(&self) {
        if let Some(ws) = self.workspace_mgr.active()
            && let Some(container) = self.split_containers.get(&ws.id) {
                let terminal_panels = &self.terminal_panels;
                let browser_panels = &self.browser_panels;
                let ws_panels = &ws.panels;
                let focused_pane = ws.focused_pane_id;
                let self_ref = self.self_ref.clone();
                container.rebuild(&ws.split_tree, &|pane_id, panel_ids, selected_panel| {
                    // Build the selected panel widget.
                    let widget_opt = terminal_panels
                        .get(&selected_panel)
                        .map(|tp| tp.widget().clone().upcast::<gtk4::Widget>())
                        .or_else(|| {
                            browser_panels
                                .get(&selected_panel)
                                .map(|bp| bp.widget().clone().upcast::<gtk4::Widget>())
                        });

                    let panel_widget = if let Some(widget) = widget_opt {
                        // Re-parent: remove from current parent first.
                        if let Some(parent) = widget.parent() {
                            if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
                                if paned.start_child().as_ref() == Some(&widget) {
                                    paned.set_start_child(gtk4::Widget::NONE);
                                } else {
                                    paned.set_end_child(gtk4::Widget::NONE);
                                }
                            } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
                                bx.remove(&widget);
                            }
                        }
                        widget
                    } else {
                        let placeholder = gtk4::Label::new(Some("(no panel)"));
                        placeholder.upcast()
                    };

                    // Build tab bar (auto-hides for single panel).
                    use crate::split::tab_bar::{TabBar, TabInfo};
                    let tabs: Vec<TabInfo> = panel_ids.iter().filter_map(|pid| {
                        ws_panels.get(pid).map(|info| TabInfo {
                            panel_id: *pid,
                            title: info.title.clone(),
                            panel_type: info.panel_type,
                            is_selected: *pid == selected_panel,
                        })
                    }).collect();

                    let self_ref_select = self_ref.clone();
                    let self_ref_close = self_ref.clone();
                    let on_select = std::rc::Rc::new(move |panel_id: PanelId| {
                        if let Some(sr) = self_ref_select.as_ref().and_then(|w| w.upgrade()) {
                            glib::idle_add_local_once(move || {
                                let mut s = sr.borrow_mut();
                                s.select_panel_and_rebuild(panel_id);
                            });
                        }
                    });
                    let on_close = std::rc::Rc::new(move |panel_id: PanelId| {
                        if let Some(sr) = self_ref_close.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                let win = sr2
                                    .borrow()
                                    .window_ref
                                    .as_ref()
                                    .and_then(|w| w.upgrade());
                                if let Some(win) = win {
                                    show_close_panel_dialog(&win, &sr2, panel_id);
                                } else {
                                    sr2.borrow_mut().close_panel_by_id(panel_id);
                                }
                            });
                        }
                    });

                    let self_ref_screenshot = self_ref.clone();
                    let panel_for_screenshot = selected_panel;
                    let on_screenshot: Rc<dyn Fn()> = std::rc::Rc::new(move || {
                        if let Some(sr) = self_ref_screenshot.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                let s = sr2.borrow();
                                let widget_opt: Option<gtk4::Widget> = s.terminal_panels
                                    .get(&panel_for_screenshot)
                                    .map(|tp| tp.widget().clone().upcast::<gtk4::Widget>())
                                    .or_else(|| {
                                        s.browser_panels
                                            .get(&panel_for_screenshot)
                                            .map(|bp| bp.widget().clone().upcast::<gtk4::Widget>())
                                    });
                                if let Some(widget) = widget_opt {
                                    take_screenshot_with_feedback(&widget);
                                }
                            });
                        }
                    });

                    let self_ref_git_diff = self_ref.clone();
                    let panel_for_git_diff = selected_panel;
                    let on_git_diff: Rc<dyn Fn()> = std::rc::Rc::new(move || {
                        if let Some(sr) = self_ref_git_diff.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                let mut s = sr2.borrow_mut();
                                let cwd = s.workspace_mgr.active()
                                    .and_then(|ws| ws.panels.get(&panel_for_git_diff))
                                    .map(|p| p.location.clone());
                                if let Some(cwd) = cwd {
                                    s.open_git_diff_for_path(cwd);
                                } else {
                                    s.toggle_git_diff_panel();
                                }
                            });
                        }
                    });

                    let self_ref_find = self_ref.clone();
                    let panel_for_find = selected_panel;
                    let on_find: Rc<dyn Fn()> = std::rc::Rc::new(move || {
                        if let Some(sr) = self_ref_find.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                let s = sr2.borrow();
                                if let Some(tp) = s.terminal_panels.get(&panel_for_find) {
                                    tp.toggle_search();
                                }
                            });
                        }
                    });

                    let self_ref_split_r = self_ref.clone();
                    let pane_for_split_r = pane_id;
                    let on_split_right: Rc<dyn Fn()> = std::rc::Rc::new(move || {
                        if let Some(sr) = self_ref_split_r.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                {
                                    let mut s = sr2.borrow_mut();
                                    if let Some(ws) = s.workspace_mgr.active_mut() {
                                        ws.focused_pane_id = pane_for_split_r;
                                    }
                                }
                                let result = sr2.borrow_mut().split_pane(PaneOrientation::Horizontal);
                                if let Some((ws_id, panel_id)) = result {
                                    wire_terminal_notifications(&sr2, ws_id, panel_id);
                                }
                            });
                        }
                    });

                    let self_ref_split_d = self_ref.clone();
                    let pane_for_split_d = pane_id;
                    let on_split_down: Rc<dyn Fn()> = std::rc::Rc::new(move || {
                        if let Some(sr) = self_ref_split_d.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                {
                                    let mut s = sr2.borrow_mut();
                                    if let Some(ws) = s.workspace_mgr.active_mut() {
                                        ws.focused_pane_id = pane_for_split_d;
                                    }
                                }
                                let result = sr2.borrow_mut().split_pane(PaneOrientation::Vertical);
                                if let Some((ws_id, panel_id)) = result {
                                    wire_terminal_notifications(&sr2, ws_id, panel_id);
                                }
                            });
                        }
                    });

                    let self_ref_close_pane = self_ref.clone();
                    let pane_for_close = pane_id;
                    let on_close_pane: Rc<dyn Fn()> = std::rc::Rc::new(move || {
                        if let Some(sr) = self_ref_close_pane.as_ref().and_then(|w| w.upgrade()) {
                            let sr2 = sr.clone();
                            glib::idle_add_local_once(move || {
                                {
                                    let mut s = sr2.borrow_mut();
                                    if let Some(ws) = s.workspace_mgr.active_mut() {
                                        ws.focused_pane_id = pane_for_close;
                                    }
                                }
                                let win = sr2
                                    .borrow()
                                    .window_ref
                                    .as_ref()
                                    .and_then(|w| w.upgrade());
                                if let Some(win) = win {
                                    show_close_pane_dialog(&win, &sr2);
                                } else {
                                    sr2.borrow_mut().close_focused_pane();
                                }
                            });
                        }
                    });

                    let tab_bar = TabBar::new(&tabs, on_select, on_close, on_screenshot, on_git_diff, on_find, on_split_right, on_split_down, on_close_pane);

                    // Wire tab reorder (drag-and-drop).
                    {
                        let self_ref_reorder = self_ref.clone();
                        tab_bar.connect_reorder(move |source_panel, target_panel| {
                            if let Some(sr) = self_ref_reorder.as_ref().and_then(|w| w.upgrade()) {
                                let sr2 = sr.clone();
                                glib::idle_add_local_once(move || {
                                    let mut s = sr2.borrow_mut();
                                    if let Some(ws) = s.workspace_mgr.active_mut() {
                                        ws.swap_panels_in_pane(source_panel, target_panel);
                                    }
                                    s.rebuild_active_splits();
                                });
                            }
                        });
                    }

                    // Wrap in vertical box: tab bar on top, panel widget below.
                    let wrapper = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                    wrapper.append(tab_bar.widget());
                    panel_widget.set_hexpand(true);
                    panel_widget.set_vexpand(true);
                    wrapper.append(&panel_widget);
                    wrapper.set_hexpand(true);
                    wrapper.set_vexpand(true);

                    // Apply focused ring and unfocused opacity for multi-pane workspaces.
                    let multi_pane = ws.pane_count() > 1;
                    if pane_id == focused_pane {
                        wrapper.remove_css_class("pane-unfocused");
                        if multi_pane {
                            wrapper.add_css_class("pane-focused");
                        } else {
                            wrapper.remove_css_class("pane-focused");
                        }
                    } else {
                        wrapper.add_css_class("pane-unfocused");
                        wrapper.remove_css_class("pane-focused");
                    }

                    wrapper.upcast()
                });
            }
    }

    /// Restore a workspace from a session snapshot.
    /// Restore a workspace from a full snapshot, recreating all terminals and browsers.
    /// Returns (workspace_id, Vec<terminal_panel_ids>) for notification wiring.
    fn restore_workspace_from_snapshot(
        &mut self,
        ws_snap: &WorkspaceSnapshot,
    ) -> (Uuid, Vec<PanelId>) {
        use thane_core::workspace::Workspace;

        let ws_id = ws_snap.id;

        let ws = Workspace::restore_from_snapshot(ws_snap);
        let ws_cwd = ws.cwd.clone();
        let sandbox = ws.sandbox_policy.clone();
        self.workspace_mgr.add(ws);

        let socket_path_env = std::env::var("THANE_SOCKET_PATH").unwrap_or_default();

        // Build a lookup of panel_id → scrollback from the snapshot.
        let scrollback_map: HashMap<PanelId, &str> = ws_snap
            .panels
            .iter()
            .filter_map(|ps| ps.scrollback.as_deref().map(|sb| (ps.info.id, sb)))
            .collect();

        // Create GTK widgets for every panel in the snapshot.
        let mut terminal_panel_ids = Vec::new();
        for ps in &ws_snap.panels {
            let panel_id = ps.info.id;
            match ps.info.panel_type {
                PanelType::Terminal => {
                    let env_vars = [
                        ("THANE_WORKSPACE_ID", ws_id.to_string()),
                        ("THANE_SURFACE_ID", panel_id.to_string()),
                        ("THANE_SOCKET_PATH", socket_path_env.clone()),
                    ];
                    let env_refs: Vec<(&str, &str)> = env_vars
                        .iter()
                        .map(|(k, v)| (*k, v.as_str()))
                        .collect();
                    let terminal = if sandbox.enabled {
                        TerminalPanel::new_sandboxed(
                            &self.engine,
                            panel_id,
                            &ws_cwd,
                            &env_refs,
                            &sandbox,
                        )
                    } else {
                        TerminalPanel::new(&self.engine, panel_id, &ws_cwd, &env_refs)
                    };

                    // Feed saved scrollback into the terminal.
                    // The saved text was captured at the previous terminal width,
                    // so we strip trailing whitespace from each line and re-join
                    // with \r\n to let VTE re-wrap at the current width. We also
                    // dim the restored text so it's visually distinct from new
                    // output (colors are lost during plain-text capture).
                    if let Some(text) = scrollback_map.get(&panel_id)
                        && !text.is_empty()
                    {
                        // SGR 2 = dim, SGR 0 = reset at the end.
                        terminal.surface().feed(b"\x1b[2m");
                        for line in text.lines() {
                            let trimmed = line.trim_end();
                            terminal.surface().feed(trimmed.as_bytes());
                            terminal.surface().feed(b"\r\n");
                        }
                        terminal.surface().feed(b"\x1b[0m");
                    }

                    self.terminal_panels.insert(panel_id, terminal);
                    terminal_panel_ids.push(panel_id);
                }
                PanelType::Browser => {
                    let url = ps.url.as_deref().unwrap_or(&ps.info.location);
                    let browser = BrowserPanel::new(&self.browser_engine, panel_id, url);

                    // Wire omnibar close button.
                    if let Some(sr) = self.self_ref.as_ref().and_then(|w| w.upgrade()) {
                        let sr_clone = sr.clone();
                        browser.omnibar().connect_close(move || {
                            let sr2 = sr_clone.clone();
                            glib::idle_add_local_once(move || {
                                let win = sr2
                                    .borrow()
                                    .window_ref
                                    .as_ref()
                                    .and_then(|w| w.upgrade());
                                if let Some(win) = win {
                                    show_close_panel_dialog(&win, &sr2, panel_id);
                                } else {
                                    sr2.borrow_mut().close_panel_by_id(panel_id);
                                }
                            });
                        });
                    }

                    self.browser_panels.insert(panel_id, browser);
                }
            }
            self.panel_workspace_map.insert(panel_id, ws_id);
        }

        // Create a split container for this workspace.
        let split_container = SplitContainer::new();
        self.workspace_stack
            .add_named(split_container.widget(), Some(&ws_id.to_string()));
        self.split_containers.insert(ws_id, split_container);

        self.rebuild_active_splits();
        self.refresh_sidebar();

        (ws_id, terminal_panel_ids)
    }

    /// Create a new workspace with a terminal and add it to the stack.
    /// Returns (workspace_id, panel_id) for signal wiring.
    pub(crate) fn create_workspace(&mut self, title: &str, cwd: &str) -> (Uuid, PanelId) {
        let ws = self.workspace_mgr.create(title, cwd);
        let ws_id = ws.id;
        let panel = ws.focused_panel().unwrap();
        let panel_id = panel.id;
        let ws_cwd = ws.cwd.clone();

        // Create the terminal panel with thane env vars.
        let socket_path = std::env::var("THANE_SOCKET_PATH").unwrap_or_default();
        let env_vars = [
            ("THANE_WORKSPACE_ID", ws_id.to_string()),
            ("THANE_SURFACE_ID", panel_id.to_string()),
            ("THANE_SOCKET_PATH", socket_path),
        ];
        let env_refs: Vec<(&str, &str)> = env_vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let terminal = TerminalPanel::new(&self.engine, panel_id, &ws_cwd, &env_refs);

        // Create a split container for this workspace.
        let split_container = SplitContainer::new();
        self.workspace_stack
            .add_named(split_container.widget(), Some(&ws_id.to_string()));

        self.terminal_panels.insert(panel_id, terminal);
        self.panel_workspace_map.insert(panel_id, ws_id);
        self.split_containers.insert(ws_id, split_container);
        self.block_trackers
            .insert(panel_id, thane_core::command_block::BlockTracker::new(100));

        // Rebuild to populate the container.
        self.rebuild_active_splits();
        self.switch_to_active_workspace();
        self.refresh_sidebar();

        (ws_id, panel_id)
    }

    /// Create a new sandboxed workspace confined to the given directory.
    /// Returns (workspace_id, vec of (ws_id, panel_id) pairs) for signal wiring.
    pub(crate) fn create_sandboxed_workspace(
        &mut self,
        title: &str,
        root_dir: &str,
    ) -> Vec<(Uuid, PanelId)> {
        let (ws_id, _initial_panel) = self.create_workspace(title, root_dir);
        // Apply sandbox policy.
        if let Some(ws) = self.workspace_mgr.get_mut(ws_id) {
            ws.sandbox_policy =
                thane_core::sandbox::SandboxPolicy::confined_to(root_dir);
        }
        // Respawn terminals with sandbox enforcement.
        self.respawn_workspace_terminals()
    }

    /// Close the active workspace and remove from stack.
    /// If this was the last workspace, creates a fresh one.
    /// Records the closed workspace in history.
    pub(crate) fn close_active_workspace(&mut self) {
        // Collect panel IDs to remove.
        let panel_ids: Vec<PanelId> = self
            .workspace_mgr
            .active()
            .map(|ws| ws.panels.keys().copied().collect())
            .unwrap_or_default();

        if let Some(closed_ws) = self.workspace_mgr.close_active() {
            let ws_id = closed_ws.id;

            // Record in history before cleanup.
            let record = thane_core::session::ClosedWorkspaceRecord::from_workspace(&closed_ws);
            self.workspace_history.push(record);
            self.save_history();

            let name = ws_id.to_string();
            if let Some(child) = self.workspace_stack.child_by_name(&name) {
                self.workspace_stack.remove(&child);
            }
            self.split_containers.remove(&ws_id);
            for pid in panel_ids {
                self.terminal_panels.remove(&pid);
                self.panel_workspace_map.remove(&pid);
                self.block_trackers.remove(&pid);
                self.last_output_times.remove(&pid);
            }
            self.zoomed_pane = None;

            // If all workspaces are gone, create a fresh one.
            if self.workspace_mgr.count() == 0 {
                let cwd = dirs::home_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "/".to_string());
                self.create_workspace("Workspace", &cwd);
            } else {
                self.switch_to_active_workspace();
                self.refresh_sidebar();
            }
        }
    }

    /// Select a workspace by 0-based index.
    pub(crate) fn select_workspace(&mut self, index: usize) {
        if self.workspace_mgr.select(index) {
            self.zoomed_pane = None;
            self.switch_to_active_workspace();
            self.refresh_sidebar();
            self.refresh_panels_for_workspace();
        }
    }

    /// Select next workspace.
    fn select_next_workspace(&mut self) {
        self.workspace_mgr.select_next();
        self.zoomed_pane = None;
        self.switch_to_active_workspace();
        self.refresh_sidebar();
        self.refresh_panels_for_workspace();
    }

    /// Select previous workspace.
    fn select_prev_workspace(&mut self) {
        self.workspace_mgr.select_prev();
        self.zoomed_pane = None;
        self.switch_to_active_workspace();
        self.refresh_sidebar();
        self.refresh_panels_for_workspace();
    }

    /// Refresh any open right-side panels to reflect the active workspace's data.
    fn refresh_panels_for_workspace(&mut self) {
        // Auto-close git diff panel when switching workspaces.
        if self.git_diff_panel_visible {
            self.git_diff_panel_visible = false;
            self.git_diff_panel.widget().set_visible(false);
            self.git_diff_cwd = None;
        }
        if self.sandbox_panel_visible
            && let Some(ws) = self.workspace_mgr.active()
        {
            let policy = ws.sandbox_policy.clone();
            self.sandbox_panel.update(&policy);
        }
        if self.notification_panel_visible {
            self.refresh_notification_panel();
        }
    }

    /// Respawn all terminal panels in the active workspace.
    /// Used when sandbox settings change so the new policy is applied.
    /// Returns the list of (ws_id, panel_id) pairs for signal rewiring.
    fn respawn_workspace_terminals(&mut self) -> Vec<(Uuid, PanelId)> {
        let ws = match self.workspace_mgr.active() {
            Some(ws) => ws,
            None => return vec![],
        };
        let ws_id = ws.id;
        let ws_cwd = ws.cwd.clone();
        let sandbox = ws.sandbox_policy.clone();

        // Collect terminal panel IDs for this workspace.
        let panel_ids: Vec<PanelId> = self
            .panel_workspace_map
            .iter()
            .filter(|(_, wid)| **wid == ws_id)
            .map(|(pid, _)| *pid)
            .filter(|pid| self.terminal_panels.contains_key(pid))
            .collect();

        let mut pairs = Vec::new();
        for panel_id in panel_ids {
            // Remove old terminal.
            self.terminal_panels.remove(&panel_id);

            // Create new terminal with current sandbox settings.
            let socket_path = std::env::var("THANE_SOCKET_PATH").unwrap_or_default();
            let env_vars = [
                ("THANE_WORKSPACE_ID", ws_id.to_string()),
                ("THANE_SURFACE_ID", panel_id.to_string()),
                ("THANE_SOCKET_PATH", socket_path),
            ];
            let env_refs: Vec<(&str, &str)> =
                env_vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let terminal = if sandbox.enabled {
                TerminalPanel::new_sandboxed(&self.engine, panel_id, &ws_cwd, &env_refs, &sandbox)
            } else {
                TerminalPanel::new(&self.engine, panel_id, &ws_cwd, &env_refs)
            };
            self.terminal_panels.insert(panel_id, terminal);
            self.block_trackers
                .insert(panel_id, thane_core::command_block::BlockTracker::new(100));
            self.last_output_times.remove(&panel_id);
            pairs.push((ws_id, panel_id));
        }

        self.rebuild_active_splits();
        self.focus_active_terminal();
        pairs
    }

    /// Split the focused pane in the active workspace.
    /// Returns (workspace_id, panel_id) for signal wiring, if successful.
    pub(crate) fn split_pane(&mut self, orientation: PaneOrientation) -> Option<(Uuid, PanelId)> {
        let ws = self.workspace_mgr.active_mut()?;
        let ws_id = ws.id;
        let ws_cwd = ws.cwd.clone();
        let sandbox = ws.sandbox_policy.clone();

        let result = ws.split_terminal(orientation);
        match result {
            Ok((_new_pane_id, panel_id)) => {
                // Create a new terminal for the new pane with thane env vars.
                let socket_path = std::env::var("THANE_SOCKET_PATH").unwrap_or_default();
                let env_vars = [
                    ("THANE_WORKSPACE_ID", ws_id.to_string()),
                    ("THANE_SURFACE_ID", panel_id.to_string()),
                    ("THANE_SOCKET_PATH", socket_path),
                ];
                let env_refs: Vec<(&str, &str)> = env_vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let terminal = if sandbox.enabled {
                    TerminalPanel::new_sandboxed(&self.engine, panel_id, &ws_cwd, &env_refs, &sandbox)
                } else {
                    TerminalPanel::new(&self.engine, panel_id, &ws_cwd, &env_refs)
                };
                self.terminal_panels.insert(panel_id, terminal);
                self.panel_workspace_map.insert(panel_id, ws_id);
                self.block_trackers
                    .insert(panel_id, thane_core::command_block::BlockTracker::new(100));
                self.rebuild_active_splits();
                self.focus_active_terminal();
                Some((ws_id, panel_id))
            }
            Err(e) => {
                tracing::error!("Failed to split pane: {e}");
                None
            }
        }
    }

    /// Open a browser as a new tab in the focused pane.
    /// Returns (workspace_id, panel_id) if successful.
    pub(crate) fn open_browser(&mut self, url: &str) -> Option<(Uuid, PanelId)> {
        let ws = self.workspace_mgr.active_mut()?;
        let ws_id = ws.id;

        match ws.add_browser_to_focused_pane(url) {
            Ok(panel_id) => {
                let browser = BrowserPanel::new(&self.browser_engine, panel_id, url);

                // Wire the omnibar close button (with confirmation).
                if let Some(sr) = self.self_ref.as_ref().and_then(|w| w.upgrade()) {
                    let sr_clone = sr.clone();
                    browser.omnibar().connect_close(move || {
                        let sr2 = sr_clone.clone();
                        glib::idle_add_local_once(move || {
                            let win = sr2
                                .borrow()
                                .window_ref
                                .as_ref()
                                .and_then(|w| w.upgrade());
                            if let Some(win) = win {
                                show_close_panel_dialog(&win, &sr2, panel_id);
                            } else {
                                sr2.borrow_mut().close_panel_by_id(panel_id);
                            }
                        });
                    });
                }

                self.browser_panels.insert(panel_id, browser);
                self.panel_workspace_map.insert(panel_id, ws_id);
                self.rebuild_active_splits();
                self.focus_active_terminal();
                Some((ws_id, panel_id))
            }
            Err(e) => {
                tracing::error!("Failed to open browser: {e}");
                None
            }
        }
    }

    /// Close a panel by ID, removing its widget and cleaning up.
    fn close_panel_by_id(&mut self, panel_id: PanelId) {
        let ws = match self.workspace_mgr.active_mut() {
            Some(ws) => ws,
            None => return,
        };

        let pane_id = match ws.pane_for_panel(panel_id) {
            Some(id) => id,
            None => return,
        };

        match ws.close_panel(pane_id, panel_id) {
            Ok(pane_closed) => {
                self.terminal_panels.remove(&panel_id);
                self.browser_panels.remove(&panel_id);
                self.panel_workspace_map.remove(&panel_id);
                self.last_output_times.remove(&panel_id);

                if pane_closed {
                    // If it was the last pane with no more panels, close workspace.
                    let ws = self.workspace_mgr.active().unwrap();
                    if ws.split_tree.all_panel_ids().is_empty() {
                        self.close_active_workspace();
                        return;
                    }
                }

                self.zoomed_pane = None;
                self.rebuild_active_splits();
                self.focus_active_terminal();
            }
            Err(e) => {
                tracing::error!("Failed to close panel: {e}");
            }
        }
    }

    /// Select a panel in the focused pane and rebuild.
    fn select_panel_and_rebuild(&mut self, panel_id: PanelId) {
        if let Some(ws) = self.workspace_mgr.active_mut() {
            ws.select_panel(panel_id);
        }
        self.rebuild_active_splits();
        self.focus_active_terminal();
    }

    /// Cycle to the next panel in the focused pane.
    fn next_panel(&mut self) {
        if let Some(ws) = self.workspace_mgr.active_mut() {
            ws.next_panel();
        }
        self.rebuild_active_splits();
        self.focus_active_terminal();
    }

    /// Cycle to the previous panel in the focused pane.
    fn prev_panel(&mut self) {
        if let Some(ws) = self.workspace_mgr.active_mut() {
            ws.prev_panel();
        }
        self.rebuild_active_splits();
        self.focus_active_terminal();
    }

    /// Apply a new UI text size to sidebar/panels via dynamic CSS.
    fn set_ui_font_size(&mut self, size: f64) {
        self.current_ui_font_size = size;
        // Scale all UI text proportionally. The base design uses 14px as the
        // reference; compute a ratio so all font-size declarations scale together.
        let r = size / 14.0;
        let s = |base: f64| -> f64 { (base * r).round().max(8.0) };
        let css = format!(
            r#"
.sidebar, .status-bar, .settings-panel, .audit-panel,
.notification-panel, .token-panel, .git-diff-panel, .tab-bar {{
    font-size: {size}px;
}}
.workspace-title {{ font-size: {wt}px; }}
.workspace-cwd, .workspace-git, .workspace-cost,
.workspace-agent-active, .workspace-agent-stalled {{ font-size: {ws}px; }}
.tab-title {{ font-size: {tab}px; }}
.status-bar label {{ font-size: {sb}px; }}
.notification-title {{ font-size: {nt}px; }}
.notification-body, .audit-description, .audit-event-type {{ font-size: {nb}px; }}
.notification-time {{ font-size: {ntm}px; }}
.audit-severity, .audit-filter-btn {{ font-size: {af}px; }}
.token-detail-label, .token-detail-value {{ font-size: {td}px; }}
.token-total {{ font-size: {tt}px; }}
.token-cost-large {{ font-size: {tc}px; }}
.settings-panel label {{ font-size: {sp}px; }}
.settings-hint {{ font-size: {sh}px; }}
.cost-display {{ font-size: {cd}px; }}
.git-diff-path {{ font-size: {gp}px; }}
.git-diff-line {{ font-size: {gl}px; }}
.git-diff-status, .git-diff-line-count, .git-diff-hunk-header {{ font-size: {gs}px; }}
"#,
            wt = s(15.0), ws = s(13.0), tab = s(15.0), sb = s(14.0),
            nt = s(15.0), nb = s(14.0), ntm = s(13.0), af = s(13.0),
            td = s(14.0), tt = s(15.0), tc = s(30.0), sp = s(15.0),
            sh = s(13.0), cd = s(14.0), gp = s(15.0), gl = s(14.0),
            gs = s(13.0),
        );
        self.ui_css_provider.load_from_string(&css);
    }

    /// Navigate a browser panel to a new URL.
    pub(crate) fn browser_navigate(&self, panel_id: PanelId, url: &str) -> bool {
        if let Some(bp) = self.browser_panels.get(&panel_id) {
            use thane_browser::traits::BrowserSurface;
            bp.surface().navigate(url);
            true
        } else {
            false
        }
    }

    /// Get a reference to a browser panel by ID (for RPC handler).
    pub(crate) fn browser_panel(&self, panel_id: &PanelId) -> Option<&BrowserPanel> {
        self.browser_panels.get(panel_id)
    }

    /// Get the focused browser panel in the active workspace, if any.
    pub(crate) fn focused_browser_panel(&self) -> Option<&BrowserPanel> {
        let ws = self.workspace_mgr.active()?;
        let panel = ws.focused_panel()?;
        self.browser_panels.get(&panel.id)
    }

    /// Get a reference to a terminal panel by ID (for RPC handler).
    pub(crate) fn terminal_panel(&self, panel_id: &PanelId) -> Option<&TerminalPanel> {
        self.terminal_panels.get(panel_id)
    }

    /// Get the focused terminal panel in the active workspace, if any.
    pub(crate) fn focused_terminal_panel(&self) -> Option<&TerminalPanel> {
        let ws = self.workspace_mgr.active()?;
        let panel = ws.focused_panel()?;
        self.terminal_panels.get(&panel.id)
    }

    /// Close the focused pane in the active workspace.
    pub(crate) fn close_focused_pane(&mut self) {
        let ws = match self.workspace_mgr.active_mut() {
            Some(ws) => ws,
            None => return,
        };

        if ws.pane_count() <= 1 {
            // Don't close the last pane — close the workspace instead.
            return;
        }

        let focused = ws.focused_pane_id;

        // Collect panel IDs from the pane.
        let panel_ids: Vec<PanelId> = ws
            .split_tree
            .find_pane(focused)
            .map(|leaf| {
                if let thane_core::pane::SplitTree::Leaf { panel_ids, .. } = leaf {
                    panel_ids.clone()
                } else {
                    vec![]
                }
            })
            .unwrap_or_default();

        if ws.close_pane(focused).is_ok() {
            for pid in panel_ids {
                self.terminal_panels.remove(&pid);
                self.browser_panels.remove(&pid);
                self.panel_workspace_map.remove(&pid);
                self.last_output_times.remove(&pid);
            }
            self.zoomed_pane = None;
            self.rebuild_active_splits();
            self.focus_active_terminal();
        }
    }

    /// Focus next pane.
    pub(crate) fn focus_next_pane(&mut self) {
        if let Some(ws) = self.workspace_mgr.active_mut() {
            ws.focus_next_pane();
        }
        self.rebuild_active_splits();
        self.focus_active_terminal();
    }

    /// Focus previous pane.
    pub(crate) fn focus_prev_pane(&mut self) {
        if let Some(ws) = self.workspace_mgr.active_mut() {
            ws.focus_prev_pane();
        }
        self.rebuild_active_splits();
        self.focus_active_terminal();
    }

    /// Toggle pane zoom (fullscreen a single pane within the workspace).
    pub(crate) fn toggle_pane_zoom(&mut self) {
        if let Some(ws) = self.workspace_mgr.active() {
            if ws.pane_count() <= 1 {
                return;
            }
            let ws_id = ws.id;
            let focused = ws.focused_pane_id;

            if self.zoomed_pane == Some((ws_id, focused)) {
                // Unzoom: rebuild normally.
                self.zoomed_pane = None;
                self.rebuild_active_splits();
            } else {
                // Zoom: show only the focused pane's panel.
                self.zoomed_pane = Some((ws_id, focused));
                if let Some(container) = self.split_containers.get(&ws_id)
                    && let Some(panel) = ws.focused_panel()
                        && let Some(tp) = self.terminal_panels.get(&panel.id) {
                            // Remove from current parent.
                            let widget = tp.widget();
                            if let Some(parent) = widget.parent() {
                                if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
                                    if paned.start_child().as_ref()
                                        == Some(widget.upcast_ref())
                                    {
                                        paned.set_start_child(gtk4::Widget::NONE);
                                    } else {
                                        paned.set_end_child(gtk4::Widget::NONE);
                                    }
                                } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
                                    bx.remove(widget);
                                }
                            }

                            // Clear container and show only the zoomed pane.
                            let root = container.widget();
                            while let Some(child) = root.first_child() {
                                root.remove(&child);
                            }
                            widget.set_hexpand(true);
                            widget.set_vexpand(true);
                            widget.remove_css_class("pane-unfocused");
                            root.append(widget);
                        }
            }
            self.focus_active_terminal();
        }
    }

    /// Default CWD for new workspaces (uses home directory).
    pub(crate) fn default_cwd(&self) -> String {
        dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string())
    }

    /// Check if the config file has changed and reload if needed.
    fn check_config_reload(&mut self) {
        let config = Config::load_default();
        let source_path = match &config.source_path {
            Some(p) => p.clone(),
            None => return,
        };

        let current_mtime = std::fs::metadata(&source_path)
            .ok()
            .and_then(|m| m.modified().ok());

        if current_mtime == self.config_mtime {
            return;
        }

        self.config_mtime = current_mtime;
        tracing::info!("Config file changed, reloading: {}", source_path.display());

        // Apply updated font settings to all terminal panels.
        let font_desc = format!("{} {}", config.font_family(), config.font_size());
        let pango_font = gtk4::pango::FontDescription::from_string(&font_desc);
        for tp in self.terminal_panels.values() {
            use vte4::prelude::*;
            tp.surface().vte_terminal().set_font(Some(&pango_font));
        }

        // Update engine defaults for new terminals.
        self.engine.set_font(font_desc);
        self.engine.set_scrollback_lines(config.scrollback_limit());
    }

    /// Save the current config to disk and update the mtime to prevent
    /// the hot-reload timer from immediately overwriting it.
    fn save_config(&mut self) {
        if let Err(e) = self.config.save() {
            tracing::error!("Failed to save config: {e}");
            return;
        }
        // Update mtime so check_config_reload doesn't re-apply immediately.
        if let Some(config_dir) = dirs::config_dir() {
            let path = config_dir.join("thane").join("config");
            self.config_mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
        }
    }

    /// Refresh workspace metadata (git branch, ports, cost) for all workspaces.
    fn refresh_workspace_metadata(&mut self) {
        // Poll live CWDs from terminal surfaces (reads /proc/{pid}/cwd).
        // This catches CWD changes even when the shell doesn't emit OSC 7.
        let live_cwds: Vec<(PanelId, String)> = self
            .terminal_panels
            .iter()
            .filter_map(|(pid, tp)| {
                tp.surface().cwd().map(|cwd| (*pid, cwd))
            })
            .collect();

        // Update stored panel locations with live CWDs.
        for (pid, live_cwd) in &live_cwds {
            if let Some(&ws_id) = self.panel_workspace_map.get(pid)
                && let Some(ws) = self.workspace_mgr.get_mut(ws_id)
                && let Some(panel) = ws.panels.get_mut(pid)
                && panel.location != *live_cwd
            {
                panel.location = live_cwd.clone();
            }
        }

        // Also update the workspace CWD to match the focused panel's CWD.
        let ws_focused: Vec<(Uuid, PanelId)> = self
            .workspace_mgr
            .list()
            .iter()
            .filter_map(|ws| ws.focused_panel().map(|p| (ws.id, p.id)))
            .collect();
        for (ws_id, focused_id) in ws_focused {
            if let Some(ws) = self.workspace_mgr.get_mut(ws_id)
                && let Some(panel) = ws.panels.get(&focused_id)
                && panel.panel_type == PanelType::Terminal
                && ws.cwd != panel.location
            {
                ws.cwd = panel.location.clone();
            }
        }

        // Clear stale agent entries — they'll be repopulated from live detection below.
        self.panel_agents.clear();

        // Collect workspace IDs, CWDs, and terminal panel locations to avoid borrow conflict.
        #[allow(clippy::type_complexity)]
        let ws_info: Vec<(Uuid, String, Vec<(PanelId, String)>)> = self
            .workspace_mgr
            .list()
            .iter()
            .map(|ws| {
                let terminal_locs: Vec<(PanelId, String)> = ws
                    .panels
                    .iter()
                    .filter(|(_, p)| p.panel_type == PanelType::Terminal)
                    .map(|(id, p)| (*id, p.location.clone()))
                    .collect();
                (ws.id, ws.cwd.clone(), terminal_locs)
            })
            .collect();

        for (ws_id, cwd, terminal_locs) in ws_info {
            // Detect git info from workspace CWD (for backward compat).
            let git_info = GitInfo::detect(std::path::Path::new(&cwd));

            // Per-panel git detection.
            let mut panel_locations = std::collections::HashMap::new();
            for (pid, panel_cwd) in &terminal_locs {
                let panel_git = GitInfo::detect(std::path::Path::new(panel_cwd));
                panel_locations.insert(
                    *pid,
                    thane_core::sidebar::PanelLocationInfo {
                        cwd: panel_cwd.clone(),
                        git_branch: panel_git.branch,
                        git_dirty: panel_git.dirty,
                    },
                );
            }

            // Scan for Claude Code token usage for this project with incremental caching.
            // Use exact CWD matching (no ancestor walking) so each workspace only
            // reflects cost from its own terminal CWDs.
            let since_dt = self.started_at.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
            });
            let mut ws_cwds = std::collections::HashSet::new();
            ws_cwds.insert(cwd.clone());
            for (_, panel_cwd) in &terminal_locs {
                ws_cwds.insert(panel_cwd.clone());
            }
            let mut cost_summary = thane_core::cost_tracker::ProjectCostSummary::default();
            for panel_cwd in &ws_cwds {
                let sub = self.cost_cache.for_project_exact(panel_cwd, since_dt);
                cost_summary.merge(&sub);
            }

            // Detect agent activity via child process inspection.
            let (agent_status, ws_panel_agents) = detect_agent_status_for_workspace(
                ws_id,
                &self.terminal_panels,
                &self.panel_workspace_map,
                &self.last_output_times,
            );
            // Update per-panel agent tracking for audit attribution.
            for (pid, name) in &ws_panel_agents {
                self.panel_agents.insert(*pid, name.clone());
            }

            // Collect PIDs belonging to this workspace's terminals for port scanning.
            let ws_pids = collect_workspace_pids(
                ws_id,
                &self.terminal_panels,
                &self.panel_workspace_map,
            );
            let ws_ports = port_scanner::scan_listening_ports(&ws_pids);

            // Scan for Claude Code interactive prompts from JSONL session files.
            let prompts = thane_core::prompt_scanner::scan_project_prompts(&cwd);
            // Track the last prompt text for sidebar display.
            let last_prompt_text = prompts.last().map(|p| p.text.clone());
            for prompt in prompts {
                if prompt.uuid.is_empty() || !self.seen_prompt_uuids.insert(prompt.uuid.clone()) {
                    continue;
                }
                // Char-safe truncation for the description.
                let short: String = prompt.text.chars().take(100).collect();
                let description = format!("Claude prompt: {short}");
                self.audit_log.log(
                    ws_id,
                    None,
                    AuditEventType::UserPrompt,
                    AuditSeverity::Info,
                    &description,
                    serde_json::json!({
                        "prompt": prompt.text,
                        "session_id": prompt.session_id,
                        "timestamp": prompt.timestamp,
                        "uuid": prompt.uuid,
                    }),
                );
            }

            // Update sidebar metadata for this workspace.
            if let Some(ws) = self.workspace_mgr.get_mut(ws_id) {
                ws.sidebar.git_branch = git_info.branch;
                ws.sidebar.git_dirty = git_info.dirty;
                ws.sidebar.ports = ws_ports;
                ws.sidebar.panel_locations = panel_locations;
                if cost_summary.current_session.estimated_cost_usd > 0.0
                    || cost_summary.all_time.estimated_cost_usd > 0.0
                {
                    ws.sidebar.session_cost = Some(cost_summary.current_session.estimated_cost_usd);
                    ws.sidebar.all_time_cost = Some(cost_summary.all_time.estimated_cost_usd);
                }
                ws.sidebar.agent_status = agent_status;
                if last_prompt_text.is_some() {
                    ws.sidebar.last_prompt = last_prompt_text;
                }
            }
        }

        // Add agent queue costs to the active workspace's sidebar totals.
        // Queue tasks run headlessly and aren't tracked in per-workspace JSONL files.
        let (queue_session_cost, queue_alltime_cost) = self.queue_costs();
        if queue_session_cost > 0.0 || queue_alltime_cost > 0.0 {
            if let Some(ws) = self.workspace_mgr.active_mut() {
                let session = ws.sidebar.session_cost.unwrap_or(0.0) + queue_session_cost;
                let alltime = ws.sidebar.all_time_cost.unwrap_or(0.0) + queue_alltime_cost;
                ws.sidebar.session_cost = Some(session);
                ws.sidebar.all_time_cost = Some(alltime);
            }
        }

        self.refresh_sidebar();
        self.refresh_status_bar();
        if self.token_panel_visible {
            self.refresh_token_panel();
        }
    }

    /// Sync GTK Paned divider positions back into each workspace's split tree
    /// so that user-resized panes are captured in the next snapshot.
    fn sync_divider_positions(&mut self) {
        let ws_ids: Vec<Uuid> = self.workspace_mgr.list().iter().map(|ws| ws.id).collect();
        for ws_id in ws_ids {
            if let Some(container) = self.split_containers.get(&ws_id) {
                let positions = container.collect_divider_positions();
                if !positions.is_empty()
                    && let Some(ws) = self.workspace_mgr.get_mut(ws_id)
                {
                    ws.split_tree.update_divider_positions(&positions);
                }
            }
        }
    }

    /// Capture a snapshot of the current application state for persistence.
    fn capture_snapshot(&mut self, policy: &PersistPolicy) -> AppSnapshot {
        // Sync divider positions from live GTK widgets before capturing.
        self.sync_divider_positions();

        let active_id = self.workspace_mgr.active().map(|ws| ws.id);
        let mut workspace_snapshots = Vec::new();

        for ws in self.workspace_mgr.list() {
            let panels: Vec<PanelSnapshot> = ws
                .panels
                .values()
                .map(|panel| match panel.panel_type {
                    PanelType::Terminal => {
                        let scrollback = self
                            .terminal_panels
                            .get(&panel.id)
                            .map(|tp| {
                                let text = tp.surface().get_text();
                                policy.truncate_scrollback(&text)
                            });
                        PanelSnapshot::from_terminal(panel.clone(), scrollback)
                    }
                    PanelType::Browser => {
                        PanelSnapshot::from_browser(panel.clone(), panel.location.clone())
                    }
                })
                .collect();

            workspace_snapshots.push(WorkspaceSnapshot {
                id: ws.id,
                title: ws.title.clone(),
                cwd: ws.cwd.clone(),
                split_tree: ws.split_tree.clone(),
                panels,
                focused_pane_id: Some(ws.focused_pane_id),
                tag: ws.tag.clone(),
                sandbox_policy: ws.sandbox_policy.clone(),
            });
        }

        let mut snapshot = AppSnapshot::new(workspace_snapshots, active_id);
        snapshot.sidebar_collapsed = Some(self.sidebar.is_collapsed());
        snapshot
    }

    /// Push a notification to a workspace's store and update UI.
    fn push_notification(&mut self, ws_id: Uuid, title: &str, body: &str, panel_id: PanelId) {
        let is_active_workspace = self
            .workspace_mgr
            .active()
            .is_some_and(|ws| ws.id == ws_id);

        // Push to the workspace's notification store.
        if let Some(ws) = self.workspace_mgr.get_mut(ws_id) {
            let notification = Notification::new(panel_id, title, body);
            ws.notifications.push(notification);
        }

        // Refresh sidebar to show updated badge / preview.
        self.refresh_sidebar();

        // Update notification panel if visible and this is the active workspace.
        if self.notification_panel_visible && is_active_workspace {
            self.refresh_notification_panel();
        }

        // Send desktop notification if the workspace is not active.
        if !is_active_workspace
            && let Err(e) =
                self.notifier
                    .send_notification(title, body, NotifyUrgency::Normal)
        {
            tracing::warn!("Failed to send desktop notification: {e}");
        }
    }

    /// Refresh the notification panel with the active workspace's notifications.
    fn refresh_notification_panel(&self) {
        if let Some(ws) = self.workspace_mgr.active() {
            self.notification_panel
                .set_notifications(ws.notifications.all());
        }
    }

    /// Toggle notification panel visibility.
    fn toggle_notification_panel(&mut self) {
        if !self.notification_panel_visible {
            self.close_all_right_panels();
        }
        self.notification_panel_visible = !self.notification_panel_visible;
        self.notification_panel
            .widget()
            .set_visible(self.notification_panel_visible);

        if self.notification_panel_visible {
            self.refresh_notification_panel();
            // Mark notifications as read when panel is opened.
            if let Some(ws) = self.workspace_mgr.active_mut() {
                ws.notifications.mark_all_read();
            }
            self.refresh_sidebar();
        }
    }

    /// Toggle sidebar between expanded and collapsed modes.
    fn toggle_sidebar(&mut self) {
        self.sidebar.toggle_collapse();
    }

    /// Toggle audit panel visibility.
    fn toggle_audit_panel(&mut self) {
        if !self.audit_panel_visible {
            self.close_all_right_panels();
        }
        self.audit_panel_visible = !self.audit_panel_visible;
        self.audit_panel
            .widget()
            .set_visible(self.audit_panel_visible);

        if self.audit_panel_visible {
            self.refresh_audit_panel();
        }
    }

    /// Refresh the audit panel with current events.
    fn refresh_audit_panel(&self) {
        self.audit_panel.set_events(self.audit_log.all());
    }

    fn toggle_git_diff_panel(&mut self) {
        self.git_diff_panel_visible = !self.git_diff_panel_visible;
        self.git_diff_panel
            .widget()
            .set_visible(self.git_diff_panel_visible);

        if self.git_diff_panel_visible {
            // Default to workspace CWD when toggling via Ctrl+Shift+G.
            self.git_diff_cwd = self.workspace_mgr.active().map(|ws| ws.cwd.clone());
            self.refresh_git_diff_panel();
        } else {
            self.git_diff_cwd = None;
        }
    }

    /// Open the git diff panel for a specific directory path.
    fn open_git_diff_for_path(&mut self, cwd: String) {
        self.git_diff_cwd = Some(cwd);
        self.git_diff_panel_visible = true;
        self.git_diff_panel.widget().set_visible(true);
        self.refresh_git_diff_panel();
    }

    fn refresh_git_diff_panel(&self) {
        let cwd_str = if let Some(ref cwd) = self.git_diff_cwd {
            cwd.clone()
        } else if let Some(ws) = self.workspace_mgr.active() {
            ws.cwd.clone()
        } else {
            return;
        };
        let cwd = std::path::Path::new(&cwd_str);
        let diff_files = thane_core::git::get_diff(cwd);
        self.git_diff_panel.set_subtitle(Some(&shorten_path(&cwd_str)));
        self.git_diff_panel.refresh(&diff_files);
    }

    /// Toggle settings panel visibility.
    fn toggle_settings_panel(&mut self) {
        if !self.settings_panel_visible {
            self.close_all_right_panels();
        }
        self.settings_panel_visible = !self.settings_panel_visible;
        self.settings_panel
            .widget()
            .set_visible(self.settings_panel_visible);
    }

    /// Toggle token usage panel visibility.
    fn toggle_token_panel(&mut self) {
        if !self.token_panel_visible {
            self.close_all_right_panels();
        }
        self.token_panel_visible = !self.token_panel_visible;
        self.token_panel
            .widget()
            .set_visible(self.token_panel_visible);

        if self.token_panel_visible {
            self.refresh_token_panel();
        }
    }

    /// Toggle help panel visibility.
    fn toggle_help_panel(&mut self) {
        if !self.help_panel_visible {
            self.close_all_right_panels();
        }
        self.help_panel_visible = !self.help_panel_visible;
        self.help_panel
            .widget()
            .set_visible(self.help_panel_visible);
    }

    /// Toggle agent queue panel visibility.
    fn toggle_agent_queue_panel(&mut self) {
        if !self.agent_queue_panel_visible {
            self.close_all_right_panels();
        }
        self.agent_queue_panel_visible = !self.agent_queue_panel_visible;
        self.agent_queue_panel
            .widget()
            .set_visible(self.agent_queue_panel_visible);

        if self.agent_queue_panel_visible {
            self.agent_queue_panel.set_claude_missing(!self.has_claude);
            self.agent_queue_panel.update(&self.agent_queue);
        }
    }

    /// Toggle plans panel visibility.
    fn toggle_plans_panel(&mut self) {
        if !self.plans_panel_visible {
            self.close_all_right_panels();
        }
        self.plans_panel_visible = !self.plans_panel_visible;
        self.plans_panel
            .widget()
            .set_visible(self.plans_panel_visible);

        if self.plans_panel_visible {
            self.plans_panel.set_claude_missing(!self.has_claude);
            let entries = self.queue_history_store.load().unwrap_or_default();
            self.plans_panel.update(&entries);
        }
    }

    /// Toggle sandbox configuration panel visibility.
    fn toggle_sandbox_panel(&mut self) {
        if !self.sandbox_panel_visible {
            self.close_all_right_panels();
        }
        self.sandbox_panel_visible = !self.sandbox_panel_visible;
        self.sandbox_panel
            .widget()
            .set_visible(self.sandbox_panel_visible);

        if self.sandbox_panel_visible
            && let Some(ws) = self.workspace_mgr.active()
        {
            self.sandbox_panel.update(&ws.sandbox_policy);
        }
    }

    /// Refresh the token panel with data from the active workspace.
    fn refresh_token_panel(&mut self) {
        if let Some(cwd) = self.workspace_mgr.active().map(|ws| ws.cwd.clone()) {
            let since_dt = self.started_at.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
            });
            let mut summary = self.cost_cache.for_project_detailed(&cwd, since_dt);

            // Add agent queue costs (not tracked in JSONL files since they run headlessly).
            let (queue_session_cost, queue_alltime_cost) = self.queue_costs();
            summary.current_session.estimated_cost_usd += queue_session_cost;
            summary.all_time.estimated_cost_usd += queue_alltime_cost;

            // Fetch OAuth usage data if stale (>60s) or not yet fetched.
            // This is a blocking HTTP call, but it's fast (<200ms) and runs
            // infrequently. We cache the result to avoid hammering the API.
            let needs_refresh = self.limit_info_fetched_at
                .map(|t| t.elapsed().as_secs() >= 60)
                .unwrap_or(true);

            if needs_refresh {
                let plan = Plan::detect(self.config.plan());
                // Try reading the subscription type from credentials for more accurate plan detection.
                let plan = if let Some(sub_type) = read_subscription_type() {
                    Plan::from_str_loose(&sub_type)
                } else {
                    plan
                };

                if let Some(token) = read_oauth_token() {
                    if let Some(response) = fetch_oauth_usage(&token) {
                        self.cached_limit_info = Some(TokenLimitInfo::from_oauth(plan, &response));
                    } else {
                        // API call failed — keep stale data if we have it, otherwise show plan without usage.
                        if self.cached_limit_info.is_none() {
                            self.cached_limit_info = Some(TokenLimitInfo {
                                plan,
                                five_hour: None,
                                seven_day: None,
                                has_caps: plan.has_caps(),
                            });
                        }
                    }
                } else {
                    // No OAuth token — show plan without usage data.
                    self.cached_limit_info = Some(TokenLimitInfo {
                        plan,
                        five_hour: None,
                        seven_day: None,
                        has_caps: plan.has_caps(),
                    });
                }
                self.limit_info_fetched_at = Some(std::time::Instant::now());
            }

            self.token_panel.update(&summary, self.cached_limit_info.as_ref());

            // Show enterprise cost setting only when plan is Enterprise.
            let is_enterprise = self
                .cached_limit_info
                .as_ref()
                .is_some_and(|info| info.plan == thane_core::cost_tracker::Plan::Enterprise);
            self.settings_panel.set_enterprise_cost_visible(is_enterprise);
        }
    }

    /// Compute agent queue costs: (current_session, all_time).
    ///
    /// Current session = completed entries still in memory (this run of thane).
    /// All time = everything in the persisted queue history store.
    fn queue_costs(&mut self) -> (f64, f64) {
        // Current session: in-memory completed entries.
        let session_cost: f64 = self
            .agent_queue
            .completed_entries()
            .iter()
            .map(|e| e.tokens_used.estimated_cost_usd)
            .sum();

        // All time: cached summary avoids re-reading the full history file.
        let alltime_cost = self
            .queue_history_store
            .summary()
            .map(|s| s.alltime_cost)
            .unwrap_or(0.0);

        (session_cost, alltime_cost)
    }

    /// Close all right-side panels (exclusive panel behavior).
    fn close_all_right_panels(&mut self) {
        if self.notification_panel_visible {
            self.notification_panel_visible = false;
            self.notification_panel.widget().set_visible(false);
        }
        if self.audit_panel_visible {
            self.audit_panel_visible = false;
            self.audit_panel.widget().set_visible(false);
        }
        if self.settings_panel_visible {
            self.settings_panel_visible = false;
            self.settings_panel.widget().set_visible(false);
        }
        if self.token_panel_visible {
            self.token_panel_visible = false;
            self.token_panel.widget().set_visible(false);
        }
        if self.help_panel_visible {
            self.help_panel_visible = false;
            self.help_panel.widget().set_visible(false);
        }
        if self.agent_queue_panel_visible {
            self.agent_queue_panel_visible = false;
            self.agent_queue_panel.widget().set_visible(false);
        }
        if self.plans_panel_visible {
            self.plans_panel_visible = false;
            self.plans_panel.widget().set_visible(false);
        }
        if self.sandbox_panel_visible {
            self.sandbox_panel_visible = false;
            self.sandbox_panel.widget().set_visible(false);
        }
    }

    /// Change the terminal font size by delta, clamped to 6..72.
    fn zoom_font_size(&mut self, delta: f64) {
        let new_size = (self.current_font_size + delta).clamp(6.0, 72.0);
        self.set_terminal_font_size(new_size);
    }

    /// Reset font size to the base config value.
    fn reset_font_size(&mut self) {
        self.set_terminal_font_size(self.base_font_size);
    }

    /// Apply a font size to all terminals.
    fn set_terminal_font_size(&mut self, size: f64) {
        self.current_font_size = size;
        let font_desc = format!("{} {}", self.current_font_family, size);
        let pango_font = gtk4::pango::FontDescription::from_string(&font_desc);
        for tp in self.terminal_panels.values() {
            use vte4::prelude::*;
            let vte = tp.surface().vte_terminal();
            vte.set_font(Some(&pango_font));
            // Queue resize so the layout recalculates in one pass,
            // preventing the terminal width from bouncing.
            vte.queue_resize();
        }
        self.engine.set_font(font_desc);
        self.status_bar.set_font_size(size);
        if self.settings_panel_visible {
            self.settings_panel.set_font_size(size);
        }
    }

    /// Apply a new font family to all terminals.
    fn set_terminal_font_family(&mut self, family: String) {
        self.current_font_family = family;
        let font_desc = format!("{} {}", self.current_font_family, self.current_font_size);
        let pango_font = gtk4::pango::FontDescription::from_string(&font_desc);
        for tp in self.terminal_panels.values() {
            use vte4::prelude::*;
            let vte = tp.surface().vte_terminal();
            vte.set_font(Some(&pango_font));
            vte.queue_resize();
        }
        self.engine.set_font(font_desc);
    }

    /// Update status bar with current state.
    fn refresh_status_bar(&mut self) {
        // Agent status: find the most significant status across all workspaces.
        let agent_status = self
            .workspace_mgr
            .list()
            .iter()
            .map(|ws| &ws.sidebar.agent_status)
            .find(|s| **s == thane_core::sidebar::AgentStatus::Active)
            .or_else(|| {
                self.workspace_mgr
                    .list()
                    .iter()
                    .map(|ws| &ws.sidebar.agent_status)
                    .find(|s| **s == thane_core::sidebar::AgentStatus::Stalled)
            })
            .cloned()
            .unwrap_or(thane_core::sidebar::AgentStatus::Inactive);
        // Collect unique agent names across all panels for the status bar display.
        let agent_names: Vec<String> = self
            .panel_agents
            .values()
            .cloned()
            .collect::<std::collections::BTreeSet<String>>()
            .into_iter()
            .collect();
        self.status_bar.set_agent_status(&agent_status, &agent_names);

        // Cost: sum across all workspaces, deduplicating by CWD so that
        // multiple workspaces pointing at the same project don't double-count.
        // Also include agent queue costs (headless tasks not tracked per-workspace).
        let use_alltime = self.config.cost_display_scope() == "all-time";
        let mut seen_cwds = std::collections::HashSet::new();
        let workspace_cost: f64 = self
            .workspace_mgr
            .list()
            .iter()
            .filter(|ws| seen_cwds.insert(ws.cwd.clone()))
            .filter_map(|ws| {
                if use_alltime { ws.sidebar.all_time_cost } else { ws.sidebar.session_cost }
            })
            .sum();
        let (queue_session_cost, queue_alltime_cost) = self.queue_costs();
        let queue_cost = if use_alltime { queue_alltime_cost } else { queue_session_cost };
        let (display_mode, util_pct, sub_cost) = self
            .cached_limit_info
            .as_ref()
            .map(|info| {
                (
                    info.display_mode_with_override(self.config.enterprise_monthly_cost()),
                    info.primary_utilization(),
                    info.derived_subscription_cost_with_override(self.config.enterprise_monthly_cost()),
                )
            })
            .unwrap_or((thane_core::cost_tracker::CostDisplayMode::Dollar, None, None));
        self.status_bar.set_cost_display(
            display_mode,
            workspace_cost + queue_cost,
            util_pct,
            sub_cost,
        );

        // Audit event count.
        self.status_bar.set_audit_count(self.audit_log.all().len());

        // Font size.
        self.status_bar.set_font_size(self.current_font_size);

        // Agent queue count.
        self.status_bar
            .update_agent_queue(
                self.agent_queue.queued_count(),
                self.agent_queue.running_count(),
                self.agent_queue.token_limit_paused,
            );

        // Plans count badge.
        let plans_count = self.queue_history_store.summary().map(|s| s.entry_count).unwrap_or(0);
        self.status_bar.set_plans_count(plans_count);

        // Command block: show last completed block from the focused panel's tracker.
        if let Some(ws) = self.workspace_mgr.active()
            && let Some(panel) = ws.focused_panel()
            && let Some(tracker) = self.block_trackers.get(&panel.id)
        {
            if let Some(block) = tracker.blocks().last() {
                let duration = match (block.started_at, block.finished_at) {
                    (Some(start), Some(end)) => {
                        let secs = (end - start).num_seconds();
                        Some(format!("{secs}s"))
                    }
                    _ => None,
                };
                self.status_bar
                    .set_last_command(block.exit_code, duration.as_deref());
            } else {
                self.status_bar.set_last_command(None, None);
            }
        } else {
            self.status_bar.set_last_command(None, None);
        }
    }

    /// Try to execute the next entry in the agent queue.
    ///
    /// Spawns a headless Claude Code process (no workspace/terminal created).
    /// Returns the entry ID if one was started.
    fn execute_next_queue_entry(&mut self) -> Option<Uuid> {
        // Don't start a new entry if one is already running.
        if self.running_queue_entry.is_some() {
            return None;
        }

        // Check if token limit has reset.
        self.agent_queue.check_token_limit_reset();

        // Get the next runnable entry.
        let entry = self.agent_queue.next_runnable()?.clone();

        // Mark it as running.
        self.agent_queue.start(entry.id);

        // CWD: per-task subdirectory under the configured working dir.
        let base_dir = self.config.queue_working_dir();
        let task_dir = queue_executor::task_dir(&base_dir, entry.id);
        let cwd = task_dir.to_string_lossy().to_string();

        // Save output to ~/thane/plans/<uuid>/output.log.
        let plans_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("thane")
            .join("plans")
            .join(entry.id.to_string());
        let _ = std::fs::create_dir_all(&plans_dir);
        let log_path = plans_dir.join("output.log");

        // Spawn headless process with queue sandbox if enabled.
        let sandbox = if self.agent_queue.sandbox_policy().enabled {
            Some(self.agent_queue.sandbox_policy())
        } else {
            None
        };
        match queue_executor::spawn_headless(
            &entry.content,
            &cwd,
            &log_path.to_string_lossy(),
            sandbox,
        ) {
            Ok(child) => {
                tracing::info!(
                    "Started headless queue entry {} in {}",
                    entry.id,
                    cwd,
                );
                let ws_id = entry.workspace_id.unwrap_or(Uuid::nil());
                let entry_id = entry.id;
                self.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::QueueTaskStarted,
                    AuditSeverity::Info,
                    format!("Queue task started: {entry_id}"),
                    serde_json::json!({
                        "entry_id": entry_id.to_string(),
                        "cwd": cwd,
                        "log_path": log_path.to_string_lossy(),
                    }),
                );
                self.running_queue_entry = Some((entry.id, child));
                Some(entry_id)
            }
            Err(e) => {
                tracing::error!("Failed to spawn headless process for entry {}: {e}", entry.id);
                let ws_id = entry.workspace_id.unwrap_or(Uuid::nil());
                self.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::QueueTaskFailed,
                    AuditSeverity::Warning,
                    format!("Queue task spawn failed: {}", entry.id),
                    serde_json::json!({
                        "entry_id": entry.id.to_string(),
                        "error": format!("{e}"),
                    }),
                );
                self.agent_queue.fail(entry.id, format!("Spawn failed: {e}"));
                None
            }
        }
    }

    /// Poll the running headless child process for completion.
    ///
    /// Called from a periodic timer. Returns true if the entry finished.
    /// Poll the running queue entry. Returns (finished, optional_critical_alert_description).
    fn poll_running_queue_entry(&mut self) -> (bool, Option<String>) {
        let Some((entry_id, ref mut child)) = self.running_queue_entry else {
            return (false, None);
        };

        // Look up workspace_id before any mutations.
        let ws_id = self.agent_queue.get(entry_id)
            .and_then(|e| e.workspace_id)
            .unwrap_or(Uuid::nil());

        match child.try_wait() {
            Ok(Some(status)) => {
                let success = status.success();
                let exit_code = status.code().unwrap_or(-1);

                if success {
                    self.agent_queue.complete(entry_id);
                    tracing::info!("Queue entry {} completed successfully", entry_id);
                } else {
                    self.agent_queue.fail(entry_id, format!("Process exited with code {exit_code}"));
                    tracing::info!("Queue entry {} failed with code {}", entry_id, exit_code);
                }

                // Parse token usage from the JSON output log.
                let log_path = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .join("thane")
                    .join("plans")
                    .join(entry_id.to_string())
                    .join("output.log");
                if let Some(usage) = queue_executor::parse_usage_from_log(
                    &log_path.to_string_lossy(),
                ) {
                    self.agent_queue.update_tokens(entry_id, usage);
                }

                // Log completion/failure audit event.
                let tokens = self.agent_queue.get(entry_id)
                    .map(|e| &e.tokens_used)
                    .cloned()
                    .unwrap_or_default();
                if success {
                    self.audit_log_mut().log(
                        ws_id,
                        None,
                        AuditEventType::QueueTaskCompleted,
                        AuditSeverity::Info,
                        format!("Queue task completed: {entry_id}"),
                        serde_json::json!({
                            "entry_id": entry_id.to_string(),
                            "exit_code": exit_code,
                            "input_tokens": tokens.input_tokens,
                            "output_tokens": tokens.output_tokens,
                        }),
                    );
                } else {
                    self.audit_log_mut().log(
                        ws_id,
                        None,
                        AuditEventType::QueueTaskFailed,
                        AuditSeverity::Warning,
                        format!("Queue task failed: {entry_id} (exit code {exit_code})"),
                        serde_json::json!({
                            "entry_id": entry_id.to_string(),
                            "exit_code": exit_code,
                        }),
                    );
                }

                // Scan output log for sensitive file references and PII.
                let log_path_str = log_path.to_string_lossy();
                let scan = scan_queue_output_log(&log_path_str);
                let mut critical_alert: Option<String> = None;
                for (path, event_type) in &scan.sensitive_files {
                    let severity = match event_type {
                        AuditEventType::PrivateKeyAccess => AuditSeverity::Critical,
                        _ => AuditSeverity::Alert,
                    };
                    let description = match event_type {
                        AuditEventType::PrivateKeyAccess => {
                            format!("Private key file referenced in queue output: {path}")
                        }
                        _ => format!("Sensitive file referenced in queue output: {path}"),
                    };
                    if severity == AuditSeverity::Critical {
                        critical_alert = Some(description.clone());
                    }
                    self.audit_log_mut().log(
                        ws_id,
                        None,
                        event_type.clone(),
                        severity,
                        &description,
                        serde_json::json!({"path": path, "source": "queue_output_scan", "entry_id": entry_id.to_string()}),
                    );
                }
                if !scan.pii_findings.is_empty() {
                    self.audit_log_mut().log(
                        ws_id,
                        None,
                        AuditEventType::PiiDetected,
                        AuditSeverity::Alert,
                        format!("PII detected in queue output: {}", scan.pii_findings.join(", ")),
                        serde_json::json!({"findings": scan.pii_findings, "source": "queue_output_scan", "entry_id": entry_id.to_string()}),
                    );
                }

                // Persist to history.
                if let Some(entry) = self.agent_queue.get(entry_id) {
                    let entry_clone = entry.clone();
                    if let Err(e) = self.queue_history_store.append(&entry_clone) {
                        tracing::error!("Failed to save queue history: {e}");
                    }
                }

                self.running_queue_entry = None;

                // Reset process_all if queue is empty.
                if self.agent_queue.queued_count() == 0 {
                    self.agent_queue.process_all = false;
                }

                (true, critical_alert)
            }
            Ok(None) => {
                // Still running.
                (false, None)
            }
            Err(e) => {
                tracing::error!("Error polling queue entry {}: {e}", entry_id);
                self.agent_queue.fail(entry_id, format!("Poll error: {e}"));
                self.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::QueueTaskFailed,
                    AuditSeverity::Warning,
                    format!("Queue task poll error: {entry_id}"),
                    serde_json::json!({
                        "entry_id": entry_id.to_string(),
                        "error": format!("{e}"),
                    }),
                );
                self.running_queue_entry = None;
                (true, None)
            }
        }
    }

    /// Handle a leader mode key press. Returns the action taken.
    fn handle_leader_key(&mut self, key: Key) -> LeaderAction {
        self.leader_mode = false;
        self.status_bar.set_leader_mode(false);
        tracing::debug!("Leader key: {:?}", key);

        match key {
            k if k == Key::n => { self.select_next_workspace(); LeaderAction::Consumed }
            k if k == Key::p => { self.select_prev_workspace(); LeaderAction::Consumed }
            k if k == Key::c => {
                let count = self.workspace_mgr.count() + 1;
                let title = format!("Workspace {count}");
                let cwd = self.default_cwd();
                let (ws_id, panel_id) = self.create_workspace(&title, &cwd);
                LeaderAction::WireNotifications(ws_id, panel_id)
            }
            k if k == Key::x => { self.close_active_workspace(); LeaderAction::Consumed }
            k if k == Key::comma => LeaderAction::ShowRenameDialog,
            k if k == Key::_1 => { self.select_workspace(0); LeaderAction::Consumed }
            k if k == Key::_2 => { self.select_workspace(1); LeaderAction::Consumed }
            k if k == Key::_3 => { self.select_workspace(2); LeaderAction::Consumed }
            k if k == Key::_4 => { self.select_workspace(3); LeaderAction::Consumed }
            k if k == Key::_5 => { self.select_workspace(4); LeaderAction::Consumed }
            k if k == Key::_6 => { self.select_workspace(5); LeaderAction::Consumed }
            k if k == Key::_7 => { self.select_workspace(6); LeaderAction::Consumed }
            k if k == Key::_8 => { self.select_workspace(7); LeaderAction::Consumed }
            k if k == Key::_9 => { self.select_workspace(8); LeaderAction::Consumed }
            _ => {
                tracing::debug!("Unknown leader key: {:?}", key);
                LeaderAction::NotConsumed
            }
        }
    }
}

/// Result of handling a leader key.
enum LeaderAction {
    Consumed,
    NotConsumed,
    ShowRenameDialog,
    WireNotifications(Uuid, PanelId),
}

/// Take a screenshot of a widget with clipboard copy and confirmation dialog.
/// Used by both the tab bar button and the right-click context menu.
pub(crate) fn take_screenshot_with_feedback(widget: &gtk4::Widget) {
    if let Some(path) = screenshot_widget(widget) {
        // Copy path to clipboard.
        let display = widget.display();
        display.clipboard().set_text(&path);
        // Show confirmation dialog if we can find the toplevel window.
        if let Some(root) = widget.root()
            && let Some(win) = root.downcast_ref::<gtk4::ApplicationWindow>()
        {
            show_screenshot_dialog(win, &path);
        }
    }
}

/// Build a modal dialog window styled to match the main thane dark theme.
/// Returns (dialog, content_box) — append your widgets to the content_box.
pub(crate) fn styled_dialog(
    parent: &gtk4::ApplicationWindow,
    title: &str,
    width: i32,
    height: i32,
) -> (gtk4::Window, gtk4::Box) {
    let dialog = gtk4::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(width)
        .default_height(height)
        .resizable(false)
        .build();
    dialog.add_css_class("thane-dialog");

    // Custom dark header bar matching the main window.
    let header_bar = gtk4::HeaderBar::new();
    header_bar.add_css_class("thane-header");
    let title_label = gtk4::Label::new(Some(title));
    title_label.add_css_class("thane-header-title");
    header_bar.set_title_widget(Some(&title_label));
    dialog.set_titlebar(Some(&header_bar));

    // Content box with dark background.
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.add_css_class("thane-dialog-content");
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    dialog.set_child(Some(&vbox));

    (dialog, vbox)
}

/// Take a screenshot of a GTK widget and save to /tmp.
/// Returns the file path on success, or None on failure.
fn screenshot_widget(widget: &gtk4::Widget) -> Option<String> {
    use gdk4::prelude::{PaintableExt, TextureExt};

    let width = widget.width();
    let height = widget.height();
    if width <= 0 || height <= 0 {
        tracing::warn!("Cannot screenshot widget with zero size");
        return None;
    }

    let paintable = gtk4::WidgetPaintable::new(Some(widget));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(
        snapshot.upcast_ref::<gdk4::Snapshot>(),
        width as f64,
        height as f64,
    );
    let node = match snapshot.to_node() {
        Some(n) => n,
        None => {
            tracing::warn!("Failed to create render node for screenshot");
            return None;
        }
    };

    let native = match widget.native() {
        Some(n) => n,
        None => {
            tracing::warn!("Widget has no native surface");
            return None;
        }
    };
    let renderer = match native.renderer() {
        Some(r) => r,
        None => {
            tracing::warn!("No renderer available for screenshot");
            return None;
        }
    };

    let bounds = gtk4::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));
    let png_bytes = texture.save_to_png_bytes();

    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let path = format!("/tmp/thane-screenshot-{timestamp}.png");
    match std::fs::write(&path, png_bytes.as_ref()) {
        Ok(()) => {
            tracing::info!("Screenshot saved to {path}");
            Some(path)
        }
        Err(e) => {
            tracing::error!("Failed to save screenshot: {e}");
            None
        }
    }
}

/// Show a modal dialog confirming a screenshot was captured.
fn show_screenshot_dialog(window: &gtk4::ApplicationWindow, path: &str) {
    let (dialog, vbox) = styled_dialog(window, "Screenshot Captured", 400, 140);

    let msg = gtk4::Label::new(Some("Screenshot saved and path copied to clipboard."));
    msg.set_halign(gtk4::Align::Start);
    vbox.append(&msg);

    let path_label = gtk4::Label::new(Some(path));
    path_label.set_halign(gtk4::Align::Start);
    path_label.set_selectable(true);
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    path_label.add_css_class("dim-label");
    vbox.append(&path_label);

    let ok_btn = gtk4::Button::with_label("OK");
    ok_btn.add_css_class("suggested-action");
    ok_btn.set_halign(gtk4::Align::End);
    let dlg = dialog.clone();
    ok_btn.connect_clicked(move |_| dlg.close());
    vbox.append(&ok_btn);

    dialog.present();
    // Grab focus on OK button so the path label text is not selected.
    ok_btn.grab_focus();
}

fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(home_str.as_ref()) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

/// Show a rename dialog for the active workspace.
fn show_rename_dialog(window: &gtk4::ApplicationWindow, state: &Rc<RefCell<AppState>>) {
    let current_title = state
        .borrow()
        .workspace_mgr
        .active()
        .map(|ws| ws.title.clone())
        .unwrap_or_default();

    let (dialog, vbox) = styled_dialog(window, "Rename Workspace", 350, 120);

    let label = gtk4::Label::new(Some("Workspace name:"));
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let entry = gtk4::Entry::new();
    entry.set_text(&current_title);
    entry.select_region(0, -1);
    vbox.append(&entry);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    btn_box.append(&cancel_btn);

    let rename_btn = gtk4::Button::with_label("Rename");
    rename_btn.add_css_class("suggested-action");
    btn_box.append(&rename_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        cancel_btn.connect_clicked(move |_| dlg.close());
    }

    {
        let dlg = dialog.clone();
        let entry_ref = entry.clone();
        let state_ref = state.clone();
        rename_btn.connect_clicked(move |_| {
            let new_title = entry_ref.text().to_string();
            if !new_title.is_empty() {
                let mut s = state_ref.borrow_mut();
                s.workspace_mgr.rename_active(&new_title);
                s.refresh_sidebar();
            }
            dlg.close();
        });
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        entry.connect_activate(move |entry| {
            let new_title = entry.text().to_string();
            if !new_title.is_empty() {
                let mut s = state_ref.borrow_mut();
                s.workspace_mgr.rename_active(&new_title);
                s.refresh_sidebar();
            }
            dlg.close();
        });
    }

    dialog.present();
    entry.grab_focus();
}

/// Show a confirmation dialog before closing a workspace.
fn show_close_workspace_dialog(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    index: usize,
) {
    let ws_title = state
        .borrow()
        .workspace_mgr
        .list()
        .get(index)
        .map(|ws| ws.title.clone())
        .unwrap_or_default();

    let (dialog, vbox) = styled_dialog(window, "Close Workspace", 380, 140);

    let label = gtk4::Label::new(Some(&format!(
        "Close workspace \"{ws_title}\"?\n\nAll terminals in this workspace will be terminated."
    )));
    label.set_wrap(true);
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    btn_box.append(&cancel_btn);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.add_css_class("destructive-action");
    btn_box.append(&close_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        cancel_btn.connect_clicked(move |_| dlg.close());
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        close_btn.connect_clicked(move |_| {
            let mut s = state_ref.borrow_mut();
            // Select the workspace first, then close it.
            s.select_workspace(index);
            s.close_active_workspace();
            dlg.close();
        });
    }

    dialog.present();
}

/// Show a confirmation dialog before closing a panel (tab).
fn show_close_panel_dialog(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    panel_id: PanelId,
) {
    let panel_title = state
        .borrow()
        .workspace_mgr
        .active()
        .and_then(|ws| ws.panels.get(&panel_id))
        .map(|p| p.title.clone())
        .unwrap_or_else(|| "this panel".to_string());

    let (dialog, vbox) = styled_dialog(window, "Close Panel", 380, 120);

    let label = gtk4::Label::new(Some(&format!(
        "Close \"{panel_title}\"?\n\nThe terminal session will be terminated."
    )));
    label.set_wrap(true);
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    btn_box.append(&cancel_btn);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.add_css_class("destructive-action");
    btn_box.append(&close_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        cancel_btn.connect_clicked(move |_| dlg.close());
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        close_btn.connect_clicked(move |_| {
            state_ref.borrow_mut().close_panel_by_id(panel_id);
            dlg.close();
        });
    }

    dialog.present();
}

/// Show a confirmation dialog before closing the focused pane.
fn show_close_pane_dialog(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
) {
    let (dialog, vbox) = styled_dialog(window, "Close Pane", 380, 120);

    let label = gtk4::Label::new(Some(
        "Close this pane?\n\nAll tabs in this pane will be terminated."
    ));
    label.set_wrap(true);
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    btn_box.append(&cancel_btn);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.add_css_class("destructive-action");
    btn_box.append(&close_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        cancel_btn.connect_clicked(move |_| dlg.close());
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        close_btn.connect_clicked(move |_| {
            state_ref.borrow_mut().close_focused_pane();
            dlg.close();
        });
    }

    dialog.present();
}

/// Show a confirmation dialog before removing a sandbox path.
fn show_remove_sandbox_path_dialog(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    path: std::path::PathBuf,
    category: &str,
) {
    let (dialog, vbox) = styled_dialog(window, "Remove Path", 400, 140);

    let label = gtk4::Label::new(Some(&format!(
        "Remove {} path?\n\n{}",
        category,
        path.to_string_lossy()
    )));
    label.set_wrap(true);
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    btn_box.append(&cancel_btn);

    let remove_btn = gtk4::Button::with_label("Remove");
    remove_btn.add_css_class("destructive-action");
    btn_box.append(&remove_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        cancel_btn.connect_clicked(move |_| dlg.close());
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        let category = category.to_string();
        remove_btn.connect_clicked(move |_| {
            let mut s = state_ref.borrow_mut();
            if let Some(ws) = s.workspace_mgr.active_mut() {
                match category.as_str() {
                    "read-only" => ws.sandbox_policy.read_only_paths.retain(|p| p != &path),
                    "read-write" => ws.sandbox_policy.read_write_paths.retain(|p| p != &path),
                    "denied" => ws.sandbox_policy.denied_paths.retain(|p| p != &path),
                    _ => {}
                }
                s.sandbox_panel
                    .update(&s.workspace_mgr.active().unwrap().sandbox_policy);
            }
            dlg.close();
        });
    }

    dialog.present();
}

/// Show a confirmation dialog before applying a sandbox change that will respawn terminals.
fn show_sandbox_respawn_dialog(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    message: &str,
    on_confirm: impl FnOnce(&Rc<RefCell<AppState>>) + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    let (dialog, vbox) = styled_dialog(window, "Sandbox Change", 420, 140);

    let label = gtk4::Label::new(Some(message));
    label.set_wrap(true);
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    btn_box.append(&cancel_btn);

    let apply_btn = gtk4::Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");
    btn_box.append(&apply_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        let on_cancel = std::cell::Cell::new(Some(on_cancel));
        cancel_btn.connect_clicked(move |_| {
            if let Some(cb) = on_cancel.take() {
                cb();
            }
            dlg.close();
        });
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        let on_confirm = std::cell::Cell::new(Some(on_confirm));
        apply_btn.connect_clicked(move |_| {
            if let Some(cb) = on_confirm.take() {
                cb(&state_ref);
            }
            dlg.close();
        });
    }

    dialog.present();
}

/// Show a modal security alert dialog for Critical audit events.
/// Provides "View Audit Log" (opens audit panel) and "Dismiss" buttons.
fn show_security_alert_dialog(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    description: &str,
) {
    let (dialog, vbox) = styled_dialog(window, "Security Alert", 440, 180);

    // Warning icon + description.
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    content.set_halign(gtk4::Align::Start);

    let icon = gtk4::Image::from_icon_name("dialog-warning-symbolic");
    icon.set_pixel_size(32);
    icon.set_valign(gtk4::Align::Start);
    content.append(&icon);

    let label = gtk4::Label::new(Some(description));
    label.set_wrap(true);
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    content.append(&label);

    vbox.append(&content);

    // Buttons.
    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let dismiss_btn = gtk4::Button::with_label("Dismiss");
    btn_box.append(&dismiss_btn);

    let view_audit_btn = gtk4::Button::with_label("View Audit Log");
    view_audit_btn.add_css_class("suggested-action");
    btn_box.append(&view_audit_btn);

    vbox.append(&btn_box);

    {
        let dlg = dialog.clone();
        dismiss_btn.connect_clicked(move |_| {
            dlg.close();
        });
    }

    {
        let dlg = dialog.clone();
        let state_ref = state.clone();
        view_audit_btn.connect_clicked(move |_| {
            let mut s = state_ref.borrow_mut();
            if !s.audit_panel_visible {
                s.toggle_audit_panel();
            }
            dlg.close();
        });
    }

    dialog.present();
}

/// Collect all child PIDs from terminals belonging to a workspace.
/// Used for per-workspace port scanning. Expands each shell PID to include
/// all descendant processes (grandchildren, etc.) so that servers started
/// by wrapper scripts like `npm run dev` are detected.
fn collect_workspace_pids(
    ws_id: Uuid,
    terminal_panels: &HashMap<PanelId, TerminalPanel>,
    panel_workspace_map: &HashMap<PanelId, Uuid>,
) -> Vec<u32> {
    let mut pids = Vec::new();
    for (panel_id, ws) in panel_workspace_map {
        if *ws != ws_id {
            continue;
        }
        if let Some(tp) = terminal_panels.get(panel_id)
            && let Some(pid) = tp.surface().child_pid()
        {
            pids.push(pid);
            // Also include all descendant processes of this shell.
            for descendant in thane_core::agent::collect_descendant_pids(pid as i32) {
                pids.push(descendant as u32);
            }
        }
    }
    pids
}

/// Duration after which an active agent with no terminal output is considered stalled.
const AGENT_STALL_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(60);

/// Detect agent activity for a workspace by checking child processes of its terminals.
/// Returns the workspace-level status and a map of panel_id -> agent_name for active agents.
/// An agent is considered stalled if it is active but its terminal has not produced output
/// for longer than [`AGENT_STALL_THRESHOLD`].
fn detect_agent_status_for_workspace(
    ws_id: Uuid,
    terminal_panels: &HashMap<PanelId, TerminalPanel>,
    panel_workspace_map: &HashMap<PanelId, Uuid>,
    last_output_times: &HashMap<PanelId, std::time::Instant>,
) -> (thane_core::sidebar::AgentStatus, HashMap<PanelId, String>) {
    detect_agent_status_for_workspace_at(
        ws_id,
        terminal_panels,
        panel_workspace_map,
        last_output_times,
        std::time::Instant::now(),
    )
}

/// Inner implementation that accepts `now` for testability.
fn detect_agent_status_for_workspace_at(
    ws_id: Uuid,
    terminal_panels: &HashMap<PanelId, TerminalPanel>,
    panel_workspace_map: &HashMap<PanelId, Uuid>,
    last_output_times: &HashMap<PanelId, std::time::Instant>,
    now: std::time::Instant,
) -> (thane_core::sidebar::AgentStatus, HashMap<PanelId, String>) {
    use thane_core::agent::detect_agent_for_pid;

    // Collect per-panel agent detections from live process inspection.
    let mut detections = Vec::new();
    for (panel_id, ws) in panel_workspace_map {
        if *ws != ws_id {
            continue;
        }
        if let Some(tp) = terminal_panels.get(panel_id) {
            let pid = tp.surface().child_pid().map(|p| p as i32);
            let detection = detect_agent_for_pid(pid);
            detections.push((*panel_id, detection));
        }
    }

    resolve_agent_stall_status(&detections, last_output_times, now)
}

/// Pure logic for resolving workspace agent status with stall detection.
/// Takes pre-resolved agent detections (one per panel) and output timestamps.
/// Returns the aggregate workspace status and a map of panel_id → agent name.
///
/// Priority: Active > Stalled > Inactive.
fn resolve_agent_stall_status(
    detections: &[(PanelId, thane_core::agent::AgentDetection)],
    last_output_times: &HashMap<PanelId, std::time::Instant>,
    now: std::time::Instant,
) -> (thane_core::sidebar::AgentStatus, HashMap<PanelId, String>) {
    use thane_core::sidebar::AgentStatus;

    let mut overall = AgentStatus::Inactive;
    let mut panel_agents = HashMap::new();

    for (panel_id, detection) in detections {
        if detection.status == AgentStatus::Active {
            // Check if the agent has produced output recently.
            let stalled = match last_output_times.get(panel_id) {
                Some(last) => now.duration_since(*last) > AGENT_STALL_THRESHOLD,
                // No output ever recorded — treat as stalled.
                None => true,
            };
            let panel_status = if stalled {
                AgentStatus::Stalled
            } else {
                AgentStatus::Active
            };
            // Priority: Active > Stalled > Inactive.
            match (&overall, &panel_status) {
                (AgentStatus::Inactive, _) => overall = panel_status,
                (AgentStatus::Stalled, AgentStatus::Active) => overall = AgentStatus::Active,
                _ => {}
            }
            if let Some(name) = &detection.agent_name {
                panel_agents.insert(*panel_id, name.clone());
            }
        }
    }

    (overall, panel_agents)
}

/// The main application window.
pub struct AppWindow {
    window: gtk4::ApplicationWindow,
}

/// Wire notification signals on a terminal panel.
/// Connects VTE's notification-received (OSC 777) and commit-based OSC scanning
/// (OSC 9, 99) to the shared app state's notification pipeline.
fn wire_terminal_notifications(
    state: &Rc<RefCell<AppState>>,
    ws_id: Uuid,
    panel_id: PanelId,
) {
    let s = state.borrow();
    let terminal = match s.terminal_panels.get(&panel_id) {
        Some(tp) => tp,
        None => return,
    };

    // Wire commit-based OSC scanning for notification sequences (OSC 9, 99, 777).
    {
        let state_ref = state.clone();
        terminal.connect_osc_commit(move |osc_num, payload| {
            if let Some((title, body)) = parse_osc_notification(osc_num, payload) {
                tracing::debug!("OSC {osc_num} notification: {title}: {body}");
                if let Ok(mut s) = state_ref.try_borrow_mut() {
                    s.push_notification(ws_id, &title, &body, panel_id);
                }
            }
        });
    }

    // Wire OSC 133 shell integration marks for command block tracking.
    {
        let state_ref = state.clone();
        terminal.connect_osc_commit(move |osc_num, payload| {
            if osc_num != 133 {
                return;
            }
            use thane_core::command_block::ShellMark;
            let mark = match payload.chars().next() {
                Some('A') => ShellMark::PromptStart,
                Some('B') => ShellMark::CommandStart,
                Some('C') => ShellMark::CommandExecuted,
                Some('D') => {
                    // Parse exit code from ";D;N" format.
                    let exit_code = payload
                        .strip_prefix("D;")
                        .or_else(|| payload.strip_prefix("D"))
                        .and_then(|s| s.trim_start_matches(';').parse::<i32>().ok());
                    ShellMark::CommandFinished(exit_code)
                }
                _ => return,
            };
            if let Ok(mut s) = state_ref.try_borrow_mut()
                && let Some(tracker) = s.block_trackers.get_mut(&panel_id)
            {
                tracker.handle_mark(mark);
            }
        });
    }

    // Wire real-time audit scanning: detect sensitive file paths and PII in terminal output.
    {
        use thane_core::audit::{
            detect_pii, extract_file_paths, is_sensitive_file, strip_terminal_codes,
            SensitiveOpAction,
        };
        let state_ref = state.clone();
        terminal.connect_raw_output(move |raw_text| {
            // Strip ANSI escape sequences so audit descriptions are clean.
            let text = &strip_terminal_codes(raw_text);
            // Look up the workspace's sensitive operation policy.
            // Use try_borrow to avoid panics when AppState is already borrowed
            // (e.g., during workspace switch which triggers VTE commit signals).
            let action = {
                let Ok(s) = state_ref.try_borrow() else { return };
                s.workspace_mgr()
                    .get(ws_id)
                    .map(|ws| ws.sensitive_op_action)
                    .unwrap_or_default()
            };

            if action == SensitiveOpAction::Allow {
                // Allow mode: still log at Info level but don't scan deeply.
                return;
            }

            // Scan for sensitive file paths referenced in the output.
            let paths = extract_file_paths(text);
            let mut critical_alert: Option<String> = None;
            for path in &paths {
                if let Some(event_type) = is_sensitive_file(path) {
                    let severity = match &event_type {
                        AuditEventType::PrivateKeyAccess => AuditSeverity::Critical,
                        _ => AuditSeverity::Alert,
                    };
                    let description = match &event_type {
                        AuditEventType::PrivateKeyAccess => {
                            format!("Private key file referenced: {path}")
                        }
                        _ => format!("Sensitive file referenced: {path}"),
                    };
                    let Ok(mut s) = state_ref.try_borrow_mut() else { return };
                    let agent = s.panel_agents.get(&panel_id).cloned();
                    s.audit_log_mut().log_with_agent(
                        ws_id,
                        Some(panel_id),
                        event_type.clone(),
                        severity,
                        &description,
                        serde_json::json!({"path": path, "action": format!("{action:?}")}),
                        agent,
                    );

                    // Warn + Block: send desktop notification.
                    if let Err(e) = s.notifier.send_notification(
                        "Security Alert",
                        &description,
                        if severity == AuditSeverity::Critical {
                            NotifyUrgency::Critical
                        } else {
                            NotifyUrgency::Normal
                        },
                    ) {
                        tracing::warn!("Failed to send security notification: {e}");
                    }

                    if severity == AuditSeverity::Critical {
                        critical_alert = Some(description.clone());
                    }

                    // Block: pause the terminal's child process with SIGTSTP.
                    if action == SensitiveOpAction::Block
                        && let Some(tp) = s.terminal_panels.get(&panel_id)
                        && let Some(pid) = tp.surface().child_pid()
                    {
                        tracing::warn!(
                            "Blocking: sending SIGTSTP to PID {pid} for sensitive access: {path}"
                        );
                        unsafe {
                            libc::kill(pid as i32, libc::SIGTSTP);
                        }
                    }
                }
            }

            // Show security alert dialog for Critical events (after releasing borrow).
            if let Some(alert_desc) = critical_alert {
                let win = state_ref.try_borrow().ok().and_then(|s| {
                    s.window_ref.as_ref().and_then(|w| w.upgrade())
                });
                if let Some(win) = win {
                    show_security_alert_dialog(&win, &state_ref, &alert_desc);
                }
            }

            // Scan for PII patterns in the output.
            let pii_findings = detect_pii(text);
            if !pii_findings.is_empty() {
                let description = format!(
                    "PII detected in terminal output: {}",
                    pii_findings.join(", ")
                );
                let Ok(mut s) = state_ref.try_borrow_mut() else { return };
                let agent = s.panel_agents.get(&panel_id).cloned();
                s.audit_log_mut().log_with_agent(
                    ws_id,
                    Some(panel_id),
                    AuditEventType::PiiDetected,
                    AuditSeverity::Alert,
                    &description,
                    serde_json::json!({"findings": pii_findings, "action": format!("{action:?}")}),
                    agent,
                );

                // Block: pause the terminal's child process.
                if action == SensitiveOpAction::Block
                    && let Some(tp) = s.terminal_panels.get(&panel_id)
                    && let Some(pid) = tp.surface().child_pid()
                {
                    tracing::warn!(
                        "Blocking: sending SIGTSTP to PID {pid} for PII detection"
                    );
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTSTP);
                    }
                }
            }
        });
    }

    // Wire agent prompt capture: detect agent commands and log them.
    // Prompts are captured and can be submitted to the agent queue.
    {
        let state_ref = state.clone();
        terminal.connect_raw_output(move |text| {
            // Check each line for agent commands.
            for line in text.lines() {
                if let Some(captured) = thane_core::prompt_capture::detect_agent_prompt(line) {
                    tracing::info!(
                        "Captured {} prompt: {} (print_mode={})",
                        captured.agent_name,
                        &captured.text[..captured.text.len().min(50)],
                        captured.print_mode,
                    );
                    // Log as audit event with agent attribution.
                    let Ok(mut s) = state_ref.try_borrow_mut() else { return };
                    let agent = Some(captured.agent_name.clone());
                    s.audit_log_mut().log_with_agent(
                        ws_id,
                        Some(panel_id),
                        AuditEventType::AgentInvocation,
                        AuditSeverity::Info,
                        format!("{} invoked: {}", captured.agent_name, &captured.text[..captured.text.len().min(100)]),
                        serde_json::json!({
                            "prompt": captured.text,
                            "agent": captured.agent_name,
                            "print_mode": captured.print_mode,
                            "command_line": captured.command_line,
                        }),
                        agent,
                    );
                }
            }
        });
    }

    // Track last terminal output time for agent stall detection.
    {
        let state_ref = state.clone();
        terminal.connect_raw_output(move |_text| {
            if let Ok(mut s) = state_ref.try_borrow_mut() {
                s.last_output_times.insert(panel_id, std::time::Instant::now());
            }
        });
    }

    // Wire CWD changes to update the workspace.
    {
        let state_ref = state.clone();
        terminal.connect_cwd_changed(move |cwd| {
            if let Some(new_cwd) = cwd {
                let Ok(mut s) = state_ref.try_borrow_mut() else { return };
                if let Some(ws) = s.workspace_mgr.get_mut(ws_id) {
                    ws.cwd = new_cwd.clone();
                    // Update per-panel location info.
                    if let Some(panel) = ws.panels.get_mut(&panel_id) {
                        panel.location = new_cwd.clone();
                    }
                    let git = GitInfo::detect(std::path::Path::new(&new_cwd));
                    ws.sidebar.panel_locations.insert(
                        panel_id,
                        thane_core::sidebar::PanelLocationInfo {
                            cwd: new_cwd,
                            git_branch: git.branch,
                            git_dirty: git.dirty,
                        },
                    );
                }
                s.refresh_sidebar();
            }
        });
    }

    // Wire hyperlink clicks: normal click → embedded browser, Shift+click → system browser.
    {
        let state_ref = state.clone();
        terminal.connect_hyperlink_clicked(move |url, shift_held| {
            if shift_held {
                // Open in the system's default browser (if enabled).
                let enabled = state_ref
                    .try_borrow()
                    .map(|s| s.link_url_in_browser)
                    .unwrap_or(true);
                if enabled {
                    let launcher = gtk4::UriLauncher::new(url);
                    launcher.launch(
                        gtk4::Window::NONE,
                        gtk4::gio::Cancellable::NONE,
                        |result| {
                            if let Err(e) = result {
                                tracing::warn!("Failed to launch URI in system browser: {e}");
                            }
                        },
                    );
                }
            } else {
                // Open in embedded browser tab (if enabled).
                if let Ok(mut s) = state_ref.try_borrow_mut()
                    && s.link_url_in_app
                {
                    let url = url.to_string();
                    s.open_browser(&url);
                }
            }
        });
    }
}

impl AppWindow {
    pub fn new(app: &gtk4::Application) -> Self {
        let config = Config::load_default();

        // Create the VTE engine with config.
        let mut engine = VteEngine::new();
        engine.set_font(format!("{} {}", config.font_family(), config.font_size()));
        engine.set_scrollback_lines(config.scrollback_limit());

        // Default CWD — always start in the user's home directory.
        let cwd = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let base_font_size = config.font_size();
        let current_font_family = config.font_family().to_string();

        // Build the window.
        let window = gtk4::ApplicationWindow::builder()
            .application(app)
            .title("thane")
            .default_width(1200)
            .default_height(800)
            .build();

        // Enforce a minimum window size so the layout never collapses.
        window.set_size_request(800, 500);

        // Header bar.
        let header_bar = gtk4::HeaderBar::new();
        header_bar.add_css_class("thane-header");
        let title_label = gtk4::Label::new(Some("thane"));
        title_label.add_css_class("thane-header-title");
        header_bar.set_title_widget(Some(&title_label));

        let settings_btn = gtk4::Button::from_icon_name("emblem-system-symbolic");
        settings_btn.set_tooltip_text(Some("Settings (Ctrl+,)"));
        settings_btn.add_css_class("flat");
        header_bar.pack_end(&settings_btn);

        let contact_btn = gtk4::Button::from_icon_name("mail-unread-symbolic");
        contact_btn.set_tooltip_text(Some("Contact Us"));
        contact_btn.add_css_class("flat");
        header_bar.pack_end(&contact_btn);

        let help_btn = gtk4::Button::from_icon_name("help-about-symbolic");
        help_btn.set_tooltip_text(Some("Help (F1)"));
        help_btn.add_css_class("flat");
        header_bar.pack_end(&help_btn);

        window.set_titlebar(Some(&header_bar));

        // Outer layout: main_box (horizontal) on top, status_bar at bottom.
        let outer_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Main layout: sidebar | content | panels
        let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        main_box.set_vexpand(true);

        let sidebar = SidebarView::new();
        main_box.append(sidebar.widget());

        let workspace_stack = gtk4::Stack::new();
        workspace_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        workspace_stack.set_transition_duration(100);
        workspace_stack.set_hexpand(true);
        workspace_stack.set_vexpand(true);
        main_box.append(&workspace_stack);

        // Notification panel (right side, hidden by default).
        let notification_panel = NotificationPanel::new();
        notification_panel.widget().set_visible(false);
        main_box.append(notification_panel.widget());

        // Audit panel (right side, hidden by default).
        let audit_panel = AuditPanel::new();
        audit_panel.widget().set_visible(false);
        main_box.append(audit_panel.widget());

        // Git diff panel (right side, hidden by default).
        let git_diff_panel = GitDiffPanel::new();
        git_diff_panel.widget().set_visible(false);
        main_box.append(git_diff_panel.widget());

        // Settings panel (right side, hidden by default).
        let settings_panel = SettingsPanel::new();
        settings_panel.widget().set_visible(false);
        settings_panel.set_font_size(base_font_size);
        settings_panel.set_font_family(&current_font_family);
        settings_panel.set_ui_font_size(config.ui_text_size());
        settings_panel.set_scrollback_limit(config.scrollback_limit());
        settings_panel.set_cursor_style(config.cursor_style());
        settings_panel.set_cursor_blink(config.cursor_blink());
        settings_panel.set_confirm_close(config.confirm_close_surface());
        settings_panel.set_link_url_in_app(config.link_url_in_app());
        settings_panel.set_link_url_in_browser(config.link_url_in_browser());
        settings_panel.set_font_color(config.terminal_font_color());
        settings_panel.set_sensitive_policy(config.sensitive_data_policy());
        settings_panel.set_queue_mode(config.queue_mode());
        settings_panel.set_queue_schedule(config.queue_schedule());
        settings_panel.set_queue_sandbox_mode(config.queue_sandbox_mode());
        settings_panel.set_enterprise_monthly_cost(config.enterprise_monthly_cost());
        settings_panel.set_cost_display_scope(config.cost_display_scope());
        // Enterprise cost row is only visible for Enterprise plans (detected later).
        settings_panel.set_enterprise_cost_visible(false);
        // Working directory is set to a static location, not user-configurable.
        main_box.append(settings_panel.widget());

        // Token usage panel (right side, hidden by default).
        let token_panel = TokenPanel::new();
        token_panel.set_scope(config.cost_display_scope());
        token_panel.widget().set_visible(false);
        main_box.append(token_panel.widget());

        // Help panel (right side, hidden by default).
        let help_panel = HelpPanel::new();
        help_panel.widget().set_visible(false);
        main_box.append(help_panel.widget());

        // Agent queue panel (right side, hidden by default).
        let agent_queue_panel = AgentQueuePanel::new();
        agent_queue_panel.widget().set_visible(false);
        main_box.append(agent_queue_panel.widget());

        // Plans panel (right side, hidden by default).
        let plans_panel = PlansPanel::new();
        plans_panel.widget().set_visible(false);
        main_box.append(plans_panel.widget());

        // Sandbox configuration panel (right side, hidden by default).
        let sandbox_panel = SandboxPanel::new();
        sandbox_panel.widget().set_visible(false);
        main_box.append(sandbox_panel.widget());

        outer_box.append(&main_box);

        // Status bar at bottom.
        let status_bar = StatusBar::new();
        status_bar.set_font_size(base_font_size);
        outer_box.append(status_bar.widget());

        window.set_child(Some(&outer_box));

        // Dynamic UI font size CSS provider.
        let ui_css_provider = gtk4::CssProvider::new();
        gtk4::style_context_add_provider_for_display(
            &gdk4::Display::default().expect("Could not get default display"),
            &ui_css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );

        // Create shared app state.
        let state = Rc::new(RefCell::new(AppState {
            workspace_mgr: WorkspaceManager::new(),
            engine,
            browser_engine: WebKitEngine::new(),
            sidebar,
            notification_panel,
            audit_panel,
            git_diff_panel,
            settings_panel,
            settings_panel_visible: false,
            token_panel,
            token_panel_visible: false,
            help_panel,
            help_panel_visible: false,
            agent_queue_panel,
            agent_queue_panel_visible: false,
            plans_panel,
            plans_panel_visible: false,
            sandbox_panel,
            sandbox_panel_visible: false,
            status_bar,
            current_font_size: base_font_size,
            base_font_size,
            current_font_family,
            workspace_stack,
            split_containers: HashMap::new(),
            terminal_panels: HashMap::new(),
            browser_panels: HashMap::new(),
            panel_workspace_map: HashMap::new(),
            notifier: LinuxNotifier,
            leader_mode: false,
            notification_panel_visible: false,
            audit_panel_visible: false,
            git_diff_panel_visible: false,
            zoomed_pane: None,
            config_mtime: None,
            agent_queue: AgentQueue::new(),
            running_queue_entry: None,
            audit_log: AuditLog::new(10000),
            self_ref: None,
            window_ref: None,
            current_ui_font_size: config.ui_text_size(),
            ui_css_provider,
            confirm_close: config.confirm_close_surface(),
            link_url_in_app: config.link_url_in_app(),
            link_url_in_browser: config.link_url_in_browser(),
            config: config.clone(),
            workspace_history: WorkspaceHistory::new(),
            block_trackers: HashMap::new(),
            seen_prompt_uuids: std::collections::HashSet::new(),
            panel_agents: HashMap::new(),
            last_output_times: HashMap::new(),
            git_diff_cwd: None,
            started_at: std::time::SystemTime::now(),
            queue_schedule: parse_schedule(config.queue_schedule()),
            queue_history_store: QueueHistoryStore::new(
                thane_platform::dirs::LinuxDirs.sessions_dir(),
            ),
            cost_cache: thane_core::cost_tracker::CostCache::new(),
            cached_limit_info: None,
            limit_info_fetched_at: None,
            has_claude: queue_executor::which_claude().is_some(),
        }));

        // Set the self-reference and window reference.
        state.borrow_mut().self_ref = Some(Rc::downgrade(&state));
        {
            let weak_win = glib::WeakRef::new();
            weak_win.set(Some(&window));
            state.borrow_mut().window_ref = Some(weak_win);
        }

        // Apply saved UI text size from config.
        {
            let ui_size = config.ui_text_size();
            if (ui_size - 14.0).abs() > 0.5 {
                state.borrow_mut().set_ui_font_size(ui_size);
            }
        }

        // Register window-level actions (usable from any child widget via "win.*").
        {
            let toggle_git_diff = gio::SimpleAction::new("toggle-git-diff", None);
            let state_ref = state.clone();
            toggle_git_diff.connect_activate(move |_, _| {
                state_ref.borrow_mut().toggle_git_diff_panel();
            });
            window.add_action(&toggle_git_diff);
        }

        // Wire sidebar: port click opens browser.
        {
            let state_ref = state.clone();
            state.borrow_mut().sidebar.set_port_click_handler(move |port| {
                let url = format!("http://localhost:{port}");
                tracing::info!("Port click: opening {url}");
                state_ref.borrow_mut().open_browser(&url);
            });
        }

        // Wire sidebar: workspace close button (with confirmation dialog).
        {
            let state_ref = state.clone();
            let window_ref = window.downgrade();
            state.borrow_mut().sidebar.set_close_workspace_handler(move |index| {
                let state_ref2 = state_ref.clone();
                if let Some(win) = window_ref.upgrade() {
                    show_close_workspace_dialog(&win, &state_ref2, index);
                }
            });
        }

        // Wire sidebar: right-click workspace row to rename.
        {
            let state_ref = state.clone();
            let window_ref = window.downgrade();
            state.borrow_mut().sidebar.set_rename_workspace_handler(move |index| {
                // Defer to avoid re-entrant borrow (ListBox selection signals
                // fire during row rebuild inside select_workspace).
                let sr = state_ref.clone();
                let wr = window_ref.clone();
                glib::idle_add_local_once(move || {
                    sr.borrow_mut().select_workspace(index);
                    if let Some(win) = wr.upgrade() {
                        show_rename_dialog(&win, &sr);
                    }
                });
            });
        }

        // Try to restore a saved session, or create a fresh workspace.
        {
            let sessions_dir = thane_platform::dirs::LinuxDirs.sessions_dir();
            let store = SessionStore::new(sessions_dir);
            let restored = match store.load() {
                Ok(Some(snapshot)) if !snapshot.workspaces.is_empty() => {
                    tracing::info!(
                        "Restoring session with {} workspace(s)",
                        snapshot.workspaces.len()
                    );
                    let mut all_pairs: Vec<(Uuid, Vec<PanelId>)> = Vec::new();
                    for ws_snap in &snapshot.workspaces {
                        let (ws_id, terminal_ids) = state
                            .borrow_mut()
                            .restore_workspace_from_snapshot(ws_snap);
                        all_pairs.push((ws_id, terminal_ids));
                    }
                    // Restore sidebar collapsed state.
                    if let Some(collapsed) = snapshot.sidebar_collapsed {
                        state.borrow_mut().sidebar.set_collapsed(collapsed);
                    }
                    // Select the previously active workspace.
                    if let Some(active_id) = snapshot.active_workspace_id {
                        let mut s = state.borrow_mut();
                        s.workspace_mgr.select_by_id(active_id);
                        s.switch_to_active_workspace();
                        s.refresh_sidebar();
                    }
                    // Wire notifications for all restored terminal panels.
                    for (ws_id, terminal_ids) in all_pairs {
                        for panel_id in terminal_ids {
                            wire_terminal_notifications(&state, ws_id, panel_id);
                        }
                    }
                    true
                }
                Ok(other) => {
                    tracing::info!("Session loaded but empty or None: {:?}", other.is_some());
                    false
                }
                Err(e) => {
                    tracing::warn!("Failed to load session, starting fresh: {e}");
                    false
                }
            };
            if !restored {
                let (ws_id, panel_id) =
                    state.borrow_mut().create_workspace("Workspace 1", &cwd);
                wire_terminal_notifications(&state, ws_id, panel_id);
            }
        }

        // Load workspace close history from disk.
        {
            let sessions_dir = thane_platform::dirs::LinuxDirs.sessions_dir();
            let history_store = HistoryStore::new(sessions_dir);
            match history_store.load() {
                Ok(history) => {
                    if !history.entries.is_empty() {
                        tracing::info!("Loaded {} closed workspace(s) from history", history.entries.len());
                    }
                    state.borrow_mut().workspace_history = history;
                    state.borrow_mut().refresh_sidebar();
                }
                Err(e) => {
                    tracing::warn!("Failed to load workspace history: {e}");
                }
            }
        }

        // Wire sidebar: reopen from history.
        {
            let state_ref = state.clone();
            state.borrow_mut().sidebar.set_reopen_handler(move |original_id| {
                let sr = state_ref.clone();
                glib::idle_add_local_once(move || {
                    let result = sr.borrow_mut().reopen_from_history(original_id);
                    if let Some((ws_id, panel_id)) = result {
                        wire_terminal_notifications(&sr, ws_id, panel_id);
                    }
                });
            });
        }

        // Wire sidebar: clear history.
        {
            let state_ref = state.clone();
            state.borrow_mut().sidebar.set_clear_history_handler(move || {
                let mut s = state_ref.borrow_mut();
                s.workspace_history.clear();
                s.save_history();
                s.refresh_sidebar();
            });
        }

        // Every-launch dependency check + first-run CLAUDE.md injection.
        crate::setup::run_startup_checks(&window);

        // Wire notification panel: mark all read button.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .notification_panel
                .connect_mark_all_read(move || {
                    let mut s = state_ref.borrow_mut();
                    if let Some(ws) = s.workspace_mgr.active_mut() {
                        ws.notifications.mark_all_read();
                    }
                    s.refresh_notification_panel();
                    s.refresh_sidebar();
                });
        }

        // Wire notification panel: clear button.
        {
            let state_ref = state.clone();
            state.borrow().notification_panel.connect_clear(move || {
                let mut s = state_ref.borrow_mut();
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    ws.notifications.clear();
                }
                s.refresh_notification_panel();
                s.refresh_sidebar();
            });
        }

        // Wire audit panel: clear button.
        {
            let state_ref = state.clone();
            state.borrow().audit_panel.connect_clear(move || {
                let mut s = state_ref.borrow_mut();
                s.audit_log.clear();
                s.refresh_audit_panel();
            });
        }

        // Wire audit panel: export button.
        {
            let state_ref = state.clone();
            let window_ref = window.downgrade();
            state.borrow().audit_panel.connect_export(move || {
                let json_str = {
                    let s = state_ref.borrow();
                    s.audit_log.export_json().unwrap_or_default()
                };
                if json_str.is_empty() {
                    tracing::warn!("No audit events to export");
                    return;
                }
                if let Some(win) = window_ref.upgrade() {
                    let dialog = gtk4::FileDialog::new();
                    dialog.set_initial_name(Some("thane-audit.json"));
                    let json_data = json_str;
                    dialog.save(Some(&win), gtk4::gio::Cancellable::NONE, move |result| {
                        if let Ok(file) = result
                            && let Some(path) = file.path()
                            && let Err(e) = std::fs::write(&path, &json_data)
                        {
                            tracing::error!("Failed to write audit export: {e}");
                        }
                    });
                }
            });
        }

        // Wire audit panel: date range dropdown.
        {
            let state_ref = state.clone();
            state.borrow().audit_panel.connect_date_range(move |range| {
                let s = state_ref.borrow();
                s.audit_panel.set_date_range(range);
                s.refresh_audit_panel();
            });
        }

        // Wire audit panel: severity filter buttons.
        {
            let state_ref = state.clone();
            state.borrow().audit_panel.connect_filter(move |filter| {
                let s = state_ref.borrow();
                s.audit_panel.set_severity_filter(filter);
                s.refresh_audit_panel();
            });
        }
        {
            let state_ref = state.clone();
            state.borrow().audit_panel.connect_search(move |text| {
                let s = state_ref.borrow();
                s.audit_panel.set_search_text(text);
                s.refresh_audit_panel();
            });
        }
        {
            let window_ref = window.downgrade();
            state.borrow().audit_panel.connect_row_activated(move |event| {
                if let Some(win) = window_ref.upgrade() {
                    show_audit_detail_dialog(&win, &event);
                }
            });
        }

        // Wire right panel close buttons.
        {
            let state_ref = state.clone();
            state.borrow().notification_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_notification_panel();
            });
        }
        {
            let state_ref = state.clone();
            state.borrow().audit_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_audit_panel();
            });
        }
        {
            let state_ref = state.clone();
            state.borrow().git_diff_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_git_diff_panel();
            });
        }
        {
            let state_ref = state.clone();
            state.borrow().token_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_token_panel();
            });
        }
        {
            let state_ref = state.clone();
            state.borrow().token_panel.connect_scope_changed(move |scope| {
                let mut s = state_ref.borrow_mut();
                s.config.set("cost-display-scope", scope);
                let _ = s.config.save();
                s.refresh_status_bar();
                s.refresh_token_panel();
            });
        }
        {
            let state_ref = state.clone();
            state.borrow().settings_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_settings_panel();
            });
        }

        // Wire sidebar: collapse/expand toggle.
        {
            let state_ref = state.clone();
            state.borrow().sidebar.connect_collapse(move || {
                state_ref.borrow_mut().sidebar.toggle_collapse();
            });
        }

        // Wire settings panel: font size slider.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_font_size_changed(move |size| {
                    let mut s = state_ref.borrow_mut();
                    s.set_terminal_font_size(size);
                    s.config.set("font-size", &format!("{}", size as u32));
                    s.save_config();
                });
        }

        // Wire settings panel: font family dropdown.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_font_family_changed(move |family| {
                    let mut s = state_ref.borrow_mut();
                    s.set_terminal_font_family(family.clone());
                    s.config.set("font-family", &family);
                    s.save_config();
                });
        }

        // Wire settings panel: font color picker.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_font_color_changed(move |hex_color| {
                    let mut s = state_ref.borrow_mut();
                    if let Ok(rgba) = gdk4::RGBA::parse(&hex_color) {
                        for tp in s.terminal_panels.values() {
                            use vte4::prelude::*;
                            tp.surface().vte_terminal().set_color_foreground(&rgba);
                        }
                    }
                    s.config.set("terminal-foreground", &hex_color);
                    s.save_config();
                });
        }

        // Wire settings panel: UI text size slider.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_ui_size_changed(move |size| {
                    let mut s = state_ref.borrow_mut();
                    s.set_ui_font_size(size);
                    s.config.set("ui-text-size", &format!("{}", size as u32));
                    s.save_config();
                });
        }

        // Wire settings panel: scrollback limit.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_scrollback_changed(move |value| {
                    let mut s = state_ref.borrow_mut();
                    for tp in s.terminal_panels.values() {
                        use vte4::prelude::*;
                        tp.surface().vte_terminal().set_scrollback_lines(value);
                    }
                    s.config.set("scrollback-limit", &format!("{value}"));
                    s.save_config();
                });
        }

        // Wire settings panel: cursor style.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_cursor_style_changed(move |idx| {
                    let (shape, name) = match idx {
                        0 => (vte4::CursorShape::Block, "block"),
                        1 => (vte4::CursorShape::Ibeam, "bar"),
                        2 => (vte4::CursorShape::Underline, "underline"),
                        _ => (vte4::CursorShape::Block, "block"),
                    };
                    let mut s = state_ref.borrow_mut();
                    for tp in s.terminal_panels.values() {
                        use vte4::prelude::*;
                        tp.surface().vte_terminal().set_cursor_shape(shape);
                    }
                    s.config.set("cursor-style", name);
                    s.save_config();
                });
        }

        // Wire settings panel: cursor blink.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_cursor_blink_changed(move |blink| {
                    let mode = if blink {
                        vte4::CursorBlinkMode::On
                    } else {
                        vte4::CursorBlinkMode::Off
                    };
                    let mut s = state_ref.borrow_mut();
                    for tp in s.terminal_panels.values() {
                        use vte4::prelude::*;
                        tp.surface().vte_terminal().set_cursor_blink_mode(mode);
                    }
                    s.config.set("cursor-style-blink", &format!("{blink}"));
                    s.save_config();
                });
        }

        // Wire settings panel: confirm close.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_confirm_close_changed(move |confirm| {
                    let mut s = state_ref.borrow_mut();
                    s.confirm_close = confirm;
                    s.config.set("confirm-close-surface", &format!("{confirm}"));
                    s.save_config();
                });
        }

        // Wire settings panel: open URLs in app.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_link_url_in_app_changed(move |enabled| {
                    let mut s = state_ref.borrow_mut();
                    s.link_url_in_app = enabled;
                    s.config.set("link-url-in-app", &format!("{enabled}"));
                    s.save_config();
                });
        }

        // Wire settings panel: open URLs in browser.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_link_url_in_browser_changed(move |enabled| {
                    let mut s = state_ref.borrow_mut();
                    s.link_url_in_browser = enabled;
                    s.config.set("link-url-in-browser", &format!("{enabled}"));
                    s.save_config();
                });
        }

        // Wire settings panel: sensitive data policy.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_sensitive_policy_changed(move |idx| {
                    let (action, name) = match idx {
                        0 => (thane_core::audit::SensitiveOpAction::Allow, "allow"),
                        1 => (thane_core::audit::SensitiveOpAction::Warn, "warn"),
                        2 => (thane_core::audit::SensitiveOpAction::Block, "block"),
                        _ => (thane_core::audit::SensitiveOpAction::Warn, "warn"),
                    };
                    let mut s = state_ref.borrow_mut();
                    if let Some(ws) = s.workspace_mgr.active_mut() {
                        ws.sensitive_op_action = action;
                    }
                    s.config.set("sensitive-data-policy", name);
                    s.save_config();
                });
        }

        // Wire settings panel: plan selection.
        // Plan is determined by the user's Claude account, not a settings dropdown.

        // Wire settings: queue mode changed.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_queue_mode_changed(move |idx| {
                    let name = match idx {
                        0 => "automatic",
                        1 => "manual",
                        2 => "scheduled",
                        _ => "automatic",
                    };
                    let mut s = state_ref.borrow_mut();
                    s.config.set("queue-mode", name);
                    s.save_config();
                });
        }

        // Wire settings: queue schedule changed.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_queue_schedule_changed(move |schedule| {
                    let mut s = state_ref.borrow_mut();
                    s.queue_schedule = parse_schedule(&schedule);
                    s.config.set("queue-schedule", &schedule);
                    s.save_config();
                });
        }

        // Wire settings: queue sandbox mode changed.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_queue_sandbox_changed(move |idx| {
                    let name = match idx {
                        0 => "off",
                        1 => "workspace",
                        2 => "strict",
                        _ => "off",
                    };
                    let mut s = state_ref.borrow_mut();
                    s.config.set("queue-sandbox", name);
                    s.save_config();
                });
        }

        // Wire settings: enterprise monthly cost changed.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_enterprise_cost_changed(move |cost| {
                    let mut s = state_ref.borrow_mut();
                    match cost {
                        Some(c) => s.config.set("enterprise-monthly-cost", &format!("{c:.2}")),
                        None => s.config.remove("enterprise-monthly-cost"),
                    }
                    s.save_config();
                });
        }

        // Wire settings: cost display scope changed.
        {
            let state_ref = state.clone();
            state
                .borrow()
                .settings_panel
                .connect_cost_scope_changed(move |idx| {
                    let scope = if idx == 1 { "all-time" } else { "session" };
                    let mut s = state_ref.borrow_mut();
                    s.config.set("cost-display-scope", scope);
                    s.save_config();
                });
        }

        // Wire header bar: gear button opens settings panel.
        {
            let state_ref = state.clone();
            settings_btn.connect_clicked(move |_| {
                state_ref.borrow_mut().toggle_settings_panel();
            });
        }

        // Wire header bar: contact button opens website contact page.
        {
            contact_btn.connect_clicked(move |_| {
                let launcher = gtk4::UriLauncher::new("https://getthane.com/contact");
                launcher.launch(
                    gtk4::Window::NONE,
                    gtk4::gio::Cancellable::NONE,
                    |result| {
                        if let Err(e) = result {
                            tracing::warn!("Failed to open contact page: {e}");
                        }
                    },
                );
            });
        }

        // Wire header bar: help button opens help panel.
        {
            let state_ref = state.clone();
            help_btn.connect_clicked(move |_| {
                state_ref.borrow_mut().toggle_help_panel();
            });
        }

        // Wire help panel: close button.
        {
            let state_ref = state.clone();
            state.borrow().help_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_help_panel();
            });
        }

        // Wire agent queue panel: close button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_agent_queue_panel();
            });
        }

        // Wire agent queue panel: submit button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_submit(move |content| {
                let mut s = state_ref.borrow_mut();
                let ws_id = s.workspace_mgr().active().map(|ws| ws.id);
                let content_preview: String = content.chars().take(200).collect();
                let entry_id = s.agent_queue.submit(content, ws_id, 0);
                s.audit_log_mut().log(
                    ws_id.unwrap_or(Uuid::nil()),
                    None,
                    AuditEventType::QueueTaskSubmitted,
                    AuditSeverity::Info,
                    format!("Queue task submitted: {content_preview}"),
                    serde_json::json!({"entry_id": entry_id.to_string(), "source": "ui"}),
                );
                s.agent_queue_panel.update(&s.agent_queue);
                s.refresh_status_bar();
            });
        }

        // Wire agent queue panel: cancel button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_cancel(move |plan_id| {
                let mut s = state_ref.borrow_mut();
                let ws_id = s.agent_queue.get(plan_id)
                    .and_then(|e| e.workspace_id)
                    .unwrap_or(Uuid::nil());
                s.agent_queue.cancel(plan_id);
                s.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::QueueTaskCancelled,
                    AuditSeverity::Info,
                    format!("Queue task cancelled: {plan_id}"),
                    serde_json::json!({"entry_id": plan_id.to_string(), "source": "ui"}),
                );
                s.agent_queue_panel.update(&s.agent_queue);
                s.refresh_status_bar();
            });
        }

        // Wire agent queue panel: "Process All" button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_process_all(move || {
                let mut s = state_ref.borrow_mut();
                s.agent_queue.process_all = true;
                s.execute_next_queue_entry();
                s.agent_queue_panel.update(&s.agent_queue);
                s.refresh_status_bar();
            });
        }

        // Wire agent queue panel: "Process Next" button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_process_next(move || {
                let mut s = state_ref.borrow_mut();
                s.agent_queue.process_all = false;
                s.execute_next_queue_entry();
                s.agent_queue_panel.update(&s.agent_queue);
                s.refresh_status_bar();
            });
        }

        // Wire agent queue panel: dismiss button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_dismiss(move |entry_id| {
                let mut s = state_ref.borrow_mut();
                s.agent_queue.remove(entry_id);
                s.agent_queue_panel.update(&s.agent_queue);
                s.refresh_status_bar();
            });
        }

        // Wire agent queue panel: retry button.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_retry(move |entry_id| {
                let mut s = state_ref.borrow_mut();
                s.agent_queue.retry(entry_id);
                s.agent_queue_panel.update(&s.agent_queue);
                s.refresh_status_bar();
            });
        }

        // Wire agent queue panel: sandbox controls.
        {
            let state_ref = state.clone();
            state.borrow().agent_queue_panel.connect_sandbox_changed(move |enabled, enforcement_idx, allow_network| {
                let mut s = state_ref.borrow_mut();
                let needs_init = enabled && !s.agent_queue.sandbox_policy().enabled;
                if needs_init {
                    let queue_dir = s.config().get("queue-working-dir").map(String::from);
                    let base_dir = queue_dir
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| {
                            dirs::home_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                .join("thane-tasks")
                        });
                    s.agent_queue.sandbox_policy = thane_core::sandbox::SandboxPolicy::confined_to(base_dir);
                }
                let policy = s.agent_queue.sandbox_policy_mut();
                policy.enabled = enabled;
                policy.enforcement = match enforcement_idx {
                    0 => thane_core::sandbox::EnforcementLevel::Permissive,
                    2 => thane_core::sandbox::EnforcementLevel::Strict,
                    _ => thane_core::sandbox::EnforcementLevel::Enforcing,
                };
                policy.allow_network = allow_network;
            });
        }

        // Wire plans panel: close button.
        {
            let state_ref = state.clone();
            state.borrow().plans_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_plans_panel();
            });
        }

        // Wire plans panel: row activated (double-click → detail dialog).
        {
            let win_weak = window.downgrade();
            state.borrow().plans_panel.connect_row_activated(move |entry| {
                if let Some(win) = win_weak.upgrade() {
                    show_plan_detail_dialog(&win, &entry);
                }
            });
        }

        // Wire sandbox panel: close button.
        {
            let state_ref = state.clone();
            state.borrow().sandbox_panel.connect_close(move || {
                state_ref.borrow_mut().toggle_sandbox_panel();
            });
        }

        // Wire sandbox panel: enabled toggle (with confirmation dialog).
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow().sandbox_panel.connect_enabled_changed(move |enabled| {
                let Some(win) = win_ref.upgrade() else { return };
                let sr = state_ref.clone();
                show_sandbox_respawn_dialog(
                    &win,
                    &state_ref,
                    "Changing sandbox will restart all terminals in this workspace. Continue?",
                    move |state_ref| {
                        // Phase 1: update policy and respawn terminals (mutable borrow).
                        let pairs = {
                            let mut s = state_ref.borrow_mut();
                            if let Some(ws) = s.workspace_mgr.active_mut() {
                                ws.sandbox_policy.enabled = enabled;
                                let ws_id = ws.id;
                                let ws_title = ws.title.clone();
                                if enabled {
                                    ws.sandbox_policy.root_dir = std::path::PathBuf::from(&ws.cwd);
                                    if ws.sandbox_policy.read_write_paths.is_empty() {
                                        ws.sandbox_policy.read_write_paths = vec![std::path::PathBuf::from(&ws.cwd)];
                                    }
                                }
                                s.audit_log.log(
                                    ws_id,
                                    None,
                                    AuditEventType::SandboxToggle,
                                    AuditSeverity::Warning,
                                    format!(
                                        "Sandbox {} for workspace '{ws_title}'",
                                        if enabled { "enabled" } else { "disabled" },
                                    ),
                                    serde_json::json!({ "enabled": enabled }),
                                );
                            }
                            let pairs = s.respawn_workspace_terminals();
                            s.refresh_sidebar();
                            pairs
                        };
                        // Phase 2: rewire signals on respawned terminals (immutable borrow).
                        for (ws_id, panel_id) in pairs {
                            wire_terminal_notifications(state_ref, ws_id, panel_id);
                        }
                    },
                    move || {
                        // Revert: re-apply current policy to panel (updating guard suppresses handlers).
                        let s = sr.borrow();
                        if let Some(ws) = s.workspace_mgr.active() {
                            s.sandbox_panel.update(&ws.sandbox_policy);
                        }
                    },
                );
            });
        }

        // Wire sandbox panel: enforcement level change.
        {
            let state_ref = state.clone();
            state.borrow().sandbox_panel.connect_enforcement_changed(move |idx| {
                let mut s = state_ref.borrow_mut();
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    ws.sandbox_policy.enforcement = match idx {
                        0 => thane_core::sandbox::EnforcementLevel::Permissive,
                        2 => thane_core::sandbox::EnforcementLevel::Strict,
                        _ => thane_core::sandbox::EnforcementLevel::Enforcing,
                    };
                }
            });
        }

        // Wire sandbox panel: network access toggle (with confirmation dialog).
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow().sandbox_panel.connect_network_changed(move |allowed| {
                let Some(win) = win_ref.upgrade() else { return };
                let sr = state_ref.clone();
                show_sandbox_respawn_dialog(
                    &win,
                    &state_ref,
                    "Changing network access will restart all terminals in this workspace. Continue?",
                    move |state_ref| {
                        let pairs = {
                            let mut s = state_ref.borrow_mut();
                            if let Some(ws) = s.workspace_mgr.active_mut() {
                                ws.sandbox_policy.allow_network = allowed;
                            }
                            let pairs = s.respawn_workspace_terminals();
                            s.refresh_sidebar();
                            pairs
                        };
                        for (ws_id, panel_id) in pairs {
                            wire_terminal_notifications(state_ref, ws_id, panel_id);
                        }
                    },
                    move || {
                        let s = sr.borrow();
                        if let Some(ws) = s.workspace_mgr.active() {
                            s.sandbox_panel.update(&ws.sandbox_policy);
                        }
                    },
                );
            });
        }

        // Wire sandbox panel: add read-only path.
        {
            let state_ref = state.clone();
            state.borrow().sandbox_panel.connect_add_ro(move || {
                let mut s = state_ref.borrow_mut();
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    // Add the workspace CWD as a default read-only path.
                    let path = std::path::PathBuf::from(&ws.cwd);
                    if !ws.sandbox_policy.read_only_paths.contains(&path) {
                        ws.sandbox_policy.read_only_paths.push(path);
                    }
                    s.sandbox_panel.update(&s.workspace_mgr.active().unwrap().sandbox_policy);
                }
            });
        }

        // Wire sandbox panel: add read-write path.
        {
            let state_ref = state.clone();
            state.borrow().sandbox_panel.connect_add_rw(move || {
                let mut s = state_ref.borrow_mut();
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    let path = std::path::PathBuf::from(&ws.cwd);
                    if !ws.sandbox_policy.read_write_paths.contains(&path) {
                        ws.sandbox_policy.read_write_paths.push(path);
                    }
                    s.sandbox_panel.update(&s.workspace_mgr.active().unwrap().sandbox_policy);
                }
            });
        }

        // Wire sandbox panel: add denied path.
        {
            let state_ref = state.clone();
            state.borrow().sandbox_panel.connect_add_deny(move || {
                let mut s = state_ref.borrow_mut();
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    let path = std::path::PathBuf::from("/tmp");
                    if !ws.sandbox_policy.denied_paths.contains(&path) {
                        ws.sandbox_policy.denied_paths.push(path);
                    }
                    s.sandbox_panel.update(&s.workspace_mgr.active().unwrap().sandbox_policy);
                }
            });
        }

        // Wire sandbox panel: remove read-only path (with confirmation).
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow().sandbox_panel.connect_remove_ro(move |path| {
                if let Some(win) = win_ref.upgrade() {
                    show_remove_sandbox_path_dialog(&win, &state_ref, path, "read-only");
                }
            });
        }

        // Wire sandbox panel: remove read-write path (with confirmation).
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow().sandbox_panel.connect_remove_rw(move |path| {
                if let Some(win) = win_ref.upgrade() {
                    show_remove_sandbox_path_dialog(&win, &state_ref, path, "read-write");
                }
            });
        }

        // Wire sandbox panel: remove denied path (with confirmation).
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow().sandbox_panel.connect_remove_deny(move |path| {
                if let Some(win) = win_ref.upgrade() {
                    show_remove_sandbox_path_dialog(&win, &state_ref, path, "denied");
                }
            });
        }

        // Wire status bar: queue button opens agent queue panel.
        {
            let state_ref = state.clone();
            state.borrow().status_bar.connect_queue_clicked(move || {
                state_ref.borrow_mut().toggle_agent_queue_panel();
            });
        }

        // Wire status bar: plans button opens plans panel.
        {
            let state_ref = state.clone();
            state.borrow().status_bar.connect_plans_clicked(move || {
                state_ref.borrow_mut().toggle_plans_panel();
            });
        }

        // Wire status bar: cost button opens token panel.
        {
            let state_ref = state.clone();
            state.borrow().status_bar.connect_cost_clicked(move || {
                state_ref.borrow_mut().toggle_token_panel();
            });
        }

        // Wire sidebar: folder button → folder picker, then prompt for sandbox.
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow().sidebar.connect_sandbox_clicked(move || {
                let Some(win) = win_ref.upgrade() else { return };
                let dialog = gtk4::FileDialog::builder()
                    .title("Select folder to open as workspace")
                    .modal(true)
                    .build();
                let sr = state_ref.clone();
                let win2 = win.downgrade();
                dialog.select_folder(Some(&win), gtk4::gio::Cancellable::NONE, move |result| {
                    if let Ok(folder) = result
                        && let Some(path) = folder.path()
                    {
                        let dir = path.to_string_lossy().to_string();
                        let title = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Workspace".to_string());
                        let Some(win) = win2.upgrade() else { return };

                        // Ask whether to sandbox the new workspace.
                        let (dlg, vbox) = styled_dialog(&win, "New Workspace", 420, 160);

                        let label = gtk4::Label::new(Some(&format!(
                            "Open \"{}\" as a sandboxed workspace?",
                            title
                        )));
                        label.set_wrap(true);
                        label.set_halign(gtk4::Align::Start);
                        vbox.append(&label);

                        let desc = gtk4::Label::new(Some(
                            "Sandboxing confines file and network access to this folder.",
                        ));
                        desc.add_css_class("dim-label");
                        desc.set_wrap(true);
                        desc.set_halign(gtk4::Align::Start);
                        desc.set_margin_top(4);
                        vbox.append(&desc);

                        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                        btn_box.set_halign(gtk4::Align::End);
                        btn_box.set_margin_top(12);

                        let no_btn = gtk4::Button::with_label("No");
                        btn_box.append(&no_btn);

                        let yes_btn = gtk4::Button::with_label("Yes, sandbox");
                        yes_btn.add_css_class("suggested-action");
                        btn_box.append(&yes_btn);

                        vbox.append(&btn_box);

                        // "No" → create normal workspace.
                        {
                            let d = dlg.clone();
                            let sr = sr.clone();
                            let title = title.clone();
                            let dir = dir.clone();
                            no_btn.connect_clicked(move |_| {
                                let (ws_id, panel_id) =
                                    sr.borrow_mut().create_workspace(&title, &dir);
                                wire_terminal_notifications(&sr, ws_id, panel_id);
                                d.close();
                            });
                        }

                        // "Yes, sandbox" → create sandboxed workspace.
                        {
                            let d = dlg.clone();
                            let sr = sr.clone();
                            yes_btn.connect_clicked(move |_| {
                                let pairs =
                                    sr.borrow_mut().create_sandboxed_workspace(&title, &dir);
                                for (ws_id, panel_id) in pairs {
                                    wire_terminal_notifications(&sr, ws_id, panel_id);
                                }
                                d.close();
                            });
                        }

                        dlg.present();
                    }
                });
            });
        }

        // Wire sidebar: toggle sandbox on right-click context menu.
        {
            let state_ref = state.clone();
            let win_ref = window.downgrade();
            state.borrow_mut().sidebar.set_toggle_sandbox_handler(move |index| {
                let Some(win) = win_ref.upgrade() else { return };
                let Ok(mut s) = state_ref.try_borrow_mut() else { return };
                // Select workspace first.
                s.select_workspace(index);
                let ws = match s.workspace_mgr.active() {
                    Some(ws) => ws,
                    None => return,
                };
                let currently_enabled = ws.sandbox_policy.enabled;
                let ws_cwd = ws.cwd.clone();
                drop(s);

                if currently_enabled {
                    let sr = state_ref.clone();
                    show_sandbox_respawn_dialog(
                        &win,
                        &sr,
                        "Remove sandbox from this workspace? Terminals will be respawned.",
                        move |state| {
                            let mut s = state.borrow_mut();
                            if let Some(ws) = s.workspace_mgr.active_mut() {
                                ws.sandbox_policy.enabled = false;
                            }
                            let pairs = s.respawn_workspace_terminals();
                            drop(s);
                            for (ws_id, panel_id) in pairs {
                                wire_terminal_notifications(state, ws_id, panel_id);
                            }
                        },
                        || {},
                    );
                } else {
                    let sr = state_ref.clone();
                    show_sandbox_respawn_dialog(
                        &win,
                        &sr,
                        "Convert this workspace to a sandbox? Terminals will be respawned with confinement.",
                        move |state| {
                            let mut s = state.borrow_mut();
                            if let Some(ws) = s.workspace_mgr.active_mut() {
                                ws.sandbox_policy =
                                    thane_core::sandbox::SandboxPolicy::confined_to(&ws_cwd);
                            }
                            let pairs = s.respawn_workspace_terminals();
                            drop(s);
                            for (ws_id, panel_id) in pairs {
                                wire_terminal_notifications(state, ws_id, panel_id);
                            }
                        },
                        || {},
                    );
                }
            });
        }

        // Wire status bar: audit button opens audit panel.
        {
            let state_ref = state.clone();
            state.borrow().status_bar.connect_audit_clicked(move || {
                state_ref.borrow_mut().toggle_audit_panel();
            });
        }

        // Start the IPC socket server in a background tokio runtime.
        {
            let socket_path = thane_platform::dirs::LinuxDirs.socket_path();
            let handler = crate::rpc_handler::start_rpc_bridge(state.clone());

            // Access mode: Open — any same-user process may connect.
            // Security is enforced at the filesystem level (socket is 0700).
            let access_mode = thane_ipc::auth::AccessMode::Open;

            // Set the socket path as env var so child processes can find it.
            // SAFETY: Called early during single-threaded GTK init, before any child
            // processes are spawned that depend on this value.
            unsafe {
                std::env::set_var("THANE_SOCKET_PATH", socket_path.to_string_lossy().as_ref());
            }

            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime for IPC");

                rt.block_on(async move {
                    if let Err(e) =
                        thane_ipc::server::start_server(&socket_path, handler, access_mode).await
                    {
                        tracing::error!("IPC server error: {e}");
                    }
                });
            });

            tracing::info!("IPC socket server starting");
        }

        // Wire sidebar: workspace selection.
        {
            let state_ref = state.clone();
            state
                .borrow_mut()
                .sidebar
                .connect_workspace_selected(move |index| {
                    // Use try_borrow_mut to avoid re-entrant panics when
                    // refresh_sidebar() rebuilds the ListBox and triggers
                    // the row_selected signal while we're already borrowed.
                    if let Ok(mut s) = state_ref.try_borrow_mut() {
                        s.select_workspace(index);
                    }
                });
        }

        // Wire sidebar: add workspace button.
        {
            let state_ref = state.clone();
            state.borrow().sidebar.connect_add_workspace(move || {
                let (ws_id, panel_id) = {
                    let mut s = state_ref.borrow_mut();
                    let count = s.workspace_mgr.count() + 1;
                    let title = format!("Workspace {count}");
                    let cwd = s.default_cwd();
                    s.create_workspace(&title, &cwd)
                };
                wire_terminal_notifications(&state_ref, ws_id, panel_id);
            });
        }

        // Leader mode key event controller.
        {
            let state_ref = state.clone();
            let window_ref = window.downgrade();
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, _modifiers| {
                // Escape closes any visible right-side panel (or git diff).
                if keyval == Key::Escape {
                    let mut s = state_ref.borrow_mut();
                    let any_right = s.notification_panel_visible
                        || s.audit_panel_visible
                        || s.settings_panel_visible
                        || s.token_panel_visible
                        || s.help_panel_visible
                        || s.agent_queue_panel_visible
                        || s.plans_panel_visible
                        || s.sandbox_panel_visible
                        || s.git_diff_panel_visible;
                    if any_right {
                        s.close_all_right_panels();
                        if s.git_diff_panel_visible {
                            s.git_diff_panel_visible = false;
                            s.git_diff_panel.widget().set_visible(false);
                            s.git_diff_cwd = None;
                        }
                        drop(s);
                        return glib::Propagation::Stop;
                    }
                    drop(s);
                }

                let is_leader = state_ref.borrow().leader_mode;
                if is_leader {
                    let action = state_ref.borrow_mut().handle_leader_key(keyval);
                    match action {
                        LeaderAction::Consumed => return glib::Propagation::Stop,
                        LeaderAction::ShowRenameDialog => {
                            if let Some(win) = window_ref.upgrade() {
                                show_rename_dialog(&win, &state_ref);
                            }
                            return glib::Propagation::Stop;
                        }
                        LeaderAction::WireNotifications(ws_id, panel_id) => {
                            wire_terminal_notifications(&state_ref, ws_id, panel_id);
                            return glib::Propagation::Stop;
                        }
                        LeaderAction::NotConsumed => {}
                    }
                }

                // Platform action modifier: Ctrl on Linux/Windows, Super (Cmd) on macOS.
                let has_action_mod = _modifiers.contains(gdk4::ModifierType::CONTROL_MASK)
                    || (cfg!(target_os = "macos")
                        && _modifiers.contains(gdk4::ModifierType::SUPER_MASK));
                let has_shift = _modifiers.contains(gdk4::ModifierType::SHIFT_MASK);

                // Ctrl+C (or Cmd+C) with selection → copy to clipboard instead of SIGINT
                if keyval == Key::c && has_action_mod && !has_shift {
                    let s = state_ref.borrow();
                    if let Some(ws) = s.workspace_mgr.active()
                        && let Some(panel) = ws.focused_panel()
                        && let Some(tp) = s.terminal_panels.get(&panel.id)
                        && tp.has_selection()
                    {
                        tp.copy_clipboard();
                        return glib::Propagation::Stop;
                    }
                }

                // Ctrl+V (or Cmd+V) → paste from clipboard
                if keyval == Key::v && has_action_mod && !has_shift {
                    let s = state_ref.borrow();
                    if let Some(ws) = s.workspace_mgr.active()
                        && let Some(panel) = ws.focused_panel()
                        && let Some(tp) = s.terminal_panels.get(&panel.id)
                    {
                        tp.paste_clipboard();
                        return glib::Propagation::Stop;
                    }
                }

                glib::Propagation::Proceed
            });
            window.add_controller(key_controller);
        }

        // Wire keyboard shortcuts.
        let state_ref = state.clone();
        let window_weak = window.downgrade();
        let state_for_rename = state.clone();
        let state_for_split = state.clone();
        let state_for_close = state.clone();
        shortcuts::setup_shortcuts_with_config(
            &window,
            &config,
            Box::new(move |action| {
                tracing::debug!("Shortcut action: {:?}", action);

                // Handle rename separately (needs to drop borrow first).
                if action == KeyAction::WorkspaceRename {
                    if let Some(win) = window_weak.upgrade() {
                        show_rename_dialog(&win, &state_for_rename);
                    }
                    return;
                }

                // Handle close actions with confirmation dialogs (before borrow_mut).
                if action == KeyAction::WorkspaceClose {
                    if let Some(win) = window_weak.upgrade() {
                        let index = state_for_close.borrow().workspace_mgr.active_index();
                        show_close_workspace_dialog(&win, &state_for_close, index);
                    }
                    return;
                }
                if action == KeyAction::PaneClose {
                    if let Some(win) = window_weak.upgrade() {
                        show_close_pane_dialog(&win, &state_for_close);
                    }
                    return;
                }
                if action == KeyAction::PanelClose {
                    if let Some(win) = window_weak.upgrade() {
                        let panel_id = state_for_close
                            .borrow()
                            .workspace_mgr
                            .active()
                            .and_then(|ws| ws.focused_panel())
                            .map(|p| p.id);
                        if let Some(pid) = panel_id {
                            show_close_panel_dialog(&win, &state_for_close, pid);
                        }
                    }
                    return;
                }

                // Handle split pane separately (needs to wire signals after).
                if action == KeyAction::SplitRight || action == KeyAction::SplitDown {
                    let orientation = if action == KeyAction::SplitRight {
                        PaneOrientation::Horizontal
                    } else {
                        PaneOrientation::Vertical
                    };
                    let result = state_ref.borrow_mut().split_pane(orientation);
                    if let Some((ws_id, panel_id)) = result {
                        wire_terminal_notifications(&state_for_split, ws_id, panel_id);
                    }
                    return;
                }

                // Handle workspace new separately (needs to wire signals after).
                if action == KeyAction::WorkspaceNew {
                    let (ws_id, panel_id) = {
                        let mut s = state_ref.borrow_mut();
                        let count = s.workspace_mgr.count() + 1;
                        let title = format!("Workspace {count}");
                        let cwd = s.default_cwd();
                        s.create_workspace(&title, &cwd)
                    };
                    wire_terminal_notifications(&state_for_split, ws_id, panel_id);
                    return;
                }

                let mut s = state_ref.borrow_mut();

                match action {
                    KeyAction::EnterLeaderMode => {
                        s.leader_mode = true;
                        s.status_bar.set_leader_mode(true);
                        tracing::debug!("Entered leader mode (Ctrl+B)");
                        let state_for_timeout = state_ref.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_secs(2),
                            move || {
                                state_for_timeout.borrow().status_bar.set_leader_mode(false);
                            },
                        );
                    }
                    // Workspace management
                    KeyAction::WorkspaceNext => s.select_next_workspace(),
                    KeyAction::WorkspacePrev => s.select_prev_workspace(),
                    KeyAction::WorkspaceSelect(n) => {
                        s.select_workspace((n as usize).saturating_sub(1));
                    }
                    // Split pane management
                    KeyAction::PaneNext => s.focus_next_pane(),
                    KeyAction::PanePrev => s.focus_prev_pane(),
                    KeyAction::PaneFocusUp | KeyAction::PaneFocusDown
                    | KeyAction::PaneFocusLeft | KeyAction::PaneFocusRight => {
                        match action {
                            KeyAction::PaneFocusRight | KeyAction::PaneFocusDown => {
                                s.focus_next_pane();
                            }
                            _ => {
                                s.focus_prev_pane();
                            }
                        }
                    }
                    KeyAction::PaneZoomToggle => s.toggle_pane_zoom(),
                    // Panel tab management
                    KeyAction::PanelNext => s.next_panel(),
                    KeyAction::PanelPrev => s.prev_panel(),
                    // Navigation
                    KeyAction::ToggleSidebar => s.toggle_sidebar(),
                    KeyAction::ToggleNotifications => s.toggle_notification_panel(),
                    KeyAction::ToggleAuditPanel => s.toggle_audit_panel(),
                    KeyAction::ToggleGitDiff => s.toggle_git_diff_panel(),
                    KeyAction::ToggleSettings => s.toggle_settings_panel(),
                    KeyAction::ToggleTokenUsage => s.toggle_token_panel(),
                    KeyAction::ToggleHelp => s.toggle_help_panel(),
                    KeyAction::ToggleAgentQueue => s.toggle_agent_queue_panel(),
                    KeyAction::TogglePlans => s.toggle_plans_panel(),
                    KeyAction::FontSizeIncrease => s.zoom_font_size(1.0),
                    KeyAction::FontSizeDecrease => s.zoom_font_size(-1.0),
                    KeyAction::FontSizeReset => s.reset_font_size(),
                    KeyAction::Copy => {
                        if let Some(ws) = s.workspace_mgr.active()
                            && let Some(panel) = ws.focused_panel()
                            && let Some(tp) = s.terminal_panels.get(&panel.id)
                        {
                            tp.copy_clipboard();
                        }
                    }
                    KeyAction::Paste => {
                        if let Some(ws) = s.workspace_mgr.active()
                            && let Some(panel) = ws.focused_panel()
                            && let Some(tp) = s.terminal_panels.get(&panel.id)
                        {
                            tp.paste_clipboard();
                        }
                    }
                    KeyAction::FindInTerminal => {
                        // Toggle search bar on the focused terminal panel.
                        if let Some(ws) = s.workspace_mgr.active()
                            && let Some(panel) = ws.focused_panel()
                            && let Some(tp) = s.terminal_panels.get(&panel.id)
                        {
                            tp.toggle_search();
                        }
                    }
                    KeyAction::ToggleSandbox => {
                        s.toggle_sandbox_panel();
                    }
                    KeyAction::ToggleFullscreen => {
                        if let Some(win) = window_weak.upgrade() {
                            if win.is_fullscreen() {
                                win.unfullscreen();
                            } else {
                                win.fullscreen();
                            }
                        }
                    }
                    _ => {
                        tracing::debug!("Unhandled shortcut action: {:?}", action);
                    }
                }
            }),
        );

        // Auto-save timer: capture snapshot every 8 seconds.
        {
            let state_ref = state.clone();
            let sessions_dir = thane_platform::dirs::LinuxDirs.sessions_dir();
            glib::timeout_add_seconds_local(8, move || {
                let mut s = state_ref.borrow_mut();
                let policy = PersistPolicy::default();
                let snapshot = s.capture_snapshot(&policy);
                let store = SessionStore::new(sessions_dir.clone());
                if let Err(e) = store.save(&snapshot) {
                    tracing::error!("Auto-save failed: {e}");
                }
                glib::ControlFlow::Continue
            });
        }

        // Audit log flush timer: persist new audit events to disk every 10 seconds.
        {
            let state_ref = state.clone();
            let audit_dir = thane_platform::dirs::LinuxDirs.data_dir().join("audit");
            let flushed_count = std::cell::Cell::new(0usize);
            glib::timeout_add_seconds_local(10, move || {
                let s = state_ref.borrow();
                let all_events = s.audit_log.all();
                let already_flushed = flushed_count.get();
                if all_events.len() > already_flushed {
                    let store = AuditStore::new(audit_dir.clone());
                    for event in &all_events[already_flushed..] {
                        if let Err(e) = store.append(event) {
                            tracing::error!("Audit flush failed: {e}");
                            break;
                        }
                    }
                    flushed_count.set(all_events.len());
                }
                glib::ControlFlow::Continue
            });
        }

        // Metadata refresh timer: update git branch + ports + config every 5 seconds.
        {
            let state_ref = state.clone();
            glib::timeout_add_seconds_local(5, move || {
                let mut s = state_ref.borrow_mut();
                s.refresh_workspace_metadata();
                s.check_config_reload();
                // Refresh sandbox panel if visible.
                if s.sandbox_panel_visible
                    && let Some(ws) = s.workspace_mgr.active()
                {
                    let policy = ws.sandbox_policy.clone();
                    s.sandbox_panel.update(&policy);
                }
                // Note: git diff panel is NOT auto-refreshed here to preserve
                // the user's expand/collapse state on file entries. The panel
                // refreshes when opened and when the user toggles it.
                glib::ControlFlow::Continue
            });
        }

        // Agent queue polling timer: poll child process + check for runnable entries every 2 seconds.
        {
            let state_ref = state.clone();
            glib::timeout_add_seconds_local(2, move || {
                let critical_alert = {
                    let mut s = state_ref.borrow_mut();

                    // Poll running headless child process.
                    let (finished, critical_alert) = s.poll_running_queue_entry();

                    // If a task just finished (or none running), try starting the next one.
                    if (finished || s.running_queue_entry.is_none())
                        && s.agent_queue.queued_count() > 0
                    {
                        let mode = QueueProcessingMode::from_str(s.config.queue_mode());
                        if s.agent_queue.should_auto_process(mode, &s.queue_schedule) {
                            s.execute_next_queue_entry();
                        }
                    }

                    // Auto-dismiss finished entries after 5 seconds.
                    s.agent_queue.remove_stale(chrono::Duration::seconds(5));

                    // Refresh agent queue panel if visible.
                    if s.agent_queue_panel_visible {
                        s.agent_queue_panel.update(&s.agent_queue);
                    }

                    critical_alert
                };

                // Show security alert dialog after releasing borrow.
                if let Some(alert_desc) = critical_alert {
                    let win = state_ref.try_borrow().ok().and_then(|s| {
                        s.window_ref.as_ref().and_then(|w| w.upgrade())
                    });
                    if let Some(win) = win {
                        show_security_alert_dialog(&win, &state_ref, &alert_desc);
                    }
                }

                glib::ControlFlow::Continue
            });
        }

        // Update check: run once at startup (after 5s delay), then every 4 hours.
        // Uses a channel to send the result back to the main thread since Rc is not Send.
        {
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            let run_check = move || {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    match check_latest_version() {
                        Ok(remote) => {
                            let current = env!("CARGO_PKG_VERSION");
                            if is_version_newer(&remote, current) {
                                let _ = tx.send(remote);
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Update check failed: {e}");
                        }
                    }
                });
            };
            // Run once at startup after a short delay.
            {
                let run_check = run_check.clone();
                glib::timeout_add_seconds_local(5, move || {
                    run_check();
                    glib::ControlFlow::Break
                });
            }
            // Then every 4 hours.
            {
                let run_check = run_check.clone();
                glib::timeout_add_seconds_local(4 * 60 * 60, move || {
                    run_check();
                    glib::ControlFlow::Continue
                });
            }
            // Poll the channel every 10 seconds to pick up results.
            {
                let state_ref = state.clone();
                glib::timeout_add_seconds_local(10, move || {
                    if let Ok(remote) = rx.try_recv() {
                        let mut s = state_ref.borrow_mut();
                        let current = env!("CARGO_PKG_VERSION");
                        let body = format!(
                            "A new version of thane is available.\n\
                             Current: v{current}\nLatest: v{remote}"
                        );
                        if let Some(ws) = s.workspace_mgr.active() {
                            let ws_id = ws.id;
                            let panel_id = ws
                                .focused_panel()
                                .map(|p| p.id)
                                .unwrap_or(uuid::Uuid::new_v4());
                            s.push_notification(
                                ws_id,
                                "Update Available",
                                &body,
                                panel_id,
                            );
                        }
                        let _ = s.notifier.send_notification(
                            "Update Available",
                            &body,
                            NotifyUrgency::Normal,
                        );
                    }
                    glib::ControlFlow::Continue
                });
            }
        }

        // Save session and history on window close.
        {
            let state_ref = state.clone();
            window.connect_close_request(move |_| {
                let mut s = state_ref.borrow_mut();
                let policy = PersistPolicy::default();
                let snapshot = s.capture_snapshot(&policy);
                let sessions_dir = thane_platform::dirs::LinuxDirs.sessions_dir();
                let store = SessionStore::new(sessions_dir);
                if let Err(e) = store.save(&snapshot) {
                    tracing::error!("Save on exit failed: {e}");
                }
                s.save_history();
                tracing::info!("Session and history saved on exit");
                glib::Propagation::Proceed
            });
        }

        // Do an initial metadata refresh.
        state.borrow_mut().refresh_workspace_metadata();

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}

/// Fetch the latest Linux version string from the marketing site.
/// Reads per-platform version from version.json: { "macos": "x.y.z", "linux": "x.y.z" }
fn check_latest_version() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let body = ureq::get("https://getthane.com/version.json").call()?.body_mut().read_to_string()?;
    let json: serde_json::Value = serde_json::from_str(&body)?;
    let version = json.get("linux")
        .and_then(|v| v.as_str())
        .ok_or("missing linux key in version.json")?
        .trim()
        .to_string();
    if version.is_empty() {
        return Err("empty linux version in version.json".into());
    }
    Ok(version)
}

/// Compare two semver-ish version strings. Returns true if `remote` is newer than `local`.
/// Strips leading 'v' and any pre-release suffix for comparison of numeric parts,
/// but treats any pre-release as older than the same numeric version without one.
fn is_version_newer(remote: &str, local: &str) -> bool {
    fn parse(s: &str) -> (Vec<u32>, Option<&str>) {
        let s = s.strip_prefix('v').unwrap_or(s);
        let (num, pre) = match s.find('-') {
            Some(i) => (&s[..i], Some(&s[i + 1..])),
            None => (s, None),
        };
        let parts: Vec<u32> = num.split('.').filter_map(|p| p.parse().ok()).collect();
        (parts, pre)
    }
    let (r_parts, r_pre) = parse(remote);
    let (l_parts, l_pre) = parse(local);
    let len = r_parts.len().max(l_parts.len());
    for i in 0..len {
        let r = r_parts.get(i).copied().unwrap_or(0);
        let l = l_parts.get(i).copied().unwrap_or(0);
        if r > l {
            return true;
        }
        if r < l {
            return false;
        }
    }
    // Same numeric version: pre-release is older than release.
    // remote has no pre-release but local does → remote is newer.
    r_pre.is_none() && l_pre.is_some()
}

#[cfg(test)]
mod tests {
    use super::is_version_newer;
    use super::{resolve_agent_stall_status, AGENT_STALL_THRESHOLD};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    use thane_core::agent::AgentDetection;
    use thane_core::panel::PanelId;
    use thane_core::sidebar::AgentStatus;
    use uuid::Uuid;

    fn make_active(name: &str) -> AgentDetection {
        AgentDetection {
            status: AgentStatus::Active,
            agent_name: Some(name.to_string()),
        }
    }

    fn make_inactive() -> AgentDetection {
        AgentDetection {
            status: AgentStatus::Inactive,
            agent_name: None,
        }
    }

    #[test]
    fn stall_no_agents_returns_inactive() {
        let now = Instant::now();
        let detections = vec![];
        let timestamps = HashMap::new();
        let (status, agents) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Inactive);
        assert!(agents.is_empty());
    }

    #[test]
    fn stall_active_agent_with_recent_output() {
        let now = Instant::now();
        let pid = PanelId::from(Uuid::new_v4());
        let detections = vec![(pid, make_active("claude"))];
        let mut timestamps = HashMap::new();
        timestamps.insert(pid, now - Duration::from_secs(10));
        let (status, agents) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Active);
        assert_eq!(agents.get(&pid).unwrap(), "claude");
    }

    #[test]
    fn stall_active_agent_with_old_output() {
        let now = Instant::now();
        let pid = PanelId::from(Uuid::new_v4());
        let detections = vec![(pid, make_active("claude"))];
        let mut timestamps = HashMap::new();
        timestamps.insert(pid, now - Duration::from_secs(90));
        let (status, agents) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Stalled);
        assert_eq!(agents.get(&pid).unwrap(), "claude");
    }

    #[test]
    fn stall_active_agent_with_no_recorded_output() {
        let now = Instant::now();
        let pid = PanelId::from(Uuid::new_v4());
        let detections = vec![(pid, make_active("codex"))];
        let timestamps = HashMap::new();
        let (status, agents) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Stalled);
        assert_eq!(agents.get(&pid).unwrap(), "codex");
    }

    #[test]
    fn stall_mixed_active_wins_over_stalled() {
        let now = Instant::now();
        let stalled_panel = PanelId::from(Uuid::new_v4());
        let active_panel = PanelId::from(Uuid::new_v4());
        let detections = vec![
            (stalled_panel, make_active("claude")),
            (active_panel, make_active("claude")),
        ];
        let mut timestamps = HashMap::new();
        timestamps.insert(stalled_panel, now - Duration::from_secs(120));
        timestamps.insert(active_panel, now - Duration::from_secs(5));
        let (status, _agents) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Active);
    }

    #[test]
    fn stall_inactive_agent_regardless_of_timestamps() {
        let now = Instant::now();
        let pid = PanelId::from(Uuid::new_v4());
        let detections = vec![(pid, make_inactive())];
        let mut timestamps = HashMap::new();
        timestamps.insert(pid, now - Duration::from_secs(5));
        let (status, agents) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Inactive);
        assert!(agents.is_empty());
    }

    #[test]
    fn stall_boundary_at_exactly_threshold() {
        let now = Instant::now();
        let pid = PanelId::from(Uuid::new_v4());
        let detections = vec![(pid, make_active("claude"))];
        let mut timestamps = HashMap::new();
        // Exactly at the threshold — should NOT be stalled (> not >=).
        timestamps.insert(pid, now - AGENT_STALL_THRESHOLD);
        let (status, _) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Active);
    }

    #[test]
    fn stall_boundary_one_second_past_threshold() {
        let now = Instant::now();
        let pid = PanelId::from(Uuid::new_v4());
        let detections = vec![(pid, make_active("claude"))];
        let mut timestamps = HashMap::new();
        timestamps.insert(pid, now - AGENT_STALL_THRESHOLD - Duration::from_secs(1));
        let (status, _) = resolve_agent_stall_status(&detections, &timestamps, now);
        assert_eq!(status, AgentStatus::Stalled);
    }

    #[test]
    fn newer_major() {
        assert!(is_version_newer("2.0.0", "1.0.0"));
    }

    #[test]
    fn newer_minor() {
        assert!(is_version_newer("0.2.0", "0.1.0"));
    }

    #[test]
    fn newer_patch() {
        assert!(is_version_newer("0.1.1", "0.1.0"));
    }

    #[test]
    fn same_version() {
        assert!(!is_version_newer("0.1.0", "0.1.0"));
    }

    #[test]
    fn older_version() {
        assert!(!is_version_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn release_newer_than_prerelease() {
        assert!(is_version_newer("0.1.0", "0.1.0-beta.2"));
    }

    #[test]
    fn prerelease_not_newer_than_release() {
        assert!(!is_version_newer("0.1.0-beta.2", "0.1.0"));
    }

    #[test]
    fn same_prerelease() {
        assert!(!is_version_newer("0.1.0-beta.2", "0.1.0-beta.2"));
    }

    #[test]
    fn strips_v_prefix() {
        assert!(is_version_newer("v0.2.0", "v0.1.0"));
        assert!(!is_version_newer("v0.1.0", "v0.2.0"));
    }

    #[test]
    fn higher_version_with_prerelease_is_newer() {
        assert!(is_version_newer("0.2.0-beta.1", "0.1.0"));
    }
}
