import AppKit

/// Window controller that manages the main thane window and acts as
/// the RustBridge delegate, routing Rust callbacks to UI updates.
@MainActor
final class MainWindowController: NSWindowController, RustBridgeDelegate {

    private let bridge: RustBridge

    private var mainWindow: MainWindow {
        window as! MainWindow
    }

    // MARK: - Init

    init(bridge: RustBridge) {
        self.bridge = bridge

        let window = MainWindow(bridge: bridge)
        super.init(window: window)

        bridge.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Public API (called from AppDelegate menu actions)

    func reloadFromBridge() {
        mainWindow.sidebarView.reloadWorkspaces()
        mainWindow.workspaceView.rebuildFromBridge()
        mainWindow.statusBarView.refresh()
    }

    /// Force-rebuild a specific workspace's terminals (used after sandbox toggle).
    func forceRebuildWorkspace(id: String) {
        mainWindow.workspaceView.forceRebuildWorkspace(id: id)
        mainWindow.sidebarView.reloadWorkspaces()
    }

    /// Lightweight refresh that updates sidebar and status bar without
    /// rebuilding the split tree. Used by periodic timers to avoid stealing
    /// terminal focus.
    private var metadataRefreshInFlight = false

    func refreshMetadata() {
        // Kick off async git + cost refresh (results arrive on next cycle)
        bridge.refreshGitInfoAsync()
        bridge.refreshCostCacheAsync()
        mainWindow.sidebarView.reloadWorkspaces()
        // Supply shell PIDs from active workspace so agent detection works
        if let wsId = mainWindow.workspaceView.activeWorkspaceId {
            mainWindow.statusBarView.activeWorkspacePids = mainWindow.workspaceView.shellPidsForWorkspace(id: wsId)
        }
        // Scan terminal buffers for security events (only active workspace to reduce main-thread work)
        mainWindow.workspaceView.scanAllTerminalBuffers()
        // Scan Claude Code JSONL session files on a background thread to avoid blocking UI
        if !metadataRefreshInFlight {
            metadataRefreshInFlight = true
            bridge.scanSessionPromptsAsync { [weak self] in
                guard let self else { return }
                self.metadataRefreshInFlight = false
                self.mainWindow.statusBarView.refresh()
                self.mainWindow.refreshActivePanel()
            }
        } else {
            // Refresh status bar even if scan is still in flight
            mainWindow.statusBarView.refresh()
            mainWindow.refreshActivePanel()
        }
    }

    func applyConfig() {
        mainWindow.workspaceView.applyConfig()
    }

    func closeCurrentPanel() {
        guard let panel = bridge.focusedPanel() else { return }
        if mainWindow.confirmClosePanel() {
            _ = try? bridge.closePanel(panelId: panel.id)
        }
    }

    func toggleSidebar() {
        mainWindow.toggleSidebar()
    }

    func showRightPanel(_ type: RightPanelType) {
        mainWindow.showRightPanel(type)
    }

    func toggleGitDiff() {
        mainWindow.showRightPanel(.gitDiff)
    }

    func adjustFontSize(delta: CGFloat) {
        mainWindow.workspaceView.adjustFontSize(delta: delta)
        mainWindow.statusBarView.refresh()
    }

    func resetFontSize() {
        mainWindow.workspaceView.resetFontSize()
        mainWindow.statusBarView.refresh()
    }

    func selectNextWorkspace() {
        let workspaces = bridge.listWorkspaces()
        guard let active = bridge.activeWorkspace(),
              let idx = workspaces.firstIndex(where: { $0.id == active.id }) else { return }
        let next = (idx + 1) % workspaces.count
        _ = try? bridge.selectWorkspace(id: workspaces[next].id)
    }

    func selectPreviousWorkspace() {
        let workspaces = bridge.listWorkspaces()
        guard let active = bridge.activeWorkspace(),
              let idx = workspaces.firstIndex(where: { $0.id == active.id }) else { return }
        let prev = (idx - 1 + workspaces.count) % workspaces.count
        _ = try? bridge.selectWorkspace(id: workspaces[prev].id)
    }

    func toggleZoomPane() {
        mainWindow.workspaceView.toggleZoomPane()
    }

    func toggleFindInTerminal() {
        mainWindow.workspaceView.toggleFindInTerminal()
    }

    func scanPorts() {
        mainWindow.scanPorts()
    }

    // MARK: - RustBridgeDelegate

    func workspaceChanged(activeId: String) {
        mainWindow.sidebarView.updateActiveWorkspace(id: activeId)
        mainWindow.workspaceView.switchToWorkspace(id: activeId)
        // Defer panel refresh so the workspace swap renders immediately
        DispatchQueue.main.async { [weak self] in
            self?.mainWindow.refreshActivePanel()
        }
    }

    func workspaceListChanged() {
        mainWindow.sidebarView.reloadWorkspaces()
        // Always refresh history on list changes (close, create, historyClear all route here)
        mainWindow.sidebarView.reloadHistory()
        mainWindow.workspaceView.rebuildFromBridge()
        mainWindow.refreshActivePanel()
    }

    private var sidebarUpdatePending = false

    func sidebarNeedsUpdate() {
        // Coalesce rapid-fire updates (e.g. multiple CWD poll results arriving at once)
        // into a single sidebar reload on the next run-loop tick.
        guard !sidebarUpdatePending else { return }
        sidebarUpdatePending = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.sidebarUpdatePending = false
            self.mainWindow.sidebarView.reloadWorkspaces()
        }
    }

    func notificationReceived(workspaceId: String, title: String, body: String) {
        mainWindow.sidebarView.reloadWorkspaces() // update unread badges
        mainWindow.statusBarView.refresh()
        mainWindow.refreshActivePanel()
    }

    func agentStatusChanged(workspaceId: String, active: Bool) {
        mainWindow.statusBarView.refresh()
        mainWindow.sidebarView.reloadWorkspaces()
    }

    func queueEntryCompleted(entryId: String, success: Bool) {
        mainWindow.statusBarView.refresh()
        mainWindow.refreshActivePanel()
    }

    func paneLayoutChanged(workspaceId: String) {
        mainWindow.workspaceView.handlePaneLayoutChanged()
        mainWindow.sidebarView.reloadWorkspaces()
    }

    func configChanged() {
        mainWindow.workspaceView.applyConfig()
        mainWindow.statusBarView.refresh()
        mainWindow.refreshActivePanel()
        mainWindow.applyUIFontSize()
    }
}
