use std::rc::Rc;

use thane_core::cost_tracker::CostDisplayMode;
use thane_core::session::ClosedWorkspaceRecord;
use thane_core::workspace::Workspace;
use gtk4::prelude::*;
use uuid::Uuid;

use super::workspace_row::WorkspaceRow;

/// The sidebar container showing workspace tabs and notifications.
/// Supports expanded and collapsed (compact) modes.
pub struct SidebarView {
    container: gtk4::Box,
    /// Expanded mode widgets.
    expanded_box: gtk4::Box,
    list_box: gtk4::ListBox,
    /// Collapsed mode widgets (avatar circles).
    collapsed_box: gtk4::Box,
    collapsed_list: gtk4::Box,
    /// Header widgets.
    #[allow(dead_code)]
    header_title: gtk4::Label,
    add_btn: gtk4::Button,
    sandbox_btn: gtk4::Button,
    collapse_btn: gtk4::Button,
    /// Collapsed mode widgets.
    collapsed_add_btn: gtk4::Button,
    collapsed_sandbox_btn: gtk4::Button,
    collapsed_expand_btn: gtk4::Button,
    /// State.
    collapsed: bool,
    rows: Vec<WorkspaceRow>,
    port_click_handler: Option<Rc<dyn Fn(u16)>>,
    close_workspace_handler: Option<Rc<dyn Fn(usize)>>,
    workspace_select_handler: Option<Rc<dyn Fn(usize)>>,
    rename_workspace_handler: Option<Rc<dyn Fn(usize)>>,
    toggle_sandbox_handler: Option<Rc<dyn Fn(usize)>>,
    /// Cached workspace data for compact mode.
    cached_workspaces: Vec<(String, usize)>, // (title, index)
    cached_active: usize,
    /// Recently-closed history section (expanded mode only).
    #[allow(dead_code)]
    history_separator: gtk4::Separator,
    #[allow(dead_code)]
    history_header: gtk4::Label,
    history_list: gtk4::ListBox,
    history_clear_btn: gtk4::Button,
    history_section: gtk4::Box,
    reopen_handler: Option<Rc<dyn Fn(Uuid)>>,
    clear_history_handler: Option<Rc<dyn Fn()>>,
}

impl Default for SidebarView {
    fn default() -> Self {
        Self::new()
    }
}

impl SidebarView {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("sidebar");
        container.set_width_request(240);

        // === Expanded mode ===
        let expanded_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Header.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let collapse_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        collapse_btn.add_css_class("flat");
        collapse_btn.set_tooltip_text(Some("Collapse sidebar"));
        header.append(&collapse_btn);

        let header_title = gtk4::Label::new(Some("Workspaces"));
        header_title.add_css_class("workspace-title");
        header_title.set_hexpand(true);
        header_title.set_halign(gtk4::Align::Start);
        header.append(&header_title);

        let sandbox_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
        sandbox_btn.set_tooltip_text(Some("Open folder as workspace"));
        sandbox_btn.add_css_class("flat");
        header.append(&sandbox_btn);

