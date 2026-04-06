use std::cell::RefCell;
use std::rc::Rc;

use chrono::{Duration, Utc};
use thane_core::audit::{AuditEvent, AuditSeverity};
use gtk4::prelude::*;

/// Date range filter for audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateRange {
    Today,
    ThreeDays,
    SevenDays,
    All,
}

impl DateRange {
    /// Returns the label shown in the dropdown.
    pub fn label(self) -> &'static str {
        match self {
            DateRange::Today => "Today",
            DateRange::ThreeDays => "3 Days",
            DateRange::SevenDays => "7 Days",
            DateRange::All => "All Time",
        }
    }

    /// All variants in display order.
    pub const ALL: [DateRange; 4] = [
        DateRange::Today,
        DateRange::ThreeDays,
        DateRange::SevenDays,
        DateRange::All,
    ];

    /// Convert dropdown index to enum variant.
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => DateRange::Today,
            1 => DateRange::ThreeDays,
            2 => DateRange::SevenDays,
            _ => DateRange::All,
        }
    }

    /// Returns true if the given timestamp passes this date range filter.
    pub fn includes(self, timestamp: &chrono::DateTime<Utc>) -> bool {
        match self {
            DateRange::All => true,
            DateRange::Today => *timestamp >= Utc::now() - Duration::days(1),
            DateRange::ThreeDays => *timestamp >= Utc::now() - Duration::days(3),
            DateRange::SevenDays => *timestamp >= Utc::now() - Duration::days(7),
        }
    }
}

/// A panel showing chronological audit event list with severity filtering
/// and free-text search.
pub struct AuditPanel {
    container: gtk4::Box,
    list_box: gtk4::ListBox,
    header: gtk4::Box,
    filter_box: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    severity_filter: std::cell::Cell<Option<AuditSeverity>>,
    /// Current date range filter.
    date_range: std::cell::Cell<DateRange>,
    /// Current search text (lowercased). Shared with the search callback.
    search_text: RefCell<String>,
    export_btn: gtk4::Button,
    close_btn: gtk4::Button,
    date_range_dropdown: gtk4::DropDown,
    /// Callback invoked when a row is double-clicked.
    row_activated: Rc<RefCell<Option<Box<dyn Fn(AuditEvent)>>>>,
}

