import AppKit

/// The main thane window: sidebar | content (workspace splits) | right panel.
///
/// Layout (horizontal NSSplitView):
/// ┌──────────┬─────────────────────────┬──────────┐
/// │ Sidebar  │   Workspace Content     │  Right   │
/// │          │   (SplitContainer)      │  Panel   │
/// │          │                         │ (opt.)   │
/// ├──────────┴─────────────────────────┴──────────┤
/// │                  Status Bar                    │
/// └───────────────────────────────────────────────┘
@MainActor
final class MainWindow: NSWindow {

    // MARK: - Subviews

    let sidebarView: SidebarView
    let workspaceView: WorkspaceView
    let statusBarView: StatusBarView

    private let mainSplitView = NSSplitView()
    private let contentBox = NSView()
    private var rightPanelView: NSView?
    private var rightPanelType: RightPanelType?

    /// Cached panel content views — preserves scroll position, filter state, and form inputs
    /// across close/reopen cycles.
    private var cachedPanels: [RightPanelType: NSView] = [:]

    private let bridge: RustBridge
    // Port scanning is driven by AppDelegate's periodic timer — no local timer needed.

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        self.sidebarView = SidebarView(bridge: bridge)
        self.workspaceView = WorkspaceView(bridge: bridge)
        self.statusBarView = StatusBarView(bridge: bridge)

