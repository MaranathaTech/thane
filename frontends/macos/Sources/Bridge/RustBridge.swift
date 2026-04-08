import Foundation
import AppKit

// MARK: - Swift convenience wrapper around UniFFI-generated ThaneBridge

/// `RustBridge` is a `@MainActor` wrapper that owns the UniFFI `ThaneBridge`
/// instance and serves as the single source of truth for all Rust-side state.
///
/// UniFFI auto-generates the following in `ThaneBridge.swift`:
///   - `ThaneBridge` class (init, workspace/pane/panel/config/queue/sandbox/session methods)
///   - `UiCallbackProtocol` protocol
///   - Data structs: WorkspaceInfo, PanelInfo, SplitResult, NotificationInfo, etc.
///   - Enums: SplitOrientation, BridgePanelType, BridgeError, etc.
///
/// Import the generated bindings alongside this file. Build the Rust bridge with:
///   cargo build --release -p thane-bridge
/// which produces libthane_bridge.a and the generated Swift bindings.

// MARK: - Callback delegation

/// Protocol for the Swift UI layer to receive push notifications from the Rust core.
@MainActor
protocol RustBridgeDelegate: AnyObject {
    func workspaceChanged(activeId: String)
    func workspaceListChanged()
    /// Lightweight callback when only sidebar content needs refreshing (e.g. CWD changed).
    /// Does NOT rebuild workspace containers — just updates sidebar row data.
    func sidebarNeedsUpdate()
    func notificationReceived(workspaceId: String, title: String, body: String)
    func agentStatusChanged(workspaceId: String, active: Bool)
    func queueEntryCompleted(entryId: String, success: Bool)
    func paneLayoutChanged(workspaceId: String)
    func configChanged()
}

// MARK: - RustBridge

@MainActor
final class RustBridge {

    weak var delegate: RustBridgeDelegate?

    // In-memory state until UniFFI bridge is wired up.
    private var workspaces: [WorkspaceInfoDTO] = []
    private var activeWorkspaceId: String?
    private(set) var splitTrees: [String: SplitNode] = [:] // workspaceId -> split tree
    private var focusedPanelId: String?

    /// Per-panel CWD tracking (panelId -> path).
    private var panelCwds: [String: String] = [:]

    /// Per-panel detected agent name (panelId -> agent name, e.g. "claude", "codex").
    /// Updated by `updatePanelAgent` when agent detection runs.
    private(set) var panelAgents: [String: String] = [:]

    /// In-memory config storage.
    private var configStore: [String: String] = [:]

    /// Persisted per-workspace cost (workspaceId -> alltimeCost in USD).
    private var workspaceCosts: [String: Double] = [:]

    /// Cached token limits (refreshed at most every 60 seconds).
    private var cachedTokenLimits: TokenLimitsDTO?
    private var tokenLimitsFetchedAt: Date?

    /// UniFFI-generated Rust bridge for sandbox and session operations.
    private let rustCoreBridge: ThaneBridge?

    /// Per-workspace sandbox policies (Swift-local, since Rust bridge workspace manager is separate).
    private var sandboxPolicies: [String: SandboxInfoDTO] = [:]

    /// Queue-level sandbox policy for headless task execution.
    private var queueSandboxPolicy: SandboxInfoDTO?

    /// In-memory agent queue entries.
    private var queueEntries: [QueueEntryInfoDTO] = []

    /// In-memory notification store.
    private var notifications: [NotificationInfoDTO] = []
    private var nextNotificationId: Int = 1

    /// Whether the user has consented to keychain access for token limits (this session only).
    private var keychainAccessConsented = false

    /// Consecutive auth failures when calling the usage API (stale credentials detection).
    private var consecutiveAuthFailures = 0

    /// Whether credentials appear stale (user changed password, token revoked, etc.).
    private(set) var credentialsStale = false

    /// UUIDs of prompts already logged as audit events (deduplication).
    private var seenPromptUuids: Set<String> = []

    /// Per-workspace listening ports.
    private(set) var workspacePorts: [String: [UInt16]] = [:]

    /// Cached git info per directory path (branch, dirty). Updated asynchronously.
    private var gitInfoCache: [String: (branch: String?, dirty: Bool)] = [:]
    /// Whether an async git refresh is already in flight.
    private var gitRefreshInFlight = false

    /// Recently closed workspaces (max 20 entries).
    private static let maxHistoryEntries = 20
    private(set) var closedWorkspaces: [ClosedWorkspaceDTO] = []

    /// Callback to extract scrollback text from a terminal panel (set by WorkspaceView).
    var scrollbackProvider: ((String) -> String?)?

    /// Scrollback text to restore into terminals, keyed by panel ID.
    /// Populated during session restore, consumed when terminals are built.
    var panelScrollback: [String: String] = [:]

    /// Timestamp when the app launched (for session vs all-time filtering).
    let appLaunchDate = Date()

    /// Cached cost data per CWD — avoids re-parsing JSONL files on every sidebar update.
    /// Refreshed by the periodic metadata timer via `refreshCostCache()`.
    private var costCache: [String: ProjectCostDTO] = [:]

    /// Last split/close action for in-place view updates.
    struct PaneAction {
        enum Kind { case split, close }
        let kind: Kind
        let panelId: String
        let orientation: SplitOrientationDTO?
    }
    private(set) var lastPaneAction: PaneAction?

    init(configPath: String? = nil) throws {
        // Initialize the Rust core bridge for sandbox enforcement.
        self.rustCoreBridge = try? ThaneBridge(configPath: configPath)
        logAuditEvent(workspaceId: "", eventType: "AppLaunched", severity: .info,
                      description: "thane launched", metadata: ["version": "0.1.0-beta.19"])
    }

    // MARK: - Workspace management

    func createWorkspace(title: String, cwd: String) throws -> WorkspaceInfoDTO {
        let id = UUID().uuidString
        let panelId = UUID().uuidString
        let ws = WorkspaceInfoDTO(
            id: id,
            title: title,
            cwd: cwd,
            tag: nil,
            paneCount: 1,
            panelCount: 1,
            unreadNotifications: 0
        )
        workspaces.append(ws)
        let panel = PanelInfoDTO(
            id: panelId, panelType: .terminal, title: "Terminal", location: cwd, hasUnread: false
        )
        splitTrees[id] = .leaf(panel)
        panelCwds[panelId] = cwd
        activeWorkspaceId = id
        focusedPanelId = panelId
        logAuditEvent(workspaceId: id, eventType: "WorkspaceCreated", severity: .info,
                      description: "Created workspace \"\(title)\"",
                      metadata: ["cwd": cwd])
        delegate?.workspaceListChanged()
        return ws
    }

    func listWorkspaces() -> [WorkspaceInfoDTO] {
        return workspaces
    }

    func selectWorkspace(id: String) throws -> Bool {
        guard workspaces.contains(where: { $0.id == id }) else { return false }
        activeWorkspaceId = id
        focusedPanelId = splitTrees[id]?.allPanels.first?.id
        delegate?.workspaceChanged(activeId: id)
        return true
    }

    func closeWorkspace(id: String) throws -> Bool {
        // Capture workspace info for history before removing
        if let ws = workspaces.first(where: { $0.id == id }) {
            let formatter = ISO8601DateFormatter()
            let closed = ClosedWorkspaceDTO(
                id: ws.id,
                title: ws.title,
                cwd: ws.cwd,
                tag: ws.tag,
                closedAt: formatter.string(from: Date())
            )
            closedWorkspaces.insert(closed, at: 0)
            if closedWorkspaces.count > Self.maxHistoryEntries {
                closedWorkspaces = Array(closedWorkspaces.prefix(Self.maxHistoryEntries))
            }
        }

        logAuditEvent(workspaceId: id, eventType: "WorkspaceClosed", severity: .warning,
                      description: "Closed workspace \"\(workspaces.first(where: { $0.id == id })?.title ?? id)\"")
        workspaces.removeAll { $0.id == id }
        splitTrees.removeValue(forKey: id)
        if activeWorkspaceId == id {
            activeWorkspaceId = workspaces.first?.id
        }
        delegate?.workspaceListChanged()
        return true
    }

    // MARK: - Workspace history

    func historyList() -> [ClosedWorkspaceDTO] {
        return closedWorkspaces
    }

    func historyReopen(id: String) throws -> WorkspaceInfoDTO {
        guard let idx = closedWorkspaces.firstIndex(where: { $0.id == id }) else {
            throw NSError(domain: "thane", code: -1, userInfo: [
                NSLocalizedDescriptionKey: "No closed workspace found with id \(id)"
            ])
        }
        let closed = closedWorkspaces.remove(at: idx)
        return try createWorkspace(title: closed.title, cwd: closed.cwd)
    }

    func historyClear() {
        closedWorkspaces.removeAll()
        delegate?.workspaceListChanged()
    }

    func renameWorkspace(id: String, title: String) throws -> Bool {
        guard let idx = workspaces.firstIndex(where: { $0.id == id }) else { return false }
        let old = workspaces[idx]
        workspaces[idx] = old.updating(title: title)
        delegate?.sidebarNeedsUpdate()
        return true
    }

    /// Update a panel's CWD (called when the terminal detects a directory change).
    func updatePanelCwd(workspaceId: String, panelId: String, cwd: String) {
        // Normalize: strip file:// prefix and trailing slash
        var dir = cwd
        if dir.hasPrefix("file://") {
            dir = String(dir.dropFirst(7))
        }
        if dir.hasSuffix("/") && dir.count > 1 {
            dir = String(dir.dropLast())
        }
        if let decoded = dir.removingPercentEncoding {
            dir = decoded
        }
        guard dir != panelCwds[panelId] else { return }
        panelCwds[panelId] = dir

        // Also update workspace-level CWD to match the focused panel
        if panelId == focusedPanelId,
           let idx = workspaces.firstIndex(where: { $0.id == workspaceId }) {
            let old = workspaces[idx]
            workspaces[idx] = old.updating(cwd: dir)
        }
        delegate?.sidebarNeedsUpdate()
    }

    /// Get location info (CWD + git) for all panels in a workspace.
    /// Returns cached git info — never blocks the main thread.
    func panelLocations(for workspaceId: String) -> [PanelLocationInfo] {
        guard let tree = splitTrees[workspaceId] else { return [] }
        return tree.allPanels.map { panel in
            let cwd = panelCwds[panel.id] ?? panel.location
            let cached = gitInfoCache[cwd]
            return PanelLocationInfo(cwd: cwd, gitBranch: cached?.branch, gitDirty: cached?.dirty ?? false)
        }
    }

