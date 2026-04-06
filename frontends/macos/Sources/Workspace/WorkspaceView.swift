import AppKit

/// Manages the split-pane layout for one workspace at a time.
///
/// Acts like a GTK Stack — each workspace has its own `SplitContainer`,
/// and this view swaps between them when the active workspace changes.
@MainActor
final class WorkspaceView: NSView {

    private let bridge: RustBridge

    /// One SplitContainer per workspace ID.
    private var containers: [String: SplitContainer] = [:]

    /// The currently displayed workspace ID.
    private(set) var activeWorkspaceId: String?

    /// Current font size (user zoom level).
    private var currentFontSize: CGFloat = ThaneTheme.defaultFontSize

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = ThaneTheme.backgroundColor.cgColor
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public API

    /// Callback for git diff button — wired by MainWindow.
    var onGitDiff: (() -> Void)?

    /// Rebuild the entire workspace view from bridge state.
    func rebuildFromBridge() {
        let workspaces = bridge.listWorkspaces()
        let activeId = bridge.activeWorkspace()?.id

        // Create containers for any new workspaces
        for ws in workspaces {
            if containers[ws.id] == nil {
                let container = SplitContainer(bridge: bridge, workspaceId: ws.id)
                container.onGitDiff = { [weak self] in self?.onGitDiff?() }
                containers[ws.id] = container
            }
        }

        // Wire scrollback provider so session save can extract terminal content
        bridge.scrollbackProvider = { [weak self] panelId in
            guard let self else { return nil }
            for container in self.containers.values {
                if let text = container.getScrollbackText(panelId: panelId) {
                    return text
                }
            }
            return nil
        }

        // Remove containers for deleted workspaces
        let currentIds = Set(workspaces.map(\.id))
        for id in containers.keys where !currentIds.contains(id) {
            let container = containers.removeValue(forKey: id)
            container?.removeFromSuperview()
        }

        // Show the active workspace (skip full rebuild if already showing it)
        if let activeId, activeId != activeWorkspaceId {
            switchToWorkspace(id: activeId)
        }
    }

    /// Force-rebuild a workspace's split container (kills existing terminals, respawns fresh).
    /// Used after sandbox toggle to respawn terminals with/without sandbox-exec.
    func forceRebuildWorkspace(id: String) {
        let wasActive = activeWorkspaceId == id
        if let old = containers.removeValue(forKey: id) {
            old.removeFromSuperview()
        }
        let container = SplitContainer(bridge: bridge, workspaceId: id)
        container.onGitDiff = { [weak self] in self?.onGitDiff?() }
        containers[id] = container

        if wasActive {
            activeWorkspaceId = nil
            switchToWorkspace(id: id)
        }
    }

    /// Switch the displayed workspace.
    func switchToWorkspace(id: String) {
        // Hide previous workspace (not the one we're switching TO)
        if let currentId = activeWorkspaceId, currentId != id, let current = containers[currentId] {
            current.isHidden = true
        }

        // Already showing this workspace — nothing to do
        guard id != activeWorkspaceId || containers[id]?.superview != self else { return }

        activeWorkspaceId = id

        guard let container = containers[id] else { return }

        if container.isBuilt {
            // Already in the view hierarchy — just unhide
            if container.superview == self {
                container.isHidden = false
            } else {
                showContainer(container)
            }
        } else {
            // First visit — build synchronously (no spinner deferral needed)
            showContainer(container)
            container.rebuild()
            container.isHidden = false
        }
    }

    private func showContainer(_ container: SplitContainer) {
        container.translatesAutoresizingMaskIntoConstraints = false
        addSubview(container)
        NSLayoutConstraint.activate([
            container.topAnchor.constraint(equalTo: topAnchor),
            container.leadingAnchor.constraint(equalTo: leadingAnchor),
            container.trailingAnchor.constraint(equalTo: trailingAnchor),
            container.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    /// Adjust font size across all terminal panels.
    func adjustFontSize(delta: CGFloat) {
        currentFontSize = max(8, min(32, currentFontSize + delta))
        applyFontSize()
    }

    /// Reset font size to default.
    func resetFontSize() {
        currentFontSize = CGFloat(bridge.configFontSize())
        applyFontSize()
    }

    /// Apply current configuration (font, theme, cursor) to all panels.
    func applyConfig() {
        currentFontSize = CGFloat(bridge.configFontSize())
        applyFontSize()
        for container in containers.values {
            container.applyForegroundColor()
        }
    }

    /// Handle a pane layout change (split or close) by rebuilding.
    func handlePaneLayoutChanged() {
        guard let id = activeWorkspaceId, let container = containers[id] else { return }
        container.rebuild()
    }

    /// Toggle zoom on the focused pane.
    func toggleZoomPane() {
        guard let id = activeWorkspaceId, let container = containers[id] else { return }
        container.toggleZoom()
    }

    /// Toggle find-in-terminal search bar.
    func toggleFindInTerminal() {
        guard let id = activeWorkspaceId, let container = containers[id] else { return }
        container.toggleFindInTerminal()
    }

    // MARK: - Browser access

    /// Get the focused BrowserView from the active workspace's container.
    func focusedBrowserView() -> BrowserView? {
        guard let id = activeWorkspaceId, let container = containers[id] else { return nil }
        return container.focusedBrowserView()
    }

    // MARK: - Port scanning support

    /// Return shell PIDs for all terminals in a given workspace.
    func shellPidsForWorkspace(id: String) -> [Int32] {
        containers[id]?.shellPids() ?? []
    }

    /// Scan terminal buffers in all workspaces for security-relevant content.
    func scanAllTerminalBuffers() {
        for container in containers.values {
            container.scanTerminalBuffers()
        }
    }

    // MARK: - Private

    private func applyFontSize() {
        for container in containers.values {
            container.applyFontSize(currentFontSize)
        }
    }
}
