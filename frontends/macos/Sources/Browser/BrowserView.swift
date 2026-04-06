import AppKit
import WebKit

/// Wraps a WKWebView for embedding in thane's panel system.
///
/// Handles:
/// - Initial URL loading from bridge
/// - `target="_blank"` links (redirect to same view)
/// - Developer extras and JavaScript
/// - Load state and title change delegation
/// - Vimium-style keyboard navigation
/// - Screenshot capture
@MainActor
final class BrowserView: NSView {

    private(set) var webView: WKWebView!
    private let panelId: String

    /// Delegate for load/title/URL changes.
    weak var delegate: BrowserViewDelegate?

    /// Current Vimium mode.
    private var vimiumMode: VimiumMode = .normal

    /// Accumulated characters during hint mode.
    private var hintBuffer = ""

    // MARK: - Init

    init(panelId: String, initialURL: String) {
        self.panelId = panelId
        super.init(frame: .zero)

        let config = WKWebViewConfiguration()
        // Developer extras disabled in production to prevent Web Inspector access
        #if DEBUG
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        #endif

        let thaneWebView = ThaneWebView(frame: .zero, configuration: config)
        thaneWebView.parentBrowserView = self
        webView = thaneWebView
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.translatesAutoresizingMaskIntoConstraints = false
        webView.allowsBackForwardNavigationGestures = true

        addSubview(webView)
        NSLayoutConstraint.activate([
            webView.topAnchor.constraint(equalTo: topAnchor),
            webView.leadingAnchor.constraint(equalTo: leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: trailingAnchor),
            webView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Observe title changes via KVO.
        webView.addObserver(self, forKeyPath: "title", options: .new, context: nil)
        webView.addObserver(self, forKeyPath: "URL", options: .new, context: nil)

        // Load the initial URL.
        if let url = URL(string: initialURL) {
            webView.load(URLRequest(url: url))
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        webView?.removeObserver(self, forKeyPath: "title")
        webView?.removeObserver(self, forKeyPath: "URL")
    }

    // MARK: - KVO

    override nonisolated func observeValue(
        forKeyPath keyPath: String?,
        of object: Any?,
        change: [NSKeyValueChangeKey: Any]?,
        context: UnsafeMutableRawPointer?
    ) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            switch keyPath {
            case "title":
                self.delegate?.browserView(self, titleDidChange: self.webView.title ?? "")
            case "URL":
                self.delegate?.browserView(self, urlDidChange: self.webView.url?.absoluteString ?? "")
            default:
                break
            }
        }
    }

    // MARK: - Navigation

    func navigate(to urlString: String) {
        let url = BrowserView.normalizeURL(urlString)
        if let parsed = URL(string: url) {
            webView.load(URLRequest(url: parsed))
        }
    }

    func goBack() { webView.goBack() }
    func goForward() { webView.goForward() }
    func reload() { webView.reload() }

    var currentURL: String? { webView.url?.absoluteString }
    var currentTitle: String? { webView.title }

    // MARK: - JavaScript evaluation

    func evaluateJavaScript(_ script: String, completion: ((Result<String, Error>) -> Void)? = nil) {
        webView.evaluateJavaScript(script) { result, error in
            if let error {
                completion?(.failure(error))
            } else if let value = result {
                completion?(.success(String(describing: value)))
            } else {
                completion?(.success(""))
            }
        }
    }

    // MARK: - Screenshot

    func takeScreenshot(completion: @escaping (NSImage?) -> Void) {
        let config = WKSnapshotConfiguration()
        webView.takeSnapshot(with: config) { image, error in
            if let error {
                NSLog("thane: browser screenshot failed: \(error)")
            }
            completion(image)
        }
    }

    // MARK: - Vimium keyboard navigation

    /// Returns true if the key was consumed by Vimium handling.
    func handleVimiumKey(_ key: String) -> Bool {
        switch vimiumMode {
        case .normal:
            return handleVimiumNormalKey(key)
        case .hints:
            return handleVimiumHintKey(key)
        }
    }

    /// Whether the browser is currently in Vimium hint mode.
    var inHintMode: Bool { vimiumMode == .hints }

    private func handleVimiumNormalKey(_ key: String) -> Bool {
        switch key {
        case "f":
            vimiumMode = .hints
            hintBuffer = ""
            evaluateJavaScript(BrowserScripting.showHintsJS)
            return true
        case "j":
            evaluateJavaScript(BrowserScripting.scrollDownJS)
            return true
        case "k":
            evaluateJavaScript(BrowserScripting.scrollUpJS)
            return true
        case "g":
            evaluateJavaScript(BrowserScripting.scrollTopJS)
            return true
        case "G":
            evaluateJavaScript(BrowserScripting.scrollBottomJS)
            return true
        case "H":
            goBack()
            return true
        case "L":
            goForward()
            return true
        case "r":
            reload()
            return true
        default:
            return false
        }
    }

    private func handleVimiumHintKey(_ key: String) -> Bool {
        if key == "Escape" {
            vimiumMode = .normal
            hintBuffer = ""
            evaluateJavaScript(BrowserScripting.clearHintsJS)
            return true
        }

        // Accumulate lowercase characters as hint label prefix.
        if key.count == 1, let ch = key.first, ch.isLowercase {
            hintBuffer.append(key)
            let js = BrowserScripting.matchHintJS(prefix: hintBuffer)
            evaluateJavaScript(js) { [weak self] result in
                guard let self else { return }
                DispatchQueue.main.async {
                    MainActor.assumeIsolated {
                        switch result {
                        case .success(let value):
                            if value == "clicked" || value == "navigated" {
                                self.vimiumMode = .normal
                                self.hintBuffer = ""
                            } else if value == "0" {
                                self.vimiumMode = .normal
                                self.hintBuffer = ""
                            }
                            // Otherwise multiple matches remain — stay in hint mode
                        case .failure:
                            self.vimiumMode = .normal
                            self.hintBuffer = ""
                        }
                    }
                }
            }
            return true
        }

        // Unknown key in hint mode — cancel.
        vimiumMode = .normal
        hintBuffer = ""
        evaluateJavaScript(BrowserScripting.clearHintsJS)
        return false
    }

    // MARK: - URL normalization

    /// URI schemes that are blocked for security (prevent local file access, script injection).
    private static let blockedSchemes: Set<String> = ["file", "javascript", "data", "blob"]

    static func normalizeURL(_ input: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespaces)

        // Block dangerous URI schemes
        let lowered = trimmed.lowercased()
        for scheme in blockedSchemes {
            if lowered.hasPrefix("\(scheme):") {
                NSLog("thane: blocked navigation to \(scheme): URI")
                return "about:blank"
            }
        }

        if trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://") {
            return trimmed
        }
        if trimmed.hasPrefix("about:") {
            return trimmed
        }

