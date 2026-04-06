import AppKit

/// Bottom status bar showing agent status, cost, queue indicator, and audit badge.
@MainActor
final class StatusBarView: NSView {

    private let bridge: RustBridge

    private let agentLabel = NSTextField(labelWithString: "Agent: idle")
    private let agentSpinner = NSProgressIndicator()
    private var agentActiveTimestamp: Date?
    private let leaderLabel = NSTextField(labelWithString: "LEADER")
    private let commandBlockLabel = NSTextField(labelWithString: "")
    private let fontSizeLabel = NSTextField(labelWithString: "13pt")
    private let versionLabel = NSTextField(labelWithString: "v0.1.0-beta.17")

    // Clickable status items
    private var costButton: StatusBarButton!
    private var queueButton: StatusBarButton!
    private var plansButton: StatusBarButton!
    private var auditButton: StatusBarButton!

    // Callback for showing panels
    var onShowPanel: ((RightPanelType) -> Void)?

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

    /// Shell PIDs for the active workspace — set by MainWindow before refresh.
    var activeWorkspacePids: [Int32] = []

    func refresh() {
        // Determine agent activity from queue state OR terminal child process inspection
        let queueEntries = bridge.queueList()
        let runningEntries = queueEntries.filter { $0.status == .running }

        // Collect distinct active agent names from per-panel tracking
        let activeAgentNames = Set(bridge.panelAgents.values)
        let hasTerminalAgent = !activeAgentNames.isEmpty
        let isAgentActive = !runningEntries.isEmpty || hasTerminalAgent

        if isAgentActive {
            if activeAgentNames.count > 1 {
                agentLabel.stringValue = "Agents: \(activeAgentNames.sorted().joined(separator: ", "))"
            } else if let name = activeAgentNames.first {
                agentLabel.stringValue = "Agent: \(name)"
            } else {
                agentLabel.stringValue = "Agent: active"
            }
            agentSpinner.isHidden = false
            agentSpinner.startAnimation(nil)

            // Track when agent became active for stalled detection
            if agentActiveTimestamp == nil {
                agentActiveTimestamp = Date()
            }

            // Check for stalled state (active > 60 seconds without update)
            if let activeStart = agentActiveTimestamp,
               Date().timeIntervalSince(activeStart) > 60 {
                agentLabel.textColor = ThaneTheme.warningColor
            } else {
                agentLabel.textColor = ThaneTheme.agentActiveColor
            }
        } else {
            agentLabel.stringValue = "Agent: idle"
            agentLabel.textColor = ThaneTheme.agentInactiveColor
            agentSpinner.isHidden = true
            agentSpinner.stopAnimation(nil)
            agentActiveTimestamp = nil
        }

        // Aggregate cost across all workspaces (deduplicated by CWD)
        let costScope = bridge.configGet(key: "cost-display-scope") ?? "session"
        let useAllTime = costScope == "all-time"
        var seenCwds = Set<String>()
        var totalCost: Double = 0
        for ws in bridge.listWorkspaces() {
            if seenCwds.insert(ws.cwd).inserted {
                let cost = bridge.getProjectCostForCwd(ws.cwd)
                totalCost += useAllTime ? cost.alltimeCostUsd : cost.sessionCostUsd
            }
        }
        // Add queue costs
        let queueCost = queueEntries.reduce(0.0) { $0 + $1.estimatedCostUsd }
        totalCost += queueCost

        // Use token limits (which has OAuth data) for display mode decision.
        let limits = bridge.getTokenLimits()
        let displayMode = limits.displayMode
        let fiveHourUtil = limits.fiveHourUtilization

        if displayMode == "utilization", let pct = fiveHourUtil {
            // Derive cost from monthly plan price × utilization%.
            let monthlyPrice = Self.monthlyPrice(for: limits.planName, bridge: bridge)
            let derivedCost = monthlyPrice.map { $0 * pct / 100.0 }
            let displayCost = derivedCost ?? totalCost
            costButton.title = String(format: "%.0f%% · $%.2f", pct, displayCost)
            costButton.toolTip = String(format: "5h utilization: %.0f%% | ~$%.2f of plan used | $%.2f API equiv.", pct, displayCost, totalCost)
            // Color thresholds.
            if pct >= 85 {
                costButton.contentTintColor = ThaneTheme.errorColor
            } else if pct >= 60 {
                costButton.contentTintColor = ThaneTheme.warningColor
            } else {
                costButton.contentTintColor = nil
            }
        } else {
            costButton.title = totalCost < 0.01 && totalCost > 0 ? "<$0.01" : String(format: "$%.2f", totalCost)
            costButton.toolTip = "Token usage (Ctrl+Shift+U)"
            costButton.contentTintColor = nil
        }

        let activeCount = queueEntries.filter {
            $0.status == .queued || $0.status == .running
        }.count
        queueButton.title = "Queue: \(activeCount)"

        let completedCount = queueEntries.filter {
            $0.status == .completed || $0.status == .failed
        }.count
        plansButton.title = completedCount > 0 ? "Processed (\(completedCount))" : "Processed"

        let auditCount = bridge.auditEventCount
        auditButton.title = auditCount > 0 ? "Audit (\(auditCount))" : "Audit"

        let fontSize = bridge.configFontSize()
        fontSizeLabel.stringValue = "\(Int(fontSize))pt"
    }

