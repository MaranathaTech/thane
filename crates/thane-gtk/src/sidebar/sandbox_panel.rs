use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use thane_core::sandbox::{EnforcementLevel, SandboxPolicy};
use gtk4::prelude::*;

type BoolHandler = Rc<RefCell<Option<Box<dyn Fn(bool)>>>>;
type U32Handler = Rc<RefCell<Option<Box<dyn Fn(u32)>>>>;
type VoidHandler = Rc<RefCell<Option<Box<dyn Fn()>>>>;
type PathHandler = Rc<RefCell<Option<Rc<dyn Fn(PathBuf)>>>>;

/// A right-side panel for configuring sandbox settings per workspace.
#[allow(dead_code, clippy::type_complexity)]
pub struct SandboxPanel {
    container: gtk4::Box,
    close_btn: gtk4::Button,
    enabled_switch: gtk4::Switch,
    enforcement_dropdown: gtk4::DropDown,
    root_dir_label: gtk4::Label,
    network_switch: gtk4::Switch,
    ro_list: gtk4::ListBox,
    rw_list: gtk4::ListBox,
    deny_list: gtk4::ListBox,
    add_ro_btn: gtk4::Button,
    add_rw_btn: gtk4::Button,
    add_deny_btn: gtk4::Button,
    landlock_label: gtk4::Label,
    /// Guard flag: when true, suppress handler callbacks (prevents re-entrant borrows).
    updating: Rc<Cell<bool>>,
    // Callback holders.
    enabled_handler: BoolHandler,
    enforcement_handler: U32Handler,
    network_handler: BoolHandler,
    add_ro_handler: VoidHandler,
    add_rw_handler: VoidHandler,
    add_deny_handler: VoidHandler,
    remove_ro_handler: PathHandler,
    remove_rw_handler: PathHandler,
    remove_deny_handler: PathHandler,
}

