import AppKit

/// Right-side panel showing git diff with a nested file tree and inline diffs.
///
/// - Hierarchical file tree with collapsible directories
/// - Click a file to show its diff below
/// - Change type indicators (M/A/D/R) with color coding
/// - Syntax-highlighted diff display
@MainActor
final class GitDiffPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge

    // Header
    private let titleLabel = NSTextField(labelWithString: "Git Changes")
    private let statusLabel = NSTextField(labelWithString: "")
    private let cwdLabel = NSTextField(labelWithString: "")

    // File tree (scrollable stack of clickable rows)
    private let treeScroll = NSScrollView()
    private let treeStack = NSStackView()

    // Diff display
    private let diffScroll = NSScrollView()
    private let diffTextView = NSTextView()

    private var diffFiles: [DiffFile] = []
    private var currentCwd: String = ""
    private var gitRoot: String = ""
    private var fileButtons: [String: NSButton] = [:]

    struct DiffFile {
        let path: String
        let changeType: String // "M", "A", "D", "R", "?"
    }

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
        loadGitDiff()
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        // Title row
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(titleLabel)

        let refreshBtn = NSButton(title: "Refresh", target: self, action: #selector(refreshClicked))
        refreshBtn.bezelStyle = .recessed
        refreshBtn.controlSize = .small
        refreshBtn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        refreshBtn.translatesAutoresizingMaskIntoConstraints = false
        addSubview(refreshBtn)

        // Status label (file count)
        statusLabel.font = ThaneTheme.uiFont(size: 11)
        statusLabel.textColor = ThaneTheme.secondaryText
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(statusLabel)

        // CWD label
        cwdLabel.font = ThaneTheme.terminalFont(size: 10)
        cwdLabel.textColor = ThaneTheme.tertiaryText
        cwdLabel.lineBreakMode = .byTruncatingHead
        cwdLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(cwdLabel)

        // File tree scroll area
        treeScroll.translatesAutoresizingMaskIntoConstraints = false
        treeScroll.hasVerticalScroller = true
        treeScroll.borderType = .noBorder
        treeScroll.backgroundColor = .clear
        treeScroll.drawsBackground = false
        addSubview(treeScroll)

        treeStack.orientation = .vertical
        treeStack.alignment = .leading
        treeStack.spacing = 0
        treeStack.translatesAutoresizingMaskIntoConstraints = false

        // Use a flipped view so content starts from the top
        let treeContent = FlippedDocumentView()
        treeContent.translatesAutoresizingMaskIntoConstraints = false
        treeContent.addSubview(treeStack)
        NSLayoutConstraint.activate([
            treeStack.topAnchor.constraint(equalTo: treeContent.topAnchor),
            treeStack.leadingAnchor.constraint(equalTo: treeContent.leadingAnchor),
            treeStack.trailingAnchor.constraint(equalTo: treeContent.trailingAnchor),
            treeStack.bottomAnchor.constraint(equalTo: treeContent.bottomAnchor),
        ])
        treeScroll.documentView = treeContent

        // Divider
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        addSubview(divider)

        // Diff display
        diffScroll.translatesAutoresizingMaskIntoConstraints = false
        diffScroll.hasVerticalScroller = true
        diffScroll.hasHorizontalScroller = true
        diffScroll.borderType = .noBorder
        diffScroll.backgroundColor = .clear
        addSubview(diffScroll)

        diffTextView.isEditable = false
        diffTextView.isSelectable = true
        diffTextView.backgroundColor = ThaneTheme.backgroundColor
        diffTextView.textContainerInset = NSSize(width: 8, height: 8)
        diffTextView.font = ThaneTheme.terminalFont(size: 11)
        diffTextView.isVerticallyResizable = true
        diffTextView.isHorizontallyResizable = true
        diffTextView.autoresizingMask = [.width]
        diffTextView.textContainer?.widthTracksTextView = false
        diffTextView.textContainer?.containerSize = NSSize(width: 10000, height: 10000)
        diffScroll.documentView = diffTextView

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),

            refreshBtn.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            refreshBtn.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),

            statusLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 2),
            statusLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            statusLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),

            cwdLabel.topAnchor.constraint(equalTo: statusLabel.bottomAnchor, constant: 2),
            cwdLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            cwdLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),

            treeScroll.topAnchor.constraint(equalTo: cwdLabel.bottomAnchor, constant: 8),
            treeScroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            treeScroll.trailingAnchor.constraint(equalTo: trailingAnchor),

            divider.topAnchor.constraint(equalTo: treeScroll.bottomAnchor),
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1),

            diffScroll.topAnchor.constraint(equalTo: divider.bottomAnchor),
            diffScroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            diffScroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            diffScroll.bottomAnchor.constraint(equalTo: bottomAnchor),

            // Split: tree gets ~40%, diff gets ~60%
            treeScroll.heightAnchor.constraint(equalTo: heightAnchor, multiplier: 0.4, constant: -40),
        ])
    }

    // MARK: - Actions

    @objc private func refreshClicked() {
        reload()
    }

    // MARK: - Git operations

    private func loadGitDiff() {
        guard let ws = bridge.activeWorkspace() else {
            setEmptyState("No active workspace")
            return
        }

        let cwd = bridge.focusedPanelCwd() ?? ws.cwd
        currentCwd = cwd

        // Run git commands on a background queue to avoid blocking the main thread
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            let toplevel = self.runGit(["rev-parse", "--show-toplevel"], cwd: cwd)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let diffCwd = toplevel.isEmpty ? cwd : toplevel
            let nameStatusOutput = self.runGit(["diff", "--name-status"], cwd: diffCwd)
            let fullDiff = self.runGit(["diff"], cwd: diffCwd)

            let files = nameStatusOutput.split(separator: "\n").compactMap { line -> DiffFile? in
                let parts = line.split(separator: "\t", maxSplits: 1)
                guard parts.count == 2 else { return nil }
                return DiffFile(path: String(parts[1]), changeType: String(parts[0]))
            }.sorted { $0.path < $1.path }

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.gitRoot = diffCwd

                let homeDir = FileManager.default.homeDirectoryForCurrentUser.path
                self.cwdLabel.stringValue = diffCwd.hasPrefix(homeDir)
                    ? "~" + diffCwd.dropFirst(homeDir.count)
                    : diffCwd

                self.diffFiles = files

                let modified = files.filter { $0.changeType == "M" }.count
                let added = files.filter { $0.changeType.hasPrefix("A") }.count
                let deleted = files.filter { $0.changeType == "D" }.count
                self.statusLabel.stringValue = "\(files.count) files (+\(added) ~\(modified) -\(deleted))"

                self.buildFileTree()

                if fullDiff.isEmpty {
                    self.setEmptyState("No changes")
                } else {
                    self.displayDiff(fullDiff)
                }
            }
        }
    }

    // MARK: - File tree

    private struct DirNode {
        var name: String
        var children: [String: DirNode] = [:]   // subdirectories
        var files: [DiffFile] = []               // files at this level
    }

    private func buildFileTree() {
        treeStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        fileButtons.removeAll()

        // Build tree structure
        var root = DirNode(name: "")
        for file in diffFiles {
            let components = file.path.split(separator: "/").map(String.init)
            var node = root
            insertIntoTree(&root, components: components, file: file)
            _ = node // suppress warning
        }

        // Render tree recursively
        renderNode(root, depth: 0, parentPath: "")
    }

    private func insertIntoTree(_ node: inout DirNode, components: [String], file: DiffFile) {
        if components.count == 1 {
            node.files.append(file)
        } else {
            let dirName = components[0]
            if node.children[dirName] == nil {
                node.children[dirName] = DirNode(name: dirName)
            }
            var remaining = Array(components.dropFirst())
            insertIntoTree(&node.children[dirName]!, components: remaining, file: file)
            _ = remaining
        }
    }

    private func renderNode(_ node: DirNode, depth: Int, parentPath: String) {
        // Sort: directories first, then files
        let sortedDirs = node.children.sorted { $0.key < $1.key }

        for (dirName, child) in sortedDirs {
            // Compact single-child directory chains
            var compacted = dirName
            var current = child
            while current.files.isEmpty && current.children.count == 1 {
                let next = current.children.first!
                compacted += "/" + next.key
                current = next.value
            }

            let dirPath = parentPath.isEmpty ? compacted : parentPath + "/" + compacted
            let dirRow = makeDirRow(name: compacted, depth: depth)
            treeStack.addArrangedSubview(dirRow)
            dirRow.widthAnchor.constraint(equalTo: treeStack.widthAnchor).isActive = true

            // Render children of the (possibly compacted) directory
            renderNode(current, depth: depth + 1, parentPath: dirPath)
        }

        // Files at this level
        for file in node.files.sorted(by: { $0.path < $1.path }) {
            let fileName = file.path.split(separator: "/").last.map(String.init) ?? file.path
            let fileRow = makeFileRow(name: fileName, file: file, depth: depth)
            treeStack.addArrangedSubview(fileRow)
            fileRow.widthAnchor.constraint(equalTo: treeStack.widthAnchor).isActive = true
        }
    }

    private func makeDirRow(name: String, depth: Int) -> NSView {
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        row.heightAnchor.constraint(equalToConstant: 22).isActive = true

        let indent = CGFloat(8 + depth * 16)

        let icon = NSTextField(labelWithString: "📁")
        icon.font = ThaneTheme.uiFont(size: 11)
        icon.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(icon)

        let label = NSTextField(labelWithString: name)
        label.font = ThaneTheme.boldLabelFont(size: 11)
        label.textColor = ThaneTheme.primaryText
        label.lineBreakMode = .byTruncatingTail
        label.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(label)

        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: indent),
            icon.centerYAnchor.constraint(equalTo: row.centerYAnchor),
            label.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 4),
            label.centerYAnchor.constraint(equalTo: row.centerYAnchor),
            label.trailingAnchor.constraint(lessThanOrEqualTo: row.trailingAnchor, constant: -8),
        ])

        return row
    }

    private func makeFileRow(name: String, file: DiffFile, depth: Int) -> NSView {
        let indent = String(repeating: "  ", count: depth)
        let indicator = changeTypeIndicator(file.changeType)
        let title = "\(indent)\(indicator)  \(name)"

        let btn = NSButton(title: title, target: self, action: #selector(fileButtonClicked(_:)))
        btn.bezelStyle = .recessed
        btn.isBordered = false
        btn.alignment = .left
        btn.font = ThaneTheme.uiFont(size: 11)
        btn.contentTintColor = ThaneTheme.primaryText
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.heightAnchor.constraint(equalToConstant: 24).isActive = true
        btn.identifier = NSUserInterfaceItemIdentifier(file.path)

        // Store file path -> button mapping for highlighting
        fileButtons[file.path] = btn

        return btn
    }

    @objc private func fileButtonClicked(_ sender: NSButton) {
        guard let filePath = sender.identifier?.rawValue else { return }
        showDiffForFile(filePath)
    }

    private func showDiffForFile(_ filePath: String) {
        // Highlight the clicked row
        for (path, btn) in fileButtons {
            btn.wantsLayer = true
            btn.layer?.backgroundColor = (path == filePath)
                ? ThaneTheme.accentColor.withAlphaComponent(0.15).cgColor
                : nil
        }

        // Show diff for this file (paths are relative to git root)
        let fileDiff = runGit(["diff", "--", filePath], cwd: gitRoot)
        if fileDiff.isEmpty {
            setEmptyState("No diff available for \(filePath)")
        } else {
            displayDiff(fileDiff)
        }
    }

    // MARK: - Git runner

    nonisolated private func runGit(_ args: [String], cwd: String) -> String {
        guard FileManager.default.fileExists(atPath: cwd) else { return "" }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = args
        process.currentDirectoryURL = URL(fileURLWithPath: cwd)

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()

        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            return String(data: data, encoding: .utf8) ?? ""
        } catch {
            return ""
        }
    }

    // MARK: - Diff display

    private func displayDiff(_ diff: String) {
        if diff.isEmpty {
            setEmptyState("No changes")
            return
        }

        let attributed = NSMutableAttributedString()
        let defaultAttrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.terminalFont(size: 11),
            .foregroundColor: ThaneTheme.primaryText,
        ]

        for line in diff.split(separator: "\n", omittingEmptySubsequences: false) {
            let lineStr = String(line)
            var attrs = defaultAttrs
            if lineStr.hasPrefix("+") && !lineStr.hasPrefix("+++") {
                attrs[.foregroundColor] = ThaneTheme.agentActiveColor
                attrs[.backgroundColor] = ThaneTheme.agentActiveColor.withAlphaComponent(0.1)
            } else if lineStr.hasPrefix("-") && !lineStr.hasPrefix("---") {
                attrs[.foregroundColor] = ThaneTheme.errorColor
                attrs[.backgroundColor] = ThaneTheme.errorColor.withAlphaComponent(0.1)
            } else if lineStr.hasPrefix("@@") {
                attrs[.foregroundColor] = ThaneTheme.accentColor
            } else if lineStr.hasPrefix("diff ") || lineStr.hasPrefix("index ") {
                attrs[.foregroundColor] = ThaneTheme.secondaryText
            }

            attributed.append(NSAttributedString(string: lineStr + "\n", attributes: attrs))
        }

        diffTextView.textStorage?.setAttributedString(attributed)
        diffTextView.scrollToBeginningOfDocument(nil)
    }

    private func setEmptyState(_ message: String) {
        let attrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.uiFont(size: ThaneTheme.uiFontSize),
            .foregroundColor: ThaneTheme.tertiaryText,
        ]
        diffTextView.textStorage?.setAttributedString(NSAttributedString(string: message, attributes: attrs))
    }

    // MARK: - Helpers

    private func changeTypeIndicator(_ type: String) -> String {
        switch type {
        case "A", "?", "??": return "A"
        case "D": return "D"
        case "M": return "M"
        case let t where t.hasPrefix("R"): return "R"
        default: return "?"
        }
    }

    private func changeTypeColor(_ type: String) -> NSColor {
        switch type {
        case "A", "?", "??": return ThaneTheme.agentActiveColor
        case "D": return ThaneTheme.errorColor
        case "M": return ThaneTheme.costColor
        case let t where t.hasPrefix("R"): return ThaneTheme.accentColor
        default: return ThaneTheme.secondaryText
        }
    }
}
