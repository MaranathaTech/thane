import AppKit

/// Workspace list sidebar.
///
/// Expanded mode: scrollable list of WorkspaceRowViews with header buttons.
/// Collapsed mode: vertical stack of avatar circles.
@MainActor
final class SidebarView: NSView {

    private let bridge: RustBridge

    private let scrollView = NSScrollView()
    private let tableView = NSTableView()
    private var workspaces: [WorkspaceInfoDTO] = []
    /// Guard to suppress selection-change callbacks during programmatic row updates.
    private var isUpdatingSelection = false

    // Collapsed mode
    private let collapsedStack = NSStackView()
    private var isCollapsed = false

    // Header
    private let headerView = NSView()
    private let titleLabel = NSTextField(labelWithString: "Workspaces")

    // History section
    private let historyContainer = NSView()
    private let historyHeaderLabel = NSTextField(labelWithString: "Recently closed")
    private let historyClearButton = NSButton()
    private let historyScrollView = NSScrollView()
    private let historyStackView = NSStackView()
    private var historyContainerHeightConstraint: NSLayoutConstraint!

    // Callbacks for window-level actions
    var onToggleSidebar: (() -> Void)?
    var onShowPanel: ((RightPanelType) -> Void)?
    /// Called when a port badge is clicked (port, shiftHeld)
    var onPortClick: ((UInt16, Bool) -> Void)?

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(frame: .zero)
        setupViews()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public API

    func reloadWorkspaces() {
        let newWorkspaces = bridge.listWorkspaces()
        let oldIds = workspaces.map(\.id)
        let newIds = newWorkspaces.map(\.id)
        workspaces = newWorkspaces

        if isCollapsed {
            rebuildCollapsedView()
            reloadHistory()
        } else if oldIds != newIds {
            // Workspace list changed (added/removed/reordered) — full reload required
            isUpdatingSelection = true
            tableView.reloadData()
            if !workspaces.isEmpty {
                let allRows = IndexSet(0..<workspaces.count)
                tableView.noteHeightOfRows(withIndexesChanged: allRows)
            }
            // Re-sync selection to active workspace
            if let activeId = bridge.activeWorkspace()?.id,
               let activeRow = workspaces.firstIndex(where: { $0.id == activeId }) {
                tableView.selectRowIndexes(IndexSet(integer: activeRow), byExtendingSelection: false)
            }
            isUpdatingSelection = false
            // History only changes when workspaces are added/removed
            reloadHistory()
        } else {
            // Same workspace list — update existing rows in-place (no blink)
            for (idx, ws) in workspaces.enumerated() {
                guard let cellView = tableView.view(atColumn: 0, row: idx, makeIfNecessary: false) as? WorkspaceRowView else { continue }
                let panelLocations = bridge.panelLocations(for: ws.id)
                let isSandboxed = bridge.isSandboxed(workspaceId: ws.id)
                let ports = bridge.workspacePorts[ws.id] ?? []
                let wsCost = bridge.getProjectCostForCwd(ws.cwd)
                let costScope = bridge.configGet(key: "cost-display-scope") ?? "session"
                let displayCost = costScope == "all-time" ? wsCost.alltimeCostUsd : wsCost.sessionCostUsd
                bridge.updateWorkspaceCost(workspaceId: ws.id, cost: displayCost)
                let isActive = ws.id == bridge.activeWorkspace()?.id
                cellView.update(
                    workspace: ws, isActive: isActive,
                    panelLocations: panelLocations,
                    isSandboxed: isSandboxed, ports: ports,
                    cost: displayCost
                )
                // Update row view background
                if let rowView = tableView.rowView(atRow: idx, makeIfNecessary: false) {
                    rowView.wantsLayer = true
                    rowView.layer?.backgroundColor = isActive
                        ? ThaneTheme.tabSelectedBackground.cgColor
                        : nil
                }
            }
        }
    }

