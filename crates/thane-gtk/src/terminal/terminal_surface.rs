use thane_core::panel::PanelId;
use thane_core::sandbox::SandboxPolicy;
use thane_terminal::traits::{TerminalEngine, TerminalSurface};
use thane_terminal::vte_backend::{VteEngine, VteSurface};
use gdk4::{self, Key};
use gio;
use gtk4::prelude::*;
use vte4::prelude::*;

/// GTK wrapper around a VTE terminal surface, adding UI integration.
pub struct TerminalPanel {
    surface: VteSurface,
    container: gtk4::Box,
    search_bar: gtk4::Box,
    search_entry: gtk4::Entry,
    jump_to_bottom_btn: gtk4::Button,
}

impl TerminalPanel {
    /// Create a new terminal panel widget.
    pub fn new(engine: &VteEngine, panel_id: PanelId, cwd: &str, extra_env: &[(&str, &str)]) -> Self {
        let surface = engine.create_surface(panel_id, cwd, None, extra_env);
        Self::from_surface(surface)
    }

    /// Create a new sandboxed terminal panel widget.
    pub fn new_sandboxed(
        engine: &VteEngine,
        panel_id: PanelId,
        cwd: &str,
        extra_env: &[(&str, &str)],
        sandbox: &SandboxPolicy,
    ) -> Self {
        let surface = engine.create_sandboxed_surface(panel_id, cwd, None, extra_env, sandbox);
        Self::from_surface(surface)
    }

    /// Build the panel UI from a pre-created VTE surface.
    fn from_surface(surface: VteSurface) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("terminal-pane");

        // Search bar (hidden by default).
        let search_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        search_bar.add_css_class("terminal-search-bar");
        search_bar.set_margin_start(4);
        search_bar.set_margin_end(4);
        search_bar.set_margin_top(2);
        search_bar.set_margin_bottom(2);
        search_bar.set_visible(false);

        let search_entry = gtk4::Entry::new();
        search_entry.set_placeholder_text(Some("Find..."));
        search_entry.set_hexpand(true);
        search_bar.append(&search_entry);

        let prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
        prev_btn.set_tooltip_text(Some("Previous match"));
        prev_btn.add_css_class("flat");
        search_bar.append(&prev_btn);

        let next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
        next_btn.set_tooltip_text(Some("Next match"));
        next_btn.add_css_class("flat");
        search_bar.append(&next_btn);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.set_tooltip_text(Some("Close search"));
        close_btn.add_css_class("flat");
        search_bar.append(&close_btn);

        container.append(&search_bar);

        // Wire search entry: update pattern as user types.
        {
            let surface_ref = surface.vte_terminal().clone();
            search_entry.connect_changed(move |entry| {
                let text = entry.text().to_string();
                // Escape regex special chars for literal search.
                let escaped = glib::Regex::escape_string(&text);
                if let Ok(regex) = vte4::Regex::for_search(&escaped, 0) {
                    surface_ref.search_set_regex(Some(&regex), 0);
                    surface_ref.search_set_wrap_around(true);
                    surface_ref.search_find_next();
                }
            });
        }

        // Wire search entry: Enter finds next.
        {
            let surface_ref = surface.vte_terminal().clone();
            search_entry.connect_activate(move |_| {
                surface_ref.search_find_next();
            });
        }

        // Wire next/prev/close buttons.
        {
            let surface_ref = surface.vte_terminal().clone();
            next_btn.connect_clicked(move |_| {
                surface_ref.search_find_next();
            });
        }
        {
            let surface_ref = surface.vte_terminal().clone();
            prev_btn.connect_clicked(move |_| {
                surface_ref.search_find_previous();
            });
        }
        {
            let search_bar_ref = search_bar.clone();
            let surface_ref = surface.vte_terminal().clone();
            close_btn.connect_clicked(move |_| {
                search_bar_ref.set_visible(false);
                surface_ref.search_set_regex(None::<&vte4::Regex>, 0);
            });
        }

