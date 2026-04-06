import AppKit

/// Right-side panel listing notifications from the Rust bridge.
///
/// - NSTableView of NotificationInfoDTO rows (title, body, timestamp, urgency badge)
/// - Unread indicator (bold text)
/// - "Mark All Read" and "Clear" toolbar buttons
/// - Refreshed via `reload()` (called from UiCallback.notification_received)
@MainActor
final class NotificationPanel: NSView, ReloadablePanel {

    private let bridge: RustBridge
    private let scrollView = NSScrollView()
    private let tableView = NSTableView()
    private let emptyBox = NSView()
    private var notifications: [NotificationInfoDTO] = []

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
        notifications = bridge.listNotifications()
        tableView.reloadData()
        emptyBox.isHidden = !notifications.isEmpty
        scrollView.isHidden = notifications.isEmpty
    }

    // MARK: - Setup

    private func setupViews() {
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor

        // Header
        let header = makePanelHeader(
            title: "Notifications",
            buttons: [
                ("Mark All Read", #selector(markAllReadClicked)),
                ("Clear", #selector(clearClicked)),
            ]
        )
        addSubview(header)

        // Table
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        scrollView.backgroundColor = .clear
        scrollView.drawsBackground = false
        addSubview(scrollView)

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("notification"))
        column.title = ""
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 64
        tableView.backgroundColor = .clear
        tableView.delegate = self
        tableView.dataSource = self
        tableView.selectionHighlightStyle = .none

        scrollView.documentView = tableView

        // Empty state
        setupEmptyBox()
        addSubview(emptyBox)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 44),

            scrollView.topAnchor.constraint(equalTo: header.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),

            emptyBox.topAnchor.constraint(equalTo: header.bottomAnchor),
            emptyBox.leadingAnchor.constraint(equalTo: leadingAnchor),
            emptyBox.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    private func setupEmptyBox() {
        let content = ViewFactories.makeEmptyState(
            icon: "bell.slash",
            title: "No notifications",
            hint: "Notifications from workspace agents will appear here"
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

    // MARK: - Actions

    @objc private func markAllReadClicked() {
        bridge.markAllNotificationsRead()
        reload()
    }

    @objc private func clearClicked() {
        bridge.clearNotifications()
        reload()
    }

    // MARK: - Helpers

    private func makePanelHeader(title: String, buttons: [(String, Selector)]) -> NSView {
        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false

        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = ThaneTheme.boldLabelFont(size: 14)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(titleLabel)

        let buttonStack = NSStackView()
        buttonStack.orientation = .horizontal
        buttonStack.spacing = 8
        buttonStack.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(buttonStack)

        for (label, action) in buttons {
            let btn = NSButton(title: label, target: self, action: action)
            btn.bezelStyle = .recessed
            btn.controlSize = .small
            btn.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
            buttonStack.addArrangedSubview(btn)
        }

        NSLayoutConstraint.activate([
            titleLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            buttonStack.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            buttonStack.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -8),
        ])

        return header
    }

    private func urgencyColor(for urgency: NotifyUrgencyDTO) -> NSColor {
        switch urgency {
        case .low: return ThaneTheme.secondaryText
        case .normal: return ThaneTheme.accentColor
        case .critical: return ThaneTheme.errorColor
        }
    }
}

// MARK: - NSTableViewDataSource

extension NotificationPanel: NSTableViewDataSource {
    func numberOfRows(in tableView: NSTableView) -> Int {
        notifications.count
    }
}

// MARK: - NSTableViewDelegate

extension NotificationPanel: NSTableViewDelegate {
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < notifications.count else { return nil }
        let notif = notifications[row]

        let cell = NSView()

        // Title
        let titleLabel = NSTextField(labelWithString: notif.title)
        titleLabel.font = notif.read
            ? ThaneTheme.uiFont(size: ThaneTheme.uiFontSize)
            : ThaneTheme.boldLabelFont(size: ThaneTheme.uiFontSize)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(titleLabel)

        // Body
        let bodyLabel = NSTextField(labelWithString: notif.body)
        bodyLabel.font = ThaneTheme.uiFont(size: ThaneTheme.smallFontSize)
        bodyLabel.textColor = ThaneTheme.secondaryText
        bodyLabel.lineBreakMode = .byTruncatingTail
        bodyLabel.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(bodyLabel)

        // Timestamp
        let timeLabel = NSTextField(labelWithString: notif.timestamp)
        timeLabel.font = ThaneTheme.uiFont(size: 10)
        timeLabel.textColor = ThaneTheme.tertiaryText
        timeLabel.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(timeLabel)

        // Urgency badge
        let badge = NSTextField(labelWithString: " \(urgencyLabel(notif.urgency)) ")
        badge.font = ThaneTheme.uiFont(size: 9)
        badge.textColor = .white
        badge.wantsLayer = true
        badge.layer?.cornerRadius = 4
        badge.layer?.backgroundColor = urgencyColor(for: notif.urgency).cgColor
        badge.alignment = .center
        badge.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(badge)

        // Unread dot
        if !notif.read {
            let dot = NSView()
            dot.wantsLayer = true
            dot.layer?.cornerRadius = 4
            dot.layer?.backgroundColor = ThaneTheme.accentColor.cgColor
            dot.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(dot)
            NSLayoutConstraint.activate([
                dot.widthAnchor.constraint(equalToConstant: 8),
                dot.heightAnchor.constraint(equalToConstant: 8),
                dot.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
                dot.topAnchor.constraint(equalTo: cell.topAnchor, constant: 8),
            ])
        }

        let leadingOffset: CGFloat = notif.read ? 12 : 16

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: cell.topAnchor, constant: 6),
            titleLabel.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: leadingOffset),
            titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: badge.leadingAnchor, constant: -4),

            badge.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
            badge.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8),

            bodyLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 2),
            bodyLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            bodyLabel.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8),

            timeLabel.topAnchor.constraint(equalTo: bodyLabel.bottomAnchor, constant: 2),
            timeLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
        ])

        return cell
    }

    private func urgencyLabel(_ urgency: NotifyUrgencyDTO) -> String {
        switch urgency {
        case .low: return "LOW"
        case .normal: return "NORMAL"
        case .critical: return "CRITICAL"
        }
    }
}