    /// Lightweight update when only the active workspace changed (no list mutation).
    /// Updates active-row backgrounds in-place without recreating any row views.
    func updateActiveWorkspace(id: String) {
        if isCollapsed {
            rebuildCollapsedView()
            return
        }

        // Update row view backgrounds directly (full-width, no reload)
        for (idx, ws) in workspaces.enumerated() {
            if let rowView = tableView.rowView(atRow: idx, makeIfNecessary: false) {
                rowView.wantsLayer = true
                rowView.layer?.backgroundColor = (ws.id == id)
                    ? ThaneTheme.tabSelectedBackground.cgColor
                    : nil
            }
        }

        // Sync table selection to new active row without triggering callback
        isUpdatingSelection = true
        if let newRow = workspaces.firstIndex(where: { $0.id == id }) {
            tableView.selectRowIndexes(IndexSet(integer: newRow), byExtendingSelection: false)
        }
        isUpdatingSelection = false
    }

    func setCollapsed(_ collapsed: Bool) {
        isCollapsed = collapsed
        scrollView.isHidden = collapsed
        headerView.isHidden = collapsed
        historyContainer.isHidden = collapsed
        collapsedStack.isHidden = !collapsed

        if collapsed {
            rebuildCollapsedView()
        } else {
            tableView.reloadData()
            reloadHistory()
        }
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        setupHeader()
        setupHistoryView()
        setupTableView()
        setupCollapsedView()

        collapsedStack.isHidden = true
    }

    private func setupHeader() {
        headerView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(headerView)

        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        headerView.addSubview(titleLabel)

        // Button row: [collapse] [folder] | spacer | [+]
        let buttonStack = NSStackView()
        buttonStack.orientation = .horizontal
        buttonStack.spacing = 4
        buttonStack.translatesAutoresizingMaskIntoConstraints = false
        headerView.addSubview(buttonStack)

        // Collapse sidebar button
        let collapseBtn = makeHeaderButton(
            symbol: "sidebar.left",
            tooltip: "Collapse sidebar (Cmd+Shift+B)",
            action: #selector(collapseClicked)
        )
        buttonStack.addArrangedSubview(collapseBtn)

        // Open folder button
        let folderBtn = makeHeaderButton(
            symbol: "folder",
            tooltip: "Open folder as workspace",
            action: #selector(openFolderClicked)
        )
        buttonStack.addArrangedSubview(folderBtn)


        // Add workspace button
        let addBtn = makeHeaderButton(
            symbol: "plus",
            tooltip: "New workspace (Cmd+Shift+T)",
            action: #selector(addWorkspaceClicked)
        )
        addBtn.translatesAutoresizingMaskIntoConstraints = false
        headerView.addSubview(addBtn)

        NSLayoutConstraint.activate([
            headerView.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            headerView.leadingAnchor.constraint(equalTo: leadingAnchor),
            headerView.trailingAnchor.constraint(equalTo: trailingAnchor),
            headerView.heightAnchor.constraint(equalToConstant: 44),

            titleLabel.centerYAnchor.constraint(equalTo: headerView.centerYAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: headerView.leadingAnchor, constant: 12),

            buttonStack.centerYAnchor.constraint(equalTo: headerView.centerYAnchor),
            buttonStack.trailingAnchor.constraint(equalTo: addBtn.leadingAnchor, constant: -4),

            addBtn.centerYAnchor.constraint(equalTo: headerView.centerYAnchor),
            addBtn.trailingAnchor.constraint(equalTo: headerView.trailingAnchor, constant: -8),
            addBtn.widthAnchor.constraint(equalToConstant: 24),
            addBtn.heightAnchor.constraint(equalToConstant: 24),
        ])
    }

    private func makeHeaderButton(symbol: String, tooltip: String, action: Selector) -> NSButton {
        let button = NSButton()
        button.bezelStyle = .recessed
        button.isBordered = false
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        button.contentTintColor = ThaneTheme.secondaryText
        button.toolTip = tooltip
        button.target = self
        button.action = action
        button.widthAnchor.constraint(equalToConstant: 24).isActive = true
        button.heightAnchor.constraint(equalToConstant: 24).isActive = true
        return button
    }

