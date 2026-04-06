use std::rc::Rc;

use thane_core::cost_tracker::CostDisplayMode;
use thane_core::sidebar::{AgentStatus, StatusEntry, StatusStyle};
use gtk4::prelude::*;

/// A single row in the sidebar representing a workspace.
#[derive(Clone)]
pub struct WorkspaceRow {
    container: gtk4::Box,
    title_label: gtk4::Label,
    tag_label: gtk4::Label,
    close_btn: gtk4::Button,
    panels_info_box: gtk4::Box,
    badge_label: gtk4::Label,
    notification_preview: gtk4::Label,
    last_prompt_label: gtk4::Label,
    ports_box: gtk4::Box,
    agent_label: gtk4::Label,
    cost_label: gtk4::Label,
    sandbox_label: gtk4::Label,
    status_box: gtk4::Box,
}

impl Default for WorkspaceRow {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceRow {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        container.add_css_class("sidebar-row");

        // Top row: title + badge
        let top_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let title_label = gtk4::Label::new(Some("Workspace"));
        title_label.add_css_class("workspace-title");
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_hexpand(true);
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        top_row.append(&title_label);

        let tag_label = gtk4::Label::new(None);
        tag_label.add_css_class("workspace-tag");
        tag_label.set_visible(false);
        top_row.append(&tag_label);

        let sandbox_label = gtk4::Label::new(Some("sandboxed"));
        sandbox_label.add_css_class("sandbox-badge");
        sandbox_label.set_visible(false);
        top_row.append(&sandbox_label);

        let badge_label = gtk4::Label::new(None);
        badge_label.add_css_class("unread-badge");
        badge_label.set_visible(false);
        top_row.append(&badge_label);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("workspace-close-btn");
        close_btn.set_tooltip_text(Some("Close workspace"));
        close_btn.set_focusable(false);
        top_row.append(&close_btn);

        container.append(&top_row);

        // Per-panel location info (CWD + git per terminal panel).
        let panels_info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        container.append(&panels_info_box);

        // Notification preview
        let notification_preview = gtk4::Label::new(None);
        notification_preview.add_css_class("notification-body");
        notification_preview.set_halign(gtk4::Align::Start);
        notification_preview.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        notification_preview.set_max_width_chars(30);
        notification_preview.set_visible(false);
        container.append(&notification_preview);

        // Last prompt preview
        let last_prompt_label = gtk4::Label::new(None);
        last_prompt_label.add_css_class("workspace-last-prompt");
        last_prompt_label.set_halign(gtk4::Align::Start);
        last_prompt_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        last_prompt_label.set_max_width_chars(30);
        last_prompt_label.set_visible(false);
        container.append(&last_prompt_label);

        // Middle row: agent status + cost
        let meta_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let agent_label = gtk4::Label::new(None);
        agent_label.add_css_class("workspace-agent");
        agent_label.set_halign(gtk4::Align::Start);
        agent_label.set_hexpand(true);
        agent_label.set_visible(false);
        meta_row.append(&agent_label);

        let cost_label = gtk4::Label::new(None);
        cost_label.add_css_class("workspace-cost");
        cost_label.set_halign(gtk4::Align::End);
        cost_label.set_visible(false);
        meta_row.append(&cost_label);

        container.append(&meta_row);

        // Status entries box (below meta_row, populated via set_status_entries).
        let status_box = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        status_box.set_visible(false);
        container.append(&status_box);

        // Bottom row: ports only (git moved into panels_info_box)
        let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let ports_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        bottom_row.append(&ports_box);

        container.append(&bottom_row);

