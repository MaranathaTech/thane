import AppKit
import WebKit
import SwiftTerm

/// Recursive NSSplitView that mirrors the Rust `SplitTree` structure.
///
/// Each workspace has one `SplitContainer`. The split tree can be:
///   - Leaf: a single pane with a TabBarView + panel content
///   - Split: two children separated by a divider (H or V)
///
/// Rebuilding the view tree from bridge state is done by traversing the
/// split tree recursively and creating NSSplitView instances.
@MainActor
final class SplitContainer: NSView {

    private let bridge: RustBridge
    private let workspaceId: String

    /// Tracks whether a pane is zoomed (showing one pane fullscreen).
    private var isZoomed = false
    private var zoomedPaneId: String?
    private var normalView: NSView?

    /// Cached BrowserView instances keyed by panel ID.
    /// Prevents recreating the web view (and losing page state) on rebuild.
    private var browserViews: [String: BrowserView] = [:]

    /// Cached terminal views keyed by panel ID.
    /// Preserves terminal session across rebuilds.
    private var terminalViews: [String: ThaneTerminalView] = [:]

    /// Cached terminal wrapper containers keyed by panel ID.
    /// Each wrapper holds the terminal + search bar overlay + jump-to-bottom button.
    private var terminalWrappers: [String: TerminalWrapperView] = [:]

    /// Process delegates keyed by panel ID (strong refs so they aren't deallocated).
    private var terminalDelegates: [String: TerminalProcessDelegate] = [:]

    /// Pane wrapper containers keyed by panel ID (tab bar + content).
    /// Used for in-place splitting without tearing down terminals.
    private var paneContainers: [String: NSView] = [:]

    /// Strong references to delegate bridges so they aren't deallocated
    /// (BrowserView.delegate is weak).
    private var omnibarBridges: [String: BrowserOmnibarBridge] = [:]

    /// Context menu handlers for terminal panels (strong refs).
    private var contextMenuHandlers: [String: TerminalContextMenuHandler] = [:]

    /// Tracks last scanned content hash per terminal (avoids rescanning same content).
    private var lastScannedRow: [String: Int] = [:]

    /// Tracks findings already reported per terminal to avoid duplicates.
    private var reportedFindings: [String: Set<String>] = [:]

    /// The panel ID that currently has a focus ring drawn.
    private var focusRingPanelId: String?

    /// Whether this container has been built at least once (terminals spawned).
    private(set) var isBuilt = false

    /// Callback for git diff button — wired by MainWindowController.
    var onGitDiff: (() -> Void)?

    /// Timer for polling child process CWD (fallback when shell doesn't send OSC 7).
    private var cwdPollTimer: Timer?

    // MARK: - Init

    init(bridge: RustBridge, workspaceId: String) {
        self.bridge = bridge
        self.workspaceId = workspaceId
        super.init(frame: .zero)
        wantsLayer = true
        startCwdPolling()
    }

    deinit {
        cwdPollTimer?.invalidate()
    }

