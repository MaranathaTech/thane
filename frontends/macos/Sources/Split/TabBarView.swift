import AppKit

/// Per-pane tab strip showing panel tabs + action buttons.
///
/// Layout:
/// ┌─────────────────────────────────────────────────────┐
/// │ [Screenshot] [GitDiff] │ Tab1 Tab2 … │ [⏐] [─] [×] │
/// └─────────────────────────────────────────────────────┘
///
/// - Left actions: Screenshot, Git Diff
/// - Center: Tab items (only shown when 2+ tabs)
/// - Right actions: Split Right, Split Down, Close Pane
///
/// Supports drag-and-drop: dragging a tab to another pane's tab bar
/// swaps the two panels.
@MainActor
final class TabBarView: NSView {

    /// Pasteboard type for tab drag operations.
    static let tabDragType = NSPasteboard.PasteboardType("com.thane.tabDrag")

    private let panels: [PanelInfoDTO]
    private let onSelect: (String) -> Void
    private let onClose: (String) -> Void
    private let onSplitRight: () -> Void
    private let onSplitDown: () -> Void
    private let onClosePane: () -> Void
    private let onScreenshot: () -> Void
    private let onGitDiff: () -> Void
    private let onFind: () -> Void

    /// Called when a panel is dropped here from another tab bar.
    /// Parameters: (droppedPanelId, targetPanelId)
    var onReorder: ((String, String) -> Void)?

    // MARK: - Init

    init(
        panels: [PanelInfoDTO],
        onSelect: @escaping (String) -> Void,
        onClose: @escaping (String) -> Void,
        onSplitRight: @escaping () -> Void,
        onSplitDown: @escaping () -> Void,
        onClosePane: @escaping () -> Void,
        onScreenshot: @escaping () -> Void = {},
        onGitDiff: @escaping () -> Void = {},
        onFind: @escaping () -> Void = {}
    ) {
        self.panels = panels
        self.onSelect = onSelect
        self.onClose = onClose
        self.onSplitRight = onSplitRight
        self.onSplitDown = onSplitDown
        self.onClosePane = onClosePane
        self.onScreenshot = onScreenshot
        self.onGitDiff = onGitDiff
        self.onFind = onFind
        super.init(frame: .zero)
        registerForDraggedTypes([TabBarView.tabDragType])
        setupViews()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.tabBarBackground.cgColor

        let stack = NSStackView()
        stack.orientation = .horizontal
        stack.spacing = 4
        stack.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 8)
        stack.translatesAutoresizingMaskIntoConstraints = false

        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Left actions
        let leftActions = NSStackView()
        leftActions.orientation = .horizontal
        leftActions.spacing = 2

        leftActions.addArrangedSubview(makeActionButton(
            symbol: "camera",
            tooltip: "Screenshot",
            action: { [weak self] in self?.onScreenshot() }
        ))
        leftActions.addArrangedSubview(makeActionButton(
            symbol: "arrow.triangle.branch",
            tooltip: "Git Diff (Cmd+Shift+G)",
            action: { [weak self] in self?.onGitDiff() }
        ))
        leftActions.addArrangedSubview(makeActionButton(
            symbol: "magnifyingglass",
            tooltip: "Find (Cmd+Shift+F)",
            action: { [weak self] in self?.onFind() }
        ))

        stack.addArrangedSubview(leftActions)

        // Flexible spacer
        let leftSpacer = NSView()
        leftSpacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        stack.addArrangedSubview(leftSpacer)

        // Tab items (only show when 2+ tabs)
        if panels.count >= 2 {
            let tabStack = NSStackView()
            tabStack.orientation = .horizontal
            tabStack.spacing = 2

            for panel in panels {
                let tab = makeTabItem(panel: panel)
                tabStack.addArrangedSubview(tab)
            }

            stack.addArrangedSubview(tabStack)
        }

        // Right spacer
        let rightSpacer = NSView()
        rightSpacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        stack.addArrangedSubview(rightSpacer)

        // Right actions
        let rightActions = NSStackView()
        rightActions.orientation = .horizontal
        rightActions.spacing = 2

        rightActions.addArrangedSubview(makeActionButton(
            symbol: "rectangle.split.2x1",
            tooltip: "Split Right (Cmd+Shift+D)",
            action: { [weak self] in self?.onSplitRight() }
        ))
        rightActions.addArrangedSubview(makeActionButton(
            symbol: "rectangle.split.1x2",
            tooltip: "Split Down (Cmd+Shift+E)",
            action: { [weak self] in self?.onSplitDown() }
        ))
        rightActions.addArrangedSubview(makeActionButton(
            symbol: "xmark",
            tooltip: "Close Pane (Cmd+Shift+X)",
            tint: ThaneTheme.errorColor,
            action: { [weak self] in self?.onClosePane() }
        ))