impl Default for AuditPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("audit-panel");
        container.set_width_request(360);

        // Header with title and clear button.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("Audit Trail"));
        title.add_css_class("workspace-title");
        title.set_hexpand(true);
        title.set_halign(gtk4::Align::Start);
        header.append(&title);

        let clear_btn = gtk4::Button::with_label("Clear");
        clear_btn.add_css_class("flat");
        header.append(&clear_btn);

        let export_btn = gtk4::Button::with_label("Export");
        export_btn.add_css_class("flat");
        export_btn.set_tooltip_text(Some("Export audit log as JSON"));
        header.append(&export_btn);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close"));
        header.append(&close_btn);

        container.append(&header);

        // Filter buttons row.
        let filter_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        filter_box.set_margin_start(12);
        filter_box.set_margin_end(12);
        filter_box.set_margin_bottom(4);

        let all_btn = gtk4::ToggleButton::with_label("All");
        all_btn.set_active(true);
        all_btn.add_css_class("flat");
        all_btn.add_css_class("audit-filter-btn");
        filter_box.append(&all_btn);

        let warn_btn = gtk4::ToggleButton::with_label("Warn+");
        warn_btn.add_css_class("flat");
        warn_btn.add_css_class("audit-filter-btn");
        filter_box.append(&warn_btn);

        let alert_btn = gtk4::ToggleButton::with_label("Alert+");
        alert_btn.add_css_class("flat");
        alert_btn.add_css_class("audit-filter-btn");
        filter_box.append(&alert_btn);

        let critical_btn = gtk4::ToggleButton::with_label("Critical");
        critical_btn.add_css_class("flat");
        critical_btn.add_css_class("audit-filter-btn");
        filter_box.append(&critical_btn);

        // Group toggle buttons.
        warn_btn.set_group(Some(&all_btn));
        alert_btn.set_group(Some(&all_btn));
        critical_btn.set_group(Some(&all_btn));

        container.append(&filter_box);

        // Date range dropdown.
        let date_range_labels: Vec<&str> = DateRange::ALL.iter().map(|d| d.label()).collect();
        let string_list = gtk4::StringList::new(&date_range_labels);
        let date_range_dropdown = gtk4::DropDown::new(Some(string_list), gtk4::Expression::NONE);
        date_range_dropdown.set_selected(3); // Default: "All Time"
        date_range_dropdown.set_margin_start(12);
        date_range_dropdown.set_margin_end(12);
        date_range_dropdown.set_margin_bottom(4);
        container.append(&date_range_dropdown);

        // Search / filter input.
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search events..."));
        search_entry.set_margin_start(12);
        search_entry.set_margin_end(12);
        search_entry.set_margin_bottom(4);
        container.append(&search_entry);

        // Retention hint label.
        let retention_label = gtk4::Label::new(Some("Logs retained for 7 days"));
        retention_label.add_css_class("dim-label");
        retention_label.set_halign(gtk4::Align::Start);
        retention_label.set_margin_start(12);
        retention_label.set_margin_end(12);
        retention_label.set_margin_bottom(4);
        container.append(&retention_label);

        // Separator.
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        container.append(&sep);

        // Scrollable list of audit events.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&list_box));

        container.append(&scrolled);

        Self {
            container,
            list_box,
            header,
            filter_box,
            search_entry,
            severity_filter: std::cell::Cell::new(None),
            date_range: std::cell::Cell::new(DateRange::All),
            search_text: RefCell::new(String::new()),
            export_btn,
            close_btn,
            date_range_dropdown,
            row_activated: Rc::new(RefCell::new(None)),
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Get the current severity filter.
    pub fn severity_filter(&self) -> Option<AuditSeverity> {
        self.severity_filter.get()
    }

    /// Set the severity filter and return it (for use in callbacks).
    pub fn set_severity_filter(&self, filter: Option<AuditSeverity>) {
        self.severity_filter.set(filter);
    }

    /// Get the current search text.
    pub fn search_text(&self) -> String {
        self.search_text.borrow().clone()
    }

    /// Set the search text (lowercased) used for filtering.
    pub fn set_search_text(&self, text: &str) {
        *self.search_text.borrow_mut() = text.to_lowercase();
    }

    /// Get the current date range filter.
    pub fn date_range(&self) -> DateRange {
        self.date_range.get()
    }

    /// Set the date range filter.
    pub fn set_date_range(&self, range: DateRange) {
        self.date_range.set(range);
    }

    /// Update the displayed audit events with current severity, date range, and text filters applied.
    pub fn set_events(&self, events: &[AuditEvent]) {
        // Clear existing rows.
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        let filter = self.severity_filter.get();
        let date_range = self.date_range.get();
        let search = self.search_text.borrow();
        let filtered: Vec<&AuditEvent> = events
            .iter()
            .filter(|e| match filter {
                None => true,
                Some(min) => e.severity >= min,
            })
            .filter(|e| date_range.includes(&e.timestamp))
            .filter(|e| {
                if search.is_empty() {
                    return true;
                }
                // Match against description, event type debug repr, and metadata string.
                let desc_lower = e.description.to_lowercase();
                if desc_lower.contains(search.as_str()) {
                    return true;
                }
                let type_str = format!("{:?}", e.event_type).to_lowercase();
                if type_str.contains(search.as_str()) {
                    return true;
                }
                let meta_str = e.metadata.to_string().to_lowercase();
                meta_str.contains(search.as_str())
            })
            .collect();

        if filtered.is_empty() {
            let empty_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            empty_box.set_margin_top(40);
            empty_box.set_margin_bottom(20);
            empty_box.set_halign(gtk4::Align::Center);

            let icon = gtk4::Image::from_icon_name("security-high-symbolic");
            icon.set_pixel_size(32);
            icon.set_opacity(0.3);
            empty_box.append(&icon);

            let title = gtk4::Label::new(Some("No security events"));
            title.add_css_class("panel-meta");
            empty_box.append(&title);

            let hint = gtk4::Label::new(Some(
                "Audit events will appear here as they occur",
            ));
            hint.add_css_class("queue-empty-hint");
            hint.set_wrap(true);
            empty_box.append(&hint);

            self.list_box.append(&empty_box);
            return;
        }

        // Add events in reverse chronological order.
        for event in filtered.iter().rev() {
            let row = create_audit_row(event, &self.row_activated);
            self.list_box.append(&row);
        }
    }

    /// Connect a callback invoked when an audit row is double-clicked.
    pub fn connect_row_activated<F: Fn(AuditEvent) + 'static>(&self, f: F) {
        *self.row_activated.borrow_mut() = Some(Box::new(f));
    }

    /// Connect the clear button callback.
    pub fn connect_clear<F: Fn() + 'static>(&self, f: F) {
        // Clear button is the second-to-last child of the header (before close_btn).
        let mut child = self.header.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(button) = widget.clone().downcast::<gtk4::Button>() {
                // The clear button has label "Clear", not an icon.
                if button.label().is_some_and(|l| l == "Clear") {
                    button.connect_clicked(move |_| f());
                    break;
                }
            }
            child = next;
        }
    }

    /// Connect the export button callback.
    pub fn connect_export<F: Fn() + 'static>(&self, f: F) {
        self.export_btn.connect_clicked(move |_| f());
    }

    /// Connect the close button callback.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }

    /// Connect the search entry's changed callback.
    /// The callback is invoked with the current search text whenever the user types.
    pub fn connect_search<F: Fn(&str) + 'static>(&self, f: F) {
        self.search_entry.connect_search_changed(move |entry| {
            let text = entry.text().to_string();
            f(&text);
        });
    }

    /// Connect date range dropdown callback. The callback receives the new date range.
    pub fn connect_date_range<F: Fn(DateRange) + 'static>(&self, f: F) {
        self.date_range_dropdown.connect_selected_notify(move |dd| {
            let range = DateRange::from_index(dd.selected());
            f(range);
        });
    }

    /// Connect filter button callbacks. The callback receives the new severity filter.
    pub fn connect_filter<F: Fn(Option<AuditSeverity>) + Clone + 'static>(&self, f: F) {
        // The filter buttons are the children of filter_box.
        let mut child = self.filter_box.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            if let Ok(toggle) = widget.clone().downcast::<gtk4::ToggleButton>() {
                let cb = f.clone();
                let filter = match index {
                    0 => None,
                    1 => Some(AuditSeverity::Warning),
                    2 => Some(AuditSeverity::Alert),
                    3 => Some(AuditSeverity::Critical),
                    _ => None,
                };
                toggle.connect_toggled(move |btn| {
                    if btn.is_active() {
                        cb(filter);
                    }
                });
            }
            child = widget.next_sibling();
            index += 1;
        }
    }
}