    /// Poll child process CWD every 2 seconds as fallback for shells that
    /// don't send OSC 7 (macOS zsh default).
    private func startCwdPolling() {
        cwdPollTimer = Timer.scheduledTimer(withTimeInterval: 4.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                // Only poll CWD for the active workspace to avoid wasting cycles
                guard self.bridge.activeWorkspace()?.id == self.workspaceId else { return }
                self.pollAllTerminalCwds()
            }
        }
    }

    private func pollAllTerminalCwds() {
        for (panelId, termView) in terminalViews {
            guard termView.process.running else { continue }
            let childPid = termView.process.shellPid
            guard childPid > 0 else { continue }

            // Read CWD of the shell's foreground process group
            // Use /proc or lsof to find the cwd
            DispatchQueue.global(qos: .utility).async { [weak self] in
                guard let cwd = Self.readProcessCwd(pid: childPid) else { return }
                DispatchQueue.main.async {
                    MainActor.assumeIsolated {
                        guard let self else { return }
                        self.bridge.updatePanelCwd(
                            workspaceId: self.workspaceId,
                            panelId: panelId,
                            cwd: cwd
                        )
                    }
                }
            }
        }
    }

    /// Scan terminal buffers for new security-relevant content (sensitive files, PII, agent invocations).
    /// Called periodically from the metadata refresh timer.
    func scanTerminalBuffers() {
        for (panelId, termView) in terminalViews {
            // Update per-panel agent detection so audit events are attributed.
            let shellPid = termView.process.shellPid
            if shellPid > 0 {
                bridge.updatePanelAgent(panelId: panelId, shellPid: shellPid)
            }
            let agent = bridge.agentForPanel(panelId)

            let terminal = termView.getTerminal()
            let rows = terminal.rows

            // Read visible terminal content via public API (getLine uses visible row indices)
            var lines: [String] = []
            for row in 0..<rows {
                if let bufLine = terminal.getLine(row: row) {
                    var text = ""
                    for col in 0..<bufLine.count {
                        text.append(bufLine[col].getCharacter())
                    }
                    let trimmed = text.trimmingCharacters(in: .whitespaces)
                    if !trimmed.isEmpty { lines.append(trimmed) }
                }
            }
            let text = lines.joined(separator: "\n")
            guard !text.isEmpty else { continue }

            // Hash to detect changes — skip if unchanged since last scan
            let hash = text.hashValue
            if lastScannedRow[panelId] == hash { continue }
            lastScannedRow[panelId] = hash

            let result = AuditScanner.scan(text: text)
            var reported = reportedFindings[panelId] ?? []
            for finding in result.sensitiveFiles {
                let key = "\(finding.eventType):\(finding.path)"
                guard reported.insert(key).inserted else { continue }
                bridge.logAuditEvent(
                    workspaceId: workspaceId, panelId: panelId,
                    eventType: finding.eventType, severity: finding.severity,
                    description: "Sensitive file detected: \(finding.path)",
                    metadata: ["path": finding.path],
                    agentName: agent)
            }
            for pii in result.piiFindings {
                let key = "PII:\(pii)"
                guard reported.insert(key).inserted else { continue }
                bridge.logAuditEvent(
                    workspaceId: workspaceId, panelId: panelId,
                    eventType: "PiiDetected", severity: .alert,
                    description: pii, metadata: [:],
                    agentName: agent)
            }
            for invocation in result.agentInvocations {
                let key = "Agent:\(invocation.prefix(50))"
                guard reported.insert(key).inserted else { continue }
                bridge.logAuditEvent(
                    workspaceId: workspaceId, panelId: panelId,
                    eventType: "AgentInvocation", severity: .info,
                    description: "Agent command detected",
                    metadata: ["command": String(invocation.prefix(200))],
                    agentName: agent)
            }
            reportedFindings[panelId] = reported
        }
    }

    /// Read a process's current working directory on macOS using lsof.
    private static func readProcessCwd(pid: Int32) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/lsof")
        // -a = AND (combine -p and -d filters), -p = PID, -d cwd = only cwd fd, -Fn = name output
        process.arguments = ["-a", "-p", "\(pid)", "-d", "cwd", "-Fn"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()
        do {
            try process.run()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else { return nil }
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            let output = String(data: data, encoding: .utf8) ?? ""
            // Parse: output is "p<pid>\nfcwd\nn<path>\n"
            for line in output.split(separator: "\n") {
                if line.hasPrefix("n/") {
                    return String(line.dropFirst(1))
                }
            }
        } catch {}
        return nil
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public API

    /// Rebuild the entire split tree from bridge state.
    /// Uses a "safe reparent" strategy: terminal views are moved to a hidden
    /// holding area before the old tree is torn down, preserving their Metal
    /// rendering context. Then they're placed into the new tree.
    func rebuild() {
        // 0. Save the current first responder so we can restore focus after rebuild.
        //    Periodic timers trigger rebuilds while the user types, so losing focus
        //    is extremely disruptive.
        let savedResponder = window?.firstResponder

        // 1. Reparent all live terminal views to a hidden holding view
        //    so they stay in the window and their rendering survives.
        let holder = NSView(frame: NSRect(x: -9999, y: -9999, width: 1, height: 1))
        holder.isHidden = true
        window?.contentView?.addSubview(holder)
        for (_, wrapper) in terminalWrappers {
            wrapper.removeFromSuperview()
            holder.addSubview(wrapper)
        }

        // 2. Tear down the old view tree (terminals are safe in holder)
        subviews.forEach { $0.removeFromSuperview() }
        paneContainers.removeAll()

        // 3. Build new tree from bridge state
        guard let tree = bridge.splitTree(for: workspaceId) else {
            holder.removeFromSuperview()
            return
        }
        let rootView = buildFromNode(tree)
        rootView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(rootView)
        NSLayoutConstraint.activate([
            rootView.topAnchor.constraint(equalTo: topAnchor),
            rootView.leadingAnchor.constraint(equalTo: leadingAnchor),
            rootView.trailingAnchor.constraint(equalTo: trailingAnchor),
            rootView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // 4. Clean up the holder (terminals are now in the new tree)
        holder.removeFromSuperview()

        // 5. Prune terminals for panels that no longer exist
        pruneViews()

        // 6. Restore first responder — the view was removed and re-added during
        //    rebuild, which causes macOS to clear first responder automatically.
        if let responder = savedResponder as? NSView, responder.window != nil {
            window?.makeFirstResponder(responder)
        }

        isBuilt = true
        updateFocusRing()
    }

    /// Recursively build the view tree from a SplitNode.
    private func buildFromNode(_ node: SplitNode) -> NSView {
        switch node {
        case .leaf(let panel):
            return buildPaneView(panels: [panel])
        case .split(let orientation, let first, let second):
            let firstView = buildFromNode(first)
            let secondView = buildFromNode(second)
            // .horizontal = split right = side by side = NSSplitView.isVertical = true
            // .vertical = split down = stacked = NSSplitView.isVertical = false
            let isVertical = (orientation == .horizontal)
            return buildSplitView(isVertical: isVertical, first: firstView, second: secondView)
        }
    }

    /// Toggle zoom on the focused pane.
    func toggleZoom() {
        isZoomed.toggle()
        rebuild()
    }

    /// Update the focus ring to highlight the currently focused pane.
    func updateFocusRing() {
        guard let focusedPanel = bridge.focusedPanel() else { return }
        let newId = focusedPanel.id

        // Remove old focus ring
        if let oldId = focusRingPanelId, oldId != newId, let oldContainer = paneContainers[oldId] {
            oldContainer.layer?.borderWidth = 0
            oldContainer.layer?.borderColor = nil
        }

        // Draw new focus ring (only when 2+ panes exist)
        if paneContainers.count > 1, let container = paneContainers[newId] {
            container.wantsLayer = true
            container.layer?.borderWidth = 2
            container.layer?.borderColor = ThaneTheme.accentColor.cgColor
        }

        focusRingPanelId = newId
    }

    /// Apply font size to all terminal panels in this container.
    func applyFontSize(_ size: CGFloat) {
        let family = bridge.configFontFamily()
        let font = NSFont(name: family, size: size)
            ?? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
        for tv in terminalViews.values {
            tv.font = font
        }
    }

    /// Apply terminal foreground color from config to all terminals.
    func applyForegroundColor() {
        let fgHex = bridge.configGet(key: "terminal-foreground") ?? "#e4e4e7"
        let color = ThaneTheme.colorFromHex(fgHex)
            ?? NSColor(red: 0.894, green: 0.894, blue: 0.906, alpha: 1.0)
        for tv in terminalViews.values {
            tv.nativeForegroundColor = color
        }
    }

    // MARK: - Tree building

    /// Build a horizontal or vertical split view from two children.
    /// The divider is draggable — users can resize panes freely.
    func buildSplitView(
        isVertical: Bool,
        first: NSView,
        second: NSView,
        dividerPosition: CGFloat = 0.5
    ) -> NSView {
        let container = NSView()
        container.wantsLayer = true
        container.translatesAutoresizingMaskIntoConstraints = false

        first.translatesAutoresizingMaskIntoConstraints = false
        second.translatesAutoresizingMaskIntoConstraints = false

        // Draggable divider (thin visible line + wider hit target)
        let divider = SplitDividerView(isVertical: isVertical)
        divider.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(first)
        container.addSubview(divider)
        container.addSubview(second)

        // The ratio constraint controls the split position.
        // first.width = container.width * ratio (for vertical splits)
        // first.height = container.height * ratio (for horizontal splits)
        let ratioConstraint: NSLayoutConstraint

        if isVertical {
            // Side by side: first | divider | second
            ratioConstraint = first.widthAnchor.constraint(
                equalTo: container.widthAnchor, multiplier: dividerPosition)
            ratioConstraint.priority = .defaultHigh

            NSLayoutConstraint.activate([
                first.topAnchor.constraint(equalTo: container.topAnchor),
                first.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                first.bottomAnchor.constraint(equalTo: container.bottomAnchor),
                ratioConstraint,
                first.widthAnchor.constraint(greaterThanOrEqualToConstant: 60),

                divider.topAnchor.constraint(equalTo: container.topAnchor),
                divider.leadingAnchor.constraint(equalTo: first.trailingAnchor),
                divider.bottomAnchor.constraint(equalTo: container.bottomAnchor),
                divider.widthAnchor.constraint(equalToConstant: 5), // hit target

                second.topAnchor.constraint(equalTo: container.topAnchor),
                second.leadingAnchor.constraint(equalTo: divider.trailingAnchor),
                second.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                second.bottomAnchor.constraint(equalTo: container.bottomAnchor),
                second.widthAnchor.constraint(greaterThanOrEqualToConstant: 60),
            ])
        } else {
            // Stacked: first / divider / second
            ratioConstraint = first.heightAnchor.constraint(
                equalTo: container.heightAnchor, multiplier: dividerPosition)
            ratioConstraint.priority = .defaultHigh

            NSLayoutConstraint.activate([
                first.topAnchor.constraint(equalTo: container.topAnchor),
                first.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                first.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                ratioConstraint,
                first.heightAnchor.constraint(greaterThanOrEqualToConstant: 60),

                divider.topAnchor.constraint(equalTo: first.bottomAnchor),
                divider.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                divider.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                divider.heightAnchor.constraint(equalToConstant: 5), // hit target

                second.topAnchor.constraint(equalTo: divider.bottomAnchor),
                second.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                second.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                second.bottomAnchor.constraint(equalTo: container.bottomAnchor),
                second.heightAnchor.constraint(greaterThanOrEqualToConstant: 60),
            ])
        }

        // Use a mutable state object so the drag closure can recreate constraints
        // (NSLayoutConstraint.multiplier is read-only).
        let state = DividerDragState(constraint: ratioConstraint, firstView: first, container: container, isVertical: isVertical)
        divider.onDrag = { delta in
            state.applyDelta(delta)
        }

        return container
    }

    /// Build a leaf pane view: tab bar + panel content.
    private func buildPaneView(panels: [PanelInfoDTO]) -> NSView {
        let container = NSView()
        container.wantsLayer = true

        // Register this container for the panel
        if let panel = panels.first {
            paneContainers[panel.id] = container
        }

        // Tab bar at the top
        // Capture the panel ID so split/close operate on THIS pane, not whatever
        // happens to be focused (mirrors Linux GTK behaviour).
        let panelIdForPane = panels.first?.id
        let tabBar = TabBarView(
            panels: panels,
            onSelect: { [weak self] panelId in
                _ = self?.bridge.selectPanel(panelId: panelId)
                self?.updateFocusRing()
            },
            onClose: { [weak self] panelId in
                _ = try? self?.bridge.closePanel(panelId: panelId)
            },
            onSplitRight: { [weak self] in
                if let id = panelIdForPane {
                    _ = self?.bridge.selectPanel(panelId: id)
                }
                _ = try? self?.bridge.splitTerminal(orientation: .horizontal)
            },
            onSplitDown: { [weak self] in
                if let id = panelIdForPane {
                    _ = self?.bridge.selectPanel(panelId: id)
                }
                _ = try? self?.bridge.splitTerminal(orientation: .vertical)
            },
            onClosePane: { [weak self] in
                if let id = panelIdForPane {
                    _ = self?.bridge.selectPanel(panelId: id)
                }
                try? self?.bridge.closePane()
            },
            onScreenshot: { [weak self] in
                self?.takeScreenshot()
            },
            onGitDiff: { [weak self] in
                self?.onGitDiff?()
            },
            onFind: { [weak self] in
                self?.toggleFindInTerminal()
            }
        )
        tabBar.onReorder = { [weak self] droppedPanelId, targetPanelId in
            guard let self else { return }
            // Swap the two panels in the tree
            _ = self.bridge.reorderPanel(panelId: droppedPanelId, newIndex:
                self.bridge.listPanels().firstIndex(where: { $0.id == targetPanelId }) ?? 0)
        }
        tabBar.translatesAutoresizingMaskIntoConstraints = false

        // Panel content area — use the specific panel passed in, not the focused one
        let contentView = buildPanelContent(for: panels.first)
        contentView.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(tabBar)
        container.addSubview(contentView)

        NSLayoutConstraint.activate([
            tabBar.topAnchor.constraint(equalTo: container.topAnchor),
            tabBar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            tabBar.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            tabBar.heightAnchor.constraint(equalToConstant: ThaneTheme.tabBarHeight),

            contentView.topAnchor.constraint(equalTo: tabBar.bottomAnchor),
            contentView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            contentView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            contentView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        return container
    }

    /// Build the content view for a panel. Browser panels get a real WKWebView;
    /// terminal panels get a SwiftTerm LocalProcessTerminalView.
    private func buildPanelContent(for panel: PanelInfoDTO?) -> NSView {
        guard let panel else { return buildTerminalView(panelId: UUID().uuidString, cwd: nil) }

        switch panel.panelType {
        case .browser:
            return buildBrowserContent(panel: panel)
        case .terminal:
            return buildTerminalView(panelId: panel.id, cwd: panel.location)
        }
    }

    /// Build a browser panel: OmnibarView + BrowserView stacked vertically.
    private func buildBrowserContent(panel: PanelInfoDTO) -> NSView {
        let container = NSView()
        container.wantsLayer = true

        // Reuse existing BrowserView if we already have one for this panel.
        let browserView: BrowserView
        if let existing = browserViews[panel.id] {
            browserView = existing
        } else {
            browserView = BrowserView(panelId: panel.id, initialURL: panel.location)
            browserViews[panel.id] = browserView
        }

        let omnibar = OmnibarView()
        omnibar.setURL(browserView.currentURL ?? panel.location)
        omnibar.translatesAutoresizingMaskIntoConstraints = false
        browserView.translatesAutoresizingMaskIntoConstraints = false

        // Wire omnibar actions to the browser view.
        omnibar.focusTargetAfterNavigate = browserView.webView
        omnibar.onNavigate = { [weak browserView] urlString in
            browserView?.navigate(to: urlString)
        }
        omnibar.onBack = { [weak browserView] in browserView?.goBack() }
        omnibar.onForward = { [weak browserView] in browserView?.goForward() }
        omnibar.onReload = { [weak browserView] in browserView?.reload() }

        // Update omnibar when browser navigates.
        // Keep a strong reference so the delegate isn't deallocated.
        let bridge = BrowserOmnibarBridge(omnibar: omnibar, bridge: self.bridge, panelId: panel.id)
        omnibarBridges[panel.id] = bridge
        browserView.delegate = bridge

        container.addSubview(omnibar)
        container.addSubview(browserView)

        NSLayoutConstraint.activate([
            omnibar.topAnchor.constraint(equalTo: container.topAnchor),
            omnibar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            omnibar.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            omnibar.heightAnchor.constraint(equalToConstant: ThaneTheme.tabBarHeight),

            browserView.topAnchor.constraint(equalTo: omnibar.bottomAnchor),
            browserView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            browserView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            browserView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        return container
    }

    /// Build a real SwiftTerm terminal view for a panel, wrapped in a container
    /// with search bar overlay and jump-to-bottom button.
    private func buildTerminalView(panelId: String, cwd: String?) -> NSView {
        // Reuse existing wrapper (it was safely held in the holder during rebuild)
        if let existingWrapper = terminalWrappers[panelId] {
            existingWrapper.removeFromSuperview() // detach from holder
            return existingWrapper
        }

        // Give the terminal an initial reasonable frame so it can compute rows/cols
        let termView = ThaneTerminalView(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
        // Use saved font config instead of hardcoded default
        let fontFamily = bridge.configFontFamily()
        let fontSize = CGFloat(bridge.configFontSize())
        termView.font = NSFont(name: fontFamily, size: fontSize)
            ?? ThaneTheme.terminalFont(size: fontSize)

        // Set terminal colors to match theme (foreground is user-configurable)
        termView.nativeBackgroundColor = NSColor(red: 0.047, green: 0.047, blue: 0.055, alpha: 1.0)
        let fgHex = bridge.configGet(key: "terminal-foreground") ?? "#e4e4e7"
        termView.nativeForegroundColor = ThaneTheme.colorFromHex(fgHex)
            ?? NSColor(red: 0.894, green: 0.894, blue: 0.906, alpha: 1.0)

        terminalViews[panelId] = termView

        // Create wrapper with search bar and jump-to-bottom button
        let wrapper = TerminalWrapperView(terminalView: termView)
        terminalWrappers[panelId] = wrapper

        // Terminal right-click context menu
        let handler = TerminalContextMenuHandler()
        handler.terminalView = termView
        handler.onScreenshot = { [weak self] in self?.takeScreenshot() }
        handler.onGitDiff = { [weak self] in self?.onGitDiff?() }
        handler.onFind = { [weak self] in self?.toggleFindInTerminal() }
        contextMenuHandlers[panelId] = handler

        let menu = NSMenu()
        menu.delegate = handler

        let copyItem = NSMenuItem(title: "Copy", action: #selector(TerminalContextMenuHandler.copyClicked), keyEquivalent: "")
        copyItem.target = handler
        let pasteItem = NSMenuItem(title: "Paste", action: #selector(TerminalContextMenuHandler.pasteClicked), keyEquivalent: "")
        pasteItem.target = handler
        let screenshotItem = NSMenuItem(title: "Take Screenshot", action: #selector(TerminalContextMenuHandler.screenshotClicked), keyEquivalent: "")
        screenshotItem.target = handler
        let gitDiffItem = NSMenuItem(title: "Git Diff", action: #selector(TerminalContextMenuHandler.gitDiffClicked), keyEquivalent: "")
        gitDiffItem.target = handler
        menu.addItem(copyItem)
        menu.addItem(pasteItem)
        menu.addItem(NSMenuItem.separator())
        menu.addItem(screenshotItem)
        menu.addItem(gitDiffItem)
        menu.addItem(NSMenuItem.separator())
        let findItem = NSMenuItem(title: "Find in Terminal", action: #selector(TerminalContextMenuHandler.findClicked), keyEquivalent: "")
        findItem.target = handler
        menu.addItem(findItem)
        termView.menu = menu

        // Set up delegate to track CWD changes
        let processDelegate = TerminalProcessDelegate(
            bridge: bridge,
            workspaceId: workspaceId,
            panelId: panelId
        )
        processDelegate.onShellReady = { [weak wrapper] in
            wrapper?.hideLoadingOverlay()
        }
        terminalDelegates[panelId] = processDelegate
        termView.processDelegate = processDelegate

        // Fallback: hide loading overlay after 2 seconds if shell signals don't fire
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak wrapper] in
            wrapper?.hideLoadingOverlay()
        }

        // Wire hyperlink click handler (bridge ref for opening embedded browser)
        termView.linkBridge = bridge

        // Update focused panel when this terminal is clicked (click-to-focus)
        wrapper.onClicked = { [weak self] in
            _ = self?.bridge.selectPanel(panelId: panelId)
            self?.updateFocusRing()
        }

        // Defer process start until after the view is in the hierarchy and laid out
        let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        let workingDir = cwd ?? FileManager.default.homeDirectoryForCurrentUser.path
        let socketPath = bridge.socketPath()
        let sandboxCmd = bridge.sandboxGetCommand(workspaceId: workspaceId, shell: shell)
        if let cmd = sandboxCmd {
            NSLog("thane: sandbox command: \(cmd.executable) \(cmd.args.joined(separator: " "))")
        }
        DispatchQueue.main.async {
            var env = Terminal.getEnvironmentVariables(termName: "xterm-256color")
            env.append("TERM_PROGRAM=thane")
            if !socketPath.isEmpty {
                env.append("THANE_SOCKET_PATH=\(socketPath)")
            }
            if let cmd = sandboxCmd {
                for envVar in cmd.extraEnv {
                    env.append(envVar)
                }
                termView.startProcess(
                    executable: cmd.executable,
                    args: cmd.args,
                    environment: env,
                    execName: nil,
                    currentDirectory: workingDir
                )
            } else {
                termView.startProcess(
                    executable: shell,
                    args: ["-l"],
                    environment: env,
                    execName: "-" + (shell as NSString).lastPathComponent,
                    currentDirectory: workingDir
                )
            }

            // Feed saved scrollback into the terminal (dimmed, matching Linux behavior)
            if let scrollback = self.bridge.panelScrollback[panelId], !scrollback.isEmpty {
                // Small delay to let the terminal initialize before feeding
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak termView] in
                    guard let termView else { return }
                    let dimPrefix = "\u{1b}[2m"
                    let resetSuffix = "\u{1b}[0m\r\n"
                    let lines = scrollback.components(separatedBy: "\n")
                    let feedText = dimPrefix + lines.joined(separator: "\r\n") + resetSuffix
                    termView.feed(text: feedText)
                }
                self.bridge.panelScrollback.removeValue(forKey: panelId)
            }
        }

        return wrapper
    }

    // MARK: - Find in Terminal

    /// Toggle find bar in the focused terminal.
    func toggleFindInTerminal() {
        guard let panel = bridge.focusedPanel(), panel.panelType == .terminal else { return }
        if let wrapper = terminalWrappers[panel.id] {
            wrapper.toggleSearchBar()
        }
    }

    // MARK: - Scrollback Extraction

    /// Extract scrollback text for a terminal panel, truncated to maxLines.
    func getScrollbackText(panelId: String, maxLines: Int = 5000) -> String? {
        guard let termView = terminalViews[panelId] else { return nil }
        let terminal = termView.getTerminal()
        let data = terminal.getBufferAsData()
        guard let text = String(data: data, encoding: .utf8), !text.isEmpty else { return nil }

        var lines = text.components(separatedBy: "\n")

        // Trim trailing whitespace per line
        lines = lines.map { $0.replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression) }

        // Remove trailing empty lines
        while let last = lines.last, last.isEmpty {
            lines.removeLast()
        }

        // Keep the most recent lines if over limit
        if lines.count > maxLines {
            lines = Array(lines.suffix(maxLines))
        }

        let result = lines.joined(separator: "\n")
        return result.isEmpty ? nil : result
    }

    // MARK: - Screenshot

    /// Take a screenshot of the terminal or browser content and save to /tmp.
    private func takeScreenshot() {
        guard let panel = bridge.focusedPanel() else { return }

        if let termView = terminalViews[panel.id] {
            guard let rep = termView.bitmapImageRepForCachingDisplay(in: termView.bounds) else { return }
            termView.cacheDisplay(in: termView.bounds, to: rep)
            let image = NSImage(size: termView.bounds.size)
            image.addRepresentation(rep)
            guard let tiffData = image.tiffRepresentation,
                  let bitmap = NSBitmapImageRep(data: tiffData),
                  let pngData = bitmap.representation(using: .png, properties: [:]) else { return }
            let uniqueId = ProcessInfo.processInfo.globallyUniqueString
            let path = "/tmp/thane_screenshot_\(uniqueId).png"
            try? pngData.write(to: URL(fileURLWithPath: path))
            copyAndShowScreenshotAlert(path: path)
        } else if let browserView = browserViews[panel.id] {
            browserView.takeScreenshot { image in
                guard let image else {
                    NSLog("thane: browser screenshot returned nil image")
                    return
                }
                DispatchQueue.main.async { [weak self] in
                    guard let tiffData = image.tiffRepresentation,
                          let bitmap = NSBitmapImageRep(data: tiffData),
                          let pngData = bitmap.representation(using: .png, properties: [:]) else {
                        NSLog("thane: browser screenshot PNG conversion failed")
                        return
                    }
                    let uniqueId = ProcessInfo.processInfo.globallyUniqueString
                    let path = "/tmp/thane_screenshot_\(uniqueId).png"
                    do {
                        try pngData.write(to: URL(fileURLWithPath: path))
                    } catch {
                        NSLog("thane: browser screenshot write failed: \(error)")
                        return
                    }
                    // Show alert even if self is nil (use class-level alert)
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(path, forType: .string)
                    NSLog("thane: browser screenshot saved to \(path)")

                    let alert = NSAlert()
                    alert.messageText = "Screenshot Captured"
                    alert.informativeText = "Screenshot saved and path copied to clipboard.\n\(path)"
                    alert.alertStyle = .informational
                    alert.addButton(withTitle: "OK")
                    if let window = self?.window ?? NSApp.keyWindow {
                        alert.beginSheetModal(for: window)
                    } else {
                        alert.runModal()
                    }
                }
            }
        }
    }

    private func copyAndShowScreenshotAlert(path: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(path, forType: .string)
        NSLog("thane: screenshot saved to \(path)")

        let alert = NSAlert()
        alert.messageText = "Screenshot Captured"
        alert.informativeText = "Screenshot saved and path copied to clipboard.\n\(path)"
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    // MARK: - Shell PIDs (for port scanning)

    /// Return shell PIDs for all running terminal processes in this workspace.
    func shellPids() -> [Int32] {
        terminalViews.values.compactMap { tv in
            tv.process.running ? tv.process.shellPid : nil
        }.filter { $0 > 0 }
    }

    // MARK: - Browser access

    /// Get the BrowserView for a given panel ID, if it exists.
    func browserView(forPanelId panelId: String) -> BrowserView? {
        browserViews[panelId]
    }

    /// Get the currently focused BrowserView, if the focused panel is a browser.
    func focusedBrowserView() -> BrowserView? {
        guard let panel = bridge.focusedPanel(), panel.panelType == .browser else { return nil }
        return browserViews[panel.id]
    }

    /// Remove cached views for panels that no longer exist.
    func pruneViews() {
        let currentPanelIds = Set(bridge.listPanels().map(\.id))
        for id in browserViews.keys where !currentPanelIds.contains(id) {
            browserViews.removeValue(forKey: id)
            omnibarBridges.removeValue(forKey: id)
        }
        for id in terminalViews.keys where !currentPanelIds.contains(id) {
            terminalViews[id]?.terminate()
            terminalViews.removeValue(forKey: id)
            terminalWrappers.removeValue(forKey: id)
        }
    }
}

// MARK: - SplitDividerView (draggable pane divider)

/// A thin divider line with a wider invisible hit target for dragging.
/// Shows resize cursor on hover and highlights on drag.
@MainActor
private final class SplitDividerView: NSView {
    private let isVertical: Bool
    var onDrag: ((CGFloat) -> Void)?

    init(isVertical: Bool) {
        self.isVertical = isVertical
        super.init(frame: .zero)
        wantsLayer = true
        // Transparent background — the visible line is drawn inside
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func draw(_ dirtyRect: NSRect) {
        ThaneTheme.dividerColor.setFill()
        if isVertical {
            // Center 1px line vertically within the 5px hit target
            let lineRect = NSRect(x: bounds.midX - 0.5, y: 0, width: 1, height: bounds.height)
            lineRect.fill()
        } else {
            let lineRect = NSRect(x: 0, y: bounds.midY - 0.5, width: bounds.width, height: 1)
            lineRect.fill()
        }
    }

    override func resetCursorRects() {
        let cursor: NSCursor = isVertical ? .resizeLeftRight : .resizeUpDown
        addCursorRect(bounds, cursor: cursor)
    }

    override func mouseDown(with event: NSEvent) {
        var lastPoint = convert(event.locationInWindow, from: nil)

        window?.trackEvents(matching: [.leftMouseDragged, .leftMouseUp], timeout: .infinity, mode: .eventTracking) { trackEvent, stop in
            guard let trackEvent else { stop.pointee = true; return }
            if trackEvent.type == .leftMouseUp {
                stop.pointee = true
                return
            }
            let current = self.convert(trackEvent.locationInWindow, from: nil)
            let delta = self.isVertical ? (current.x - lastPoint.x) : (current.y - lastPoint.y)
            // For vertical stacks, positive mouse Y = up, but we want downward = more first
            let adjustedDelta = self.isVertical ? delta : -delta
            self.onDrag?(adjustedDelta)
            lastPoint = current
        }
    }
}

// MARK: - DividerDragState

/// Mutable state for the divider drag closure. Holds the current ratio constraint
/// and recreates it on each drag delta (since NSLayoutConstraint.multiplier is read-only).
@MainActor
private final class DividerDragState {
    private var constraint: NSLayoutConstraint
    private weak var firstView: NSView?
    private weak var container: NSView?
    private let isVertical: Bool

    init(constraint: NSLayoutConstraint, firstView: NSView, container: NSView, isVertical: Bool) {
        self.constraint = constraint
        self.firstView = firstView
        self.container = container
        self.isVertical = isVertical
    }

    func applyDelta(_ delta: CGFloat) {
        guard let container, let firstView else { return }
        let totalSize = isVertical ? container.bounds.width : container.bounds.height
        guard totalSize > 0 else { return }

        let currentRatio = constraint.multiplier
        let newRatio = min(max(currentRatio + delta / totalSize, 0.1), 0.9)

        constraint.isActive = false
        let newConstraint: NSLayoutConstraint
        if isVertical {
            newConstraint = firstView.widthAnchor.constraint(
                equalTo: container.widthAnchor, multiplier: newRatio)
        } else {
            newConstraint = firstView.heightAnchor.constraint(
                equalTo: container.heightAnchor, multiplier: newRatio)
        }
        newConstraint.priority = .defaultHigh
        newConstraint.isActive = true
        constraint = newConstraint
    }
}

// MARK: - TerminalProcessDelegate

/// Receives process callbacks from SwiftTerm and updates bridge state (CWD tracking).
@MainActor
final class TerminalProcessDelegate: LocalProcessTerminalViewDelegate {
    private weak var bridge: RustBridge?
    private let workspaceId: String
    private let panelId: String

    /// Called once when the shell is ready (title set or CWD reported).
    var onShellReady: (() -> Void)?
    private var shellReadyFired = false

    init(bridge: RustBridge, workspaceId: String, panelId: String) {
        self.bridge = bridge
        self.workspaceId = workspaceId
        self.panelId = panelId
    }

    private func fireShellReady() {
        guard !shellReadyFired else { return }
        shellReadyFired = true
        onShellReady?()
        onShellReady = nil
    }

    nonisolated func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}

    nonisolated func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                self?.fireShellReady()
            }
        }
    }

    nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        guard let dir = directory else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            MainActor.assumeIsolated {
                self.fireShellReady()
                self.bridge?.updatePanelCwd(
                    workspaceId: self.workspaceId,
                    panelId: self.panelId,
                    cwd: dir
                )
                // Scan the new CWD for sensitive file access
                let result = AuditScanner.scan(text: dir)
                for finding in result.sensitiveFiles {
                    self.bridge?.logAuditEvent(
                        workspaceId: self.workspaceId,
                        panelId: self.panelId,
                        eventType: finding.eventType,
                        severity: finding.severity,
                        description: "Sensitive file path: \(finding.path)",
                        metadata: ["path": finding.path],
                        agentName: self.bridge?.agentForPanel(self.panelId)
                    )
                }
            }
        }
    }

    nonisolated func processTerminated(source: TerminalView, exitCode: Int32?) {}
}

// MARK: - BrowserOmnibarBridge

/// Bridges BrowserView delegate callbacks to update the OmnibarView
/// and persist the current URL to the bridge for session save/restore.
@MainActor
final class BrowserOmnibarBridge: BrowserViewDelegate {
    private weak var omnibar: OmnibarView?
    private weak var bridge: RustBridge?
    private let panelId: String

    init(omnibar: OmnibarView, bridge: RustBridge, panelId: String) {
        self.omnibar = omnibar
        self.bridge = bridge
        self.panelId = panelId
    }

    func browserView(_ view: BrowserView, titleDidChange title: String) {
        // Title updates could be used to update tab bar labels.
    }

    func browserView(_ view: BrowserView, urlDidChange url: String) {
        omnibar?.setURL(url)
        bridge?.updatePanelCwd(panelId: panelId, cwd: url)
    }

    func browserView(_ view: BrowserView, loadingStateChanged isLoading: Bool) {
        // Could show a loading indicator in the omnibar.
    }
}

// MARK: - Terminal Context Menu Handler

@MainActor
final class TerminalContextMenuHandler: NSObject, NSMenuDelegate {
    weak var terminalView: LocalProcessTerminalView?
    var onScreenshot: (() -> Void)?
    var onGitDiff: (() -> Void)?
    var onFind: (() -> Void)?

    @objc func copyClicked() { terminalView?.copy(self) }
    @objc func pasteClicked() { terminalView?.paste(self) }
    @objc func screenshotClicked() { onScreenshot?() }
    @objc func gitDiffClicked() { onGitDiff?() }
    @objc func findClicked() { onFind?() }

    nonisolated func menuNeedsUpdate(_ menu: NSMenu) {
        // All items always enabled — Copy will just be a no-op if nothing selected
    }
}

// MARK: - ThaneTerminalView

/// Subclass of `LocalProcessTerminalView` that notifies its wrapper when scroll position changes
/// and handles hyperlink clicks (opening in embedded browser or system browser).
@MainActor
final class ThaneTerminalView: LocalProcessTerminalView {
    /// Callback fired whenever the terminal scrolls (position is 0.0 at top, 1.0 at bottom).
    var onScrollPositionChanged: ((Double) -> Void)?

    /// Bridge reference for opening URLs in the embedded browser.
    weak var linkBridge: RustBridge?

    override func scrolled(source: TerminalView, position: Double) {
        super.scrolled(source: source, position: position)
        onScrollPositionChanged?(position)
    }

    /// Scroll the terminal to the very bottom of the scrollback.
    func scrollToBottom() {
        scroll(toPosition: 1.0)
    }

    /// Handle hyperlink clicks from the terminal.
    ///
    /// - Normal click (no Shift): opens the URL in an embedded browser pane via horizontal split.
    /// - Shift+click: opens the URL in the system default browser.
    func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        guard let url = URL(string: link) else { return }

        let shiftHeld = NSEvent.modifierFlags.contains(.shift)

        if shiftHeld {
            NSWorkspace.shared.open(url)
        } else if let bridge = linkBridge {
            _ = try? bridge.splitBrowser(url: link, orientation: .horizontal)
        } else {
            // Fallback: open in system browser if no bridge is available
            NSWorkspace.shared.open(url)
        }
    }
}

// MARK: - TerminalWrapperView

/// Wraps a `ThaneTerminalView` with a search bar overlay at the top and a
/// jump-to-bottom floating button at the bottom-right corner.
@MainActor
final class TerminalWrapperView: NSView {

    let terminalView: ThaneTerminalView
    private let searchBar: TerminalSearchBarView
    private let jumpToBottomButton: NSButton
    private var loadingOverlay: NSView?

    /// Callback fired when any mouse-down lands inside this wrapper (focus tracking).
    var onClicked: (() -> Void)?

    private var searchBarTopConstraint: NSLayoutConstraint!
    private var isSearchBarVisible = false

    private static let searchBarHeight: CGFloat = 30
    private static let jumpButtonSize: CGFloat = 32
    private static let jumpButtonMargin: CGFloat = 12

    private var localMonitor: Any?

    init(terminalView: ThaneTerminalView) {
        self.terminalView = terminalView
        self.searchBar = TerminalSearchBarView()
        self.jumpToBottomButton = NSButton(frame: .zero)
        super.init(frame: .zero)

        wantsLayer = true
        setupTerminalView()
        setupSearchBar()
        setupJumpToBottomButton()
        setupLoadingOverlay()
        wireCallbacks()
        installClickMonitor()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        if let monitor = localMonitor {
            NSEvent.removeMonitor(monitor)
        }
    }

    /// Install a local event monitor that fires `onClicked` when a mouse-down
    /// lands anywhere inside this wrapper (including subviews like the terminal).
    private func installClickMonitor() {
        localMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            guard let self else { return event }
            if let window = self.window,
               event.window === window {
                let point = self.convert(event.locationInWindow, from: nil)
                if self.bounds.contains(point) {
                    self.onClicked?()
                }
            }
            return event
        }
    }

    // MARK: - Setup

    private func setupTerminalView() {
        terminalView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(terminalView)

        NSLayoutConstraint.activate([
            terminalView.topAnchor.constraint(equalTo: topAnchor),
            terminalView.leadingAnchor.constraint(equalTo: leadingAnchor),
            terminalView.trailingAnchor.constraint(equalTo: trailingAnchor),
            terminalView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    private func setupSearchBar() {
        searchBar.translatesAutoresizingMaskIntoConstraints = false
        addSubview(searchBar)

        // Start hidden above the view (negative top offset)
        searchBarTopConstraint = searchBar.topAnchor.constraint(
            equalTo: topAnchor,
            constant: -Self.searchBarHeight
        )

        NSLayoutConstraint.activate([
            searchBarTopConstraint,
            searchBar.leadingAnchor.constraint(equalTo: leadingAnchor),
            searchBar.trailingAnchor.constraint(equalTo: trailingAnchor),
            searchBar.heightAnchor.constraint(equalToConstant: Self.searchBarHeight),
        ])

        searchBar.isHidden = true
    }

    private func setupLoadingOverlay() {
        let overlay = NSView()
        overlay.wantsLayer = true
        overlay.layer?.backgroundColor = NSColor(red: 0.047, green: 0.047, blue: 0.055, alpha: 1.0).cgColor
        overlay.translatesAutoresizingMaskIntoConstraints = false
        addSubview(overlay)

        NSLayoutConstraint.activate([
            overlay.topAnchor.constraint(equalTo: topAnchor),
            overlay.leadingAnchor.constraint(equalTo: leadingAnchor),
            overlay.trailingAnchor.constraint(equalTo: trailingAnchor),
            overlay.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        let spinner = NSProgressIndicator()
        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.translatesAutoresizingMaskIntoConstraints = false
        spinner.startAnimation(nil)
        overlay.addSubview(spinner)

        NSLayoutConstraint.activate([
            spinner.centerXAnchor.constraint(equalTo: overlay.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: overlay.centerYAnchor),
        ])

        loadingOverlay = overlay
    }

    /// Hide the loading overlay (called when shell is ready).
    func hideLoadingOverlay() {
        guard let overlay = loadingOverlay else { return }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.15
            overlay.animator().alphaValue = 0
        } completionHandler: { [weak self] in
            self?.loadingOverlay?.removeFromSuperview()
            self?.loadingOverlay = nil
        }
    }

    private func setupJumpToBottomButton() {
        jumpToBottomButton.translatesAutoresizingMaskIntoConstraints = false
        jumpToBottomButton.wantsLayer = true
        jumpToBottomButton.isBordered = false
        jumpToBottomButton.bezelStyle = .regularSquare
        jumpToBottomButton.title = ""

        // Down arrow icon
        if let arrowImage = NSImage(systemSymbolName: "arrow.down", accessibilityDescription: "Scroll to bottom") {
            let config = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
            jumpToBottomButton.image = arrowImage.withSymbolConfiguration(config)
        }
        jumpToBottomButton.contentTintColor = .white

        // Indigo background, rounded
        jumpToBottomButton.layer?.backgroundColor = ThaneTheme.accentColor.cgColor
        jumpToBottomButton.layer?.cornerRadius = Self.jumpButtonSize / 2
        jumpToBottomButton.layer?.shadowColor = NSColor.black.cgColor
        jumpToBottomButton.layer?.shadowOffset = CGSize(width: 0, height: -2)
        jumpToBottomButton.layer?.shadowOpacity = 0.4
        jumpToBottomButton.layer?.shadowRadius = 4

        jumpToBottomButton.isHidden = true

        addSubview(jumpToBottomButton)

        NSLayoutConstraint.activate([
            jumpToBottomButton.widthAnchor.constraint(equalToConstant: Self.jumpButtonSize),
            jumpToBottomButton.heightAnchor.constraint(equalToConstant: Self.jumpButtonSize),
            jumpToBottomButton.trailingAnchor.constraint(
                equalTo: trailingAnchor,
                constant: -Self.jumpButtonMargin
            ),
            jumpToBottomButton.bottomAnchor.constraint(
                equalTo: bottomAnchor,
                constant: -Self.jumpButtonMargin
            ),
        ])
    }

    private func wireCallbacks() {
        // Search bar callbacks
        searchBar.onSearchForward = { [weak self] text in
            guard let self, !text.isEmpty else { return }
            self.terminalView.findNext(text)
        }
        searchBar.onSearchBackward = { [weak self] text in
            guard let self, !text.isEmpty else { return }
            self.terminalView.findPrevious(text)
        }
        searchBar.onClose = { [weak self] in
            self?.hideSearchBar()
        }

        // Jump-to-bottom button
        jumpToBottomButton.target = self
        jumpToBottomButton.action = #selector(jumpToBottomClicked)

        // Scroll position monitoring
        terminalView.onScrollPositionChanged = { [weak self] position in
            guard let self else { return }
            // Show button when not at the bottom (position < 1.0 means scrolled up)
            let isScrolledUp = position < 0.999
            if self.jumpToBottomButton.isHidden == isScrolledUp {
                self.jumpToBottomButton.isHidden = !isScrolledUp
            }
        }
    }

    // MARK: - Public API

    func toggleSearchBar() {
        if isSearchBarVisible {
            hideSearchBar()
        } else {
            showSearchBar()
        }
    }

    // MARK: - Private

    private func showSearchBar() {
        isSearchBarVisible = true
        searchBar.isHidden = false
        searchBarTopConstraint.constant = 0

        NSAnimationContext.runAnimationGroup { context in
            context.duration = ThaneTheme.animationDuration
            context.allowsImplicitAnimation = true
            self.layoutSubtreeIfNeeded()
        }

        searchBar.focusSearchField()
    }

    private func hideSearchBar() {
        isSearchBarVisible = false
        searchBarTopConstraint.constant = -Self.searchBarHeight

        NSAnimationContext.runAnimationGroup({ context in
            context.duration = ThaneTheme.animationDuration
            context.allowsImplicitAnimation = true
            self.layoutSubtreeIfNeeded()
        }, completionHandler: {
            self.searchBar.isHidden = true
        })

        // Return focus to terminal
        window?.makeFirstResponder(terminalView)
    }

    @objc private func jumpToBottomClicked() {
        terminalView.scrollToBottom()
    }
}

// MARK: - TerminalSearchBarView

/// Compact search bar overlay for find-in-terminal.
/// Contains a text field, previous/next buttons, and a close button.
@MainActor
final class TerminalSearchBarView: NSView, NSTextFieldDelegate {

    var onSearchForward: ((String) -> Void)?
    var onSearchBackward: ((String) -> Void)?
    var onClose: (() -> Void)?

    private let searchField: NSTextField
    private let prevButton: NSButton
    private let nextButton: NSButton
    private let closeButton: NSButton

    override init(frame: NSRect) {
        searchField = NSTextField(frame: .zero)
        prevButton = NSButton(frame: .zero)
        nextButton = NSButton(frame: .zero)
        closeButton = NSButton(frame: .zero)
        super.init(frame: frame)

        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.sidebarBackground.cgColor
        layer?.borderColor = ThaneTheme.dividerColor.cgColor
        layer?.borderWidth = 1

        setupSearchField()
        setupButtons()
        setupLayout()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func focusSearchField() {
        window?.makeFirstResponder(searchField)
    }

    // MARK: - Setup

    private func setupSearchField() {
        searchField.translatesAutoresizingMaskIntoConstraints = false
        searchField.placeholderString = "Find..."
        searchField.font = ThaneTheme.uiFont(size: 12)
        searchField.textColor = ThaneTheme.primaryText
        searchField.backgroundColor = ThaneTheme.raisedBackground
        searchField.isBordered = true
        searchField.isBezeled = true
        searchField.bezelStyle = .roundedBezel
        searchField.focusRingType = .none
        searchField.delegate = self
        addSubview(searchField)
    }

    private func setupButtons() {
        // Previous button (chevron.left)
        configureButton(
            prevButton,
            symbolName: "chevron.left",
            accessibilityLabel: "Previous match",
            action: #selector(prevClicked)
        )

        // Next button (chevron.right)
        configureButton(
            nextButton,
            symbolName: "chevron.right",
            accessibilityLabel: "Next match",
            action: #selector(nextClicked)
        )

        // Close button (xmark)
        configureButton(
            closeButton,
            symbolName: "xmark",
            accessibilityLabel: "Close search",
            action: #selector(closeClicked)
        )

        addSubview(prevButton)
        addSubview(nextButton)
        addSubview(closeButton)
    }

    private func configureButton(_ button: NSButton, symbolName: String, accessibilityLabel: String, action: Selector) {
        button.translatesAutoresizingMaskIntoConstraints = false
        button.isBordered = false
        button.bezelStyle = .regularSquare
        button.title = ""
        button.setAccessibilityLabel(accessibilityLabel)
        if let image = NSImage(systemSymbolName: symbolName, accessibilityDescription: accessibilityLabel) {
            let config = NSImage.SymbolConfiguration(pointSize: 11, weight: .medium)
            button.image = image.withSymbolConfiguration(config)
        }
        button.contentTintColor = ThaneTheme.secondaryText
        button.target = self
        button.action = action
    }

    private func setupLayout() {
        let buttonSize: CGFloat = 24

        NSLayoutConstraint.activate([
            // Search field: left-aligned with padding, stretches to buttons
            searchField.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            searchField.centerYAnchor.constraint(equalTo: centerYAnchor),
            searchField.heightAnchor.constraint(equalToConstant: 22),

            // Previous button
            prevButton.leadingAnchor.constraint(equalTo: searchField.trailingAnchor, constant: 4),
            prevButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            prevButton.widthAnchor.constraint(equalToConstant: buttonSize),
            prevButton.heightAnchor.constraint(equalToConstant: buttonSize),

            // Next button
            nextButton.leadingAnchor.constraint(equalTo: prevButton.trailingAnchor, constant: 2),
            nextButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            nextButton.widthAnchor.constraint(equalToConstant: buttonSize),
            nextButton.heightAnchor.constraint(equalToConstant: buttonSize),

            // Close button
            closeButton.leadingAnchor.constraint(equalTo: nextButton.trailingAnchor, constant: 4),
            closeButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            closeButton.widthAnchor.constraint(equalToConstant: buttonSize),
            closeButton.heightAnchor.constraint(equalToConstant: buttonSize),
            closeButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
        ])
    }

    // MARK: - Actions

    @objc private func prevClicked() {
        onSearchBackward?(searchField.stringValue)
    }

    @objc private func nextClicked() {
        onSearchForward?(searchField.stringValue)
    }

    @objc private func closeClicked() {
        onClose?()
    }

    // MARK: - NSTextFieldDelegate

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        if commandSelector == #selector(NSResponder.insertNewline(_:)) {
            // Enter = search forward, Shift+Enter = search backward
            let event = NSApp.currentEvent
            if event?.modifierFlags.contains(.shift) == true {
                onSearchBackward?(searchField.stringValue)
            } else {
                onSearchForward?(searchField.stringValue)
            }
            return true
        }
        if commandSelector == #selector(NSResponder.cancelOperation(_:)) {
            // Escape = close search bar
            onClose?()
            return true
        }
        return false
    }
}
