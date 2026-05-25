import AppKit

/// Right-side panel listing audit events with severity filtering and date range selection.
///
/// - Date range picker: "Today" (default), preset buttons (7d, 3d, 1d), or custom date pickers
/// - NSSegmentedControl filter: All / Warning+ / Alert+ / Critical
/// - "Export JSON" and "Clear" buttons
/// - NSTableView of AuditEventInfoDTO rows
///
/// Today shows live in-memory events. Historical dates load from daily JSONL files on disk
/// (~/Library/Application Support/thane/audit/audit-YYYY-MM-DD.jsonl).
/// Free tier retains 7 days of logs.
@MainActor
final class AuditPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge
    private let scrollView = NSScrollView()
    private let tableView = NSTableView()
    private let emptyBox = NSView()
    private let filterControl = NSSegmentedControl()
    private let searchField = NSSearchField()
    private let dateRangeControl = NSSegmentedControl()
    private let fromPicker = NSDatePicker()
    private let toPicker = NSDatePicker()
    private let customDateRow = NSStackView()
    private let retentionLabel = NSTextField(labelWithString: "")
    private var verifyBtn: NSButton?
    /// Phase 5: row of per-sink status pills. Lives above the filter row.
    /// Empty / hidden when no external sink is configured.
    private let sinkPillsRow = NSStackView()
    private var allEvents: [AuditEventInfoDTO] = []
    private var filteredEvents: [AuditEventInfoDTO] = []
    private var minSeverity: AuditSeverityDTO? = nil
    private var searchText: String = ""

    /// Whether we're showing live (in-memory) events or historical (disk) events.
    private var isLiveMode = true

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
        if isLiveMode {
            allEvents = bridge.listAuditEvents().reversed()
        } else {
            let from = Calendar.current.startOfDay(for: fromPicker.dateValue)
            let to = Calendar.current.date(byAdding: .day, value: 1, to: Calendar.current.startOfDay(for: toPicker.dateValue))!
            allEvents = bridge.loadAuditEvents(from: from, to: to)
        }
        applyFilter()
        refreshSinkPills()
    }

    /// Rebuild the per-sink status pill row from the latest dispatcher snapshot.
    /// Hidden when no sink is configured.
    private func refreshSinkPills() {
        // Remove old pills.
        for arranged in sinkPillsRow.arrangedSubviews {
            sinkPillsRow.removeArrangedSubview(arranged)
            arranged.removeFromSuperview()
        }
        let sinks = bridge.auditSinkStatus()
        if sinks.isEmpty {
            sinkPillsRow.isHidden = true
            return
        }
        sinkPillsRow.isHidden = false

        let label = NSTextField(labelWithString: "Sinks:")
        label.font = ThaneTheme.uiFont(size: 11)
        label.textColor = ThaneTheme.secondaryText
        sinkPillsRow.addArrangedSubview(label)

        for sink in sinks {
            let btn = NSButton(title: sink.name, target: self, action: #selector(sinkPillClicked(_:)))
            btn.bezelStyle = .recessed
            btn.controlSize = .small
            btn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
            // Tag carries the index so the click handler can re-fetch the sink dict.
            btn.tag = sinkPillsRow.arrangedSubviews.count - 1
            switch sink.status {
            case "failing":
                btn.contentTintColor = NSColor.systemRed
            case "degraded":
                btn.contentTintColor = NSColor.systemYellow
            default:
                btn.contentTintColor = NSColor.systemGreen
            }
            btn.toolTip = "\(sink.name): \(sink.status) — queued \(sink.queued), sent \(sink.sentTotal), DLQ \(sink.dlqTotal)"
            sinkPillsRow.addArrangedSubview(btn)
        }
    }

    @objc private func sinkPillClicked(_ sender: NSButton) {
        let sinks = bridge.auditSinkStatus()
        // The tag is (pill button index in the row, minus the "Sinks:" label).
        // We re-derive it instead of trusting tag because the row may have
        // re-rendered between click registration and click delivery.
        guard let arrangedIdx = sinkPillsRow.arrangedSubviews.firstIndex(of: sender) else { return }
        let sinkIdx = arrangedIdx - 1 // first arranged subview is the "Sinks:" label
        guard sinkIdx >= 0 && sinkIdx < sinks.count else { return }
        showSinkDetailDialog(sinks[sinkIdx])
    }

    private func showSinkDetailDialog(_ sink: RustBridge.SinkStatusDTO) {
        let alert = NSAlert()
        alert.messageText = "Audit Sink: \(sink.name)"
        let lines: [String] = [
            "Status: \(sink.status)",
            "Enabled: \(sink.enabled)",
            "Queued: \(sink.queued)",
            "Sent total: \(sink.sentTotal)",
            "DLQ total: \(sink.dlqTotal)",
            "Last success: \(sink.lastSuccess ?? "—")",
            "Last error: \(sink.lastError ?? "—")",
        ]
        alert.informativeText = lines.joined(separator: "\n")
        alert.addButton(withTitle: "OK")
        alert.beginSheetModal(for: self.window ?? NSApp.mainWindow!) { _ in }
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        // Header with title
        let titleLabel = NSTextField(labelWithString: "Audit Log")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(titleLabel)

        // Phase 4: at-rest encryption indicator. The lock icon sits next to the
        // title when `audit-encryption-enabled` is true (the default).
        let encryptionEnabled = (bridge.configGet(key: "audit-encryption-enabled") ?? "true") != "false"
        if encryptionEnabled {
            let lockImage = NSImage(systemSymbolName: "lock.shield", accessibilityDescription: "Audit logs encrypted at rest")
            let lockView = NSImageView(image: lockImage ?? NSImage())
            lockView.symbolConfiguration = .init(pointSize: 11, weight: .medium)
            lockView.contentTintColor = ThaneTheme.secondaryText
            lockView.toolTip = "Audit logs encrypted at rest (AES-256-GCM). Key stored in Keychain."
            lockView.translatesAutoresizingMaskIntoConstraints = false
            addSubview(lockView)
            NSLayoutConstraint.activate([
                lockView.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
                lockView.leadingAnchor.constraint(equalTo: titleLabel.trailingAnchor, constant: 6),
                lockView.widthAnchor.constraint(equalToConstant: 14),
                lockView.heightAnchor.constraint(equalToConstant: 14),
            ])
        }

        // Action buttons
        let exportBtn = NSButton(title: "Export JSON", target: self, action: #selector(exportClicked))
        exportBtn.bezelStyle = .recessed
        exportBtn.controlSize = .small
        exportBtn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        exportBtn.translatesAutoresizingMaskIntoConstraints = false
        addSubview(exportBtn)

        let clearBtn = NSButton(title: "Clear", target: self, action: #selector(clearClicked))
        clearBtn.bezelStyle = .recessed
        clearBtn.controlSize = .small
        clearBtn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        clearBtn.translatesAutoresizingMaskIntoConstraints = false
        // Audit clearing is opt-in via `audit-allow-clear`. Hide the control unless
        // policy permits it; the underlying call still re-checks.
        clearBtn.isHidden = !bridge.auditAllowClear
        addSubview(clearBtn)

        let verifyBtn = NSButton(title: "Verify", target: self, action: #selector(verifyClicked))
        verifyBtn.bezelStyle = .recessed
        verifyBtn.controlSize = .small
        verifyBtn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        verifyBtn.toolTip = "Verify the cryptographic integrity of the audit log"
        verifyBtn.translatesAutoresizingMaskIntoConstraints = false
        addSubview(verifyBtn)
        self.verifyBtn = verifyBtn

        // ── Date range selector ──
        dateRangeControl.segmentCount = 4
        dateRangeControl.setLabel("Today", forSegment: 0)
        dateRangeControl.setLabel("3d", forSegment: 1)
        dateRangeControl.setLabel("7d", forSegment: 2)
        dateRangeControl.setLabel("Custom", forSegment: 3)
        dateRangeControl.selectedSegment = 0
        dateRangeControl.target = self
        dateRangeControl.action = #selector(dateRangeChanged)
        dateRangeControl.controlSize = .small
        dateRangeControl.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        dateRangeControl.translatesAutoresizingMaskIntoConstraints = false
        addSubview(dateRangeControl)

        // Custom date row (hidden by default)
        setupCustomDateRow()
        customDateRow.isHidden = true
        addSubview(customDateRow)

        // Retention hint — text derived from `audit-retention-days` policy.
        retentionLabel.font = ThaneTheme.uiFont(size: 10)
        retentionLabel.textColor = ThaneTheme.tertiaryText
        let retention = bridge.auditRetentionDays
        retentionLabel.stringValue = retention == 0
            ? "Logs retained indefinitely"
            : "Logs retained for \(retention) days"
        retentionLabel.translatesAutoresizingMaskIntoConstraints = false
        retentionLabel.isHidden = true
        addSubview(retentionLabel)

        // Phase 5: per-sink status pills row (hidden when no sink configured).
        sinkPillsRow.orientation = .horizontal
        sinkPillsRow.spacing = 4
        sinkPillsRow.alignment = .centerY
        sinkPillsRow.isHidden = true
        sinkPillsRow.translatesAutoresizingMaskIntoConstraints = false
        addSubview(sinkPillsRow)

        // Severity filter
        filterControl.segmentCount = 4
        filterControl.setLabel("All", forSegment: 0)
        filterControl.setLabel("Warning+", forSegment: 1)
        filterControl.setLabel("Alert+", forSegment: 2)
        filterControl.setLabel("Critical", forSegment: 3)
        filterControl.selectedSegment = 0
        filterControl.target = self
        filterControl.action = #selector(filterChanged)
        filterControl.controlSize = .small
        filterControl.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        filterControl.translatesAutoresizingMaskIntoConstraints = false
        addSubview(filterControl)

        // Search field
        searchField.placeholderString = "Search events..."
        searchField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        searchField.target = self
        searchField.action = #selector(searchChanged)
        searchField.translatesAutoresizingMaskIntoConstraints = false
        addSubview(searchField)

        // Table
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        addSubview(scrollView)

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("audit"))
        column.title = ""
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 56
        tableView.backgroundColor = .clear
        tableView.delegate = self
        tableView.dataSource = self
        tableView.selectionHighlightStyle = .none
        tableView.doubleAction = #selector(rowDoubleClicked)
        tableView.target = self

        scrollView.documentView = tableView

        // Empty state
        setupEmptyBox()
        addSubview(emptyBox)

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),

            clearBtn.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            clearBtn.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),

            verifyBtn.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            verifyBtn.trailingAnchor.constraint(equalTo: clearBtn.leadingAnchor, constant: -4),

            exportBtn.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            exportBtn.trailingAnchor.constraint(equalTo: verifyBtn.leadingAnchor, constant: -4),

            dateRangeControl.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 8),
            dateRangeControl.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            dateRangeControl.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),

            customDateRow.topAnchor.constraint(equalTo: dateRangeControl.bottomAnchor, constant: 6),
            customDateRow.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            customDateRow.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),

            retentionLabel.topAnchor.constraint(equalTo: customDateRow.bottomAnchor, constant: 2),
            retentionLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),

            sinkPillsRow.topAnchor.constraint(equalTo: retentionLabel.bottomAnchor, constant: 4),
            sinkPillsRow.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            sinkPillsRow.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),

            filterControl.topAnchor.constraint(equalTo: sinkPillsRow.bottomAnchor, constant: 6),
            filterControl.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            filterControl.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),

            searchField.topAnchor.constraint(equalTo: filterControl.bottomAnchor, constant: 6),
            searchField.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            searchField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),

            scrollView.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 6),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),

            emptyBox.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 6),
            emptyBox.leadingAnchor.constraint(equalTo: leadingAnchor),
            emptyBox.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    private func setupCustomDateRow() {
        customDateRow.orientation = .horizontal
        customDateRow.spacing = 6
        customDateRow.alignment = .centerY
        customDateRow.translatesAutoresizingMaskIntoConstraints = false

        let fromLabel = NSTextField(labelWithString: "From:")
        fromLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        fromLabel.textColor = ThaneTheme.secondaryText

        fromPicker.datePickerStyle = .textFieldAndStepper
        fromPicker.datePickerElements = .yearMonthDay
        fromPicker.dateValue = Date()
        fromPicker.controlSize = .small
        fromPicker.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        fromPicker.target = self
        fromPicker.action = #selector(customDateChanged)
        // Limit to retention window. `0` is the retain-forever sentinel — fall back to
        // a year so the picker is still usable.
        let calendar = Calendar.current
        let retentionDays = bridge.auditRetentionDays
        let effectiveLookback = retentionDays == 0 ? 365 : retentionDays
        fromPicker.minDate = calendar.date(byAdding: .day, value: -effectiveLookback, to: Date())
        fromPicker.maxDate = Date()

        let toLabel = NSTextField(labelWithString: "To:")
        toLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        toLabel.textColor = ThaneTheme.secondaryText

        toPicker.datePickerStyle = .textFieldAndStepper
        toPicker.datePickerElements = .yearMonthDay
        toPicker.dateValue = Date()
        toPicker.controlSize = .small
        toPicker.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        toPicker.target = self
        toPicker.action = #selector(customDateChanged)
        toPicker.minDate = fromPicker.minDate
        toPicker.maxDate = Date()

        customDateRow.addArrangedSubview(fromLabel)
        customDateRow.addArrangedSubview(fromPicker)
        customDateRow.addArrangedSubview(toLabel)
        customDateRow.addArrangedSubview(toPicker)
    }

    private func setupEmptyBox() {
        let content = ViewFactories.makeEmptyState(
            icon: "shield",
            title: "No audit events",
            hint: "Security events will be logged here as they occur"
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

    // MARK: - Date range

    @objc private func dateRangeChanged() {
        let calendar = Calendar.current
        let today = Date()

        switch dateRangeControl.selectedSegment {
        case 0: // Today (live)
            isLiveMode = true
            customDateRow.isHidden = true
            retentionLabel.isHidden = true
        case 1: // 3d
            isLiveMode = false
            customDateRow.isHidden = true
            retentionLabel.isHidden = true
            fromPicker.dateValue = calendar.date(byAdding: .day, value: -2, to: today)!
            toPicker.dateValue = today
        case 2: // 7d
            isLiveMode = false
            customDateRow.isHidden = true
            retentionLabel.isHidden = false
            fromPicker.dateValue = calendar.date(byAdding: .day, value: -6, to: today)!
            toPicker.dateValue = today
        case 3: // Custom
            isLiveMode = false
            customDateRow.isHidden = false
            retentionLabel.isHidden = false
        default:
            break
        }
        reload()
    }

    @objc private func customDateChanged() {
        // Ensure from <= to
        if fromPicker.dateValue > toPicker.dateValue {
            toPicker.dateValue = fromPicker.dateValue
        }
        reload()
    }

    // MARK: - Actions

    @objc private func filterChanged() {
        switch filterControl.selectedSegment {
        case 0: minSeverity = nil
        case 1: minSeverity = .warning
        case 2: minSeverity = .alert
        case 3: minSeverity = .critical
        default: minSeverity = nil
        }
        applyFilter()
    }

    @objc private func searchChanged() {
        searchText = searchField.stringValue
        applyFilter()
    }

    @objc private func exportClicked() {
        // Export currently visible (filtered) events
        let events = filteredEvents
        let dicts = events.map { e -> [String: Any] in
            var d: [String: Any] = [
                "id": e.id, "timestamp": e.timestamp,
                "workspace_id": e.workspaceId,
                "event_type": e.eventType,
                "severity": e.severity.label,
                "description": e.description,
                "metadata": e.metadataJson,
            ]
            if let pid = e.panelId { d["panel_id"] = pid }
            if let agent = e.agentName { d["agent_name"] = agent }
            return d
        }
        guard let data = try? JSONSerialization.data(withJSONObject: dicts, options: .prettyPrinted),
              let json = String(data: data, encoding: .utf8) else { return }

        let savePanel = NSSavePanel()
        savePanel.allowedContentTypes = [.json]
        savePanel.nameFieldStringValue = "thane-audit.json"
        savePanel.beginSheetModal(for: self.window ?? NSApp.mainWindow!) { response in
            guard response == .OK, let url = savePanel.url else { return }
            try? json.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    @objc private func verifyClicked() {
        let result = bridge.verifyAuditIntegrity()
        let alert = NSAlert()
        alert.alertStyle = result.verified ? .informational : .critical
        if result.verified {
            alert.messageText = "Audit log verified (\(result.eventsChecked) events)"
            alert.informativeText = "Signatures valid and hash chain intact."
        } else {
            alert.messageText = "Audit log integrity FAILED"
            let summary = result.failureSummary ?? "Verification returned no failure detail."
            alert.informativeText = "Examined \(result.eventsChecked) events before failure.\n\(summary)"
        }
        alert.addButton(withTitle: "OK")
        alert.beginSheetModal(for: self.window ?? NSApp.mainWindow!) { _ in }
    }

    @objc private func clearClicked() {
        let count = bridge.auditEventCount
        let alert = NSAlert()
        alert.messageText = "Clear Audit Log?"
        alert.informativeText = "This action will be recorded as a Critical audit event with the reason below. \(count) events will be removed."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Clear")
        alert.addButton(withTitle: "Cancel")

        let reasonField = NSTextField(frame: NSRect(x: 0, y: 0, width: 300, height: 24))
        reasonField.placeholderString = "Reason (required)"
        alert.accessoryView = reasonField

        let attempt: (NSApplication.ModalResponse) -> Void = { [weak self] response in
            guard let self = self, response == .alertFirstButtonReturn else { return }
            let reason = reasonField.stringValue
            if self.bridge.clearAuditLogAdmin(reason: reason) {
                self.reload()
            } else {
                let nag = NSAlert()
                nag.messageText = "Audit clear rejected"
                nag.informativeText = reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "A non-empty reason is required."
                    : "Clearing is disabled by policy (audit-allow-clear)."
                nag.alertStyle = .warning
                nag.runModal()
            }
        }

        guard let window = self.window ?? NSApp.mainWindow else {
            attempt(alert.runModal())
            return
        }
        alert.beginSheetModal(for: window, completionHandler: attempt)
    }

    @objc private func rowDoubleClicked() {
        let row = tableView.clickedRow
        guard row >= 0, row < filteredEvents.count else { return }
        let event = filteredEvents[row]

        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 420),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        panel.title = "Audit Event Detail"
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = false

        let contentView = NSView(frame: panel.contentView!.bounds)
        contentView.autoresizingMask = [.width, .height]

        var yOffset: CGFloat = contentView.bounds.height - 8

        // Helper to add a label row (wraps long values)
        func addField(label: String, value: String, bold: Bool = false) {
            let labelField = NSTextField(labelWithString: label)
            labelField.font = ThaneTheme.boldLabelFont(size: 11)
            labelField.textColor = ThaneTheme.secondaryText
            labelField.frame = NSRect(x: 12, y: yOffset - 18, width: 120, height: 18)
            contentView.addSubview(labelField)

            let valueField = NSTextField(wrappingLabelWithString: value)
            valueField.font = bold ? ThaneTheme.boldLabelFont(size: 12) : ThaneTheme.uiFont(size: 12)
            valueField.textColor = ThaneTheme.primaryText
            valueField.isSelectable = true
            valueField.maximumNumberOfLines = 0
            valueField.preferredMaxLayoutWidth = 330
            // Size to fit the content
            let fittingSize = valueField.sizeThatFits(NSSize(width: 330, height: CGFloat.greatestFiniteMagnitude))
            let rowHeight = max(18, ceil(fittingSize.height))
            valueField.frame = NSRect(x: 136, y: yOffset - rowHeight, width: 330, height: rowHeight)
            contentView.addSubview(valueField)
            // Align label to top of value
            labelField.frame.origin.y = yOffset - 18

            yOffset -= rowHeight + 8
        }

        addField(label: "Event Type:", value: event.eventType, bold: true)
        addField(label: "Timestamp:", value: event.timestamp)
        addField(label: "Severity:", value: severityLabel(event.severity))
        // Render literal \n sequences as actual line breaks in the detail view
        addField(label: "Description:", value: event.description
            .replacingOccurrences(of: "\\n", with: "\n")
            .replacingOccurrences(of: "\\t", with: "\t"))
        if let agent = event.agentName, !agent.isEmpty {
            addField(label: "Agent:", value: agent, bold: true)
        }
        addField(label: "Workspace:", value: event.workspaceId)

        // Metadata JSON section
        let metaLabel = NSTextField(labelWithString: "Metadata:")
        metaLabel.font = ThaneTheme.boldLabelFont(size: 11)
        metaLabel.textColor = ThaneTheme.secondaryText
        metaLabel.frame = NSRect(x: 12, y: yOffset - 18, width: 120, height: 18)
        contentView.addSubview(metaLabel)
        yOffset -= 24

        let metaScrollView = NSScrollView(frame: NSRect(x: 12, y: 12, width: 456, height: max(yOffset - 12, 200)))
        metaScrollView.hasVerticalScroller = true
        metaScrollView.borderType = .lineBorder
        metaScrollView.autoresizingMask = [.width, .height]

        let metaTextView = NSTextView(frame: metaScrollView.contentView.bounds)
        metaTextView.isEditable = false
        metaTextView.isSelectable = true
        metaTextView.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        metaTextView.textColor = ThaneTheme.primaryText
        metaTextView.backgroundColor = ThaneTheme.backgroundColor
        metaTextView.autoresizingMask = [.width]

        // Format metadata as JSON
        let metadataJson = event.metadataJson
        if let jsonData = metadataJson.data(using: .utf8),
           let jsonObj = try? JSONSerialization.jsonObject(with: jsonData),
           let prettyData = try? JSONSerialization.data(withJSONObject: jsonObj, options: [.prettyPrinted, .sortedKeys]),
           let prettyStr = String(data: prettyData, encoding: .utf8) {
            metaTextView.string = prettyStr
        } else {
            metaTextView.string = metadataJson.isEmpty ? "{}" : metadataJson
        }

        metaScrollView.documentView = metaTextView
        contentView.addSubview(metaScrollView)

        panel.contentView = contentView
        panel.center()
        panel.makeKeyAndOrderFront(nil)
    }

    // MARK: - Filtering

    private func applyFilter() {
        var events = allEvents

        // Apply severity filter
        if let min = minSeverity {
            let minOrdinal = severityOrdinal(min)
            events = events.filter { severityOrdinal($0.severity) >= minOrdinal }
        }

        // Apply text search filter
        if !searchText.isEmpty {
            let query = searchText.lowercased()
            events = events.filter {
                $0.description.lowercased().contains(query) ||
                $0.eventType.lowercased().contains(query)
            }
        }

        filteredEvents = events
        tableView.reloadData()
        emptyBox.isHidden = !filteredEvents.isEmpty
        scrollView.isHidden = filteredEvents.isEmpty
    }

    private func severityOrdinal(_ severity: AuditSeverityDTO) -> Int {
        switch severity {
        case .info: return 0
        case .warning: return 1
        case .alert: return 2
        case .critical: return 3
        }
    }

    private func severityColor(_ severity: AuditSeverityDTO) -> NSColor {
        switch severity {
        case .info: return ThaneTheme.secondaryText
        case .warning: return ThaneTheme.warningColor
        case .alert: return NSColor(red: 1.0, green: 0.6, blue: 0.2, alpha: 1.0) // distinct orange
        case .critical: return ThaneTheme.errorColor
        }
    }

    private func severityLabel(_ severity: AuditSeverityDTO) -> String {
        switch severity {
        case .info: return "INFO"
        case .warning: return "WARN"
        case .alert: return "ALERT"
        case .critical: return "CRIT"
        }
    }

    private func eventTypeIcon(_ eventType: String) -> String {
        switch eventType.lowercased() {
        case let t where t.contains("queue"): return "tray.and.arrow.down"
        case let t where t.contains("model"): return "cpu"
        case let t where t.contains("file"): return "doc"
        case let t where t.contains("command"): return "terminal"
        case let t where t.contains("network"): return "network"
        case let t where t.contains("secret"), let t where t.contains("sensitive"): return "lock.shield"
        default: return "exclamationmark.triangle"
        }
    }
}

// MARK: - NSTableViewDataSource

extension AuditPanel: NSTableViewDataSource {
    func numberOfRows(in tableView: NSTableView) -> Int {
        filteredEvents.count
    }
}

// MARK: - NSTableViewDelegate

extension AuditPanel: NSTableViewDelegate {
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < filteredEvents.count else { return nil }
        let event = filteredEvents[row]

        let cell = NSView()

        // Event type icon
        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: eventTypeIcon(event.eventType),
                             accessibilityDescription: event.eventType)
        icon.contentTintColor = severityColor(event.severity)
        icon.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(icon)

        // Timestamp
        let timeLabel = NSTextField(labelWithString: event.timestamp)
        timeLabel.font = ThaneTheme.uiFont(size: 10)
        timeLabel.textColor = ThaneTheme.tertiaryText
        timeLabel.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(timeLabel)

        // Agent name badge (if present)
        var agentBadge: NSTextField?
        if let agent = event.agentName, !agent.isEmpty {
            let ab = NSTextField(labelWithString: " \(agent) ")
            ab.font = ThaneTheme.uiFont(size: 9)
            ab.textColor = ThaneTheme.accentColor
            ab.wantsLayer = true
            ab.layer?.cornerRadius = 3
            ab.layer?.borderWidth = 1
            ab.layer?.borderColor = ThaneTheme.accentColor.cgColor
            ab.alignment = .center
            ab.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(ab)
            agentBadge = ab
        }

        // [redacted] indicator badge — shown when description or metadata
        // carries the `[REDACTED:` marker stamped on by the Rust redactor.
        var redactedBadge: NSTextField?
        if AuditPanel.eventHasRedaction(event) {
            let rb = NSTextField(labelWithString: " [redacted] ")
            rb.font = ThaneTheme.uiFont(size: 9)
            rb.textColor = ThaneTheme.tertiaryText
            rb.wantsLayer = true
            rb.layer?.cornerRadius = 3
            rb.layer?.borderWidth = 1
            rb.layer?.borderColor = ThaneTheme.tertiaryText.cgColor
            rb.alignment = .center
            rb.toolTip = "Free-form content was scrubbed before this event was stored. " +
                "Adjust under Settings → Audit Redaction Policy."
            rb.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(rb)
            redactedBadge = rb
        }

        // Description (strip literal \n for single-line list display)
        let descText = event.description
            .replacingOccurrences(of: "\\n", with: " ")
            .replacingOccurrences(of: "\n", with: " ")
        let descLabel = NSTextField(labelWithString: descText)
        descLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        descLabel.textColor = ThaneTheme.primaryText
        descLabel.lineBreakMode = .byTruncatingTail
        descLabel.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(descLabel)

        // Severity badge
        let badge = NSTextField(labelWithString: " \(severityLabel(event.severity)) ")
        badge.font = ThaneTheme.uiFont(size: 9)
        badge.textColor = .white
        badge.wantsLayer = true
        badge.layer?.cornerRadius = 4
        badge.layer?.backgroundColor = severityColor(event.severity).cgColor
        badge.alignment = .center
        badge.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(badge)

        var constraints = [
            icon.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 8),
            icon.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 20),
            icon.heightAnchor.constraint(equalToConstant: 20),

            timeLabel.topAnchor.constraint(equalTo: cell.topAnchor, constant: 6),
            timeLabel.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),

            badge.centerYAnchor.constraint(equalTo: timeLabel.centerYAnchor),
            badge.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8),

            descLabel.topAnchor.constraint(equalTo: timeLabel.bottomAnchor, constant: 2),
            descLabel.leadingAnchor.constraint(equalTo: timeLabel.leadingAnchor),
            descLabel.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8),
        ]

        if let ab = agentBadge {
            constraints.append(contentsOf: [
                ab.centerYAnchor.constraint(equalTo: timeLabel.centerYAnchor),
                ab.leadingAnchor.constraint(equalTo: timeLabel.trailingAnchor, constant: 6),
            ])
        }

        if let rb = redactedBadge {
            constraints.append(contentsOf: [
                rb.centerYAnchor.constraint(equalTo: timeLabel.centerYAnchor),
                rb.leadingAnchor.constraint(
                    equalTo: agentBadge?.trailingAnchor ?? timeLabel.trailingAnchor,
                    constant: 6),
            ])
        }

        NSLayoutConstraint.activate(constraints)

        return cell
    }

    /// Whether any free-form field on `event` carries the redaction marker.
    /// Mirrors `event_has_redaction` in `crates/thane-gtk/src/sidebar/audit_panel.rs`.
    static func eventHasRedaction(_ event: AuditEventInfoDTO) -> Bool {
        if event.description.contains("[REDACTED:") { return true }
        return event.metadataJson.contains("[REDACTED:")
    }
}