        // Escape key in search entry closes search.
        {
            let search_bar_ref = search_bar.clone();
            let surface_ref2 = surface.vte_terminal().clone();
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, _modifiers| {
                if keyval == Key::Escape {
                    search_bar_ref.set_visible(false);
                    surface_ref2.search_set_regex(None::<&vte4::Regex>, 0);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            search_entry.add_controller(key_controller);
        }

        // The VTE terminal widget in an overlay for the "jump to bottom" button.
        let overlay = gtk4::Overlay::new();
        let widget = surface.widget();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_halign(gtk4::Align::Fill);
        overlay.set_child(Some(widget));
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);

        // "Jump to bottom" button — shown when user scrolls up during active output.
        let jump_to_bottom_btn = gtk4::Button::with_label("\u{2193} Jump to bottom");
        jump_to_bottom_btn.add_css_class("jump-to-bottom");
        jump_to_bottom_btn.set_halign(gtk4::Align::Center);
        jump_to_bottom_btn.set_valign(gtk4::Align::End);
        jump_to_bottom_btn.set_margin_bottom(8);
        jump_to_bottom_btn.set_visible(false);
        overlay.add_overlay(&jump_to_bottom_btn);

        // Clicking the button scrolls to the bottom of the terminal.
        {
            let vte = surface.vte_terminal().clone();
            let btn = jump_to_bottom_btn.clone();
            jump_to_bottom_btn.connect_clicked(move |_| {
                // VTE scroll to bottom: set scroll position to the maximum.
                if let Some(adj) = vte.vadjustment() {
                    adj.set_value(adj.upper() - adj.page_size());
                }
                btn.set_visible(false);
            });
        }

        // Monitor scroll position: show the button when scrolled up.
        {
            let btn = jump_to_bottom_btn.clone();
            let vte = surface.vte_terminal().clone();
            if let Some(adj) = vte.vadjustment() {
                adj.connect_value_changed(move |adj| {
                    let at_bottom = (adj.value() + adj.page_size()) >= (adj.upper() - 1.0);
                    btn.set_visible(!at_bottom);
                });
            }
        }

        container.append(&overlay);

        // Right-click context menu for copy/paste.
        {
            let vte_ref = surface.vte_terminal().clone();
            let popover = gtk4::PopoverMenu::from_model(None::<&gio::MenuModel>);
            popover.set_parent(widget);
            popover.set_has_arrow(false);

            let menu = gio::Menu::new();
            menu.append(Some("Copy"), Some("terminal.copy"));
            menu.append(Some("Paste"), Some("terminal.paste"));
            menu.append(Some("Find"), Some("terminal.find"));
            menu.append(Some("Take Screenshot"), Some("terminal.screenshot"));
            menu.append(Some("Git Diff"), Some("win.toggle-git-diff"));
            popover.set_menu_model(Some(&menu));

            // Register actions on the VTE widget.
            let action_group = gio::SimpleActionGroup::new();

            let copy_action = gio::SimpleAction::new("copy", None);
            let vte_copy = vte_ref.clone();
            copy_action.connect_activate(move |_, _| {
                vte_copy.copy_clipboard_format(vte4::Format::Text);
            });
            action_group.add_action(&copy_action);

            let paste_action = gio::SimpleAction::new("paste", None);
            let vte_paste = vte_ref.clone();
            paste_action.connect_activate(move |_, _| {
                vte_paste.paste_clipboard();
            });
            action_group.add_action(&paste_action);

            let screenshot_action = gio::SimpleAction::new("screenshot", None);
            let vte_screenshot = vte_ref.clone();
            screenshot_action.connect_activate(move |_, _| {
                // Defer until after the popover menu closes so widget.root() works.
                let vte = vte_screenshot.clone();
                glib::idle_add_local_once(move || {
                    let widget = vte.upcast_ref::<gtk4::Widget>();
                    crate::window::take_screenshot_with_feedback(widget);
                });
            });
            action_group.add_action(&screenshot_action);

            let find_action = gio::SimpleAction::new("find", None);
            let search_bar_find = search_bar.clone();
            let search_entry_find = search_entry.clone();
            find_action.connect_activate(move |_, _| {
                let bar = search_bar_find.clone();
                let entry = search_entry_find.clone();
                glib::idle_add_local_once(move || {
                    if bar.is_visible() {
                        bar.set_visible(false);
                    } else {
                        bar.set_visible(true);
                        entry.grab_focus();
                        entry.select_region(0, -1);
                    }
                });
            });
            action_group.add_action(&find_action);

            widget.insert_action_group("terminal", Some(&action_group));

            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // Right click
            let popover_ref = popover.clone();
            let vte_for_menu = vte_ref.clone();
            gesture.connect_released(move |g, _n, x, y| {
                g.set_state(gtk4::EventSequenceState::Claimed);
                // Enable/disable copy based on whether there's a selection.
                copy_action.set_enabled(vte_for_menu.has_selection());
                popover_ref.set_pointing_to(Some(&gdk4::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover_ref.popup();
            });
            widget.add_controller(gesture);
        }

        Self {
            surface,
            container,
            search_bar,
            search_entry,
            jump_to_bottom_btn,
        }
    }

    /// Get the GTK widget for embedding in the UI.
    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Get a reference to the underlying terminal surface.
    pub fn surface(&self) -> &VteSurface {
        &self.surface
    }

    /// Grab keyboard focus.
    pub fn grab_focus(&self) {
        self.surface.grab_focus();
    }

    /// Get the panel ID.
    pub fn panel_id(&self) -> PanelId {
        self.surface.panel_id()
    }

    /// Copy selected text to clipboard.
    pub fn copy_clipboard(&self) {
        self.surface.copy_selection();
    }

    /// Paste from clipboard.
    pub fn paste_clipboard(&self) {
        self.surface.paste_clipboard();
    }

    /// Check if the terminal has a selection.
    pub fn has_selection(&self) -> bool {
        self.surface.has_selection()
    }

    /// Send text to the terminal's child process (as if the user typed it).
    pub fn feed_child(&self, text: &str) {
        self.surface.feed_child(text);
    }

    /// Connect to OSC sequences in raw commit data.
    /// Scans for OSC 9 (iTerm2), OSC 99 (Kitty), and OSC 777 (rxvt) notifications.
    /// Callback receives (osc_number, payload).
    pub fn connect_osc_commit<F: Fn(u32, &str) + 'static>(&self, f: F) {
        self.surface.connect_commit(move |text| {
            // Scan for OSC sequences: ESC ] <number> ; <payload> BEL/ST
            // ESC = \x1b, BEL = \x07, ST = \x1b\\
            scan_osc_sequences(text, &f);
        });
    }

    /// Connect to raw terminal output text for audit/scanning purposes.
    /// The callback receives raw text chunks committed to the terminal.
    pub fn connect_raw_output<F: Fn(&str) + 'static>(&self, f: F) {
        self.surface.connect_commit(f);
    }

