import AppKit

/// Right-side panel for the agent queue.
///
/// Matches the Linux GTK `AgentQueuePanel` layout:
/// - Header with title, status count, "+" toggle
/// - Hint banner with left accent border (visible when empty)
/// - Pause banner with warning styling
/// - Scrollable list of queue entries (2-row per entry, GTK style)
/// - Process bar with Process All / Process Next buttons
@MainActor
final class AgentQueuePanel: NSView, ReloadablePanel {

    private let bridge: RustBridge
    private let listStack = NSStackView()
    private let scrollView = NSScrollView()
    private let submitField = NSTextField()
    private let submitButton: NSButton
    private let pauseBanner = NSView()
    private let hintBanner = NSView()
    private let emptyBox = NSView()
    private let processBar = NSView()
    private let statusLabel = NSTextField(labelWithString: "No tasks")
    private weak var processDivider: NSView?
    private weak var submitRow: NSStackView?
    private var entries: [QueueEntryInfoDTO] = []

    // Sandbox controls
    private let sandboxSwitch = NSSwitch()
    private let sandboxEnforcementPopup = NSPopUpButton()
    private let sandboxNetworkSwitch = NSSwitch()

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        self.submitButton = NSButton(title: "Submit", target: nil, action: nil)
        super.init(frame: .zero)
        submitButton.target = self
        submitButton.action = #selector(submitClicked)
        setupViews()
        reload()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // MARK: - Public

