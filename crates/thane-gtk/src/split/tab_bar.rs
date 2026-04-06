use std::cell::RefCell;
use std::rc::Rc;

use thane_core::panel::{PanelId, PanelType};
use gdk4::prelude::*;
use gtk4::prelude::*;

/// Information about a single tab in the tab bar.
pub struct TabInfo {
    pub panel_id: PanelId,
    pub title: String,
    pub panel_type: PanelType,
    pub is_selected: bool,
}

/// A horizontal tab bar showing panel tabs within a pane.
/// Always visible — shows action buttons even for a single tab.
pub struct TabBar {
    container: gtk4::Box,
    on_reorder: Rc<RefCell<Option<Rc<dyn Fn(PanelId, PanelId)>>>>,
}

impl TabBar {
    /// Create a new tab bar.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tabs: &[TabInfo],
        on_select: Rc<dyn Fn(PanelId)>,
        on_close: Rc<dyn Fn(PanelId)>,
        on_screenshot: Rc<dyn Fn()>,
        on_git_diff: Rc<dyn Fn()>,
        on_find: Rc<dyn Fn()>,
        on_split_right: Rc<dyn Fn()>,
        on_split_down: Rc<dyn Fn()>,
        on_close_pane: Rc<dyn Fn()>,
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        container.add_css_class("tab-bar");

        let on_reorder: Rc<RefCell<Option<Rc<dyn Fn(PanelId, PanelId)>>>> =
            Rc::new(RefCell::new(None));

        // Tab items (scrollable region).
        let tabs_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        tabs_box.set_hexpand(true);

        // Only show individual tab items when there are 2+ tabs.
        if tabs.len() >= 2 {
            for tab in tabs {
                let tab_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                tab_box.add_css_class("tab-item");
                if tab.is_selected {
                    tab_box.add_css_class("tab-item-selected");
                }

                // Type icon.
                let icon_name = match tab.panel_type {
                    PanelType::Terminal => "utilities-terminal-symbolic",
                    PanelType::Browser => "web-browser-symbolic",
                };
                let icon = gtk4::Image::from_icon_name(icon_name);
                icon.add_css_class("tab-icon");
                icon.set_pixel_size(14);
                tab_box.append(&icon);

                // Title label (ellipsized).
                let label = gtk4::Label::new(Some(&tab.title));
                label.add_css_class("tab-title");
                label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                label.set_max_width_chars(20);
                tab_box.append(&label);

                // Close button.
                let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
                close_btn.add_css_class("flat");
                close_btn.add_css_class("tab-close-btn");
                close_btn.set_tooltip_text(Some("Close tab"));
                tab_box.append(&close_btn);

                // Tab click → select.
                let panel_id = tab.panel_id;
                let select_cb = on_select.clone();
                let gesture = gtk4::GestureClick::new();
                gesture.connect_released(move |g, _n, _x, _y| {
                    g.set_state(gtk4::EventSequenceState::Claimed);
                    select_cb(panel_id);
                });
                tab_box.add_controller(gesture);

                // Close click.
                let close_panel_id = tab.panel_id;
                let close_cb = on_close.clone();
                close_btn.connect_clicked(move |_| {
                    close_cb(close_panel_id);
                });

                // --- Drag-and-drop reordering ---

                // DragSource: store panel UUID as string content.
                let drag_source = gtk4::DragSource::new();
                drag_source.set_actions(gdk4::DragAction::MOVE);
                let drag_panel_id = tab.panel_id;
                drag_source.connect_prepare(move |_src, _x, _y| {
                    let provider = gdk4::ContentProvider::for_value(
                        &drag_panel_id.to_string().to_value(),
                    );
                    Some(provider)
                });
                tab_box.add_controller(drag_source);

                // DropTarget: accept string, extract UUID, call reorder callback.
                let drop_target = gtk4::DropTarget::new(glib::Type::STRING, gdk4::DragAction::MOVE);
                let drop_panel_id = tab.panel_id;
                let reorder_ref = on_reorder.clone();
                drop_target.connect_drop(move |_target, value, _x, _y| {
                    if let Ok(source_str) = value.get::<String>() {
                        if let Ok(source_id) = source_str.parse::<uuid::Uuid>() {
                            if source_id != drop_panel_id {
                                if let Some(ref cb) = *reorder_ref.borrow() {
                                    cb(source_id, drop_panel_id);
                                }
                            }
                            return true;
                        }
                    }
                    false
                });
                tab_box.add_controller(drop_target);

                tabs_box.append(&tab_box);
            }
        }