impl Default for SandboxPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("sandbox-panel");
        container.set_width_request(400);

        // Header.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(4);

        let title = gtk4::Label::new(Some("Sandbox"));
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

        // Scrollable content.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);

        // Enable toggle.
        let enable_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let enable_lbl = gtk4::Label::new(Some("Sandbox Enabled"));
        enable_lbl.set_hexpand(true);
        enable_lbl.set_halign(gtk4::Align::Start);
        enable_row.append(&enable_lbl);

        let enabled_switch = gtk4::Switch::new();
        enabled_switch.set_active(false);
        enabled_switch.set_valign(gtk4::Align::Center);
        enable_row.append(&enabled_switch);

        content.append(&enable_row);

        // Enforcement dropdown.
        let enforce_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let enforce_lbl = gtk4::Label::new(Some("Enforcement Level"));
        enforce_lbl.set_hexpand(true);
        enforce_lbl.set_halign(gtk4::Align::Start);
        enforce_row.append(&enforce_lbl);

        let levels = gtk4::StringList::new(&["Permissive", "Enforcing", "Strict"]);
        let enforcement_dropdown = gtk4::DropDown::new(Some(levels), gtk4::Expression::NONE);
        enforcement_dropdown.set_selected(1); // Enforcing default
        enforce_row.append(&enforcement_dropdown);

        content.append(&enforce_row);

        // Root dir (read-only).
        let root_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let root_lbl = gtk4::Label::new(Some("Root Directory"));
        root_lbl.set_halign(gtk4::Align::Start);
        root_lbl.add_css_class("sandbox-section-title");
        root_row.append(&root_lbl);

        let root_dir_label = gtk4::Label::new(Some("(not set)"));
        root_dir_label.add_css_class("sandbox-path-item");
        root_dir_label.set_hexpand(true);
        root_dir_label.set_halign(gtk4::Align::End);
        root_dir_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        root_row.append(&root_dir_label);

        content.append(&root_row);

        // ── Access section ──
        let access_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        access_sep.set_margin_top(4);
        content.append(&access_sep);

        let access_title = gtk4::Label::new(Some("Access"));
        access_title.add_css_class("sandbox-section-title");
        access_title.set_halign(gtk4::Align::Start);
        content.append(&access_title);

        // Network switch.
        let net_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let net_lbl = gtk4::Label::new(Some("Network Access"));
        net_lbl.set_hexpand(true);
        net_lbl.set_halign(gtk4::Align::Start);
        net_row.append(&net_lbl);

        let network_switch = gtk4::Switch::new();
        network_switch.set_active(true);
        network_switch.set_valign(gtk4::Align::Center);
        net_row.append(&network_switch);

        content.append(&net_row);

        // ── Paths section ──
        let paths_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        paths_sep.set_margin_top(4);
        content.append(&paths_sep);

        let paths_title = gtk4::Label::new(Some("Paths"));
        paths_title.add_css_class("sandbox-section-title");
        paths_title.set_halign(gtk4::Align::Start);
        content.append(&paths_title);

        // Read-only paths.
        let ro_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let ro_lbl = gtk4::Label::new(Some("Read-Only"));
        ro_lbl.set_hexpand(true);
        ro_lbl.set_halign(gtk4::Align::Start);
        ro_header.append(&ro_lbl);

        let add_ro_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        add_ro_btn.add_css_class("flat");
        add_ro_btn.set_tooltip_text(Some("Add read-only path"));
        ro_header.append(&add_ro_btn);

        content.append(&ro_header);

        let ro_list = gtk4::ListBox::new();
        ro_list.set_selection_mode(gtk4::SelectionMode::None);
        ro_list.add_css_class("boxed-list");
        content.append(&ro_list);

        // Read-write paths.
        let rw_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let rw_lbl = gtk4::Label::new(Some("Read-Write"));
        rw_lbl.set_hexpand(true);
        rw_lbl.set_halign(gtk4::Align::Start);
        rw_header.append(&rw_lbl);

        let add_rw_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        add_rw_btn.add_css_class("flat");
        add_rw_btn.set_tooltip_text(Some("Add read-write path"));
        rw_header.append(&add_rw_btn);

        content.append(&rw_header);

        let rw_list = gtk4::ListBox::new();
        rw_list.set_selection_mode(gtk4::SelectionMode::None);
        rw_list.add_css_class("boxed-list");
        content.append(&rw_list);

        // Denied paths.
        let deny_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let deny_lbl = gtk4::Label::new(Some("Denied"));
        deny_lbl.set_hexpand(true);
        deny_lbl.set_halign(gtk4::Align::Start);
        deny_header.append(&deny_lbl);

        let add_deny_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        add_deny_btn.add_css_class("flat");
        add_deny_btn.set_tooltip_text(Some("Add denied path"));
        deny_header.append(&add_deny_btn);

        content.append(&deny_header);

        let deny_list = gtk4::ListBox::new();
        deny_list.set_selection_mode(gtk4::SelectionMode::None);
        deny_list.add_css_class("boxed-list");
        content.append(&deny_list);

        // Landlock support indicator.
        let landlock_label = gtk4::Label::new(None);
        landlock_label.add_css_class("panel-meta");
        landlock_label.set_halign(gtk4::Align::Start);
        landlock_label.set_margin_top(12);
        content.append(&landlock_label);

        scrolled.set_child(Some(&content));
        container.append(&scrolled);

        let enabled_handler: BoolHandler = Rc::new(RefCell::new(None));
        let enforcement_handler: U32Handler = Rc::new(RefCell::new(None));
        let network_handler: BoolHandler = Rc::new(RefCell::new(None));
        let add_ro_handler: VoidHandler = Rc::new(RefCell::new(None));
        let add_rw_handler: VoidHandler = Rc::new(RefCell::new(None));
        let add_deny_handler: VoidHandler = Rc::new(RefCell::new(None));

        let updating: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        // Wire switch/dropdown signals to handler refs.
        // Each checks the `updating` guard to suppress callbacks during programmatic updates.
        {
            let h = enabled_handler.clone();
            let guard = updating.clone();
            enabled_switch.connect_active_notify(move |sw| {
                if guard.get() { return; }
                if let Some(f) = h.borrow().as_ref() {
                    f(sw.is_active());
                }
            });
        }
        {
            let h = enforcement_handler.clone();
            let guard = updating.clone();
            enforcement_dropdown.connect_selected_notify(move |dd| {
                if guard.get() { return; }
                if let Some(f) = h.borrow().as_ref() {
                    f(dd.selected());
                }
            });
        }
        {
            let h = network_handler.clone();
            let guard = updating.clone();
            network_switch.connect_active_notify(move |sw| {
                if guard.get() { return; }
                if let Some(f) = h.borrow().as_ref() {
                    f(sw.is_active());
                }
            });
        }
        {
            let h = add_ro_handler.clone();
            add_ro_btn.connect_clicked(move |_| {
                if let Some(f) = h.borrow().as_ref() {
                    f();
                }
            });
        }
        {
            let h = add_rw_handler.clone();
            add_rw_btn.connect_clicked(move |_| {
                if let Some(f) = h.borrow().as_ref() {
                    f();
                }
            });
        }
        {
            let h = add_deny_handler.clone();
            add_deny_btn.connect_clicked(move |_| {
                if let Some(f) = h.borrow().as_ref() {
                    f();
                }
            });
        }

        Self {
            container,
            close_btn,
            enabled_switch,
            enforcement_dropdown,
            root_dir_label,
            network_switch,
            ro_list,
            rw_list,
            deny_list,
            add_ro_btn,
            add_rw_btn,
            add_deny_btn,
            landlock_label,
            updating,
            enabled_handler,
            enforcement_handler,
            network_handler,
            add_ro_handler,
            add_rw_handler,
            add_deny_handler,
            remove_ro_handler: Rc::new(RefCell::new(None)),
            remove_rw_handler: Rc::new(RefCell::new(None)),
            remove_deny_handler: Rc::new(RefCell::new(None)),
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Connect close button.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }

    // ── Connect callbacks (stored in RefCell so they can be set after construction) ──

    pub fn connect_enabled_changed<F: Fn(bool) + 'static>(&self, f: F) {
        *self.enabled_handler.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_enforcement_changed<F: Fn(u32) + 'static>(&self, f: F) {
        *self.enforcement_handler.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_network_changed<F: Fn(bool) + 'static>(&self, f: F) {
        *self.network_handler.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_add_ro<F: Fn() + 'static>(&self, f: F) {
        *self.add_ro_handler.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_add_rw<F: Fn() + 'static>(&self, f: F) {
        *self.add_rw_handler.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_add_deny<F: Fn() + 'static>(&self, f: F) {
        *self.add_deny_handler.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_remove_ro<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.remove_ro_handler.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_remove_rw<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.remove_rw_handler.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_remove_deny<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.remove_deny_handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Update the panel to reflect the current sandbox policy.
    pub fn update(&self, policy: &SandboxPolicy) {
        // Suppress handler callbacks during programmatic updates to avoid re-entrant borrows.
        self.updating.set(true);

        self.enabled_switch.set_active(policy.enabled);

        let enforce_idx = match policy.enforcement {
            EnforcementLevel::Permissive => 0,
            EnforcementLevel::Enforcing => 1,
            EnforcementLevel::Strict => 2,
        };
        self.enforcement_dropdown.set_selected(enforce_idx);

        let root_text = policy.root_dir.to_string_lossy();
        self.root_dir_label.set_text(if root_text.is_empty() {
            "(not set)"
        } else {
            &root_text
        });

        self.network_switch.set_active(policy.allow_network);

        // Populate path lists.
        self.rebuild_path_list(&self.ro_list, &policy.read_only_paths, &self.remove_ro_handler);
        self.rebuild_path_list(&self.rw_list, &policy.read_write_paths, &self.remove_rw_handler);
        self.rebuild_path_list(&self.deny_list, &policy.denied_paths, &self.remove_deny_handler);

        // Landlock indicator.
        if thane_platform::is_landlock_supported() {
            self.landlock_label
                .set_text("Kernel enforcement: supported");
        } else {
            self.landlock_label
                .set_text("Kernel enforcement: not supported");
        }

        self.updating.set(false);
    }

    fn rebuild_path_list(
        &self,
        list: &gtk4::ListBox,
        paths: &[PathBuf],
        remove_handler: &PathHandler,
    ) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        if paths.is_empty() {
            let empty = gtk4::Label::new(Some("(none)"));
            empty.add_css_class("panel-meta");
            empty.set_margin_start(8);
            empty.set_margin_top(4);
            empty.set_margin_bottom(4);
            list.append(&empty);
            return;
        }

        for path in paths {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            row.set_margin_start(8);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            let path_label = gtk4::Label::new(Some(&path.to_string_lossy()));
            path_label.add_css_class("sandbox-path-item");
            path_label.set_hexpand(true);
            path_label.set_halign(gtk4::Align::Start);
            path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
            row.append(&path_label);

            let remove_btn = gtk4::Button::from_icon_name("edit-delete-symbolic");
            remove_btn.add_css_class("flat");
            remove_btn.add_css_class("sandbox-path-remove");
            remove_btn.set_tooltip_text(Some("Remove"));
            remove_btn.set_focusable(false);

            let path_clone = path.clone();
            let handler_ref = remove_handler.clone();
            remove_btn.connect_clicked(move |_| {
                if let Some(handler) = handler_ref.borrow().as_ref() {
                    handler(path_clone.clone());
                }
            });
            row.append(&remove_btn);

            list.append(&row);
        }
    }
}
