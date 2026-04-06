use chrono::{DateTime, Utc};
use thane_core::cost_tracker::{CostDisplayMode, CostTracker, ProjectCostSummary, TokenLimitInfo};
use gtk4::prelude::*;

/// A right-side panel showing detailed token usage and cost breakdown.
///
/// Three sections:
/// 1. Current Session — cost, tokens, input/output/cache breakdown
/// 2. Project Total (All Time) — cost, tokens, session count, date range
/// 3. Token Limits — plan name, rolling window progress bars, reset countdowns
pub struct TokenPanel {
    container: gtk4::Box,
    content: gtk4::Box,
    scope_dropdown: gtk4::DropDown,
    // Section containers for reordering.
    session_section: gtk4::Box,
    session_sep: gtk4::Separator,
    alltime_section: gtk4::Box,
    alltime_sep: gtk4::Separator,
    limits_wrapper: gtk4::Box,
    limits_sep: gtk4::Separator,
    footnote: gtk4::Label,
    // Section 1: Current Session
    session_cost_label: gtk4::Label,
    session_tokens_label: gtk4::Label,
    session_input_label: gtk4::Label,
    session_output_label: gtk4::Label,
    session_cache_read_label: gtk4::Label,
    session_cache_write_label: gtk4::Label,
    // Section 2: All-time
    alltime_cost_label: gtk4::Label,
    alltime_tokens_label: gtk4::Label,
    alltime_session_count_label: gtk4::Label,
    alltime_date_range_label: gtk4::Label,
    // Section 3: Token limits
    limits_header: gtk4::Label,
    limits_section: gtk4::Box,
    plan_label: gtk4::Label,
    team_footnote: gtk4::Label,
    session_window_box: gtk4::Box,
    session_window_bar: gtk4::ProgressBar,
    session_window_label: gtk4::Label,
    weekly_window_box: gtk4::Box,
    weekly_window_bar: gtk4::ProgressBar,
    weekly_window_label: gtk4::Label,
    no_caps_label: gtk4::Label,
    close_btn: gtk4::Button,
}

