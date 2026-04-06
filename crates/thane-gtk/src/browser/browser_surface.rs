use thane_browser::traits::{BrowserEngine, BrowserSurface};
use thane_browser::webkit_backend::{WebKitEngine, WebKitSurface};
use thane_core::panel::PanelId;
use gtk4::prelude::*;
use webkit6::prelude::*;

use super::omnibar::Omnibar;

/// Current Vimium mode for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VimiumMode {
    /// Normal browsing — keys go to the web page.
    Normal,
    /// Hint mode — typing characters to select a link hint.
    Hints,
}

/// GTK wrapper around a WebKit browser surface with omnibar.
#[allow(dead_code)]
pub struct BrowserPanel {
    surface: WebKitSurface,
    omnibar: Omnibar,
    container: gtk4::Box,
    vimium_mode: std::cell::Cell<VimiumMode>,
    hint_buffer: std::cell::RefCell<String>,
}

impl BrowserPanel {
    /// Create a new browser panel widget.
    pub fn new(engine: &WebKitEngine, panel_id: PanelId, url: &str) -> Self {
        let surface = engine.create_surface(panel_id, url);

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Omnibar at the top.
        let omnibar = Omnibar::new();
        omnibar.set_url(url);
        container.append(omnibar.widget());

        // WebView fills the rest.
        let web_widget = surface.widget();
        web_widget.set_hexpand(true);
        web_widget.set_vexpand(true);
        container.append(web_widget);

        // Wire up omnibar navigation.
        {
            let web_view = surface.web_view().clone();
            omnibar.connect_navigate(move |url| {
                web_view.load_uri(&url);
            });
        }

        // Wire up omnibar back/forward/reload buttons.
        {
            let wv = surface.web_view().clone();
            omnibar.connect_back(move || { wv.go_back(); });
        }
        {
            let wv = surface.web_view().clone();
            omnibar.connect_forward(move || { wv.go_forward(); });
        }
        {
            let wv = surface.web_view().clone();
            omnibar.connect_reload(move || { wv.reload(); });
        }

        // Update omnibar URL when page navigation occurs.
        {
            let omnibar_clone = omnibar.clone();
            let wv = surface.web_view().clone();
            surface.connect_load_changed(move |event| {
                if (event == webkit6::LoadEvent::Committed || event == webkit6::LoadEvent::Finished)
                    && let Some(uri) = wv.uri()
                {
                    omnibar_clone.set_url(&uri);
                }
            });
        }

        // Load error page on navigation failure.
        {
            let wv = surface.web_view().clone();
            surface.web_view().connect_load_failed(move |_view, _event, uri, error| {
                let error_html = build_error_page(uri, &error.to_string());
                wv.load_html(&error_html, Some(uri));
                true // we handled it
            });
        }

        // Right-click context menu.
        {
            let wv = surface.web_view().clone();
            let widget = surface.widget().clone();
            let right_click = gtk4::GestureClick::builder()
                .button(3) // right-click
                .build();
            right_click.connect_released(move |gesture, _n_press, x, y| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                show_browser_context_menu(&widget, &wv, x, y);
            });
            surface.widget().add_controller(right_click);
        }

