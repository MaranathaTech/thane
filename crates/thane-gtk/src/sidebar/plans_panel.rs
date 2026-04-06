use std::cell::RefCell;
use std::rc::Rc;

use thane_core::agent_queue::{QueueEntry, QueueEntryStatus};
use thane_core::queue_executor::shorten_model_name;
use gtk4::prelude::*;

/// A right-side panel showing completed queue task history ("Plans").
pub struct PlansPanel {
    container: gtk4::Box,
    list_box: gtk4::ListBox,
    status_label: gtk4::Label,
    close_btn: gtk4::Button,
    empty_box: gtk4::Box,
    row_activated: Rc<RefCell<Option<Box<dyn Fn(QueueEntry)>>>>,
    /// Banner shown when Claude CLI is not installed.
    claude_missing_banner: gtk4::Box,
}

impl Default for PlansPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl PlansPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("plans-panel");
        container.set_width_request(380);

        // Header.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("Processed"));
        title.add_css_class("workspace-title");
        title.set_halign(gtk4::Align::Start);
        header.append(&title);

        let status_label = gtk4::Label::new(Some("0 entries"));
        status_label.add_css_class("panel-meta");
        status_label.set_hexpand(true);
        status_label.set_halign(gtk4::Align::End);
        status_label.set_margin_end(4);
        header.append(&status_label);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close"));
        header.append(&close_btn);

        container.append(&header);

        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        container.append(&sep);

        // Claude CLI missing banner — hidden by default.
        let claude_missing_banner = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        claude_missing_banner.add_css_class("claude-missing-banner");
        claude_missing_banner.set_visible(false);

        let missing_icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
        missing_icon.set_pixel_size(16);
        claude_missing_banner.append(&missing_icon);

        let missing_label = gtk4::Label::new(Some("Claude CLI not found. Install it to use the agent queue."));
        missing_label.add_css_class("claude-missing-text");
        missing_label.set_halign(gtk4::Align::Start);
        missing_label.set_wrap(true);
        claude_missing_banner.append(&missing_label);

        container.append(&claude_missing_banner);

        // Scrollable list.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&list_box));

        container.append(&scrolled);

        // Empty state box.
        let empty_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        empty_box.set_halign(gtk4::Align::Center);
        empty_box.set_valign(gtk4::Align::Start);
        empty_box.set_margin_top(40);

        let empty_icon = gtk4::Image::from_icon_name("document-open-recent-symbolic");
        empty_icon.set_pixel_size(32);
        empty_icon.set_opacity(0.3);
        empty_box.append(&empty_icon);

        let empty_title = gtk4::Label::new(Some("No processed tasks"));
        empty_title.add_css_class("panel-meta");
        empty_box.append(&empty_title);

        let empty_hint =
            gtk4::Label::new(Some("Completed queue tasks will appear here"));
        empty_hint.add_css_class("queue-empty-hint");
        empty_box.append(&empty_hint);

        Self {
            container,
            list_box,
            status_label,
            close_btn,
            empty_box,
            row_activated: Rc::new(RefCell::new(None)),
            claude_missing_banner,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Show or hide the "Claude CLI not found" banner.
    pub fn set_claude_missing(&self, missing: bool) {
        self.claude_missing_banner.set_visible(missing);
    }

    /// Connect close button.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }

    /// Connect row double-click callback.
    pub fn connect_row_activated<F: Fn(QueueEntry) + 'static>(&self, f: F) {
        *self.row_activated.borrow_mut() = Some(Box::new(f));
    }

    /// Update the panel with completed queue entries (shown in reverse chronological order).
    pub fn update(&self, entries: &[QueueEntry]) {
        // Clear existing rows.
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        if entries.is_empty() {
            self.status_label.set_text("0 entries");
            self.list_box.append(&self.empty_box);
            return;
        }

        self.status_label
            .set_text(&format!("{} entries", entries.len()));

        // Show in reverse chronological order.
        for entry in entries.iter().rev() {
            let row = create_plan_row(entry, &self.row_activated);
            self.list_box.append(&row);
        }
    }
}