    private func setupTableView() {
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        addSubview(scrollView)

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("workspace"))
        column.title = ""
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.usesAutomaticRowHeights = true
        tableView.backgroundColor = .clear
        tableView.delegate = self
        tableView.dataSource = self
        tableView.selectionHighlightStyle = .none

        // Right-click context menu
        let menu = NSMenu()
        menu.delegate = self
        menu.addItem(NSMenuItem(title: "Rename", action: #selector(renameWorkspace), keyEquivalent: ""))
        menu.addItem(NSMenuItem(title: "Convert to Sandbox", action: #selector(toggleSandboxMenu), keyEquivalent: ""))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Close", action: #selector(closeWorkspaceMenu), keyEquivalent: ""))
        tableView.menu = menu

        scrollView.documentView = tableView

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: headerView.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: historyContainer.topAnchor),
        ])
    }

    private func setupCollapsedView() {
        collapsedStack.orientation = .vertical
        collapsedStack.spacing = 8
        collapsedStack.alignment = .centerX
        collapsedStack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(collapsedStack)

        NSLayoutConstraint.activate([
            collapsedStack.topAnchor.constraint(equalTo: topAnchor, constant: 36),
            collapsedStack.centerXAnchor.constraint(equalTo: centerXAnchor),
            collapsedStack.widthAnchor.constraint(equalToConstant: ThaneTheme.sidebarCollapsedWidth),
        ])
    }

    private func setupHistoryView() {
        historyContainer.translatesAutoresizingMaskIntoConstraints = false
        historyContainer.wantsLayer = true
        addSubview(historyContainer)

        // Header row: label + clear button
        let headerRow = NSView()
        headerRow.translatesAutoresizingMaskIntoConstraints = false
        historyContainer.addSubview(headerRow)

        historyHeaderLabel.font = ThaneTheme.labelFont(size: 11)
        historyHeaderLabel.textColor = ThaneTheme.secondaryText
        historyHeaderLabel.translatesAutoresizingMaskIntoConstraints = false
        headerRow.addSubview(historyHeaderLabel)

        historyClearButton.title = "Clear"
        historyClearButton.bezelStyle = .recessed
        historyClearButton.isBordered = false
        historyClearButton.font = ThaneTheme.labelFont(size: 11)
        historyClearButton.contentTintColor = ThaneTheme.secondaryText
        historyClearButton.target = self
        historyClearButton.action = #selector(clearHistoryClicked)
        historyClearButton.translatesAutoresizingMaskIntoConstraints = false
        headerRow.addSubview(historyClearButton)

        // Scrollable stack for history entries
        historyScrollView.translatesAutoresizingMaskIntoConstraints = false
        historyScrollView.hasVerticalScroller = true
        historyScrollView.borderType = .noBorder
        historyScrollView.backgroundColor = .clear
        historyScrollView.drawsBackground = false
        historyContainer.addSubview(historyScrollView)

        historyStackView.orientation = .vertical
        historyStackView.spacing = 2
        historyStackView.alignment = .leading
        historyStackView.translatesAutoresizingMaskIntoConstraints = false

        // Wrap stack in a flipped clip view for top-aligned scrolling
        let clipView = NSClipView()
        clipView.documentView = historyStackView
        clipView.drawsBackground = false
        historyScrollView.contentView = clipView

        historyContainerHeightConstraint = historyContainer.heightAnchor.constraint(equalToConstant: 0)

        NSLayoutConstraint.activate([
            historyContainer.leadingAnchor.constraint(equalTo: leadingAnchor),
            historyContainer.trailingAnchor.constraint(equalTo: trailingAnchor),
            historyContainer.bottomAnchor.constraint(equalTo: bottomAnchor),
            historyContainerHeightConstraint,

            headerRow.topAnchor.constraint(equalTo: historyContainer.topAnchor, constant: 4),
            headerRow.leadingAnchor.constraint(equalTo: historyContainer.leadingAnchor, constant: 12),
            headerRow.trailingAnchor.constraint(equalTo: historyContainer.trailingAnchor, constant: -8),
            headerRow.heightAnchor.constraint(equalToConstant: 20),

            historyHeaderLabel.leadingAnchor.constraint(equalTo: headerRow.leadingAnchor),
            historyHeaderLabel.centerYAnchor.constraint(equalTo: headerRow.centerYAnchor),

            historyClearButton.trailingAnchor.constraint(equalTo: headerRow.trailingAnchor),
            historyClearButton.centerYAnchor.constraint(equalTo: headerRow.centerYAnchor),

            historyScrollView.topAnchor.constraint(equalTo: headerRow.bottomAnchor, constant: 2),
            historyScrollView.leadingAnchor.constraint(equalTo: historyContainer.leadingAnchor),
            historyScrollView.trailingAnchor.constraint(equalTo: historyContainer.trailingAnchor),
            historyScrollView.bottomAnchor.constraint(equalTo: historyContainer.bottomAnchor, constant: -4),

            historyStackView.leadingAnchor.constraint(equalTo: historyScrollView.leadingAnchor),
            historyStackView.trailingAnchor.constraint(equalTo: historyScrollView.trailingAnchor),
        ])
    }