        let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some("New workspace (Ctrl+Shift+T)"));
        add_btn.add_css_class("flat");
        header.append(&add_btn);

        expanded_box.append(&header);

        // Scrollable workspace list.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::Single);
        scrolled.set_child(Some(&list_box));

        expanded_box.append(&scrolled);

        // History section (below workspace list, expanded mode only).
        let history_section = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        history_section.set_visible(false); // hidden when empty

        let history_separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        history_separator.set_margin_top(4);
        history_separator.set_margin_bottom(4);
        history_section.append(&history_separator);

        let history_header_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        history_header_box.set_margin_start(12);
        history_header_box.set_margin_end(12);
        history_header_box.set_margin_bottom(4);

        let history_header = gtk4::Label::new(Some("Recently Closed"));
        history_header.add_css_class("history-section-header");
        history_header.set_halign(gtk4::Align::Start);
        history_header.set_hexpand(true);
        history_header_box.append(&history_header);

        let history_clear_btn = gtk4::Button::with_label("Clear");
        history_clear_btn.add_css_class("flat");
        history_clear_btn.add_css_class("history-section-header");
        history_header_box.append(&history_clear_btn);

        history_section.append(&history_header_box);

        let history_list = gtk4::ListBox::new();
        history_list.set_selection_mode(gtk4::SelectionMode::None);
        history_section.append(&history_list);

        expanded_box.append(&history_section);

        container.append(&expanded_box);

        // === Collapsed mode ===
        let collapsed_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        collapsed_box.add_css_class("sidebar-collapsed");
        collapsed_box.set_visible(false);

        // Expand button at top.
        let collapsed_expand_btn = gtk4::Button::from_icon_name("go-next-symbolic");
        collapsed_expand_btn.add_css_class("flat");
        collapsed_expand_btn.set_tooltip_text(Some("Expand sidebar"));
        collapsed_expand_btn.set_halign(gtk4::Align::Center);
        collapsed_expand_btn.set_margin_top(8);
        collapsed_box.append(&collapsed_expand_btn);

        // Avatar list.
        let collapsed_list = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        collapsed_list.set_vexpand(true);
        collapsed_list.set_halign(gtk4::Align::Center);
        collapsed_list.set_margin_top(4);
        collapsed_box.append(&collapsed_list);

        // Open folder button (compact).
        let collapsed_sandbox_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
        collapsed_sandbox_btn.add_css_class("flat");
        collapsed_sandbox_btn.set_tooltip_text(Some("Open folder as workspace"));
        collapsed_sandbox_btn.set_halign(gtk4::Align::Center);
        collapsed_box.append(&collapsed_sandbox_btn);

        // Add workspace button (compact).
        let collapsed_add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        collapsed_add_btn.add_css_class("flat");
        collapsed_add_btn.set_tooltip_text(Some("New workspace"));
        collapsed_add_btn.set_halign(gtk4::Align::Center);
        collapsed_add_btn.set_margin_bottom(8);
        collapsed_box.append(&collapsed_add_btn);

        container.append(&collapsed_box);

        Self {
            container,
            expanded_box,
            list_box,
            collapsed_box,
            collapsed_list,
            header_title,
            add_btn,
            sandbox_btn,
            collapse_btn,
            collapsed_add_btn,
            collapsed_sandbox_btn,
            collapsed_expand_btn,
            collapsed: false,
            rows: Vec::new(),
            port_click_handler: None,
            close_workspace_handler: None,
            workspace_select_handler: None,
            rename_workspace_handler: None,
            toggle_sandbox_handler: None,
            cached_workspaces: Vec::new(),
            cached_active: 0,
            history_separator,
            history_header,
            history_list,
            history_clear_btn,
            history_section,
            reopen_handler: None,
            clear_history_handler: None,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Whether the sidebar is currently collapsed.
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Toggle between expanded and collapsed modes.
    pub fn toggle_collapse(&mut self) {
        self.collapsed = !self.collapsed;
        self.apply_collapse_state();
    }

    /// Set the collapsed state directly.
    pub fn set_collapsed(&mut self, collapsed: bool) {
        if self.collapsed != collapsed {
            self.collapsed = collapsed;
            self.apply_collapse_state();
        }
    }

    fn apply_collapse_state(&self) {
        if self.collapsed {
            self.expanded_box.set_visible(false);
            self.collapsed_box.set_visible(true);
            self.container.set_width_request(48);
            self.container.remove_css_class("sidebar");
            self.container.add_css_class("sidebar");
            self.container.add_css_class("sidebar-collapsed");
        } else {
            self.expanded_box.set_visible(true);
            self.collapsed_box.set_visible(false);
            self.container.set_width_request(240);
            self.container.remove_css_class("sidebar-collapsed");
        }
    }

    /// Update the sidebar to reflect the current workspace list.
    pub fn update_workspaces(
        &mut self,
        workspaces: &[Workspace],
        active_index: usize,
        cost_display_mode: CostDisplayMode,
        utilization: Option<f64>,
        subscription_cost: Option<f64>,
        use_alltime: bool,
    ) {
        // Cache workspace info for compact mode.
        self.cached_workspaces = workspaces
            .iter()
            .enumerate()
            .map(|(i, ws)| (ws.title.clone(), i))
            .collect();
        self.cached_active = active_index;

        // Update expanded mode.
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        self.rows.clear();

        for (i, workspace) in workspaces.iter().enumerate() {
            let row = WorkspaceRow::new();
            row.set_title(&workspace.title);
            row.set_tag(workspace.tag.as_deref());

            // Build per-panel location info from panel_locations, falling
            // back to workspace-level CWD + git if empty.
            let locations: Vec<(String, Option<String>, bool)> =
                if workspace.sidebar.panel_locations.is_empty() {
                    vec![(
                        workspace.cwd.clone(),
                        workspace.sidebar.git_branch.clone(),
                        workspace.sidebar.git_dirty,
                    )]
                } else {
                    // Sort panels by their position in the split tree (panel
                    // ordering) for stable display.
                    let panel_order = workspace.split_tree.all_panel_ids();
                    let mut locs = Vec::new();
                    for pid in &panel_order {
                        if let Some(info) = workspace.sidebar.panel_locations.get(pid) {
                            locs.push((
                                info.cwd.clone(),
                                info.git_branch.clone(),
                                info.git_dirty,
                            ));
                        }
                    }
                    // Include any panels not in the tree order (shouldn't happen,
                    // but be resilient).
                    for (pid, info) in &workspace.sidebar.panel_locations {
                        if !panel_order.contains(pid) {
                            locs.push((
                                info.cwd.clone(),
                                info.git_branch.clone(),
                                info.git_dirty,
                            ));
                        }
                    }
                    if locs.is_empty() {
                        vec![(
                            workspace.cwd.clone(),
                            workspace.sidebar.git_branch.clone(),
                            workspace.sidebar.git_dirty,
                        )]
                    } else {
                        locs
                    }
                };
            row.set_panel_locations(&locations);

            row.set_unread_count(workspace.notifications.unread_count());
            row.set_notification_preview(
                workspace
                    .notifications
                    .latest()
                    .map(|n| n.body.as_str()),
            );
            row.set_ports_with_click(
                &workspace.sidebar.ports,
                self.port_click_handler.as_ref(),
            );
            // Only show last prompt when an agent is actively running.
            let show_prompt = workspace.sidebar.agent_status != thane_core::sidebar::AgentStatus::Inactive;
            row.set_last_prompt(if show_prompt {
                workspace.sidebar.last_prompt.as_deref()
            } else {
                None
            });
            row.set_agent_status(&workspace.sidebar.agent_status);
            row.set_cost(
                workspace.sidebar.session_cost,
                workspace.sidebar.all_time_cost,
                cost_display_mode,
                utilization,
                subscription_cost,
                use_alltime,
            );
            row.set_sandbox_enabled(workspace.sandbox_policy.enabled);
            row.set_status_entries(&workspace.sidebar.status_entries);
            row.set_selected(i == active_index);

            // Wire close button.
            if let Some(handler) = &self.close_workspace_handler {
                let handler = handler.clone();
                row.connect_close(move || handler(i));
            }

            // Wire right-click context menu.
            let has_rename = self.rename_workspace_handler.is_some();
            let has_sandbox = self.toggle_sandbox_handler.is_some();
            if has_rename || has_sandbox {
                let rename_handler = self.rename_workspace_handler.clone();
                let sandbox_handler = self.toggle_sandbox_handler.clone();
                let sandbox_enabled = workspace.sandbox_policy.enabled;
                let widget = row.widget().clone();

                let gesture = gtk4::GestureClick::new();
                gesture.set_button(3); // secondary (right) click
                gesture.connect_pressed(move |gesture, _n, x, y| {
                    gesture.set_state(gtk4::EventSequenceState::Claimed);

                    // Build a simple popover with buttons.
                    let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                    menu_box.set_margin_top(4);
                    menu_box.set_margin_bottom(4);
                    menu_box.set_margin_start(4);
                    menu_box.set_margin_end(4);

                    let popover = gtk4::Popover::new();
                    popover.set_parent(&widget);
                    popover.set_has_arrow(true);
                    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                        x as i32, y as i32, 1, 1,
                    )));

                    if let Some(handler) = &rename_handler {
                        let handler = handler.clone();
                        let popover_ref = popover.clone();
                        let btn = gtk4::Button::with_label("Rename");
                        btn.add_css_class("flat");
                        btn.connect_clicked(move |_| {
                            popover_ref.popdown();
                            handler(i);
                        });
                        menu_box.append(&btn);
                    }

                    if let Some(handler) = &sandbox_handler {
                        let handler = handler.clone();
                        let popover_ref = popover.clone();
                        let label = if sandbox_enabled {
                            "Remove Sandbox"
                        } else {
                            "Convert to Sandbox"
                        };
                        let btn = gtk4::Button::with_label(label);
                        btn.add_css_class("flat");
                        btn.connect_clicked(move |_| {
                            popover_ref.popdown();
                            handler(i);
                        });
                        menu_box.append(&btn);
                    }

                    popover.set_child(Some(&menu_box));

                    // Clean up parent reference when popover closes.
                    let popover_ref = popover.clone();
                    popover.connect_closed(move |_| {
                        popover_ref.unparent();
                    });

                    popover.popup();
                });
                row.widget().add_controller(gesture);
            }

            self.list_box.append(row.widget());
            self.rows.push(row);
        }

        // Update collapsed mode avatars.
        self.rebuild_collapsed_avatars(workspaces, active_index);
    }

    fn rebuild_collapsed_avatars(&self, workspaces: &[Workspace], active_index: usize) {
        while let Some(child) = self.collapsed_list.first_child() {
            self.collapsed_list.remove(&child);
        }

        for (i, ws) in workspaces.iter().enumerate() {
            let abbrev = workspace_abbrev(&ws.title);

            let btn = gtk4::Button::with_label(&abbrev);
            btn.add_css_class("sidebar-avatar");
            btn.add_css_class("flat");
            btn.set_halign(gtk4::Align::Center);
            btn.set_valign(gtk4::Align::Center);

            if i == active_index {
                btn.add_css_class("sidebar-avatar-selected");
            }

            if ws.sandbox_policy.enabled {
                btn.add_css_class("sidebar-avatar-sandboxed");
            }

            btn.set_tooltip_text(Some(&ws.title));

            if let Some(handler) = &self.workspace_select_handler {
                let handler = handler.clone();
                btn.connect_clicked(move |_| handler(i));
            }

            self.collapsed_list.append(&btn);
        }
    }

    /// Connect to workspace selection changes (both expanded and collapsed modes).
    pub fn connect_workspace_selected<F: Fn(usize) + 'static>(&mut self, f: F) {
        let handler = Rc::new(f);
        self.workspace_select_handler = Some(handler.clone());
        self.list_box.connect_row_selected(move |_list, row| {
            if let Some(row) = row {
                handler(row.index() as usize);
            }
        });
    }

    /// Set the port click handler for opening ports in the browser.
    pub fn set_port_click_handler<F: Fn(u16) + 'static>(&mut self, f: F) {
        self.port_click_handler = Some(Rc::new(f));
    }

    /// Set the workspace close handler (called with workspace index).
    pub fn set_close_workspace_handler<F: Fn(usize) + 'static>(&mut self, f: F) {
        self.close_workspace_handler = Some(Rc::new(f));
    }

    /// Set the workspace rename handler (called with workspace index on right-click).
    pub fn set_rename_workspace_handler<F: Fn(usize) + 'static>(&mut self, f: F) {
        self.rename_workspace_handler = Some(Rc::new(f));
    }

    /// Set the sandbox toggle handler (called with workspace index from context menu).
    pub fn set_toggle_sandbox_handler<F: Fn(usize) + 'static>(&mut self, f: F) {
        self.toggle_sandbox_handler = Some(Rc::new(f));
    }

    /// Connect the add workspace button (both expanded and collapsed modes).
    pub fn connect_add_workspace<F: Fn() + Clone + 'static>(&self, f: F) {
        let f2 = f.clone();
        self.add_btn.connect_clicked(move |_| f());
        self.collapsed_add_btn.connect_clicked(move |_| f2());
    }

    /// Connect the sandbox button (both expanded and collapsed modes).
    pub fn connect_sandbox_clicked<F: Fn() + Clone + 'static>(&self, f: F) {
        let f2 = f.clone();
        self.sandbox_btn.connect_clicked(move |_| f());
        self.collapsed_sandbox_btn.connect_clicked(move |_| f2());
    }

    /// Connect the collapse button (expanded → collapsed).
    pub fn connect_collapse<F: Fn() + Clone + 'static>(&self, f: F) {
        let f2 = f.clone();
        self.collapse_btn.connect_clicked(move |_| f());
        self.collapsed_expand_btn.connect_clicked(move |_| f2());
    }

    /// Set the handler called when a history entry is clicked (reopen).
    pub fn set_reopen_handler<F: Fn(Uuid) + 'static>(&mut self, f: F) {
        self.reopen_handler = Some(Rc::new(f));
    }

    /// Set the handler called when the "Clear" history button is clicked.
    pub fn set_clear_history_handler<F: Fn() + 'static>(&mut self, f: F) {
        let handler = Rc::new(f);
        self.clear_history_handler = Some(handler.clone());
        self.history_clear_btn.connect_clicked(move |_| handler());
    }

    /// Update the recently-closed history section.
    pub fn update_history(&mut self, entries: &[ClosedWorkspaceRecord]) {
        // Clear existing rows.
        while let Some(child) = self.history_list.first_child() {
            self.history_list.remove(&child);
        }

        if entries.is_empty() || self.collapsed {
            self.history_section.set_visible(false);
            return;
        }

        self.history_section.set_visible(true);

        for record in entries {
            let row_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            row_box.add_css_class("history-row");
            row_box.set_cursor_from_name(Some("pointer"));
            row_box.set_margin_start(12);
            row_box.set_margin_end(12);
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);

            let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

            let title = gtk4::Label::new(Some(&record.title));
            title.add_css_class("history-title");
            title.set_halign(gtk4::Align::Start);
            title.set_hexpand(true);
            title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            top.append(&title);

            let time_label = gtk4::Label::new(Some(&format_relative_time(record.closed_at)));
            time_label.add_css_class("history-time");
            top.append(&time_label);

            row_box.append(&top);

            // Abbreviated CWD.
            let cwd_display = abbreviate_path(&record.cwd);
            let cwd_label = gtk4::Label::new(Some(&cwd_display));
            cwd_label.add_css_class("workspace-cwd");
            cwd_label.set_halign(gtk4::Align::Start);
            cwd_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            row_box.append(&cwd_label);

            let list_row = gtk4::ListBoxRow::new();
            list_row.set_child(Some(&row_box));

            // Wire click to reopen.
            if let Some(handler) = &self.reopen_handler {
                let handler = handler.clone();
                let original_id = record.original_id;
                let gesture = gtk4::GestureClick::new();
                gesture.connect_pressed(move |gesture, _n, _x, _y| {
                    gesture.set_state(gtk4::EventSequenceState::Claimed);
                    handler(original_id);
                });
                list_row.add_controller(gesture);
            }

            self.history_list.append(&list_row);
        }
    }
}