    /// Refresh git info for all panels across all workspaces on a background queue.
    /// Results are applied to the cache on the main thread, then the sidebar is reloaded.
    func refreshGitInfoAsync() {
        guard !gitRefreshInFlight else { return }
        gitRefreshInFlight = true

        // Collect unique CWDs to query
        var cwds = Set<String>()
        for (_, tree) in splitTrees {
            for panel in tree.allPanels {
                let cwd = panelCwds[panel.id] ?? panel.location
                cwds.insert(cwd)
            }
        }
        let cwdList = Array(cwds)

        DispatchQueue.global(qos: .utility).async { [weak self] in
            var results: [String: (branch: String?, dirty: Bool)] = [:]
            for cwd in cwdList {
                results[cwd] = Self.gitInfo(for: cwd)
            }
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.gitInfoCache = results
                self.gitRefreshInFlight = false
            }
        }
    }

    /// Synchronously refresh git info for a single workspace. Intended for testing.
    func updateGitInfoSync(for workspaceId: String) {
        guard let tree = splitTrees[workspaceId] else { return }
        for panel in tree.allPanels {
            let cwd = panelCwds[panel.id] ?? panel.location
            gitInfoCache[cwd] = Self.gitInfo(for: cwd)
        }
    }

    /// Detect git branch and dirty state for a directory.
    nonisolated private static func gitInfo(for path: String) -> (branch: String?, dirty: Bool) {
        let branchResult = Self.runGit(args: ["rev-parse", "--abbrev-ref", "HEAD"], cwd: path)
        guard let branch = branchResult else { return (nil, false) }
        let statusResult = Self.runGit(args: ["status", "--porcelain"], cwd: path)
        let dirty = statusResult.map { !$0.isEmpty } ?? false
        return (branch, dirty)
    }

    nonisolated private static func runGit(args: [String], cwd: String) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = args
        process.currentDirectoryURL = URL(fileURLWithPath: cwd)
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()
        do {
            try process.run()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else { return nil }
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            return String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
        } catch {
            return nil
        }
    }

    func activeWorkspace() -> WorkspaceInfoDTO? {
        guard let id = activeWorkspaceId else { return workspaces.first }
        return workspaces.first { $0.id == id }
    }

    // MARK: - Pane / split operations

    func splitTerminal(orientation: SplitOrientationDTO) throws -> SplitResultDTO {
        guard let wsId = activeWorkspaceId,
              let focusId = focusedPanelId,
              let tree = splitTrees[wsId] else {
            return SplitResultDTO(paneId: UUID().uuidString, panelId: UUID().uuidString)
        }
        let paneId = UUID().uuidString
        let panelId = UUID().uuidString
        let ws = workspaces.first { $0.id == wsId }
        let cwd = ws?.cwd ?? FileManager.default.homeDirectoryForCurrentUser.path
        // Use the focused panel's CWD for the new terminal
        let panelCwd = panelCwds[focusId] ?? ws?.cwd ?? FileManager.default.homeDirectoryForCurrentUser.path
        let newPanel = PanelInfoDTO(
            id: panelId, panelType: .terminal, title: "Terminal", location: panelCwd, hasUnread: false
        )
        panelCwds[panelId] = panelCwd
        // Find the focused leaf and replace it with a split containing the old leaf + new leaf
        let focusedLeaf = tree.allPanels.first { $0.id == focusId }
        if let focused = focusedLeaf {
            let newSplit = SplitNode.split(orientation, .leaf(focused), .leaf(newPanel))
            splitTrees[wsId] = tree.replacing(panelId: focusId, with: newSplit)
        }
        focusedPanelId = panelId
        lastPaneAction = PaneAction(kind: .split, panelId: panelId, orientation: orientation)
        let dir = orientation == .horizontal ? "right" : "down"
        logAuditEvent(workspaceId: wsId, panelId: panelId, eventType: "PaneSplit", severity: .info,
                      description: "Split pane \(dir)", metadata: ["cwd": panelCwd])
        delegate?.paneLayoutChanged(workspaceId: wsId)
        return SplitResultDTO(paneId: paneId, panelId: panelId)
    }

    func splitBrowser(url: String, orientation: SplitOrientationDTO) throws -> SplitResultDTO {
        guard let wsId = activeWorkspaceId,
              let focusId = focusedPanelId,
              let tree = splitTrees[wsId] else {
            return SplitResultDTO(paneId: UUID().uuidString, panelId: UUID().uuidString)
        }
        let paneId = UUID().uuidString
        let panelId = UUID().uuidString
        let newPanel = PanelInfoDTO(
            id: panelId, panelType: .browser, title: "Browser", location: url, hasUnread: false
        )
        let focusedLeaf = tree.allPanels.first { $0.id == focusId }
        if let focused = focusedLeaf {
            let newSplit = SplitNode.split(orientation, .leaf(focused), .leaf(newPanel))
            splitTrees[wsId] = tree.replacing(panelId: focusId, with: newSplit)
        }
        focusedPanelId = panelId
        delegate?.paneLayoutChanged(workspaceId: wsId)
        return SplitResultDTO(paneId: paneId, panelId: panelId)
    }

    func closePane() throws {
        guard let wsId = activeWorkspaceId,
              let focusId = focusedPanelId,
              let tree = splitTrees[wsId] else { return }
        lastPaneAction = PaneAction(kind: .close, panelId: focusId, orientation: nil)
        if let newTree = tree.removing(panelId: focusId) {
            splitTrees[wsId] = newTree
            focusedPanelId = newTree.allPanels.first?.id
        }
        delegate?.paneLayoutChanged(workspaceId: wsId)
    }

    func focusNextPane() { focusPanelAtOffset(1) }
    func focusPrevPane() { focusPanelAtOffset(-1) }

    private func focusPanelAtOffset(_ offset: Int) {
        guard let wsId = activeWorkspaceId,
              let tree = splitTrees[wsId] else { return }
        let panels = tree.allPanels
        guard !panels.isEmpty else { return }
        let currentIdx = panels.firstIndex(where: { $0.id == focusedPanelId }) ?? 0
        let nextIdx = (currentIdx + offset + panels.count) % panels.count
        focusedPanelId = panels[nextIdx].id
        delegate?.paneLayoutChanged(workspaceId: wsId)
    }

    func focusDirection(_ direction: String) throws {
        switch direction {
        case "left", "up": focusPrevPane()
        case "right", "down": focusNextPane()
        default: break
        }
    }

    // MARK: - Panel management

    func addBrowserPanel(url: String) throws -> String {
        let result = try splitBrowser(url: url, orientation: .horizontal)
        return result.panelId
    }

    func closePanel(panelId: String) throws -> Bool {
        guard let wsId = activeWorkspaceId,
              let tree = splitTrees[wsId] else { return false }
        if let newTree = tree.removing(panelId: panelId) {
            splitTrees[wsId] = newTree
            if focusedPanelId == panelId {
                focusedPanelId = newTree.allPanels.first?.id
            }
        } else {
            return false
        }
        delegate?.paneLayoutChanged(workspaceId: wsId)
        return true
    }

    func selectPanel(panelId: String) -> Bool {
        focusedPanelId = panelId
        return true
    }

    /// Reorder a panel within the focused pane (move tab to new index).
    func reorderPanel(panelId: String, newIndex: Int) -> Bool {
        guard let wsId = activeWorkspaceId,
              let tree = splitTrees[wsId] else { return false }
        let panels = tree.allPanels
        guard let currentIdx = panels.firstIndex(where: { $0.id == panelId }),
              newIndex >= 0, newIndex < panels.count, newIndex != currentIdx else { return false }
        // Swap the two panels' positions in the tree
        let targetPanel = panels[newIndex]
        let movingPanel = panels[currentIdx]
        let swapped = tree
            .replacing(panelId: movingPanel.id, with: .leaf(targetPanel))
            .replacing(panelId: targetPanel.id, with: .leaf(movingPanel))
        splitTrees[wsId] = swapped
        delegate?.paneLayoutChanged(workspaceId: wsId)
        return true
    }

    func listPanels() -> [PanelInfoDTO] {
        guard let wsId = activeWorkspaceId, let tree = splitTrees[wsId] else { return [] }
        return tree.allPanels
    }

    func focusedPanel() -> PanelInfoDTO? {
        let all = listPanels()
        guard let id = focusedPanelId else { return all.first }
        return all.first { $0.id == id }
    }

    /// Get the CWD of the currently focused panel.
    func focusedPanelCwd() -> String? {
        guard let id = focusedPanelId else { return nil }
        return panelCwds[id]
    }

    /// Update a panel's current location (CWD for terminals, URL for browsers).
    func updatePanelCwd(panelId: String, cwd: String) {
        panelCwds[panelId] = cwd
    }

    /// Get the split tree for a workspace.
    func splitTree(for workspaceId: String) -> SplitNode? {
        return splitTrees[workspaceId]
    }

    // MARK: - Notifications

    func listNotifications(workspaceId: String? = nil) -> [NotificationInfoDTO] {
        if let wsId = workspaceId {
            return notifications.filter { $0.panelId == wsId }
        }
        return notifications
    }

    func markNotificationRead(id: String) {
        if let idx = notifications.firstIndex(where: { $0.id == id }) {
            let n = notifications[idx]
            notifications[idx] = NotificationInfoDTO(
                id: n.id, panelId: n.panelId, title: n.title, body: n.body,
                urgency: n.urgency, timestamp: n.timestamp, read: true
            )
        }
    }

    func markAllNotificationsRead() {
        notifications = notifications.map {
            NotificationInfoDTO(id: $0.id, panelId: $0.panelId, title: $0.title,
                              body: $0.body, urgency: $0.urgency, timestamp: $0.timestamp, read: true)
        }
    }

    func unreadNotificationCount() -> UInt64 {
        UInt64(notifications.filter { !$0.read }.count)
    }

    func postNotification(workspaceId: String, title: String, body: String,
                          urgency: NotifyUrgencyDTO = .normal) {
        let id = "\(nextNotificationId)"
        nextNotificationId += 1
        let formatter = ISO8601DateFormatter()
        let timestamp = formatter.string(from: Date())
        let notif = NotificationInfoDTO(
            id: id, panelId: workspaceId, title: title, body: body,
            urgency: urgency, timestamp: timestamp, read: false
        )
        notifications.insert(notif, at: 0)
        delegate?.notificationReceived(workspaceId: workspaceId, title: title, body: body)
    }

    // MARK: - Configuration

    func configGet(key: String) -> String? {
        return configStore[key]
    }

    func configSet(key: String, value: String) {
        configStore[key] = value
        delegate?.configChanged()
    }

    func configFontFamily() -> String {
        return configStore["font-family"] ?? ThaneTheme.fontFamily
    }

    func configFontSize() -> Double {
        if let str = configStore["font-size"], let val = Double(str) { return val }
        return Double(ThaneTheme.defaultFontSize)
    }

    // MARK: - Agent Detection

    /// Update the detected agent name for a panel. Called by SplitContainer
    /// when it knows the shell PID for a terminal panel.
    func updatePanelAgent(panelId: String, shellPid: Int32) {
        let result = detectAgentForPids([shellPid])
        if result.hasPrefix("active:"), let name = result.split(separator: ":").last {
            panelAgents[panelId] = String(name)
        } else {
            panelAgents.removeValue(forKey: panelId)
        }
    }

    /// Get the agent name for a panel, if one is currently detected.
    func agentForPanel(_ panelId: String) -> String? {
        panelAgents[panelId]
    }

    /// Known agent binary names — order matters: longer prefixes first.
    /// Short names ("amp", "cody") use exact match to avoid false positives.
    private static let agentNames: [(name: String, exact: Bool)] = [
        ("claude-code", false), ("claude", false), ("codex", false), ("gemini", false),
        ("goose", false), ("opencode", false), ("cline", false), ("amp", true),
        ("auggie", false), ("openhands", false), ("plandex", false), ("qwen", false),
        ("devin", false), ("tabnine", false), ("cursor", false), ("aider", false),
        ("copilot", false), ("cody", true), ("continue", false),
    ]

    /// Check if any of the given shell PIDs have an agent process running as a child.
    /// Returns "active:<agent_name>" if found, or "idle" if no agent detected.
    func detectAgentForPids(_ pids: [Int32]) -> String {
        for pid in pids {
            if let name = findAgentChildOf(pid) {
                return "active:\(name)"
            }
        }
        return "idle"
    }

    /// Check if a PID has a child process that looks like an AI agent.
    /// Returns the agent name if found, nil otherwise.
    private func findAgentChildOf(_ parentPid: Int32) -> String? {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        task.arguments = ["-P", "\(parentPid)"]
        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = FileHandle.nullDevice
        do { try task.run() } catch { return nil }
        task.waitUntilExit()

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let output = String(data: data, encoding: .utf8) else { return nil }

        let childPids = output.split(separator: "\n").compactMap { Int32($0.trimmingCharacters(in: .whitespaces)) }

        for childPid in childPids {
            let psTask = Process()
            psTask.executableURL = URL(fileURLWithPath: "/bin/ps")
            psTask.arguments = ["-p", "\(childPid)", "-o", "comm="]
            let psPipe = Pipe()
            psTask.standardOutput = psPipe
            psTask.standardError = FileHandle.nullDevice
            do { try psTask.run() } catch { continue }
            psTask.waitUntilExit()

            let psData = psPipe.fileHandleForReading.readDataToEndOfFile()
            if let name = String(data: psData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
                for agent in Self.agentNames {
                    if agent.exact {
                        if name == agent.name { return agent.name }
                    } else {
                        if name.contains(agent.name) { return agent.name }
                    }
                }
            }

            // Also check grandchildren (agent may be spawned by node/npx)
            if let grandchild = findAgentChildOf(childPid) { return grandchild }
        }
        return nil
    }

    // MARK: - Keychain consent

    /// Show a custom modal explaining why keychain access is needed before the system
    /// keychain prompt appears. Returns true if user consents.
    /// Skips the modal if credentials are already accessible (macOS already authorized the app).
    @discardableResult
    func requestKeychainConsent() -> Bool {
        if keychainAccessConsented { return true }
        // Check if credentials are already accessible without showing our modal.
        // Try file first (no keychain prompt), then the `security` CLI (no ACL issues).
        if readClaudeOAuthCredentialsFromFile() != nil
            || readClaudeOAuthCredentialsFromSecurityCLI() != nil {
            keychainAccessConsented = true
            return true
        }

        // Build a custom explanation panel instead of a plain NSAlert
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 420, height: 340),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        panel.title = "Keychain Access"
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = false
        panel.titlebarAppearsTransparent = true
        panel.backgroundColor = ThaneTheme.sidebarBackground

        let content = NSView(frame: panel.contentView!.bounds)
        content.autoresizingMask = [.width, .height]

        // Icon
        let iconView = NSImageView()
        if let lockImage = NSImage(systemSymbolName: "key.fill", accessibilityDescription: "Keychain") {
            let config = NSImage.SymbolConfiguration(pointSize: 36, weight: .medium)
            iconView.image = lockImage.withSymbolConfiguration(config)
        }
        iconView.contentTintColor = ThaneTheme.accentColor
        iconView.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(iconView)

        // Title
        let titleLabel = NSTextField(labelWithString: "Keychain Access Required")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 18)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.alignment = .center
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(titleLabel)

        // Explanation
        let explanationText = """
        thane needs one-time access to your macOS Keychain to read your Claude Code credentials.

        This is used to:
        """
        let explanationLabel = NSTextField(wrappingLabelWithString: explanationText)
        explanationLabel.font = ThaneTheme.uiFont(size: 13)
        explanationLabel.textColor = ThaneTheme.secondaryText
        explanationLabel.alignment = .center
        explanationLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(explanationLabel)

        // Bullet points
        let bullets = [
            ("chart.bar", "Display your token usage and plan limits"),
            ("gauge.with.dots.needle.33percent", "Show rate limit status and reset times"),
            ("lock.shield", "All data stays local — nothing is sent externally"),
        ]
        let bulletStack = NSStackView()
        bulletStack.orientation = .vertical
        bulletStack.alignment = .leading
        bulletStack.spacing = 8
        bulletStack.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(bulletStack)

        for (iconName, text) in bullets {
            let row = NSStackView()
            row.orientation = .horizontal
            row.spacing = 8
            row.alignment = .centerY

            let bulletIcon = NSImageView()
            if let img = NSImage(systemSymbolName: iconName, accessibilityDescription: nil) {
                let config = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
                bulletIcon.image = img.withSymbolConfiguration(config)
            }
            bulletIcon.contentTintColor = ThaneTheme.accentColor
            bulletIcon.translatesAutoresizingMaskIntoConstraints = false
            bulletIcon.widthAnchor.constraint(equalToConstant: 20).isActive = true
            bulletIcon.heightAnchor.constraint(equalToConstant: 20).isActive = true

            let label = NSTextField(labelWithString: text)
            label.font = ThaneTheme.uiFont(size: 12)
            label.textColor = ThaneTheme.primaryText

            row.addArrangedSubview(bulletIcon)
            row.addArrangedSubview(label)
            bulletStack.addArrangedSubview(row)
        }

        // Hint
        let hintLabel = NSTextField(wrappingLabelWithString: "You may see a macOS Keychain prompt after clicking Allow.")
        hintLabel.font = ThaneTheme.uiFont(size: 11)
        hintLabel.textColor = ThaneTheme.tertiaryText
        hintLabel.alignment = .center
        hintLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(hintLabel)

        // Buttons
        var userConsented = false

        let allowBtn = NSButton(title: "Allow Keychain Access", target: nil, action: nil)
        allowBtn.bezelStyle = .rounded
        allowBtn.controlSize = .large
        allowBtn.font = ThaneTheme.boldLabelFont(size: 13)
        allowBtn.keyEquivalent = "\r"
        allowBtn.translatesAutoresizingMaskIntoConstraints = false
        allowBtn.contentTintColor = .white
        allowBtn.wantsLayer = true
        allowBtn.layer?.backgroundColor = ThaneTheme.accentColor.cgColor
        allowBtn.layer?.cornerRadius = 6
        content.addSubview(allowBtn)

        let skipBtn = NSButton(title: "Not Now", target: nil, action: nil)
        skipBtn.bezelStyle = .rounded
        skipBtn.controlSize = .regular
        skipBtn.font = ThaneTheme.uiFont(size: 12)
        skipBtn.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(skipBtn)

        // Use target-action with closures via a helper
        class ButtonHandler: NSObject {
            var action: (() -> Void)?
            @objc func clicked() { action?() }
        }
        let allowHandler = ButtonHandler()
        allowHandler.action = { userConsented = true; NSApp.stopModal() }
        allowBtn.target = allowHandler
        allowBtn.action = #selector(ButtonHandler.clicked)

        let skipHandler = ButtonHandler()
        skipHandler.action = { userConsented = false; NSApp.stopModal() }
        skipBtn.target = skipHandler
        skipBtn.action = #selector(ButtonHandler.clicked)

        // Stop modal when the panel's close button (X) is clicked, otherwise
        // runModal keeps running invisibly and the app appears frozen.
        class CloseDelegate: NSObject, NSWindowDelegate {
            func windowWillClose(_ notification: Notification) { NSApp.stopModal() }
        }
        let closeDelegate = CloseDelegate()
        panel.delegate = closeDelegate

        // Keep handlers alive
        objc_setAssociatedObject(panel, "allowHandler", allowHandler, .OBJC_ASSOCIATION_RETAIN)
        objc_setAssociatedObject(panel, "skipHandler", skipHandler, .OBJC_ASSOCIATION_RETAIN)
        objc_setAssociatedObject(panel, "closeDelegate", closeDelegate, .OBJC_ASSOCIATION_RETAIN)

        // Layout
        NSLayoutConstraint.activate([
            iconView.topAnchor.constraint(equalTo: content.topAnchor, constant: 24),
            iconView.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            iconView.widthAnchor.constraint(equalToConstant: 48),
            iconView.heightAnchor.constraint(equalToConstant: 48),

            titleLabel.topAnchor.constraint(equalTo: iconView.bottomAnchor, constant: 12),
            titleLabel.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
            titleLabel.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),

            explanationLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 10),
            explanationLabel.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 30),
            explanationLabel.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -30),

            bulletStack.topAnchor.constraint(equalTo: explanationLabel.bottomAnchor, constant: 10),
            bulletStack.centerXAnchor.constraint(equalTo: content.centerXAnchor),

            hintLabel.topAnchor.constraint(equalTo: bulletStack.bottomAnchor, constant: 16),
            hintLabel.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 30),
            hintLabel.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -30),

            allowBtn.topAnchor.constraint(equalTo: hintLabel.bottomAnchor, constant: 16),
            allowBtn.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            allowBtn.widthAnchor.constraint(equalToConstant: 200),
            allowBtn.heightAnchor.constraint(equalToConstant: 34),

            skipBtn.topAnchor.constraint(equalTo: allowBtn.bottomAnchor, constant: 8),
            skipBtn.centerXAnchor.constraint(equalTo: content.centerXAnchor),
        ])

        panel.contentView = content
        panel.center()
        panel.level = .modalPanel
        panel.makeKeyAndOrderFront(nil)
        NSApp.runModal(for: panel)
        panel.orderOut(nil)

        keychainAccessConsented = userConsented
        return keychainAccessConsented
    }

    /// Show a modal explaining that credentials appear stale and guide the user to re-authenticate.
    /// Called when the usage API returns consecutive auth errors.
    func promptCredentialsRefresh() {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 420, height: 300),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        panel.title = "Credentials Update Needed"
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = false
        panel.titlebarAppearsTransparent = true
        panel.backgroundColor = ThaneTheme.sidebarBackground

        let content = NSView(frame: panel.contentView!.bounds)
        content.autoresizingMask = [.width, .height]

        // Icon
        let iconView = NSImageView()
        if let img = NSImage(systemSymbolName: "exclamationmark.arrow.circlepath",
                             accessibilityDescription: "Credentials expired") {
            let config = NSImage.SymbolConfiguration(pointSize: 36, weight: .medium)
            iconView.image = img.withSymbolConfiguration(config)
        }
        iconView.contentTintColor = ThaneTheme.warningColor
        iconView.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(iconView)

        // Title
        let titleLabel = NSTextField(labelWithString: "Credentials Need Refreshing")
        titleLabel.font = ThaneTheme.boldLabelFont(size: 18)
        titleLabel.textColor = ThaneTheme.primaryText
        titleLabel.alignment = .center
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(titleLabel)

        // Explanation
        let explanationLabel = NSTextField(wrappingLabelWithString: """
        thane can no longer access your Claude Code credentials. This usually happens when:
        """)
        explanationLabel.font = ThaneTheme.uiFont(size: 13)
        explanationLabel.textColor = ThaneTheme.secondaryText
        explanationLabel.alignment = .center
        explanationLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(explanationLabel)

        // Reasons
        let reasons = [
            ("person.badge.key", "You changed your Anthropic password"),
            ("arrow.triangle.2.circlepath", "Your OAuth token expired or was revoked"),
            ("lock.rotation", "Claude Code was re-authenticated with a new account"),
        ]
        let reasonStack = NSStackView()
        reasonStack.orientation = .vertical
        reasonStack.alignment = .leading
        reasonStack.spacing = 6
        reasonStack.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(reasonStack)

        for (iconName, text) in reasons {
            let row = NSStackView()
            row.orientation = .horizontal
            row.spacing = 8
            row.alignment = .centerY

            let bulletIcon = NSImageView()
            if let img = NSImage(systemSymbolName: iconName, accessibilityDescription: nil) {
                let config = NSImage.SymbolConfiguration(pointSize: 13, weight: .medium)
                bulletIcon.image = img.withSymbolConfiguration(config)
            }
            bulletIcon.contentTintColor = ThaneTheme.warningColor
            bulletIcon.translatesAutoresizingMaskIntoConstraints = false
            bulletIcon.widthAnchor.constraint(equalToConstant: 18).isActive = true
            bulletIcon.heightAnchor.constraint(equalToConstant: 18).isActive = true

            let label = NSTextField(labelWithString: text)
            label.font = ThaneTheme.uiFont(size: 12)
            label.textColor = ThaneTheme.primaryText

            row.addArrangedSubview(bulletIcon)
            row.addArrangedSubview(label)
            reasonStack.addArrangedSubview(row)
        }

        // Fix instructions
        let fixLabel = NSTextField(wrappingLabelWithString: "To fix this, run `claude` in a terminal to re-authenticate, then restart thane.")
        fixLabel.font = ThaneTheme.boldLabelFont(size: 12)
        fixLabel.textColor = ThaneTheme.primaryText
        fixLabel.alignment = .center
        fixLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(fixLabel)

        // Dismiss button
        class ButtonHandler: NSObject {
            var action: (() -> Void)?
            @objc func clicked() { action?() }
        }
        let handler = ButtonHandler()
        handler.action = { NSApp.stopModal() }

        let dismissBtn = NSButton(title: "OK", target: handler, action: #selector(ButtonHandler.clicked))
        dismissBtn.bezelStyle = .rounded
        dismissBtn.controlSize = .large
        dismissBtn.font = ThaneTheme.boldLabelFont(size: 13)
        dismissBtn.keyEquivalent = "\r"
        dismissBtn.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(dismissBtn)

        // Stop modal when the panel's close button (X) is clicked.
        class CloseDelegate: NSObject, NSWindowDelegate {
            func windowWillClose(_ notification: Notification) { NSApp.stopModal() }
        }
        let closeDelegate = CloseDelegate()
        panel.delegate = closeDelegate

        objc_setAssociatedObject(panel, "handler", handler, .OBJC_ASSOCIATION_RETAIN)
        objc_setAssociatedObject(panel, "closeDelegate", closeDelegate, .OBJC_ASSOCIATION_RETAIN)

        NSLayoutConstraint.activate([
            iconView.topAnchor.constraint(equalTo: content.topAnchor, constant: 24),
            iconView.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            iconView.widthAnchor.constraint(equalToConstant: 48),
            iconView.heightAnchor.constraint(equalToConstant: 48),

            titleLabel.topAnchor.constraint(equalTo: iconView.bottomAnchor, constant: 12),
            titleLabel.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
            titleLabel.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),

            explanationLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 10),
            explanationLabel.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 30),
            explanationLabel.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -30),

            reasonStack.topAnchor.constraint(equalTo: explanationLabel.bottomAnchor, constant: 8),
            reasonStack.centerXAnchor.constraint(equalTo: content.centerXAnchor),

            fixLabel.topAnchor.constraint(equalTo: reasonStack.bottomAnchor, constant: 16),
            fixLabel.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 30),
            fixLabel.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -30),

            dismissBtn.topAnchor.constraint(equalTo: fixLabel.bottomAnchor, constant: 16),
            dismissBtn.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            dismissBtn.widthAnchor.constraint(equalToConstant: 100),
        ])

        panel.contentView = content
        panel.center()
        panel.level = .modalPanel
        panel.makeKeyAndOrderFront(nil)
        NSApp.runModal(for: panel)
        panel.orderOut(nil)

        // Reset state so the next token panel open will re-try
        keychainAccessConsented = false
        cachedTokenLimits = nil
        tokenLimitsFetchedAt = nil
        consecutiveAuthFailures = 0
    }

    // MARK: - Cost & Token Limits

    /// Determine display mode: "utilization" for plans with data and cost info, "dollar" otherwise.
    private static func resolveDisplayMode(planName: String, hasCaps: Bool, fiveHourUtil: Double?, sevenDayUtil: Double?) -> String {
        let hasUsage = fiveHourUtil != nil || sevenDayUtil != nil
        if !hasUsage { return "dollar" }
        if hasCaps { return "utilization" }
        // Enterprise always shows utilization (has default cost estimate).
        if planName.lowercased() == "enterprise" { return "utilization" }
        return "dollar"
    }

    func getTokenLimits() -> TokenLimitsDTO {
        guard keychainAccessConsented else {
            NSLog("thane: getTokenLimits — keychain not consented")
            return TokenLimitsDTO(planName: "Unknown", hasCaps: true, displayMode: "dollar",
                                  fiveHourUtilization: nil, fiveHourResetsAt: nil,
                                  sevenDayUtilization: nil, sevenDayResetsAt: nil)
        }

        // Return cached result if fetched within 60 seconds
        if let cached = cachedTokenLimits,
           let fetchedAt = tokenLimitsFetchedAt,
           Date().timeIntervalSince(fetchedAt) < 60 {
            return cached
        }

        // Read credentials from macOS Keychain (Claude Code stores them there)
        // and from ~/.claude/.credentials.json as fallback.
        guard let oauth = readClaudeOAuthCredentials(),
              let accessToken = oauth["accessToken"] as? String else {
            NSLog("thane: getTokenLimits — no credentials found (keychain, security CLI, and file all failed)")
            return TokenLimitsDTO(planName: "Unknown", hasCaps: true, displayMode: "dollar",
                                  fiveHourUtilization: nil, fiveHourResetsAt: nil,
                                  sevenDayUtilization: nil, sevenDayResetsAt: nil)
        }
        NSLog("thane: getTokenLimits — credentials found, subscription=\(oauth["subscriptionType"] ?? "nil")")

        // Detect plan
        let subscriptionType = oauth["subscriptionType"] as? String ?? ""
        let rateLimitTier = oauth["rateLimitTier"] as? String ?? ""
        let planName: String
        let hasCaps: Bool
        switch subscriptionType.lowercased() {
        case "max":
            planName = rateLimitTier.contains("20x") ? "Max (20x)" : "Max (5x)"
            hasCaps = true
        case "pro":
            planName = "Pro"
            hasCaps = true
        case "team":
            planName = "Team"
            hasCaps = true
        case "enterprise":
            planName = "Enterprise"
            hasCaps = false
        case "api":
            planName = "API"
            hasCaps = false
        default:
            planName = subscriptionType.isEmpty ? "Pro" : subscriptionType.capitalized
            hasCaps = true
        }

        // Fetch usage from API — pass auth header via temp config file to avoid
        // leaking the bearer token in process arguments (visible via `ps`)
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/curl")
        let configContent = "header = \"Authorization: Bearer \(accessToken)\"\n"
        let configPath = NSTemporaryDirectory() + "thane-curl-\(UUID().uuidString).conf"
        try? configContent.write(toFile: configPath, atomically: true, encoding: .utf8)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: configPath)
        task.arguments = ["-s",
                          "-K", configPath,
                          "-H", "anthropic-beta: oauth-2025-04-20",
                          "https://api.anthropic.com/api/oauth/usage"]
        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = FileHandle.nullDevice
        do { try task.run() } catch {
            try? FileManager.default.removeItem(atPath: configPath)
            let fallback = TokenLimitsDTO(planName: planName, hasCaps: hasCaps, displayMode: "dollar",
                                  fiveHourUtilization: nil, fiveHourResetsAt: nil,
                                  sevenDayUtilization: nil, sevenDayResetsAt: nil)
            cachedTokenLimits = fallback
            tokenLimitsFetchedAt = Date()
            return fallback
        }
        task.waitUntilExit()
        // Clean up temp config file containing the auth token
        try? FileManager.default.removeItem(atPath: configPath)

        let responseData = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let usage = try? JSONSerialization.jsonObject(with: responseData) as? [String: Any] else {
            let fallback = TokenLimitsDTO(planName: planName, hasCaps: hasCaps, displayMode: "dollar",
                                  fiveHourUtilization: nil, fiveHourResetsAt: nil,
                                  sevenDayUtilization: nil, sevenDayResetsAt: nil)
            cachedTokenLimits = fallback
            tokenLimitsFetchedAt = Date()
            return fallback
        }

        // Check for API error (rate limit, auth error, etc.)
        NSLog("thane: usage API response keys=\(Array(usage.keys))")
        if usage["error"] != nil {
            let errorType = (usage["error"] as? [String: Any])?["type"] as? String
                ?? usage["error"] as? String ?? ""
            let isAuthError = errorType.contains("authentication") || errorType.contains("invalid")
                || errorType.contains("unauthorized") || errorType.contains("forbidden")
                || task.terminationStatus != 0

            if isAuthError {
                consecutiveAuthFailures += 1
                NSLog("thane: usage API auth failure #\(consecutiveAuthFailures): \(errorType)")
                if consecutiveAuthFailures >= 2 {
                    credentialsStale = true
                }
            }

            // Return cached data if available, otherwise plan-only
            let fallback = cachedTokenLimits ?? TokenLimitsDTO(
                planName: planName, hasCaps: hasCaps, displayMode: "dollar",
                fiveHourUtilization: nil, fiveHourResetsAt: nil,
                sevenDayUtilization: nil, sevenDayResetsAt: nil)
            tokenLimitsFetchedAt = Date()
            return fallback
        }

        // Successful API call — reset failure tracking
        consecutiveAuthFailures = 0
        credentialsStale = false

        let fiveHour = usage["five_hour"] as? [String: Any]
        let sevenDay = usage["seven_day"] as? [String: Any]
        let fiveHourUtil = fiveHour?["utilization"] as? Double
        let sevenDayUtil = sevenDay?["utilization"] as? Double

        let result = TokenLimitsDTO(
            planName: planName,
            hasCaps: hasCaps,
            displayMode: Self.resolveDisplayMode(planName: planName, hasCaps: hasCaps, fiveHourUtil: fiveHourUtil, sevenDayUtil: sevenDayUtil),
            fiveHourUtilization: fiveHourUtil,
            fiveHourResetsAt: fiveHour?["resets_at"] as? String,
            sevenDayUtilization: sevenDayUtil,
            sevenDayResetsAt: sevenDay?["resets_at"] as? String
        )
        cachedTokenLimits = result
        tokenLimitsFetchedAt = Date()
        return result
    }

    func getProjectCost() -> ProjectCostDTO {
        guard let ws = activeWorkspace() else { return ProjectCostDTO.zero }
        return getProjectCostForCwd(ws.cwd)
    }

    func getProjectCostForCwd(_ cwd: String) -> ProjectCostDTO {
        costCache[cwd] ?? .zero
    }

    /// Refresh cost cache for all workspaces on a background queue.
    /// Called from the periodic metadata timer — never blocks the main thread.
    func refreshCostCacheAsync() {
        let cwds = workspaces.map(\.cwd)
        let launchDate = appLaunchDate
        DispatchQueue.global(qos: .utility).async { [weak self] in
            var results: [String: ProjectCostDTO] = [:]
            for cwd in cwds {
                results[cwd] = CostScanner.projectCost(cwd: cwd, since: launchDate)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                let changed = self.costCache != results
                self.costCache = results
                if changed {
                    self.delegate?.sidebarNeedsUpdate()
                }
            }
        }
    }

    /// Update the stored all-time cost for a workspace (persisted in session file).
    func updateWorkspaceCost(workspaceId: String, cost: Double) {
        if cost > 0 { workspaceCosts[workspaceId] = cost }
    }

    /// Get the stored all-time cost for a workspace.
    func storedWorkspaceCost(workspaceId: String) -> Double {
        workspaceCosts[workspaceId] ?? 0
    }

    /// Read Claude OAuth credentials from macOS Keychain or ~/.claude/.credentials.json.
    /// Read credentials from file only (no keychain access — won't trigger a system prompt).
    private func readClaudeOAuthCredentialsFromFile() -> [String: Any]? {
        let credsPath = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude/.credentials.json").path
        if let data = FileManager.default.contents(atPath: credsPath),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let oauth = json["claudeAiOauth"] as? [String: Any] {
            return oauth
        }
        return nil
    }

    /// Read credentials from macOS Keychain via Security framework.
    private func readClaudeOAuthCredentialsFromKeychain() -> [String: Any]? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "Claude Code-credentials",
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecSuccess, let data = result as? Data,
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let oauth = json["claudeAiOauth"] as? [String: Any] {
            return oauth
        }
        NSLog("thane: SecItemCopyMatching status=\(status) (0=success, -25293=authFailed, -25308=interactionNotAllowed)")
        return nil
    }

    /// Read credentials from keychain via the `security` CLI tool.
    /// This bypasses SecItemCopyMatching ACL restrictions that can silently
    /// block access from unsigned or dev builds.
    private func readClaudeOAuthCredentialsFromSecurityCLI() -> [String: Any]? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/security")
        process.arguments = ["find-generic-password", "-s", "Claude Code-credentials", "-w"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        do { try process.run() } catch { return nil }
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        // The `-w` flag returns the password value (JSON string) followed by a newline
        if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let oauth = json["claudeAiOauth"] as? [String: Any] {
            return oauth
        }
        return nil
    }

    /// Read credentials: try Security framework, then `security` CLI, then file fallback.
    private func readClaudeOAuthCredentials() -> [String: Any]? {
        if let oauth = readClaudeOAuthCredentialsFromKeychain() { return oauth }
        if let oauth = readClaudeOAuthCredentialsFromSecurityCLI() { return oauth }
        return readClaudeOAuthCredentialsFromFile()
    }

    // MARK: - Agent queue

    func queueSubmit(content: String, workspaceId: String? = nil, priority: Int32 = 0) -> String {
        let id = UUID().uuidString
        let wsId = workspaceId ?? activeWorkspaceId ?? ""
        let now = ISO8601DateFormatter().string(from: Date())
        let entry = QueueEntryInfoDTO(
            id: id, content: content, workspaceId: wsId.isEmpty ? nil : wsId,
            priority: priority, status: .queued, createdAt: now,
            startedAt: nil, completedAt: nil, error: nil,
            inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0, estimatedCostUsd: 0.0,
            model: nil
        )
        queueEntries.append(entry)
        logAuditEvent(workspaceId: wsId, eventType: "QueueTaskSubmitted", severity: .info,
                      description: "Queue task submitted",
                      metadata: ["entry_id": id, "content_preview": String(content.prefix(100))])
        delegate?.workspaceListChanged()
        return id
    }

    func queueList() -> [QueueEntryInfoDTO] {
        return queueEntries
    }

    func queueCancel(entryId: String) -> Bool {
        guard let idx = queueEntries.firstIndex(where: { $0.id == entryId }) else { return false }
        queueEntries[idx] = QueueEntryInfoDTO(
            id: queueEntries[idx].id, content: queueEntries[idx].content,
            workspaceId: queueEntries[idx].workspaceId, priority: queueEntries[idx].priority,
            status: .cancelled, createdAt: queueEntries[idx].createdAt,
            startedAt: queueEntries[idx].startedAt,
            completedAt: ISO8601DateFormatter().string(from: Date()),
            error: nil, inputTokens: queueEntries[idx].inputTokens,
            outputTokens: queueEntries[idx].outputTokens,
            cacheReadTokens: queueEntries[idx].cacheReadTokens,
            cacheWriteTokens: queueEntries[idx].cacheWriteTokens,
            estimatedCostUsd: queueEntries[idx].estimatedCostUsd,
            model: queueEntries[idx].model
        )
        logAuditEvent(workspaceId: "", eventType: "QueueTaskCancelled", severity: .info,
                      description: "Queue task cancelled", metadata: ["entry_id": entryId])
        delegate?.workspaceListChanged()
        return true
    }

    func queueRetry(entryId: String) -> Bool {
        guard let idx = queueEntries.firstIndex(where: { $0.id == entryId }) else { return false }
        queueEntries[idx] = QueueEntryInfoDTO(
            id: queueEntries[idx].id, content: queueEntries[idx].content,
            workspaceId: queueEntries[idx].workspaceId, priority: queueEntries[idx].priority,
            status: .queued, createdAt: queueEntries[idx].createdAt,
            startedAt: nil, completedAt: nil, error: nil,
            inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0, estimatedCostUsd: 0.0,
            model: nil
        )
        delegate?.workspaceListChanged()
        return true
    }

    func queueStatus(entryId: String) -> QueueEntryInfoDTO? {
        return queueEntries.first(where: { $0.id == entryId })
    }

    /// Update the model used by Claude Code for a queue entry.
    func queueUpdateModel(entryId: String, model: String) {
        guard let idx = queueEntries.firstIndex(where: { $0.id == entryId }) else { return }
        let old = queueEntries[idx]
        queueEntries[idx] = QueueEntryInfoDTO(
            id: old.id, content: old.content, workspaceId: old.workspaceId,
            priority: old.priority, status: old.status, createdAt: old.createdAt,
            startedAt: old.startedAt, completedAt: old.completedAt, error: old.error,
            inputTokens: old.inputTokens, outputTokens: old.outputTokens,
            cacheReadTokens: old.cacheReadTokens, cacheWriteTokens: old.cacheWriteTokens,
            estimatedCostUsd: old.estimatedCostUsd,
            model: model
        )
        // Audit: log the model selection
        let wsId = old.workspaceId ?? ""
        logAuditEvent(workspaceId: wsId, eventType: "QueueModelSelected", severity: .info,
                      description: "Claude Code selected model: \(model)",
                      metadata: ["entry_id": entryId, "model": model,
                                 "content_preview": String(old.content.prefix(100))])
        delegate?.workspaceListChanged()
    }

    /// Remove completed/failed/cancelled entries older than `maxAge` seconds.
    func queueRemoveStale(maxAge: TimeInterval) {
        let formatter = ISO8601DateFormatter()
        let cutoff = Date().addingTimeInterval(-maxAge)
        let before = queueEntries.count
        queueEntries.removeAll { entry in
            guard entry.status == .completed || entry.status == .failed || entry.status == .cancelled else {
                return false
            }
            guard let completedStr = entry.completedAt, let completedDate = formatter.date(from: completedStr) else {
                return false
            }
            return completedDate < cutoff
        }
        if queueEntries.count != before {
            delegate?.workspaceListChanged()
        }
    }

    func queueUpdateStatus(entryId: String, status: QueueEntryStatusDTO, error: String? = nil) {
        guard let idx = queueEntries.firstIndex(where: { $0.id == entryId }) else { return }
        let old = queueEntries[idx]
        let now = (status == .completed || status == .failed || status == .cancelled)
            ? ISO8601DateFormatter().string(from: Date()) : old.completedAt
        let startedNow = (status == .running && old.startedAt == nil)
            ? ISO8601DateFormatter().string(from: Date()) : old.startedAt
        queueEntries[idx] = QueueEntryInfoDTO(
            id: old.id, content: old.content, workspaceId: old.workspaceId,
            priority: old.priority, status: status, createdAt: old.createdAt,
            startedAt: startedNow, completedAt: now, error: error,
            inputTokens: old.inputTokens, outputTokens: old.outputTokens,
            cacheReadTokens: old.cacheReadTokens, cacheWriteTokens: old.cacheWriteTokens,
            estimatedCostUsd: old.estimatedCostUsd,
            model: old.model
        )

        // Persist terminal states to queue_history.json for the Processed panel
        if status == .completed || status == .failed || status == .cancelled {
            appendToQueueHistory(queueEntries[idx])
        }

        delegate?.workspaceListChanged()
    }

    /// Append a completed/failed/cancelled entry to ~/Library/Application Support/thane/queue_history.json.
    private func appendToQueueHistory(_ entry: QueueEntryInfoDTO) {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let historyFile = appSupport.appendingPathComponent("thane/queue_history.json")

        var array: [[String: Any]] = []
        if let data = try? Data(contentsOf: historyFile),
           let existing = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] {
            array = existing
        }

        let statusStr: String = {
            switch entry.status {
            case .completed: return "completed"
            case .failed: return "failed"
            case .cancelled: return "cancelled"
            case .queued: return "queued"
            case .running: return "running"
            case .pausedTokenLimit: return "paused_token_limit"
            case .pausedByUser: return "paused_by_user"
            }
        }()

        var dict: [String: Any] = [
            "id": entry.id,
            "content": entry.content,
            "status": statusStr,
            "createdAt": entry.createdAt,
            "priority": entry.priority,
            "inputTokens": entry.inputTokens,
            "outputTokens": entry.outputTokens,
            "costUsd": entry.estimatedCostUsd,
        ]
        if let ws = entry.workspaceId { dict["workspaceId"] = ws }
        if let started = entry.startedAt { dict["startedAt"] = started }
        if let completed = entry.completedAt { dict["completedAt"] = completed }
        if let error = entry.error { dict["error"] = error }
        if let model = entry.model { dict["model"] = model }

        array.append(dict)

        if let data = try? JSONSerialization.data(withJSONObject: array, options: [.prettyPrinted]) {
            try? data.write(to: historyFile, options: .atomic)
        }
    }

    // MARK: - Audit log

    /// In-memory audit event storage.
    private var auditEvents: [AuditEventInfoDTO] = []
    private static let maxAuditEvents = 10_000

    /// Callback for security alerts that should be shown as modal dialogs.
    var onSecurityAlert: ((AuditSeverityDTO, String, String) -> Void)?

    func logAuditEvent(workspaceId: String, panelId: String? = nil,
                       eventType: String, severity: AuditSeverityDTO,
                       description: String, metadata: [String: Any] = [:],
                       timestamp: String? = nil, agentName: String? = nil) {
        let metaJson = (try? JSONSerialization.data(withJSONObject: metadata))
            .flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        let event = AuditEventInfoDTO(
            id: UUID().uuidString,
            timestamp: timestamp ?? ISO8601DateFormatter().string(from: Date()),
            workspaceId: workspaceId,
            panelId: panelId,
            eventType: eventType,
            severity: severity,
            description: description,
            metadataJson: metaJson,
            agentName: agentName
        )
        auditEvents.append(event)
        if auditEvents.count > Self.maxAuditEvents {
            auditEvents.removeFirst(auditEvents.count - Self.maxAuditEvents)
        }
    }

    func listAuditEvents(minSeverity: AuditSeverityDTO? = nil) -> [AuditEventInfoDTO] {
        guard let min = minSeverity else { return auditEvents }
        let minOrd = min.ordinal
        return auditEvents.filter { $0.severity.ordinal >= minOrd }
    }

    func exportAuditJson() -> String {
        let dicts = auditEvents.map { e -> [String: Any] in
            var d: [String: Any] = [
                "id": e.id, "timestamp": e.timestamp,
                "workspace_id": e.workspaceId,
                "event_type": e.eventType,
                "severity": e.severity.label,
                "description": e.description,
                "metadata": e.metadataJson,
            ]
            if let pid = e.panelId { d["panel_id"] = pid }
            return d
        }
        guard let data = try? JSONSerialization.data(withJSONObject: dicts, options: .prettyPrinted),
              let str = String(data: data, encoding: .utf8) else { return "[]" }
        return str
    }

    func clearAuditLog() {
        let count = auditEvents.count
        logAuditEvent(
            workspaceId: "", eventType: "AuditLogCleared", severity: .critical,
            description: "Audit log cleared (\(count) events removed)",
            metadata: ["events_cleared": "\(count)"])
        // Keep only the clear marker (last event)
        if let last = auditEvents.last {
            auditEvents = [last]
        }
    }

    var auditEventCount: Int { auditEvents.count }

    /// Async version: scans JSONL files on background thread, processes results on main thread.
    func scanSessionPromptsAsync(completion: @escaping @MainActor () -> Void) {
        // Capture workspace data for background work
        let cwdPairs = workspaces.map { ($0.id, $0.cwd) }
        let launchDate = appLaunchDate

        DispatchQueue.global(qos: .utility).async {
            // Do all file I/O on background thread
            var results: [(String, [PromptScanner.PromptRecord])] = []
            for (wsId, cwd) in cwdPairs {
                let records = PromptScanner.scanPrompts(cwd: cwd)
                if !records.isEmpty {
                    results.append((wsId, records))
                }
            }

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                // Process results on main thread (audit logging touches main-actor state)
                let launchIso = ISO8601DateFormatter().string(from: launchDate)
                for (wsId, records) in results {
                    self.processPromptRecords(records, workspaceId: wsId, launchIso: launchIso)
                }
                completion()
            }
        }
    }

    /// Process scanned prompt records and log audit events.
    private func processPromptRecords(_ records: [PromptScanner.PromptRecord], workspaceId: String, launchIso: String) {
        for record in records {
            guard !record.timestamp.isEmpty, record.timestamp >= launchIso else { continue }
            guard !record.uuid.isEmpty,
                  seenPromptUuids.insert(record.uuid).inserted else { continue }
            logPromptRecord(record, workspaceId: workspaceId)
        }
    }

    /// Scan Claude Code JSONL session files for new prompts and tool use,
    /// and log them as audit events. Synchronous version (used at shutdown).
    func scanSessionPrompts() {
        let launchIso = ISO8601DateFormatter().string(from: appLaunchDate)
        for ws in workspaces {
            let records = PromptScanner.scanPrompts(cwd: ws.cwd)
            processPromptRecords(records, workspaceId: ws.id, launchIso: launchIso)
        }
    }

    private func logPromptRecord(_ record: PromptScanner.PromptRecord, workspaceId wsId: String) {
        if record.role == "user" {
            let short = String(record.text.prefix(100))
            let piiFindings = AuditScanner.detectPii(text: record.text)
            if !piiFindings.isEmpty {
                let findings = piiFindings.joined(separator: ", ")
                logAuditEvent(
                    workspaceId: wsId, eventType: "PiiLeaked", severity: .alert,
                    description: "PII sent to model: \(findings)",
                    metadata: ["findings": findings, "direction": "to_model",
                               "uuid": record.uuid],
                    timestamp: record.timestamp)
                onSecurityAlert?(.alert, "PII Sent to AI Model",
                    "Personally identifiable information was detected in a prompt sent to Claude:\n\n\(findings)\n\nThis data may be stored in conversation logs.")
            }
            let redactedPrompt = AuditScanner.redactSecrets(record.text)
            logAuditEvent(
                workspaceId: wsId, eventType: "UserPrompt", severity: .info,
                description: "Claude prompt: \(short)",
                metadata: ["prompt": redactedPrompt, "session_id": record.sessionId,
                           "uuid": record.uuid],
                timestamp: record.timestamp)
        } else if record.role == "tool_use" {
            let severity: AuditSeverityDTO =
                record.text.hasPrefix("Edit:") || record.text.hasPrefix("Write:")
                || record.text.hasPrefix("Bash:") ? .warning : .info

            let eventType: String
            if record.text.hasPrefix("Bash:") {
                eventType = "CommandExecuted"
            } else if record.text.hasPrefix("Write:") {
                eventType = "FileWrite"
            } else if record.text.hasPrefix("Edit:") {
                eventType = "FileWrite"
            } else if record.text.hasPrefix("Read:") {
                eventType = "FileRead"
            } else {
                eventType = "ToolUse"
            }

            logAuditEvent(
                workspaceId: wsId, eventType: eventType, severity: severity,
                description: record.text,
                metadata: ["session_id": record.sessionId,
                           "uuid": record.uuid,
                           "tool_type": eventType],
                timestamp: record.timestamp)

            let filePath: String? = {
                let prefixes = ["Read: ", "Write: ", "Edit: "]
                for p in prefixes {
                    if record.text.hasPrefix(p) {
                        return String(record.text.dropFirst(p.count))
                    }
                }
                return nil
            }()
            if let path = filePath {
                let sensitivePatterns: [(String, Bool)] = [
                    (".ssh/", true), (".gnupg/", false), (".aws/", false),
                    (".env", false), ("credentials", false), ("secrets", false),
                    (".key", true), (".pem", true), (".p12", true), (".pfx", true),
                    (".netrc", false), (".pgpass", false),
                ]
                for (pattern, isKey) in sensitivePatterns {
                    if path.contains(pattern) {
                        let sev: AuditSeverityDTO = isKey ? .critical : .alert
                        let evtType = isKey ? "PrivateKeyAccess" : "SecretAccess"
                        logAuditEvent(
                            workspaceId: wsId, eventType: evtType, severity: sev,
                            description: "Sensitive file accessed by tool: \(path)",
                            metadata: ["path": path, "tool": eventType,
                                       "uuid": record.uuid],
                            timestamp: record.timestamp)
                        onSecurityAlert?(sev, "Sensitive File Access",
                            "Claude Code accessed a sensitive file:\n\n\(path)\n\nReview the audit log for details.")
                        break
                    }
                }
            }
        } else if record.role == "response" {
            let short = String(record.text.prefix(150))
            let piiInResp = AuditScanner.detectPii(text: record.text)
            if !piiInResp.isEmpty {
                let findings = piiInResp.joined(separator: ", ")
                logAuditEvent(
                    workspaceId: wsId, eventType: "PiiInResponse", severity: .alert,
                    description: "PII in model response: \(findings)",
                    metadata: ["findings": findings, "direction": "from_model",
                               "uuid": record.uuid],
                    timestamp: record.timestamp)
            }
            let redactedResponse = AuditScanner.redactSecrets(String(record.text.prefix(2000)))
            logAuditEvent(
                workspaceId: wsId, eventType: "AssistantResponse", severity: .info,
                description: "Claude: \(short)",
                metadata: ["response": redactedResponse,
                           "session_id": record.sessionId,
                           "uuid": record.uuid],
                timestamp: record.timestamp)
        }
    }

    /// Number of events already flushed to disk.
    private var auditFlushedCount = 0

    /// Maximum audit log retention in days (free tier).
    static let auditRetentionDays = 7

    /// Directory where daily audit log files are stored.
    static var auditLogDirectory: String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return (home as NSString).appendingPathComponent("Library/Application Support/thane/audit")
    }

    /// Date formatter for daily audit log filenames (audit-2026-03-25.jsonl).
    private static let auditDateFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd"
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        return f
    }()

    /// Returns the file path for a given day's audit log.
    private static func auditFilePath(for date: Date) -> String {
        let fileName = "audit-\(auditDateFormatter.string(from: date)).jsonl"
        return (auditLogDirectory as NSString).appendingPathComponent(fileName)
    }

    /// Serialize an audit event to a JSONL dictionary.
    private static func auditEventToDict(_ event: AuditEventInfoDTO) -> [String: Any] {
        let dict: [String: Any] = [
            "id": event.id, "timestamp": event.timestamp,
            "workspace_id": event.workspaceId,
            "panel_id": event.panelId ?? "",
            "event_type": event.eventType,
            "severity": event.severity.label,
            "description": event.description,
            "metadata": event.metadataJson,
        ]
        return dict
    }

    /// Flush new audit events to disk (JSONL append to daily file).
    /// Sets file permissions to owner-only read/write (0600) to prevent casual tampering.
    func flushAuditLog() {
        guard auditFlushedCount < auditEvents.count else { return }
        let auditDir = Self.auditLogDirectory
        try? FileManager.default.createDirectory(atPath: auditDir, withIntermediateDirectories: true)

        let filePath = Self.auditFilePath(for: Date())
        let handle: FileHandle
        if FileManager.default.fileExists(atPath: filePath) {
            guard let h = FileHandle(forWritingAtPath: filePath) else { return }
            h.seekToEndOfFile()
            handle = h
        } else {
            FileManager.default.createFile(atPath: filePath, contents: nil)
            try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: filePath)
            guard let h = FileHandle(forWritingAtPath: filePath) else { return }
            handle = h
        }
        var hasCritical = false
        for event in auditEvents[auditFlushedCount...] {
            let dict = Self.auditEventToDict(event)
            if let data = try? JSONSerialization.data(withJSONObject: dict),
               var line = String(data: data, encoding: .utf8) {
                line.append("\n")
                handle.write(line.data(using: .utf8)!)
            }
            if event.severity == .critical { hasCritical = true }
        }
        if hasCritical { handle.synchronizeFile() }
        handle.closeFile()
        auditFlushedCount = auditEvents.count
    }

    /// Load audit events from daily log files for a date range.
    /// Returns events sorted newest-first.
    func loadAuditEvents(from startDate: Date, to endDate: Date) -> [AuditEventInfoDTO] {
        let auditDir = Self.auditLogDirectory
        let fm = FileManager.default
        guard fm.fileExists(atPath: auditDir) else { return [] }

        let isoFormatter = ISO8601DateFormatter()
        var events: [AuditEventInfoDTO] = []
        let calendar = Calendar.current

        // Iterate each day in range
        var current = calendar.startOfDay(for: startDate)
        let end = calendar.startOfDay(for: endDate)
        while current <= end {
            let filePath = Self.auditFilePath(for: current)
            if let content = try? String(contentsOfFile: filePath, encoding: .utf8) {
                for line in content.split(separator: "\n") where !line.isEmpty {
                    guard let data = line.data(using: .utf8),
                          let dict = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
                    let event = Self.parseDictToAuditEvent(dict)
                    // Filter by exact timestamp range
                    if let ts = isoFormatter.date(from: event.timestamp),
                       ts >= startDate, ts <= endDate {
                        events.append(event)
                    } else if event.timestamp.isEmpty {
                        // Malformed entry — include it on that day
                        events.append(event)
                    } else {
                        // If ISO parse fails, include it (the day file implies it's in range)
                        events.append(event)
                    }
                }
            }
            guard let next = calendar.date(byAdding: .day, value: 1, to: current) else { break }
            current = next
        }

        return events.reversed()
    }

    /// Parse a JSONL dictionary back into an AuditEventInfoDTO.
    private static func parseDictToAuditEvent(_ dict: [String: Any]) -> AuditEventInfoDTO {
        let severityStr = dict["severity"] as? String ?? "Info"
        let severity: AuditSeverityDTO
        switch severityStr.lowercased() {
        case "warning", "warn": severity = .warning
        case "alert": severity = .alert
        case "critical", "crit": severity = .critical
        default: severity = .info
        }
        return AuditEventInfoDTO(
            id: dict["id"] as? String ?? UUID().uuidString,
            timestamp: dict["timestamp"] as? String ?? "",
            workspaceId: dict["workspace_id"] as? String ?? "",
            panelId: dict["panel_id"] as? String,
            eventType: dict["event_type"] as? String ?? "Unknown",
            severity: severity,
            description: dict["description"] as? String ?? "",
            metadataJson: dict["metadata"] as? String ?? "{}",
            agentName: dict["agent_name"] as? String
        )
    }

    /// Delete audit log files older than the retention period.
    /// Called periodically (e.g. once per flush cycle).
    func purgeStaleAuditLogs() {
        let auditDir = Self.auditLogDirectory
        let fm = FileManager.default
        guard let files = try? fm.contentsOfDirectory(atPath: auditDir) else { return }

        let calendar = Calendar.current
        guard let cutoff = calendar.date(byAdding: .day, value: -Self.auditRetentionDays, to: Date()) else { return }
        let cutoffDay = calendar.startOfDay(for: cutoff)

        for file in files {
            guard file.hasPrefix("audit-"), file.hasSuffix(".jsonl") else { continue }
            // Extract date from filename: audit-2026-03-25.jsonl
            let dateStr = file
                .replacingOccurrences(of: "audit-", with: "")
                .replacingOccurrences(of: ".jsonl", with: "")
            guard let fileDate = Self.auditDateFormatter.date(from: dateStr) else { continue }
            if fileDate < cutoffDay {
                let fullPath = (auditDir as NSString).appendingPathComponent(file)
                try? fm.removeItem(atPath: fullPath)
                NSLog("thane: purged stale audit log: \(file)")
            }
        }

        // Also remove the legacy single audit.jsonl if it exists
        let legacyPath = (auditDir as NSString).appendingPathComponent("audit.jsonl")
        if fm.fileExists(atPath: legacyPath) {
            try? fm.removeItem(atPath: legacyPath)
            NSLog("thane: removed legacy audit.jsonl")
        }
    }

    /// Returns the available date range for audit logs (oldest file to today).
    func auditLogDateRange() -> (earliest: Date, latest: Date) {
        let auditDir = Self.auditLogDirectory
        let fm = FileManager.default
        let today = Date()
        guard let files = try? fm.contentsOfDirectory(atPath: auditDir) else {
            return (today, today)
        }

        var earliest = today
        for file in files {
            guard file.hasPrefix("audit-"), file.hasSuffix(".jsonl") else { continue }
            let dateStr = file
                .replacingOccurrences(of: "audit-", with: "")
                .replacingOccurrences(of: ".jsonl", with: "")
            if let fileDate = Self.auditDateFormatter.date(from: dateStr), fileDate < earliest {
                earliest = fileDate
            }
        }
        return (earliest, today)
    }

    // MARK: - Notifications (additional)

    func clearNotifications() {
        notifications.removeAll()
    }

    // MARK: - Sandbox (Swift-local state — Rust bridge workspace manager is separate)

    func sandboxStatus(workspaceId: String) -> SandboxInfoDTO? {
        return sandboxPolicies[workspaceId]
    }

    func sandboxEnable(workspaceId: String) throws {
        let ws = workspaces.first(where: { $0.id == workspaceId })
        let cwd = ws?.cwd ?? FileManager.default.homeDirectoryForCurrentUser.path
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        NSLog("thane: sandboxEnable called for workspace \(workspaceId), cwd=\(cwd)")

        // Initialize a confined policy with sensible defaults (matching Rust SandboxPolicy::confined_to)
        sandboxPolicies[workspaceId] = SandboxInfoDTO(
            enabled: true,
            rootDir: cwd,
            readOnlyPaths: ["/usr", "/bin", "/sbin", "/etc", "/opt",
                            // Home directory (for ls ~, readdir, shell startup)
                            home,
                            // Shell dotfiles
                            "\(home)/.bashrc", "\(home)/.bash_profile", "\(home)/.bash_logout",
                            "\(home)/.zshrc", "\(home)/.zshenv", "\(home)/.zprofile",
                            "\(home)/.profile", "\(home)/.gitconfig",
                            // Toolchains
                            "\(home)/.cargo", "\(home)/.rustup", "\(home)/.nvm",
                            // User-installed binaries (Claude Code, codex, pipx, etc.)
                            "\(home)/.local"],
            readWritePaths: [cwd, "/dev", "/tmp", "/private/tmp",
                             // Tool configs (gh/gcloud/op credentials denied separately)
                             "\(home)/.config",
                             // Build tool caches
                             "\(home)/.cache",
                             // Claude Code
                             "\(home)/.claude", "\(home)/.claude.json",
                             // OpenAI Codex CLI
                             "\(home)/.codex"],
            deniedPaths: [
                // SSH & GPG keys
                "\(home)/.ssh", "\(home)/.gnupg", "\(home)/.pgpass",
                // Cloud provider credentials
                "\(home)/.aws", "\(home)/.config/gcloud", "\(home)/.azure",
                "\(home)/.docker", "\(home)/.kube",
                // Package manager auth tokens
                "\(home)/.npmrc", "\(home)/.pypirc",
                "\(home)/.gem/credentials", "\(home)/.config/pip",
                // JVM ecosystem credentials
                "\(home)/.m2/settings.xml", "\(home)/.m2/settings-security.xml",
                "\(home)/.gradle/gradle.properties",
                "\(home)/.ivy2/.credentials", "\(home)/.sbt/credentials",
                // Environment files with secrets
                "\(home)/.env", "\(home)/.env.local", "\(home)/.env.production",
                // Network auth & git credentials
                "\(home)/.netrc", "\(home)/.git-credentials",
                // GitHub CLI, Terraform, 1Password
                "\(home)/.config/gh",
                "\(home)/.terraform.d/credentials.tfrc.json",
                "\(home)/.config/op",
                // Claude Code credentials
                "\(home)/.claude/.credentials.json",
                // macOS-specific
                "\(home)/Library/Keychains", "\(home)/Library/Cookies",
            ],
            allowNetwork: true,
            maxOpenFiles: 1024, maxWriteBytes: 1_073_741_824, maxCpuSeconds: 600,
            enforcement: .enforcing
        )

        logAuditEvent(workspaceId: workspaceId, eventType: "SandboxToggle", severity: .warning,
                      description: "Sandbox enabled", metadata: ["enabled": true, "root_dir": cwd])
        delegate?.workspaceListChanged()
    }

    func sandboxDisable(workspaceId: String) {
        sandboxPolicies.removeValue(forKey: workspaceId)
        logAuditEvent(workspaceId: workspaceId, eventType: "SandboxToggle", severity: .warning,
                      description: "Sandbox disabled", metadata: ["enabled": false])
        delegate?.workspaceListChanged()
    }

    func isSandboxed(workspaceId: String) -> Bool {
        sandboxPolicies[workspaceId]?.enabled ?? false
    }

    func sandboxSetEnforcement(workspaceId: String, level: String) {
        guard var policy = sandboxPolicies[workspaceId] else { return }
        let dto: EnforcementLevelDTO
        switch level {
        case "permissive": dto = .permissive
        case "strict": dto = .strict
        default: dto = .enforcing
        }
        policy = SandboxInfoDTO(
            enabled: policy.enabled, rootDir: policy.rootDir,
            readOnlyPaths: policy.readOnlyPaths, readWritePaths: policy.readWritePaths,
            deniedPaths: policy.deniedPaths, allowNetwork: policy.allowNetwork,
            maxOpenFiles: policy.maxOpenFiles, maxWriteBytes: policy.maxWriteBytes,
            maxCpuSeconds: policy.maxCpuSeconds, enforcement: dto
        )
        sandboxPolicies[workspaceId] = policy
    }

    func sandboxSetNetwork(workspaceId: String, allow: Bool) {
        guard var policy = sandboxPolicies[workspaceId] else { return }
        policy = SandboxInfoDTO(
            enabled: policy.enabled, rootDir: policy.rootDir,
            readOnlyPaths: policy.readOnlyPaths, readWritePaths: policy.readWritePaths,
            deniedPaths: policy.deniedPaths, allowNetwork: allow,
            maxOpenFiles: policy.maxOpenFiles, maxWriteBytes: policy.maxWriteBytes,
            maxCpuSeconds: policy.maxCpuSeconds, enforcement: policy.enforcement
        )
        sandboxPolicies[workspaceId] = policy
    }

    func sandboxAllowPath(workspaceId: String, path: String, writable: Bool) throws {
        guard var policy = sandboxPolicies[workspaceId] else { return }
        var roPaths = policy.readOnlyPaths
        var rwPaths = policy.readWritePaths
        if writable { rwPaths.append(path) } else { roPaths.append(path) }
        policy = SandboxInfoDTO(
            enabled: policy.enabled, rootDir: policy.rootDir,
            readOnlyPaths: roPaths, readWritePaths: rwPaths,
            deniedPaths: policy.deniedPaths, allowNetwork: policy.allowNetwork,
            maxOpenFiles: policy.maxOpenFiles, maxWriteBytes: policy.maxWriteBytes,
            maxCpuSeconds: policy.maxCpuSeconds, enforcement: policy.enforcement
        )
        sandboxPolicies[workspaceId] = policy
    }

    func sandboxDenyPath(workspaceId: String, path: String) throws {
        guard var policy = sandboxPolicies[workspaceId] else { return }
        var denied = policy.deniedPaths
        denied.append(path)
        policy = SandboxInfoDTO(
            enabled: policy.enabled, rootDir: policy.rootDir,
            readOnlyPaths: policy.readOnlyPaths, readWritePaths: policy.readWritePaths,
            deniedPaths: denied, allowNetwork: policy.allowNetwork,
            maxOpenFiles: policy.maxOpenFiles, maxWriteBytes: policy.maxWriteBytes,
            maxCpuSeconds: policy.maxCpuSeconds, enforcement: policy.enforcement
        )
        sandboxPolicies[workspaceId] = policy
    }

    /// Get the sandbox-exec command to launch a sandboxed shell for a workspace.
    /// Returns nil if sandbox is not enabled. Permissive mode still applies
    /// a Seatbelt profile in audit-only mode (violations logged to syslog).
    func sandboxGetCommand(workspaceId: String, shell: String) -> (executable: String, args: [String], extraEnv: [String])? {
        guard let policy = sandboxPolicies[workspaceId], policy.enabled else { return nil }

        // Generate Seatbelt profile via the Rust bridge
        if let cmd = rustCoreBridge?.sandboxGetCommand(workspaceId: workspaceId, shell: shell) {
            return (cmd.executable, cmd.args, cmd.extraEnv)
        }

        // Fallback: generate sandbox-exec command locally
        guard FileManager.default.fileExists(atPath: "/usr/bin/sandbox-exec") else { return nil }
        let profile = generateSeatbeltProfile(policy)
        let envVars = ["THANE_SANDBOX=1", "THANE_SANDBOX_ROOT=\(policy.rootDir)"]
        return ("/usr/bin/sandbox-exec", ["-p", profile, "--", shell, "-l"], envVars)
    }

    /// Get the sandbox-exec command for a queue task, with mode-specific overrides.
    ///
    /// In "workspace" mode, uses the workspace's existing sandbox policy.
    /// In "strict" mode, additionally disables network and restricts exec to system binaries.
    /// Returns nil if sandbox is not enabled for the workspace.
    func sandboxGetQueueCommand(workspaceId: String, shell: String, mode: String) -> (executable: String, args: [String], extraEnv: [String])? {
        guard let basePolicy = sandboxPolicies[workspaceId], basePolicy.enabled else { return nil }

        // For workspace mode, use the policy as-is
        let policy: SandboxInfoDTO
        if mode == "strict" {
            policy = SandboxInfoDTO(
                enabled: true,
                rootDir: basePolicy.rootDir,
                readOnlyPaths: basePolicy.readOnlyPaths,
                readWritePaths: basePolicy.readWritePaths,
                deniedPaths: basePolicy.deniedPaths,
                allowNetwork: false,
                maxOpenFiles: basePolicy.maxOpenFiles,
                maxWriteBytes: basePolicy.maxWriteBytes,
                maxCpuSeconds: basePolicy.maxCpuSeconds,
                enforcement: .strict
            )
        } else {
            policy = basePolicy
        }

        guard FileManager.default.fileExists(atPath: "/usr/bin/sandbox-exec") else { return nil }
        let profile = generateSeatbeltProfile(policy)
        var envVars = ["THANE_SANDBOX=1", "THANE_SANDBOX_ROOT=\(policy.rootDir)"]
        if mode == "strict" {
            envVars.append("THANE_SANDBOX_STRICT=1")
        }
        return ("/usr/bin/sandbox-exec", ["-p", profile, "--", shell, "-l"], envVars)
    }

    // MARK: - Queue Sandbox

    func queueSandboxStatus() -> SandboxInfoDTO? {
        return queueSandboxPolicy
    }

    func queueSandboxEnable() {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let baseDir = configGet(key: "queue-working-dir")
            ?? "\(home)/thane-tasks"

        queueSandboxPolicy = SandboxInfoDTO(
            enabled: true,
            rootDir: baseDir,
            readOnlyPaths: ["/usr", "/bin", "/sbin", "/etc", "/opt",
                            home,
                            "\(home)/.bashrc", "\(home)/.bash_profile", "\(home)/.bash_logout",
                            "\(home)/.zshrc", "\(home)/.zshenv", "\(home)/.zprofile",
                            "\(home)/.profile", "\(home)/.gitconfig",
                            "\(home)/.cargo", "\(home)/.rustup", "\(home)/.nvm",
                            "\(home)/.local"],
            readWritePaths: [baseDir, "/dev", "/tmp", "/private/tmp",
                             "\(home)/.config", "\(home)/.cache",
                             "\(home)/.claude", "\(home)/.claude.json",
                             "\(home)/.codex"],
            deniedPaths: [
                "\(home)/.ssh", "\(home)/.gnupg", "\(home)/.pgpass",
                "\(home)/.aws", "\(home)/.config/gcloud", "\(home)/.azure",
                "\(home)/.docker", "\(home)/.kube",
                "\(home)/.npmrc", "\(home)/.pypirc",
                "\(home)/.gem/credentials", "\(home)/.config/pip",
                "\(home)/.m2/settings.xml", "\(home)/.m2/settings-security.xml",
                "\(home)/.gradle/gradle.properties",
                "\(home)/.ivy2/.credentials", "\(home)/.sbt/credentials",
                "\(home)/.env", "\(home)/.env.local", "\(home)/.env.production",
                "\(home)/.netrc", "\(home)/.git-credentials",
                "\(home)/.config/gh",
                "\(home)/.terraform.d/credentials.tfrc.json",
                "\(home)/.config/op",
                "\(home)/.claude/.credentials.json",
                "\(home)/Library/Keychains", "\(home)/Library/Cookies",
            ],
            allowNetwork: false,
            maxOpenFiles: 1024, maxWriteBytes: 1_073_741_824, maxCpuSeconds: 600,
            enforcement: .enforcing
        )

        logAuditEvent(workspaceId: "", eventType: "QueueSandboxToggle", severity: .warning,
                      description: "Queue sandbox enabled", metadata: ["enabled": true, "root_dir": baseDir])
    }

    func queueSandboxDisable() {
        queueSandboxPolicy = SandboxInfoDTO(
            enabled: false,
            rootDir: queueSandboxPolicy?.rootDir ?? "",
            readOnlyPaths: queueSandboxPolicy?.readOnlyPaths ?? [],
            readWritePaths: queueSandboxPolicy?.readWritePaths ?? [],
            deniedPaths: queueSandboxPolicy?.deniedPaths ?? [],
            allowNetwork: queueSandboxPolicy?.allowNetwork ?? true,
            maxOpenFiles: queueSandboxPolicy?.maxOpenFiles,
            maxWriteBytes: queueSandboxPolicy?.maxWriteBytes,
            maxCpuSeconds: queueSandboxPolicy?.maxCpuSeconds,
            enforcement: queueSandboxPolicy?.enforcement ?? .enforcing
        )

        logAuditEvent(workspaceId: "", eventType: "QueueSandboxToggle", severity: .warning,
                      description: "Queue sandbox disabled", metadata: ["enabled": false])
    }

    func queueSandboxSetEnforcement(_ level: EnforcementLevelDTO) {
        guard var policy = queueSandboxPolicy else { return }
        policy = SandboxInfoDTO(
            enabled: policy.enabled, rootDir: policy.rootDir,
            readOnlyPaths: policy.readOnlyPaths, readWritePaths: policy.readWritePaths,
            deniedPaths: policy.deniedPaths, allowNetwork: policy.allowNetwork,
            maxOpenFiles: policy.maxOpenFiles, maxWriteBytes: policy.maxWriteBytes,
            maxCpuSeconds: policy.maxCpuSeconds, enforcement: level
        )
        queueSandboxPolicy = policy
    }

    func queueSandboxSetNetwork(_ allow: Bool) {
        guard var policy = queueSandboxPolicy else { return }
        policy = SandboxInfoDTO(
            enabled: policy.enabled, rootDir: policy.rootDir,
            readOnlyPaths: policy.readOnlyPaths, readWritePaths: policy.readWritePaths,
            deniedPaths: policy.deniedPaths, allowNetwork: allow,
            maxOpenFiles: policy.maxOpenFiles, maxWriteBytes: policy.maxWriteBytes,
            maxCpuSeconds: policy.maxCpuSeconds, enforcement: policy.enforcement
        )
        queueSandboxPolicy = policy
    }

    /// Generate a Seatbelt profile for the queue sandbox policy.
    func generateQueueSandboxProfile(_ policy: SandboxInfoDTO) -> String {
        return generateSeatbeltProfile(policy)
    }

    /// Generate a deny-default SBPL Seatbelt profile from a sandbox policy.
    /// Denies all file operations by default, then allows specific paths.
    /// Denied paths (secrets) come last to override broader allows.
    private func generateSeatbeltProfile(_ policy: SandboxInfoDTO) -> String {
        var lines: [String] = []

        lines.append("(version 1)")
        lines.append("")

        // Allow system operations (macOS needs these for dyld, Mach, etc.)
        lines.append("(allow process*)")
        lines.append("(allow sysctl*)")
        lines.append("(allow mach*)")
        lines.append("(allow ipc*)")
        lines.append("(allow signal)")
        lines.append("(allow system*)")
        lines.append("")

        // Deny all file access by default
        lines.append("(deny file*)")
        lines.append("")

        // System paths (read-only)
        for sysPath in ["/System", "/Library", "/usr", "/bin", "/sbin", "/etc",
                        "/opt", "/Applications", "/private/etc", "/private/var"] {
            lines.append("(allow file-read* (subpath \"\(sysPath)\"))")
        }
        // Root directory needs full read (zsh reads / during startup)
        lines.append("(allow file-read* (literal \"/\"))")
        // Path traversal literals — macOS needs file-read-metadata on parent
        // directories to resolve paths to children.
        for literal in ["/Users", "/var", "/home", "/private"] {
            lines.append("(allow file-read-metadata (literal \"\(literal)\"))")
        }
        lines.append("")

        // Device and temp paths (read-write)
        for rwSys in ["/dev", "/tmp", "/private/tmp", "/var/folders", "/private/var/folders"] {
            lines.append("(allow file* (subpath \"\(rwSys)\"))")
        }
        lines.append("")

        // Policy read-only paths (shell dotfiles, toolchains)
        for path in policy.readOnlyPaths {
            let escaped = path.replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            let name = (path as NSString).lastPathComponent
            let looksLikeFile = name.contains(".") && !name.hasPrefix(".")
            if looksLikeFile {
                lines.append("(allow file-read* (literal \"\(escaped)\"))")
            } else {
                lines.append("(allow file-read* (subpath \"\(escaped)\"))")
            }
        }
        lines.append("")

        // Working directory (full access)
        lines.append("(allow file* (subpath \"\(policy.rootDir)\"))")
        for path in policy.readWritePaths where path != policy.rootDir {
            lines.append("(allow file* (subpath \"\(path)\"))")
        }
        lines.append("")

        // Denied paths — MUST come last (overrides allows)
        for path in policy.deniedPaths {
            let escaped = path.replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            let name = (path as NSString).lastPathComponent
            let looksLikeFile = name.contains(".") && !name.hasPrefix(".")
            if looksLikeFile {
                lines.append("(deny file-read* file-write* (literal \"\(escaped)\"))")
            } else {
                lines.append("(deny file-read* file-write* (subpath \"\(escaped)\"))")
            }
        }
        lines.append("")

        // Strict mode: deny-default exec, then allowlist system binaries only.
        // This prevents execution of arbitrary user-writable binaries.
        if policy.enforcement == .strict {
            lines.append("(deny process-exec)")
            lines.append("(allow process-exec (subpath \"/usr/bin\"))")
            lines.append("(allow process-exec (subpath \"/usr/sbin\"))")
            lines.append("(allow process-exec (subpath \"/usr/libexec\"))")
            lines.append("(allow process-exec (subpath \"/bin\"))")
            lines.append("(allow process-exec (subpath \"/sbin\"))")
            lines.append("(allow process-exec (subpath \"/System\"))")
            lines.append("")
        }

        // Network restrictions — block all outbound and bind operations
        if !policy.allowNetwork {
            lines.append("(deny network*)")
            lines.append("")
        }

        return lines.joined(separator: "\n")
    }

    // MARK: - Port scanning

    /// Update listening ports for a workspace given its shell PIDs.
    func updatePorts(workspaceId: String, ports: [UInt16]) {
        if workspacePorts[workspaceId] != ports {
            workspacePorts[workspaceId] = ports
            delegate?.sidebarNeedsUpdate()
        }
    }

    /// Scan listening TCP ports filtered to the given PIDs (and their descendants) using lsof.
    /// Should be called on a background queue; returns results synchronously.
    nonisolated static func scanListeningPorts(pids: [Int32]) -> [UInt16] {
        guard !pids.isEmpty else { return [] }

        // Expand to include all descendant PIDs (child processes spawned by shell)
        var allPids = Set(pids)
        for pid in pids {
            let descendants = getDescendantPids(pid)
            allPids.formUnion(descendants)
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/lsof")
        process.arguments = ["-iTCP", "-sTCP:LISTEN", "-nP", "-F", "pn"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()
        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else { return [] }
            let output = String(data: data, encoding: .utf8) ?? ""
            return parseLsofOutput(output, filterPids: allPids)
        } catch {
            return []
        }
    }

    /// Get all descendant PIDs of a given PID using pgrep.
    private nonisolated static func getDescendantPids(_ pid: Int32) -> [Int32] {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        process.arguments = ["-P", "\(pid)"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()
        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            let output = String(data: data, encoding: .utf8) ?? ""
            var result: [Int32] = []
            for line in output.split(separator: "\n") {
                if let childPid = Int32(line) {
                    result.append(childPid)
                    // Recurse one level deeper (grandchildren)
                    result.append(contentsOf: getDescendantPids(childPid))
                }
            }
            return result
        } catch {
            return []
        }
    }

    private nonisolated static func parseLsofOutput(_ output: String, filterPids: Set<Int32>) -> [UInt16] {
        var ports = Set<UInt16>()
        var currentPid: Int32 = 0
        for line in output.split(separator: "\n") {
            if line.hasPrefix("p") {
                // PID line: "p1234"
                currentPid = Int32(line.dropFirst()) ?? 0
            } else if line.hasPrefix("n") && filterPids.contains(currentPid) {
                // Name line: "n*:3000" or "n127.0.0.1:8080"
                let name = String(line.dropFirst())
                if let colonIdx = name.lastIndex(of: ":") {
                    let portStr = name[name.index(after: colonIdx)...]
                    if let port = UInt16(portStr) {
                        ports.insert(port)
                    }
                }
            }
        }
        return ports.sorted()
    }

    // MARK: - Configuration (additional)

    func configAll() -> [ConfigEntryDTO] {
        // return bridge.configAll().map { $0.toDTO() }
        return []
    }

    // MARK: - Browser JavaScript evaluation

    func evalJs(panelId: String, script: String, completion: @escaping (Result<String, Error>) -> Void) {
        // In the full integration this routes through the bridge to the
        // browser surface. For now the Swift-side BrowserView handles
        // JS evaluation directly via WKWebView.evaluateJavaScript.
        // This stub exists so the RPC path can be wired up later.
        completion(.failure(NSError(domain: "thane", code: -1, userInfo: [
            NSLocalizedDescriptionKey: "evalJs via bridge not yet wired — use BrowserView directly"
        ])))
    }

    // MARK: - Browser screenshot

    func browserScreenshot(panelId: String) -> String? {
        // Placeholder — screenshot is handled directly by
        // BrowserView.takeScreenshot() on the Swift side.
        return nil
    }

    // MARK: - Session persistence

    private static var sessionFileURL: URL {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let thaneDir = appSupport.appendingPathComponent("thane")
        try? FileManager.default.createDirectory(at: thaneDir, withIntermediateDirectories: true)
        return thaneDir.appendingPathComponent("session.json")
    }

    func saveSession() throws {
        var wsData: [[String: Any]] = []
        for ws in workspaces {
            var entry: [String: Any] = [
                "id": ws.id,
                "title": ws.title,
                "cwd": ws.cwd,
            ]
            if let tag = ws.tag { entry["tag"] = tag }
            if let cost = workspaceCosts[ws.id], cost > 0 {
                entry["alltimeCost"] = cost
            }

            // Save split tree structure
            if let tree = splitTrees[ws.id] {
                entry["splitTree"] = tree.toDict()
                // Also save per-panel CWDs
                let panelCwdMap = tree.allPanels.reduce(into: [String: String]()) { dict, panel in
                    dict[panel.id] = panelCwds[panel.id] ?? panel.location
                }
                entry["panelCwds"] = panelCwdMap

                // Save per-panel scrollback text for terminals
                var scrollbackMap: [String: String] = [:]
                for panel in tree.allPanels where panel.panelType == .terminal {
                    if let text = scrollbackProvider?(panel.id), !text.isEmpty {
                        scrollbackMap[panel.id] = text
                    }
                }
                if !scrollbackMap.isEmpty {
                    entry["panelScrollback"] = scrollbackMap
                }
            }

            // Save sandbox policy if enabled
            if let policy = sandboxPolicies[ws.id] {
                let enfStr: String
                switch policy.enforcement {
                case .permissive: enfStr = "permissive"
                case .enforcing: enfStr = "enforcing"
                case .strict: enfStr = "strict"
                }
                entry["sandbox"] = [
                    "enabled": policy.enabled,
                    "rootDir": policy.rootDir,
                    "enforcement": enfStr,
                    "allowNetwork": policy.allowNetwork,
                    "readOnlyPaths": policy.readOnlyPaths,
                    "readWritePaths": policy.readWritePaths,
                    "deniedPaths": policy.deniedPaths,
                ] as [String: Any]
            }

            wsData.append(entry)
        }

        // Save closed workspace history
        let historyData: [[String: Any]] = closedWorkspaces.map { entry in
            var dict: [String: Any] = [
                "id": entry.id,
                "title": entry.title,
                "cwd": entry.cwd,
                "closedAt": entry.closedAt,
            ]
            if let tag = entry.tag { dict["tag"] = tag }
            return dict
        }

        let session: [String: Any] = [
            "activeWorkspaceId": activeWorkspaceId ?? "",
            "workspaces": wsData,
            "config": configStore,
            "closedWorkspaces": historyData,
        ]

        let data = try JSONSerialization.data(withJSONObject: session, options: .prettyPrinted)
        // Atomic write: write to temp then rename
        let tempURL = Self.sessionFileURL.deletingLastPathComponent().appendingPathComponent("session.tmp")
        try data.write(to: tempURL)
        let fm = FileManager.default
        if fm.fileExists(atPath: Self.sessionFileURL.path) {
            try fm.removeItem(at: Self.sessionFileURL)
        }
        try fm.moveItem(at: tempURL, to: Self.sessionFileURL)
        // Set restrictive permissions (owner-only read/write) — session file may contain
        // sandbox policies, scrollback text with credentials, workspace paths
        try? fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: Self.sessionFileURL.path)
    }

    func restoreSession() throws -> SessionInfoDTO {
        let url = Self.sessionFileURL
        guard FileManager.default.fileExists(atPath: url.path) else {
            return SessionInfoDTO(restored: false, workspaceCount: 0)
        }

        let data = try Data(contentsOf: url)
        guard let session = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let wsArray = session["workspaces"] as? [[String: Any]] else {
            return SessionInfoDTO(restored: false, workspaceCount: 0)
        }

        // Restore config
        if let savedConfig = session["config"] as? [String: String] {
            for (key, value) in savedConfig {
                configStore[key] = value
            }
        }

        // Sandbox state is restored by the Rust bridge via restore_session().

        // Restore closed workspace history
        if let savedHistory = session["closedWorkspaces"] as? [[String: Any]] {
            closedWorkspaces = savedHistory.compactMap { dict in
                guard let id = dict["id"] as? String,
                      let title = dict["title"] as? String,
                      let cwd = dict["cwd"] as? String,
                      let closedAt = dict["closedAt"] as? String else { return nil }
                let tag = dict["tag"] as? String
                return ClosedWorkspaceDTO(id: id, title: title, cwd: cwd, tag: tag, closedAt: closedAt)
            }
        }

        // Restore workspaces
        for wsData in wsArray {
            guard let id = wsData["id"] as? String,
                  let title = wsData["title"] as? String,
                  let cwd = wsData["cwd"] as? String else { continue }

            let tag = wsData["tag"] as? String

            // Restore persisted cost
            if let cost = wsData["alltimeCost"] as? Double, cost > 0 {
                workspaceCosts[id] = cost
            }

            // Restore per-panel CWDs
            let savedPanelCwds = wsData["panelCwds"] as? [String: String] ?? [:]
            for (panelId, panelCwd) in savedPanelCwds {
                panelCwds[panelId] = panelCwd
            }

            // Restore per-panel scrollback text
            if let savedScrollback = wsData["panelScrollback"] as? [String: String] {
                for (panelId, text) in savedScrollback {
                    panelScrollback[panelId] = text
                }
            }

            // Restore sandbox policy
            if let sbx = wsData["sandbox"] as? [String: Any], sbx["enabled"] as? Bool == true {
                let enfStr = sbx["enforcement"] as? String ?? "enforcing"
                let enf: EnforcementLevelDTO
                switch enfStr {
                case "permissive": enf = .permissive
                case "strict": enf = .strict
                default: enf = .enforcing
                }
                // Validate: merge restored denied paths with default set to prevent
                // session file tampering from removing critical credential protections.
                let home = FileManager.default.homeDirectoryForCurrentUser.path
                let defaultDenied: Set<String> = [
                    "\(home)/.ssh", "\(home)/.gnupg", "\(home)/.aws",
                    "\(home)/.docker", "\(home)/.kube",
                    "\(home)/.env", "\(home)/.netrc",
                    "\(home)/.git-credentials", "\(home)/.config/gh",
                    "\(home)/.claude/.credentials.json",
                    "\(home)/Library/Keychains", "\(home)/Library/Cookies",
                ]
                let restoredDenied = Set(sbx["deniedPaths"] as? [String] ?? [])
                let mergedDenied = Array(restoredDenied.union(defaultDenied))

                sandboxPolicies[id] = SandboxInfoDTO(
                    enabled: true,
                    rootDir: sbx["rootDir"] as? String ?? cwd,
                    readOnlyPaths: sbx["readOnlyPaths"] as? [String] ?? [],
                    readWritePaths: sbx["readWritePaths"] as? [String] ?? [],
                    deniedPaths: mergedDenied,
                    allowNetwork: sbx["allowNetwork"] as? Bool ?? true,
                    maxOpenFiles: 1024, maxWriteBytes: 1_073_741_824, maxCpuSeconds: 600,
                    enforcement: enf
                )
            }

            // Restore split tree structure
            var restoredTree: SplitNode?
            if let treeDict = wsData["splitTree"] as? [String: Any] {
                restoredTree = SplitNode.fromDict(treeDict, panelCwds: savedPanelCwds)
            }

            // Fallback: restore from flat panel list (old format)
            if restoredTree == nil {
                if let panels = wsData["panels"] as? [[String: String]] {
                    var panelList: [PanelInfoDTO] = []
                    for panelData in panels {
                        let panelId = panelData["id"] ?? UUID().uuidString
                        let panelCwd = panelData["cwd"] ?? cwd
                        let panel = PanelInfoDTO(
                            id: panelId, panelType: .terminal, title: "Terminal",
                            location: panelCwd, hasUnread: false
                        )
                        panelList.append(panel)
                        panelCwds[panelId] = panelCwd
                    }
                    if !panelList.isEmpty {
                        restoredTree = .leaf(panelList[0])
                        for i in 1..<panelList.count {
                            restoredTree = .split(.horizontal, restoredTree!, .leaf(panelList[i]))
                        }
                    }
                }
            }

            // Final fallback: single terminal
            if restoredTree == nil {
                let panelId = UUID().uuidString
                let panel = PanelInfoDTO(
                    id: panelId, panelType: .terminal, title: "Terminal",
                    location: cwd, hasUnread: false
                )
                panelCwds[panelId] = cwd
                restoredTree = .leaf(panel)
            }

            let panelCount = restoredTree?.allPanels.count ?? 1
            let ws = WorkspaceInfoDTO(
                id: id, title: title, cwd: cwd, tag: tag,
                paneCount: UInt64(panelCount), panelCount: UInt64(panelCount),
                unreadNotifications: 0
            )
            workspaces.append(ws)
            splitTrees[id] = restoredTree
        }

        // Restore active workspace
        let savedActiveId = session["activeWorkspaceId"] as? String ?? ""
        if workspaces.contains(where: { $0.id == savedActiveId }) {
            activeWorkspaceId = savedActiveId
            focusedPanelId = splitTrees[savedActiveId]?.allPanels.first?.id
        } else if let first = workspaces.first {
            activeWorkspaceId = first.id
            focusedPanelId = splitTrees[first.id]?.allPanels.first?.id
        }

        // Log audit events for restored workspaces
        for ws in workspaces {
            logAuditEvent(workspaceId: ws.id, eventType: "WorkspaceRestored", severity: .info,
                          description: "Restored workspace \"\(ws.title)\"",
                          metadata: ["cwd": ws.cwd])
        }

        return SessionInfoDTO(restored: !workspaces.isEmpty, workspaceCount: UInt64(workspaces.count))
    }

    // MARK: - IPC server

    /// The socket path for the IPC server.
    /// Matches the Rust `MacosDirs.socket_path()`: ~/Library/Application Support/thane/run/thane.sock
    private lazy var ipcSocketPath: String = {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return appSupport.appendingPathComponent("thane/run/thane.sock").path
    }()

    private var ipcServer: IpcServer?

    func startIpcServer() throws {
        let dir = (ipcSocketPath as NSString).deletingLastPathComponent
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        setenv("THANE_SOCKET_PATH", ipcSocketPath, 1)

        // Capture self weakly for the handler closure.
        // The handler runs on a background thread; dispatch to main for RustBridge access.
        let server = IpcServer(socketPath: ipcSocketPath) { [weak self] request in
            guard let self else {
                return JsonRpcResponse(
                    jsonrpc: "2.0", result: nil,
                    error: JsonRpcError(code: -32603, message: "Bridge unavailable", data: nil),
                    id: request.id
                )
            }

            var result: AnyCodable?
            var error: JsonRpcError?

            let semaphore = DispatchSemaphore(value: 0)
            DispatchQueue.main.async {
                (result, error) = self.handleRpcRequest(method: request.method, params: request.params)
                semaphore.signal()
            }
            semaphore.wait()

            if let error {
                return JsonRpcResponse(jsonrpc: "2.0", result: nil, error: error, id: request.id)
            }
            return JsonRpcResponse(jsonrpc: "2.0", result: result, error: nil, id: request.id)
        }

        try server.start()
        ipcServer = server
        NSLog("thane: IPC server started on \(ipcSocketPath)")
    }

    func stopIpcServer() {
        ipcServer?.stop()
        ipcServer = nil
    }

    func socketPath() -> String {
        ipcSocketPath
    }

    // MARK: - RPC dispatch

    /// Handle an incoming JSON-RPC method call. Returns (result, error).
    /// Called on the main thread.
    private func handleRpcRequest(method: String, params: AnyCodable?) -> (AnyCodable?, JsonRpcError?) {
        let obj = params?.objectValue ?? [:]

        switch method {
        case "ping":
            return (.object(["status": .string("ok")]), nil)

        case "get_version":
            let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1.0"
            return (.object(["version": .string(version)]), nil)

        // ── Agent queue ──

        case "agent_queue.submit":
            guard let content = obj["content"]?.stringValue, !content.trimmingCharacters(in: .whitespaces).isEmpty else {
                return (nil, JsonRpcError(code: -32602, message: "Missing or empty 'content' parameter", data: nil))
            }
            let wsId = obj["workspace_id"]?.stringValue
            let priority = obj["priority"]?.intValue ?? 0
            let id = queueSubmit(content: content, workspaceId: wsId, priority: Int32(priority))
            return (.object(["id": .string(id), "status": .string("queued")]), nil)

        case "agent_queue.list":
            let entries = queueList()
            let arr: [AnyCodable] = entries.map { entry in
                .object([
                    "id": .string(entry.id),
                    "status": .string(Self.statusString(entry.status)),
                    "content_preview": .string(String(entry.content.prefix(100))),
                ])
            }
            return (.object(["entries": .array(arr)]), nil)

        case "agent_queue.status":
            guard let entryId = obj["id"]?.stringValue else {
                return (nil, JsonRpcError(code: -32602, message: "Missing 'id' parameter", data: nil))
            }
            if let entry = queueStatus(entryId: entryId) {
                return (.object([
                    "id": .string(entry.id),
                    "status": .string(Self.statusString(entry.status)),
                    "content_preview": .string(String(entry.content.prefix(100))),
                ]), nil)
            }
            return (nil, JsonRpcError(code: -32602, message: "Entry not found", data: nil))

        case "agent_queue.cancel":
            guard let entryId = obj["id"]?.stringValue else {
                return (nil, JsonRpcError(code: -32602, message: "Missing 'id' parameter", data: nil))
            }
            let ok = queueCancel(entryId: entryId)
            return (.object(["cancelled": .bool(ok)]), nil)

        // ── Workspace ──

        case "workspace.list":
            let wsList = listWorkspaces()
            let arr: [AnyCodable] = wsList.map { ws in
                .object([
                    "id": .string(ws.id),
                    "title": .string(ws.title),
                    "cwd": .string(ws.cwd),
                ])
            }
            return (.object(["workspaces": .array(arr)]), nil)

        // ── Sandbox ──

        case "sandbox.status":
            let wsId = obj["id"]?.stringValue ?? activeWorkspaceId ?? ""
            if let info = sandboxStatus(workspaceId: wsId) {
                return (.object([
                    "enabled": .bool(info.enabled),
                    "root_dir": .string(info.rootDir),
                    "enforcement": .string(Self.enforcementString(info.enforcement)),
                    "allow_network": .bool(info.allowNetwork),
                ]), nil)
            }
            return (.object(["enabled": .bool(false)]), nil)

        case "sandbox.enable":
            let wsId = obj["id"]?.stringValue ?? activeWorkspaceId ?? ""
            do {
                try sandboxEnable(workspaceId: wsId)
                return (.object(["ok": .bool(true)]), nil)
            } catch {
                return (nil, JsonRpcError(code: -32603, message: error.localizedDescription, data: nil))
            }

        case "sandbox.disable":
            let wsId = obj["id"]?.stringValue ?? activeWorkspaceId ?? ""
            sandboxDisable(workspaceId: wsId)
            return (.object(["ok": .bool(true)]), nil)

        default:
            return (nil, JsonRpcError(code: -32601, message: "Method not found: \(method)", data: nil))
        }
    }

    private static func statusString(_ status: QueueEntryStatusDTO) -> String {
        switch status {
        case .queued: return "queued"
        case .running: return "running"
        case .pausedTokenLimit: return "paused_token_limit"
        case .pausedByUser: return "paused_by_user"
        case .completed: return "completed"
        case .failed: return "failed"
        case .cancelled: return "cancelled"
        }
    }

    private static func enforcementString(_ level: EnforcementLevelDTO) -> String {
        switch level {
        case .permissive: return "permissive"
        case .enforcing: return "enforcing"
        case .strict: return "strict"
        }
    }
}

