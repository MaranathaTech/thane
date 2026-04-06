use thane_core::notification::{Notification, Urgency};
use gtk4::prelude::*;

/// A panel showing chronological notification list.
pub struct NotificationPanel {
    container: gtk4::Box,
    list_box: gtk4::ListBox,
    header: gtk4::Box,
    close_btn: gtk4::Button,
}

impl Default for NotificationPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("notification-panel");
        container.set_width_request(320);

        // Header with title and clear button.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(8);

        let title = gtk4::Label::new(Some("Notifications"));
        title.add_css_class("workspace-title");
        title.set_hexpand(true);
        title.set_halign(gtk4::Align::Start);
        header.append(&title);

        let mark_all_read_btn = gtk4::Button::with_label("Mark All Read");
        mark_all_read_btn.add_css_class("flat");
        header.append(&mark_all_read_btn);

        let clear_btn = gtk4::Button::with_label("Clear");
        clear_btn.add_css_class("flat");
        header.append(&clear_btn);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close"));
        header.append(&close_btn);

        container.append(&header);

        // Scrollable list of notifications.
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
            close_btn,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Update the displayed notifications.
    pub fn set_notifications(&self, notifications: &[Notification]) {
        // Clear existing rows.
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        if notifications.is_empty() {
            let empty_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            empty_box.set_margin_top(40);
            empty_box.set_margin_bottom(20);
            empty_box.set_halign(gtk4::Align::Center);

            let icon = gtk4::Image::from_icon_name("preferences-system-notifications-symbolic");
            icon.set_pixel_size(32);
            icon.set_opacity(0.3);
            empty_box.append(&icon);

            let title = gtk4::Label::new(Some("No notifications"));
            title.add_css_class("notification-title");
            empty_box.append(&title);

            let hint = gtk4::Label::new(Some("Workspace notifications will appear here"));
            hint.add_css_class("notification-body");
            empty_box.append(&hint);

            self.list_box.append(&empty_box);
            return;
        }

        // Add notifications in reverse chronological order.
        for notification in notifications.iter().rev() {
            let row = self.create_notification_row(notification);
            self.list_box.append(&row);
        }
    }

    fn create_notification_row(&self, notification: &Notification) -> gtk4::Box {
        let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        row.add_css_class("notification-item");

        if !notification.read {
            row.add_css_class("notification-item-unread");
        }

        // Title row.
        let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let title = gtk4::Label::new(Some(&notification.title));
        title.add_css_class("notification-title");
        title.set_halign(gtk4::Align::Start);
        title.set_hexpand(true);
        title_row.append(&title);

        let urgency_label = gtk4::Label::new(Some(urgency_text(notification.urgency)));
        urgency_label.add_css_class(urgency_css_class(notification.urgency));
        title_row.append(&urgency_label);

        let time = gtk4::Label::new(Some(
            &notification.timestamp.format("%H:%M").to_string(),
        ));
        time.add_css_class("notification-time");
        title_row.append(&time);

        row.append(&title_row);

        // Body.
        let body = gtk4::Label::new(Some(&notification.body));
        body.add_css_class("notification-body");
        body.set_halign(gtk4::Align::Start);
        body.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        body.set_max_width_chars(40);
        row.append(&body);

        row
    }

    /// Connect the "Mark All Read" button.
    pub fn connect_mark_all_read<F: Fn() + 'static>(&self, f: F) {
        // Find the mark-all-read button by label.
        let mut child = self.header.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(button) = widget.clone().downcast::<gtk4::Button>()
                && button.label().is_some_and(|l| l == "Mark All Read")
            {
                button.connect_clicked(move |_| f());
                break;
            }
            child = next;
        }
    }

    /// Connect the clear button.
    pub fn connect_clear<F: Fn() + 'static>(&self, f: F) {
        // Find the clear button by label.
        let mut child = self.header.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(button) = widget.clone().downcast::<gtk4::Button>()
                && button.label().is_some_and(|l| l == "Clear")
            {
                button.connect_clicked(move |_| f());
                break;
            }
            child = next;
        }
    }

    /// Connect the close button callback.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }
}

/// Return display text for an urgency level.
fn urgency_text(urgency: Urgency) -> &'static str {
    match urgency {
        Urgency::Low => "low",
        Urgency::Normal => "normal",
        Urgency::Critical => "critical",
    }
}

/// Return the CSS class name for an urgency level.
fn urgency_css_class(urgency: Urgency) -> &'static str {
    match urgency {
        Urgency::Low => "notification-urgency-low",
        Urgency::Normal => "notification-urgency-normal",
        Urgency::Critical => "notification-urgency-critical",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urgency_badge_text() {
        assert_eq!(urgency_text(Urgency::Low), "low");
        assert_eq!(urgency_text(Urgency::Normal), "normal");
        assert_eq!(urgency_text(Urgency::Critical), "critical");
    }

    #[test]
    fn test_urgency_css_class() {
        assert_eq!(urgency_css_class(Urgency::Low), "notification-urgency-low");
        assert_eq!(
            urgency_css_class(Urgency::Normal),
            "notification-urgency-normal"
        );
        assert_eq!(
            urgency_css_class(Urgency::Critical),
            "notification-urgency-critical"
        );
    }
}