    /// Scroll the terminal to the very bottom.
    pub fn scroll_to_bottom(&self) {
        if let Some(adj) = self.surface.vte_terminal().vadjustment() {
            adj.set_value(adj.upper() - adj.page_size());
        }
        self.jump_to_bottom_btn.set_visible(false);
    }

    /// Connect to CWD changes (OSC 7).
    pub fn connect_cwd_changed<F: Fn(Option<String>) + 'static>(&self, f: F) {
        self.surface.connect_cwd_changed(f);
    }

    /// Connect to child process exit.
    pub fn connect_child_exited<F: Fn(i32) + 'static>(&self, f: F) {
        self.surface.connect_child_exited(f);
    }

    /// Connect to terminal bell.
    pub fn connect_bell<F: Fn() + 'static>(&self, f: F) {
        self.surface.connect_bell(f);
    }

    /// Connect to hyperlink clicks. Callback receives (url, shift_held).
    pub fn connect_hyperlink_clicked<F: Fn(&str, bool) + 'static>(&self, f: F) {
        self.surface.connect_hyperlink_clicked(f);
    }

    /// Show the find-in-terminal search bar.
    pub fn show_search(&self) {
        self.search_bar.set_visible(true);
        self.search_entry.grab_focus();
        self.search_entry.select_region(0, -1);
    }

    /// Hide the find-in-terminal search bar and clear highlights.
    pub fn hide_search(&self) {
        self.search_bar.set_visible(false);
        self.surface.search_clear();
    }

    /// Toggle the search bar visibility.
    pub fn toggle_search(&self) {
        if self.search_bar.is_visible() {
            self.hide_search();
        } else {
            self.show_search();
        }
    }

    /// Add or remove the notification ring CSS animation.
    pub fn set_notification_ring(&self, active: bool) {
        if active {
            self.container.add_css_class("notification-ring");
        } else {
            self.container.remove_css_class("notification-ring");
        }
    }
}

