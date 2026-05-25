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
    private let auditRedactionPopup = NSPopUpButton()
    private let auditCodeSessionsSwitch = NSSwitch()
    private let auditAppChatsSwitch = NSSwitch()
    private let auditQueuePromptsSwitch = NSSwitch()
    private let auditRetentionField = NSTextField()
    private let auditAllowClearSwitch = NSSwitch()

    // Audit Sinks (Phase 5)
    private let auditSinkSyslogEnable = NSSwitch()
    private let auditSinkSyslogHostField = NSTextField()
    private let auditSinkSyslogSeverityPopup = NSPopUpButton()
    private let auditSinkSyslogTestBtn = NSButton(title: "Send test event", target: nil, action: nil)
    private let auditSinkWebhookEnable = NSSwitch()
    private let auditSinkWebhookUrlField = NSTextField()
    private let auditSinkWebhookSeverityPopup = NSPopUpButton()
    private let auditSinkWebhookTestBtn = NSButton(title: "Send test event", target: nil, action: nil)

    // Audit Sinks (Phase 6) — enterprise destinations
    private let auditSinkS3Enable = NSSwitch()
    private let auditSinkS3BucketField = NSTextField()
    private let auditSinkS3SeverityPopup = NSPopUpButton()
    private let auditSinkS3TestBtn = NSButton(title: "Send test event", target: nil, action: nil)
    private let auditSinkSplunkEnable = NSSwitch()
    private let auditSinkSplunkUrlField = NSTextField()
    private let auditSinkSplunkSeverityPopup = NSPopUpButton()
    private let auditSinkSplunkTestBtn = NSButton(title: "Send test event", target: nil, action: nil)
    private let auditSinkDatadogEnable = NSSwitch()
    private let auditSinkDatadogRegionPopup = NSPopUpButton()
    private let auditSinkDatadogSeverityPopup = NSPopUpButton()
    private let auditSinkDatadogTestBtn = NSButton(title: "Send test event", target: nil, action: nil)

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

        auditRedactionPopup.addItems(withTitles: [
            "None — store events verbatim (NOT RECOMMENDED)",
            "Redact — scrub detected secrets and PII (recommended)",
            "Strict — additionally strip free-form fields",
        ])
        auditRedactionPopup.target = self
        auditRedactionPopup.action = #selector(auditRedactionChanged)
        auditRedactionPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Audit Redaction Policy", control: auditRedactionPopup))

        let redactionHint = makeLabel(
            "Scrubs detected secrets/PII before events hit disk. The HMAC is computed over the redacted form.",
            bold: false, size: 10)
        redactionHint.textColor = ThaneTheme.tertiaryText
        redactionHint.lineBreakMode = .byWordWrapping
        redactionHint.preferredMaxLayoutWidth = 240
        stack.addArrangedSubview(redactionHint)

        auditCodeSessionsSwitch.target = self
        auditCodeSessionsSwitch.action = #selector(auditCodeSessionsChanged)
        auditCodeSessionsSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Audit Claude Code Sessions", control: auditCodeSessionsSwitch))

        auditAppChatsSwitch.target = self
        auditAppChatsSwitch.action = #selector(auditAppChatsChanged)
        auditAppChatsSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Audit Claude.ai Chats", control: auditAppChatsSwitch))

        let auditHint = makeLabel("Uses OAuth token from ~/.claude/.credentials.json", bold: false, size: 10)
        auditHint.textColor = ThaneTheme.tertiaryText
        auditHint.lineBreakMode = .byWordWrapping
        auditHint.preferredMaxLayoutWidth = 240
        stack.addArrangedSubview(auditHint)

        auditQueuePromptsSwitch.target = self
        auditQueuePromptsSwitch.action = #selector(auditQueuePromptsChanged)
        auditQueuePromptsSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Audit Queue Prompts", control: auditQueuePromptsSwitch))

        auditRetentionField.placeholderString = "90"
        auditRetentionField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        auditRetentionField.delegate = self
        stack.addArrangedSubview(makeFormRow("Audit Retention (days)", control: auditRetentionField))

        let retentionHint = makeLabel("Rotated audit files older than this are purged. 0 = keep forever.", bold: false, size: 10)
        retentionHint.textColor = ThaneTheme.tertiaryText
        retentionHint.lineBreakMode = .byWordWrapping
        retentionHint.preferredMaxLayoutWidth = 240
        stack.addArrangedSubview(retentionHint)

        auditAllowClearSwitch.target = self
        auditAllowClearSwitch.action = #selector(auditAllowClearChanged)
        auditAllowClearSwitch.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Allow Audit Log Clear", control: auditAllowClearSwitch))

        let clearHint = makeLabel("When off, the Clear control is hidden. Required for most compliance policies.", bold: false, size: 10)
        clearHint.textColor = ThaneTheme.tertiaryText
        clearHint.lineBreakMode = .byWordWrapping
        clearHint.preferredMaxLayoutWidth = 240
        stack.addArrangedSubview(clearHint)

        // ── Audit Sinks (Phase 5) ──
        // External shipping of audit events. Each sink has an enable toggle,
        // host/url field, min-severity selector, and a Test button that fires
        // a synthetic Info event so the operator can verify connectivity.
        stack.addArrangedSubview(makeSectionHeader("Audit Sinks"))

        auditSinkSyslogEnable.target = self
        auditSinkSyslogEnable.action = #selector(auditSinkSyslogEnableChanged)
        auditSinkSyslogEnable.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Ship to syslog (TCP/TLS)", control: auditSinkSyslogEnable))

        auditSinkSyslogHostField.placeholderString = "logs.example.com:6514"
        auditSinkSyslogHostField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        auditSinkSyslogHostField.delegate = self
        stack.addArrangedSubview(makeFormRow("Syslog host", control: auditSinkSyslogHostField))

        auditSinkSyslogSeverityPopup.addItems(withTitles: ["Info", "Warning", "Alert", "Critical"])
        auditSinkSyslogSeverityPopup.target = self
        auditSinkSyslogSeverityPopup.action = #selector(auditSinkSyslogSeverityChanged)
        auditSinkSyslogSeverityPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Syslog min severity", control: auditSinkSyslogSeverityPopup))

        auditSinkSyslogTestBtn.target = self
        auditSinkSyslogTestBtn.action = #selector(auditSinkSyslogTestClicked)
        auditSinkSyslogTestBtn.bezelStyle = .recessed
        auditSinkSyslogTestBtn.controlSize = .small
        auditSinkSyslogTestBtn.toolTip = "Fire a synthetic Info-severity event to verify syslog delivery."
        stack.addArrangedSubview(auditSinkSyslogTestBtn)

        auditSinkWebhookEnable.target = self
        auditSinkWebhookEnable.action = #selector(auditSinkWebhookEnableChanged)
        auditSinkWebhookEnable.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Ship to webhook (HMAC-signed)", control: auditSinkWebhookEnable))

        auditSinkWebhookUrlField.placeholderString = "https://siem.example.com/ingest"
        auditSinkWebhookUrlField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        auditSinkWebhookUrlField.delegate = self
        stack.addArrangedSubview(makeFormRow("Webhook URL", control: auditSinkWebhookUrlField))

        auditSinkWebhookSeverityPopup.addItems(withTitles: ["Info", "Warning", "Alert", "Critical"])
        auditSinkWebhookSeverityPopup.target = self
        auditSinkWebhookSeverityPopup.action = #selector(auditSinkWebhookSeverityChanged)
        auditSinkWebhookSeverityPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Webhook min severity", control: auditSinkWebhookSeverityPopup))

        auditSinkWebhookTestBtn.target = self
        auditSinkWebhookTestBtn.action = #selector(auditSinkWebhookTestClicked)
        auditSinkWebhookTestBtn.bezelStyle = .recessed
        auditSinkWebhookTestBtn.controlSize = .small
        auditSinkWebhookTestBtn.toolTip = "Fire a synthetic Info-severity event to verify webhook delivery."
        stack.addArrangedSubview(auditSinkWebhookTestBtn)

        // S3 (Phase 6) — bucket is the primary field; advanced options (region,
        // SSE, Object Lock, prefix, credentials) live in ~/.config/thane/config.
        auditSinkS3Enable.target = self
        auditSinkS3Enable.action = #selector(auditSinkS3EnableChanged)
        auditSinkS3Enable.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Ship to S3 (gzip JSONL)", control: auditSinkS3Enable))

        auditSinkS3BucketField.placeholderString = "my-org-audit-logs"
        auditSinkS3BucketField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        auditSinkS3BucketField.delegate = self
        stack.addArrangedSubview(makeFormRow("S3 bucket", control: auditSinkS3BucketField))

        auditSinkS3SeverityPopup.addItems(withTitles: ["Info", "Warning", "Alert", "Critical"])
        auditSinkS3SeverityPopup.target = self
        auditSinkS3SeverityPopup.action = #selector(auditSinkS3SeverityChanged)
        auditSinkS3SeverityPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("S3 min severity", control: auditSinkS3SeverityPopup))

        auditSinkS3TestBtn.target = self
        auditSinkS3TestBtn.action = #selector(auditSinkS3TestClicked)
        auditSinkS3TestBtn.bezelStyle = .recessed
        auditSinkS3TestBtn.controlSize = .small
        auditSinkS3TestBtn.toolTip = "Fire a synthetic Info-severity event to verify S3 delivery."
        stack.addArrangedSubview(auditSinkS3TestBtn)

        // Splunk HEC (Phase 6).
        auditSinkSplunkEnable.target = self
        auditSinkSplunkEnable.action = #selector(auditSinkSplunkEnableChanged)
        auditSinkSplunkEnable.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Ship to Splunk HEC", control: auditSinkSplunkEnable))

        auditSinkSplunkUrlField.placeholderString = "https://splunk.example.com:8088/services/collector/event"
        auditSinkSplunkUrlField.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        auditSinkSplunkUrlField.delegate = self
        stack.addArrangedSubview(makeFormRow("Splunk URL", control: auditSinkSplunkUrlField))

        auditSinkSplunkSeverityPopup.addItems(withTitles: ["Info", "Warning", "Alert", "Critical"])
        auditSinkSplunkSeverityPopup.target = self
        auditSinkSplunkSeverityPopup.action = #selector(auditSinkSplunkSeverityChanged)
        auditSinkSplunkSeverityPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Splunk min severity", control: auditSinkSplunkSeverityPopup))

        auditSinkSplunkTestBtn.target = self
        auditSinkSplunkTestBtn.action = #selector(auditSinkSplunkTestClicked)
        auditSinkSplunkTestBtn.bezelStyle = .recessed
        auditSinkSplunkTestBtn.controlSize = .small
        auditSinkSplunkTestBtn.toolTip = "Fire a synthetic Info-severity event to verify Splunk delivery."
        stack.addArrangedSubview(auditSinkSplunkTestBtn)

        // Datadog Logs (Phase 6). Region is a popup because crossing regions
        // silently is a data-residency footgun.
        auditSinkDatadogEnable.target = self
        auditSinkDatadogEnable.action = #selector(auditSinkDatadogEnableChanged)
        auditSinkDatadogEnable.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Ship to Datadog Logs", control: auditSinkDatadogEnable))

        auditSinkDatadogRegionPopup.addItems(withTitles: ["us", "us3", "us5", "eu", "ap1"])
        auditSinkDatadogRegionPopup.target = self
        auditSinkDatadogRegionPopup.action = #selector(auditSinkDatadogRegionChanged)
        auditSinkDatadogRegionPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Datadog region", control: auditSinkDatadogRegionPopup))

        auditSinkDatadogSeverityPopup.addItems(withTitles: ["Info", "Warning", "Alert", "Critical"])
        auditSinkDatadogSeverityPopup.target = self
        auditSinkDatadogSeverityPopup.action = #selector(auditSinkDatadogSeverityChanged)
        auditSinkDatadogSeverityPopup.controlSize = .small
        stack.addArrangedSubview(makeFormRow("Datadog min severity", control: auditSinkDatadogSeverityPopup))

        auditSinkDatadogTestBtn.target = self
        auditSinkDatadogTestBtn.action = #selector(auditSinkDatadogTestClicked)
        auditSinkDatadogTestBtn.bezelStyle = .recessed
        auditSinkDatadogTestBtn.controlSize = .small
        auditSinkDatadogTestBtn.toolTip = "Fire a synthetic Info-severity event to verify Datadog delivery."
        stack.addArrangedSubview(auditSinkDatadogTestBtn)

        let sinksHint = makeLabel(
            "Secrets (HMAC, HEC token, Datadog API key, S3 credentials) live in the platform secret store. Full per-sink options live in ~/.config/thane/config — see AUDIT_LOG.md. Restart thane for changes to take effect.",
            bold: false, size: 10)
        sinksHint.textColor = ThaneTheme.tertiaryText
        sinksHint.lineBreakMode = .byWordWrapping
        sinksHint.preferredMaxLayoutWidth = 240
        stack.addArrangedSubview(sinksHint)

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

        let redactionPolicy = (bridge.configGet(key: "audit-redaction-policy") ?? "redact").lowercased()
        let redactionIndex: Int
        switch redactionPolicy {
        case "none": redactionIndex = 0
        case "strict": redactionIndex = 2
        default: redactionIndex = 1
        }
        auditRedactionPopup.selectItem(at: redactionIndex)

        auditCodeSessionsSwitch.state = (bridge.configGet(key: "audit-claude-code-sessions") ?? "true") == "true" ? .on : .off
        auditAppChatsSwitch.state = (bridge.configGet(key: "audit-claude-app-chats") ?? "false") == "true" ? .on : .off
        auditQueuePromptsSwitch.state = (bridge.configGet(key: "audit-queue-prompts") ?? "true") == "true" ? .on : .off
        auditRetentionField.stringValue = bridge.configGet(key: "audit-retention-days") ?? "90"
        auditAllowClearSwitch.state = (bridge.configGet(key: "audit-allow-clear") ?? "false") == "true" ? .on : .off

        // Phase 5: audit sink defaults.
        auditSinkSyslogEnable.state = (bridge.configGet(key: "audit-sink-syslog-enabled") ?? "false") == "true" ? .on : .off
        let syslogHost = bridge.configGet(key: "audit-sink-syslog-host") ?? ""
        let syslogPort = bridge.configGet(key: "audit-sink-syslog-port") ?? "6514"
        auditSinkSyslogHostField.stringValue = syslogHost.isEmpty
            ? ""
            : "\(syslogHost):\(syslogPort)"
        auditSinkSyslogSeverityPopup.selectItem(withTitle:
            (bridge.configGet(key: "audit-sink-syslog-min-severity") ?? "info").capitalized)
        auditSinkWebhookEnable.state = (bridge.configGet(key: "audit-sink-webhook-enabled") ?? "false") == "true" ? .on : .off
        auditSinkWebhookUrlField.stringValue = bridge.configGet(key: "audit-sink-webhook-url") ?? ""
        auditSinkWebhookSeverityPopup.selectItem(withTitle:
            (bridge.configGet(key: "audit-sink-webhook-min-severity") ?? "info").capitalized)

        // Phase 6: enterprise sink defaults.
        auditSinkS3Enable.state = (bridge.configGet(key: "audit-sink-s3-enabled") ?? "false") == "true" ? .on : .off
        auditSinkS3BucketField.stringValue = bridge.configGet(key: "audit-sink-s3-bucket") ?? ""
        auditSinkS3SeverityPopup.selectItem(withTitle:
            (bridge.configGet(key: "audit-sink-s3-min-severity") ?? "info").capitalized)
        auditSinkSplunkEnable.state = (bridge.configGet(key: "audit-sink-splunk-enabled") ?? "false") == "true" ? .on : .off
        auditSinkSplunkUrlField.stringValue = bridge.configGet(key: "audit-sink-splunk-url") ?? ""
        auditSinkSplunkSeverityPopup.selectItem(withTitle:
            (bridge.configGet(key: "audit-sink-splunk-min-severity") ?? "info").capitalized)
        auditSinkDatadogEnable.state = (bridge.configGet(key: "audit-sink-datadog-enabled") ?? "false") == "true" ? .on : .off
        auditSinkDatadogRegionPopup.selectItem(withTitle:
            bridge.configGet(key: "audit-sink-datadog-region") ?? "us")
        auditSinkDatadogSeverityPopup.selectItem(withTitle:
            (bridge.configGet(key: "audit-sink-datadog-min-severity") ?? "info").capitalized)

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

    @objc private func auditRedactionChanged() {
        let value: String
        switch auditRedactionPopup.indexOfSelectedItem {
        case 0: value = "none"
        case 2: value = "strict"
        default: value = "redact"
        }
        bridge.configSet(key: "audit-redaction-policy", value: value)
    }

    @objc private func auditCodeSessionsChanged() {
        bridge.configSet(key: "audit-claude-code-sessions", value: auditCodeSessionsSwitch.state == .on ? "true" : "false")
    }

    @objc private func auditAppChatsChanged() {
        bridge.configSet(key: "audit-claude-app-chats", value: auditAppChatsSwitch.state == .on ? "true" : "false")
    }

    @objc private func auditQueuePromptsChanged() {
        bridge.configSet(key: "audit-queue-prompts", value: auditQueuePromptsSwitch.state == .on ? "true" : "false")
    }

    @objc private func auditAllowClearChanged() {
        bridge.configSet(key: "audit-allow-clear", value: auditAllowClearSwitch.state == .on ? "true" : "false")
    }

    // Phase 5: audit sink controls.

    @objc private func auditSinkSyslogEnableChanged() {
        let on = auditSinkSyslogEnable.state == .on
        bridge.configSet(key: "audit-sink-syslog-enabled", value: on ? "true" : "false")
    }

    @objc private func auditSinkSyslogSeverityChanged() {
        guard let sev = auditSinkSyslogSeverityPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "audit-sink-syslog-min-severity", value: sev)
    }

    @objc private func auditSinkSyslogTestClicked() {
        bridge.logAuditEvent(
            workspaceId: "",
            eventType: "AuditSinkTest",
            severity: .info,
            description: "Synthetic test event for sink 'syslog'",
            metadata: ["sink": "syslog"]
        )
    }

    @objc private func auditSinkWebhookEnableChanged() {
        let on = auditSinkWebhookEnable.state == .on
        bridge.configSet(key: "audit-sink-webhook-enabled", value: on ? "true" : "false")
    }

    @objc private func auditSinkWebhookSeverityChanged() {
        guard let sev = auditSinkWebhookSeverityPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "audit-sink-webhook-min-severity", value: sev)
    }

    @objc private func auditSinkWebhookTestClicked() {
        bridge.logAuditEvent(
            workspaceId: "",
            eventType: "AuditSinkTest",
            severity: .info,
            description: "Synthetic test event for sink 'webhook'",
            metadata: ["sink": "webhook"]
        )
    }

    // Phase 6: S3.

    @objc private func auditSinkS3EnableChanged() {
        let on = auditSinkS3Enable.state == .on
        bridge.configSet(key: "audit-sink-s3-enabled", value: on ? "true" : "false")
    }

    @objc private func auditSinkS3SeverityChanged() {
        guard let sev = auditSinkS3SeverityPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "audit-sink-s3-min-severity", value: sev)
    }

    @objc private func auditSinkS3TestClicked() {
        bridge.logAuditEvent(
            workspaceId: "",
            eventType: "AuditSinkTest",
            severity: .info,
            description: "Synthetic test event for sink 's3'",
            metadata: ["sink": "s3"]
        )
    }

    // Phase 6: Splunk HEC.

    @objc private func auditSinkSplunkEnableChanged() {
        let on = auditSinkSplunkEnable.state == .on
        bridge.configSet(key: "audit-sink-splunk-enabled", value: on ? "true" : "false")
    }

    @objc private func auditSinkSplunkSeverityChanged() {
        guard let sev = auditSinkSplunkSeverityPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "audit-sink-splunk-min-severity", value: sev)
    }

    @objc private func auditSinkSplunkTestClicked() {
        bridge.logAuditEvent(
            workspaceId: "",
            eventType: "AuditSinkTest",
            severity: .info,
            description: "Synthetic test event for sink 'splunk'",
            metadata: ["sink": "splunk"]
        )
    }

    // Phase 6: Datadog Logs.

    @objc private func auditSinkDatadogEnableChanged() {
        let on = auditSinkDatadogEnable.state == .on
        bridge.configSet(key: "audit-sink-datadog-enabled", value: on ? "true" : "false")
    }

    @objc private func auditSinkDatadogRegionChanged() {
        guard let region = auditSinkDatadogRegionPopup.titleOfSelectedItem else { return }
        bridge.configSet(key: "audit-sink-datadog-region", value: region)
    }

    @objc private func auditSinkDatadogSeverityChanged() {
        guard let sev = auditSinkDatadogSeverityPopup.titleOfSelectedItem?.lowercased() else { return }
        bridge.configSet(key: "audit-sink-datadog-min-severity", value: sev)
    }

    @objc private func auditSinkDatadogTestClicked() {
        bridge.logAuditEvent(
            workspaceId: "",
            eventType: "AuditSinkTest",
            severity: .info,
            description: "Synthetic test event for sink 'datadog'",
            metadata: ["sink": "datadog"]
        )
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
        } else if field === auditRetentionField {
            let text = field.stringValue.trimmingCharacters(in: .whitespaces)
            // Reject non-numeric input by reverting to the persisted value.
            if let days = UInt32(text) {
                bridge.configSet(key: "audit-retention-days", value: "\(days)")
            } else {
                field.stringValue = bridge.configGet(key: "audit-retention-days") ?? "90"
            }
        } else if field === auditSinkSyslogHostField {
            let raw = field.stringValue.trimmingCharacters(in: .whitespaces)
            // Accept either "host" or "host:port". Persist the two keys
            // separately so the Rust side keeps reading them through the same
            // config accessors.
            let parts = raw.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
            if parts.count == 2, let port = UInt16(parts[1]) {
                bridge.configSet(key: "audit-sink-syslog-host", value: String(parts[0]))
                bridge.configSet(key: "audit-sink-syslog-port", value: "\(port)")
            } else {
                bridge.configSet(key: "audit-sink-syslog-host", value: raw)
            }
        } else if field === auditSinkWebhookUrlField {
            bridge.configSet(key: "audit-sink-webhook-url", value: field.stringValue.trimmingCharacters(in: .whitespaces))
        } else if field === auditSinkS3BucketField {
            bridge.configSet(key: "audit-sink-s3-bucket", value: field.stringValue.trimmingCharacters(in: .whitespaces))
        } else if field === auditSinkSplunkUrlField {
            bridge.configSet(key: "audit-sink-splunk-url", value: field.stringValue.trimmingCharacters(in: .whitespaces))
        }
    }
}