    func reload() {
        entries = bridge.queueList()
        updateSandboxControls()

        // Rebuild list
        listStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        let queued = entries.filter { $0.status == .queued }.count
        let running = entries.filter { $0.status == .running }.count
        let hasPaused = entries.contains { $0.status == .pausedTokenLimit || $0.status == .pausedByUser }

        // Status label
        if hasPaused {
            statusLabel.stringValue = "\(queued) queued (paused)"
        } else if running > 0 {
            statusLabel.stringValue = "\(queued) queued, \(running) running"
        } else if queued > 0 {
            statusLabel.stringValue = "\(queued) queued"
        } else {
            statusLabel.stringValue = "No tasks"
        }

        pauseBanner.isHidden = !hasPaused
        hintBanner.isHidden = !entries.isEmpty

        // Process bar
        let showProcess = queued > 0 && running == 0 && !hasPaused
        processBar.isHidden = !showProcess
        processDivider?.isHidden = !showProcess

        if entries.isEmpty {
            // Show empty state inside list
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

        // ── Header ──
        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)

        let titleLabel = NSTextField(labelWithString: "Agent Queue")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(titleLabel)

        statusLabel.font = ThaneTheme.uiFont(size: 12)
        statusLabel.textColor = ThaneTheme.tertiaryText
        statusLabel.alignment = .right
        statusLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(statusLabel)

        let addBtn = NSButton(image: NSImage(systemSymbolName: "plus",
                                              accessibilityDescription: "Add task")!,
                               target: self, action: #selector(toggleSubmitBar))
        addBtn.bezelStyle = .recessed
        addBtn.isBordered = false
        addBtn.contentTintColor = ThaneTheme.secondaryText
        addBtn.toolTip = "Add task manually"
        addBtn.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(addBtn)

        NSLayoutConstraint.activate([
            titleLabel.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            titleLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            statusLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            statusLabel.leadingAnchor.constraint(greaterThanOrEqualTo: titleLabel.trailingAnchor, constant: 8),
            statusLabel.trailingAnchor.constraint(equalTo: addBtn.leadingAnchor, constant: -4),
            addBtn.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -12),
            addBtn.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            addBtn.widthAnchor.constraint(equalToConstant: 20),
            addBtn.heightAnchor.constraint(equalToConstant: 20),
        ])

        let headerSep = ViewFactories.makeDivider()
        addSubview(headerSep)

        // ── Top area stack (submit + banners) — hidden items collapse automatically ──
        let topStack = NSStackView()
        topStack.orientation = .vertical
        topStack.alignment = .leading
        topStack.spacing = 8
        topStack.edgeInsets = NSEdgeInsets(top: 8, left: 12, bottom: 4, right: 12)
        topStack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(topStack)

        // Submit row wrapper
        let submitRow = NSStackView()
        submitRow.orientation = .horizontal
        submitRow.spacing = 4
        submitRow.translatesAutoresizingMaskIntoConstraints = false
        submitField.placeholderString = "Enter task content..."
        submitField.font = ThaneTheme.uiFont(size: ThaneTheme.uiFontSize)
        submitField.delegate = self
        submitRow.addArrangedSubview(submitField)
        submitButton.bezelStyle = .rounded
        submitButton.controlSize = .small
        submitButton.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        submitButton.widthAnchor.constraint(equalToConstant: 60).isActive = true
        submitRow.addArrangedSubview(submitButton)
        submitRow.isHidden = true
        self.submitRow = submitRow
        topStack.addArrangedSubview(submitRow)
        submitRow.widthAnchor.constraint(equalTo: topStack.widthAnchor, constant: -24).isActive = true

        // ── Sandbox section ──
        let sandboxSection = NSView()
        sandboxSection.translatesAutoresizingMaskIntoConstraints = false

        let sandboxHeader = NSTextField(labelWithString: "Sandbox")
        sandboxHeader.font = ThaneTheme.boldLabelFont(size: 11)
        sandboxHeader.textColor = ThaneTheme.secondaryText
        sandboxHeader.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(sandboxHeader)

        // Enable row
        let sandboxLabel = NSTextField(labelWithString: "Enable")
        sandboxLabel.font = ThaneTheme.uiFont(size: 12)
        sandboxLabel.textColor = ThaneTheme.primaryText
        sandboxLabel.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(sandboxLabel)

        sandboxSwitch.controlSize = .mini
        sandboxSwitch.target = self
        sandboxSwitch.action = #selector(sandboxToggled)
        sandboxSwitch.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(sandboxSwitch)

        // Enforcement row
        let enforceLabel = NSTextField(labelWithString: "Enforcement")
        enforceLabel.font = ThaneTheme.uiFont(size: 12)
        enforceLabel.textColor = ThaneTheme.primaryText
        enforceLabel.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(enforceLabel)

        sandboxEnforcementPopup.addItems(withTitles: ["Permissive", "Enforcing", "Strict"])
        sandboxEnforcementPopup.selectItem(at: 1)
        sandboxEnforcementPopup.controlSize = .small
        sandboxEnforcementPopup.font = ThaneTheme.uiFont(size: 11)
        sandboxEnforcementPopup.target = self
        sandboxEnforcementPopup.action = #selector(sandboxEnforcementChanged)
        sandboxEnforcementPopup.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(sandboxEnforcementPopup)

        // Network row
        let netLabel = NSTextField(labelWithString: "Network")
        netLabel.font = ThaneTheme.uiFont(size: 12)
        netLabel.textColor = ThaneTheme.primaryText
        netLabel.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(netLabel)

        sandboxNetworkSwitch.controlSize = .mini
        sandboxNetworkSwitch.state = .on
        sandboxNetworkSwitch.target = self
        sandboxNetworkSwitch.action = #selector(sandboxNetworkToggled)
        sandboxNetworkSwitch.translatesAutoresizingMaskIntoConstraints = false
        sandboxSection.addSubview(sandboxNetworkSwitch)

        NSLayoutConstraint.activate([
            sandboxHeader.topAnchor.constraint(equalTo: sandboxSection.topAnchor),
            sandboxHeader.leadingAnchor.constraint(equalTo: sandboxSection.leadingAnchor),

            sandboxLabel.topAnchor.constraint(equalTo: sandboxHeader.bottomAnchor, constant: 6),
            sandboxLabel.leadingAnchor.constraint(equalTo: sandboxSection.leadingAnchor),
            sandboxSwitch.centerYAnchor.constraint(equalTo: sandboxLabel.centerYAnchor),
            sandboxSwitch.trailingAnchor.constraint(equalTo: sandboxSection.trailingAnchor),

            enforceLabel.topAnchor.constraint(equalTo: sandboxLabel.bottomAnchor, constant: 6),
            enforceLabel.leadingAnchor.constraint(equalTo: sandboxSection.leadingAnchor),
            sandboxEnforcementPopup.centerYAnchor.constraint(equalTo: enforceLabel.centerYAnchor),
            sandboxEnforcementPopup.trailingAnchor.constraint(equalTo: sandboxSection.trailingAnchor),

            netLabel.topAnchor.constraint(equalTo: enforceLabel.bottomAnchor, constant: 6),
            netLabel.leadingAnchor.constraint(equalTo: sandboxSection.leadingAnchor),
            sandboxNetworkSwitch.centerYAnchor.constraint(equalTo: netLabel.centerYAnchor),
            sandboxNetworkSwitch.trailingAnchor.constraint(equalTo: sandboxSection.trailingAnchor),
            netLabel.bottomAnchor.constraint(equalTo: sandboxSection.bottomAnchor),
        ])

        topStack.addArrangedSubview(sandboxSection)
        sandboxSection.widthAnchor.constraint(equalTo: topStack.widthAnchor, constant: -24).isActive = true

        let sandboxSep = ViewFactories.makeDivider()
        topStack.addArrangedSubview(sandboxSep)
        sandboxSep.widthAnchor.constraint(equalTo: topStack.widthAnchor, constant: -24).isActive = true

        // Pause banner
        setupPauseBanner()
        topStack.addArrangedSubview(pauseBanner)
        pauseBanner.widthAnchor.constraint(equalTo: topStack.widthAnchor, constant: -24).isActive = true

        // Hint banner
        setupHintBanner()
        topStack.addArrangedSubview(hintBanner)
        hintBanner.widthAnchor.constraint(equalTo: topStack.widthAnchor, constant: -24).isActive = true

        // ── Empty state ──
        setupEmptyBox()

        // ── Scrollable list ──
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
            // Pin document width to clip view so rows fill the full width
            listContent.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
        ])

        // ── Process bar ──
        let pDivider = ViewFactories.makeDivider()
        pDivider.isHidden = true
        addSubview(pDivider)
        self.processDivider = pDivider

        processBar.translatesAutoresizingMaskIntoConstraints = false
        processBar.isHidden = true
        addSubview(processBar)

        let processAllBtn = NSButton(title: "Process All", target: self, action: #selector(processAllClicked))
        processAllBtn.bezelStyle = .rounded
        processAllBtn.controlSize = .small
        processAllBtn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        processAllBtn.translatesAutoresizingMaskIntoConstraints = false
        processBar.addSubview(processAllBtn)

        let processNextBtn = NSButton(title: "Process Next", target: self, action: #selector(processNextClicked))
        processNextBtn.bezelStyle = .recessed
        processNextBtn.controlSize = .small
        processNextBtn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        processNextBtn.translatesAutoresizingMaskIntoConstraints = false
        processBar.addSubview(processNextBtn)

        NSLayoutConstraint.activate([
            processAllBtn.leadingAnchor.constraint(equalTo: processBar.leadingAnchor, constant: 12),
            processAllBtn.centerYAnchor.constraint(equalTo: processBar.centerYAnchor),
            processNextBtn.leadingAnchor.constraint(equalTo: processAllBtn.trailingAnchor, constant: 8),
            processNextBtn.centerYAnchor.constraint(equalTo: processBar.centerYAnchor),
        ])

        // ── Main constraints ──
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 40),

            headerSep.topAnchor.constraint(equalTo: header.bottomAnchor),
            headerSep.leadingAnchor.constraint(equalTo: leadingAnchor),
            headerSep.trailingAnchor.constraint(equalTo: trailingAnchor),

            topStack.topAnchor.constraint(equalTo: headerSep.bottomAnchor),
            topStack.leadingAnchor.constraint(equalTo: leadingAnchor),
            topStack.trailingAnchor.constraint(equalTo: trailingAnchor),

            scrollView.topAnchor.constraint(equalTo: topStack.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: pDivider.topAnchor),

            pDivider.leadingAnchor.constraint(equalTo: leadingAnchor),
            pDivider.trailingAnchor.constraint(equalTo: trailingAnchor),
            pDivider.bottomAnchor.constraint(equalTo: processBar.topAnchor),

            processBar.leadingAnchor.constraint(equalTo: leadingAnchor),
            processBar.trailingAnchor.constraint(equalTo: trailingAnchor),
            processBar.bottomAnchor.constraint(equalTo: bottomAnchor),
            processBar.heightAnchor.constraint(equalToConstant: 40),
        ])
    }

    private func setupPauseBanner() {
        pauseBanner.wantsLayer = true
        pauseBanner.layer?.backgroundColor = ThaneTheme.warningColor.withAlphaComponent(0.08).cgColor
        pauseBanner.layer?.cornerRadius = 4
        pauseBanner.translatesAutoresizingMaskIntoConstraints = false
        pauseBanner.isHidden = true
        // Added to topStack by setupViews, not directly to self

        let accent = NSView()
        accent.wantsLayer = true
        accent.layer?.backgroundColor = ThaneTheme.warningColor.cgColor
        accent.layer?.cornerRadius = 1.5
        accent.translatesAutoresizingMaskIntoConstraints = false
        pauseBanner.addSubview(accent)

        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: "exclamationmark.triangle.fill", accessibilityDescription: nil)
        icon.contentTintColor = ThaneTheme.warningColor
        icon.translatesAutoresizingMaskIntoConstraints = false
        pauseBanner.addSubview(icon)

        let label = NSTextField(labelWithString: "Queue paused \u{2014} token limit reached")
        label.font = ThaneTheme.uiFont(size: 13)
        label.textColor = ThaneTheme.warningColor
        label.translatesAutoresizingMaskIntoConstraints = false
        pauseBanner.addSubview(label)

        NSLayoutConstraint.activate([
            pauseBanner.heightAnchor.constraint(equalToConstant: 36),
            accent.leadingAnchor.constraint(equalTo: pauseBanner.leadingAnchor),
            accent.topAnchor.constraint(equalTo: pauseBanner.topAnchor),
            accent.bottomAnchor.constraint(equalTo: pauseBanner.bottomAnchor),
            accent.widthAnchor.constraint(equalToConstant: 3),
            icon.leadingAnchor.constraint(equalTo: accent.trailingAnchor, constant: 9),
            icon.centerYAnchor.constraint(equalTo: pauseBanner.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 16),
            icon.heightAnchor.constraint(equalToConstant: 16),
            label.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 6),
            label.centerYAnchor.constraint(equalTo: pauseBanner.centerYAnchor),
        ])
    }

    private func setupHintBanner() {
        hintBanner.wantsLayer = true
        hintBanner.layer?.backgroundColor = ThaneTheme.accentColor.withAlphaComponent(0.06).cgColor
        hintBanner.layer?.cornerRadius = 4
        hintBanner.translatesAutoresizingMaskIntoConstraints = false
        // Added to topStack by setupViews, not directly to self

        let accent = NSView()
        accent.wantsLayer = true
        accent.layer?.backgroundColor = ThaneTheme.accentColor.withAlphaComponent(0.4).cgColor
        accent.layer?.cornerRadius = 1.5
        accent.translatesAutoresizingMaskIntoConstraints = false
        hintBanner.addSubview(accent)

        let t1 = NSTextField(wrappingLabelWithString: "Start with /plan to flesh out your approach, then tell Claude to add it to the thane queue")
        t1.font = ThaneTheme.uiFont(size: 13)
        t1.textColor = ThaneTheme.secondaryText
        t1.maximumNumberOfLines = 3
        t1.translatesAutoresizingMaskIntoConstraints = false
        hintBanner.addSubview(t1)

        let t2 = NSTextField(wrappingLabelWithString: "e.g. \"add this plan to my thane queue\"")
        t2.font = NSFont(descriptor: ThaneTheme.uiFont(size: 12).fontDescriptor.withSymbolicTraits(.italic), size: 12)
                   ?? ThaneTheme.uiFont(size: 12)
        t2.textColor = ThaneTheme.tertiaryText
        t2.maximumNumberOfLines = 2
        t2.translatesAutoresizingMaskIntoConstraints = false
        hintBanner.addSubview(t2)

        NSLayoutConstraint.activate([
            accent.leadingAnchor.constraint(equalTo: hintBanner.leadingAnchor),
            accent.topAnchor.constraint(equalTo: hintBanner.topAnchor),
            accent.bottomAnchor.constraint(equalTo: hintBanner.bottomAnchor),
            accent.widthAnchor.constraint(equalToConstant: 3),
            t1.topAnchor.constraint(equalTo: hintBanner.topAnchor, constant: 10),
            t1.leadingAnchor.constraint(equalTo: accent.trailingAnchor, constant: 9),
            t1.trailingAnchor.constraint(equalTo: hintBanner.trailingAnchor, constant: -10),
            t2.topAnchor.constraint(equalTo: t1.bottomAnchor, constant: 4),
            t2.leadingAnchor.constraint(equalTo: t1.leadingAnchor),
            t2.bottomAnchor.constraint(equalTo: hintBanner.bottomAnchor, constant: -10),
        ])
    }

    private func setupEmptyBox() {
        let content = ViewFactories.makeEmptyState(
            icon: "list.bullet",
            title: "No tasks in queue",
            hint: "Start with /plan to flesh out your approach, then tell Claude to \"add this plan to my thane queue\""
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

    private func makeEntryRow(_ entry: QueueEntryInfoDTO, index: Int) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        container.wantsLayer = true

        // Bottom border
        let border = NSView()
        border.wantsLayer = true
        border.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        border.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(border)

        // ── Top: content + badge ──
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

        // ── Bottom: meta + buttons ──
        var metaParts = ["P\(entry.priority)", formatTime(entry.createdAt)]
        if let model = entry.model {
            // Show a concise model name (e.g. "sonnet-4-5" from "claude-sonnet-4-5-20250514")
            let displayModel = Self.shortModelName(model)
            metaParts.append(displayModel)
        }
        let metaText = metaParts.joined(separator: " \u{00B7} ")
        let metaLabel = NSTextField(labelWithString: metaText)
        metaLabel.font = ThaneTheme.uiFont(size: 11)
        metaLabel.textColor = ThaneTheme.tertiaryText
        metaLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(metaLabel)

        let buttonStack = NSStackView()
        buttonStack.orientation = .horizontal
        buttonStack.spacing = 4
        buttonStack.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(buttonStack)

        switch entry.status {
        case .queued, .running:
            buttonStack.addArrangedSubview(
                makeActionButton("Cancel", tag: index, action: #selector(cancelEntry(_:)),
                                 css: "destructive"))
        case .completed:
            buttonStack.addArrangedSubview(
                makeActionButton("Dismiss", tag: index, action: #selector(dismissEntry(_:)),
                                 css: "flat"))
        case .failed, .cancelled:
            buttonStack.addArrangedSubview(
                makeActionButton("Retry", tag: index, action: #selector(retryEntry(_:)),
                                 css: "suggested"))
            buttonStack.addArrangedSubview(
                makeActionButton("Dismiss", tag: index, action: #selector(dismissEntry(_:)),
                                 css: "flat"))
        case .pausedTokenLimit, .pausedByUser:
            break
        }

        NSLayoutConstraint.activate([
            contentLabel.topAnchor.constraint(equalTo: container.topAnchor, constant: 10),
            contentLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 14),
            contentLabel.trailingAnchor.constraint(lessThanOrEqualTo: badge.leadingAnchor, constant: -8),

            badge.centerYAnchor.constraint(equalTo: contentLabel.centerYAnchor),
            badge.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -14),

            metaLabel.topAnchor.constraint(equalTo: contentLabel.bottomAnchor, constant: 4),
            metaLabel.leadingAnchor.constraint(equalTo: contentLabel.leadingAnchor),
            metaLabel.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -10),

            buttonStack.centerYAnchor.constraint(equalTo: metaLabel.centerYAnchor),
            buttonStack.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -14),

            border.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            border.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            border.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            border.heightAnchor.constraint(equalToConstant: 1),
        ])

        return container
    }

    private func makeBadge(_ status: QueueEntryStatusDTO) -> NSTextField {
        let color = statusBadgeColor(status)
        let text = " \(statusLabelText(status)) "
        let badge = NSTextField(labelWithString: text)
        badge.font = NSFont.boldSystemFont(ofSize: 11)
        badge.textColor = color
        badge.wantsLayer = true
        badge.layer?.cornerRadius = 4

        switch status {
        case .queued, .running, .pausedTokenLimit, .pausedByUser:
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

    private func makeActionButton(_ title: String, tag: Int, action: Selector, css: String) -> NSButton {
        let btn = NSButton(title: title, target: self, action: action)
        btn.bezelStyle = .recessed
        btn.controlSize = .mini
        btn.isBordered = false
        btn.font = ThaneTheme.uiFont(size: 10)
        btn.tag = tag

        switch css {
        case "destructive": btn.contentTintColor = ThaneTheme.errorColor
        case "suggested": btn.contentTintColor = ThaneTheme.accentColor
        default: btn.contentTintColor = ThaneTheme.tertiaryText
        }

        return btn
    }

    // MARK: - Helpers

    private func statusBadgeColor(_ status: QueueEntryStatusDTO) -> NSColor {
        switch status {
        case .queued: return ThaneTheme.accentColor
        case .running: return ThaneTheme.agentActiveColor
        case .pausedTokenLimit, .pausedByUser: return ThaneTheme.warningColor
        case .completed: return ThaneTheme.agentActiveColor
        case .failed: return ThaneTheme.errorColor
        case .cancelled: return ThaneTheme.tertiaryText
        }
    }

    private func statusLabelText(_ status: QueueEntryStatusDTO) -> String {
        switch status {
        case .queued: return "Queued"
        case .running: return "Running"
        case .pausedTokenLimit, .pausedByUser: return "Paused"
        case .completed: return "Done"
        case .failed: return "Failed"
        case .cancelled: return "Cancelled"
        }
    }

    private func formatTime(_ iso: String) -> String {
        let f = ISO8601DateFormatter()
        if let date = f.date(from: iso) {
            let tf = DateFormatter()
            tf.dateFormat = "HH:mm:ss"
            return tf.string(from: date)
        }
        // Fallback: take last 8 chars
        if iso.count >= 8 { return String(iso.suffix(8)) }
        return iso
    }

    /// Shorten a full model ID like "claude-sonnet-4-5-20250514" to "sonnet-4-5".
    private static func shortModelName(_ model: String) -> String {
        var name = model
        // Strip "claude-" prefix
        if name.hasPrefix("claude-") { name = String(name.dropFirst(7)) }
        // Strip date suffix (e.g. "-20250514")
        if let range = name.range(of: #"-\d{8}$"#, options: .regularExpression) {
            name = String(name[name.startIndex..<range.lowerBound])
        }
        return name
    }

    // MARK: - Actions

    @objc private func toggleSubmitBar() {
        guard let row = submitRow else { return }
        let showing = !row.isHidden
        row.isHidden = showing
        if !showing { window?.makeFirstResponder(submitField) }
    }

    @objc private func submitClicked() {
        let content = submitField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty else { return }
        _ = bridge.queueSubmit(content: content)
        submitField.stringValue = ""
        submitRow?.isHidden = true
        reload()
    }

    @objc private func cancelEntry(_ sender: NSButton) {
        guard sender.tag < entries.count else { return }
        _ = bridge.queueCancel(entryId: entries[sender.tag].id)
        reload()
    }

    @objc private func retryEntry(_ sender: NSButton) {
        guard sender.tag < entries.count else { return }
        _ = bridge.queueRetry(entryId: entries[sender.tag].id)
        reload()
    }

    @objc private func dismissEntry(_ sender: NSButton) {
        guard sender.tag < entries.count else { return }
        _ = bridge.queueCancel(entryId: entries[sender.tag].id)
        reload()
    }

    @objc private func processNextClicked() {
        (NSApp.delegate as? AppDelegate)?.processNextQueueEntry()
    }

    @objc private func processAllClicked() {
        (NSApp.delegate as? AppDelegate)?.processAllQueueEntries()
    }

    @objc private func sandboxToggled() {
        if sandboxSwitch.state == .on {
            bridge.queueSandboxEnable()
        } else {
            bridge.queueSandboxDisable()
        }
        updateSandboxControls()
    }

    @objc private func sandboxEnforcementChanged() {
        let level: EnforcementLevelDTO
        switch sandboxEnforcementPopup.indexOfSelectedItem {
        case 0: level = .permissive
        case 2: level = .strict
        default: level = .enforcing
        }
        bridge.queueSandboxSetEnforcement(level)
    }

    @objc private func sandboxNetworkToggled() {
        bridge.queueSandboxSetNetwork(sandboxNetworkSwitch.state == .on)
    }

    private func updateSandboxControls() {
        let status = bridge.queueSandboxStatus()
        let enabled = status?.enabled ?? false
        sandboxSwitch.state = enabled ? .on : .off
        sandboxEnforcementPopup.isEnabled = enabled
        sandboxNetworkSwitch.isEnabled = enabled

        if let status = status {
            switch status.enforcement {
            case .permissive: sandboxEnforcementPopup.selectItem(at: 0)
            case .enforcing: sandboxEnforcementPopup.selectItem(at: 1)
            case .strict: sandboxEnforcementPopup.selectItem(at: 2)
            }
            sandboxNetworkSwitch.state = status.allowNetwork ? .on : .off
        }
    }
}

// MARK: - NSTextFieldDelegate

extension AgentQueuePanel: NSTextFieldDelegate {
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        if commandSelector == #selector(insertNewline(_:)) {
            submitClicked()
            return true
        }
        return false
    }
}