        Self {
            surface,
            omnibar,
            container,
            vimium_mode: std::cell::Cell::new(VimiumMode::Normal),
            hint_buffer: std::cell::RefCell::new(String::new()),
        }
    }

    /// Get the GTK widget for embedding.
    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Get the panel ID.
    pub fn panel_id(&self) -> PanelId {
        self.surface.panel_id()
    }

    /// Grab keyboard focus.
    pub fn grab_focus(&self) {
        self.surface.grab_focus();
    }

    /// Get a reference to the underlying browser surface.
    pub fn surface(&self) -> &WebKitSurface {
        &self.surface
    }

    /// Get the omnibar.
    pub fn omnibar(&self) -> &Omnibar {
        &self.omnibar
    }

    /// Handle a Vimium-style key press. Returns true if the key was consumed.
    pub fn handle_vimium_key(&self, key_name: &str) -> bool {
        use thane_browser::scripting;

        match self.vimium_mode.get() {
            VimiumMode::Normal => {
                match key_name {
                    "f" => {
                        // Enter hint mode.
                        self.vimium_mode.set(VimiumMode::Hints);
                        self.hint_buffer.borrow_mut().clear();
                        self.surface.eval_js(
                            scripting::VIMIUM_SHOW_HINTS_JS,
                            Box::new(|_result| {}),
                        );
                        true
                    }
                    "j" => {
                        self.surface.eval_js(
                            scripting::VIMIUM_SCROLL_DOWN_JS,
                            Box::new(|_| {}),
                        );
                        true
                    }
                    "k" => {
                        self.surface.eval_js(
                            scripting::VIMIUM_SCROLL_UP_JS,
                            Box::new(|_| {}),
                        );
                        true
                    }
                    "g" => {
                        // gg = scroll to top (would need double-tap detection).
                        // For now, single 'g' scrolls to top.
                        self.surface.eval_js(
                            scripting::VIMIUM_SCROLL_TOP_JS,
                            Box::new(|_| {}),
                        );
                        true
                    }
                    "G" => {
                        self.surface.eval_js(
                            scripting::VIMIUM_SCROLL_BOTTOM_JS,
                            Box::new(|_| {}),
                        );
                        true
                    }
                    "H" => {
                        self.surface.go_back();
                        true
                    }
                    "L" => {
                        self.surface.go_forward();
                        true
                    }
                    "r" => {
                        self.surface.reload();
                        true
                    }
                    _ => false,
                }
            }
            VimiumMode::Hints => {
                if key_name == "Escape" {
                    // Cancel hint mode.
                    self.vimium_mode.set(VimiumMode::Normal);
                    self.hint_buffer.borrow_mut().clear();
                    self.surface.eval_js(
                        scripting::VIMIUM_CLEAR_HINTS_JS,
                        Box::new(|_| {}),
                    );
                    return true;
                }

                // Accumulate typed characters as the hint label.
                if key_name.len() == 1 && key_name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
                    self.hint_buffer.borrow_mut().push_str(key_name);
                    let label = self.hint_buffer.borrow().clone();

                    // Try to click the hint with this label.
                    let js = scripting::vimium_click_hint_js(&label);
                    self.surface.eval_js(&js, Box::new(|_result| {}));

                    // Exit hint mode after clicking.
                    self.vimium_mode.set(VimiumMode::Normal);
                    self.hint_buffer.borrow_mut().clear();
                    return true;
                }

                // Unknown key in hint mode — cancel.
                self.vimium_mode.set(VimiumMode::Normal);
                self.hint_buffer.borrow_mut().clear();
                self.surface.eval_js(
                    scripting::VIMIUM_CLEAR_HINTS_JS,
                    Box::new(|_| {}),
                );
                false
            }
        }
    }

    /// Check if the browser is currently in Vimium hint mode.
    pub fn in_hint_mode(&self) -> bool {
        self.vimium_mode.get() == VimiumMode::Hints
    }
}

/// Show a context menu with browser actions at the given position.
fn show_browser_context_menu(widget: &gtk4::Widget, web_view: &webkit6::WebView, x: f64, y: f64) {
    let menu = gio::Menu::new();
    menu.append(Some("Copy URL"), Some("browser.copy-url"));
    menu.append(Some("Open in Default Browser"), Some("browser.open-external"));
    menu.append(Some("Take Screenshot"), Some("browser.screenshot"));

    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(widget);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.set_has_arrow(false);

    // Action group for the context menu items.
    let action_group = gio::SimpleActionGroup::new();

    // Copy URL action.
    let copy_action = gio::SimpleAction::new("copy-url", None);
    {
        let wv = web_view.clone();
        let w = widget.clone();
        copy_action.connect_activate(move |_, _| {
            if let Some(uri) = wv.uri() {
                w.display().clipboard().set_text(&uri);
            }
        });
    }
    action_group.add_action(&copy_action);

    // Open in default browser action.
    let open_action = gio::SimpleAction::new("open-external", None);
    {
        let wv = web_view.clone();
        open_action.connect_activate(move |_, _| {
            if let Some(uri) = wv.uri() {
                let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
            }
        });
    }
    action_group.add_action(&open_action);

    // Screenshot action.
    let screenshot_action = gio::SimpleAction::new("screenshot", None);
    {
        let wv = web_view.clone();
        screenshot_action.connect_activate(move |_, _| {
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let path = format!("/tmp/thane-screenshot-{timestamp}.png");
            let path_clone = path.clone();
            wv.snapshot(
                webkit6::SnapshotRegion::Visible,
                webkit6::SnapshotOptions::NONE,
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(texture) = result {
                        let _ = texture.save_to_png(&path_clone);
                    }
                },
            );
        });
    }
    action_group.add_action(&screenshot_action);

    widget.insert_action_group("browser", Some(&action_group));
    popover.popup();
}

/// Build an inline HTML error page matching the thane dark theme.
fn build_error_page(uri: &str, error: &str) -> String {
    let escaped_uri = glib::markup_escape_text(uri);
    let escaped_error = glib::markup_escape_text(error);
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><style>
body {{
    background: #0c0c0e; color: #e4e4e7; font-family: system-ui, sans-serif;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    min-height: 80vh; margin: 0; padding: 20px;
}}
h2 {{ color: #f87171; font-size: 20px; }}
p {{ color: #a1a1aa; font-size: 14px; max-width: 500px; word-break: break-all; }}
button {{
    background: #818cf8; color: white; border: none; border-radius: 6px;
    padding: 8px 24px; font-size: 14px; cursor: pointer; margin-top: 16px;
}}
button:hover {{ background: #6366f1; }}
</style></head>
<body>
<h2>Page failed to load</h2>
<p>{escaped_error}</p>
<p style="font-size:12px; color:#71717a;">{escaped_uri}</p>
<button onclick="location.reload()">Retry</button>
</body>
</html>"#
    )
}
