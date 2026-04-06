use std::cell::RefCell;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use thane_core::agent_queue::{AgentQueue, QueueEntryStatus};
use thane_core::queue_executor::shorten_model_name;
use thane_core::sandbox::EnforcementLevel;
use gtk4::prelude::*;
use uuid::Uuid;

/// A right-side panel showing the agent execution queue.
#[allow(clippy::type_complexity)]
pub struct AgentQueuePanel {
    container: gtk4::Box,
    list_box: gtk4::ListBox,
    submit_entry: gtk4::Entry,
    submit_btn: gtk4::Button,
    submit_row: gtk4::Box,
    hint_box: gtk4::Box,
    empty_box: gtk4::Box,
    status_label: gtk4::Label,
    close_btn: gtk4::Button,
    cancel_handler: Rc<RefCell<Option<Rc<dyn Fn(Uuid)>>>>,
    dismiss_handler: Rc<RefCell<Option<Rc<dyn Fn(Uuid)>>>>,
    retry_handler: Rc<RefCell<Option<Rc<dyn Fn(Uuid)>>>>,
    /// "Process All" + "Process Next" buttons bar.
    process_bar: gtk4::Box,
    process_separator: gtk4::Separator,
    process_all_btn: gtk4::Button,
    process_next_btn: gtk4::Button,
    /// Token limit pause banner.
    paused_banner: gtk4::Box,
    paused_countdown_label: gtk4::Label,
    /// Banner shown when Claude CLI is not installed.
    claude_missing_banner: gtk4::Box,
    /// Sandbox controls.
    sandbox_switch: gtk4::Switch,
    sandbox_enforcement: gtk4::DropDown,
    sandbox_network_switch: gtk4::Switch,
    /// Guard flag to prevent re-entrant updates from signal handlers.
    sandbox_updating: Rc<RefCell<bool>>,
}