/// Build a short abbreviation for the collapsed sidebar avatar.
///
/// Takes the first letter of the title plus any trailing number, e.g.:
///   "Workspace 1" → "W1", "Workspace 12" → "W12",
///   "my-project"  → "MP", "Plan: fix bug" → "PF"
fn workspace_abbrev(title: &str) -> String {
    // Extract trailing number if present (e.g. "Workspace 3" → "3").
    let trailing_num: String = title.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();

    if let Some(first) = title.chars().find(|c| c.is_alphabetic()) {
        if !trailing_num.is_empty() {
            // "W" + "1" → "W1"
            format!("{}{}", first.to_uppercase(), trailing_num)
        } else {
            // No trailing number: take first char of the first two words.
            let initials: String = title
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .take(2)
                .filter_map(|w| w.chars().next())
                .collect::<String>()
                .to_uppercase();
            if initials.is_empty() {
                "?".to_string()
            } else {
                initials
            }
        }
    } else if !trailing_num.is_empty() {
        trailing_num
    } else {
        "?".to_string()
    }
}

/// Format a UTC timestamp as a relative time string ("2m ago", "1h ago", etc.).
fn format_relative_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_seconds() < 60 {
        "just now".to_string()
    } else if diff.num_minutes() < 60 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else {
        format!("{}d ago", diff.num_days())
    }
}

/// Abbreviate a path for display: replace home dir with ~.
fn abbreviate_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(home_str.as_ref()) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}