// MARK: - UniFFI callback proxy
//
// This class implements the UniFFI-generated UiCallbackProtocol and dispatches
// callbacks to the main actor via the RustBridgeDelegate.
//
// Uncomment once ThaneBridge.swift bindings are generated:
//
// final class BridgeCallbackProxy: UiCallbackProtocol {
//     private weak var bridge: RustBridge?
//
//     init(bridge: RustBridge) {
//         self.bridge = bridge
//     }
//
//     func workspaceChanged(activeId: String) {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.workspaceChanged(activeId: activeId)
//         }
//     }
//
//     func workspaceListChanged() {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.workspaceListChanged()
//         }
//     }
//
//     func notificationReceived(workspaceId: String, title: String, body: String) {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.notificationReceived(
//                 workspaceId: workspaceId, title: title, body: body
//             )
//         }
//     }
//
//     func agentStatusChanged(workspaceId: String, active: Bool) {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.agentStatusChanged(
//                 workspaceId: workspaceId, active: active
//             )
//         }
//     }
//
//     func queueEntryCompleted(entryId: String, success: Bool) {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.queueEntryCompleted(
//                 entryId: entryId, success: success
//             )
//         }
//     }
//
//     func paneLayoutChanged(workspaceId: String) {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.paneLayoutChanged(workspaceId: workspaceId)
//         }
//     }
//
//     func configChanged() {
//         DispatchQueue.main.async { [weak self] in
//             self?.bridge?.delegate?.configChanged()
//         }
//     }
// }

