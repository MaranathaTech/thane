use thane_core::panel::PanelId;

/// Abstraction over a browser engine.
pub trait BrowserEngine {
    type Surface: BrowserSurface;

    /// Create a new browser surface.
    fn create_surface(&self, panel_id: PanelId, url: &str) -> Self::Surface;
}

/// A single browser surface.
///
/// Platform-agnostic interface for browser operations.
/// Widget access (for embedding in the UI) is provided by concrete backend
/// types (e.g. `WebKitSurface::widget()`) rather than through this trait,
/// since widget types differ across platforms.
pub trait BrowserSurface {
    /// Get the panel ID this surface belongs to.
    fn panel_id(&self) -> PanelId;

    /// Navigate to a URL.
    fn navigate(&self, url: &str);

    /// Get the current URL.
    fn current_url(&self) -> Option<String>;

    /// Get the page title.
    fn title(&self) -> Option<String>;

    /// Execute JavaScript and return the result as a string.
    fn eval_js(&self, script: &str, callback: Box<dyn FnOnce(Result<String, String>)>);

    /// Go back in history.
    fn go_back(&self);

    /// Go forward in history.
    fn go_forward(&self);

    /// Reload the page.
    fn reload(&self);

    /// Grab focus.
    fn grab_focus(&self);

    /// Check if the page is currently loading.
    fn is_loading(&self) -> bool;
}
