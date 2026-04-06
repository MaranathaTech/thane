import AppKit

/// Right-side panel for per-workspace sandbox configuration.
///
/// - Enable/disable toggle
/// - Enforcement level picker (Permissive/Enforcing/Strict)
/// - Network access toggle
/// - Path lists (read-only, read-write, denied) as editable NSTableViews
/// - "Allow Path" / "Deny Path" buttons with NSOpenPanel folder picker
@MainActor
final class SandboxPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge

    // Controls
    private let enableSwitch = NSSwitch()
    private let enforcementPopup = NSPopUpButton()
    private let networkSwitch = NSSwitch()

    // Path tables
    private let roPathsTable = NSTableView()
    private let rwPathsTable = NSTableView()
    private let deniedPathsTable = NSTableView()

    private var roPaths: [String] = []
    private var rwPaths: [String] = []
    private var deniedPaths: [String] = []

    private var sandboxInfo: SandboxInfoDTO?

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(frame: .zero)
        setupViews()
        reload()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public

    func reload() {
        guard let ws = bridge.activeWorkspace() else { return }
        sandboxInfo = bridge.sandboxStatus(workspaceId: ws.id)

        if let info = sandboxInfo {
            enableSwitch.state = info.enabled ? .on : .off
            networkSwitch.state = info.allowNetwork ? .on : .off

            switch info.enforcement {
            case .permissive: enforcementPopup.selectItem(withTitle: "Permissive")
            case .enforcing: enforcementPopup.selectItem(withTitle: "Enforcing")
            case .strict: enforcementPopup.selectItem(withTitle: "Strict")
            }

            roPaths = info.readOnlyPaths
            rwPaths = info.readWritePaths
            deniedPaths = info.deniedPaths
        } else {
            enableSwitch.state = .off
            networkSwitch.state = .off
            enforcementPopup.selectItem(at: 0)
            roPaths = []
            rwPaths = []
            deniedPaths = []
        }

        updateControlStates()
        roPathsTable.reloadData()
        rwPathsTable.reloadData()
        deniedPathsTable.reloadData()
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scrollView)

        let contentView = NSView()
        contentView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.contentView = FlippedClipView()
        scrollView.documentView = contentView

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),

            contentView.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            contentView.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            contentView.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            contentView.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
        ])

        var lastAnchor = contentView.topAnchor

        // Title
        let titleLabel = NSTextField(labelWithString: "Sandbox")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(titleLabel)
        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: lastAnchor, constant: 12),
            titleLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 12),
        ])
        lastAnchor = titleLabel.bottomAnchor

        // Enable toggle
        enableSwitch.target = self
        enableSwitch.action = #selector(enableChanged)
        lastAnchor = addRow("Enabled:", control: enableSwitch, to: contentView, below: lastAnchor)

        // Enforcement level
        enforcementPopup.addItems(withTitles: ["Permissive", "Enforcing", "Strict"])
        enforcementPopup.target = self
        enforcementPopup.action = #selector(enforcementChanged)
        lastAnchor = addRow("Enforcement:", control: enforcementPopup, to: contentView, below: lastAnchor)

        // Network access
        networkSwitch.target = self
        networkSwitch.action = #selector(networkChanged)
        lastAnchor = addRow("Network:", control: networkSwitch, to: contentView, below: lastAnchor)

        // Read-only paths
        lastAnchor = addPathSection("Read-Only Paths", table: roPathsTable, addAction: #selector(addRoPath),
                                    to: contentView, below: lastAnchor, tag: 0)

        // Read-write paths
        lastAnchor = addPathSection("Read-Write Paths", table: rwPathsTable, addAction: #selector(addRwPath),
                                    to: contentView, below: lastAnchor, tag: 1)

        // Denied paths
        lastAnchor = addPathSection("Denied Paths", table: deniedPathsTable, addAction: #selector(addDeniedPath),
                                    to: contentView, below: lastAnchor, tag: 2)

        contentView.bottomAnchor.constraint(greaterThanOrEqualTo: lastAnchor, constant: 20).isActive = true
    }

    // MARK: - Layout helpers

    private func addRow(_ labelText: String, control: NSView, to parent: NSView, below anchor: NSLayoutYAxisAnchor) -> NSLayoutYAxisAnchor {
        let label = NSTextField(labelWithString: labelText)
        label.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        label.textColor = ThaneTheme.secondaryText
        label.alignment = .right
        label.translatesAutoresizingMaskIntoConstraints = false
        parent.addSubview(label)

        control.translatesAutoresizingMaskIntoConstraints = false
        parent.addSubview(control)

        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: anchor, constant: 8),
            label.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 12),
            label.widthAnchor.constraint(equalToConstant: 100),

            control.centerYAnchor.constraint(equalTo: label.centerYAnchor),
            control.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 8),
            control.trailingAnchor.constraint(lessThanOrEqualTo: parent.trailingAnchor, constant: -12),
        ])

        return label.bottomAnchor
    }

    private func addPathSection(_ title: String, table: NSTableView, addAction: Selector,
                                 to parent: NSView, below anchor: NSLayoutYAxisAnchor, tag: Int) -> NSLayoutYAxisAnchor {
        // Section header
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        parent.addSubview(divider)

        let label = NSTextField(labelWithString: title)
        label.font = ThaneTheme.boldLabelFont(size: 11)
        label.textColor = ThaneTheme.secondaryText
        label.translatesAutoresizingMaskIntoConstraints = false
        parent.addSubview(label)

        let addButton = NSButton(title: "Add…", target: self, action: addAction)
        addButton.bezelStyle = .recessed
        addButton.controlSize = .small
        addButton.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        addButton.translatesAutoresizingMaskIntoConstraints = false
        parent.addSubview(addButton)

        // Table in a scroll view
        let tableScroll = NSScrollView()
        tableScroll.hasVerticalScroller = true
        tableScroll.borderType = .lineBorder
        tableScroll.backgroundColor = .clear
        tableScroll.translatesAutoresizingMaskIntoConstraints = false
        parent.addSubview(tableScroll)

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("path"))
        column.title = ""
        table.addTableColumn(column)
        table.headerView = nil
        table.rowHeight = 20
        table.backgroundColor = .clear
        table.delegate = self
        table.dataSource = self
        table.tag = tag

        tableScroll.documentView = table

        NSLayoutConstraint.activate([
            divider.topAnchor.constraint(equalTo: anchor, constant: 12),
            divider.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 12),
            divider.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -12),
            divider.heightAnchor.constraint(equalToConstant: 1),

            label.topAnchor.constraint(equalTo: divider.bottomAnchor, constant: 8),
            label.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 12),

            addButton.centerYAnchor.constraint(equalTo: label.centerYAnchor),
            addButton.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -12),

            tableScroll.topAnchor.constraint(equalTo: label.bottomAnchor, constant: 4),
            tableScroll.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 12),
            tableScroll.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -12),
            tableScroll.heightAnchor.constraint(equalToConstant: 80),
        ])

        return tableScroll.bottomAnchor
    }

    // MARK: - Actions

    private func updateControlStates() {
        let enabled = enableSwitch.state == .on
        enforcementPopup.isEnabled = enabled
        networkSwitch.isEnabled = enabled
    }

    @objc private func enableChanged() {
        guard let ws = bridge.activeWorkspace() else { return }
        if enableSwitch.state == .on {
            try? bridge.sandboxEnable(workspaceId: ws.id)
        } else {
            bridge.sandboxDisable(workspaceId: ws.id)
        }
        reload()
        promptRespawn(workspace: ws)
    }

    @objc private func enforcementChanged() {
        guard let ws = bridge.activeWorkspace() else { return }
        guard let level = enforcementPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.sandboxSetEnforcement(workspaceId: ws.id, level: level)
        promptRespawn(workspace: ws)
    }

    /// Show an alert asking whether to restart the workspace to apply sandbox changes.
    private func promptRespawn(workspace: WorkspaceInfoDTO) {
        let alert = NSAlert()
        alert.messageText = "Sandbox settings changed"
        alert.informativeText = "Restart this workspace to apply the new sandbox settings? All terminals will be closed — running processes, command history, and scroll position will be lost."
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Restart")
        alert.addButton(withTitle: "Cancel")

        let response = alert.runModal()
        guard response == .alertFirstButtonReturn else { return }

        let title = workspace.title
        let cwd = workspace.cwd
        try? bridge.closeWorkspace(id: workspace.id)
        _ = try? bridge.createWorkspace(title: title, cwd: cwd)
    }

    @objc private func networkChanged() {
        guard let ws = bridge.activeWorkspace() else { return }
        let allow = networkSwitch.state == .on
        bridge.sandboxSetNetwork(workspaceId: ws.id, allow: allow)
        promptRespawn(workspace: ws)
    }

    @objc private func addRoPath() {
        addPathViaOpenPanel(writable: false)
    }

    @objc private func addRwPath() {
        addPathViaOpenPanel(writable: true)
    }

    @objc private func addDeniedPath() {
        guard let ws = bridge.activeWorkspace() else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.message = "Select folder to deny access"

        panel.beginSheetModal(for: self.window ?? NSApp.mainWindow!) { response in
            guard response == .OK, let url = panel.url else { return }
            try? self.bridge.sandboxDenyPath(workspaceId: ws.id, path: url.path)
            self.reload()
        }
    }

    private func addPathViaOpenPanel(writable: Bool) {
        guard let ws = bridge.activeWorkspace() else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.message = writable ? "Select folder for read-write access" : "Select folder for read-only access"

        panel.beginSheetModal(for: self.window ?? NSApp.mainWindow!) { response in
            guard response == .OK, let url = panel.url else { return }
            try? self.bridge.sandboxAllowPath(workspaceId: ws.id, path: url.path, writable: writable)
            self.reload()
        }
    }
}

// MARK: - NSTableViewDataSource

extension SandboxPanel: NSTableViewDataSource {
    func numberOfRows(in tableView: NSTableView) -> Int {
        switch tableView.tag {
        case 0: return roPaths.count
        case 1: return rwPaths.count
        case 2: return deniedPaths.count
        default: return 0
        }
    }
}

// MARK: - NSTableViewDelegate

extension SandboxPanel: NSTableViewDelegate {
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let paths: [String]
        switch tableView.tag {
        case 0: paths = roPaths
        case 1: paths = rwPaths
        case 2: paths = deniedPaths
        default: return nil
        }

        guard row < paths.count else { return nil }

        let label = NSTextField(labelWithString: paths[row])
        label.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        label.textColor = ThaneTheme.primaryText
        label.lineBreakMode = .byTruncatingMiddle
        return label
    }
}
