import AppKit

/// Individual workspace row displayed in the sidebar table view.
///
/// Shows: workspace title, per-panel CWD + git branch, unread badge, close button.
@MainActor
final class WorkspaceRowView: NSTableCellView {

    private let titleLabel = NSTextField(labelWithString: "")
    private let badgeLabel = NSTextField(labelWithString: "")
    private let sandboxBadge = NSTextField(labelWithString: "sandbox")
    private let closeButton = NSButton()
    private let panelStack = NSStackView()
    private let agentLabel = NSTextField(labelWithString: "")
    private let costLabel = NSTextField(labelWithString: "")
    private let promptLabel = NSTextField(labelWithString: "")
    private let notifPreviewLabel = NSTextField(labelWithString: "")
    private let portStack = NSStackView()

    /// Called when the close button is clicked
    var onClose: (() -> Void)?
    /// Called when a port badge is clicked (port, shiftHeld)
    var onPortClick: ((UInt16, Bool) -> Void)?

    // MARK: - Init

    init(workspace: WorkspaceInfoDTO, isActive: Bool, panelLocations: [PanelLocationInfo],
         isSandboxed: Bool = false, ports: [UInt16] = [],
         agentStatus: String? = nil, cost: Double = 0,
         lastPrompt: String? = nil, notificationPreview: String? = nil) {
        super.init(frame: .zero)
        setupViews()
        configure(with: workspace, isActive: isActive, panelLocations: panelLocations,
                  isSandboxed: isSandboxed, ports: ports,
                  agentStatus: agentStatus, cost: cost,
                  lastPrompt: lastPrompt, notificationPreview: notificationPreview)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Setup

    private func setupViews() {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 2
        stack.edgeInsets = NSEdgeInsets(top: 6, left: 12, bottom: 6, right: 12)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Title row with badge and close button
        let titleRow = NSStackView()
        titleRow.orientation = .horizontal
        titleRow.spacing = 6

        titleLabel.font = ThaneTheme.boldLabelFont(size: ThaneTheme.uiFontSize)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.lineBreakMode = .byTruncatingTail

        badgeLabel.font = ThaneTheme.uiFont(size: 10)
        badgeLabel.textColor = .white
        badgeLabel.wantsLayer = true
        badgeLabel.layer?.cornerRadius = 8
        badgeLabel.layer?.backgroundColor = ThaneTheme.badgeColor.cgColor
        badgeLabel.alignment = .center

        // Close button (X)
        closeButton.bezelStyle = .recessed
        closeButton.isBordered = false
        closeButton.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close workspace")
        closeButton.contentTintColor = ThaneTheme.tertiaryText
        closeButton.toolTip = "Close workspace"
        closeButton.target = self
        closeButton.action = #selector(closeClicked)
        closeButton.widthAnchor.constraint(equalToConstant: 16).isActive = true
        closeButton.heightAnchor.constraint(equalToConstant: 16).isActive = true
        closeButton.isHidden = true

        // Sandbox badge styling
        sandboxBadge.font = ThaneTheme.uiFont(size: 9)
        sandboxBadge.textColor = ThaneTheme.warningColor
        sandboxBadge.wantsLayer = true
        sandboxBadge.layer?.cornerRadius = 4
        sandboxBadge.layer?.backgroundColor = ThaneTheme.warningColor.withAlphaComponent(0.12).cgColor
        sandboxBadge.isHidden = true

        titleRow.addArrangedSubview(titleLabel)
        titleRow.addArrangedSubview(sandboxBadge)
        titleRow.addArrangedSubview(badgeLabel)
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        titleRow.addArrangedSubview(spacer)
        titleRow.addArrangedSubview(closeButton)

        stack.addArrangedSubview(titleRow)
        titleRow.translatesAutoresizingMaskIntoConstraints = false
        titleRow.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true

        // Panel locations stack (per-panel CWD + git info)
        panelStack.orientation = .vertical
        panelStack.alignment = .leading
        panelStack.spacing = 1
        stack.addArrangedSubview(panelStack)
        panelStack.translatesAutoresizingMaskIntoConstraints = false
        panelStack.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true

        // Agent status + cost row
        let infoRow = NSStackView()
        infoRow.orientation = .horizontal
        infoRow.spacing = 8

        agentLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        agentLabel.isHidden = true
        infoRow.addArrangedSubview(agentLabel)

        costLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        costLabel.textColor = ThaneTheme.costColor
        costLabel.isHidden = true
        infoRow.addArrangedSubview(costLabel)

        stack.addArrangedSubview(infoRow)
        infoRow.translatesAutoresizingMaskIntoConstraints = false
        infoRow.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true

        // Last prompt preview
        promptLabel.font = {
            let desc = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize).fontDescriptor
            let italicDesc = desc.withSymbolicTraits(.italic)
            return NSFont(descriptor: italicDesc, size: ThaneTheme.smallFontSize)
                ?? ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        }()
        promptLabel.textColor = ThaneTheme.accentColor
        promptLabel.lineBreakMode = .byTruncatingTail
        promptLabel.isHidden = true
        stack.addArrangedSubview(promptLabel)
        promptLabel.translatesAutoresizingMaskIntoConstraints = false
        promptLabel.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true

        // Notification preview
        notifPreviewLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        notifPreviewLabel.textColor = ThaneTheme.tertiaryText
        notifPreviewLabel.lineBreakMode = .byTruncatingTail
        notifPreviewLabel.isHidden = true
        stack.addArrangedSubview(notifPreviewLabel)
        notifPreviewLabel.translatesAutoresizingMaskIntoConstraints = false
        notifPreviewLabel.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true

        // Port badges stack
        portStack.orientation = .horizontal
        portStack.spacing = 4
        stack.addArrangedSubview(portStack)
        portStack.translatesAutoresizingMaskIntoConstraints = false
        portStack.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true

        // Track mouse for hover effect
        let area = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
    }

