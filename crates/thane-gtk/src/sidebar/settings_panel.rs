use std::cell::Cell;
use std::rc::Rc;

use gdk4::prelude::*;
use gtk4::prelude::*;

/// A right-side panel for adjusting font/theme/terminal/security settings.
pub struct SettingsPanel {
    container: gtk4::Box,
    font_size_scale: gtk4::Scale,
    font_size_label: gtk4::Label,
    font_family_dropdown: gtk4::DropDown,
    ui_size_scale: gtk4::Scale,
    ui_size_label: gtk4::Label,
    scrollback_scale: gtk4::Scale,
    scrollback_label: gtk4::Label,
    cursor_style_dropdown: gtk4::DropDown,
    cursor_blink_switch: gtk4::Switch,
    confirm_close_switch: gtk4::Switch,
    font_color_button: gtk4::ColorDialogButton,
    link_url_in_app_switch: gtk4::Switch,
    link_url_in_browser_switch: gtk4::Switch,
    sensitive_policy_dropdown: gtk4::DropDown,
    queue_mode_dropdown: gtk4::DropDown,
    queue_sandbox_dropdown: gtk4::DropDown,
    queue_schedule_entry: gtk4::Entry,
    queue_schedule_row: gtk4::Box,
    queue_mode_desc: gtk4::Label,
    enterprise_cost_entry: gtk4::Entry,
    enterprise_cost_row: gtk4::Box,
    cost_scope_dropdown: gtk4::DropDown,
    close_btn: gtk4::Button,
    /// Guard: suppress callbacks during programmatic updates.
    updating: Rc<Cell<bool>>,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("settings-panel");
        container.set_width_request(360);

        // Header with close button.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("Settings"));
        title.add_css_class("workspace-title");
        title.set_hexpand(true);
        title.set_halign(gtk4::Align::Start);
        header.append(&title);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close"));
        header.append(&close_btn);

        container.append(&header);

        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        container.append(&sep);

        // Wrap content in a scrolled window since the panel is now tall.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);

        // ── Font section ──
        let family_label = gtk4::Label::new(Some("Font Family"));
        family_label.set_halign(gtk4::Align::Start);
        content.append(&family_label);

        let mono_families = get_monospace_families();
        let string_list = gtk4::StringList::new(
            &mono_families
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
        );
        let font_family_dropdown = gtk4::DropDown::new(Some(string_list), gtk4::Expression::NONE);
        font_family_dropdown.set_enable_search(true);
        content.append(&font_family_dropdown);

        // Terminal font size slider.
        let size_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let size_label = gtk4::Label::new(Some("Terminal Font Size"));
        size_label.set_hexpand(true);
        size_label.set_halign(gtk4::Align::Start);
        size_row.append(&size_label);

        let font_size_label = gtk4::Label::new(Some("13"));
        font_size_label.set_width_chars(3);
        size_row.append(&font_size_label);

        content.append(&size_row);