impl Default for AgentQueuePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentQueuePanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("agent-queue-panel");
        container.set_width_request(380);

        // Header.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("Agent Queue"));
        title.add_css_class("workspace-title");
        title.set_halign(gtk4::Align::Start);
        header.append(&title);

        let status_label = gtk4::Label::new(Some("0 queued"));
        status_label.add_css_class("panel-meta");
        status_label.set_hexpand(true);
        status_label.set_halign(gtk4::Align::End);
        status_label.set_margin_end(4);
        header.append(&status_label);

        // "+" button to toggle the submit row.
        let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        add_btn.add_css_class("flat");
        add_btn.set_tooltip_text(Some("Add task manually"));
        header.append(&add_btn);

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

        // Submit row — starts hidden.
        let submit_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        submit_row.set_margin_start(12);
        submit_row.set_margin_end(12);
        submit_row.set_margin_top(8);
        submit_row.set_margin_bottom(8);
        submit_row.set_visible(false);

        let submit_entry = gtk4::Entry::new();
        submit_entry.set_placeholder_text(Some("Enter task content..."));
        submit_entry.set_hexpand(true);
        submit_row.append(&submit_entry);

        let submit_btn = gtk4::Button::with_label("Submit");
        submit_btn.add_css_class("suggested-action");
        submit_row.append(&submit_btn);

        container.append(&submit_row);

        // Wire "+" button to toggle submit row visibility.
        {
            let row = submit_row.clone();
            let entry = submit_entry.clone();
            add_btn.connect_clicked(move |_| {
                let visible = row.get_visible();
                row.set_visible(!visible);
                if !visible {
                    entry.grab_focus();
                }
            });
        }

        // ── Sandbox section ──
        let sandbox_section = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        sandbox_section.set_margin_start(12);
        sandbox_section.set_margin_end(12);
        sandbox_section.set_margin_top(6);
        sandbox_section.set_margin_bottom(4);

        let sandbox_header = gtk4::Label::new(Some("Sandbox"));
        sandbox_header.add_css_class("settings-section-title");
        sandbox_header.set_halign(gtk4::Align::Start);
        sandbox_section.append(&sandbox_header);

        // Enable row
        let enable_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let enable_label = gtk4::Label::new(Some("Enable"));
        enable_label.set_hexpand(true);
        enable_label.set_halign(gtk4::Align::Start);
        enable_row.append(&enable_label);
        let sandbox_switch = gtk4::Switch::new();
        enable_row.append(&sandbox_switch);
        sandbox_section.append(&enable_row);

        // Enforcement row
        let enforce_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let enforce_label = gtk4::Label::new(Some("Enforcement"));
        enforce_label.set_hexpand(true);
        enforce_label.set_halign(gtk4::Align::Start);
        enforce_row.append(&enforce_label);
        let levels = gtk4::StringList::new(&["Permissive", "Enforcing", "Strict"]);
        let sandbox_enforcement = gtk4::DropDown::new(Some(levels), gtk4::Expression::NONE);
        sandbox_enforcement.set_selected(1); // Enforcing default
        sandbox_enforcement.set_sensitive(false);
        enforce_row.append(&sandbox_enforcement);
        sandbox_section.append(&enforce_row);

        // Network row
        let net_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let net_label = gtk4::Label::new(Some("Network"));
        net_label.set_hexpand(true);
        net_label.set_halign(gtk4::Align::Start);
        net_row.append(&net_label);
        let sandbox_network_switch = gtk4::Switch::new();
        sandbox_network_switch.set_active(true);
        sandbox_network_switch.set_sensitive(false);
        net_row.append(&sandbox_network_switch);
        sandbox_section.append(&net_row);

        container.append(&sandbox_section);
        let sandbox_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        container.append(&sandbox_sep);

        let sandbox_updating = Rc::new(RefCell::new(false));

        // Hint banner — visible when queue is empty.
        let hint_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        hint_box.add_css_class("queue-hint");

        let hint_text = gtk4::Label::new(Some("Start with /plan to flesh out your approach, then tell Claude to add it to the thane queue"));
        hint_text.add_css_class("queue-hint-text");
        hint_text.set_halign(gtk4::Align::Start);
        hint_text.set_wrap(true);
        hint_box.append(&hint_text);

        let hint_example = gtk4::Label::new(Some("e.g. \"add this plan to my thane queue\""));
        hint_example.add_css_class("queue-hint-example");
        hint_example.set_halign(gtk4::Align::Start);
        hint_box.append(&hint_example);

        container.append(&hint_box);

        let sep2 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        container.append(&sep2);

        // Token limit pause banner — hidden by default.
        let paused_banner = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        paused_banner.add_css_class("queue-paused-banner");
        paused_banner.set_visible(false);

        let paused_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let paused_icon = gtk4::Image::from_icon_name("dialog-warning-symbolic");
        paused_icon.set_pixel_size(16);
        paused_header.append(&paused_icon);
        let paused_text = gtk4::Label::new(Some("Queue paused \u{2014} token limit reached"));
        paused_text.add_css_class("queue-paused-text");
        paused_text.set_halign(gtk4::Align::Start);
        paused_header.append(&paused_text);
        paused_banner.append(&paused_header);

        let paused_countdown_label = gtk4::Label::new(None);
        paused_countdown_label.add_css_class("queue-paused-countdown");
        paused_countdown_label.set_halign(gtk4::Align::Start);
        paused_countdown_label.set_margin_start(22);
        paused_banner.append(&paused_countdown_label);

        container.append(&paused_banner);

        // Scrollable list of entries.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&list_box));

        container.append(&scrolled);

        // Process buttons bar — hidden by default.
        let process_separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        process_separator.set_visible(false);
        container.append(&process_separator);

        let process_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        process_bar.add_css_class("queue-process-bar");
        process_bar.set_margin_start(12);
        process_bar.set_margin_end(12);
        process_bar.set_margin_top(8);
        process_bar.set_margin_bottom(8);
        process_bar.set_visible(false);

        let process_all_btn = gtk4::Button::with_label("Process All");
        process_all_btn.add_css_class("suggested-action");
        process_all_btn.set_hexpand(true);
        process_bar.append(&process_all_btn);

        let process_next_btn = gtk4::Button::with_label("Process Next");
        process_next_btn.add_css_class("flat");
        process_next_btn.set_hexpand(true);
        process_bar.append(&process_next_btn);

        container.append(&process_bar);

        // Empty state box (shown inside the list area when queue is empty).
        let empty_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        empty_box.set_halign(gtk4::Align::Center);
        empty_box.set_valign(gtk4::Align::Start);
        empty_box.set_margin_top(40);

        let empty_icon = gtk4::Image::from_icon_name("view-list-symbolic");
        empty_icon.set_pixel_size(32);
        empty_icon.set_opacity(0.3);
        empty_box.append(&empty_icon);

        let empty_title = gtk4::Label::new(Some("No tasks in queue"));
        empty_title.add_css_class("panel-meta");
        empty_box.append(&empty_title);

        let empty_hint = gtk4::Label::new(Some("Start with /plan to flesh out your approach, then tell Claude to \"add this plan to my thane queue\""));
        empty_hint.add_css_class("queue-empty-hint");
        empty_hint.set_wrap(true);
        empty_box.append(&empty_hint);

        Self {
            container,
            list_box,
            submit_entry,
            submit_btn,
            submit_row,
            hint_box,
            empty_box,
            status_label,
            close_btn,
            cancel_handler: Rc::new(RefCell::new(None)),
            dismiss_handler: Rc::new(RefCell::new(None)),
            retry_handler: Rc::new(RefCell::new(None)),
            process_bar,
            process_separator,
            process_all_btn,
            process_next_btn,
            paused_banner,
            paused_countdown_label,
            claude_missing_banner,
            sandbox_switch,
            sandbox_enforcement,
            sandbox_network_switch,
            sandbox_updating,
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

    /// Connect submit (fires on button click or Entry activate).
    /// After a successful submit the input row auto-collapses.
    pub fn connect_submit<F: Fn(String) + Clone + 'static>(&self, f: F) {
        let entry = self.submit_entry.clone();
        let row = self.submit_row.clone();
        let f2 = f.clone();
        self.submit_btn.connect_clicked(move |_| {
            let text = entry.text().to_string();
            if !text.is_empty() {
                f(text);
                entry.set_text("");
                row.set_visible(false);
            }
        });

        let entry2 = self.submit_entry.clone();
        let row2 = self.submit_row.clone();
        self.submit_entry.connect_activate(move |_| {
            let text = entry2.text().to_string();
            if !text.is_empty() {
                f2(text);
                entry2.set_text("");
                row2.set_visible(false);
            }
        });
    }

    /// Connect cancel button on per-row items.
    pub fn connect_cancel<F: Fn(Uuid) + 'static>(&self, f: F) {
        *self.cancel_handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Connect dismiss button on completed/failed/cancelled entries.
    pub fn connect_dismiss<F: Fn(Uuid) + 'static>(&self, f: F) {
        *self.dismiss_handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Connect retry button on failed/cancelled entries.
    pub fn connect_retry<F: Fn(Uuid) + 'static>(&self, f: F) {
        *self.retry_handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Connect "Process All" button.
    pub fn connect_process_all<F: Fn() + 'static>(&self, f: F) {
        self.process_all_btn.connect_clicked(move |_| f());
    }

    /// Connect "Process Next" button.
    pub fn connect_process_next<F: Fn() + 'static>(&self, f: F) {
        self.process_next_btn.connect_clicked(move |_| f());
    }

    /// Connect sandbox control changes.
    /// Callback receives (enabled, enforcement_index, allow_network).
    pub fn connect_sandbox_changed<F: Fn(bool, u32, bool) + Clone + 'static>(&self, f: F) {
        let updating = self.sandbox_updating.clone();
        let enforcement = self.sandbox_enforcement.clone();
        let network = self.sandbox_network_switch.clone();

        // Enable switch
        {
            let f = f.clone();
            let updating = updating.clone();
            let enforcement = enforcement.clone();
            let network = network.clone();
            self.sandbox_switch.connect_state_set(move |switch, active| {
                if *updating.borrow() {
                    return glib::Propagation::Proceed;
                }
                enforcement.set_sensitive(active);
                network.set_sensitive(active);
                f(active, enforcement.selected(), network.is_active());
                glib::Propagation::Proceed
            });
        }

        // Enforcement dropdown
        {
            let f = f.clone();
            let updating = updating.clone();
            let switch_ref = self.sandbox_switch.clone();
            let network = network.clone();
            self.sandbox_enforcement.connect_selected_notify(move |dropdown| {
                if *updating.borrow() {
                    return;
                }
                f(switch_ref.is_active(), dropdown.selected(), network.is_active());
            });
        }

        // Network switch
        {
            let f = f;
            let updating = updating;
            let switch_ref = self.sandbox_switch.clone();
            let enforcement = enforcement;
            self.sandbox_network_switch.connect_state_set(move |_, active| {
                if *updating.borrow() {
                    return glib::Propagation::Proceed;
                }
                f(switch_ref.is_active(), enforcement.selected(), active);
                glib::Propagation::Proceed
            });
        }
    }

    /// Set the token-paused banner visibility and countdown.
    pub fn set_token_paused(&self, paused: bool, resets_at: Option<DateTime<Utc>>) {
        self.paused_banner.set_visible(paused);
        if paused {
            if let Some(reset_time) = resets_at {
                let now = Utc::now();
                if reset_time > now {
                    let diff = reset_time - now;
                    let mins = diff.num_minutes();
                    let secs = diff.num_seconds() % 60;
                    if mins > 0 {
                        self.paused_countdown_label
                            .set_text(&format!("Resumes in {}m {}s", mins, secs));
                    } else {
                        self.paused_countdown_label
                            .set_text(&format!("Resumes in {}s", secs));
                    }
                } else {
                    self.paused_countdown_label.set_text("Resuming...");
                }
            } else {
                self.paused_countdown_label.set_text("");
            }
        }
    }

    /// Update the panel contents from the agent queue data.
    pub fn update(&self, queue: &AgentQueue) {
        // Update sandbox controls.
        {
            let mut guard = self.sandbox_updating.borrow_mut();
            *guard = true;
            let policy = queue.sandbox_policy();
            self.sandbox_switch.set_active(policy.enabled);
            self.sandbox_enforcement.set_selected(match policy.enforcement {
                EnforcementLevel::Permissive => 0,
                EnforcementLevel::Enforcing => 1,
                EnforcementLevel::Strict => 2,
            });
            self.sandbox_network_switch.set_active(policy.allow_network);
            self.sandbox_enforcement.set_sensitive(policy.enabled);
            self.sandbox_network_switch.set_sensitive(policy.enabled);
            *guard = false;
        }

        // Update status label.
        let queued = queue.queued_count();
        let running = queue.running_count();
        let status_text = if queue.token_limit_paused {
            format!("{queued} queued (paused)")
        } else if running > 0 {
            format!("{queued} queued, {running} running")
        } else if queued > 0 {
            format!("{queued} queued")
        } else {
            "No tasks".to_string()
        };
        self.status_label.set_text(&status_text);

        // Token pause banner.
        self.set_token_paused(queue.token_limit_paused, queue.token_limit_resets_at);

        // Process buttons: visible when tasks are queued, nothing running, and not paused.
        let show_process = queued > 0 && running == 0 && !queue.token_limit_paused;
        self.process_bar.set_visible(show_process);
        self.process_separator.set_visible(show_process);

        // Rebuild list.
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        let entries = queue.list();
        let is_empty = entries.is_empty();

        // Show/hide hint banner and empty state based on queue contents.
        self.hint_box.set_visible(is_empty);

        if is_empty {
            self.list_box.append(&self.empty_box);
            return;
        }

        let cancel_handler = self.cancel_handler.clone();
        let dismiss_handler = self.dismiss_handler.clone();
        let retry_handler = self.retry_handler.clone();
        for entry in entries {
            let row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            row.add_css_class("queue-item");

            // Top row: content (truncated) + status badge.
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

            // Bottom row: priority + timestamp + action buttons.
            let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

            // Build metadata string: priority, timestamp, cost + model (if available).
            let mut meta_parts = vec![
                format!("P{}", entry.priority),
                entry.created_at.format("%H:%M:%S").to_string(),
            ];

            if entry.tokens_used.estimated_cost_usd > 0.0 {
                let cost_str = format!("${:.2}", entry.tokens_used.estimated_cost_usd);
                // Append model name (shortened) if present.
                if let Some(ref model) = entry.tokens_used.model {
                    if !model.is_empty() {
                        let short = shorten_model_name(model);
                        meta_parts.push(format!("{cost_str} ({short})"));
                    } else {
                        meta_parts.push(cost_str);
                    }
                } else {
                    meta_parts.push(cost_str);
                }
            } else if let Some(ref model) = entry.tokens_used.model {
                if !model.is_empty() {
                    meta_parts.push(shorten_model_name(model));
                }
            }

            let meta = gtk4::Label::new(Some(&meta_parts.join(" · ")));
            meta.add_css_class("panel-meta");
            meta.set_hexpand(true);
            meta.set_halign(gtk4::Align::Start);
            bottom.append(&meta);

            let entry_id = entry.id;

            match entry.status {
                // Cancel button for Queued or Running entries.
                QueueEntryStatus::Queued | QueueEntryStatus::Running => {
                    let cancel_btn = gtk4::Button::with_label("Cancel");
                    cancel_btn.add_css_class("flat");
                    cancel_btn.add_css_class("destructive-action");
                    cancel_btn.set_focusable(false);
                    let handler_ref = cancel_handler.clone();
                    cancel_btn.connect_clicked(move |_| {
                        if let Some(handler) = handler_ref.borrow().as_ref() {
                            handler(entry_id);
                        }
                    });
                    bottom.append(&cancel_btn);
                }
                // Completed: dismiss only.
                QueueEntryStatus::Completed => {
                    let dismiss_btn = gtk4::Button::with_label("Dismiss");
                    dismiss_btn.add_css_class("flat");
                    dismiss_btn.set_focusable(false);
                    let handler_ref = dismiss_handler.clone();
                    dismiss_btn.connect_clicked(move |_| {
                        if let Some(handler) = handler_ref.borrow().as_ref() {
                            handler(entry_id);
                        }
                    });
                    bottom.append(&dismiss_btn);
                }
                // Failed or Cancelled: retry + dismiss.
                QueueEntryStatus::Failed | QueueEntryStatus::Cancelled => {
                    let retry_btn = gtk4::Button::with_label("Retry");
                    retry_btn.add_css_class("flat");
                    retry_btn.add_css_class("suggested-action");
                    retry_btn.set_focusable(false);
                    let handler_ref = retry_handler.clone();
                    retry_btn.connect_clicked(move |_| {
                        if let Some(handler) = handler_ref.borrow().as_ref() {
                            handler(entry_id);
                        }
                    });
                    bottom.append(&retry_btn);

                    let dismiss_btn = gtk4::Button::with_label("Dismiss");
                    dismiss_btn.add_css_class("flat");
                    dismiss_btn.set_focusable(false);
                    let handler_ref2 = dismiss_handler.clone();
                    dismiss_btn.connect_clicked(move |_| {
                        if let Some(handler) = handler_ref2.borrow().as_ref() {
                            handler(entry_id);
                        }
                    });
                    bottom.append(&dismiss_btn);
                }
                // Paused: no buttons.
                _ => {}
            }

            row.append(&bottom);
            self.list_box.append(&row);
        }
    }
}

fn status_text_for(status: &QueueEntryStatus) -> &'static str {
    match status {
        QueueEntryStatus::Queued => "Queued",
        QueueEntryStatus::Running => "Running",
        QueueEntryStatus::PausedTokenLimit => "Paused",
        QueueEntryStatus::PausedByUser => "Paused",
        QueueEntryStatus::Completed => "Done",
        QueueEntryStatus::Failed => "Failed",
        QueueEntryStatus::Cancelled => "Cancelled",
    }
}

fn status_css_for(status: &QueueEntryStatus) -> &'static str {
    match status {
        QueueEntryStatus::Queued => "queue-status-queued",
        QueueEntryStatus::Running => "queue-status-running",
        QueueEntryStatus::PausedTokenLimit | QueueEntryStatus::PausedByUser => "queue-status-paused",
        QueueEntryStatus::Completed => "queue-status-completed",
        QueueEntryStatus::Failed => "queue-status-failed",
        QueueEntryStatus::Cancelled => "queue-status-cancelled",
    }
}