        stack.addArrangedSubview(rightActions)

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

    // MARK: - Helpers

    private func makeTabItem(panel: PanelInfoDTO) -> NSView {
        let button = DraggableTabButton(panelId: panel.id)
        button.bezelStyle = .recessed
        button.isBordered = false
        button.wantsLayer = true

        let icon: String
        switch panel.panelType {
        case .terminal: icon = "terminal"
        case .browser: icon = "globe"
        }

        if let image = NSImage(systemSymbolName: icon, accessibilityDescription: panel.title) {
            button.image = image
            button.imagePosition = .imageLeading
        }

        button.title = truncateTitle(panel.title, maxLength: 20)
        button.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        button.contentTintColor = ThaneTheme.primaryText

        let panelId = panel.id
        button.target = self
        button.action = #selector(tabClicked(_:))
        button.tag = panels.firstIndex(where: { $0.id == panelId }) ?? 0

        return button
    }

    @objc private func tabClicked(_ sender: NSButton) {
        let idx = sender.tag
        guard idx < panels.count else { return }
        onSelect(panels[idx].id)
    }

    private func makeActionButton(
        symbol: String,
        tooltip: String,
        tint: NSColor? = nil,
        action: @escaping () -> Void
    ) -> NSView {
        let button = ActionButton(action: action)
        button.bezelStyle = .recessed
        button.isBordered = false
        button.toolTip = tooltip

        if let image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip) {
            button.image = image
            button.imagePosition = .imageOnly
        }
        button.contentTintColor = tint ?? ThaneTheme.secondaryText

        button.widthAnchor.constraint(equalToConstant: 24).isActive = true
        button.heightAnchor.constraint(equalToConstant: 24).isActive = true

        return button
    }

    private func truncateTitle(_ title: String, maxLength: Int) -> String {
        if title.count <= maxLength { return title }
        return String(title.prefix(maxLength - 1)) + "\u{2026}"
    }
}

// MARK: - Drag & Drop

extension TabBarView {
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard sender.draggingPasteboard.data(forType: TabBarView.tabDragType) != nil else {
            return []
        }
        return .move
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard let data = sender.draggingPasteboard.data(forType: TabBarView.tabDragType),
              let droppedPanelId = String(data: data, encoding: .utf8),
              let targetPanelId = panels.first?.id,
              droppedPanelId != targetPanelId else {
            return false
        }
        onReorder?(droppedPanelId, targetPanelId)
        return true
    }
}

// MARK: - DraggableTabButton (supports initiating drag)

@MainActor
private final class DraggableTabButton: NSButton {
    let panelId: String

    init(panelId: String) {
        self.panelId = panelId
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func mouseDown(with event: NSEvent) {
        // Start drag after a small movement threshold
        let startPoint = convert(event.locationInWindow, from: nil)
        var movedEnough = false

        // Track mouse until drag threshold or mouseUp
        window?.trackEvents(matching: [.leftMouseDragged, .leftMouseUp], timeout: .infinity, mode: .eventTracking) { trackEvent, stop in
            guard let trackEvent else { stop.pointee = true; return }
            if trackEvent.type == .leftMouseUp {
                // No drag — treat as a normal click
                stop.pointee = true
                if !movedEnough {
                    self.sendAction(self.action, to: self.target)
                }
                return
            }
            let current = self.convert(trackEvent.locationInWindow, from: nil)
            let dx = abs(current.x - startPoint.x)
            let dy = abs(current.y - startPoint.y)
            if dx > 4 || dy > 4 {
                movedEnough = true
                stop.pointee = true
                // Start the drag session
                let pasteItem = NSPasteboardItem()
                pasteItem.setData(self.panelId.data(using: .utf8)!, forType: TabBarView.tabDragType)
                let dragItem = NSDraggingItem(pasteboardWriter: pasteItem)
                dragItem.setDraggingFrame(self.bounds, contents: self.snapshot())
                self.beginDraggingSession(with: [dragItem], event: event, source: self)
            }
        }
    }

    private func snapshot() -> NSImage {
        let image = NSImage(size: bounds.size)
        image.lockFocus()
        if let ctx = NSGraphicsContext.current?.cgContext {
            layer?.render(in: ctx)
        }
        image.unlockFocus()
        return image
    }
}

extension DraggableTabButton: NSDraggingSource {
    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        return .move
    }
}

// MARK: - ActionButton (closure-based NSButton)

@MainActor
private final class ActionButton: NSButton {
    private var clickAction: (() -> Void)?

    convenience init(action: @escaping () -> Void) {
        self.init()
        self.clickAction = action
        self.target = self
        self.action = #selector(handleClick)
    }

    @objc private func handleClick() {
        clickAction?()
    }
}