        container.append(&tabs_box);

        // Left-aligned action buttons (screenshot, git diff).
        let left_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        left_actions.add_css_class("tab-bar-actions-left");

        let screenshot_btn = gtk4::Button::from_icon_name("camera-photo-symbolic");
        screenshot_btn.add_css_class("flat");
        screenshot_btn.add_css_class("tab-action-btn");
        screenshot_btn.set_tooltip_text(Some("Screenshot (saves to /tmp, copies path)"));
        screenshot_btn.set_focusable(false);
        screenshot_btn.connect_clicked(move |_| on_screenshot());
        left_actions.append(&screenshot_btn);

        let git_diff_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
        git_diff_btn.add_css_class("flat");
        git_diff_btn.add_css_class("tab-action-btn");
        git_diff_btn.set_tooltip_text(Some("Git diff (Ctrl+Shift+G)"));
        git_diff_btn.set_focusable(false);
        git_diff_btn.connect_clicked(move |_| on_git_diff());
        left_actions.append(&git_diff_btn);

        let find_btn = gtk4::Button::from_icon_name("edit-find-symbolic");
        find_btn.add_css_class("flat");
        find_btn.add_css_class("tab-action-btn");
        find_btn.set_tooltip_text(Some("Find (Ctrl+Shift+F)"));
        find_btn.set_focusable(false);
        find_btn.connect_clicked(move |_| on_find());
        left_actions.append(&find_btn);

        container.append(&left_actions);

        // Right-aligned action buttons (split, close).
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        actions.add_css_class("tab-bar-actions");

        let split_right_btn = gtk4::Button::from_icon_name("object-flip-horizontal-symbolic");
        split_right_btn.add_css_class("flat");
        split_right_btn.add_css_class("tab-action-btn");
        split_right_btn.set_tooltip_text(Some("Split right (Ctrl+Shift+D)"));
        split_right_btn.set_focusable(false);
        split_right_btn.connect_clicked(move |_| on_split_right());
        actions.append(&split_right_btn);

        let split_down_btn = gtk4::Button::from_icon_name("object-flip-vertical-symbolic");
        split_down_btn.add_css_class("flat");
        split_down_btn.add_css_class("tab-action-btn");
        split_down_btn.set_tooltip_text(Some("Split down (Ctrl+Shift+E)"));
        split_down_btn.set_focusable(false);
        split_down_btn.connect_clicked(move |_| on_split_down());
        actions.append(&split_down_btn);

        let close_pane_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_pane_btn.add_css_class("flat");
        close_pane_btn.add_css_class("tab-action-btn");
        close_pane_btn.add_css_class("tab-close-pane-btn");
        close_pane_btn.set_tooltip_text(Some("Close pane (Ctrl+Shift+X)"));
        close_pane_btn.set_focusable(false);
        close_pane_btn.connect_clicked(move |_| on_close_pane());
        actions.append(&close_pane_btn);

        container.append(&actions);

        Self { container, on_reorder }
    }

    /// Connect a callback that receives (source_panel_id, target_panel_id) on tab reorder.
    pub fn connect_reorder<F: Fn(PanelId, PanelId) + 'static>(&self, f: F) {
        *self.on_reorder.borrow_mut() = Some(Rc::new(f));
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }
}