        let font_size_scale =
            gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 6.0, 72.0, 1.0);
        font_size_scale.set_value(13.0);
        font_size_scale.set_draw_value(false);
        content.append(&font_size_scale);

        // Terminal font color picker.
        let color_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let color_lbl = gtk4::Label::new(Some("Terminal Font Color"));
        color_lbl.set_hexpand(true);
        color_lbl.set_halign(gtk4::Align::Start);
        color_row.append(&color_lbl);

        let color_dialog = gtk4::ColorDialog::new();
        color_dialog.set_with_alpha(false);
        let font_color_button = gtk4::ColorDialogButton::new(Some(color_dialog));
        let default_rgba = gdk4::RGBA::parse("#e4e4e7").unwrap_or(gdk4::RGBA::WHITE);
        font_color_button.set_rgba(&default_rgba);
        color_row.append(&font_color_button);

        content.append(&color_row);

        // UI text size slider.
        let ui_size_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let ui_size_lbl = gtk4::Label::new(Some("UI Text Size"));
        ui_size_lbl.set_hexpand(true);
        ui_size_lbl.set_halign(gtk4::Align::Start);
        ui_size_row.append(&ui_size_lbl);

        let ui_size_label = gtk4::Label::new(Some("14"));
        ui_size_label.set_width_chars(3);
        ui_size_row.append(&ui_size_label);

        content.append(&ui_size_row);

        let ui_size_scale =
            gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 8.0, 24.0, 1.0);
        ui_size_scale.set_value(14.0);
        ui_size_scale.set_draw_value(false);
        content.append(&ui_size_scale);

        // ── Terminal section ──
        let term_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        term_sep.set_margin_top(8);
        content.append(&term_sep);

        let term_title = gtk4::Label::new(Some("Terminal"));
        term_title.add_css_class("settings-section-title");
        term_title.set_halign(gtk4::Align::Start);
        content.append(&term_title);

        // Scrollback limit scale.
        let scrollback_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let scrollback_lbl = gtk4::Label::new(Some("Scrollback Limit"));
        scrollback_lbl.set_hexpand(true);
        scrollback_lbl.set_halign(gtk4::Align::Start);
        scrollback_row.append(&scrollback_lbl);

        let scrollback_label = gtk4::Label::new(Some("10000"));
        scrollback_label.set_width_chars(6);
        scrollback_row.append(&scrollback_label);

        content.append(&scrollback_row);

        let scrollback_scale =
            gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 1000.0, 100000.0, 1000.0);
        scrollback_scale.set_value(10000.0);
        scrollback_scale.set_draw_value(false);
        content.append(&scrollback_scale);

        // Cursor style dropdown.
        let cursor_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let cursor_lbl = gtk4::Label::new(Some("Cursor Style"));
        cursor_lbl.set_hexpand(true);
        cursor_lbl.set_halign(gtk4::Align::Start);
        cursor_row.append(&cursor_lbl);

        let cursor_styles =
            gtk4::StringList::new(&["Block", "Bar", "Underline"]);
        let cursor_style_dropdown =
            gtk4::DropDown::new(Some(cursor_styles), gtk4::Expression::NONE);
        cursor_style_dropdown.set_selected(0); // Block default
        cursor_row.append(&cursor_style_dropdown);

        content.append(&cursor_row);

        // Cursor blink switch.
        let blink_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let blink_lbl = gtk4::Label::new(Some("Cursor Blink"));
        blink_lbl.set_hexpand(true);
        blink_lbl.set_halign(gtk4::Align::Start);
        blink_row.append(&blink_lbl);

        let cursor_blink_switch = gtk4::Switch::new();
        cursor_blink_switch.set_active(true);
        cursor_blink_switch.set_valign(gtk4::Align::Center);
        blink_row.append(&cursor_blink_switch);

        content.append(&blink_row);

        // Confirm close switch.
        let confirm_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let confirm_lbl = gtk4::Label::new(Some("Confirm Close"));
        confirm_lbl.set_hexpand(true);
        confirm_lbl.set_halign(gtk4::Align::Start);
        confirm_row.append(&confirm_lbl);

        let confirm_close_switch = gtk4::Switch::new();
        confirm_close_switch.set_active(true);
        confirm_close_switch.set_valign(gtk4::Align::Center);
        confirm_row.append(&confirm_close_switch);

        content.append(&confirm_row);

        // Open URLs in app switch.
        let link_app_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let link_app_lbl = gtk4::Label::new(Some("Open URLs in App"));
        link_app_lbl.set_hexpand(true);
        link_app_lbl.set_halign(gtk4::Align::Start);
        link_app_row.append(&link_app_lbl);

        let link_url_in_app_switch = gtk4::Switch::new();
        link_url_in_app_switch.set_active(true);
        link_url_in_app_switch.set_valign(gtk4::Align::Center);
        link_url_in_app_switch.set_tooltip_text(Some("Click a URL to open in embedded browser"));
        link_app_row.append(&link_url_in_app_switch);

        content.append(&link_app_row);

        let link_app_hint = gtk4::Label::new(Some("(click a URL)"));
        link_app_hint.add_css_class("dim-label");
        link_app_hint.set_halign(gtk4::Align::Start);
        link_app_hint.set_margin_start(4);
        content.append(&link_app_hint);

        // Open URLs in system browser switch.
        let link_browser_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let link_browser_lbl = gtk4::Label::new(Some("Open URLs in Browser"));
        link_browser_lbl.set_hexpand(true);
        link_browser_lbl.set_halign(gtk4::Align::Start);
        link_browser_row.append(&link_browser_lbl);

        let link_url_in_browser_switch = gtk4::Switch::new();
        link_url_in_browser_switch.set_active(false);
        link_url_in_browser_switch.set_valign(gtk4::Align::Center);
        link_url_in_browser_switch.set_tooltip_text(Some("Shift+click a URL to open in default browser"));
        link_browser_row.append(&link_url_in_browser_switch);

        content.append(&link_browser_row);

        let link_browser_hint = gtk4::Label::new(Some("(shift+click a URL)"));
        link_browser_hint.add_css_class("dim-label");
        link_browser_hint.set_halign(gtk4::Align::Start);
        link_browser_hint.set_margin_start(4);
        content.append(&link_browser_hint);

        // ── Security section ──
        let sec_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sec_sep.set_margin_top(8);
        content.append(&sec_sep);

        let sec_title = gtk4::Label::new(Some("Security"));
        sec_title.add_css_class("settings-section-title");
        sec_title.set_halign(gtk4::Align::Start);
        content.append(&sec_title);

        // Sensitive data policy dropdown.
        let policy_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let policy_lbl = gtk4::Label::new(Some("Sensitive Data Policy"));
        policy_lbl.set_hexpand(true);
        policy_lbl.set_halign(gtk4::Align::Start);
        policy_row.append(&policy_lbl);

        let policies = gtk4::StringList::new(&["Allow", "Warn", "Block"]);
        let sensitive_policy_dropdown =
            gtk4::DropDown::new(Some(policies), gtk4::Expression::NONE);
        sensitive_policy_dropdown.set_selected(1); // Warn default
        policy_row.append(&sensitive_policy_dropdown);

        content.append(&policy_row);

        // ── Agent Queue section ──
        let queue_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        queue_sep.set_margin_top(8);
        content.append(&queue_sep);

        let queue_title = gtk4::Label::new(Some("Agent Queue"));
        queue_title.add_css_class("settings-section-title");
        queue_title.set_halign(gtk4::Align::Start);
        content.append(&queue_title);

        // Queue mode dropdown.
        let queue_mode_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let queue_mode_lbl = gtk4::Label::new(Some("Processing Mode"));
        queue_mode_lbl.set_hexpand(true);
        queue_mode_lbl.set_halign(gtk4::Align::Start);
        queue_mode_row.append(&queue_mode_lbl);

        let modes = gtk4::StringList::new(&["Automatic", "Manual", "Scheduled"]);
        let queue_mode_dropdown =
            gtk4::DropDown::new(Some(modes), gtk4::Expression::NONE);
        queue_mode_dropdown.set_selected(0); // Automatic default
        queue_mode_row.append(&queue_mode_dropdown);

        content.append(&queue_mode_row);

        // Mode description label.
        let queue_mode_desc = gtk4::Label::new(Some("Tasks run automatically when queued"));
        queue_mode_desc.add_css_class("dim-label");
        queue_mode_desc.set_halign(gtk4::Align::Start);
        queue_mode_desc.set_margin_start(4);
        content.append(&queue_mode_desc);

        // Schedule entry (visible only when Scheduled is selected).
        let queue_schedule_row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        queue_schedule_row.set_visible(false);

        let queue_schedule_entry = gtk4::Entry::new();
        queue_schedule_entry.set_placeholder_text(Some("Mon:09:00,Wed:14:00,Fri:18:00"));
        queue_schedule_row.append(&queue_schedule_entry);

        let schedule_hint = gtk4::Label::new(Some("Format: Day:HH:MM (press Enter to save)"));
        schedule_hint.add_css_class("dim-label");
        schedule_hint.set_halign(gtk4::Align::Start);
        queue_schedule_row.append(&schedule_hint);

        content.append(&queue_schedule_row);

        // Queue sandbox mode dropdown.
        let sandbox_mode_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let sandbox_mode_lbl = gtk4::Label::new(Some("Sandbox Mode"));
        sandbox_mode_lbl.set_hexpand(true);
        sandbox_mode_lbl.set_halign(gtk4::Align::Start);
        sandbox_mode_row.append(&sandbox_mode_lbl);

        let sandbox_modes = gtk4::StringList::new(&["Off", "Workspace", "Strict"]);
        let queue_sandbox_dropdown =
            gtk4::DropDown::new(Some(sandbox_modes), gtk4::Expression::NONE);
        queue_sandbox_dropdown.set_selected(0); // Off default
        sandbox_mode_row.append(&queue_sandbox_dropdown);

        content.append(&sandbox_mode_row);

        let sandbox_hint = gtk4::Label::new(Some("Sandbox isolation for queued tasks"));
        sandbox_hint.add_css_class("dim-label");
        sandbox_hint.set_halign(gtk4::Align::Start);
        sandbox_hint.set_margin_start(4);
        content.append(&sandbox_hint);

        // Wire mode dropdown to show/hide schedule entry and update description.
        {
            let sched_row = queue_schedule_row.clone();
            let desc = queue_mode_desc.clone();
            queue_mode_dropdown.connect_selected_notify(move |dd| {
                let idx = dd.selected();
                sched_row.set_visible(idx == 2);
                desc.set_text(match idx {
                    0 => "Tasks run automatically when queued",
                    1 => "Tasks only run when you click Process",
                    2 => "Tasks run during scheduled time windows",
                    _ => "",
                });
            });
        }

        // ── Enterprise Monthly Cost ──
        let enterprise_cost_row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        enterprise_cost_row.set_margin_start(12);
        enterprise_cost_row.set_margin_end(12);
        enterprise_cost_row.set_margin_top(8);

        let ent_cost_label = gtk4::Label::new(Some("Enterprise Monthly Cost ($)"));
        ent_cost_label.add_css_class("settings-label");
        ent_cost_label.set_halign(gtk4::Align::Start);
        enterprise_cost_row.append(&ent_cost_label);

        let enterprise_cost_entry = gtk4::Entry::new();
        enterprise_cost_entry.set_placeholder_text(Some("e.g. 500"));
        enterprise_cost_row.append(&enterprise_cost_entry);

        let ent_cost_hint = gtk4::Label::new(Some(
            "Your per-seat monthly cost (Enterprise only).\nUsed to calculate cost from utilization. Press Enter to save.",
        ));
        ent_cost_hint.add_css_class("dim-label");
        ent_cost_hint.set_halign(gtk4::Align::Start);
        ent_cost_hint.set_wrap(true);
        enterprise_cost_row.append(&ent_cost_hint);

        content.append(&enterprise_cost_row);

        // ── Cost Display Scope ──
        let cost_scope_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let cost_scope_lbl = gtk4::Label::new(Some("Cost Display Scope"));
        cost_scope_lbl.set_hexpand(true);
        cost_scope_lbl.set_halign(gtk4::Align::Start);
        cost_scope_row.append(&cost_scope_lbl);

        let cost_scope_items = gtk4::StringList::new(&["Session", "All Time"]);
        let cost_scope_dropdown =
            gtk4::DropDown::new(Some(cost_scope_items), gtk4::Expression::NONE);
        cost_scope_dropdown.set_selected(1); // All Time default
        cost_scope_row.append(&cost_scope_dropdown);

        content.append(&cost_scope_row);

        let cost_scope_hint = gtk4::Label::new(Some(
            "Controls whether the status bar and token panel show session or all-time costs",
        ));
        cost_scope_hint.add_css_class("dim-label");
        cost_scope_hint.set_halign(gtk4::Align::Start);
        cost_scope_hint.set_margin_start(4);
        cost_scope_hint.set_wrap(true);
        content.append(&cost_scope_hint);

        // Hint.
        let hint = gtk4::Label::new(Some(
            "Changes apply immediately and are saved\nautomatically to ~/.config/thane/config.",
        ));
        hint.add_css_class("settings-hint");
        hint.set_halign(gtk4::Align::Start);
        hint.set_margin_top(16);
        hint.set_wrap(true);
        content.append(&hint);

        scrolled.set_child(Some(&content));
        container.append(&scrolled);

        Self {
            container,
            font_size_scale,
            font_size_label,
            font_family_dropdown,
            font_color_button,
            ui_size_scale,
            ui_size_label,
            scrollback_scale,
            scrollback_label,
            cursor_style_dropdown,
            cursor_blink_switch,
            confirm_close_switch,
            link_url_in_app_switch,
            link_url_in_browser_switch,
            sensitive_policy_dropdown,
            queue_mode_dropdown,
            queue_sandbox_dropdown,
            queue_schedule_entry,
            queue_schedule_row,
            queue_mode_desc,
            enterprise_cost_entry,
            enterprise_cost_row,
            cost_scope_dropdown,
            close_btn,
            updating: Rc::new(Cell::new(false)),
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    // ── Setters ──
    // All setters use the updating guard to suppress callbacks during
    // programmatic updates, preventing re-entrant RefCell borrows.

    /// Set the displayed font size value (for syncing with external zoom changes).
    pub fn set_font_size(&self, size: f64) {
        self.updating.set(true);
        self.font_size_scale.set_value(size);
        self.font_size_label.set_text(&format!("{}", size as u32));
        self.updating.set(false);
    }

    /// Set the displayed font family value.
    pub fn set_font_family(&self, family: &str) {
        self.updating.set(true);
        if let Some(model) = self.font_family_dropdown.model() {
            let n = model.n_items();
            for i in 0..n {
                if let Some(item) = model.item(i)
                    && let Ok(string_obj) = item.downcast::<gtk4::StringObject>()
                    && string_obj.string() == family
                {
                    self.font_family_dropdown.set_selected(i);
                    break;
                }
            }
        }
        self.updating.set(false);
    }

    /// Set the terminal font color from a hex string (e.g. "#e4e4e7").
    pub fn set_font_color(&self, hex: &str) {
        self.updating.set(true);
        if let Ok(rgba) = gdk4::RGBA::parse(hex) {
            self.font_color_button.set_rgba(&rgba);
        }
        self.updating.set(false);
    }

    /// Set the displayed UI text size value.
    pub fn set_ui_font_size(&self, size: f64) {
        self.updating.set(true);
        self.ui_size_scale.set_value(size);
        self.ui_size_label.set_text(&format!("{}", size as u32));
        self.updating.set(false);
    }

    /// Set the scrollback limit value.
    pub fn set_scrollback_limit(&self, limit: i64) {
        self.updating.set(true);
        self.scrollback_scale.set_value(limit as f64);
        self.scrollback_label.set_text(&format!("{limit}"));
        self.updating.set(false);
    }

    /// Set cursor style. "block" → 0, "bar"/"ibeam" → 1, "underline" → 2.
    pub fn set_cursor_style(&self, style: &str) {
        self.updating.set(true);
        let idx = match style.to_lowercase().as_str() {
            "block" => 0,
            "bar" | "ibeam" => 1,
            "underline" => 2,
            _ => 0,
        };
        self.cursor_style_dropdown.set_selected(idx);
        self.updating.set(false);
    }

    /// Set cursor blink enabled.
    pub fn set_cursor_blink(&self, blink: bool) {
        self.updating.set(true);
        self.cursor_blink_switch.set_active(blink);
        self.updating.set(false);
    }

    /// Set confirm close enabled.
    pub fn set_confirm_close(&self, confirm: bool) {
        self.updating.set(true);
        self.confirm_close_switch.set_active(confirm);
        self.updating.set(false);
    }

    /// Set "open URLs in app" enabled.
    pub fn set_link_url_in_app(&self, enabled: bool) {
        self.updating.set(true);
        self.link_url_in_app_switch.set_active(enabled);
        self.updating.set(false);
    }

    /// Set "open URLs in browser" enabled.
    pub fn set_link_url_in_browser(&self, enabled: bool) {
        self.updating.set(true);
        self.link_url_in_browser_switch.set_active(enabled);
        self.updating.set(false);
    }

    /// Set sensitive data policy. "allow" → 0, "warn" → 1, "block" → 2.
    pub fn set_sensitive_policy(&self, policy: &str) {
        self.updating.set(true);
        let idx = match policy.to_lowercase().as_str() {
            "allow" => 0,
            "warn" => 1,
            "block" => 2,
            _ => 1,
        };
        self.sensitive_policy_dropdown.set_selected(idx);
        self.updating.set(false);
    }

    // ── Connect callbacks ──
    // All callbacks check the updating guard to prevent re-entrant borrows
    // when setters are called programmatically.

    /// Connect callback for font size changes from the slider.
    pub fn connect_font_size_changed<F: Fn(f64) + 'static>(&self, f: F) {
        let label = self.font_size_label.clone();
        let guard = self.updating.clone();
        self.font_size_scale.connect_value_changed(move |scale| {
            if guard.get() { return; }
            let size = scale.value();
            label.set_text(&format!("{}", size as u32));
            f(size);
        });
    }

    /// Connect callback for font family changes from the dropdown.
    pub fn connect_font_family_changed<F: Fn(String) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.font_family_dropdown
            .connect_selected_notify(move |dd| {
                if guard.get() { return; }
                if let Some(item) = dd.selected_item()
                    && let Ok(string_obj) = item.downcast::<gtk4::StringObject>()
                {
                    f(string_obj.string().to_string());
                }
            });
    }

    /// Connect callback for font color changes. Provides the hex color string.
    pub fn connect_font_color_changed<F: Fn(String) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.font_color_button
            .connect_rgba_notify(move |btn| {
                if guard.get() { return; }
                let rgba = btn.rgba();
                let hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    (rgba.red() * 255.0) as u8,
                    (rgba.green() * 255.0) as u8,
                    (rgba.blue() * 255.0) as u8,
                );
                f(hex);
            });
    }

    /// Connect callback for UI text size changes.
    pub fn connect_ui_size_changed<F: Fn(f64) + 'static>(&self, f: F) {
        let label = self.ui_size_label.clone();
        let guard = self.updating.clone();
        self.ui_size_scale.connect_value_changed(move |scale| {
            if guard.get() { return; }
            let size = scale.value();
            label.set_text(&format!("{}", size as u32));
            f(size);
        });
    }

    /// Connect callback for scrollback limit changes.
    pub fn connect_scrollback_changed<F: Fn(i64) + 'static>(&self, f: F) {
        let label = self.scrollback_label.clone();
        let guard = self.updating.clone();
        self.scrollback_scale.connect_value_changed(move |scale| {
            if guard.get() { return; }
            let value = scale.value() as i64;
            label.set_text(&format!("{value}"));
            f(value);
        });
    }

    /// Connect callback for cursor style changes.
    pub fn connect_cursor_style_changed<F: Fn(u32) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.cursor_style_dropdown
            .connect_selected_notify(move |dd| {
                if guard.get() { return; }
                f(dd.selected());
            });
    }

    /// Connect callback for cursor blink changes.
    pub fn connect_cursor_blink_changed<F: Fn(bool) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.cursor_blink_switch
            .connect_active_notify(move |switch| {
                if guard.get() { return; }
                f(switch.is_active());
            });
    }

    /// Connect callback for confirm close changes.
    pub fn connect_confirm_close_changed<F: Fn(bool) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.confirm_close_switch
            .connect_active_notify(move |switch| {
                if guard.get() { return; }
                f(switch.is_active());
            });
    }

    /// Connect callback for "open URLs in app" changes.
    pub fn connect_link_url_in_app_changed<F: Fn(bool) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.link_url_in_app_switch
            .connect_active_notify(move |switch| {
                if guard.get() { return; }
                f(switch.is_active());
            });
    }

    /// Connect callback for "open URLs in browser" changes.
    pub fn connect_link_url_in_browser_changed<F: Fn(bool) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.link_url_in_browser_switch
            .connect_active_notify(move |switch| {
                if guard.get() { return; }
                f(switch.is_active());
            });
    }

    /// Connect callback for sensitive policy changes (index: 0=Allow, 1=Warn, 2=Block).
    pub fn connect_sensitive_policy_changed<F: Fn(u32) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.sensitive_policy_dropdown
            .connect_selected_notify(move |dd| {
                if guard.get() { return; }
                f(dd.selected());
            });
    }

    /// Set queue mode. "automatic" → 0, "manual" → 1, "scheduled" → 2.
    pub fn set_queue_mode(&self, mode: &str) {
        self.updating.set(true);
        let idx = match mode.to_lowercase().as_str() {
            "automatic" => 0,
            "manual" => 1,
            "scheduled" => 2,
            _ => 0,
        };
        self.queue_mode_dropdown.set_selected(idx);
        self.queue_schedule_row.set_visible(idx == 2);
        self.queue_mode_desc.set_text(match idx {
            0 => "Tasks run automatically when queued",
            1 => "Tasks only run when you click Process",
            2 => "Tasks run during scheduled time windows",
            _ => "",
        });
        self.updating.set(false);
    }

    /// Set the queue schedule entry text.
    pub fn set_queue_schedule(&self, schedule: &str) {
        self.updating.set(true);
        self.queue_schedule_entry.set_text(schedule);
        self.updating.set(false);
    }

    /// Set queue sandbox mode. "off" -> 0, "workspace" -> 1, "strict" -> 2.
    pub fn set_queue_sandbox_mode(&self, mode: &str) {
        self.updating.set(true);
        let idx = match mode.to_lowercase().as_str() {
            "off" => 0,
            "workspace" => 1,
            "strict" => 2,
            _ => 0,
        };
        self.queue_sandbox_dropdown.set_selected(idx);
        self.updating.set(false);
    }

    /// Connect callback for queue mode changes (index: 0=Automatic, 1=Manual, 2=Scheduled).
    pub fn connect_queue_mode_changed<F: Fn(u32) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.queue_mode_dropdown
            .connect_selected_notify(move |dd| {
                if guard.get() { return; }
                f(dd.selected());
            });
    }

    /// Connect callback for queue schedule changes (fires on Enter key).
    pub fn connect_queue_schedule_changed<F: Fn(String) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.queue_schedule_entry.connect_activate(move |entry| {
            if guard.get() { return; }
            f(entry.text().to_string());
        });
    }

    /// Connect callback for queue sandbox mode changes (0=Off, 1=Workspace, 2=Strict).
    pub fn connect_queue_sandbox_changed<F: Fn(u32) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.queue_sandbox_dropdown
            .connect_selected_notify(move |dd| {
                if guard.get() { return; }
                f(dd.selected());
            });
    }

    /// Set the enterprise monthly cost display value.
    pub fn set_enterprise_monthly_cost(&self, cost: Option<f64>) {
        self.updating.set(true);
        match cost {
            Some(c) => self.enterprise_cost_entry.set_text(&format!("{c:.2}")),
            None => self.enterprise_cost_entry.set_text(""),
        }
        self.updating.set(false);
    }

    /// Show or hide the enterprise cost row based on plan type.
    pub fn set_enterprise_cost_visible(&self, visible: bool) {
        self.enterprise_cost_row.set_visible(visible);
    }

    /// Set the cost display scope dropdown: "session" or "all-time".
    pub fn set_cost_display_scope(&self, scope: &str) {
        self.updating.set(true);
        let idx = if scope == "all-time" { 1 } else { 0 };
        self.cost_scope_dropdown.set_selected(idx);
        self.updating.set(false);
    }

    /// Connect callback for cost display scope changes (0=Session, 1=All Time).
    pub fn connect_cost_scope_changed<F: Fn(u32) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.cost_scope_dropdown
            .connect_selected_notify(move |dd| {
                if guard.get() { return; }
                f(dd.selected());
            });
    }

    /// Connect callback for enterprise monthly cost changes (fires on Enter).
    pub fn connect_enterprise_cost_changed<F: Fn(Option<f64>) + 'static>(&self, f: F) {
        let guard = self.updating.clone();
        self.enterprise_cost_entry.connect_activate(move |entry| {
            if guard.get() {
                return;
            }
            let text = entry.text().to_string();
            let value = if text.is_empty() {
                None
            } else {
                text.parse::<f64>().ok().filter(|v| *v > 0.0)
            };
            f(value);
        });
    }

    /// Connect the close button callback.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }
}

/// Query Pango for all monospace font families, sorted alphabetically.
fn get_monospace_families() -> Vec<String> {
    let tmp = gtk4::Label::new(None);
    let pango_ctx = tmp.pango_context();
    let font_map = pango_ctx.font_map().expect("No font map available");
    let families = font_map.list_families();
    let mut mono: Vec<String> = families
        .iter()
        .filter(|f: &&gtk4::pango::FontFamily| f.is_monospace())
        .map(|f: &gtk4::pango::FontFamily| f.name().to_string())
        .collect();
    mono.sort_unstable_by_key(|a| a.to_lowercase());
    if mono.is_empty() {
        mono.push("monospace".to_string());
    }
    mono
}