/// Create a single audit event row widget with double-click gesture.
fn create_audit_row(
    event: &AuditEvent,
    row_activated: &Rc<RefCell<Option<Box<dyn Fn(AuditEvent)>>>>,
) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    row.add_css_class("audit-item");

    // Severity-based CSS class.
    match event.severity {
        AuditSeverity::Critical => row.add_css_class("audit-item-critical"),
        AuditSeverity::Alert => row.add_css_class("audit-item-alert"),
        AuditSeverity::Warning => row.add_css_class("audit-item-warning"),
        AuditSeverity::Info => {}
    }

    // Top row: severity badge + event type + timestamp.
    let top_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

    let severity_label = gtk4::Label::new(Some(severity_text(event.severity)));
    severity_label.add_css_class("audit-severity");
    severity_label.add_css_class(severity_css_class(event.severity));
    top_row.append(&severity_label);

    if let Some(ref agent) = event.agent_name {
        let agent_label = gtk4::Label::new(Some(agent));
        agent_label.add_css_class("audit-agent-badge");
        top_row.append(&agent_label);
    }

    let type_label = gtk4::Label::new(Some(&format!("{:?}", event.event_type)));
    type_label.add_css_class("audit-event-type");
    type_label.set_hexpand(true);
    type_label.set_halign(gtk4::Align::Start);
    type_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    top_row.append(&type_label);

    let time = gtk4::Label::new(Some(
        &event.timestamp.format("%H:%M:%S").to_string(),
    ));
    time.add_css_class("notification-time");
    top_row.append(&time);

    row.append(&top_row);

    // Description.
    let desc = gtk4::Label::new(Some(&event.description));
    desc.add_css_class("audit-description");
    desc.set_halign(gtk4::Align::Start);
    desc.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    desc.set_max_width_chars(50);
    desc.set_wrap(true);
    desc.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    desc.set_lines(2);
    row.append(&desc);

    // Double-click gesture to open detail dialog.
    let gesture = gtk4::GestureClick::new();
    let event_clone = event.clone();
    let cb = row_activated.clone();
    gesture.connect_released(move |g, n_press, _x, _y| {
        if n_press == 2 {
            g.set_state(gtk4::EventSequenceState::Claimed);
            if let Some(ref f) = *cb.borrow() {
                f(event_clone.clone());
            }
        }
    });
    row.add_controller(gesture);

    row
}

