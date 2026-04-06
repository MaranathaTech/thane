import AppKit

/// Right-side settings panel matching Linux GTK settings_panel.rs.
/// All 14 settings across 5 sections with proper slider + label layout.
@MainActor
final class SettingsPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge

    // Font
    private let fontFamilyPopup = NSPopUpButton()
    private let terminalFontSizeSlider = NSSlider()
    private let terminalFontSizeValue = NSTextField(labelWithString: "13")
    private let uiTextSizeSlider = NSSlider()
    private let uiTextSizeValue = NSTextField(labelWithString: "14")
    private let fontColorWell = NSColorWell()

    // Terminal
    private let scrollbackSlider = NSSlider()
    private let scrollbackValue = NSTextField(labelWithString: "10000")
    // Behavior
    private let confirmCloseSwitch = NSSwitch()
    private let openUrlInAppSwitch = NSSwitch()
    private let openUrlInBrowserSwitch = NSSwitch()

    // Security
    private let sensitiveDataPopup = NSPopUpButton()

    // Queue
    private let queueModePopup = NSPopUpButton()
    private let queueScheduleField = NSTextField()
    private var scheduleRow: NSView?

    // Cost display
    private let costScopePopup = NSPopUpButton()

    // Enterprise
    private let enterpriseCostField = NSTextField()
    private var enterpriseCostRow: NSView?

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(frame: .zero)
        setupViews()
        loadValues()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public

    func reload() {
        loadValues()
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

        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 4
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 20, right: 12)
        scrollView.contentView = FlippedClipView()
        scrollView.documentView = stack

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            stack.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            stack.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
        ])

        // Title
        let title = makeLabel("Settings", bold: true, size: 14)
        stack.addArrangedSubview(title)
        stack.setCustomSpacing(12, after: title)

        // ── Font ──
        stack.addArrangedSubview(makeSectionHeader("Font"))

        populateFontFamilies()
        fontFamilyPopup.target = self
        fontFamilyPopup.action = #selector(fontFamilyChanged)
        fontFamilyPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Family", control: fontFamilyPopup))

        terminalFontSizeSlider.minValue = 6
        terminalFontSizeSlider.maxValue = 72
        terminalFontSizeSlider.doubleValue = 13
        terminalFontSizeSlider.target = self
        terminalFontSizeSlider.action = #selector(terminalFontSizeChanged)
        terminalFontSizeSlider.isContinuous = true
        terminalFontSizeSlider.controlSize = .small
        stack.addArrangedSubview(makeSliderRow("Terminal Font Size", slider: terminalFontSizeSlider, valueLabel: terminalFontSizeValue))

        uiTextSizeSlider.minValue = 8
        uiTextSizeSlider.maxValue = 24
        uiTextSizeSlider.doubleValue = 14
        uiTextSizeSlider.target = self
        uiTextSizeSlider.action = #selector(uiTextSizeChanged)
        uiTextSizeSlider.isContinuous = true
        uiTextSizeSlider.controlSize = .small
        stack.addArrangedSubview(makeSliderRow("UI Text Size", slider: uiTextSizeSlider, valueLabel: uiTextSizeValue))

        fontColorWell.color = ThaneTheme.colorFromHex("#e4e4e7") ?? .white
        fontColorWell.target = self
        fontColorWell.action = #selector(fontColorChanged)
        fontColorWell.translatesAutoresizingMaskIntoConstraints = false
        fontColorWell.widthAnchor.constraint(equalToConstant: 44).isActive = true
        fontColorWell.heightAnchor.constraint(equalToConstant: 24).isActive = true
        stack.addArrangedSubview(makeFormRow("Terminal Font Color", control: fontColorWell))

        // ── Terminal ──
        stack.addArrangedSubview(makeSectionHeader("Terminal"))

        scrollbackSlider.minValue = 1000
        scrollbackSlider.maxValue = 100000
        scrollbackSlider.doubleValue = 10000
        scrollbackSlider.target = self
        scrollbackSlider.action = #selector(scrollbackChanged)
        scrollbackSlider.isContinuous = true
        scrollbackSlider.controlSize = .small
        stack.addArrangedSubview(makeSliderRow("Scrollback Limit", slider: scrollbackSlider, valueLabel: scrollbackValue))

        // ── Behavior ──
        stack.addArrangedSubview(makeSectionHeader("Behavior"))

        confirmCloseSwitch.target = self
        confirmCloseSwitch.action = #selector(confirmCloseChanged)
        confirmCloseSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Confirm Close", control: confirmCloseSwitch))

        // ── Link Handling ──
        stack.addArrangedSubview(makeSectionHeader("Link Handling"))

        openUrlInAppSwitch.target = self
        openUrlInAppSwitch.action = #selector(openUrlInAppChanged)
        openUrlInAppSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Open URLs in App", control: openUrlInAppSwitch))

        openUrlInBrowserSwitch.target = self
        openUrlInBrowserSwitch.action = #selector(openUrlInBrowserChanged)
        openUrlInBrowserSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Open URLs in Browser", control: openUrlInBrowserSwitch))

        // ── Cost Display ──
        stack.addArrangedSubview(makeSectionHeader("Cost Display"))

        costScopePopup.addItems(withTitles: ["Session", "All Time"])
        costScopePopup.target = self
        costScopePopup.action = #selector(costScopeChanged)
        costScopePopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Cost Display Scope", control: costScopePopup))

        // ── Security ──
        stack.addArrangedSubview(makeSectionHeader("Security"))

        sensitiveDataPopup.addItems(withTitles: ["Allow", "Warn", "Block"])
        sensitiveDataPopup.target = self
        sensitiveDataPopup.action = #selector(sensitiveDataChanged)
        sensitiveDataPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Sensitive Data Policy", control: sensitiveDataPopup))

        // ── Agent Queue ──
        stack.addArrangedSubview(makeSectionHeader("Agent Queue"))

        queueModePopup.addItems(withTitles: ["Automatic", "Manual", "Scheduled"])
        queueModePopup.target = self
        queueModePopup.action = #selector(queueModeChanged)
        queueModePopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Processing Mode", control: queueModePopup))

        queueScheduleField.placeholderString = "Mon:09:00,Wed:14:00"
        queueScheduleField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        queueScheduleField.delegate = self
        let schedRow = makeFormRow("Schedule", control: queueScheduleField)

        let schedHint = makeLabel("Format: Day:HH:MM (e.g. Mon:09:00,Fri:18:00)", bold: false, size: 10)
        schedHint.textColor = ThaneTheme.tertiaryText
        schedHint.lineBreakMode = .byWordWrapping
        schedHint.preferredMaxLayoutWidth = 240
        if let schedStack = schedRow as? NSStackView {
            schedStack.addArrangedSubview(schedHint)
        }

        schedRow.isHidden = true
        scheduleRow = schedRow
        stack.addArrangedSubview(schedRow)

        // Enterprise monthly cost
        enterpriseCostField.placeholderString = "200.00"
        enterpriseCostField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        enterpriseCostField.delegate = self
        let entCostRow = makeFormRow("Enterprise Monthly Cost ($)", control: enterpriseCostField)

        let entCostHint = makeLabel("Your per-seat monthly cost. Default: $200 (worst-case estimate).", bold: false, size: 10)
        entCostHint.textColor = ThaneTheme.tertiaryText
        entCostHint.lineBreakMode = .byWordWrapping
        entCostHint.preferredMaxLayoutWidth = 240
        if let entStack = entCostRow as? NSStackView {
            entStack.addArrangedSubview(entCostHint)
        }

        entCostRow.isHidden = true  // Shown only for Enterprise plan
        enterpriseCostRow = entCostRow
        stack.addArrangedSubview(entCostRow)

        // Hint
        let hint = makeLabel("Changes apply immediately and are saved to ~/.config/thane/config", bold: false, size: 10)
        hint.textColor = ThaneTheme.tertiaryText
        hint.lineBreakMode = .byWordWrapping
        hint.preferredMaxLayoutWidth = 280
        stack.setCustomSpacing(16, after: stack.arrangedSubviews.last!)
        stack.addArrangedSubview(hint)
    }

    // MARK: - Layout helpers

    private func makeSectionHeader(_ title: String) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = ThaneTheme.dividerColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(divider)

        let label = makeLabel(title.uppercased(), bold: true, size: 10)
        label.textColor = ThaneTheme.tertiaryText
        container.addSubview(label)

        NSLayoutConstraint.activate([
            container.heightAnchor.constraint(equalToConstant: 32),
            divider.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
            divider.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1),
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            label.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        return container
    }

    private func makeFormRow(_ labelText: String, control: NSView) -> NSView {
        let row = NSStackView()
        row.orientation = .vertical
        row.alignment = .leading
        row.spacing = 4

        let label = makeLabel(labelText, bold: false, size: ThaneTheme.smallFontSize)
        label.textColor = ThaneTheme.secondaryText

        control.translatesAutoresizingMaskIntoConstraints = false

        row.addArrangedSubview(label)
        row.addArrangedSubview(control)

        // Make control stretch to fill width
        row.translatesAutoresizingMaskIntoConstraints = false
        if control is NSPopUpButton || control is NSTextField {
            control.widthAnchor.constraint(greaterThanOrEqualToConstant: 160).isActive = true
        }

        return row
    }

    private func makeSliderRow(_ labelText: String, slider: NSSlider, valueLabel: NSTextField) -> NSView {
        let row = NSStackView()
        row.orientation = .vertical
        row.alignment = .leading
        row.spacing = 4

        let label = makeLabel(labelText, bold: false, size: ThaneTheme.smallFontSize)
        label.textColor = ThaneTheme.secondaryText

        let sliderRow = NSStackView()
        sliderRow.orientation = .horizontal
        sliderRow.spacing = 8
        slider.translatesAutoresizingMaskIntoConstraints = false
        valueLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        valueLabel.textColor = ThaneTheme.primaryText
        valueLabel.alignment = .right
        valueLabel.widthAnchor.constraint(equalToConstant: 44).isActive = true
        sliderRow.addArrangedSubview(slider)
        sliderRow.addArrangedSubview(valueLabel)

        row.addArrangedSubview(label)
        row.addArrangedSubview(sliderRow)

        row.translatesAutoresizingMaskIntoConstraints = false
        sliderRow.translatesAutoresizingMaskIntoConstraints = false
        slider.widthAnchor.constraint(greaterThanOrEqualToConstant: 140).isActive = true

        return row
    }

    private func makeLabel(_ text: String, bold: Bool, size: CGFloat) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = bold ? ThaneTheme.boldLabelFont(size: size) : ThaneTheme.uiFont(size: size)
        label.textColor = ThaneTheme.primaryText
        label.translatesAutoresizingMaskIntoConstraints = false
        return label
    }

    // MARK: - Load values

    private func loadValues() {
        let family = bridge.configGet(key: "font-family") ?? ThaneTheme.fontFamily
        fontFamilyPopup.selectItem(withTitle: family)

        let termSize = Double(bridge.configGet(key: "font-size") ?? "") ?? bridge.configFontSize()
        terminalFontSizeSlider.doubleValue = termSize
        terminalFontSizeValue.stringValue = "\(Int(termSize))pt"

        let uiSize = Double(bridge.configGet(key: "ui-font-size") ?? "") ?? Double(ThaneTheme.uiFontSize)
        uiTextSizeSlider.doubleValue = uiSize
        uiTextSizeValue.stringValue = "\(Int(uiSize))pt"

        let fgHex = bridge.configGet(key: "terminal-foreground") ?? "#e4e4e7"
        fontColorWell.color = ThaneTheme.colorFromHex(fgHex) ?? .white

        let scrollback = Double(bridge.configGet(key: "scrollback-limit") ?? "") ?? 10000
        scrollbackSlider.doubleValue = scrollback
        scrollbackValue.stringValue = formatScrollback(Int(scrollback))

        confirmCloseSwitch.state = (bridge.configGet(key: "confirm-close") ?? "true") == "true" ? .on : .off
        openUrlInAppSwitch.state = (bridge.configGet(key: "urls-open-in-app") ?? "true") == "true" ? .on : .off
        openUrlInBrowserSwitch.state = (bridge.configGet(key: "urls-open-in-browser") ?? "false") == "true" ? .on : .off

        let costScope = bridge.configGet(key: "cost-display-scope") ?? "session"
        costScopePopup.selectItem(withTitle: costScope == "all-time" ? "All Time" : "Session")

        let sensitivePolicy = bridge.configGet(key: "sensitive-data-policy") ?? "warn"
        sensitiveDataPopup.selectItem(withTitle: sensitivePolicy.capitalized)

        let queueMode = bridge.configGet(key: "queue-mode") ?? "automatic"
        queueModePopup.selectItem(withTitle: queueMode.capitalized)
        updateScheduleVisibility()

        queueScheduleField.stringValue = bridge.configGet(key: "queue-schedule") ?? ""

        // Enterprise cost: show field only for Enterprise plan, load saved value.
        enterpriseCostField.stringValue = bridge.configGet(key: "enterprise-monthly-cost") ?? ""
        let limits = bridge.getTokenLimits()
        enterpriseCostRow?.isHidden = limits.planName.lowercased() != "enterprise"
    }

    private func formatScrollback(_ value: Int) -> String {
        if value >= 1000 { return "\(value / 1000)K" }
        return "\(value)"
    }

    private func updateScheduleVisibility() {
        scheduleRow?.isHidden = queueModePopup.titleOfSelectedItem != "Scheduled"
    }

    // MARK: - Actions

    @objc private func fontFamilyChanged() {
        guard let family = fontFamilyPopup.titleOfSelectedItem else { return }
        bridge.configSet(key: "font-family", value: family)
    }

    @objc private func terminalFontSizeChanged() {
        let size = Int(terminalFontSizeSlider.doubleValue)
        terminalFontSizeValue.stringValue = "\(size)pt"
        bridge.configSet(key: "font-size", value: "\(size)")
    }

    @objc private func uiTextSizeChanged() {
        let size = Int(uiTextSizeSlider.doubleValue)
        uiTextSizeValue.stringValue = "\(size)pt"
        bridge.configSet(key: "ui-font-size", value: "\(size)")
    }

    @objc private func scrollbackChanged() {
        let value = Int(scrollbackSlider.doubleValue / 1000) * 1000
        scrollbackValue.stringValue = formatScrollback(value)
        bridge.configSet(key: "scrollback-limit", value: "\(value)")
    }

    @objc private func confirmCloseChanged() {
        bridge.configSet(key: "confirm-close", value: confirmCloseSwitch.state == .on ? "true" : "false")
    }

    @objc private func openUrlInAppChanged() {
        bridge.configSet(key: "urls-open-in-app", value: openUrlInAppSwitch.state == .on ? "true" : "false")
    }

    @objc private func openUrlInBrowserChanged() {
        bridge.configSet(key: "urls-open-in-browser", value: openUrlInBrowserSwitch.state == .on ? "true" : "false")
    }

    @objc private func sensitiveDataChanged() {
        guard let policy = sensitiveDataPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "sensitive-data-policy", value: policy)
    }

    @objc private func costScopeChanged() {
        let scope = costScopePopup.indexOfSelectedItem == 1 ? "all-time" : "session"
        bridge.configSet(key: "cost-display-scope", value: scope)
    }

    @objc private func fontColorChanged() {
        let hex = ThaneTheme.hexFromColor(fontColorWell.color)
        bridge.configSet(key: "terminal-foreground", value: hex)
    }

    @objc private func queueModeChanged() {
        guard let mode = queueModePopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "queue-mode", value: mode)
        updateScheduleVisibility()
    }

    // MARK: - Font population

    private func populateFontFamilies() {
        fontFamilyPopup.removeAllItems()
        let monoFonts = NSFontManager.shared.availableFontFamilies.filter { family in
            guard let font = NSFont(name: family, size: 12) else { return false }
            return font.isFixedPitch
        }.sorted()

        var fonts = monoFonts
        // Always include the bundled JetBrains Mono NL at the top, even if
        // NSFont.isFixedPitch doesn't detect it
        if let idx = fonts.firstIndex(of: ThaneTheme.fontFamily) {
            fonts.remove(at: idx)
        }
        fonts.insert(ThaneTheme.fontFamily, at: 0)

        fontFamilyPopup.addItems(withTitles: fonts)
    }
}

// MARK: - NSTextFieldDelegate

extension SettingsPanel: NSTextFieldDelegate {
    func controlTextDidEndEditing(_ obj: Notification) {
        guard let field = obj.object as? NSTextField else { return }
        if field === queueScheduleField {
            bridge.configSet(key: "queue-schedule", value: field.stringValue)
        } else if field === enterpriseCostField {
            let text = field.stringValue.trimmingCharacters(in: .whitespaces)
            if text.isEmpty {
                bridge.configSet(key: "enterprise-monthly-cost", value: "")
            } else if let value = Double(text), value > 0 {
                bridge.configSet(key: "enterprise-monthly-cost", value: String(format: "%.2f", value))
            }
        }
    }
}
