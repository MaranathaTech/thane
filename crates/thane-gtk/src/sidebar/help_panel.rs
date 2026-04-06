use gtk4::prelude::*;

/// A right-side panel showing keyboard shortcuts and usage tips.
pub struct HelpPanel {
    container: gtk4::Box,
    close_btn: gtk4::Button,
}

impl Default for HelpPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("help-panel");
        container.set_width_request(320);

        // Header with close button.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("Keyboard Shortcuts"));
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

        // Scrollable content area.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(12);
        content.set_margin_bottom(16);

        // Shortcut sections.
        add_shortcut_section(
            &content,
            "Workspaces",
            &[
                ("Ctrl+Shift+T", "New workspace"),
                ("Ctrl+Shift+W", "Close panel"),
                ("Ctrl+Tab", "Next workspace"),
                ("Ctrl+Shift+Tab", "Previous workspace"),
            ],
        );

        add_shortcut_section(
            &content,
            "Split Panes",
            &[
                ("Ctrl+Shift+D", "Split right"),
                ("Ctrl+Shift+E", "Split down"),
                ("Ctrl+Shift+X", "Close pane"),
                ("Ctrl+Shift+]", "Next pane"),
                ("Ctrl+Shift+[", "Previous pane"),
                ("Alt+H/J/K/L", "Focus pane (vim-style)"),
                ("Ctrl+Shift+Z", "Toggle pane zoom"),
            ],
        );

        add_shortcut_section(
            &content,
            "Panel Tabs",
            &[
                ("Ctrl+PgDn", "Next panel tab"),
                ("Ctrl+PgUp", "Previous panel tab"),
            ],
        );

        add_shortcut_section(
            &content,
            "UI Panels",
            &[
                ("Ctrl+,", "Settings"),
                ("Ctrl+I", "Notifications"),
                ("Ctrl+Shift+A", "Audit log"),
                ("Ctrl+Shift+G", "Git diff (tab bar)"),
                ("Ctrl+Shift+U", "Token usage"),
                ("Ctrl+Shift+P", "Agent queue"),
                ("F1", "Help (this panel)"),
            ],
        );

        add_shortcut_section(
            &content,
            "Terminal",
            &[
                ("Ctrl+Shift+C", "Copy"),
                ("Ctrl+Shift+V", "Paste"),
                ("Ctrl+Shift+F", "Find in terminal"),
            ],
        );

        add_shortcut_section(
            &content,
            "Font Size",
            &[
                ("Ctrl+=", "Increase font size"),
                ("Ctrl+-", "Decrease font size"),
                ("Ctrl+0", "Reset font size"),
            ],
        );

        add_shortcut_section(
            &content,
            "Leader Mode (Ctrl+B, then...)",
            &[
                ("n / p", "Next / previous workspace"),
                ("c", "New workspace"),
                ("x", "Close workspace"),
                (",", "Rename workspace"),
                ("1..9", "Select workspace by number"),
            ],
        );

        add_shortcut_section(
            &content,
            "Other",
            &[
                ("Ctrl+Shift+B", "Toggle sidebar"),
                ("Ctrl+B", "Enter leader mode"),
                ("Ctrl+Shift+R", "Reload config"),
                ("F11", "Toggle fullscreen"),
            ],
        );

        // Tips section.
        let tips_title = gtk4::Label::new(Some("TIPS"));
        tips_title.add_css_class("help-section-title");
        tips_title.set_halign(gtk4::Align::Start);
        tips_title.set_margin_top(8);
        content.append(&tips_title);

        let tips = [
            "Right-click a workspace to rename it.",
            "Click port badges in the sidebar to open a browser tab.",
            "Agent activity appears in the status bar.",
            "Config file: ~/.config/thane/config",
            "Session auto-saves every 8 seconds.",
            "Right-side panels are mutually exclusive (except git diff).",
        ];

        for tip in &tips {
            let label = gtk4::Label::new(Some(tip));
            label.add_css_class("help-tip");
            label.set_halign(gtk4::Align::Start);
            label.set_wrap(true);
            label.set_xalign(0.0);
            content.append(&label);
        }

        scrolled.set_child(Some(&content));
        container.append(&scrolled);

        Self {
            container,
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
}

/// Add a section of keyboard shortcuts to the content box.
fn add_shortcut_section(content: &gtk4::Box, title: &str, shortcuts: &[(&str, &str)]) {
    let section_title = gtk4::Label::new(Some(title));
    section_title.add_css_class("help-section-title");
    section_title.set_halign(gtk4::Align::Start);
    content.append(&section_title);

    let grid = gtk4::Grid::new();
    grid.set_row_spacing(4);
    grid.set_column_spacing(12);

    for (i, (key, desc)) in shortcuts.iter().enumerate() {
        let key_label = gtk4::Label::new(Some(key));
        key_label.add_css_class("help-shortcut-key");
        key_label.set_halign(gtk4::Align::Start);
        grid.attach(&key_label, 0, i as i32, 1, 1);

        let desc_label = gtk4::Label::new(Some(desc));
        desc_label.add_css_class("help-shortcut-desc");
        desc_label.set_halign(gtk4::Align::Start);
        desc_label.set_hexpand(true);
        grid.attach(&desc_label, 1, i as i32, 1, 1);
    }

    content.append(&grid);
}