/// Show a modal dialog with full details for an audit event.
pub fn show_audit_detail_dialog(parent: &gtk4::ApplicationWindow, event: &AuditEvent) {
    let (dialog, vbox) = crate::window::styled_dialog(parent, "Audit Event Detail", 500, 420);

    // Make the dialog content scrollable.
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    content.set_margin_top(4);
    content.set_margin_bottom(4);

    // Top row: severity badge + event type.
    let top_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let severity_label = gtk4::Label::new(Some(severity_text(event.severity)));
    severity_label.add_css_class("audit-severity");
    severity_label.add_css_class(severity_css_class(event.severity));
    top_row.append(&severity_label);

    let type_label = gtk4::Label::new(Some(&format!("{:?}", event.event_type)));
    type_label.add_css_class("audit-event-type");
    type_label.set_halign(gtk4::Align::Start);
    top_row.append(&type_label);
    content.append(&top_row);

    // Timestamp.
    let ts_label = gtk4::Label::new(Some("Timestamp:"));
    ts_label.add_css_class("audit-detail-field-label");
    ts_label.set_halign(gtk4::Align::Start);
    content.append(&ts_label);

    let ts_value = gtk4::Label::new(Some(
        &event.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    ));
    ts_value.set_halign(gtk4::Align::Start);
    ts_value.set_selectable(true);
    content.append(&ts_value);

    // Description.
    let desc_label = gtk4::Label::new(Some("Description:"));
    desc_label.add_css_class("audit-detail-field-label");
    desc_label.set_halign(gtk4::Align::Start);
    content.append(&desc_label);

    let desc_value = gtk4::Label::new(Some(&event.description));
    desc_value.set_halign(gtk4::Align::Start);
    desc_value.set_wrap(true);
    desc_value.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    desc_value.set_selectable(true);
    content.append(&desc_value);

    // Workspace ID.
    let ws_label = gtk4::Label::new(Some("Workspace ID:"));
    ws_label.add_css_class("audit-detail-field-label");
    ws_label.set_halign(gtk4::Align::Start);
    content.append(&ws_label);

    let ws_value = gtk4::Label::new(Some(&event.workspace_id.to_string()));
    ws_value.set_halign(gtk4::Align::Start);
    ws_value.set_selectable(true);
    content.append(&ws_value);

    // Panel ID.
    let panel_label = gtk4::Label::new(Some("Panel ID:"));
    panel_label.add_css_class("audit-detail-field-label");
    panel_label.set_halign(gtk4::Align::Start);
    content.append(&panel_label);

    let panel_text = event
        .panel_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "N/A".to_string());
    let panel_value = gtk4::Label::new(Some(&panel_text));
    panel_value.set_halign(gtk4::Align::Start);
    panel_value.set_selectable(true);
    content.append(&panel_value);

    // Metadata.
    let meta_label = gtk4::Label::new(Some("Metadata:"));
    meta_label.add_css_class("audit-detail-field-label");
    meta_label.set_halign(gtk4::Align::Start);
    content.append(&meta_label);

    let meta_json = serde_json::to_string_pretty(&event.metadata).unwrap_or_default();
    let meta_buffer = gtk4::TextBuffer::new(None);
    meta_buffer.set_text(&meta_json);

    let meta_view = gtk4::TextView::with_buffer(&meta_buffer);
    meta_view.set_editable(false);
    meta_view.set_cursor_visible(false);
    meta_view.set_monospace(true);
    meta_view.add_css_class("audit-detail-metadata");
    meta_view.set_wrap_mode(gtk4::WrapMode::WordChar);

    // Compute a reasonable height for the metadata view (min 60, max 200).
    let line_count = meta_json.lines().count();
    let meta_height = ((line_count as i32) * 18).clamp(60, 200);
    meta_view.set_size_request(-1, meta_height);

    content.append(&meta_view);

    scrolled.set_child(Some(&content));
    vbox.append(&scrolled);

    // Close button.
    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    let dialog_ref = dialog.downgrade();
    close_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_ref.upgrade() {
            d.close();
        }
    });
    vbox.append(&close_btn);

    dialog.present();
}

fn severity_text(severity: AuditSeverity) -> &'static str {
    match severity {
        AuditSeverity::Info => "INFO",
        AuditSeverity::Warning => "WARN",
        AuditSeverity::Alert => "ALERT",
        AuditSeverity::Critical => "CRIT",
    }
}