    func reloadHistory() {
        let history = bridge.historyList()

        // Remove old entries
        historyStackView.arrangedSubviews.forEach { $0.removeFromSuperview() }

        if history.isEmpty {
            historyContainerHeightConstraint.constant = 0
            historyContainer.isHidden = true
            return
        }

        historyContainer.isHidden = isCollapsed

        for entry in history {
            let rowView = makeHistoryRowView(entry: entry)
            historyStackView.addArrangedSubview(rowView)
            rowView.leadingAnchor.constraint(equalTo: historyStackView.leadingAnchor).isActive = true
            rowView.trailingAnchor.constraint(equalTo: historyStackView.trailingAnchor).isActive = true
        }

        // Height: header (24) + entries (min 24 each, max ~5 visible) + padding
        let entryCount = min(history.count, 5)
        let height = CGFloat(24 + entryCount * 24 + 8)
        historyContainerHeightConstraint.constant = height
    }

    private func makeHistoryRowView(entry: ClosedWorkspaceDTO) -> NSView {
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        row.wantsLayer = true
        row.heightAnchor.constraint(equalToConstant: 24).isActive = true

        let titleLabel = NSTextField(labelWithString: entry.title)
        titleLabel.font = ThaneTheme.labelFont(size: 12)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(titleLabel)

        let timeLabel = NSTextField(labelWithString: formatClosedTime(entry.closedAt))
        timeLabel.font = ThaneTheme.labelFont(size: 10)
        timeLabel.textColor = ThaneTheme.tertiaryText
        timeLabel.translatesAutoresizingMaskIntoConstraints = false
        timeLabel.setContentCompressionResistancePriority(.required, for: .horizontal)
        row.addSubview(timeLabel)

        NSLayoutConstraint.activate([
            titleLabel.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: 16),
            titleLabel.centerYAnchor.constraint(equalTo: row.centerYAnchor),
            titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: timeLabel.leadingAnchor, constant: -4),

            timeLabel.trailingAnchor.constraint(equalTo: row.trailingAnchor, constant: -12),
            timeLabel.centerYAnchor.constraint(equalTo: row.centerYAnchor),
        ])

        // Click gesture to reopen
        let clickGesture = NSClickGestureRecognizer(target: self, action: #selector(historyEntryClicked(_:)))
        row.addGestureRecognizer(clickGesture)

        // Store the entry ID so we can look it up on click
        row.identifier = NSUserInterfaceItemIdentifier(entry.id)

        // Hover effect
        let trackingArea = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: ["view": row]
        )
        row.addTrackingArea(trackingArea)

        return row
    }

    private func formatClosedTime(_ isoString: String) -> String {
        let formatter = ISO8601DateFormatter()
        guard let date = formatter.date(from: isoString) else { return "" }
        let elapsed = Date().timeIntervalSince(date)
        if elapsed < 60 { return "just now" }
        if elapsed < 3600 { return "\(Int(elapsed / 60))m ago" }
        if elapsed < 86400 { return "\(Int(elapsed / 3600))h ago" }
        return "\(Int(elapsed / 86400))d ago"
    }

    @objc private func historyEntryClicked(_ gesture: NSClickGestureRecognizer) {
        guard let view = gesture.view,
              let entryId = view.identifier?.rawValue else { return }
        _ = try? bridge.historyReopen(id: entryId)
    }

    @objc private func clearHistoryClicked() {
        bridge.historyClear()
    }

    override func mouseEntered(with event: NSEvent) {
        if let userInfo = event.trackingArea?.userInfo,
           let view = userInfo["view"] as? NSView {
            view.layer?.backgroundColor = ThaneTheme.raisedBackground.cgColor
        }
    }

    override func mouseExited(with event: NSEvent) {
        if let userInfo = event.trackingArea?.userInfo,
           let view = userInfo["view"] as? NSView {
            view.layer?.backgroundColor = nil
        }
    }

    // MARK: - Actions

    @objc private func addWorkspaceClicked() {
        let homeDir = FileManager.default.homeDirectoryForCurrentUser.path
        _ = try? bridge.createWorkspace(title: "workspace", cwd: homeDir)
    }

    @objc private func collapseClicked() {
        onToggleSidebar?()
    }

    @objc private func settingsClicked() {
        onShowPanel?(.settings)
    }

    @objc private func openFolderClicked() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Open"
        panel.message = "Select a folder to open as a workspace"

        panel.begin { [weak self] result in
            guard result == .OK, let url = panel.url else { return }
            let path = url.path
            let title = url.lastPathComponent
            _ = try? self?.bridge.createWorkspace(title: title, cwd: path)
        }
    }

    @objc private func renameWorkspace() {
        let row = tableView.clickedRow
        guard row >= 0, row < workspaces.count else { return }
        let ws = workspaces[row]

        let alert = NSAlert()
        alert.messageText = "Rename Workspace"
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
        input.stringValue = ws.title
        alert.accessoryView = input

        if alert.runModal() == .alertFirstButtonReturn {
            let newTitle = input.stringValue
            if !newTitle.isEmpty {
                _ = try? bridge.renameWorkspace(id: ws.id, title: newTitle)
            }
        }
    }

    @objc private func toggleSandboxMenu() {
        let row = tableView.clickedRow
        guard row >= 0, row < workspaces.count else { return }
        let ws = workspaces[row]

        let sandboxInfo = bridge.sandboxStatus(workspaceId: ws.id)
        let enabling = sandboxInfo?.enabled != true

        // Confirm with user — terminals will be respawned
        let alert = NSAlert()
        alert.alertStyle = .informational
        if enabling {
            alert.messageText = "Convert to Sandbox?"
            alert.informativeText = "This workspace will be sandboxed. All terminals will be closed and respawned with confinement — running processes, command history, and scroll position will be lost. Sensitive paths (.ssh, .aws, .gnupg, etc.) will be blocked."
        } else {
            alert.messageText = "Remove Sandbox?"
            alert.informativeText = "Sandbox restrictions will be removed. All terminals will be closed and respawned without confinement — running processes, command history, and scroll position will be lost."
        }
        alert.addButton(withTitle: enabling ? "Convert" : "Remove")
        alert.addButton(withTitle: "Cancel")

        guard alert.runModal() == .alertFirstButtonReturn else { return }

        if enabling {
            try? bridge.sandboxEnable(workspaceId: ws.id)
        } else {
            bridge.sandboxDisable(workspaceId: ws.id)
        }

        // Respawn terminals with new sandbox state
        _ = try? bridge.selectWorkspace(id: ws.id)
        (NSApp.delegate as? AppDelegate)?.forceRebuildWorkspace(id: ws.id)
    }

    @objc private func closeWorkspaceMenu() {
        let row = tableView.clickedRow
        guard row >= 0, row < workspaces.count else { return }
        let ws = workspaces[row]

        let alert = NSAlert()
        alert.messageText = "Close workspace \"\(ws.title)\"?"
        alert.informativeText = "This will close all panes and panels in this workspace."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Close")
        alert.addButton(withTitle: "Cancel")

        if alert.runModal() == .alertFirstButtonReturn {
            _ = try? bridge.closeWorkspace(id: ws.id)
        }
    }

    // MARK: - Collapsed view

    private func rebuildCollapsedView() {
        collapsedStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        // Expand button
        let expandBtn = NSButton()
        expandBtn.bezelStyle = .recessed
        expandBtn.isBordered = false
        expandBtn.image = NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: "Expand sidebar")
        expandBtn.contentTintColor = ThaneTheme.secondaryText
        expandBtn.toolTip = "Expand sidebar"
        expandBtn.target = self
        expandBtn.action = #selector(collapseClicked)
        expandBtn.widthAnchor.constraint(equalToConstant: 32).isActive = true
        expandBtn.heightAnchor.constraint(equalToConstant: 32).isActive = true
        collapsedStack.addArrangedSubview(expandBtn)

        // Add workspace button
        let addBtn = NSButton()
        addBtn.bezelStyle = .recessed
        addBtn.isBordered = false
        addBtn.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "New workspace")
        addBtn.contentTintColor = ThaneTheme.secondaryText
        addBtn.toolTip = "New workspace"
        addBtn.target = self
        addBtn.action = #selector(addWorkspaceClicked)
        addBtn.widthAnchor.constraint(equalToConstant: 32).isActive = true
        addBtn.heightAnchor.constraint(equalToConstant: 32).isActive = true
        collapsedStack.addArrangedSubview(addBtn)

        // Workspace avatars
        for workspace in workspaces {
            let avatar = makeAvatarView(for: workspace)
            collapsedStack.addArrangedSubview(avatar)
        }
    }

    private func makeAvatarView(for workspace: WorkspaceInfoDTO) -> NSView {
        let button = NSButton()
        button.bezelStyle = .circular
        button.isBordered = false
        button.wantsLayer = true
        button.layer?.cornerRadius = 16
        button.layer?.backgroundColor = ThaneTheme.accentColor.withAlphaComponent(0.2).cgColor

        let initial = String(workspace.title.prefix(1)).uppercased()
        button.title = initial
        button.font = ThaneTheme.boldLabelFont(size: 14)
        button.contentTintColor = ThaneTheme.primaryText
        button.toolTip = workspace.title

        button.widthAnchor.constraint(equalToConstant: 32).isActive = true
        button.heightAnchor.constraint(equalToConstant: 32).isActive = true

        // Active workspace: indigo border
        let isActive = workspace.id == bridge.activeWorkspace()?.id
        let isSandboxed = bridge.isSandboxed(workspaceId: workspace.id)

        if isActive {
            button.layer?.borderWidth = 2
            button.layer?.borderColor = ThaneTheme.accentColor.cgColor
        }

        // Sandboxed: add orange dot indicator (bottom-right corner)
        // Does NOT override active border — both indicators coexist.
        if isSandboxed {
            if !isActive {
                button.layer?.borderWidth = 2
                button.layer?.borderColor = ThaneTheme.warningColor.cgColor
            }
            let dot = NSView()
            dot.wantsLayer = true
            dot.layer?.backgroundColor = ThaneTheme.warningColor.cgColor
            dot.layer?.cornerRadius = 4
            dot.translatesAutoresizingMaskIntoConstraints = false
            button.addSubview(dot)
            NSLayoutConstraint.activate([
                dot.widthAnchor.constraint(equalToConstant: 8),
                dot.heightAnchor.constraint(equalToConstant: 8),
                dot.trailingAnchor.constraint(equalTo: button.trailingAnchor, constant: -1),
                dot.bottomAnchor.constraint(equalTo: button.bottomAnchor, constant: -1),
            ])
        }

        // Click to select
        let wsId = workspace.id
        button.target = self
        button.action = #selector(avatarClicked(_:))
        button.tag = workspaces.firstIndex(where: { $0.id == wsId }) ?? 0

        return button
    }

    @objc private func avatarClicked(_ sender: NSButton) {
        let idx = sender.tag
        guard idx < workspaces.count else { return }
        _ = try? bridge.selectWorkspace(id: workspaces[idx].id)
    }
}