// MARK: - DTO types, enums, and SplitNode
// Extracted to DTOTypes.swift

// MARK: - CostScanner (JSONL session file parser)

/// Scans Claude Code JSONL session files to calculate token usage and cost.
/// Mirrors the Rust `cost_tracker.rs` logic for the macOS pure-Swift path.
enum CostScanner {

    // MARK: - Pricing

    private struct ModelPricing {
        let inputPerMillion: Double
        let outputPerMillion: Double
        let cacheReadPerMillion: Double
        let cacheWritePerMillion: Double
    }

    private static let opusPricing = ModelPricing(
        inputPerMillion: 15.0, outputPerMillion: 75.0,
        cacheReadPerMillion: 1.5, cacheWritePerMillion: 18.75
    )
    private static let sonnetPricing = ModelPricing(
        inputPerMillion: 3.0, outputPerMillion: 15.0,
        cacheReadPerMillion: 0.3, cacheWritePerMillion: 3.75
    )

    private static func pricing(for model: String) -> ModelPricing {
        model.lowercased().contains("opus") ? opusPricing : sonnetPricing
    }

    // MARK: - Parsed entry

    private struct UsageEntry {
        let model: String
        let inputTokens: UInt64
        let outputTokens: UInt64
        let cacheReadTokens: UInt64
        let cacheWriteTokens: UInt64
        let timestamp: Date?
    }

