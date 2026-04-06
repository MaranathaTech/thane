import AppKit

/// Right-side panel showing token cost breakdown per workspace.
///
/// - Per-workspace cost rows
/// - Dropdown to toggle between session and all-time view
/// - Formatted as currency ($X.XX)
@MainActor
final class TokenPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge
    private let scrollView = NSScrollView()
    private let stackView = NSStackView()
    private let scopePopup = NSPopUpButton(frame: .zero, pullsDown: false)

    /// false = session (since app launch), true = all time
    private var showAllTime: Bool = false

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(frame: .zero)
        setupViews()
        reload(force: true)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public

    /// Tracks last full reload to avoid rebuilding every 5 seconds.
    private var lastReloadAt: Date?

    func reload() {
        reload(force: false)
    }

    private func reload(force: Bool) {
        if !force {
            if let last = lastReloadAt, Date().timeIntervalSince(last) < 60 { return }
        }
        lastReloadAt = Date()

        // Re-prompt for keychain consent if not yet granted
        bridge.requestKeychainConsent()

        // Sync scope dropdown with config setting
        let costScope = bridge.configGet(key: "cost-display-scope") ?? "session"
        showAllTime = costScope == "all-time"
        scopePopup.selectItem(at: showAllTime ? 1 : 0)

        // Check for stale credentials after fetching limits
        rebuildContent()

        // If credentials are stale (consecutive auth failures), prompt the user
        if bridge.credentialsStale {
            bridge.promptCredentialsRefresh()
        }
    }

    private func rebuildContent() {
        // Clear existing rows
        stackView.arrangedSubviews.forEach { $0.removeFromSuperview() }

        let workspaces = bridge.listWorkspaces()
        let limits = bridge.getTokenLimits()
        let isUtilizationMode = limits.displayMode == "utilization"

        // In utilization mode, show usage section first.
        if isUtilizationMode {
            addLimitsSection(limits: limits, headerTitle: "Usage")
            addFootnoteRow("Cost estimates below are approximate API-equivalent values. Actual costs may vary based on your plan.")
        }

        // ── Cost by Workspace ──
        let header = makeSectionLabel("Cost by Workspace")
        stackView.addArrangedSubview(header)

        if workspaces.isEmpty {
            let empty = makeLabelRow("No workspaces", value: "")
            stackView.addArrangedSubview(empty)
        }

        // Per-workspace cost from JSONL session files (deduplicated by CWD)
        var seenCwds = Set<String>()
        var costTotal: Double = 0
        var totalInput: UInt64 = 0, totalOutput: UInt64 = 0
        var totalCacheRead: UInt64 = 0, totalCacheWrite: UInt64 = 0

        for ws in workspaces {
            let cost = bridge.getProjectCostForCwd(ws.cwd)
            let displayCost = showAllTime ? cost.alltimeCostUsd : cost.sessionCostUsd
            let input = showAllTime ? cost.alltimeInputTokens : cost.sessionInputTokens
            let output = showAllTime ? cost.alltimeOutputTokens : cost.sessionOutputTokens
            let cacheRead = showAllTime ? cost.alltimeCacheReadTokens : cost.sessionCacheReadTokens
            let cacheWrite = showAllTime ? cost.alltimeCacheWriteTokens : cost.sessionCacheWriteTokens

            let tooltip = costTooltip(input: input, output: output, cacheRead: cacheRead, cacheWrite: cacheWrite, total: displayCost)
            let row = makeLabelRow(ws.title, value: formatCost(displayCost), tooltip: tooltip)
            stackView.addArrangedSubview(row)

            if seenCwds.insert(ws.cwd).inserted {
                costTotal += displayCost
                totalInput += input
                totalOutput += output
                totalCacheRead += cacheRead
                totalCacheWrite += cacheWrite
            }
        }

        // Divider
        stackView.addArrangedSubview(makeDivider())

        // Summary
        let summaryHeader = makeSectionLabel("Summary")
        stackView.addArrangedSubview(summaryHeader)

        let costLabel = showAllTime ? "All-Time Cost" : "Session Cost"
        let summaryTooltip = costTooltip(input: totalInput, output: totalOutput, cacheRead: totalCacheRead, cacheWrite: totalCacheWrite, total: costTotal)
        let costRow = makeLabelRow(costLabel, value: "~\(formatCost(costTotal))", tooltip: summaryTooltip)
        stackView.addArrangedSubview(costRow)

        // Token counts
        if totalInput > 0 || totalOutput > 0 {
            stackView.addArrangedSubview(makeDivider())

            let tokenHeader = makeSectionLabel("Token Usage")
            stackView.addArrangedSubview(tokenHeader)

            let inputRow = makeLabelRow("Input Tokens", value: formatTokenCount(totalInput))
            stackView.addArrangedSubview(inputRow)

            let outputRow = makeLabelRow("Output Tokens", value: formatTokenCount(totalOutput))
            stackView.addArrangedSubview(outputRow)

            if totalCacheRead > 0 || totalCacheWrite > 0 {
                let cacheReadRow = makeLabelRow("Cache Read", value: formatTokenCount(totalCacheRead))
                stackView.addArrangedSubview(cacheReadRow)

                let cacheWriteRow = makeLabelRow("Cache Write", value: formatTokenCount(totalCacheWrite))
                stackView.addArrangedSubview(cacheWriteRow)
            }
        }

        // In dollar mode, show limits section at the bottom (original position).
        if !isUtilizationMode {
            addLimitsSection(limits: limits, headerTitle: "Token Limits")
        }

        // Always show cost disclaimer at the bottom.
        addFootnoteRow("Costs are estimates based on public API pricing and may not reflect your actual bill.")
    }

    /// Build the token limits section (progress bars, plan info, caps).
    private func addLimitsSection(limits: TokenLimitsDTO, headerTitle: String) {
        stackView.addArrangedSubview(makeDivider())

        let limitsHeader = makeSectionLabel(headerTitle)
        stackView.addArrangedSubview(limitsHeader)

        let planRow = makeLabelRow("Plan", value: limits.planName)
        stackView.addArrangedSubview(planRow)

        if let util = limits.fiveHourUtilization {
            let pct = min(util, 100.0)
            let resetText = formatResetCountdown(limits.fiveHourResetsAt)
            let label = makeLabelRow("5-Hour Window", value: String(format: "%.0f%% used%@", pct, resetText))
            stackView.addArrangedSubview(label)

            let bar = makeProgressBar(current: pct, max: 100)
            stackView.addArrangedSubview(bar)
        }

        if let util = limits.sevenDayUtilization {
            let pct = min(util, 100.0)
            let resetText = formatResetCountdown(limits.sevenDayResetsAt)
            let label = makeLabelRow("Weekly Cap", value: String(format: "%.0f%% used%@", pct, resetText))
            stackView.addArrangedSubview(label)

            let bar = makeProgressBar(current: pct, max: 100)
            stackView.addArrangedSubview(bar)
        }

        if limits.fiveHourUtilization == nil && limits.sevenDayUtilization == nil {
            if !limits.hasCaps {
                let noCapsLabel = makeLabel("No usage caps (Enterprise/API)")
                stackView.addArrangedSubview(noCapsLabel)
            } else if limits.planName == "Unknown" {
                let noDataLabel = makeLabel("Could not read Claude Code credentials")
                stackView.addArrangedSubview(noDataLabel)
            } else {
                let noDataLabel = makeLabel("Usage data temporarily unavailable")
                stackView.addArrangedSubview(noDataLabel)
            }
        }

        // Team pool footnote.
        if limits.planName == "Team" {
            addFootnoteRow("Usage reflects your allocation within the team pool.")
        }
    }

    /// Add a small hint-style footnote row.
    private func addFootnoteRow(_ text: String) {
        let wrapper = NSView()
        wrapper.translatesAutoresizingMaskIntoConstraints = false

        let label = NSTextField(wrappingLabelWithString: text)
        label.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize - 1)
        label.textColor = ThaneTheme.tertiaryText
        label.isEditable = false
        label.isBordered = false
        label.backgroundColor = .clear
        label.translatesAutoresizingMaskIntoConstraints = false
        wrapper.addSubview(label)

        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: wrapper.topAnchor, constant: 4),
            label.bottomAnchor.constraint(equalTo: wrapper.bottomAnchor, constant: -4),
            label.leadingAnchor.constraint(equalTo: wrapper.leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(equalTo: wrapper.trailingAnchor, constant: -12),
        ])

        stackView.addArrangedSubview(wrapper)
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        // Title
        let titleLabel = NSTextField(labelWithString: "CC Token Usage")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(titleLabel)

        // Scope dropdown
        scopePopup.addItems(withTitles: ["Session", "All Time"])
        scopePopup.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        scopePopup.translatesAutoresizingMaskIntoConstraints = false
        scopePopup.target = self
        scopePopup.action = #selector(scopeChanged)
        scopePopup.isBordered = true
        (scopePopup.cell as? NSPopUpButtonCell)?.bezelStyle = .roundRect
        addSubview(scopePopup)

        // Scrollable content
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        addSubview(scrollView)

        stackView.orientation = .vertical
        stackView.spacing = 4
        stackView.alignment = .leading
        stackView.translatesAutoresizingMaskIntoConstraints = false

        let flippedClip = FlippedClipView()
        scrollView.contentView = flippedClip
        scrollView.documentView = stackView
        let clipView = flippedClip

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),

            scopePopup.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            scopePopup.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),

            scrollView.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 8),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),

            stackView.topAnchor.constraint(equalTo: clipView.topAnchor),
            stackView.leadingAnchor.constraint(equalTo: clipView.leadingAnchor),
            stackView.trailingAnchor.constraint(equalTo: clipView.trailingAnchor),
            stackView.widthAnchor.constraint(equalTo: clipView.widthAnchor),
        ])
    }

    @objc private func scopeChanged() {
        showAllTime = scopePopup.indexOfSelectedItem == 1
        bridge.configSet(key: "cost-display-scope", value: showAllTime ? "all-time" : "session")
        rebuildContent()
    }

    // MARK: - Helpers

    private func makeDivider() -> NSView {
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return divider
    }

    private func makeSectionLabel(_ text: String) -> NSView {
        let label = NSTextField(labelWithString: text)
        label.font = ThaneTheme.boldLabelFont(size: 11)
        label.textColor = ThaneTheme.secondaryText
        label.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(label)
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            label.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -4),
            container.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
        ])
        return container
    }

    private func makeLabelRow(_ title: String, value: String, tooltip: String? = nil) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        if let tooltip { container.toolTip = tooltip }

        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(titleLabel)

        let valueLabel = NSTextField(labelWithString: value)
        valueLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        valueLabel.textColor = ThaneTheme.costColor
        valueLabel.alignment = .right
        valueLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(valueLabel)

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: container.topAnchor, constant: 4),
            titleLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            titleLabel.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -4),

            valueLabel.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            valueLabel.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            valueLabel.leadingAnchor.constraint(greaterThanOrEqualTo: titleLabel.trailingAnchor, constant: 8),

            container.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
        ])

        return container
    }

    /// Build a tooltip showing how cost was estimated from token counts.
    private func costTooltip(input: UInt64, output: UInt64, cacheRead: UInt64, cacheWrite: UInt64, total: Double) -> String {
        guard input + output + cacheRead + cacheWrite > 0 else {
            return "No token usage recorded"
        }
        var lines: [String] = ["Estimated cost breakdown (Sonnet rates):"]
        if input > 0 {
            let cost = Double(input) * 3.0 / 1_000_000
            lines.append("  Input: \(formatTokenCount(input)) tokens \u{00d7} $3.00/M = \(formatCost(cost))")
        }
        if output > 0 {
            let cost = Double(output) * 15.0 / 1_000_000
            lines.append("  Output: \(formatTokenCount(output)) tokens \u{00d7} $15.00/M = \(formatCost(cost))")
        }
        if cacheRead > 0 {
            let cost = Double(cacheRead) * 0.3 / 1_000_000
            lines.append("  Cache read: \(formatTokenCount(cacheRead)) tokens \u{00d7} $0.30/M = \(formatCost(cost))")
        }
        if cacheWrite > 0 {
            let cost = Double(cacheWrite) * 3.75 / 1_000_000
            lines.append("  Cache write: \(formatTokenCount(cacheWrite)) tokens \u{00d7} $3.75/M = \(formatCost(cost))")
        }
        lines.append("")
        lines.append("Costs are estimates based on public API pricing.")
        lines.append("Actual costs may vary based on your plan.")
        lines.append("Opus usage is priced at higher rates when detected.")
        return lines.joined(separator: "\n")
    }

    private func makeLabel(_ text: String) -> NSView {
        let label = NSTextField(labelWithString: text)
        label.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        label.textColor = ThaneTheme.secondaryText
        label.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(label)
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: container.topAnchor, constant: 4),
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            label.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -4),
            container.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
        ])
        return container
    }

    private func makeProgressBar(current: Double, max limit: Double) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        let progress = NSProgressIndicator()
        progress.style = .bar
        progress.isIndeterminate = false
        progress.minValue = 0
        progress.maxValue = limit
        progress.doubleValue = min(current, limit)
        progress.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(progress)

        NSLayoutConstraint.activate([
            progress.topAnchor.constraint(equalTo: container.topAnchor, constant: 2),
            progress.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            progress.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            progress.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -4),
            progress.heightAnchor.constraint(equalToConstant: 8),
            container.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
        ])

        return container
    }

    private func formatResetCountdown(_ isoString: String?) -> String {
        guard let str = isoString else { return "" }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = formatter.date(from: str) ?? ISO8601DateFormatter().date(from: str) else { return "" }
        let interval = date.timeIntervalSinceNow
        if interval <= 0 { return " — resetting" }
        let hours = Int(interval) / 3600
        let minutes = (Int(interval) % 3600) / 60
        if hours > 0 {
            return " — resets in \(hours)h \(minutes)m"
        }
        return " — resets in \(minutes)m"
    }

    private func formatCost(_ cost: Double) -> String {
        if cost > 0 && cost < 0.01 { return "<$0.01" }
        return String(format: "$%.2f", cost)
    }

    private func formatTokenCount(_ count: UInt64) -> String {
        if count >= 1_000_000 {
            return String(format: "%.1fM", Double(count) / 1_000_000)
        } else if count >= 1_000 {
            return String(format: "%.1fK", Double(count) / 1_000)
        }
        return "\(count)"
    }

    private func formatNumber(_ n: UInt64) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return formatter.string(from: NSNumber(value: n)) ?? "\(n)"
    }
}
