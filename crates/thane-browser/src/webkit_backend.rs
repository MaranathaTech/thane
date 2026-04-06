use thane_core::panel::PanelId;
use gtk4::prelude::*;
use webkit6::prelude::*;

use crate::traits::{BrowserEngine, BrowserSurface};

/// WebKitGTK6-based browser engine for Linux.
pub struct WebKitEngine;

impl WebKitEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebKitEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserEngine for WebKitEngine {
    type Surface = WebKitSurface;

    fn create_surface(&self, panel_id: PanelId, url: &str) -> WebKitSurface {
        let web_view = webkit6::WebView::new();

        // Enable developer tools
        let settings = webkit6::prelude::WebViewExt::settings(&web_view);
        if let Some(settings) = settings {
            settings.set_enable_developer_extras(true);
            settings.set_enable_javascript(true);
            settings.set_enable_javascript_markup(true);
        }

        // Handle the `create` signal so that target="_blank" links and window.open()
        // navigate in the same view instead of opening a blank GTK window.
        {
            let view_clone = web_view.clone();
            web_view.connect_create(move |_view, nav_action| {
                if let Some(request) = nav_action.clone().request()
                    && let Some(uri) = request.uri()
                {
                    view_clone.load_uri(&uri);
                }
                // Return the same widget so WebKit doesn't create a new window.
                // Since we already loaded the URI above, the navigation is handled.
                _view.clone().upcast::<gtk4::Widget>()
            });
        }

        web_view.load_uri(url);

        WebKitSurface {
            web_view,
            panel_id,
        }
    }
}

/// A WebKitGTK6 browser surface.
pub struct WebKitSurface {
    web_view: webkit6::WebView,
    panel_id: PanelId,
}

impl BrowserSurface for WebKitSurface {
    fn panel_id(&self) -> PanelId {
        self.panel_id
    }

    fn navigate(&self, url: &str) {
        self.web_view.load_uri(url);
    }

    fn current_url(&self) -> Option<String> {
        self.web_view.uri().map(|s| s.to_string())
    }

    fn title(&self) -> Option<String> {
        self.web_view.title().map(|s| s.to_string())
    }

    fn eval_js(&self, script: &str, callback: Box<dyn FnOnce(Result<String, String>)>) {
        self.web_view.evaluate_javascript(
            script,
            None::<&str>,   // world_name
            None::<&str>,   // source_uri
            None::<&gtk4::gio::Cancellable>,
            move |result: Result<javascriptcore6::Value, glib::Error>| {
                match result {
                    Ok(value) => {
                        let text = value
                            .to_string();
                        callback(Ok(text));
                    }
                    Err(e) => {
                        callback(Err(e.to_string()));
                    }
                }
            },
        );
    }

    fn go_back(&self) {
        self.web_view.go_back();
    }

    fn go_forward(&self) {
        self.web_view.go_forward();
    }

    fn reload(&self) {
        self.web_view.reload();
    }

    fn grab_focus(&self) {
        self.web_view.grab_focus();
    }

    fn is_loading(&self) -> bool {
        self.web_view.is_loading()
    }
}

impl WebKitSurface {
    /// Get the underlying GTK widget (for embedding in the UI).
    pub fn widget(&self) -> &gtk4::Widget {
        self.web_view.upcast_ref()
    }

    /// Get a reference to the underlying WebView widget.
    pub fn web_view(&self) -> &webkit6::WebView {
        &self.web_view
    }

    /// Connect to the `load-changed` signal.
    pub fn connect_load_changed<F: Fn(webkit6::LoadEvent) + 'static>(&self, f: F) {
        self.web_view.connect_load_changed(move |_view, event| {
            f(event);
        });
    }

    /// Connect to the `title` property notification.
    pub fn connect_title_changed<F: Fn(String) + 'static>(&self, f: F) {
        self.web_view.connect_title_notify(move |view| {
            let title = view
                .title()
                .map(|s| s.to_string())
                .unwrap_or_default();
            f(title);
        });
    }

    /// Connect to the `load-failed` signal.
    ///
    /// The callback receives (load_event, failing_uri, error).
    /// Return true if the error was handled (prevents default error page).
    pub fn connect_load_failed<F: Fn(webkit6::LoadEvent, &str, &glib::Error) -> bool + 'static>(
        &self,
        f: F,
    ) {
        self.web_view.connect_load_failed(move |_view, event, uri, error| {
            f(event, uri, error)
        });
    }
}
