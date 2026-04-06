use thane_core::cost_tracker::CostDisplayMode;
use thane_core::sidebar::AgentStatus;
use gtk4::prelude::*;

use crate::agent::dashboard::AgentDashboard;

/// A bottom status bar showing agent status, font size, cost, and audit event count.
pub struct StatusBar {
    container: gtk4::Box,
    dashboard: AgentDashboard,
    leader_label: gtk4::Label,
    cmd_label: gtk4::Label,
    font_label: gtk4::Label,
    cost_btn: gtk4::Button,
    queue_btn: gtk4::Button,
    plans_btn: gtk4::Button,
    audit_btn: gtk4::Button,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        container.add_css_class("status-bar");
        container.set_valign(gtk4::Align::End);

        // Left: agent dashboard.
        let dashboard = AgentDashboard::new();
        container.append(dashboard.widget());

        // Leader key indicator (initially hidden).
        let leader_label = gtk4::Label::new(Some("LEADER"));
        leader_label.add_css_class("status-leader-badge");
        leader_label.set_visible(false);
        container.append(&leader_label);

        // Command block status (initially hidden).
        let cmd_label = gtk4::Label::new(None);
        cmd_label.add_css_class("status-cmd-info");
        cmd_label.set_margin_start(8);
        cmd_label.set_visible(false);
        container.append(&cmd_label);

        // Spacer.
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        container.append(&spacer);

        // Font size indicator.
        let font_label = gtk4::Label::new(Some("13pt"));
        font_label.set_margin_end(4);
        container.append(&font_label);

        // Separator.
        let sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
        container.append(&sep);

        // Cost button (clickable to open token panel).
        let cost_btn = gtk4::Button::with_label("$0.00");
        cost_btn.add_css_class("flat");
        cost_btn.add_css_class("status-bar-clickable");
        cost_btn.set_tooltip_text(Some("Token usage (Ctrl+Shift+U)"));
        container.append(&cost_btn);

        // Separator.
        let sep2 = gtk4::Separator::new(gtk4::Orientation::Vertical);
        container.append(&sep2);

        // Agent queue button (always visible).
        let queue_btn = gtk4::Button::with_label("Queue");
        queue_btn.add_css_class("flat");
        queue_btn.add_css_class("status-bar-clickable");
        queue_btn.set_tooltip_text(Some("Agent queue (Ctrl+Shift+P)"));
        container.append(&queue_btn);

        // Separator.
        let sep3 = gtk4::Separator::new(gtk4::Orientation::Vertical);
        container.append(&sep3);

        // Plans button (clickable to open plans panel).
        let plans_btn = gtk4::Button::with_label("Processed");
        plans_btn.add_css_class("flat");
        plans_btn.add_css_class("status-bar-clickable");
        plans_btn.set_tooltip_text(Some("Processed tasks (Ctrl+Shift+L)"));
        container.append(&plans_btn);

        // Separator.
        let sep3b = gtk4::Separator::new(gtk4::Orientation::Vertical);
        container.append(&sep3b);

        // Audit event count button (clickable to open audit panel).
        let audit_btn = gtk4::Button::with_label("0 events");
        audit_btn.add_css_class("flat");
        audit_btn.add_css_class("status-bar-clickable");
        audit_btn.set_tooltip_text(Some("Audit trail (Ctrl+Shift+A)"));
        container.append(&audit_btn);

        // Separator.
        let sep4 = gtk4::Separator::new(gtk4::Orientation::Vertical);
        container.append(&sep4);

        // Version label.
        let version_label = gtk4::Label::new(Some(&format!("v{}", env!("CARGO_PKG_VERSION"))));
        version_label.add_css_class("status-version");
        version_label.set_margin_end(4);
        version_label.set_opacity(0.6);
        container.append(&version_label);