// MARK: - NSMenuDelegate

extension SidebarView: NSMenuDelegate {
    func menuNeedsUpdate(_ menu: NSMenu) {
        let row = tableView.clickedRow
        guard row >= 0, row < workspaces.count else { return }
        let ws = workspaces[row]

        // Update sandbox menu item text based on current state
        if let sandboxItem = menu.items.first(where: { $0.action == #selector(toggleSandboxMenu) }) {
            let sandboxInfo = bridge.sandboxStatus(workspaceId: ws.id)
            sandboxItem.title = (sandboxInfo?.enabled == true) ? "Remove Sandbox" : "Convert to Sandbox"
        }
    }
}

// MARK: - NSTableViewDataSource

extension SidebarView: NSTableViewDataSource {
    func numberOfRows(in tableView: NSTableView) -> Int {
        return workspaces.count
    }
}

// MARK: - NSTableViewDelegate

extension SidebarView: NSTableViewDelegate {
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < workspaces.count else { return nil }
        let workspace = workspaces[row]

        let panelLocations = bridge.panelLocations(for: workspace.id)
        let isSandboxed = bridge.isSandboxed(workspaceId: workspace.id)
        let ports = bridge.workspacePorts[workspace.id] ?? []
        let wsCost = bridge.getProjectCostForCwd(workspace.cwd)
        let costScope = bridge.configGet(key: "cost-display-scope") ?? "session"
        let displayCost = costScope == "all-time" ? wsCost.alltimeCostUsd : wsCost.sessionCostUsd
        bridge.updateWorkspaceCost(workspaceId: workspace.id, cost: displayCost)
        let rowView = WorkspaceRowView(
            workspace: workspace,
            isActive: workspace.id == bridge.activeWorkspace()?.id,
            panelLocations: panelLocations,
            isSandboxed: isSandboxed,
            ports: ports,
            cost: displayCost
        )
        let wsId = workspace.id
        let wsTitle = workspace.title
        rowView.onPortClick = { [weak self] port, shiftHeld in
            self?.onPortClick?(port, shiftHeld)
        }
        rowView.onClose = { [weak self] in
            guard let self else { return }
            let alert = NSAlert()
            alert.messageText = "Close workspace \"\(wsTitle)\"?"
            alert.informativeText = "This will close all panes and panels in this workspace."
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Close")
            alert.addButton(withTitle: "Cancel")
            if alert.runModal() == .alertFirstButtonReturn {
                _ = try? self.bridge.closeWorkspace(id: wsId)
            }
        }
        return rowView
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let rowView = NSTableRowView()
        rowView.wantsLayer = true
        if row < workspaces.count, workspaces[row].id == bridge.activeWorkspace()?.id {
            rowView.layer?.backgroundColor = ThaneTheme.tabSelectedBackground.cgColor
        }
        return rowView
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !isUpdatingSelection else { return }
        let row = tableView.selectedRow
        guard row >= 0, row < workspaces.count else { return }
        _ = try? bridge.selectWorkspace(id: workspaces[row].id)
    }
}
