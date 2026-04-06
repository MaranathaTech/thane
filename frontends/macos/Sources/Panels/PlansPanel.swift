import AppKit

/// Right-side panel showing processed queue task history.
///
/// Matches the Linux GTK `PlansPanel`:
/// - Header with title + entry count
/// - Scrollable list of completed/failed/cancelled entries
/// - 2-row layout per entry: content+badge on top, timestamp+cost on bottom
/// - Click to view detail sheet
@MainActor
final class PlansPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge
    private let listStack = NSStackView()
    private let scrollView = NSScrollView()
    private let statusLabel = NSTextField(labelWithString: "0 entries")
    private let emptyBox = NSView()
    private var entries: [ProcessedEntry] = []

    struct ProcessedEntry {
        let id: String
        let content: String
        let status: String
        let createdAt: String
        let completedAt: String?
        let error: String?
        let costUsd: Double
        let outputLogPath: String?
    }

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(frame: .zero)
        setupViews()
        reload()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // MARK: - Public

    func reload() {
        entries = []
        listStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        let historyEntries = loadQueueHistory()
        let homeDir = FileManager.default.homeDirectoryForCurrentUser
        let plansDir = homeDir.appendingPathComponent("thane/plans")

        for entry in historyEntries {
            let logPath = plansDir.appendingPathComponent(entry.id).appendingPathComponent("output.log")
            let hasLog = FileManager.default.fileExists(atPath: logPath.path)
            entries.append(ProcessedEntry(
                id: entry.id, content: entry.content, status: entry.status,
                createdAt: entry.createdAt, completedAt: entry.completedAt,
                error: entry.error, costUsd: entry.costUsd,
                outputLogPath: hasLog ? logPath.path : nil
            ))
        }

        entries.sort { $0.createdAt > $1.createdAt }
        statusLabel.stringValue = "\(entries.count) entries"

        if entries.isEmpty {
            listStack.addArrangedSubview(emptyBox)
            emptyBox.widthAnchor.constraint(equalTo: listStack.widthAnchor).isActive = true
        } else {
            for (idx, entry) in entries.enumerated() {
                let row = makeEntryRow(entry, index: idx)
                listStack.addArrangedSubview(row)
                row.widthAnchor.constraint(equalTo: listStack.widthAnchor).isActive = true
            }
        }
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        // Header
        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)

        let titleLabel = NSTextField(labelWithString: "Processed")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(titleLabel)

        statusLabel.font = ThaneTheme.uiFont(size: 12)
        statusLabel.textColor = ThaneTheme.tertiaryText
        statusLabel.alignment = .right
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(statusLabel)

        NSLayoutConstraint.activate([
            titleLabel.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            titleLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            statusLabel.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -12),
            statusLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
        ])

        let headerSep = ViewFactories.makeDivider()
        addSubview(headerSep)

        // Empty state
        setupEmptyBox()

        // Scrollable list
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        addSubview(scrollView)

        listStack.orientation = .vertical
        listStack.alignment = .leading
        listStack.spacing = 0
        listStack.translatesAutoresizingMaskIntoConstraints = false

        let listContent = FlippedDocumentView()
        listContent.translatesAutoresizingMaskIntoConstraints = false
        listContent.addSubview(listStack)
        scrollView.contentView = FlippedClipView()
        scrollView.documentView = listContent
        NSLayoutConstraint.activate([
            listStack.topAnchor.constraint(equalTo: listContent.topAnchor),
            listStack.leadingAnchor.constraint(equalTo: listContent.leadingAnchor),
            listStack.trailingAnchor.constraint(equalTo: listContent.trailingAnchor),
            listStack.bottomAnchor.constraint(equalTo: listContent.bottomAnchor),
            listContent.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
        ])

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 40),

            headerSep.topAnchor.constraint(equalTo: header.bottomAnchor),
            headerSep.leadingAnchor.constraint(equalTo: leadingAnchor),
            headerSep.trailingAnchor.constraint(equalTo: trailingAnchor),

            scrollView.topAnchor.constraint(equalTo: headerSep.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    private func setupEmptyBox() {
        let content = ViewFactories.makeEmptyState(
            icon: "clock.arrow.circlepath",
            title: "No processed tasks",
            hint: "Completed queue tasks will appear here"
        )
        content.translatesAutoresizingMaskIntoConstraints = false
        emptyBox.translatesAutoresizingMaskIntoConstraints = false
        emptyBox.addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: emptyBox.topAnchor),
            content.leadingAnchor.constraint(equalTo: emptyBox.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: emptyBox.trailingAnchor),
            content.bottomAnchor.constraint(equalTo: emptyBox.bottomAnchor),
        ])
    }

    // MARK: - Row building (matches GTK queue-item layout)

    private func makeEntryRow(_ entry: ProcessedEntry, index: Int) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        container.wantsLayer = true

        let border = NSView()
        border.wantsLayer = true
        border.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        border.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(border)

        // Top: content + badge
        let firstLine = entry.content
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .components(separatedBy: .newlines).first ?? entry.content
        let truncated = firstLine.count > 60 ? String(firstLine.prefix(57)) + "..." : firstLine

        let contentLabel = NSTextField(labelWithString: truncated)
        contentLabel.font = ThaneTheme.uiFont(size: 13)
        contentLabel.textColor = ThaneTheme.primaryText
        contentLabel.lineBreakMode = .byTruncatingTail
        contentLabel.maximumNumberOfLines = 1
        contentLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        contentLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(contentLabel)

        let badge = makeBadge(entry.status)
        container.addSubview(badge)

        // Bottom: timestamp + cost
        let timeStr = formatTimestamp(entry.completedAt ?? entry.createdAt)
        let costStr = entry.costUsd > 0 ? " \u{00B7} $\(String(format: "%.4f", entry.costUsd))" : ""
        let metaLabel = NSTextField(labelWithString: "\(timeStr)\(costStr)")
        metaLabel.font = ThaneTheme.uiFont(size: 11)
        metaLabel.textColor = ThaneTheme.tertiaryText
        metaLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(metaLabel)

        NSLayoutConstraint.activate([
            contentLabel.topAnchor.constraint(equalTo: container.topAnchor, constant: 10),
            contentLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 14),
            contentLabel.trailingAnchor.constraint(lessThanOrEqualTo: badge.leadingAnchor, constant: -8),

            badge.centerYAnchor.constraint(equalTo: contentLabel.centerYAnchor),
            badge.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -14),

            metaLabel.topAnchor.constraint(equalTo: contentLabel.bottomAnchor, constant: 4),
            metaLabel.leadingAnchor.constraint(equalTo: contentLabel.leadingAnchor),
            metaLabel.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -10),

            border.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            border.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            border.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            border.heightAnchor.constraint(equalToConstant: 1),
        ])

        // Click gesture to show detail
        let btn = NSButton(title: "", target: self, action: #selector(entryClicked(_:)))
        btn.tag = index
        btn.bezelStyle = .recessed
        btn.isBordered = false
        btn.isTransparent = true
        btn.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(btn)
        NSLayoutConstraint.activate([
            btn.topAnchor.constraint(equalTo: container.topAnchor),
            btn.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            btn.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            btn.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        return container
    }

    private func makeBadge(_ statusStr: String) -> NSTextField {
        let color = statusColor(statusStr)
        let text = " \(statusDisplayText(statusStr)) "
        let badge = NSTextField(labelWithString: text)
        badge.font = NSFont.boldSystemFont(ofSize: 11)
        badge.textColor = color
        badge.wantsLayer = true
        badge.layer?.cornerRadius = 4

        // Only queued/running/paused get background pills
        switch statusStr {
        case "queued", "running":
            badge.layer?.backgroundColor = color.withAlphaComponent(0.12).cgColor
        default:
            break
        }

        badge.alignment = .center
        badge.translatesAutoresizingMaskIntoConstraints = false
        badge.setContentHuggingPriority(.required, for: .horizontal)
        badge.setContentCompressionResistancePriority(.required, for: .horizontal)
        return badge
    }

    // MARK: - Detail view

    @objc private func entryClicked(_ sender: NSButton) {
        guard sender.tag >= 0, sender.tag < entries.count else { return }
        let entry = entries[sender.tag]

        // Highlight selected row
        for (i, arranged) in listStack.arrangedSubviews.enumerated() {
            arranged.layer?.backgroundColor = (i == sender.tag)
                ? ThaneTheme.accentColor.withAlphaComponent(0.1).cgColor
                : nil
        }

        showDetailSheet(entry)
    }

    private func showDetailSheet(_ entry: ProcessedEntry) {
        guard let w = window else { return }

        let sheet = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 560, height: 440),
                            styleMask: [.titled, .closable, .resizable],
                            backing: .buffered, defer: false)
        sheet.title = "Plan Detail"
        sheet.isFloatingPanel = true

        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        sheet.contentView = container

        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        container.addSubview(scrollView)

        let textView = NSTextView()
        textView.isEditable = false
        textView.isSelectable = true
        textView.backgroundColor = ThaneTheme.backgroundColor
        textView.textContainerInset = NSSize(width: 12, height: 12)
        textView.font = ThaneTheme.terminalFont(size: 11)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        scrollView.documentView = textView

        // Close button
        let closeBtn = NSButton(title: "Close", target: nil, action: nil)
        closeBtn.bezelStyle = .rounded
        closeBtn.keyEquivalent = "\u{1b}" // Escape key
        closeBtn.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(closeBtn)

        // Wire close action
        closeBtn.target = self
        closeBtn.action = #selector(closeDetailSheet(_:))
        closeBtn.identifier = NSUserInterfaceItemIdentifier("sheetClose")

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: closeBtn.topAnchor, constant: -8),

            closeBtn.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            closeBtn.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -12),
        ])

        // Build detail text
        var text = ""
        text += "Status: \(statusDisplayText(entry.status))\n"
        text += "Content: \(entry.content)\n\n"
        text += "Created: \(formatTimestamp(entry.createdAt))\n"
        if let completed = entry.completedAt {
            text += "Completed: \(formatTimestamp(completed))\n"
        }
        if entry.costUsd > 0 {
            text += String(format: "Cost: $%.4f\n", entry.costUsd)
        }
        if let error = entry.error {
            text += "Error: \(error)\n"
        }

        if let logPath = entry.outputLogPath,
           let logContent = try? String(contentsOfFile: logPath, encoding: .utf8) {
            text += "\n--- Output Log ---\n\n"
            text += logContent
        }

        let attrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.terminalFont(size: 11),
            .foregroundColor: ThaneTheme.primaryText,
        ]
        textView.textStorage?.setAttributedString(NSAttributedString(string: text, attributes: attrs))

        w.beginSheet(sheet)
    }

    @objc private func closeDetailSheet(_ sender: NSButton) {
        guard let sheet = sender.window, let parent = window else { return }
        parent.endSheet(sheet)
    }

    // MARK: - Helpers

    private func statusDisplayText(_ status: String) -> String {
        switch status {
        case "completed": return "Done"
        case "failed": return "Failed"
        case "cancelled": return "Cancelled"
        case "running": return "Running"
        case "queued": return "Queued"
        default: return status.capitalized
        }
    }

    private func statusColor(_ status: String) -> NSColor {
        switch status {
        case "completed": return ThaneTheme.agentActiveColor
        case "failed": return ThaneTheme.errorColor
        case "cancelled": return ThaneTheme.tertiaryText
        case "running": return ThaneTheme.agentActiveColor
        case "queued": return ThaneTheme.accentColor
        default: return ThaneTheme.tertiaryText
        }
    }

    private func formatTimestamp(_ ts: String) -> String {
        if ts.count > 16 {
            return String(ts.prefix(16)).replacingOccurrences(of: "T", with: " ")
        }
        return ts
    }

    // MARK: - Queue history

    private struct HistoryEntry {
        let id: String
        let content: String
        let status: String
        let createdAt: String
        let completedAt: String?
        let error: String?
        let costUsd: Double
    }

    private func loadQueueHistory() -> [HistoryEntry] {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let historyFile = appSupport.appendingPathComponent("thane/queue_history.json")

        guard let data = try? Data(contentsOf: historyFile),
              let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
            return []
        }

        return array.compactMap { dict in
            guard let id = dict["id"] as? String,
                  let content = dict["content"] as? String,
                  let status = dict["status"] as? String,
                  let createdAt = dict["createdAt"] as? String else { return nil }
            return HistoryEntry(
                id: id, content: content, status: status, createdAt: createdAt,
                completedAt: dict["completedAt"] as? String,
                error: dict["error"] as? String,
                costUsd: dict["costUsd"] as? Double ?? 0
            )
        }
    }
}