    // MARK: - Public API

    /// Calculate project cost for a given CWD, optionally filtering session entries by `since`.
    static func projectCost(cwd: String, since: Date?) -> ProjectCostDTO {
        let dirs = projectDirs(for: cwd)
        var sessionInput: UInt64 = 0, sessionOutput: UInt64 = 0
        var sessionCacheRead: UInt64 = 0, sessionCacheWrite: UInt64 = 0
        var sessionCost: Double = 0
        var alltimeInput: UInt64 = 0, alltimeOutput: UInt64 = 0
        var alltimeCacheRead: UInt64 = 0, alltimeCacheWrite: UInt64 = 0
        var alltimeCost: Double = 0
        var sessionCount: UInt64 = 0

        let fm = FileManager.default
        for dir in dirs {
            guard let files = try? fm.contentsOfDirectory(atPath: dir) else { continue }
            let jsonlFiles = files.filter { $0.hasSuffix(".jsonl") }
            sessionCount += UInt64(jsonlFiles.count)

            for file in jsonlFiles {
                let path = (dir as NSString).appendingPathComponent(file)
                let entries = parseJsonlFile(path)

                // File mtime for fallback timestamp check
                let fileMtime = (try? fm.attributesOfItem(atPath: path))?[.modificationDate] as? Date

                for entry in entries {
                    let p = pricing(for: entry.model)
                    let cost = Double(entry.inputTokens) * p.inputPerMillion / 1_000_000
                        + Double(entry.outputTokens) * p.outputPerMillion / 1_000_000
                        + Double(entry.cacheReadTokens) * p.cacheReadPerMillion / 1_000_000
                        + Double(entry.cacheWriteTokens) * p.cacheWritePerMillion / 1_000_000

                    // All-time always accumulates
                    alltimeInput += entry.inputTokens
                    alltimeOutput += entry.outputTokens
                    alltimeCacheRead += entry.cacheReadTokens
                    alltimeCacheWrite += entry.cacheWriteTokens
                    alltimeCost += cost

                    // Session: filter by since
                    let entryDate = entry.timestamp ?? fileMtime
                    if let since, let d = entryDate, d < since { continue }
                    sessionInput += entry.inputTokens
                    sessionOutput += entry.outputTokens
                    sessionCacheRead += entry.cacheReadTokens
                    sessionCacheWrite += entry.cacheWriteTokens
                    sessionCost += cost
                }
            }
        }

        return ProjectCostDTO(
            sessionCostUsd: sessionCost,
            sessionInputTokens: sessionInput,
            sessionOutputTokens: sessionOutput,
            sessionCacheReadTokens: sessionCacheRead,
            sessionCacheWriteTokens: sessionCacheWrite,
            alltimeCostUsd: alltimeCost,
            alltimeInputTokens: alltimeInput,
            alltimeOutputTokens: alltimeOutput,
            alltimeCacheReadTokens: alltimeCacheRead,
            alltimeCacheWriteTokens: alltimeCacheWrite,
            sessionCount: sessionCount,
            planName: "Pro",
            displayMode: "dollar",
            fiveHourUtilization: nil,
            sevenDayUtilization: nil
        )
    }