        Self {
            container,
            title_label,
            tag_label,
            close_btn,
            panels_info_box,
            badge_label,
            notification_preview,
            last_prompt_label,
            ports_box,
            agent_label,
            cost_label,
            sandbox_label,
            status_box,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    pub fn set_title(&self, title: &str) {
        self.title_label.set_text(title);
    }

    pub fn set_tag(&self, tag: Option<&str>) {
        match tag {
            Some(t) if !t.is_empty() => {
                self.tag_label.set_text(t);
                self.tag_label.set_visible(true);
            }
            _ => {
                self.tag_label.set_visible(false);
            }
        }
    }

    /// Set per-panel location info.
    /// Each entry is (cwd, git_branch, git_dirty).
    pub fn set_panel_locations(&self, locations: &[(String, Option<String>, bool)]) {
        // Clear existing children.
        while let Some(child) = self.panels_info_box.first_child() {
            self.panels_info_box.remove(&child);
        }

        if locations.is_empty() {
            self.panels_info_box.set_visible(false);
            return;
        }

        self.panels_info_box.set_visible(true);
        for (cwd, git_branch, git_dirty) in locations {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

            let cwd_label = gtk4::Label::new(Some(&shorten_path(cwd)));
            cwd_label.add_css_class("workspace-cwd");
            cwd_label.set_halign(gtk4::Align::Start);
            cwd_label.set_hexpand(true);
            cwd_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
            row.append(&cwd_label);

            match git_branch {
                Some(b) => {
                    let git_label = gtk4::Label::new(None);
                    git_label.set_halign(gtk4::Align::End);
                    let text = if *git_dirty {
                        format!("\u{e0a0} {b}*")
                    } else {
                        format!("\u{e0a0} {b}")
                    };
                    git_label.set_text(&text);
                    git_label.add_css_class("workspace-git");
                    if *git_dirty {
                        git_label.add_css_class("workspace-git-dirty");
                    }
                    row.append(&git_label);
                }
                None => {
                    let icon = gtk4::Image::from_icon_name("dialog-warning-symbolic");
                    icon.set_pixel_size(12);
                    icon.set_halign(gtk4::Align::End);
                    icon.set_tooltip_text(Some("Not a git repository — changes are not tracked"));
                    icon.add_css_class("workspace-git-untracked-icon");
                    row.append(&icon);
                }
            }

            self.panels_info_box.append(&row);
        }
    }

    pub fn set_unread_count(&self, count: usize) {
        if count > 0 {
            self.badge_label.set_text(&count.to_string());
            self.badge_label.set_visible(true);
        } else {
            self.badge_label.set_visible(false);
        }
    }

    pub fn set_ports(&self, ports: &[u16]) {
        self.set_ports_with_click(ports, None);
    }

    /// Set ports with an optional click handler that receives the port number.
    ///
    /// Normal click opens the port in the embedded browser (via `on_click`).
    /// Shift+click opens `http://localhost:{port}` in the system default browser.
    pub fn set_ports_with_click(
        &self,
        ports: &[u16],
        on_click: Option<&Rc<dyn Fn(u16)>>,
    ) {
        // Remove all existing port badges.
        while let Some(child) = self.ports_box.first_child() {
            self.ports_box.remove(&child);
        }

        for &port in ports.iter().take(3) {
            if let Some(handler) = on_click {
                // Wrap in a clickable button with GestureClick for Shift detection.
                let btn = gtk4::Button::with_label(&format!(":{port}"));
                btn.add_css_class("port-badge");
                btn.add_css_class("flat");
                btn.set_tooltip_text(Some(&format!(
                    "Click: open in browser tab | Shift+click: open in system browser"
                )));
                // Prevent focus on the button so clicking it doesn't
                // steal focus from the ListBox row.
                btn.set_focusable(false);

                let handler_clone = handler.clone();
                let gesture = gtk4::GestureClick::new();
                gesture.connect_released(move |g, _n, _x, _y| {
                    let shift_held = g
                        .current_event_state()
                        .contains(gdk4::ModifierType::SHIFT_MASK);

                    if shift_held {
                        let uri = format!("http://localhost:{port}");
                        if let Err(e) = gio::AppInfo::launch_default_for_uri(
                            &uri,
                            None::<&gio::AppLaunchContext>,
                        ) {
                            tracing::warn!("Failed to open {uri} in system browser: {e}");
                        }
                    } else {
                        handler_clone(port);
                    }

                    g.set_state(gtk4::EventSequenceState::Claimed);
                });
                btn.add_controller(gesture);

                self.ports_box.append(&btn);
            } else {
                let badge = gtk4::Label::new(Some(&format!(":{port}")));
                badge.add_css_class("port-badge");
                self.ports_box.append(&badge);
            }
        }

        if ports.len() > 3 {
            let more = gtk4::Label::new(Some(&format!("+{}", ports.len() - 3)));
            more.add_css_class("port-badge");
            self.ports_box.append(&more);
        }
    }

    pub fn set_notification_preview(&self, text: Option<&str>) {
        match text {
            Some(t) if !t.is_empty() => {
                self.notification_preview.set_text(t);
                self.notification_preview.set_visible(true);
            }
            _ => {
                self.notification_preview.set_visible(false);
            }
        }
    }

    pub fn set_last_prompt(&self, prompt: Option<&str>) {
        match prompt {
            Some(t) if !t.is_empty() => {
                // Show a truncated single-line version of the prompt.
                let oneline: String = t.chars()
                    .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                    .take(80)
                    .collect();
                self.last_prompt_label.set_text(&oneline);
                self.last_prompt_label.set_tooltip_text(Some(&t[..t.len().min(300)]));
                self.last_prompt_label.set_visible(true);
            }
            _ => {
                self.last_prompt_label.set_visible(false);
            }
        }
    }

    pub fn set_agent_status(&self, status: &AgentStatus) {
        match status {
            AgentStatus::Active => {
                self.agent_label.set_text("Agent running");
                self.agent_label.set_visible(true);
                self.agent_label.remove_css_class("workspace-agent-stalled");
                self.agent_label.add_css_class("workspace-agent-active");
            }
            AgentStatus::Stalled => {
                self.agent_label.set_text("Agent stalled");
                self.agent_label.set_visible(true);
                self.agent_label.remove_css_class("workspace-agent-active");
                self.agent_label.add_css_class("workspace-agent-stalled");
            }
            AgentStatus::Inactive => {
                self.agent_label.set_visible(false);
            }
        }
    }

    pub fn set_cost(
        &self,
        session_cost: Option<f64>,
        all_time_cost: Option<f64>,
        mode: CostDisplayMode,
        utilization: Option<f64>,
        subscription_cost: Option<f64>,
        use_alltime: bool,
    ) {
        let format_cost = |c: f64| -> String {
            if c < 0.01 { "<$0.01".to_string() } else { format!("${c:.2}") }
        };

        // In utilization mode with data, show utilization + derived subscription cost.
        if mode == CostDisplayMode::Utilization {
            if let Some(pct) = utilization {
                let display_cost = subscription_cost.unwrap_or(0.0);
                self.cost_label
                    .set_text(&format!("{pct:.0}% · ${display_cost:.2}"));
                self.cost_label.set_visible(true);
                self.cost_label.remove_css_class("workspace-cost-historical");
                let api_cost_str = session_cost
                    .filter(|c| *c > 0.0)
                    .or(all_time_cost.filter(|c| *c > 0.0))
                    .map(|c| format_cost(c))
                    .unwrap_or_else(|| "$0.00".to_string());
                self.cost_label.set_tooltip_text(Some(&format!(
                    "5h utilization: {pct:.0}% | ~${display_cost:.2} of plan used | {api_cost_str} API equiv."
                )));
                return;
            }
            // Fall through to dollar display if no utilization data.
        }

        // Select primary/secondary based on cost-display-scope setting.
        let (primary, secondary) = if use_alltime {
            (all_time_cost, session_cost)
        } else {
            (session_cost, all_time_cost)
        };
        let primary_label = if use_alltime { "All-time" } else { "Session" };
        let secondary_label = if use_alltime { "Session" } else { "All-time" };

        match (primary.filter(|c| *c > 0.0), secondary.filter(|c| *c > 0.0)) {
            (Some(pri), sec) => {
                self.cost_label.set_text(&format_cost(pri));
                self.cost_label.set_visible(true);
                self.cost_label.remove_css_class("workspace-cost-historical");
                if let Some(s) = sec {
                    self.cost_label.set_tooltip_text(Some(&format!(
                        "{primary_label}: {} | {secondary_label}: {}",
                        format_cost(pri),
                        format_cost(s),
                    )));
                } else {
                    self.cost_label.set_tooltip_text(Some(&format!(
                        "{primary_label}: {}",
                        format_cost(pri),
                    )));
                }
            }
            (None, Some(sec)) => {
                self.cost_label.set_text(&format_cost(sec));
                self.cost_label.set_visible(true);
                self.cost_label.add_css_class("workspace-cost-historical");
                self.cost_label.set_tooltip_text(Some(&format!(
                    "{secondary_label}: {} (no {primary_label} activity)",
                    format_cost(sec),
                )));
            }
            _ => {
                self.cost_label.set_visible(false);
                self.cost_label.set_tooltip_text(None);
            }
        }
    }

    /// Display status entries from sidebar metadata, capped at 4.
    pub fn set_status_entries(&self, entries: &[StatusEntry]) {
        // Clear existing children.
        while let Some(child) = self.status_box.first_child() {
            self.status_box.remove(&child);
        }

        if entries.is_empty() {
            self.status_box.set_visible(false);
            return;
        }

        self.status_box.set_visible(true);
        for entry in entries.iter().take(4) {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

            let label = gtk4::Label::new(Some(&entry.label));
            label.add_css_class("status-entry-label");
            label.set_halign(gtk4::Align::Start);
            label.set_hexpand(true);
            row.append(&label);

            let value = gtk4::Label::new(Some(&entry.value));
            let style_class = match entry.style {
                StatusStyle::Normal => "status-entry-normal",
                StatusStyle::Success => "status-entry-success",
                StatusStyle::Warning => "status-entry-warning",
                StatusStyle::Error => "status-entry-error",
                StatusStyle::Muted => "status-entry-muted",
            };
            value.add_css_class(style_class);
            value.set_halign(gtk4::Align::End);
            row.append(&value);

            self.status_box.append(&row);
        }
    }

    pub fn set_sandbox_enabled(&self, enabled: bool) {
        self.sandbox_label.set_visible(enabled);
    }

    /// Connect the close button callback.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }

    pub fn set_selected(&self, selected: bool) {
        if selected {
            self.container.add_css_class("sidebar-row-selected");
        } else {
            self.container.remove_css_class("sidebar-row-selected");
        }
    }
}

/// Shorten a filesystem path for display.
fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(home_str.as_ref()) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}