    // MARK: - Mouse hover

    override func mouseEntered(with event: NSEvent) {
        closeButton.isHidden = false
    }

    override func mouseExited(with event: NSEvent) {
        closeButton.isHidden = true
    }

    // MARK: - Actions

    @objc private func closeClicked() {
        onClose?()
    }

    // MARK: - Configure

    /// Update the row in-place with new data (avoids recreating the view).
    func update(workspace: WorkspaceInfoDTO, isActive: Bool, panelLocations: [PanelLocationInfo],
                isSandboxed: Bool = false, ports: [UInt16] = [],
                agentStatus: String? = nil, cost: Double = 0,
                lastPrompt: String? = nil, notificationPreview: String? = nil) {
        configure(with: workspace, isActive: isActive, panelLocations: panelLocations,
                  isSandboxed: isSandboxed, ports: ports,
                  agentStatus: agentStatus, cost: cost,
                  lastPrompt: lastPrompt, notificationPreview: notificationPreview)
    }

    private func configure(with workspace: WorkspaceInfoDTO, isActive: Bool, panelLocations: [PanelLocationInfo],
                           isSandboxed: Bool, ports: [UInt16],
                           agentStatus: String? = nil, cost: Double = 0,
                           lastPrompt: String? = nil, notificationPreview: String? = nil) {
        var title = workspace.title
        if let tag = workspace.tag {
            title += " [\(tag)]"
        }
        titleLabel.stringValue = title

        // Sandbox badge
        sandboxBadge.isHidden = !isSandboxed

        // Unread badge
        if workspace.unreadNotifications > 0 {
            badgeLabel.stringValue = " \(workspace.unreadNotifications) "
            badgeLabel.isHidden = false
            // Pulse animation when badge first appears
            addBadgePulseAnimation()
        } else {
            badgeLabel.isHidden = true
        }

        // Per-panel location rows
        panelStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        if panelLocations.isEmpty {
            // Fallback: show workspace-level CWD
            let row = makePanelLocationRow(
                cwd: workspace.cwd,
                gitBranch: nil,
                gitDirty: false
            )
            panelStack.addArrangedSubview(row)
        } else {
            // Deduplicate by CWD — show each unique directory once
            var seenCwds = Set<String>()
            for loc in panelLocations {
                guard seenCwds.insert(loc.cwd).inserted else { continue }
                let row = makePanelLocationRow(
                    cwd: loc.cwd,
                    gitBranch: loc.gitBranch,
                    gitDirty: loc.gitDirty
                )
                panelStack.addArrangedSubview(row)
            }
        }

        // Agent status
        if let status = agentStatus, status != "idle" {
            agentLabel.stringValue = "Agent: \(status)"
            agentLabel.textColor = status == "active" ? ThaneTheme.agentActiveColor : ThaneTheme.warningColor
            agentLabel.isHidden = false
        } else {
            agentLabel.isHidden = true
        }

        // Cost
        if cost > 0 {
            costLabel.stringValue = String(format: "$%.2f", cost)
            costLabel.isHidden = false
        } else {
            costLabel.isHidden = true
        }

        // Last prompt preview
        if let prompt = lastPrompt, !prompt.isEmpty {
            let truncated = prompt.count > 30 ? String(prompt.prefix(30)) + "..." : prompt
            promptLabel.stringValue = truncated
            promptLabel.isHidden = false
        } else {
            promptLabel.isHidden = true
        }

        // Notification preview
        if let preview = notificationPreview, !preview.isEmpty {
            let truncated = preview.count > 30 ? String(preview.prefix(30)) + "..." : preview
            notifPreviewLabel.stringValue = truncated
            notifPreviewLabel.isHidden = false
        } else {
            notifPreviewLabel.isHidden = true
        }

        // Port badges
        portStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if !ports.isEmpty {
            for port in ports.prefix(3) {
                let btn = PortBadgeButton(port: port) { [weak self] port in
                    let shiftHeld = NSEvent.modifierFlags.contains(.shift)
                    self?.onPortClick?(port, shiftHeld)
                }
                portStack.addArrangedSubview(btn)
            }
            if ports.count > 3 {
                let more = NSTextField(labelWithString: "+\(ports.count - 3)")
                more.font = ThaneTheme.uiFont(size: 10)
                more.textColor = ThaneTheme.tertiaryText
                portStack.addArrangedSubview(more)
            }
        }
    }