    // MARK: - Project directory discovery

    /// Find ~/.claude/projects/<mangled>/ directory for this exact CWD.
    /// Does NOT walk ancestors — `/Users/foo/Documents` should not pick up
    /// costs from `/Users/foo` or `/Users/foo/repo/project`.
    private static func projectDirs(for cwd: String) -> [String] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let baseDir = (home as NSString).appendingPathComponent(".claude/projects")
        let mangled = cwd.replacingOccurrences(of: "/", with: "-")
        let dir = (baseDir as NSString).appendingPathComponent(mangled)
        if FileManager.default.fileExists(atPath: dir) {
            return [dir]
        }
        return []
    }

    // MARK: - JSONL parsing

    private static let iso8601Formatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let iso8601FallbackFormatter = ISO8601DateFormatter()

    private static func parseDate(_ str: String) -> Date? {
        iso8601Formatter.date(from: str) ?? iso8601FallbackFormatter.date(from: str)
    }

    private static func parseJsonlFile(_ path: String) -> [UsageEntry] {
        guard let data = FileManager.default.contents(atPath: path),
              let content = String(data: data, encoding: .utf8) else { return [] }

        var entries: [UsageEntry] = []
        for line in content.split(separator: "\n", omittingEmptySubsequences: true) {
            guard let lineData = line.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any]
            else { continue }

            if let entry = parseJournalFormat(json) ?? parseFlatFormat(json) {
                entries.append(entry)
            }
        }
        return entries
    }

    /// Parse nested journal format: { "type": "assistant", "message": { "model": ..., "usage": { ... } } }
    private static func parseJournalFormat(_ json: [String: Any]) -> UsageEntry? {
        guard let type = json["type"] as? String, type == "assistant",
              let message = json["message"] as? [String: Any],
              let model = message["model"] as? String,
              let usage = message["usage"] as? [String: Any]
        else { return nil }

        let input = (usage["input_tokens"] as? NSNumber)?.uint64Value ?? 0
        let output = (usage["output_tokens"] as? NSNumber)?.uint64Value ?? 0
        let cacheRead = (usage["cache_read_input_tokens"] as? NSNumber)?.uint64Value ?? 0
        let cacheWrite = (usage["cache_creation_input_tokens"] as? NSNumber)?.uint64Value ?? 0

        guard input + output + cacheRead + cacheWrite > 0 else { return nil }

        let ts = (json["timestamp"] as? String).flatMap(parseDate)
        return UsageEntry(model: model, inputTokens: input, outputTokens: output,
                          cacheReadTokens: cacheRead, cacheWriteTokens: cacheWrite,
                          timestamp: ts)
    }

    /// Parse flat camelCase format: { "model": ..., "inputTokens": ..., ... }
    private static func parseFlatFormat(_ json: [String: Any]) -> UsageEntry? {
        guard let model = json["model"] as? String else { return nil }

        let input = (json["inputTokens"] as? NSNumber)?.uint64Value ?? 0
        let output = (json["outputTokens"] as? NSNumber)?.uint64Value ?? 0
        let cacheRead = (json["cacheReadInputTokens"] as? NSNumber)?.uint64Value ?? 0
        let cacheWrite = (json["cacheCreationInputTokens"] as? NSNumber)?.uint64Value ?? 0

        guard input + output + cacheRead + cacheWrite > 0 else { return nil }

        let ts = (json["timestamp"] as? String).flatMap(parseDate)
        return UsageEntry(model: model, inputTokens: input, outputTokens: output,
                          cacheReadTokens: cacheRead, cacheWriteTokens: cacheWrite,
                          timestamp: ts)
    }
}