/// Create a single plan entry row with double-click gesture.
fn create_plan_row(
    entry: &QueueEntry,
    row_activated: &Rc<RefCell<Option<Box<dyn Fn(QueueEntry)>>>>,
) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    row.add_css_class("queue-item");

    // Top row: content text (truncated) + status badge.
    let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

    let content_text: String = entry.content.chars().take(60).collect();
    let content_text = if entry.content.len() > 60 {
        format!("{content_text}...")
    } else {
        content_text
    };
    let content_label = gtk4::Label::new(Some(&content_text));
    content_label.add_css_class("queue-content");
    content_label.set_halign(gtk4::Align::Start);
    content_label.set_hexpand(true);
    content_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    top.append(&content_label);

    let status_badge = gtk4::Label::new(Some(status_text_for(&entry.status)));
    status_badge.add_css_class(status_css_for(&entry.status));
    top.append(&status_badge);

    row.append(&top);

    // Bottom row: timestamp + cost.
    let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

    let time_str = entry
        .completed_at
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| entry.created_at.format("%Y-%m-%d %H:%M").to_string());

    let cost = entry.tokens_used.estimated_cost_usd;
    let model_short = entry
        .tokens_used
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .map(shorten_model_name);
    let meta_text = match (cost > 0.0, model_short) {
        (true, Some(model)) => format!("{time_str} \u{00b7} ${cost:.4} ({model})"),
        (true, None) => format!("{time_str} \u{00b7} ${cost:.4}"),
        (false, Some(model)) => format!("{time_str} \u{00b7} {model}"),
        (false, None) => time_str,
    };

    let meta = gtk4::Label::new(Some(&meta_text));
    meta.add_css_class("panel-meta");
    meta.set_hexpand(true);
    meta.set_halign(gtk4::Align::Start);
    bottom.append(&meta);

    row.append(&bottom);

    // Double-click gesture.
    let gesture = gtk4::GestureClick::new();
    let entry_clone = entry.clone();
    let cb = row_activated.clone();
    gesture.connect_released(move |g, n_press, _x, _y| {
        if n_press == 2 {
            g.set_state(gtk4::EventSequenceState::Claimed);
            if let Some(ref f) = *cb.borrow() {
                f(entry_clone.clone());
            }
        }
    });
    row.add_controller(gesture);

    row
}