        let screenFrame = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800)
        let windowWidth = min(screenFrame.width * 0.85, 1600)
        let windowHeight = min(screenFrame.height * 0.85, 1000)
        let windowRect = NSRect(
            x: screenFrame.midX - windowWidth / 2,
            y: screenFrame.midY - windowHeight / 2,
            width: windowWidth,
            height: windowHeight
        )

        super.init(
            contentRect: windowRect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )

        self.title = "thane"
        self.minSize = NSSize(width: 800, height: 500)
        self.isReleasedWhenClosed = false
        self.backgroundColor = ThaneTheme.backgroundColor
        self.toolbarStyle = .unifiedCompact
        self.titleVisibility = .visible

        setupToolbar()
        setupLayout()
    }

    // MARK: - Toolbar (Header Bar)

    private static let settingsToolbarId = NSToolbarItem.Identifier("settings")
    private static let contactToolbarId = NSToolbarItem.Identifier("contact")
    private static let helpToolbarId = NSToolbarItem.Identifier("help")

    private func setupToolbar() {
        let toolbar = NSToolbar(identifier: "thane-toolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        self.toolbar = toolbar
    }

    // MARK: - Layout

    private func setupLayout() {
        guard let contentView = self.contentView else { return }

        // Root vertical stack: main split + status bar
        let rootStack = NSStackView()
        rootStack.orientation = .vertical
        rootStack.spacing = 0
        rootStack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(rootStack)

        NSLayoutConstraint.activate([
            rootStack.topAnchor.constraint(equalTo: contentView.topAnchor),
            rootStack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            rootStack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            rootStack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])

        // Main horizontal split: sidebar | content
        mainSplitView.isVertical = true // left-right split
        mainSplitView.dividerStyle = .thin
        mainSplitView.autosaveName = "thaneSidebar"
        mainSplitView.translatesAutoresizingMaskIntoConstraints = false

        // Sidebar
        sidebarView.translatesAutoresizingMaskIntoConstraints = false
        sidebarView.widthAnchor.constraint(greaterThanOrEqualToConstant: ThaneTheme.sidebarCollapsedWidth).isActive = true
        let sidebarWidth = sidebarView.widthAnchor.constraint(equalToConstant: ThaneTheme.sidebarWidth)
        sidebarWidth.priority = .defaultHigh
        sidebarWidth.isActive = true

        mainSplitView.delegate = self

        // Content area (holds workspace view, stretches)
        contentBox.translatesAutoresizingMaskIntoConstraints = false
        workspaceView.translatesAutoresizingMaskIntoConstraints = false
        contentBox.addSubview(workspaceView)
        NSLayoutConstraint.activate([
            workspaceView.topAnchor.constraint(equalTo: contentBox.topAnchor),
            workspaceView.leadingAnchor.constraint(equalTo: contentBox.leadingAnchor),
            workspaceView.trailingAnchor.constraint(equalTo: contentBox.trailingAnchor),
            workspaceView.bottomAnchor.constraint(equalTo: contentBox.bottomAnchor),
        ])

        mainSplitView.addArrangedSubview(sidebarView)
        mainSplitView.addArrangedSubview(contentBox)

        mainSplitView.setHoldingPriority(.defaultHigh, forSubviewAt: 0)   // sidebar holds its width
        mainSplitView.setHoldingPriority(.defaultLow, forSubviewAt: 1)    // content stretches

        rootStack.addArrangedSubview(mainSplitView)

        // Status bar at bottom
        statusBarView.translatesAutoresizingMaskIntoConstraints = false
        statusBarView.heightAnchor.constraint(equalToConstant: ThaneTheme.statusBarHeight).isActive = true
        rootStack.addArrangedSubview(statusBarView)

        // Wire up workspace git diff callback
        workspaceView.onGitDiff = { [weak self] in
            self?.showRightPanel(.gitDiff)
        }

        // Wire up sidebar callbacks
        sidebarView.onToggleSidebar = { [weak self] in
            self?.toggleSidebar()
        }
        sidebarView.onShowPanel = { [weak self] type in
            self?.showRightPanel(type)
        }

        // Wire up port click callback
        sidebarView.onPortClick = { [weak self] port, shiftHeld in
            guard let self else { return }
            if shiftHeld {
                if let url = URL(string: "http://localhost:\(port)") {
                    NSWorkspace.shared.open(url)
                }
            } else {
                _ = try? self.bridge.splitBrowser(url: "http://localhost:\(port)", orientation: .horizontal)
            }
        }

        // Port scanning is driven by AppDelegate's periodic timer.

        // Wire up status bar callbacks
        statusBarView.onShowPanel = { [weak self] type in
            self?.showRightPanel(type)
        }

        // Set initial sidebar divider position after layout
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.mainSplitView.setPosition(ThaneTheme.sidebarWidth, ofDividerAt: 0)
            NSLog("thane: statusBar frame=\(self.statusBarView.frame), hidden=\(self.statusBarView.isHidden)")
            NSLog("thane: mainSplit frame=\(self.mainSplitView.frame)")
        }
    }

    // MARK: - Right panel management

    /// Panels that require Claude Code to be installed.
    private static let claudeDependentPanels: Set<RightPanelType> = [
        .agentQueue, .plans
    ]

    func showRightPanel(_ type: RightPanelType) {
        // Gate Claude-dependent panels behind an install check
        if Self.claudeDependentPanels.contains(type) {
            if let appDelegate = NSApp.delegate as? AppDelegate, !appDelegate.requireClaude() {
                return
            }
        }

        // Toggle off if already showing this panel
        if rightPanelType == type {
            hideRightPanel()
            return
        }

        hideRightPanel()

        let panelContent = createRightPanelView(for: type)
        panelContent.translatesAutoresizingMaskIntoConstraints = false

        // Wrapper: close header + panel content
        let panel = NSView()
        panel.translatesAutoresizingMaskIntoConstraints = false
        panel.wantsLayer = true
        panel.layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        // Left border divider
        let border = NSView()
        border.wantsLayer = true
        border.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        border.translatesAutoresizingMaskIntoConstraints = false
        panel.addSubview(border)

        // Close header row
        let closeBtn = NSButton(image: NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close panel")!,
                                target: self, action: #selector(closeRightPanelClicked))
        closeBtn.bezelStyle = .recessed
        closeBtn.isBordered = false
        closeBtn.contentTintColor = ThaneTheme.tertiaryText
        closeBtn.toolTip = "Close panel"
        closeBtn.translatesAutoresizingMaskIntoConstraints = false
        panel.addSubview(closeBtn)

        // Panel content below close button
        panel.addSubview(panelContent)

        NSLayoutConstraint.activate([
            border.topAnchor.constraint(equalTo: panel.topAnchor),
            border.leadingAnchor.constraint(equalTo: panel.leadingAnchor),
            border.bottomAnchor.constraint(equalTo: panel.bottomAnchor),
            border.widthAnchor.constraint(equalToConstant: 1),

            closeBtn.topAnchor.constraint(equalTo: panel.topAnchor, constant: 4),
            closeBtn.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -4),
            closeBtn.widthAnchor.constraint(equalToConstant: 20),
            closeBtn.heightAnchor.constraint(equalToConstant: 20),

            panelContent.topAnchor.constraint(equalTo: closeBtn.bottomAnchor, constant: 2),
            panelContent.leadingAnchor.constraint(equalTo: panel.leadingAnchor),
            panelContent.trailingAnchor.constraint(equalTo: panel.trailingAnchor),
            panelContent.bottomAnchor.constraint(equalTo: panel.bottomAnchor),
        ])

        // Set a preferred width
        let widthConstraint = panel.widthAnchor.constraint(equalToConstant: ThaneTheme.rightPanelWidth)
        widthConstraint.priority = .defaultHigh
        widthConstraint.isActive = true
        panel.widthAnchor.constraint(greaterThanOrEqualToConstant: 200).isActive = true

        // Temporarily lower content box holding priority so the split view
        // gives space to the new right panel.
        let contentIndex = mainSplitView.arrangedSubviews.firstIndex(of: contentBox) ?? 1
        mainSplitView.setHoldingPriority(.defaultLow - 1, forSubviewAt: contentIndex)

        mainSplitView.addArrangedSubview(panel)
        let panelIndex = mainSplitView.arrangedSubviews.count - 1
        mainSplitView.setHoldingPriority(.defaultHigh + 1, forSubviewAt: panelIndex)

        // Force layout: set divider position so the right panel gets its width
        mainSplitView.layoutSubtreeIfNeeded()
        let dividerIndex = mainSplitView.arrangedSubviews.count - 2
        if dividerIndex >= 0 {
            let pos = mainSplitView.frame.width - ThaneTheme.rightPanelWidth
            mainSplitView.setPosition(pos, ofDividerAt: dividerIndex)
        }

        rightPanelView = panel
        rightPanelType = type
    }

    @objc private func closeRightPanelClicked() {
        hideRightPanel()
    }

    func hideRightPanel() {
        if let panel = rightPanelView {
            mainSplitView.removeArrangedSubview(panel)
            panel.removeFromSuperview()
            rightPanelView = nil
            rightPanelType = nil
            activePanel = nil
        }
    }

    /// Currently active panel instance (for refresh on callbacks).
    private var activePanel: NSView?

    private func createRightPanelView(for type: RightPanelType) -> NSView {
        // Reuse cached panel to preserve scroll position, filter state, and form inputs
        if let cached = cachedPanels[type] {
            activePanel = cached
            // Refresh data when reopening
            refreshPanel(cached)
            return cached
        }

        let panel: NSView
        switch type {
        case .notifications:
            panel = NotificationPanel(bridge: bridge)
        case .audit:
            panel = AuditPanel(bridge: bridge)
        case .settings:
            panel = SettingsPanel(bridge: bridge)
        case .tokenUsage:
            panel = TokenPanel(bridge: bridge)
        case .help:
            panel = HelpPanel(bridge: bridge)
        case .agentQueue:
            panel = AgentQueuePanel(bridge: bridge)
        case .sandbox:
            panel = SandboxPanel(bridge: bridge)
        case .gitDiff:
            panel = GitDiffPanel(bridge: bridge)
        case .plans:
            panel = PlansPanel(bridge: bridge)
        }
        cachedPanels[type] = panel
        activePanel = panel
        return panel
    }

    private func refreshPanel(_ panel: NSView) {
        (panel as? ReloadablePanel)?.reload()
    }

    /// Refresh the currently visible right panel's data.
    func refreshActivePanel() {
        guard let panel = activePanel else { return }
        (panel as? ReloadablePanel)?.reload()
    }

    // MARK: - UI Font Size

    /// Apply the UI font size from config to all UI components.
    func applyUIFontSize() {
        let size = CGFloat(bridge.configGet(key: "ui-font-size").flatMap { Double($0) } ?? Double(ThaneTheme.uiFontSize))
        applyFontRecursively(to: sidebarView, size: size)
        applyFontRecursively(to: statusBarView, size: size)
        if let panel = rightPanelView {
            applyFontRecursively(to: panel, size: size)
        }
    }

    private func applyFontRecursively(to view: NSView, size: CGFloat) {
        if let textField = view as? NSTextField {
            if let currentFont = textField.font {
                textField.font = NSFont(descriptor: currentFont.fontDescriptor, size: size)
                    ?? currentFont.withSize(size)
            }
        } else if let button = view as? NSButton {
            if let currentFont = button.font {
                button.font = NSFont(descriptor: currentFont.fontDescriptor, size: size)
                    ?? currentFont.withSize(size)
            }
        }
        for subview in view.subviews {
            applyFontRecursively(to: subview, size: size)
        }
    }

    // MARK: - Sidebar toggle

    private var sidebarCollapsed = false

    func toggleSidebar() {
        sidebarCollapsed.toggle()
        let targetWidth = sidebarCollapsed
            ? ThaneTheme.sidebarCollapsedWidth
            : ThaneTheme.sidebarWidth

        NSAnimationContext.runAnimationGroup { context in
            context.duration = ThaneTheme.animationDuration
            sidebarView.animator().frame.size.width = targetWidth
        }

        sidebarView.setCollapsed(sidebarCollapsed)
    }

    // MARK: - Browser access

    /// Get the focused BrowserView from the active workspace, if any.
    private func focusedBrowserView() -> BrowserView? {
        workspaceView.focusedBrowserView()
    }

    // MARK: - Key handling (leader key: Cmd+B)

    private var leaderActive = false

    override func keyDown(with event: NSEvent) {
        // Leader key: Cmd+B
        if event.modifierFlags.contains(.command) && event.charactersIgnoringModifiers == "b"
            && !event.modifierFlags.contains(.shift) {
            leaderActive = true
            statusBarView.showLeaderIndicator()
            return
        }

        if leaderActive {
            leaderActive = false
            statusBarView.hideLeaderIndicator()
            handleLeaderKey(event)
            return
        }

        // Route keys to focused BrowserView for Vimium navigation.
        // Only when no modifier keys are held (except Shift for G, H, L).
        if let browser = focusedBrowserView(),
           !event.modifierFlags.contains(.command),
           !event.modifierFlags.contains(.control),
           !event.modifierFlags.contains(.option) {
            let key = event.modifierFlags.contains(.shift)
                ? (event.charactersIgnoringModifiers ?? "").uppercased()
                : (event.charactersIgnoringModifiers ?? "")
            if browser.handleVimiumKey(key) {
                return
            }
        }

        // Escape: close right panel if one is open
        if event.keyCode == 53 && rightPanelView != nil {
            hideRightPanel()
            return
        }

        // Opt+H/J/K/L for directional focus
        if event.modifierFlags.contains(.option) {
            switch event.charactersIgnoringModifiers {
            case "h": try? bridge.focusDirection("left"); return
            case "j": try? bridge.focusDirection("down"); return
            case "k": try? bridge.focusDirection("up"); return
            case "l": try? bridge.focusDirection("right"); return
            default: break
            }
        }

        super.keyDown(with: event)
    }

    private func handleLeaderKey(_ event: NSEvent) {
        guard let chars = event.charactersIgnoringModifiers else { return }

        switch chars {
        case "c":
            // New workspace
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            _ = try? bridge.createWorkspace(title: "workspace", cwd: home)
        case "n":
            // Next workspace
            let workspaces = bridge.listWorkspaces()
            if let active = bridge.activeWorkspace(),
               let idx = workspaces.firstIndex(where: { $0.id == active.id }),
               idx + 1 < workspaces.count {
                _ = try? bridge.selectWorkspace(id: workspaces[idx + 1].id)
            }
        case "p":
            // Previous workspace
            let workspaces = bridge.listWorkspaces()
            if let active = bridge.activeWorkspace(),
               let idx = workspaces.firstIndex(where: { $0.id == active.id }),
               idx > 0 {
                _ = try? bridge.selectWorkspace(id: workspaces[idx - 1].id)
            }
        case "x":
            // Close current pane (with confirmation)
            if confirmClosePane() {
                try? bridge.closePane()
            }
        case ",":
            // Rename workspace
            showRenameWorkspaceDialog()
        case "1", "2", "3", "4", "5", "6", "7", "8", "9":
            let idx = Int(chars)! - 1
            let workspaces = bridge.listWorkspaces()
            if idx < workspaces.count {
                _ = try? bridge.selectWorkspace(id: workspaces[idx].id)
            }
        default:
            break
        }
    }
}