// MARK: - AuditScanner (terminal output scanning for security events)

/// Detects sensitive file access, PII, and agent invocations in terminal output.
/// Mirrors the Rust `audit.rs` logic.
enum AuditScanner {

    // MARK: - Sensitive file patterns

    /// File name/path patterns that indicate sensitive data.
    private static let sensitivePatterns: [String] = [
        ".env", ".env.local", ".env.production", ".env.staging",
        ".ssh/id_", ".ssh/config", ".ssh/known_hosts", ".ssh/authorized_keys",
        ".gnupg/", ".gpg",
        "credentials.json", "credentials.yml", "credentials.yaml",
        "secrets.json", "secrets.yml", "secrets.yaml",
        ".aws/credentials", ".aws/config",
        ".gcloud/", "application_default_credentials.json",
        ".npmrc", ".pypirc", ".docker/config.json",
        "keystore", ".keychain",
        "token", "api_key", "apikey", "secret_key", "secretkey",
        ".htpasswd", "shadow", "passwd",
        "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa",
    ]

    /// Extensions that indicate private key files.
    private static let privateKeyExtensions: Set<String> = [
        ".pem", ".key", ".p12", ".pfx",
    ]

    /// PII keywords to scan for.
    private static let piiKeywords: [String] = [
        "social security", "ssn", "passport", "credit card",
        "bank account", "routing number", "tax id", "driver license",
    ]

