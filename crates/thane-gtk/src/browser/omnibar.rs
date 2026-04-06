use gtk4::prelude::*;

/// Browser address bar widget.
#[derive(Clone)]
pub struct Omnibar {
    container: gtk4::Box,
    entry: gtk4::Entry,
    back_btn: gtk4::Button,
    forward_btn: gtk4::Button,
    reload_btn: gtk4::Button,
    close_btn: gtk4::Button,
}

impl Default for Omnibar {
    fn default() -> Self {
        Self::new()
    }
}

impl Omnibar {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        container.add_css_class("omnibar");
        container.set_margin_start(4);
        container.set_margin_end(4);
        container.set_margin_top(4);
        container.set_margin_bottom(4);

        let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_btn.set_tooltip_text(Some("Back"));
        container.append(&back_btn);

        let forward_btn = gtk4::Button::from_icon_name("go-next-symbolic");
        forward_btn.set_tooltip_text(Some("Forward"));
        container.append(&forward_btn);

        let reload_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        reload_btn.set_tooltip_text(Some("Reload"));
        container.append(&reload_btn);

        let entry = gtk4::Entry::new();
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("Enter URL or search..."));
        container.append(&entry);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.set_tooltip_text(Some("Close browser"));
        close_btn.add_css_class("flat");
        container.append(&close_btn);

        Self {
            container,
            entry,
            back_btn,
            forward_btn,
            reload_btn,
            close_btn,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    pub fn set_url(&self, url: &str) {
        self.entry.set_text(url);
    }

    pub fn get_url(&self) -> String {
        self.entry.text().to_string()
    }

    /// Connect to the navigate signal (Enter pressed in URL bar).
    pub fn connect_navigate<F: Fn(String) + 'static>(&self, f: F) {
        self.entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            // Block dangerous schemes.
            if is_blocked_scheme(&text, false) {
                return;
            }
            let url = normalize_url(&text);
            if !url.is_empty() {
                f(url);
            }
        });
    }

    /// Connect back button click.
    pub fn connect_back<F: Fn() + 'static>(&self, f: F) {
        self.back_btn.connect_clicked(move |_| f());
    }

    /// Connect forward button click.
    pub fn connect_forward<F: Fn() + 'static>(&self, f: F) {
        self.forward_btn.connect_clicked(move |_| f());
    }

    /// Connect reload button click.
    pub fn connect_reload<F: Fn() + 'static>(&self, f: F) {
        self.reload_btn.connect_clicked(move |_| f());
    }

    /// Connect close button click.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }
}

/// Check whether a URL uses a blocked scheme.
///
/// Blocks `javascript:`, `data:`, and `blob:` unconditionally.
/// Blocks `file://` unless `allow_file` is true.
pub fn is_blocked_scheme(url: &str, allow_file: bool) -> bool {
    let lower = url.trim().to_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("blob:") {
        return true;
    }
    if lower.starts_with("file://") && !allow_file {
        return true;
    }
    false
}

/// Normalize user input into a URL.
///
/// If it looks like a URL (has a dot and no spaces), add https://.
/// Otherwise, treat it as a search query.
fn normalize_url(input: &str) -> String {
    let trimmed = input.trim();

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    if trimmed.starts_with("file://") || trimmed.starts_with("about:") {
        return trimmed.to_string();
    }

    // Block dangerous schemes before further normalization.
    if is_blocked_scheme(trimmed, false) {
        return String::new();
    }

    // Looks like a domain (has dot, no spaces).
    if trimmed.contains('.') && !trimmed.contains(' ') {
        return format!("https://{trimmed}");
    }

    // Treat as search query.
    let encoded = trimmed.replace(' ', "+");
    format!("https://duckduckgo.com/?q={encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_url() {
        assert_eq!(
            normalize_url("https://example.com"),
            "https://example.com"
        );
        assert_eq!(
            normalize_url("example.com"),
            "https://example.com"
        );
        assert_eq!(
            normalize_url("rust programming"),
            "https://duckduckgo.com/?q=rust+programming"
        );
    }

    #[test]
    fn test_normalize_url_http_passthrough() {
        assert_eq!(
            normalize_url("http://example.com"),
            "http://example.com"
        );
    }

    #[test]
    fn test_normalize_url_file_protocol() {
        assert_eq!(
            normalize_url("file:///home/user"),
            "file:///home/user"
        );
    }

    #[test]
    fn test_normalize_url_about_protocol() {
        assert_eq!(
            normalize_url("about:blank"),
            "about:blank"
        );
    }

    #[test]
    fn test_normalize_url_whitespace() {
        assert_eq!(
            normalize_url("  example.com  "),
            "https://example.com"
        );
    }

    #[test]
    fn test_blocked_schemes() {
        // javascript: is always blocked.
        assert!(is_blocked_scheme("javascript:alert(1)", false));
        assert!(is_blocked_scheme("JavaScript:void(0)", false));

        // data: is always blocked.
        assert!(is_blocked_scheme("data:text/html,<h1>Hello</h1>", false));

        // blob: is always blocked.
        assert!(is_blocked_scheme("blob:https://example.com/uuid", false));

        // file:// blocked by default.
        assert!(is_blocked_scheme("file:///etc/passwd", false));
        // file:// allowed when explicitly enabled.
        assert!(!is_blocked_scheme("file:///home/user", true));

        // Normal URLs are not blocked.
        assert!(!is_blocked_scheme("https://example.com", false));
        assert!(!is_blocked_scheme("http://localhost:3000", false));
        assert!(!is_blocked_scheme("about:blank", false));
    }

    #[test]
    fn test_normalize_url_blocks_javascript() {
        // javascript: scheme should produce empty string from normalize_url.
        assert_eq!(normalize_url("javascript:alert(1)"), "");
        assert_eq!(normalize_url("data:text/html,<h1>hi</h1>"), "");
    }
}