/// Show a modal dialog with full details for a completed plan entry.
pub fn show_plan_detail_dialog(parent: &gtk4::ApplicationWindow, entry: &QueueEntry) {
    let (dialog, vbox) = crate::window::styled_dialog(parent, "Plan Detail", 600, 500);

    // Make the dialog content scrollable.
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    content.set_margin_top(4);
    content.set_margin_bottom(4);

    // Status badge.
    let status_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let status_label = gtk4::Label::new(Some(status_text_for(&entry.status)));
    status_label.add_css_class(status_css_for(&entry.status));
    status_row.append(&status_label);
    content.append(&status_row);

    // Content (full text).
    add_field(&content, "Content:");
    let content_value = gtk4::Label::new(Some(&entry.content));
    content_value.set_halign(gtk4::Align::Start);
    content_value.set_wrap(true);
    content_value.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    content_value.set_selectable(true);
    content.append(&content_value);

    // Timestamps.
    add_field(&content, "Created:");
    let created_value = gtk4::Label::new(Some(
        &entry.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    ));
    created_value.set_halign(gtk4::Align::Start);
    created_value.set_selectable(true);
    content.append(&created_value);

    if let Some(started) = entry.started_at {
        add_field(&content, "Started:");
        let started_value =
            gtk4::Label::new(Some(&started.format("%Y-%m-%d %H:%M:%S UTC").to_string()));
        started_value.set_halign(gtk4::Align::Start);
        started_value.set_selectable(true);
        content.append(&started_value);
    }

    if let Some(completed) = entry.completed_at {
        add_field(&content, "Completed:");
        let completed_value =
            gtk4::Label::new(Some(&completed.format("%Y-%m-%d %H:%M:%S UTC").to_string()));
        completed_value.set_halign(gtk4::Align::Start);
        completed_value.set_selectable(true);
        content.append(&completed_value);
    }

    // Error (if any).
    if let Some(ref error) = entry.error {
        add_field(&content, "Error:");
        let error_value = gtk4::Label::new(Some(error));
        error_value.set_halign(gtk4::Align::Start);
        error_value.set_wrap(true);
        error_value.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        error_value.set_selectable(true);
        error_value.add_css_class("queue-status-failed");
        content.append(&error_value);
    }

    // Token usage.
    let tokens = &entry.tokens_used;
    if tokens.input_tokens > 0 || tokens.output_tokens > 0 {
        add_field(&content, "Token Usage:");
        let usage_text = format!(
            "Input: {} \u{00b7} Output: {} \u{00b7} Cost: ${:.4}",
            tokens.input_tokens, tokens.output_tokens, tokens.estimated_cost_usd
        );
        let usage_value = gtk4::Label::new(Some(&usage_text));
        usage_value.set_halign(gtk4::Align::Start);
        usage_value.set_selectable(true);
        content.append(&usage_value);
    }

    // Output log.
    let log_path = plan_output_log_path(entry);
    let log_content = std::fs::read_to_string(&log_path).unwrap_or_default();

    if !log_content.is_empty() {
        add_field(&content, "Output:");

        let log_buffer = gtk4::TextBuffer::new(None);
        log_buffer.set_text(&log_content);

        let log_view = gtk4::TextView::with_buffer(&log_buffer);
        log_view.set_editable(false);
        log_view.set_cursor_visible(false);
        log_view.set_monospace(true);
        log_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        log_view.add_css_class("audit-detail-metadata");

        // Compute a reasonable height (min 100, max 300).
        let line_count = log_content.lines().count();
        let log_height = ((line_count as i32) * 18).clamp(100, 300);
        log_view.set_size_request(-1, log_height);

        content.append(&log_view);
    } else {
        add_field(&content, "Output:");
        let no_log = gtk4::Label::new(Some("No output log found"));
        no_log.set_halign(gtk4::Align::Start);
        no_log.add_css_class("panel-meta");
        content.append(&no_log);
    }

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

/// Build the path to the plan output log for a queue entry.
fn plan_output_log_path(entry: &QueueEntry) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home)
        .join("thane")
        .join("plans")
        .join(entry.id.to_string())
        .join("output.log")
}

/// Helper to add a field label.
fn add_field(container: &gtk4::Box, label: &str) {
    let lbl = gtk4::Label::new(Some(label));
    lbl.add_css_class("audit-detail-field-label");
    lbl.set_halign(gtk4::Align::Start);
    container.append(&lbl);
}

fn status_text_for(status: &QueueEntryStatus) -> &'static str {
    match status {
        QueueEntryStatus::Completed => "Done",
        QueueEntryStatus::Failed => "Failed",
        QueueEntryStatus::Cancelled => "Cancelled",
        QueueEntryStatus::Queued => "Queued",
        QueueEntryStatus::Running => "Running",
        QueueEntryStatus::PausedTokenLimit | QueueEntryStatus::PausedByUser => "Paused",
    }
}

fn status_css_for(status: &QueueEntryStatus) -> &'static str {
    match status {
        QueueEntryStatus::Completed => "queue-status-completed",
        QueueEntryStatus::Failed => "queue-status-failed",
        QueueEntryStatus::Cancelled => "queue-status-cancelled",
        QueueEntryStatus::Queued => "queue-status-queued",
        QueueEntryStatus::Running => "queue-status-running",
        QueueEntryStatus::PausedTokenLimit | QueueEntryStatus::PausedByUser => "queue-status-paused",
    }
}