fn severity_css_class(severity: AuditSeverity) -> &'static str {
    match severity {
        AuditSeverity::Info => "audit-severity-info",
        AuditSeverity::Warning => "audit-severity-warning",
        AuditSeverity::Alert => "audit-severity-alert",
        AuditSeverity::Critical => "audit-severity-critical",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
    use uuid::Uuid;

    /// Helper to create an AuditEvent with a given timestamp offset (days ago).
    fn make_event(days_ago: i64, agent_name: Option<&str>) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now() - Duration::days(days_ago),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: format!("event {} days ago", days_ago),
            metadata: serde_json::json!({}),
            agent_name: agent_name.map(|s| s.to_string()),
            prev_hash: String::new(),
        }
    }

    #[test]
    fn test_date_range_from_index() {
        assert_eq!(DateRange::from_index(0), DateRange::Today);
        assert_eq!(DateRange::from_index(1), DateRange::ThreeDays);
        assert_eq!(DateRange::from_index(2), DateRange::SevenDays);
        assert_eq!(DateRange::from_index(3), DateRange::All);
        assert_eq!(DateRange::from_index(99), DateRange::All);
    }

    #[test]
    fn test_date_range_labels() {
        assert_eq!(DateRange::Today.label(), "Today");
        assert_eq!(DateRange::ThreeDays.label(), "3 Days");
        assert_eq!(DateRange::SevenDays.label(), "7 Days");
        assert_eq!(DateRange::All.label(), "All Time");
    }

    #[test]
    fn test_date_range_all_constant() {
        assert_eq!(DateRange::ALL.len(), 4);
        assert_eq!(DateRange::ALL[0], DateRange::Today);
        assert_eq!(DateRange::ALL[3], DateRange::All);
    }

    #[test]
    fn test_date_range_filtering() {
        let now = Utc::now();
        let half_day_ago = now - Duration::hours(12);
        let two_days_ago = now - Duration::days(2);
        let five_days_ago = now - Duration::days(5);
        let ten_days_ago = now - Duration::days(10);

        // Today: includes events within the last 24 hours
        assert!(DateRange::Today.includes(&now));
        assert!(DateRange::Today.includes(&half_day_ago));
        assert!(!DateRange::Today.includes(&two_days_ago));
        assert!(!DateRange::Today.includes(&five_days_ago));
        assert!(!DateRange::Today.includes(&ten_days_ago));

        // ThreeDays: includes events within the last 3 days
        assert!(DateRange::ThreeDays.includes(&now));
        assert!(DateRange::ThreeDays.includes(&half_day_ago));
        assert!(DateRange::ThreeDays.includes(&two_days_ago));
        assert!(!DateRange::ThreeDays.includes(&five_days_ago));
        assert!(!DateRange::ThreeDays.includes(&ten_days_ago));

        // SevenDays: includes events within the last 7 days
        assert!(DateRange::SevenDays.includes(&now));
        assert!(DateRange::SevenDays.includes(&two_days_ago));
        assert!(DateRange::SevenDays.includes(&five_days_ago));
        assert!(!DateRange::SevenDays.includes(&ten_days_ago));

        // All: includes everything
        assert!(DateRange::All.includes(&now));
        assert!(DateRange::All.includes(&ten_days_ago));
    }

    #[test]
    fn test_agent_badge_presence() {
        // Events with agent_name should produce a badge-bearing row
        let with_agent = make_event(0, Some("claude"));
        assert!(with_agent.agent_name.is_some());
        assert_eq!(with_agent.agent_name.as_deref(), Some("claude"));

        let without_agent = make_event(0, None);
        assert!(without_agent.agent_name.is_none());
    }

    #[test]
    fn test_date_range_filtering_with_events() {
        let events = vec![
            make_event(0, Some("claude")),   // today
            make_event(2, None),             // 2 days ago
            make_event(5, Some("codex")),    // 5 days ago
            make_event(10, None),            // 10 days ago
        ];

        let today_events: Vec<_> = events
            .iter()
            .filter(|e| DateRange::Today.includes(&e.timestamp))
            .collect();
        assert_eq!(today_events.len(), 1);

        let three_day_events: Vec<_> = events
            .iter()
            .filter(|e| DateRange::ThreeDays.includes(&e.timestamp))
            .collect();
        assert_eq!(three_day_events.len(), 2);

        let seven_day_events: Vec<_> = events
            .iter()
            .filter(|e| DateRange::SevenDays.includes(&e.timestamp))
            .collect();
        assert_eq!(seven_day_events.len(), 3);

        let all_events: Vec<_> = events
            .iter()
            .filter(|e| DateRange::All.includes(&e.timestamp))
            .collect();
        assert_eq!(all_events.len(), 4);
    }
}