    /// Add a pulse scale animation to the unread notification badge.
    private func addBadgePulseAnimation() {
        guard let badgeLayer = badgeLabel.layer else { return }
        // Remove any existing pulse animation
        badgeLayer.removeAnimation(forKey: "badgePulse")

        let pulse = CABasicAnimation(keyPath: "transform.scale")
        pulse.fromValue = 1.0
        pulse.toValue = 1.2
        pulse.duration = 0.3
        pulse.autoreverses = true
        pulse.repeatCount = 1
        pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        badgeLayer.add(pulse, forKey: "badgePulse")
    }

    /// Build a single row: [CWD path ... git branch*]
    private func makePanelLocationRow(cwd: String, gitBranch: String?, gitDirty: Bool) -> NSView {
        let row = NSStackView()
        row.orientation = .horizontal
        row.spacing = 6

        // CWD label (abbreviated)
        let homeDir = FileManager.default.homeDirectoryForCurrentUser.path
        let displayCwd = cwd.hasPrefix(homeDir)
            ? "~" + cwd.dropFirst(homeDir.count)
            : cwd
        let cwdLabel = NSTextField(labelWithString: String(displayCwd))
        cwdLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        cwdLabel.textColor = ThaneTheme.secondaryText
        cwdLabel.lineBreakMode = .byTruncatingHead
        cwdLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        row.addArrangedSubview(cwdLabel)

        // Spacer
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        row.addArrangedSubview(spacer)

        // Git info
        if let branch = gitBranch {
            let gitText = gitDirty ? "⎇ \(branch)*" : "⎇ \(branch)"
            let gitLabel = NSTextField(labelWithString: gitText)
            gitLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
            gitLabel.textColor = gitDirty ? ThaneTheme.warningColor : ThaneTheme.tertiaryText
            gitLabel.lineBreakMode = .byTruncatingTail
            gitLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
            row.addArrangedSubview(gitLabel)
        } else {
            // Not a git repo — show warning icon
            let warningLabel = NSTextField(labelWithString: "not tracked")
            warningLabel.font = ThaneTheme.uiFont(size: 10)
            warningLabel.textColor = ThaneTheme.tertiaryText
            warningLabel.toolTip = "Not a git repository — changes are not tracked"
            row.addArrangedSubview(warningLabel)
        }

        return row
    }
}

// MARK: - Port Badge Button

@MainActor
private final class PortBadgeButton: NSButton {
    private let port: UInt16
    private let onClick: (UInt16) -> Void

    init(port: UInt16, onClick: @escaping (UInt16) -> Void) {
        self.port = port
        self.onClick = onClick
        super.init(frame: .zero)

        title = ":\(port)"
        bezelStyle = .inline
        isBordered = false
        wantsLayer = true
        layer?.cornerRadius = 4
        layer?.backgroundColor = ThaneTheme.accentColor.withAlphaComponent(0.12).cgColor
        font = ThaneTheme.uiFont(size: 10)
        contentTintColor = ThaneTheme.accentColor
        toolTip = "Click to open http://localhost:\(port)\nShift+click to open in default browser"
        target = self
        action = #selector(clicked)
        setContentHuggingPriority(.required, for: .horizontal)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    @objc private func clicked() {
        onClick(port)
    }
}