    // MARK: - Public API

    struct ScanResult {
        var sensitiveFiles: [(path: String, eventType: String, severity: AuditSeverityDTO)] = []
        var piiFindings: [String] = []
        var agentInvocations: [String] = []
    }

    /// Strip ANSI escape sequences and control characters from terminal output.
    static func stripTerminalCodes(_ text: String) -> String {
        // Remove CSI sequences (ESC [ ... final), OSC sequences (ESC ] ... BEL/ST),
        // simple ESC+letter, and lone control chars (keep \n, \t, \r).
        let pattern = try! NSRegularExpression(
            pattern: "\\x1b\\[[0-9;]*[A-Za-z]|\\x1b\\][^\\x07]*(?:\\x07|\\x1b\\\\)|\\x1b[A-Za-z0-9]|[\\x00-\\x08\\x0b\\x0c\\x0e-\\x1f\\x7f]",
            options: []
        )
        let range = NSRange(text.startIndex..., in: text)
        return pattern.stringByReplacingMatches(in: text, range: range, withTemplate: "")
    }

    /// Scan a chunk of terminal output for security-relevant events.
    static func scan(text rawText: String) -> ScanResult {
        var result = ScanResult()
        let text = stripTerminalCodes(rawText)

        // Extract file paths and check for sensitive files
        let paths = extractFilePaths(text)
        for path in paths {
            if let finding = checkSensitiveFile(path) {
                result.sensitiveFiles.append(finding)
            }
        }

        // Also check for sensitive filenames mentioned without a full path
        // (e.g., "cat .env.local", "vi credentials.json")
        for pattern in sensitivePatterns {
            if text.lowercased().contains(pattern) {
                // Avoid duplicates if already found via path extraction
                if !result.sensitiveFiles.contains(where: { $0.path.lowercased().contains(pattern) }) {
                    let isKey = privateKeyExtensions.contains(where: { pattern.hasSuffix($0) })
                        || pattern.contains(".ssh/id_")
                    result.sensitiveFiles.append((
                        path: pattern,
                        eventType: isKey ? "PrivateKeyAccess" : "SecretAccess",
                        severity: isKey ? .critical : .alert
                    ))
                }
            }
        }

        // Check for PII
        result.piiFindings = detectPii(text: text)

        // Check for agent invocations (claude command)
        if let invocation = detectAgentInvocation(text) {
            result.agentInvocations.append(invocation)
        }

        return result
    }

    // MARK: - File path extraction

    /// Extract absolute and home-relative file paths from text.
    static func extractFilePaths(_ text: String) -> [String] {
        var paths: [String] = []
        // Match absolute paths (/...) and home-relative paths (~/...)
        let pattern = try! NSRegularExpression(pattern: "(?:/[\\w.\\-]+)+|~/[\\w.\\-/]+", options: [])
        let range = NSRange(text.startIndex..., in: text)
        for match in pattern.matches(in: text, range: range) {
            if let r = Range(match.range, in: text) {
                let path = String(text[r])
                if path.count >= 3 { paths.append(path) }
            }
        }
        return paths
    }

    /// Check if a file path matches sensitive file patterns.
    private static func checkSensitiveFile(_ path: String) -> (path: String, eventType: String, severity: AuditSeverityDTO)? {
        let lower = path.lowercased()

        // Check private key extensions first (Critical severity)
        let ext = (lower as NSString).pathExtension
        if !ext.isEmpty && privateKeyExtensions.contains(".\(ext)") {
            return (path, "PrivateKeyAccess", .critical)
        }
        // SSH private keys
        if lower.contains(".ssh/id_") && !lower.hasSuffix(".pub") {
            return (path, "PrivateKeyAccess", .critical)
        }

        // Check other sensitive patterns (Alert severity)
        for pattern in sensitivePatterns {
            if lower.contains(pattern) {
                return (path, "SecretAccess", .alert)
            }
        }
        return nil
    }

    // MARK: - Secret redaction

    /// Redact potential secrets (API keys, tokens, passwords) from text before audit logging.
    /// Returns the text with secret-like patterns replaced with [REDACTED].
    static func redactSecrets(_ text: String) -> String {
        var result = text

        // Common API key patterns: long alphanumeric strings after "key", "token", "secret", "password"
        let patterns: [(String, String)] = [
            // Generic key=value with long values (sk-..., pk_..., etc.)
            ("((?:sk|pk|api|key|token|secret|password|auth|bearer)[-_]?[a-zA-Z0-9]{20,})", "[REDACTED_KEY]"),
            // AWS access key pattern (AKIA...)
            ("(AKIA[A-Z0-9]{16})", "[REDACTED_AWS_KEY]"),
            // Generic long base64-like secrets after = or :
            ("(?:(?:key|token|secret|password|credential|auth)\\s*[:=]\\s*)[\"']?([A-Za-z0-9+/=_\\-]{32,})", "[REDACTED_SECRET]"),
        ]
        for (pattern, replacement) in patterns {
            if let regex = try? NSRegularExpression(pattern: pattern, options: .caseInsensitive) {
                let range = NSRange(result.startIndex..., in: result)
                result = regex.stringByReplacingMatches(in: result, range: range, withTemplate: replacement)
            }
        }
        return result
    }

    // MARK: - PII detection

    /// Detect PII patterns in text (keywords, emails, SSNs). Public for use by prompt scanner.
    static func detectPii(text: String) -> [String] {
        var findings: [String] = []
        let lower = text.lowercased()

        // Keyword matching
        for keyword in piiKeywords {
            if lower.contains(keyword) {
                findings.append("PII keyword: \(keyword)")
            }
        }

        // SSN pattern: XXX-XX-XXXX
        let ssnPattern = try! NSRegularExpression(pattern: "\\b\\d{3}-\\d{2}-\\d{4}\\b")
        let range = NSRange(text.startIndex..., in: text)
        if ssnPattern.firstMatch(in: text, range: range) != nil {
            findings.append("SSN-like pattern detected")
        }

        // Email pattern
        let emailPattern = try! NSRegularExpression(pattern: "\\b[\\w.+-]+@[\\w.-]+\\.[a-zA-Z]{2,}\\b")
        let emailCount = emailPattern.numberOfMatches(in: text, range: range)
        if emailCount > 0 {
            findings.append("\(emailCount) email address(es)")
        }

        return findings
    }

    // MARK: - Agent invocation detection

    /// Known CLI agent binary names for prompt capture.
    private static let agentCommandNames = [
        "claude-code", "claude", "codex", "gemini", "goose", "opencode",
        "cline", "amp", "auggie", "openhands", "plandex", "qwen",
        "devin", "tabnine", "cursor", "aider", "copilot", "cody", "continue"
    ]

    /// Detect AI coding agent command invocations in terminal output.
    /// Returns a description string like "codex: Refactor auth module" if found.
    private static func detectAgentInvocation(_ text: String) -> String? {
        // Build alternation pattern from all known agent names
        let alternation = agentCommandNames.joined(separator: "|")
        let pattern = try! NSRegularExpression(
            pattern: "(?:^|[\\$>%#]\\s*)(?:\(alternation))\\b",
            options: .anchorsMatchLines
        )
        let range = NSRange(text.startIndex..., in: text)
        if pattern.firstMatch(in: text, range: range) != nil {
            return text.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return nil
    }
}

// MARK: - PromptScanner (Claude Code session JSONL prompt extraction)

/// Scans Claude Code JSONL session files for user prompts and assistant tool-use actions.
/// Mirrors the Rust `prompt_scanner.rs` logic.
enum PromptScanner {

    struct PromptRecord {
        let uuid: String
        let timestamp: String
        let sessionId: String
        let text: String
        let role: String // "user" or "assistant"
    }

    /// Scan project JSONL files for conversation records.
    /// Checks the exact mangled CWD directory plus any child project directories
    /// (e.g., workspace CWD `/Users/ernielail/repo` also matches project dir
    /// `-Users-ernielail-repo-thane`).
    static func scanPrompts(cwd: String) -> [PromptRecord] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let projectsDir = (home as NSString).appendingPathComponent(".claude/projects")
        let mangled = cwd.replacingOccurrences(of: "/", with: "-")

        let fm = FileManager.default
        guard let allDirs = try? fm.contentsOfDirectory(atPath: projectsDir) else { return [] }

        var records: [PromptRecord] = []
        for dir in allDirs {
            // Match exact CWD or any subdirectory project
            guard dir == mangled || dir.hasPrefix(mangled + "-") else { continue }
            let projectDir = (projectsDir as NSString).appendingPathComponent(dir)
            var isDir: ObjCBool = false
            guard fm.fileExists(atPath: projectDir, isDirectory: &isDir), isDir.boolValue else { continue }

            guard let files = try? fm.contentsOfDirectory(atPath: projectDir) else { continue }
            for file in files where file.hasSuffix(".jsonl") {
                let path = (projectDir as NSString).appendingPathComponent(file)
                parsePromptsFromFile(path, into: &records)
            }
        }
        return records
    }

    private static func parsePromptsFromFile(_ path: String, into records: inout [PromptRecord]) {
        guard let data = FileManager.default.contents(atPath: path),
              let content = String(data: data, encoding: .utf8) else { return }

        for line in content.split(separator: "\n", omittingEmptySubsequences: true) {
            guard let lineData = line.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any]
            else { continue }

            let recordType = json["type"] as? String ?? ""
            let uuid = json["uuid"] as? String ?? ""
            let timestamp = json["timestamp"] as? String ?? ""
            let sessionId = json["sessionId"] as? String ?? ""

            guard let message = json["message"] as? [String: Any] else { continue }
            let role = message["role"] as? String ?? ""

            if recordType == "user" && role == "user" {
                if let text = extractMessageText(message), isHumanPrompt(text) {
                    records.append(PromptRecord(
                        uuid: uuid, timestamp: timestamp,
                        sessionId: sessionId, text: text, role: "user"))
                }
            } else if recordType == "assistant" && role == "assistant" {
                // Extract tool use actions and text responses from assistant messages
                if let content = message["content"] as? [[String: Any]] {
                    // Collect text blocks into a single response
                    var textParts: [String] = []
                    for block in content {
                        let blockType = block["type"] as? String ?? ""
                        if blockType == "tool_use" {
                            let toolId = block["id"] as? String ?? uuid
                            let toolName = block["name"] as? String ?? "unknown"
                            let input = block["input"] as? [String: Any] ?? [:]
                            let description = describeToolUse(toolName, input: input)
                            records.append(PromptRecord(
                                uuid: toolId, timestamp: timestamp,
                                sessionId: sessionId, text: description, role: "tool_use"))
                        } else if blockType == "text", let text = block["text"] as? String, !text.isEmpty {
                            textParts.append(text)
                        }
                    }
                    // Log combined text response if any
                    if !textParts.isEmpty {
                        let fullText = textParts.joined(separator: "\n")
                        records.append(PromptRecord(
                            uuid: uuid, timestamp: timestamp,
                            sessionId: sessionId, text: fullText, role: "response"))
                    }
                }
            }
        }
    }

    /// Extract plain text from a message's content field (string or blocks array).
    private static func extractMessageText(_ message: [String: Any]) -> String? {
        if let text = message["content"] as? String, !text.isEmpty {
            return text
        }
        if let blocks = message["content"] as? [[String: Any]] {
            let texts = blocks.compactMap { block -> String? in
                guard (block["type"] as? String) == "text" else { return nil }
                return block["text"] as? String
            }
            let joined = texts.joined(separator: "\n")
            return joined.isEmpty ? nil : joined
        }
        return nil
    }

    /// Filter out system-injected content (XML tags, etc.).
    private static func isHumanPrompt(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return false }
        if trimmed.hasPrefix("<") { return false }
        return true
    }

    /// Create a human-readable description of a tool use action.
    private static func describeToolUse(_ name: String, input: [String: Any]) -> String {
        switch name {
        case "Edit":
            let file = input["file_path"] as? String ?? "unknown"
            return "Edit: \(file)"
        case "Write":
            let file = input["file_path"] as? String ?? "unknown"
            return "Write: \(file)"
        case "Read":
            let file = input["file_path"] as? String ?? "unknown"
            return "Read: \(file)"
        case "Bash":
            let cmd = input["command"] as? String ?? ""
            let short = String(cmd.prefix(80))
            return "Bash: \(short)"
        case "Grep":
            let pattern = input["pattern"] as? String ?? ""
            return "Grep: \(pattern)"
        case "Glob":
            let pattern = input["pattern"] as? String ?? ""
            return "Glob: \(pattern)"
        case "Agent":
            let desc = input["description"] as? String ?? ""
            return "Agent: \(desc)"
        default:
            return "Tool: \(name)"
        }
    }
}