impl Default for TokenPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("token-panel");
        container.set_width_request(320);

        // Header.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("CC Token Usage"));
        title.add_css_class("workspace-title");
        title.set_hexpand(true);
        title.set_halign(gtk4::Align::Start);
        title.set_tooltip_text(Some("Costs are estimates based on public API pricing. Actual costs may vary based on your plan."));
        header.append(&title);

        let scope_dropdown = gtk4::DropDown::from_strings(&["Session", "All Time"]);
        scope_dropdown.add_css_class("flat");
        scope_dropdown.set_tooltip_text(Some("Cost display scope"));
        header.append(&scope_dropdown);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close"));
        header.append(&close_btn);

        container.append(&header);
        container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // Wrap content in scroll.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // ── Section 1: Current Session (wrapped in a container for reordering) ──
        let session_section = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let s1_header = section_header("CURRENT SESSION");
        session_section.append(&s1_header);

        let s1 = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        s1.set_margin_start(12);
        s1.set_margin_end(12);
        s1.set_margin_top(8);
        s1.set_margin_bottom(12);

        let session_cost_label = gtk4::Label::new(Some("$0.00"));
        session_cost_label.add_css_class("token-cost-large");
        session_cost_label.set_halign(gtk4::Align::Start);
        s1.append(&session_cost_label);

        let session_tokens_label = gtk4::Label::new(Some("0 total"));
        session_tokens_label.add_css_class("token-total");
        session_tokens_label.set_halign(gtk4::Align::Start);
        s1.append(&session_tokens_label);

        let s1_details = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        s1_details.set_margin_top(8);

        let session_input_label = gtk4::Label::new(Some("0"));
        let session_output_label = gtk4::Label::new(Some("0"));
        let session_cache_read_label = gtk4::Label::new(Some("0"));
        let session_cache_write_label = gtk4::Label::new(Some("0"));

        s1_details.append(&create_detail_row("Input tokens", &session_input_label));
        s1_details.append(&create_detail_row("Output tokens", &session_output_label));
        s1_details.append(&create_detail_row("Cache read", &session_cache_read_label));
        s1_details.append(&create_detail_row("Cache write", &session_cache_write_label));

        s1.append(&s1_details);
        session_section.append(&s1);

        let session_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);

        content.append(&session_section);
        content.append(&session_sep);

        // ── Section 2: Project Total (All Time) ──
        let alltime_section = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let s2_header = section_header("PROJECT TOTAL (ALL TIME)");
        alltime_section.append(&s2_header);

        let s2 = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        s2.set_margin_start(12);
        s2.set_margin_end(12);
        s2.set_margin_top(8);
        s2.set_margin_bottom(12);

        let alltime_cost_label = gtk4::Label::new(Some("$0.00"));
        alltime_cost_label.add_css_class("token-cost-large");
        alltime_cost_label.set_halign(gtk4::Align::Start);
        s2.append(&alltime_cost_label);

        let alltime_tokens_label = gtk4::Label::new(Some("0 total"));
        alltime_tokens_label.add_css_class("token-total");
        alltime_tokens_label.set_halign(gtk4::Align::Start);
        s2.append(&alltime_tokens_label);

        let s2_meta = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        s2_meta.set_margin_top(8);

        let alltime_session_count_label = gtk4::Label::new(Some("0 sessions"));
        alltime_session_count_label.add_css_class("token-detail-label");
        alltime_session_count_label.set_halign(gtk4::Align::Start);
        s2_meta.append(&alltime_session_count_label);

        let alltime_date_range_label = gtk4::Label::new(None);
        alltime_date_range_label.add_css_class("token-detail-label");
        alltime_date_range_label.set_halign(gtk4::Align::Start);
        alltime_date_range_label.set_visible(false);
        s2_meta.append(&alltime_date_range_label);

        s2.append(&s2_meta);
        alltime_section.append(&s2);

        let alltime_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);

        content.append(&alltime_section);
        content.append(&alltime_sep);

        // ── Section 3: Token Limits ──
        let limits_wrapper = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let limits_header = section_header("TOKEN LIMITS");
        limits_wrapper.append(&limits_header);

        let limits_section = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        limits_section.set_margin_start(12);
        limits_section.set_margin_end(12);
        limits_section.set_margin_top(8);
        limits_section.set_margin_bottom(12);

        let plan_label = gtk4::Label::new(Some("Plan: Pro"));
        plan_label.add_css_class("token-detail-label");
        plan_label.set_halign(gtk4::Align::Start);
        limits_section.append(&plan_label);

        // 5-hour window.
        let session_window_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let sw_header = gtk4::Label::new(Some("5-hour window"));
        sw_header.add_css_class("token-detail-label");
        sw_header.set_halign(gtk4::Align::Start);
        session_window_box.append(&sw_header);

        let session_window_bar = gtk4::ProgressBar::new();
        session_window_bar.add_css_class("token-limit-bar");
        session_window_bar.set_fraction(0.0);
        session_window_box.append(&session_window_bar);

        let session_window_label = gtk4::Label::new(Some("No usage data"));
        session_window_label.add_css_class("token-detail-label");
        session_window_label.set_halign(gtk4::Align::Start);
        session_window_box.append(&session_window_label);

        limits_section.append(&session_window_box);

        // Weekly cap.
        let weekly_window_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let ww_header = gtk4::Label::new(Some("Weekly cap"));
        ww_header.add_css_class("token-detail-label");
        ww_header.set_halign(gtk4::Align::Start);
        weekly_window_box.append(&ww_header);

        let weekly_window_bar = gtk4::ProgressBar::new();
        weekly_window_bar.add_css_class("token-limit-bar");
        weekly_window_bar.set_fraction(0.0);
        weekly_window_box.append(&weekly_window_bar);

        let weekly_window_label = gtk4::Label::new(Some("No usage data"));
        weekly_window_label.add_css_class("token-detail-label");
        weekly_window_label.set_halign(gtk4::Align::Start);
        weekly_window_box.append(&weekly_window_label);

        limits_section.append(&weekly_window_box);

        // No-caps label (for Enterprise/API).
        let no_caps_label = gtk4::Label::new(Some("No usage caps"));
        no_caps_label.add_css_class("token-no-caps");
        no_caps_label.set_halign(gtk4::Align::Start);
        no_caps_label.set_visible(false);
        limits_section.append(&no_caps_label);

        // Team pool footnote (hidden by default).
        let team_footnote = gtk4::Label::new(Some(
            "Usage reflects your allocation within the team pool.",
        ));
        team_footnote.add_css_class("settings-hint");
        team_footnote.set_halign(gtk4::Align::Start);
        team_footnote.set_visible(false);
        team_footnote.set_wrap(true);
        limits_section.append(&team_footnote);

        limits_wrapper.append(&limits_section);

        let limits_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);

        content.append(&limits_wrapper);
        content.append(&limits_sep);

        // Footnote.
        let footnote = gtk4::Label::new(Some(
            "Costs are estimates based on public API pricing.\nActual costs may vary based on your plan.",
        ));
        footnote.add_css_class("settings-hint");
        footnote.set_halign(gtk4::Align::Start);
        footnote.set_margin_start(12);
        footnote.set_margin_end(12);
        footnote.set_margin_bottom(12);
        footnote.set_wrap(true);
        content.append(&footnote);

        scrolled.set_child(Some(&content));
        container.append(&scrolled);

        Self {
            container,
            content,
            scope_dropdown,
            session_section,
            session_sep,
            alltime_section,
            alltime_sep,
            limits_wrapper,
            limits_sep,
            limits_header,
            footnote,
            session_cost_label,
            session_tokens_label,
            session_input_label,
            session_output_label,
            session_cache_read_label,
            session_cache_write_label,
            alltime_cost_label,
            alltime_tokens_label,
            alltime_session_count_label,
            alltime_date_range_label,
            limits_section,
            plan_label,
            team_footnote,
            session_window_box,
            session_window_bar,
            session_window_label,
            weekly_window_box,
            weekly_window_bar,
            weekly_window_label,
            no_caps_label,
            close_btn,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Connect the close button callback.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }

    /// Set the scope dropdown from config value ("session" -> 0, "all-time" -> 1).
    pub fn set_scope(&self, scope: &str) {
        let idx = if scope == "session" { 0 } else { 1 };
        self.scope_dropdown.set_selected(idx);
    }

    /// Connect callback for scope dropdown changes. Callback receives "session" or "all-time".
    pub fn connect_scope_changed<F: Fn(&str) + 'static>(&self, f: F) {
        self.scope_dropdown.connect_selected_notify(move |dd| {
            let scope = if dd.selected() == 0 { "session" } else { "all-time" };
            f(scope);
        });
    }

    /// Update the display with a simple CostTracker (backward compat).
    pub fn update_simple(&self, tracker: &CostTracker) {
        let summary = ProjectCostSummary {
            current_session: tracker.clone(),
            all_time: tracker.clone(),
            sessions: Vec::new(),
        };
        self.update(&summary, None);
    }

    /// Update the display with detailed project cost summary and optional limit info.
    pub fn update(&self, summary: &ProjectCostSummary, limit_info: Option<&TokenLimitInfo>) {
        let display_mode = limit_info
            .map(|info| info.display_mode())
            .unwrap_or(CostDisplayMode::Dollar);

        // Reorder sections: in utilization mode, put limits first.
        // GTK4 Box doesn't have reorder — we remove and re-add children.
        self.content.remove(&self.session_section);
        self.content.remove(&self.session_sep);
        self.content.remove(&self.alltime_section);
        self.content.remove(&self.alltime_sep);
        self.content.remove(&self.limits_wrapper);
        self.content.remove(&self.limits_sep);
        self.content.remove(&self.footnote);

        if display_mode == CostDisplayMode::Utilization {
            // Usage first, then cost sections.
            self.limits_header.set_text("USAGE");
            self.footnote.set_text(
                "Cost estimates are based on public API pricing.\nActual costs may vary based on your plan.",
            );
            self.content.append(&self.limits_wrapper);
            self.content.append(&self.limits_sep);
            self.content.append(&self.session_section);
            self.content.append(&self.session_sep);
            self.content.append(&self.alltime_section);
            self.content.append(&self.alltime_sep);
        } else {
            // Cost first (default layout).
            self.limits_header.set_text("TOKEN LIMITS");
            self.footnote.set_text(
                "Costs are estimates based on public API pricing.\nActual costs may vary based on your plan.",
            );
            self.content.append(&self.session_section);
            self.content.append(&self.session_sep);
            self.content.append(&self.alltime_section);
            self.content.append(&self.alltime_sep);
            self.content.append(&self.limits_wrapper);
            self.content.append(&self.limits_sep);
        }
        self.content.append(&self.footnote);

        // Section 1: Current Session.
        let session = &summary.current_session;
        self.session_cost_label
            .set_text(&format!("~${:.2}", session.estimated_cost_usd));
        let session_total = session.input_tokens
            + session.output_tokens
            + session.cache_read_tokens
            + session.cache_write_tokens;
        self.session_tokens_label
            .set_text(&format!("{} total", format_token_count(session_total)));
        self.session_input_label
            .set_text(&format_token_count(session.input_tokens));
        self.session_output_label
            .set_text(&format_token_count(session.output_tokens));
        self.session_cache_read_label
            .set_text(&format_token_count(session.cache_read_tokens));
        self.session_cache_write_label
            .set_text(&format_token_count(session.cache_write_tokens));

        // Section 2: All-time.
        let alltime = &summary.all_time;
        self.alltime_cost_label
            .set_text(&format!("~${:.2}", alltime.estimated_cost_usd));
        let alltime_total = alltime.input_tokens
            + alltime.output_tokens
            + alltime.cache_read_tokens
            + alltime.cache_write_tokens;
        self.alltime_tokens_label
            .set_text(&format!("{} total", format_token_count(alltime_total)));

        let session_count = summary.sessions.len();
        self.alltime_session_count_label
            .set_text(&format!(
                "{session_count} session{}",
                if session_count == 1 { "" } else { "s" }
            ));

        // Date range.
        let first = summary.sessions.iter().filter_map(|s| s.first_timestamp).min();
        let last = summary.sessions.iter().filter_map(|s| s.last_timestamp).max();
        if let (Some(first), Some(last)) = (first, last) {
            self.alltime_date_range_label.set_text(&format!(
                "{} — {}",
                first.format("%Y-%m-%d"),
                last.format("%Y-%m-%d"),
            ));
            self.alltime_date_range_label.set_visible(true);
        } else {
            self.alltime_date_range_label.set_visible(false);
        }

        // Section 3: Token Limits.
        if let Some(info) = limit_info {
            self.plan_label
                .set_text(&format!("Plan: {}", info.plan.display_name()));

            // Show team pool footnote for Team plans.
            self.team_footnote
                .set_visible(info.plan == thane_core::cost_tracker::Plan::Team);

            if info.has_caps {
                self.session_window_box.set_visible(true);
                self.weekly_window_box.set_visible(true);
                self.no_caps_label.set_visible(false);

                // 5-hour window.
                if let Some(ref w) = info.five_hour {
                    let fraction = (w.utilization / 100.0).clamp(0.0, 1.0);
                    self.session_window_bar.set_fraction(fraction);
                    self.session_window_label.set_text(&format_usage_label(w.utilization, w.resets_at));
                } else {
                    self.session_window_bar.set_fraction(0.0);
                    self.session_window_label.set_text("No usage data");
                }

                // Weekly cap.
                if let Some(ref w) = info.seven_day {
                    let fraction = (w.utilization / 100.0).clamp(0.0, 1.0);
                    self.weekly_window_bar.set_fraction(fraction);
                    self.weekly_window_label.set_text(&format_usage_label(w.utilization, w.resets_at));
                } else {
                    self.weekly_window_bar.set_fraction(0.0);
                    self.weekly_window_label.set_text("No usage data");
                }
            } else {
                self.session_window_box.set_visible(false);
                self.weekly_window_box.set_visible(false);
                self.no_caps_label.set_visible(true);
            }
        } else {
            // No limit info available — hide the limits section content.
            self.plan_label.set_text("Plan: Unknown");
            self.team_footnote.set_visible(false);
            self.session_window_box.set_visible(false);
            self.weekly_window_box.set_visible(false);
            self.no_caps_label.set_visible(false);
        }
    }
}

