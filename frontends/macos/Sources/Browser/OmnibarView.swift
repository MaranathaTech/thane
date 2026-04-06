import AppKit

/// Browser address bar with Back / Forward / Reload buttons and a URL text field.
///
/// Layout:
/// ┌──────────────────────────────────────────────────────────┐
/// │ [<] [>] [↻]  │  https://example.com                     │
/// └──────────────────────────────────────────────────────────┘
@MainActor
final class OmnibarView: NSView, NSTextFieldDelegate {

    private let urlField: NSTextField
    private let backButton: NSButton
    private let forwardButton: NSButton
    private let reloadButton: NSButton

    /// Called when the user presses Enter in the URL field.
    var onNavigate: ((String) -> Void)?

    /// The view to focus after navigation (set to the BrowserView's webView).
    weak var focusTargetAfterNavigate: NSView?

    /// Called when Back button is clicked.
    var onBack: (() -> Void)?

    /// Called when Forward button is clicked.
    var onForward: (() -> Void)?

    /// Called when Reload button is clicked.
    var onReload: (() -> Void)?

    // MARK: - Init

    override init(frame: NSRect) {
        backButton = NSButton()
        forwardButton = NSButton()
        reloadButton = NSButton()
        urlField = NSTextField()

        super.init(frame: frame)
        setupViews()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public API

    /// Update the displayed URL (e.g. after navigation).
    func setURL(_ url: String) {
        urlField.stringValue = url
    }

    /// Get the current text in the URL field.
    func getURL() -> String {
        urlField.stringValue
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.tabBarBackground.cgColor

        let stack = NSStackView()
        stack.orientation = .horizontal
        stack.spacing = 4
        stack.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        stack.translatesAutoresizingMaskIntoConstraints = false

        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Back button
        configureNavButton(backButton, symbol: "chevron.left", tooltip: "Back (H)")
        backButton.target = self
        backButton.action = #selector(backClicked)
        stack.addArrangedSubview(backButton)

        // Forward button
        configureNavButton(forwardButton, symbol: "chevron.right", tooltip: "Forward (L)")
        forwardButton.target = self
        forwardButton.action = #selector(forwardClicked)
        stack.addArrangedSubview(forwardButton)

        // Reload button
        configureNavButton(reloadButton, symbol: "arrow.clockwise", tooltip: "Reload (r)")
        reloadButton.target = self
        reloadButton.action = #selector(reloadClicked)
        stack.addArrangedSubview(reloadButton)

        // URL field
        urlField.placeholderString = "Enter URL or search..."
        urlField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        urlField.textColor = ThaneTheme.primaryText
        urlField.backgroundColor = ThaneTheme.backgroundColor
        urlField.isBordered = true
        urlField.bezelStyle = .roundedBezel
        urlField.delegate = self
        urlField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        urlField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        stack.addArrangedSubview(urlField)

        // Bottom divider
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        addSubview(divider)
        NSLayoutConstraint.activate([
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.bottomAnchor.constraint(equalTo: bottomAnchor),
            divider.heightAnchor.constraint(equalToConstant: ThaneTheme.dividerThickness),
        ])
    }

    private func configureNavButton(_ button: NSButton, symbol: String, tooltip: String) {
        button.bezelStyle = .recessed
        button.isBordered = false
        button.toolTip = tooltip
        if let image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip) {
            button.image = image
            button.imagePosition = .imageOnly
        }
        button.contentTintColor = ThaneTheme.secondaryText
        button.widthAnchor.constraint(equalToConstant: 24).isActive = true
        button.heightAnchor.constraint(equalToConstant: 24).isActive = true
    }

    // MARK: - Actions

    @objc private func backClicked() { onBack?() }
    @objc private func forwardClicked() { onForward?() }
    @objc private func reloadClicked() { onReload?() }

    // MARK: - NSTextFieldDelegate

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        if commandSelector == #selector(NSResponder.insertNewline(_:)) {
            let text = urlField.stringValue
            onNavigate?(text)
            // Focus the browser view so Vimium keys work immediately.
            if let target = focusTargetAfterNavigate {
                window?.makeFirstResponder(target)
            } else {
                window?.makeFirstResponder(nil)
            }
            return true
        }
        return false
    }
}
