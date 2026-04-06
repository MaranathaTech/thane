import AppKit

/// Right-side help panel showing keyboard shortcuts and usage tips.
///
/// Displays macOS-specific shortcuts (Cmd instead of Ctrl)
/// with leader key (Cmd+B) reference.
@MainActor
final class HelpPanel: NSView {

    // MARK: - Init

    init(bridge: RustBridge) {
        super.init(frame: .zero)
        setupViews()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
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

        let textView = NSTextView()
        textView.isEditable = false
        textView.isSelectable = true
        textView.backgroundColor = .clear
        textView.textContainerInset = NSSize(width: 12, height: 12)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]

        scrollView.contentView = FlippedClipView()
        scrollView.documentView = textView

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Build attributed string
        let content = NSMutableAttributedString()

        let titleAttrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.boldLabelFont(size: 16),
            .foregroundColor: ThaneTheme.primaryText,
        ]
        let sectionAttrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.boldLabelFont(size: 13),
            .foregroundColor: ThaneTheme.primaryText,
        ]
        let keyAttrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.uiFont(size: 12),
            .foregroundColor: ThaneTheme.accentColor,
        ]
        let descAttrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.labelFont(size: 12),
            .foregroundColor: ThaneTheme.secondaryText,
        ]
        let tipAttrs: [NSAttributedString.Key: Any] = [
            .font: ThaneTheme.labelFont(size: 12),
            .foregroundColor: ThaneTheme.primaryText,
        ]

        content.append(NSAttributedString(string: "thane Help\n\n", attributes: titleAttrs))

        // Leader key
        content.append(NSAttributedString(string: "Leader Key\n", attributes: sectionAttrs))
        content.append(NSAttributedString(string: "Cmd+B", attributes: keyAttrs))
        content.append(NSAttributedString(string: " — activates leader mode (tmux-style)\n\n", attributes: descAttrs))

        // Leader sequences
        content.append(NSAttributedString(string: "Leader Sequences (Cmd+B, then…)\n", attributes: sectionAttrs))
        let leaderShortcuts: [(String, String)] = [
            ("c", "New workspace"),
            ("n", "Next workspace"),
            ("p", "Previous workspace"),
            ("x", "Close current pane"),
            (",", "Rename workspace"),
            ("1-9", "Jump to workspace by number"),
        ]
        for (key, desc) in leaderShortcuts {
            content.append(NSAttributedString(string: "  \(key)", attributes: keyAttrs))
            content.append(NSAttributedString(string: "  \(desc)\n", attributes: descAttrs))
        }
        content.append(NSAttributedString(string: "\n", attributes: descAttrs))

        // General shortcuts
        content.append(NSAttributedString(string: "General Shortcuts\n", attributes: sectionAttrs))
        let generalShortcuts: [(String, String)] = [
            ("Cmd+,", "Settings"),
            ("Cmd+I", "Notifications"),
            ("Cmd+Shift+A", "Audit Log"),
            ("Cmd+Shift+U", "CC Token Usage"),
            ("Cmd+Shift+P", "Agent Queue"),
            ("Cmd+Shift+S", "Sandbox"),
            ("Cmd+Shift+G", "Git Diff"),
            ("Cmd+Shift+L", "Processed Tasks"),
            ("F1", "Help (this panel)"),
            ("Cmd+Shift+B", "Toggle Sidebar"),
            ("Cmd+Shift+T", "New Workspace"),
            ("Cmd+Shift+R", "Rename Workspace"),
            ("Cmd+Shift+W", "Close Panel"),
            ("Cmd+Shift+F", "Find in Terminal"),
        ]
        for (key, desc) in generalShortcuts {
            content.append(NSAttributedString(string: "  \(key)", attributes: keyAttrs))
            content.append(NSAttributedString(string: "  \(desc)\n", attributes: descAttrs))
        }
        content.append(NSAttributedString(string: "\n", attributes: descAttrs))

        // Split & navigation
        content.append(NSAttributedString(string: "Splits & Navigation\n", attributes: sectionAttrs))
        let splitShortcuts: [(String, String)] = [
            ("Cmd+Shift+D", "Split Right"),
            ("Cmd+Shift+E", "Split Down"),
            ("Cmd+Shift+Z", "Toggle Pane Zoom"),
            ("Opt+H/J/K/L", "Focus pane (left/down/up/right)"),
            ("Cmd+Shift+]", "Next Panel Tab"),
            ("Cmd+Shift+[", "Previous Panel Tab"),
            ("Cmd+Shift+}", "Next Pane"),
            ("Cmd+Shift+{", "Previous Pane"),
        ]
        for (key, desc) in splitShortcuts {
            content.append(NSAttributedString(string: "  \(key)", attributes: keyAttrs))
            content.append(NSAttributedString(string: "  \(desc)\n", attributes: descAttrs))
        }
        content.append(NSAttributedString(string: "\n", attributes: descAttrs))

        // Zoom
        content.append(NSAttributedString(string: "Zoom\n", attributes: sectionAttrs))
        let zoomShortcuts: [(String, String)] = [
            ("Cmd+=", "Zoom In"),
            ("Cmd+-", "Zoom Out"),
            ("Cmd+0", "Reset Zoom"),
        ]
        for (key, desc) in zoomShortcuts {
            content.append(NSAttributedString(string: "  \(key)", attributes: keyAttrs))
            content.append(NSAttributedString(string: "  \(desc)\n", attributes: descAttrs))
        }
        content.append(NSAttributedString(string: "\n", attributes: descAttrs))

        // Browser Vimium keys
        content.append(NSAttributedString(string: "Browser (Vimium Mode)\n", attributes: sectionAttrs))
        let vimiumShortcuts: [(String, String)] = [
            ("f", "Show link hints"),
            ("j / k", "Scroll down / up"),
            ("g / G", "Top / Bottom"),
            ("H / L", "Back / Forward"),
            ("r", "Reload"),
            ("Escape", "Cancel hints"),
        ]
        for (key, desc) in vimiumShortcuts {
            content.append(NSAttributedString(string: "  \(key)", attributes: keyAttrs))
            content.append(NSAttributedString(string: "  \(desc)\n", attributes: descAttrs))
        }
        content.append(NSAttributedString(string: "\n", attributes: descAttrs))

        // Usage tips
        content.append(NSAttributedString(string: "Tips\n", attributes: sectionAttrs))
        let tips = [
            "Right-side panels are mutually exclusive — opening one closes others.",
            "Use the leader key (Cmd+B) for tmux-style workspace navigation.",
            "The status bar shows agent status, cost, and queue indicators. Click them to open the corresponding panel.",
            "Settings are persisted in ~/.config/thane/config (Ghostty-style key=value).",
            "Agent queue tasks run headlessly — no workspace or terminal is created.",
        ]
        for tip in tips {
            content.append(NSAttributedString(string: "  • \(tip)\n", attributes: tipAttrs))
        }

        textView.textStorage?.setAttributedString(content)
    }
}