        // Looks like a domain (has dot, no spaces).
        if trimmed.contains(".") && !trimmed.contains(" ") {
            return "https://\(trimmed)"
        }

        // Treat as search query.
        let encoded = trimmed.replacingOccurrences(of: " ", with: "+")
        return "https://duckduckgo.com/?q=\(encoded)"
    }
}

// MARK: - VimiumMode

private enum VimiumMode {
    case normal
    case hints
}

// MARK: - BrowserViewDelegate

@MainActor
protocol BrowserViewDelegate: AnyObject {
    func browserView(_ view: BrowserView, titleDidChange title: String)
    func browserView(_ view: BrowserView, urlDidChange url: String)
    func browserView(_ view: BrowserView, loadingStateChanged isLoading: Bool)
}

// MARK: - WKNavigationDelegate

extension BrowserView: WKNavigationDelegate {
    /// Block navigation to dangerous URI schemes (file://, javascript:, data:, blob:).
    nonisolated func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        if let scheme = navigationAction.request.url?.scheme?.lowercased(),
           BrowserView.blockedSchemes.contains(scheme) {
            NSLog("thane: blocked navigation to \(scheme): URI at WebKit level")
            decisionHandler(.cancel)
            return
        }
        decisionHandler(.allow)
    }

    nonisolated func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.delegate?.browserView(self, loadingStateChanged: true)
        }
    }

    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.delegate?.browserView(self, loadingStateChanged: false)
        }
    }

    nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.delegate?.browserView(self, loadingStateChanged: false)
            self.showErrorPage(error: error, url: webView.url)
        }
    }

    nonisolated func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.delegate?.browserView(self, loadingStateChanged: false)
            self.showErrorPage(error: error, url: webView.url)
        }
    }

    private func showErrorPage(error: Error, url: URL?) {
        let nsError = error as NSError
        // Don't show error page for cancelled navigations (user clicked a link while loading)
        guard nsError.code != NSURLErrorCancelled else { return }

        let urlStr = url?.absoluteString ?? "unknown"
        let code = nsError.code
        let desc = nsError.localizedDescription
        let html = """
        <html>
        <head><meta charset="utf-8"><style>
        body { background: #0c0c0e; color: #e4e4e7; font-family: -apple-system, sans-serif;
               display: flex; flex-direction: column; align-items: center; justify-content: center;
               height: 90vh; margin: 0; padding: 20px; }
        h1 { color: #f87171; font-size: 20px; margin-bottom: 8px; }
        p { color: #a1a1aa; font-size: 14px; margin: 4px 0; text-align: center; max-width: 400px; }
        .url { color: #818cf8; word-break: break-all; font-size: 13px; }
        .code { color: #71717a; font-size: 12px; }
        button { margin-top: 16px; padding: 8px 20px; background: #818cf8; color: white;
                 border: none; border-radius: 6px; font-size: 14px; cursor: pointer; }
        button:hover { background: #6366f1; }
        </style></head>
        <body>
        <h1>Unable to Connect</h1>
        <p>\(desc)</p>
        <p class="url">\(urlStr)</p>
        <p class="code">Error \(code)</p>
        <button onclick="window.location.reload()">Retry</button>
        </body></html>
        """
        webView.loadHTMLString(html, baseURL: url)
    }
}