        Self {
            container,
            dashboard,
            leader_label,
            cmd_label,
            font_label,
            cost_btn,
            queue_btn,
            plans_btn,
            audit_btn,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Show or hide the leader key indicator badge.
    pub fn set_leader_mode(&self, active: bool) {
        self.leader_label.set_visible(active);
    }

    /// Update the agent status display using the AgentDashboard widget.
    /// `agent_names` contains the deduplicated names of active agents (e.g. ["claude", "codex"]).
    pub fn set_agent_status(&self, status: &AgentStatus, agent_names: &[String]) {
        self.dashboard.set_status(status, agent_names);
    }

    /// Update the plans count badge on the plans button.
    pub fn set_plans_count(&self, count: usize) {
        if count == 0 {
            self.plans_btn.set_label("Processed");
        } else {
            self.plans_btn.set_label(&format!("Processed ({count})"));
        }
    }

    /// Update the font size display.
    pub fn set_font_size(&self, size: f64) {
        self.font_label.set_text(&format!("{}pt", size as u32));
    }

    /// Update the cost display based on the user's plan type.
    ///
    /// For subscription plans with utilization data, shows `XX% · $Y.YY` where the
    /// dollar value is derived from `monthly_price × utilization%`.
    /// For API/Enterprise plans (or when no OAuth data), shows `$X.XX` (API cost).
    pub fn set_cost_display(
        &self,
        mode: CostDisplayMode,
        api_cost: f64,
        utilization: Option<f64>,
        subscription_cost: Option<f64>,
    ) {
        let api_cost = 0.0_f64.max(api_cost);

        // Clear previous utilization CSS classes.
        for cls in &["utilization-ok", "utilization-warn", "utilization-critical"] {
            self.cost_btn.remove_css_class(cls);
        }

        match mode {
            CostDisplayMode::Utilization => {
                if let Some(pct) = utilization {
                    let display_cost = subscription_cost.unwrap_or(api_cost);
                    self.cost_btn
                        .set_label(&format!("{pct:.0}% · ${display_cost:.2}"));
                    self.cost_btn.set_tooltip_text(Some(&format!(
                        "5h utilization: {pct:.0}% | ~${display_cost:.2} of plan used | ${api_cost:.2} API equiv. (Ctrl+Shift+U)"
                    )));
                    // Color thresholds.
                    let cls = if pct >= 85.0 {
                        "utilization-critical"
                    } else if pct >= 60.0 {
                        "utilization-warn"
                    } else {
                        "utilization-ok"
                    };
                    self.cost_btn.add_css_class(cls);
                } else {
                    self.cost_btn.set_label(&format!("${api_cost:.2}"));
                    self.cost_btn
                        .set_tooltip_text(Some("Token usage (Ctrl+Shift+U)"));
                }
            }
            CostDisplayMode::Dollar => {
                self.cost_btn.set_label(&format!("${api_cost:.2}"));
                self.cost_btn
                    .set_tooltip_text(Some("Token usage (Ctrl+Shift+U)"));
            }
        }
    }

    /// Update the audit event count.
    pub fn set_audit_count(&self, count: usize) {
        let text = if count == 1 {
            "1 event".to_string()
        } else {
            format!("{count} events")
        };
        self.audit_btn.set_label(&text);
    }

    /// Update the agent queue indicator.
    pub fn update_agent_queue(&self, queued: usize, running: usize, token_limit_paused: bool) {
        if token_limit_paused {
            self.queue_btn.set_label("Queue paused");
            self.queue_btn.remove_css_class("status-bar-clickable");
            self.queue_btn.add_css_class("status-queue-paused");
            self.queue_btn.set_tooltip_text(Some("Token limit reached \u{2014} queue paused (Ctrl+Shift+P)"));
        } else {
            self.queue_btn.remove_css_class("status-queue-paused");
            self.queue_btn.add_css_class("status-bar-clickable");
            self.queue_btn.set_tooltip_text(Some("Agent queue (Ctrl+Shift+P)"));
            let total = queued + running;
            if total == 0 {
                self.queue_btn.set_label("Queue");
            } else if total == 1 {
                self.queue_btn.set_label("1 task");
            } else {
                self.queue_btn.set_label(&format!("{total} tasks"));
            }
        }
    }

    /// Update the last command block status display.
    pub fn set_last_command(&self, exit_code: Option<i32>, duration: Option<&str>) {
        match exit_code {
            Some(code) => {
                self.cmd_label.set_visible(true);
                let dur_str = duration.unwrap_or("");
                if code == 0 {
                    self.cmd_label.set_text(&format!("\u{2713} {dur_str}"));
                    self.cmd_label.remove_css_class("status-cmd-fail");
                    self.cmd_label.add_css_class("status-cmd-ok");
                } else {
                    self.cmd_label.set_text(&format!("\u{2717} {code} {dur_str}"));
                    self.cmd_label.remove_css_class("status-cmd-ok");
                    self.cmd_label.add_css_class("status-cmd-fail");
                }
            }
            None => {
                self.cmd_label.set_visible(false);
            }
        }
    }

    /// Connect callback for clicking the cost button.
    pub fn connect_cost_clicked<F: Fn() + 'static>(&self, f: F) {
        self.cost_btn.connect_clicked(move |_| f());
    }

    /// Connect callback for clicking the audit button.
    pub fn connect_audit_clicked<F: Fn() + 'static>(&self, f: F) {
        self.audit_btn.connect_clicked(move |_| f());
    }

    /// Connect callback for clicking the queue button.
    pub fn connect_queue_clicked<F: Fn() + 'static>(&self, f: F) {
        self.queue_btn.connect_clicked(move |_| f());
    }

    /// Connect callback for clicking the plans button.
    pub fn connect_plans_clicked<F: Fn() + 'static>(&self, f: F) {
        self.plans_btn.connect_clicked(move |_| f());
    }

}