/// Scan raw terminal output for OSC sequences (internal, testable).
/// Looks for: ESC ] <number> ; <payload> (BEL | ST)
fn scan_osc_sequences(text: &str, callback: &dyn Fn(u32, &str)) {
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for ESC ] (0x1b 0x5d)
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b']' {
            i += 2; // Skip ESC ]

            // Parse OSC number.
            let num_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }

            if i > num_start && i < bytes.len() && bytes[i] == b';' {
                let num_str = &text[num_start..i];
                if let Ok(osc_num) = num_str.parse::<u32>() {
                    i += 1; // Skip ;

                    // Find the end: BEL (0x07) or ST (ESC \)
                    let payload_start = i;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            // BEL terminator
                            let payload = &text[payload_start..i];
                            callback(osc_num, payload);
                            i += 1;
                            break;
                        } else if bytes[i] == 0x1b
                            && i + 1 < bytes.len()
                            && bytes[i + 1] == b'\\'
                        {
                            // ST terminator
                            let payload = &text[payload_start..i];
                            callback(osc_num, payload);
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    continue;
                }
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn test_scan_osc_bel_terminator() {
        let text = "\x1b]9;Build done!\x07";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert_eq!(results.into_inner(), vec![(9, "Build done!".to_string())]);
    }

    #[test]
    fn test_scan_osc_st_terminator() {
        let text = "\x1b]99;i=1;Task complete\x1b\\";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert_eq!(
            results.into_inner(),
            vec![(99, "i=1;Task complete".to_string())]
        );
    }

    #[test]
    fn test_scan_osc_mixed_with_text() {
        let text = "Hello world\x1b]9;Alert!\x07More text";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert_eq!(results.into_inner(), vec![(9, "Alert!".to_string())]);
    }

    #[test]
    fn test_scan_osc_multiple_sequences() {
        let text = "\x1b]9;First\x07\x1b]9;Second\x07";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert_eq!(
            results.into_inner(),
            vec![
                (9, "First".to_string()),
                (9, "Second".to_string()),
            ]
        );
    }

    #[test]
    fn test_scan_osc_no_sequences() {
        let text = "Plain text with no escape sequences";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert!(results.into_inner().is_empty());
    }

    #[test]
    fn test_scan_osc_no_semicolon() {
        // Malformed: OSC number but no semicolon separator.
        let text = "\x1b]9\x07";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert!(results.into_inner().is_empty());
    }

    #[test]
    fn test_scan_osc_unterminated() {
        // OSC sequence that never gets a BEL or ST terminator.
        let text = "\x1b]9;hello";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert!(results.into_inner().is_empty());
    }

    #[test]
    fn test_scan_osc_empty_payload() {
        // OSC with valid structure but empty payload.
        let text = "\x1b]9;\x07";
        let results = RefCell::new(Vec::new());
        scan_osc_sequences(text, &|num, payload| {
            results.borrow_mut().push((num, payload.to_string()));
        });
        assert_eq!(results.into_inner(), vec![(9, "".to_string())]);
    }
}