fn section_header(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("token-section-header");
    label.set_halign(gtk4::Align::Start);
    label.set_margin_start(12);
    label.set_margin_top(12);
    label.set_margin_bottom(4);
    label
}

fn create_detail_row(label_text: &str, value_label: &gtk4::Label) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    let label = gtk4::Label::new(Some(label_text));
    label.add_css_class("token-detail-label");
    label.set_hexpand(true);
    label.set_halign(gtk4::Align::Start);
    row.append(&label);

    value_label.add_css_class("token-detail-value");
    value_label.set_halign(gtk4::Align::End);
    row.append(value_label);

    row
}

/// Format a usage label showing utilization percentage and reset countdown.
fn format_usage_label(utilization: f64, resets_at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let remaining = resets_at - now;

    let countdown = if remaining.num_seconds() <= 0 {
        "resetting...".to_string()
    } else if remaining.num_hours() >= 24 {
        let days = remaining.num_days();
        let hours = remaining.num_hours() % 24;
        format!("resets in {days}d {hours}h")
    } else if remaining.num_hours() >= 1 {
        let hours = remaining.num_hours();
        let mins = remaining.num_minutes() % 60;
        format!("resets in {hours}h {mins}m")
    } else {
        let mins = remaining.num_minutes();
        format!("resets in {mins}m")
    };

    format!("{:.0}% used — {countdown}", utilization)
}

fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        format!("{count}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_format_token_count() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(1_500), "1.5K");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }

    #[test]
    fn test_format_token_count_boundaries() {
        // Just below 1K threshold.
        assert_eq!(format_token_count(999), "999");
        // Exactly at 1K threshold.
        assert_eq!(format_token_count(1_000), "1.0K");
        // Just below 1M threshold.
        assert_eq!(format_token_count(999_999), "1000.0K");
        // Exactly at 1M threshold.
        assert_eq!(format_token_count(1_000_000), "1.0M");
    }

    #[test]
    fn test_format_usage_label_past_reset() {
        let past = Utc::now() - Duration::minutes(5);
        let label = format_usage_label(50.0, past);
        assert!(label.contains("resetting..."), "Expected 'resetting...', got: {label}");
        assert!(label.contains("50%"), "Expected '50%', got: {label}");
    }

    #[test]
    fn test_format_usage_label_minutes_only() {
        let future = Utc::now() + Duration::minutes(45);
        let label = format_usage_label(30.0, future);
        assert!(label.contains("resets in"), "Expected 'resets in', got: {label}");
        assert!(label.contains("m"), "Expected minutes, got: {label}");
        // Should NOT contain "h" since it's less than 1 hour.
        assert!(!label.contains("h"), "Should not contain hours, got: {label}");
    }

    #[test]
    fn test_format_usage_label_hours_and_minutes() {
        let future = Utc::now() + Duration::hours(3) + Duration::minutes(20);
        let label = format_usage_label(75.0, future);
        assert!(label.contains("resets in"), "Expected 'resets in', got: {label}");
        assert!(label.contains("3h"), "Expected '3h', got: {label}");
        assert!(label.contains("m"), "Expected minutes, got: {label}");
    }

    #[test]
    fn test_format_usage_label_days() {
        // Use a large enough offset that sub-second timing doesn't change the result.
        let future = Utc::now() + Duration::days(2) + Duration::hours(5) + Duration::minutes(30);
        let label = format_usage_label(10.0, future);
        assert!(label.contains("resets in"), "Expected 'resets in', got: {label}");
        assert!(label.contains("2d"), "Expected '2d', got: {label}");
        assert!(label.contains("5h"), "Expected '5h', got: {label}");
    }
}