    /// Monthly subscription price in USD by plan name.
    ///
    /// Enterprise defaults to $200/seat/month (worst-case estimate matching Max 20x).
    /// Users can override via the `enterprise-monthly-cost` config key.
    private static func monthlyPrice(for planName: String, bridge: RustBridge) -> Double? {
        switch planName.lowercased() {
        case "pro": return 20.0
        case "max (5x)": return 100.0
        case "max (20x)": return 200.0
        case "team": return 30.0
        case "enterprise":
            if let override_ = bridge.configGet(key: "enterprise-monthly-cost"),
               let value = Double(override_), value > 0 {
                return value
            }
            return 200.0  // Default worst-case estimate
        default: return nil  // API/Unknown
        }
    }

    /// Show or hide the leader key indicator.
    private var leaderDismissWork: DispatchWorkItem?

    func showLeaderIndicator() {
        leaderLabel.isHidden = false
        leaderDismissWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.hideLeaderIndicator()
        }
        leaderDismissWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0, execute: work)
    }

    func hideLeaderIndicator() {
        leaderLabel.isHidden = true
        leaderDismissWork?.cancel()
        leaderDismissWork = nil
    }

    /// Update the command block info label (exit code + duration).
    func updateCommandBlock(exitCode: Int?, duration: TimeInterval?) {
        guard let code = exitCode else {
            commandBlockLabel.isHidden = true
            return
        }
        var text = "exit \(code)"
        if let dur = duration {
            text += String(format: ", %.1fs", dur)
        }
        commandBlockLabel.stringValue = text
        commandBlockLabel.isHidden = false
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.statusBarBackground.cgColor

        let stack = NSStackView()
        stack.orientation = .horizontal
        stack.spacing = 16
        stack.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Top divider
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        addSubview(divider)
        NSLayoutConstraint.activate([
            divider.topAnchor.constraint(equalTo: topAnchor),
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: ThaneTheme.dividerThickness),
        ])

        // Leader key indicator (hidden by default)
        leaderLabel.font = ThaneTheme.boldLabelFont(size: ThaneTheme.smallFontSize)
        leaderLabel.textColor = ThaneTheme.accentColor
        leaderLabel.wantsLayer = true
        leaderLabel.layer?.backgroundColor = ThaneTheme.accentColor.withAlphaComponent(0.15).cgColor
        leaderLabel.layer?.cornerRadius = 3
        leaderLabel.isHidden = true
        leaderLabel.setAccessibilityLabel("Leader key mode active")
        stack.addArrangedSubview(leaderLabel)

        // Agent status (not clickable)
        agentLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        agentLabel.textColor = ThaneTheme.agentInactiveColor
        agentLabel.setAccessibilityLabel("Agent status")
        stack.addArrangedSubview(agentLabel)

        // Agent spinner (visible when active)
        agentSpinner.style = .spinning
        agentSpinner.controlSize = .small
        agentSpinner.isDisplayedWhenStopped = false
        agentSpinner.isHidden = true
        agentSpinner.translatesAutoresizingMaskIntoConstraints = false
        agentSpinner.widthAnchor.constraint(equalToConstant: 16).isActive = true
        agentSpinner.heightAnchor.constraint(equalToConstant: 16).isActive = true
        stack.addArrangedSubview(agentSpinner)

        // Command block info (exit code + duration)
        commandBlockLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        commandBlockLabel.textColor = ThaneTheme.secondaryText
        commandBlockLabel.isHidden = true
        stack.addArrangedSubview(commandBlockLabel)

        stack.addArrangedSubview(makeSeparator())

        // Cost → Token panel
        costButton = StatusBarButton(title: "$0.00", color: ThaneTheme.costColor) { [weak self] in
            self?.onShowPanel?(.tokenUsage)
        }
        costButton.setAccessibilityLabel("Session cost")
        costButton.setAccessibilityHelp("Click to show Claude token usage panel")
        stack.addArrangedSubview(costButton)

        stack.addArrangedSubview(makeSeparator())

        // Queue → Agent Queue panel
        queueButton = StatusBarButton(title: "Queue: 0", color: ThaneTheme.secondaryText) { [weak self] in
            self?.onShowPanel?(.agentQueue)
        }
        queueButton.setAccessibilityLabel("CC agent queue status")
        queueButton.setAccessibilityHelp("Click to show CC agent queue panel")
        stack.addArrangedSubview(queueButton)

        stack.addArrangedSubview(makeSeparator())

        // Processed → Plans panel
        plansButton = StatusBarButton(title: "Processed", color: ThaneTheme.secondaryText) { [weak self] in
            self?.onShowPanel?(.plans)
        }
        plansButton.setAccessibilityLabel("Processed plans")
        plansButton.setAccessibilityHelp("Click to show processed plans panel")
        stack.addArrangedSubview(plansButton)

        // Spacer
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        stack.addArrangedSubview(spacer)

        // Audit → Audit panel
        auditButton = StatusBarButton(title: "Audit", color: ThaneTheme.secondaryText) { [weak self] in
            self?.onShowPanel?(.audit)
        }
        auditButton.setAccessibilityLabel("Audit log")
        auditButton.setAccessibilityHelp("Click to show audit log panel")
        stack.addArrangedSubview(auditButton)

        stack.addArrangedSubview(makeSeparator())

        // Font size (not clickable)
        fontSizeLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        fontSizeLabel.textColor = ThaneTheme.tertiaryText
        fontSizeLabel.setAccessibilityLabel("Terminal font size")
        stack.addArrangedSubview(fontSizeLabel)

        stack.addArrangedSubview(makeSeparator())

        // Version label
        versionLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        versionLabel.textColor = ThaneTheme.tertiaryText
        versionLabel.setAccessibilityLabel("Application version")
        stack.addArrangedSubview(versionLabel)
    }

    /// Create a 1px vertical separator for the status bar.
    private func makeSeparator() -> NSView {
        let sep = NSView()
        sep.wantsLayer = true
        sep.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        sep.translatesAutoresizingMaskIntoConstraints = false
        sep.widthAnchor.constraint(equalToConstant: ThaneTheme.dividerThickness).isActive = true
        sep.heightAnchor.constraint(equalToConstant: 16).isActive = true
        return sep
    }
}

// MARK: - StatusBarButton

/// A simple clickable text button for the status bar.
/// Uses NSButton styled to look like a label.
@MainActor
private final class StatusBarButton: NSButton {
    private var clickAction: (() -> Void)?

    convenience init(title: String, color: NSColor, action: @escaping () -> Void) {
        self.init(frame: .zero)
        self.clickAction = action
        self.title = title
        self.bezelStyle = .accessoryBarAction
        self.setButtonType(.momentaryPushIn)
        self.isBordered = false
        self.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        self.contentTintColor = color
        self.target = self
        self.action = #selector(handleClick)
    }

    @objc private func handleClick() {
        clickAction?()
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }
}
