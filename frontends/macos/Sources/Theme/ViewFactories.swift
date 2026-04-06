import AppKit

/// Shared view factory methods to eliminate duplicated view-building boilerplate
/// across panel files. All methods are @MainActor since they create NSView subclasses.
@MainActor
enum ViewFactories {

    // MARK: - Divider

    /// Create a 1px horizontal divider line using ThaneTheme.dividerColor.
    static func makeDivider() -> NSView {
        let v = NSView()
        v.wantsLayer = true
        v.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        v.translatesAutoresizingMaskIntoConstraints = false
        v.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return v
    }

    // MARK: - Empty State

    /// Create a centered empty-state view with an SF Symbol icon, title, and hint text.
    /// Used by panels that show a placeholder when their data list is empty.
    static func makeEmptyState(
        icon symbolName: String,
        title: String,
        hint: String,
        height: CGFloat = 140
    ) -> NSView {
        let box = NSView()
        box.translatesAutoresizingMaskIntoConstraints = false
        box.heightAnchor.constraint(equalToConstant: height).isActive = true

        let iconView = NSImageView()
        iconView.image = NSImage(systemSymbolName: symbolName, accessibilityDescription: nil)
        iconView.contentTintColor = ThaneTheme.tertiaryText.withAlphaComponent(0.3)
        iconView.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 32, weight: .regular)
        iconView.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(iconView)

        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = ThaneTheme.uiFont(size: 13)
        titleLabel.textColor = ThaneTheme.tertiaryText
        titleLabel.alignment = .center
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(titleLabel)

        let hintLabel = NSTextField(wrappingLabelWithString: hint)
        hintLabel.font = ThaneTheme.uiFont(size: 12)
        hintLabel.textColor = ThaneTheme.tertiaryText
        hintLabel.alignment = .center
        hintLabel.maximumNumberOfLines = 3
        hintLabel.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(hintLabel)

        NSLayoutConstraint.activate([
            iconView.topAnchor.constraint(equalTo: box.topAnchor, constant: 30),
            iconView.centerXAnchor.constraint(equalTo: box.centerXAnchor),
            titleLabel.topAnchor.constraint(equalTo: iconView.bottomAnchor, constant: 8),
            titleLabel.centerXAnchor.constraint(equalTo: box.centerXAnchor),
            hintLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 4),
            hintLabel.centerXAnchor.constraint(equalTo: box.centerXAnchor),
            hintLabel.leadingAnchor.constraint(greaterThanOrEqualTo: box.leadingAnchor, constant: 20),
        ])

        return box
    }

    // MARK: - Badge

    /// Create a small status badge label with rounded corners.
    /// If `backgroundColor` is provided, fills the badge; otherwise text-only with optional tint background.
    static func makeBadge(
        text: String,
        color: NSColor,
        fontSize: CGFloat = 11,
        filled: Bool = false,
        tintBackground: Bool = false
    ) -> NSTextField {
        let badge = NSTextField(labelWithString: " \(text) ")
        badge.font = NSFont.boldSystemFont(ofSize: fontSize)
        badge.textColor = filled ? .white : color
        badge.wantsLayer = true
        badge.layer?.cornerRadius = 4
        if filled {
            badge.layer?.backgroundColor = color.cgColor
        } else if tintBackground {
            badge.layer?.backgroundColor = color.withAlphaComponent(0.12).cgColor
        }
        badge.alignment = .center
        badge.translatesAutoresizingMaskIntoConstraints = false
        badge.setContentHuggingPriority(.required, for: .horizontal)
        badge.setContentCompressionResistancePriority(.required, for: .horizontal)
        return badge
    }

    // MARK: - Read-Only Text View

    /// Create a non-editable, selectable NSTextView suitable for displaying code or log output.
    static func makeReadOnlyTextView(
        font: NSFont? = nil,
        backgroundColor: NSColor? = nil,
        inset: NSSize = NSSize(width: 12, height: 12)
    ) -> NSTextView {
        let textView = NSTextView()
        textView.isEditable = false
        textView.isSelectable = true
        textView.backgroundColor = backgroundColor ?? ThaneTheme.backgroundColor
        textView.textContainerInset = inset
        textView.font = font ?? ThaneTheme.terminalFont(size: 11)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        return textView
    }

    // MARK: - Small Button

    /// Create a small recessed button matching the standard panel button style.
    static func makeSmallButton(
        title: String,
        target: AnyObject?,
        action: Selector
    ) -> NSButton {
        let btn = NSButton(title: title, target: target, action: action)
        btn.bezelStyle = .recessed
        btn.controlSize = .small
        btn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        btn.translatesAutoresizingMaskIntoConstraints = false
        return btn
    }

    // MARK: - Panel Header

    /// Create a standard panel header row with a title and optional action buttons.
    static func makePanelHeader(
        title: String,
        buttons: [(String, AnyObject?, Selector)] = []
    ) -> NSView {
        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false

        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(titleLabel)

        let buttonStack = NSStackView()
        buttonStack.orientation = .horizontal
        buttonStack.spacing = 8
        buttonStack.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(buttonStack)

        for (label, target, action) in buttons {
            let btn = makeSmallButton(title: label, target: target, action: action)
            buttonStack.addArrangedSubview(btn)
        }

        NSLayoutConstraint.activate([
            titleLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            buttonStack.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            buttonStack.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -8),
        ])

        return header
    }

    // MARK: - Section Header

    /// Create a section header with a divider line and uppercased label, used in settings-style panels.
    static func makeSectionHeader(_ title: String) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(divider)

        let label = NSTextField(labelWithString: title.uppercased())
        label.font = ThaneTheme.boldLabelFont(size: 10)
        label.textColor = ThaneTheme.tertiaryText
        label.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(label)

        NSLayoutConstraint.activate([
            container.heightAnchor.constraint(equalToConstant: 32),
            divider.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
            divider.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1),
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            label.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        return container
    }

    // MARK: - Configured Table View

    /// Configure an NSTableView with standard panel styling (no header, clear background, custom row height).
    static func configureTableView(_ tableView: NSTableView, rowHeight: CGFloat, columnId: String) {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(columnId))
        column.title = ""
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = rowHeight
        tableView.backgroundColor = .clear
        tableView.selectionHighlightStyle = .none
    }

    // MARK: - Configured Scroll View

    /// Create a standard borderless, transparent scroll view for panel content.
    static func makeScrollView() -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        return scrollView
    }

    // MARK: - Flipped View

    /// An NSView with flipped coordinates (origin at top-left) for use as scroll view document views.
    /// Ensures content renders top-down instead of bottom-up in stack-based scroll layouts.
    static func makeFlippedView() -> NSView {
        return FlippedDocumentView()
    }
}

/// NSView subclass with flipped coordinates for top-down stack views in scroll views.
/// Shared across all panels — replaces per-file private FlippedView classes.
final class FlippedDocumentView: NSView {
    override var isFlipped: Bool { true }
}