// MARK: - NSSplitViewDelegate

extension MainWindow: NSSplitViewDelegate {
    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        if dividerIndex == 0 {
            return ThaneTheme.sidebarCollapsedWidth
        }
        return proposedMinimumPosition
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        if dividerIndex == 0 {
            return 360 // max sidebar width
        }
        return proposedMaximumPosition
    }

    func splitView(_ splitView: NSSplitView, shouldAdjustSizeOfSubview view: NSView) -> Bool {
        // Only auto-resize the content area, not sidebar or right panel
        if view === sidebarView { return false }
        if view === rightPanelView { return false }
        return true
    }
}

// MARK: - NSToolbarDelegate

extension MainWindow: NSToolbarDelegate {
    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier, willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        switch itemIdentifier {
        case Self.settingsToolbarId:
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: "Settings")
            item.label = "Settings"
            item.toolTip = "Settings (Cmd+,)"
            item.target = self
            item.action = #selector(toolbarSettingsClicked)
            return item
        case Self.contactToolbarId:
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.image = NSImage(systemSymbolName: "envelope", accessibilityDescription: "Contact Us")
            item.label = "Contact"
            item.toolTip = "Request features or report bugs"
            item.target = self
            item.action = #selector(toolbarContactClicked)
            return item
        case Self.helpToolbarId:
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.image = NSImage(systemSymbolName: "questionmark.circle", accessibilityDescription: "Help")
            item.label = "Help"
            item.toolTip = "Help (F1)"
            item.target = self
            item.action = #selector(toolbarHelpClicked)
            return item
        default:
            return nil
        }
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.flexibleSpace, Self.settingsToolbarId, Self.contactToolbarId, Self.helpToolbarId]
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.flexibleSpace, .flexibleSpace, Self.settingsToolbarId, Self.contactToolbarId, Self.helpToolbarId]
    }

    @objc private func toolbarSettingsClicked() {
        showRightPanel(.settings)
    }

    @objc private func toolbarContactClicked() {
        if let url = URL(string: "https://getthane.com/contact") {
            NSWorkspace.shared.open(url)
        }
    }

    @objc private func toolbarHelpClicked() {
        showRightPanel(.help)
    }

    // MARK: - Rename workspace dialog

    func showRenameWorkspaceDialog() {
        guard let ws = bridge.activeWorkspace() else { return }

        let alert = NSAlert()
        alert.messageText = "Rename Workspace"
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
        input.stringValue = ws.title
        alert.accessoryView = input

        if alert.runModal() == .alertFirstButtonReturn {
            let newTitle = input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if !newTitle.isEmpty {
                _ = try? bridge.renameWorkspace(id: ws.id, title: newTitle)
            }
        }
    }

    // MARK: - Close confirmations

    /// Confirm before closing a panel. Returns true if user confirmed.
    func confirmClosePanel() -> Bool {
        confirmClose(message: "Close this panel?", detail: "The panel will be closed.")
    }

    /// Confirm before closing a pane. Returns true if user confirmed.
    func confirmClosePane() -> Bool {
        confirmClose(message: "Close this pane and its panels?", detail: "All panels in this pane will be closed.")
    }

    private func confirmClose(message: String, detail: String) -> Bool {
        let shouldConfirm = bridge.configGet(key: "confirm-close").map { $0 == "true" } ?? true
        guard shouldConfirm else { return true }

        let alert = NSAlert()
        alert.messageText = message
        alert.informativeText = detail
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Close")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }

    // MARK: - Port scanning

    func scanPorts() {
        let workspaces = bridge.listWorkspaces()
        for ws in workspaces {
            let pids = workspaceView.shellPidsForWorkspace(id: ws.id)
            guard !pids.isEmpty else {
                bridge.updatePorts(workspaceId: ws.id, ports: [])
                continue
            }
            let wsId = ws.id
            let capturedPids = pids
            DispatchQueue.global(qos: .utility).async { [weak self] in
                let ports = RustBridge.scanListeningPorts(pids: capturedPids)
                DispatchQueue.main.async {
                    self?.bridge.updatePorts(workspaceId: wsId, ports: ports)
                }
            }
        }
    }
}

// MARK: - Right panel types

enum RightPanelType {
    case notifications
    case audit
    case settings
    case tokenUsage
    case agentQueue
    case sandbox
    case help
    case gitDiff
    case plans

    var title: String {
        switch self {
        case .notifications: return "Notifications"
        case .audit: return "Audit Log"
        case .settings: return "Settings"
        case .tokenUsage: return "CC Token Usage"
        case .agentQueue: return "Agent Queue"
        case .sandbox: return "Sandbox"
        case .help: return "Help"
        case .gitDiff: return "Git Diff"
        case .plans: return "Processed"
        }
    }
}