// MARK: - WKUIDelegate (handle target="_blank")

extension BrowserView: WKUIDelegate {
    /// Redirect `target="_blank"` links to the same web view instead of opening a new window.
    nonisolated func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if navigationAction.targetFrame == nil {
            webView.load(navigationAction.request)
        }
        return nil
    }
}

// MARK: - ThaneWebView (custom context menu)

/// WKWebView subclass that appends thane-specific items to the default context menu.
@MainActor
final class ThaneWebView: WKWebView {
    weak var parentBrowserView: BrowserView?

    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        menu.addItem(NSMenuItem.separator())

        let copyURL = NSMenuItem(title: "Copy URL", action: #selector(copyCurrentURL), keyEquivalent: "")
        copyURL.target = self
        menu.addItem(copyURL)

        let openExternal = NSMenuItem(title: "Open in Default Browser", action: #selector(openInDefaultBrowser), keyEquivalent: "")
        openExternal.target = self
        menu.addItem(openExternal)

        let screenshot = NSMenuItem(title: "Take Screenshot", action: #selector(takeScreenshotFromMenu), keyEquivalent: "")
        screenshot.target = self
        menu.addItem(screenshot)

        super.willOpenMenu(menu, with: event)
    }

    @objc private func copyCurrentURL() {
        guard let url = url?.absoluteString else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(url, forType: .string)
    }

    @objc private func openInDefaultBrowser() {
        guard let url else { return }
        NSWorkspace.shared.open(url)
    }

    @objc private func takeScreenshotFromMenu() {
        parentBrowserView?.takeScreenshot { [weak self] image in
            guard let image else {
                NSLog("thane: browser context menu screenshot returned nil")
                return
            }
            DispatchQueue.main.async {
                guard let tiffData = image.tiffRepresentation,
                      let bitmap = NSBitmapImageRep(data: tiffData),
                      let pngData = bitmap.representation(using: .png, properties: [:]) else { return }
                let path = "/tmp/thane-screenshot-\(Int(Date().timeIntervalSince1970)).png"
                try? pngData.write(to: URL(fileURLWithPath: path))
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(path, forType: .string)
                NSLog("thane: browser screenshot saved to \(path)")

                let alert = NSAlert()
                alert.messageText = "Screenshot Captured"
                alert.informativeText = "Screenshot saved and path copied to clipboard.\n\(path)"
                alert.alertStyle = .informational
                alert.addButton(withTitle: "OK")
                if let window = self?.window ?? NSApp.keyWindow {
                    alert.beginSheetModal(for: window)
                } else {
                    alert.runModal()
                }
            }
        }
    }
}
