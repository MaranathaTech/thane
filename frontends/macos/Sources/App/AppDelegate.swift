import AppKit

@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }

    private var bridge: RustBridge!
    private var mainWindowController: MainWindowController!
    private var appearanceObserver: NSObjectProtocol?
    private var autoSaveTimer: Timer?
    private var queuePollTimer: Timer?
    private var configReloadTimer: Timer?
    private var auditFlushTimer: Timer?
    private var portScanTimer: Timer?
    private var updateCheckTimer: Timer?
    private var lastConfigMtime: Date?
    private var lockFileDescriptor: Int32 = -1

    /// The currently running queue task process, if any.
    var runningQueueProcess: Process?
    /// The entry ID of the currently running queue task.
    var runningQueueEntryId: String?
    /// Stdout pipe for the running queue process (kept alive so we can read after termination).
    var runningQueueStdoutPipe: Pipe?
    /// Stderr pipe for the running queue process.
    var runningQueueStderrPipe: Pipe?
    /// Flag indicating batch processing mode (process all queued entries sequentially).
    var processingAllQueued = false

    // MARK: - NSApplicationDelegate

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSLog("thane: applicationDidFinishLaunching START")

        // Single-instance guard using a lock file
        if !acquireSingleInstanceLock() {
            NSLog("thane: another instance already running, activating it")
            // Try to activate the other instance
            let myPid = ProcessInfo.processInfo.processIdentifier
            let bundleMatches = NSRunningApplication.runningApplications(withBundleIdentifier: "com.thane.app")
            let existing = bundleMatches.first(where: { $0.processIdentifier != myPid && !$0.isTerminated })
                ?? NSWorkspace.shared.runningApplications.first(where: {
                    $0.processIdentifier != myPid && !$0.isTerminated
                    && ($0.localizedName == "thane-macos" || $0.localizedName == "thane")
                })
            existing?.activate(options: [.activateAllWindows, .activateIgnoringOtherApps])
            DispatchQueue.main.async { NSApp.terminate(nil) }
            return
        }

        // Register bundled fonts before any UI is created
        ThaneTheme.registerBundledFonts()
        NSLog("thane: fonts registered")

        // Set app icon — generate programmatically (same as bundled icon)
        NSApp.applicationIconImage = generateAppIcon()
        NSLog("thane: app icon set")

        // Initialize the Rust bridge
        do {
            bridge = try RustBridge()
            bridge.onSecurityAlert = { severity, title, message in
                let alert = NSAlert()
                alert.messageText = title
                alert.informativeText = message
                alert.alertStyle = severity == .critical ? .critical : .warning
                alert.addButton(withTitle: "View Audit Log")
                alert.addButton(withTitle: "Dismiss")
                if alert.runModal() == .alertFirstButtonReturn {
                    self.mainWindowController?.showRightPanel(.audit)
                }
            }
            NSLog("thane: bridge initialized")
        } catch {
            NSLog("thane: bridge init FAILED: \(error)")
            let alert = NSAlert()
            alert.messageText = "Failed to initialize thane"
            alert.informativeText = error.localizedDescription
            alert.alertStyle = .critical
            alert.runModal()
            NSApp.terminate(nil)
            return
        }

        // Build the main menu
        NSApp.mainMenu = MainMenu.build(target: self)
        NSLog("thane: menu built")

        // Activate the application (ensures window appears in foreground)
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        NSLog("thane: activated")

        // Create and show the main window
        mainWindowController = MainWindowController(bridge: bridge)
        NSLog("thane: window controller created, window=\(String(describing: mainWindowController.window))")
        mainWindowController.showWindow(nil)
        mainWindowController.window?.makeKeyAndOrderFront(nil)
        NSLog("thane: window shown, isVisible=\(mainWindowController.window?.isVisible ?? false), frame=\(String(describing: mainWindowController.window?.frame))")

        // Observe system appearance changes (dark/light mode toggle)
        appearanceObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.handleAppearanceChange()
        }

        // Also observe effective appearance via KVO on NSApp
        NSApp.addObserver(self, forKeyPath: "effectiveAppearance", options: [.new], context: nil)

        // Restore previous session
        restoreSession()

        // Start the IPC server for CLI access
        do {
            try bridge.startIpcServer()
        } catch {
            NSLog("thane: failed to start IPC server: \(error)")
        }

        // Run startup checks (dependency verification, CLAUDE.md injection)
        runStartupChecks()

        // Check for app updates
        checkForUpdates()

        // Ask user for keychain consent at launch so token limits can be fetched.
        // If declined, they'll be re-prompted when opening the token panel.
        bridge.requestKeychainConsent()

        // Auto-save session every 8 seconds (matching Linux)
        autoSaveTimer = Timer.scheduledTimer(withTimeInterval: 8.0, repeats: true) { [weak self] _ in
            try? self?.bridge.saveSession()
        }

        // Poll running queue tasks every 2 seconds (matching Linux)
        queuePollTimer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak self] _ in
            self?.pollQueueProcess()
        }

        // Check config file for changes every 5 seconds (hot-reload)
        configReloadTimer = Timer.scheduledTimer(withTimeInterval: 5.0, repeats: true) { [weak self] _ in
            self?.checkConfigFileChanged()
        }

        // Sidebar + status bar metadata refresh + port scanning every 5 seconds.
        // Uses refreshMetadata() instead of reloadFromBridge() to avoid
        // rebuilding the split tree, which steals terminal focus.
        portScanTimer = Timer.scheduledTimer(withTimeInterval: 10.0, repeats: true) { [weak self] _ in
            self?.mainWindowController.refreshMetadata()
            self?.mainWindowController.scanPorts()
        }

        // Audit persistence flush every 10 seconds + daily purge of stale logs
        auditFlushTimer = Timer.scheduledTimer(withTimeInterval: 10.0, repeats: true) { [weak self] _ in
            self?.bridge.flushAuditLog()
            self?.bridge.purgeStaleAuditLogs()
        }

        // Periodic update check every 4 hours
        updateCheckTimer = Timer.scheduledTimer(withTimeInterval: 4 * 60 * 60, repeats: true) { [weak self] _ in
            self?.checkForUpdates()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        autoSaveTimer?.invalidate()
        autoSaveTimer = nil
        queuePollTimer?.invalidate()
        queuePollTimer = nil
        configReloadTimer?.invalidate()
        configReloadTimer = nil
        auditFlushTimer?.invalidate()
        auditFlushTimer = nil
        portScanTimer?.invalidate()
        portScanTimer = nil

        // Guard: bridge may be nil if we terminated early (single-instance guard)
        guard bridge != nil else { return }

        // Save session before quitting
        do {
            try bridge.saveSession()
        } catch {
            NSLog("thane: failed to save session: \(error)")
        }

        bridge.stopIpcServer()

        releaseSingleInstanceLock()

        if let observer = appearanceObserver {
            NotificationCenter.default.removeObserver(observer)
        }
        NSApp.removeObserver(self, forKeyPath: "effectiveAppearance")
    }

    // MARK: - Appearance

    override func observeValue(forKeyPath keyPath: String?, of object: Any?,
                                change: [NSKeyValueChangeKey: Any]?, context: UnsafeMutableRawPointer?) {
        if keyPath == "effectiveAppearance" {
            handleAppearanceChange()
        } else {
            super.observeValue(forKeyPath: keyPath, of: object, change: change, context: context)
        }
    }

    private func handleAppearanceChange() {
        // Force window background and views to redraw with updated theme colors
        if let window = mainWindowController?.window {
            window.backgroundColor = ThaneTheme.backgroundColor
            window.invalidateShadow()
            window.contentView?.needsDisplay = true
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return true
    }

    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
        return true
    }

    // MARK: - Session

    private func restoreSession() {
        do {
            let info = try bridge.restoreSession()
            if info.restored {
                NSLog("thane: restored \(info.workspaceCount) workspace(s)")
            }
        } catch {
            NSLog("thane: session restore failed: \(error)")
        }

        // If no workspaces exist, create a default one
        if bridge.listWorkspaces().isEmpty {
            let homeDir = FileManager.default.homeDirectoryForCurrentUser.path
            _ = try? bridge.createWorkspace(title: "default", cwd: homeDir)
        }

        mainWindowController.reloadFromBridge()

        // Seed cost cache so sidebar shows costs immediately (not after first timer tick)
        bridge.refreshCostCacheAsync()

        // Apply saved font/cursor/config to terminals after restore
        mainWindowController.applyConfig()
    }

    // MARK: - Queue polling

    private func pollQueueProcess() {
        // Check if a running process has finished
        if let process = runningQueueProcess, let entryId = runningQueueEntryId {
            guard !process.isRunning else { return }

            let success = process.terminationStatus == 0
            let exitCode = process.terminationStatus

            NSLog("thane: queue process for \(entryId) finished, status=\(exitCode), success=\(success)")

            // Update entry status immediately (doesn't depend on output)
            bridge.queueUpdateStatus(entryId: entryId, status: success ? .completed : .failed,
                                     error: success ? nil : "Exit code \(exitCode)")

            // Audit log
            if success {
                bridge.logAuditEvent(workspaceId: "", eventType: "QueueTaskCompleted", severity: .info,
                                     description: "Queue task completed", metadata: ["entry_id": entryId])
            } else {
                bridge.logAuditEvent(workspaceId: "", eventType: "QueueTaskFailed", severity: .warning,
                                     description: "Queue task failed (exit \(exitCode))",
                                     metadata: ["entry_id": entryId, "exit_code": "\(exitCode)"])
            }

            // Post a notification for the completed queue task
            let notifTitle = success ? "Queue task completed" : "Queue task failed"
            let notifBody = success
                ? "Task \(entryId.prefix(8)) finished successfully."
                : "Task \(entryId.prefix(8)) failed (exit \(exitCode))."
            bridge.postNotification(
                workspaceId: "",
                title: notifTitle,
                body: notifBody,
                urgency: success ? .normal : .critical
            )

            // Notify the bridge delegate (updates UI badges)
            bridge.delegate?.queueEntryCompleted(entryId: entryId, success: success)

            // Parse model and scan output log asynchronously — the output file is written by
            // a background DispatchQueue that may not have flushed yet. Read from the file
            // with a brief delay to let the write complete.
            let logPath = FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("thane/plans/\(entryId)/output.log").path
            DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 0.5) {
                // Parse model from JSON output (claude --print --output-format json)
                var parsedModel: String?
                if let logData = try? String(contentsOfFile: logPath, encoding: .utf8),
                   let jsonData = logData.data(using: .utf8),
                   let json = try? JSONSerialization.jsonObject(with: jsonData) as? [String: Any],
                   let model = json["model"] as? String {
                    parsedModel = model
                    NSLog("thane: queue task \(entryId) used model: \(model)")
                } else {
                    NSLog("thane: queue task \(entryId) — could not parse model from output log")
                }

                DispatchQueue.main.async { [weak self] in
                    if let model = parsedModel {
                        self?.bridge.queueUpdateModel(entryId: entryId, model: model)
                    }
                    // Scan for sensitive file access and PII
                    self?.scanOutputLogForSensitiveAccess(logPath: logPath, entryId: entryId)
                }
            }

            // Clear the running state
            runningQueueProcess = nil
            runningQueueEntryId = nil
            runningQueueStdoutPipe = nil
            runningQueueStderrPipe = nil
        }

        // Auto-dismiss finished entries after 5 seconds (matching Linux GTK)
        bridge.queueRemoveStale(maxAge: 5.0)

        // Auto-process: if nothing running and there are queued tasks, check the mode
        if runningQueueProcess == nil {
            let list = bridge.queueList()
            let hasQueued = list.contains { $0.status == .queued }
            let hasPaused = list.contains { $0.status == .pausedTokenLimit || $0.status == .pausedByUser }

            if hasQueued && !hasPaused {
                let mode = bridge.configGet(key: "queue-mode") ?? "automatic"
                if mode == "automatic" || processingAllQueued {
                    processNextQueueEntry()
                }
                if !hasQueued || mode != "automatic" {
                    processingAllQueued = false
                }
            } else {
                processingAllQueued = false
            }
        }
    }

    /// Process the next queued entry by spawning `claude --print --dangerously-skip-permissions`.
    ///
    /// Queue sandbox modes (Settings → Agent Queue → Queue Sandbox):
    /// - **Off**: no sandbox, full user permissions, task runs in ~/thane-tasks/<uuid>/
    /// - **Workspace**: runs inside the workspace's Seatbelt sandbox, CWD = workspace root.
    ///   Filesystem confined to workspace dir, credentials denied at kernel level.
    /// - **Strict**: same as Workspace + network disabled + exec restricted to system binaries.
    ///
    /// `--dangerously-skip-permissions` is safe in sandbox modes because the OS blocks the
    /// underlying syscalls regardless of Claude Code's own permission model.
    func processNextQueueEntry() {
        // Don't start a new process if one is already running
        guard runningQueueProcess == nil else { return }

        // Find the first queued entry
        let list = bridge.queueList()
        guard let entry = list.first(where: { $0.status == .queued }) else { return }

        let entryId = entry.id
        let fm = FileManager.default
        let homeDir = fm.homeDirectoryForCurrentUser

        // Create working directory ~/thane-tasks/<uuid>/
        let taskDir = homeDir.appendingPathComponent("thane-tasks")
            .appendingPathComponent(entryId)
        try? fm.createDirectory(at: taskDir, withIntermediateDirectories: true)

        // Create output log directory ~/thane/plans/<uuid>/
        let plansDir = homeDir.appendingPathComponent("thane")
            .appendingPathComponent("plans")
            .appendingPathComponent(entryId)
        try? fm.createDirectory(at: plansDir, withIntermediateDirectories: true)
        let outputLogPath = plansDir.appendingPathComponent("output.log")

        // Write the task content to a prompt file in the working directory
        let promptPath = taskDir.appendingPathComponent("prompt.md")
        try? entry.content.write(to: promptPath, atomically: true, encoding: .utf8)

        runningQueueEntryId = entryId
        bridge.queueUpdateStatus(entryId: entryId, status: .running)

        // Spawn the process — use sandbox wrapper if setting is enabled and workspace has a sandbox policy
        let process = Process()
        let searchPaths = [
            "/usr/local/bin/claude",
            "/opt/homebrew/bin/claude",
        ]
        let claudePath = searchPaths.first { fm.fileExists(atPath: $0) } ?? "claude"

        // Check queue-level sandbox
        let queueSandbox = bridge.queueSandboxStatus()
        let isSandboxed = queueSandbox?.enabled ?? false

        // Queue tasks need --dangerously-skip-permissions to run autonomously (no interactive
        // prompts in headless mode). When sandbox is enabled, the Seatbelt kernel sandbox
        // provides the actual enforcement layer — restricting filesystem, network, and exec.
        // --dangerously-skip-permissions is safe because the sandbox blocks the underlying
        // syscalls at the OS kernel level regardless of Claude Code's own permission model.
        let autonomousArgs = ["--print", "--dangerously-skip-permissions", "--output-format", "json"]

        // Determine CWD: when sandboxed, use the sandbox root directory.
        // Without sandbox, use the isolated task directory.
        let effectiveCwd: URL
        if isSandboxed, let rootDir = queueSandbox?.rootDir, !rootDir.isEmpty {
            effectiveCwd = URL(fileURLWithPath: rootDir)
        } else {
            effectiveCwd = taskDir
        }

        if isSandboxed, let policy = queueSandbox,
           FileManager.default.fileExists(atPath: "/usr/bin/sandbox-exec") {
            // Launch claude through sandbox-exec with the queue sandbox policy
            let profile = bridge.generateQueueSandboxProfile(policy)
            process.executableURL = URL(fileURLWithPath: "/usr/bin/sandbox-exec")
            var args = ["-p", profile, "--", claudePath]
            args.append(contentsOf: autonomousArgs)
            process.arguments = args
            var env = ProcessInfo.processInfo.environment
            env["THANE_SANDBOX"] = "1"
            env["THANE_SANDBOX_ROOT"] = policy.rootDir
            if policy.enforcement == .strict {
                env["THANE_SANDBOX_STRICT"] = "1"
            }
            env["THANE_QUEUE_ENTRY_ID"] = entryId
            process.environment = env
        } else {
            // No sandbox — run claude with autonomous permissions
            if fm.fileExists(atPath: claudePath) {
                process.executableURL = URL(fileURLWithPath: claudePath)
                process.arguments = autonomousArgs
            } else {
                process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
                process.arguments = ["claude"] + autonomousArgs
            }
            var env = ProcessInfo.processInfo.environment
            env["THANE_QUEUE_ENTRY_ID"] = entryId
            process.environment = env
        }

        process.currentDirectoryURL = effectiveCwd

        // Log with full command
        let sandboxLabel = isSandboxed ? " (queue sandbox)" : ""
        let entryWsId = entry.workspaceId ?? ""
        let fullCommand = ([process.executableURL!.path] + (process.arguments ?? [])).joined(separator: " ")
        bridge.logAuditEvent(workspaceId: entryWsId, eventType: "QueueTaskStarted", severity: .info,
                             description: "Queue task started\(sandboxLabel)",
                             metadata: ["entry_id": entryId,
                                        "command": fullCommand,
                                        "sandboxed": "\(isSandboxed)",
                                        "cwd": effectiveCwd.path,
                                        "content_preview": String(entry.content.prefix(200))])

        // Pipe stdin with the task content
        let stdinPipe = Pipe()
        process.standardInput = stdinPipe

        // Capture stdout and stderr — keep pipes alive so we can read after process exits
        let stdoutPipe = Pipe()
        process.standardOutput = stdoutPipe
        let stderrPipe = Pipe()
        process.standardError = stderrPipe

        runningQueueProcess = process
        runningQueueStdoutPipe = stdoutPipe
        runningQueueStderrPipe = stderrPipe

        do {
            try process.run()
            NSLog("thane: queue process started for entry \(entryId)")

            // Write prompt to stdin and close
            let promptData = entry.content.data(using: .utf8) ?? Data()
            stdinPipe.fileHandleForWriting.write(promptData)
            stdinPipe.fileHandleForWriting.closeFile()

            // Write output log to disk asynchronously (for Plans panel and sensitive scan)
            DispatchQueue.global(qos: .utility).async {
                let outputData = stdoutPipe.fileHandleForReading.readDataToEndOfFile()
                let stderrData = stderrPipe.fileHandleForReading.readDataToEndOfFile()
                // Write stdout as the primary JSON output
                try? outputData.write(to: outputLogPath)
                // Write stderr separately so it doesn't corrupt the JSON
                if !stderrData.isEmpty {
                    let stderrPath = outputLogPath.deletingLastPathComponent()
                        .appendingPathComponent("stderr.log")
                    try? stderrData.write(to: stderrPath)
                }
            }
        } catch {
            NSLog("thane: failed to start queue process for \(entryId): \(error)")
            bridge.queueUpdateStatus(entryId: entryId, status: .failed, error: error.localizedDescription)
            bridge.logAuditEvent(workspaceId: "", eventType: "QueueTaskFailed", severity: .alert,
                                 description: "Queue task failed to start: \(error.localizedDescription)",
                                 metadata: ["entry_id": entryId])
            runningQueueProcess = nil
            runningQueueEntryId = nil
        }

    }

    /// Force-rebuild a workspace's terminals (used after sandbox toggle).
    func forceRebuildWorkspace(id: String) {
        mainWindowController.forceRebuildWorkspace(id: id)
    }

    /// Process all queued entries sequentially.
    func processAllQueueEntries() {
        processingAllQueued = true
        processNextQueueEntry()
    }

    // MARK: - Queue output log scanning

    /// Sensitive file patterns matching thane-core's SENSITIVE_FILE_PATTERNS.
    private static let sensitiveFilePatterns: [(pattern: String, isKey: Bool)] = [
        (".env", false), (".env.local", false), (".env.production", false),
        ("credentials", false), ("credentials.json", false),
        ("secrets.yaml", false), ("secrets.yml", false), ("secrets.json", false),
        (".aws/credentials", false),
        (".ssh/id_rsa", true), (".ssh/id_ed25519", true), (".ssh/id_ecdsa", true),
        (".ssh/id_dsa", true), (".ssh/config", false),
        (".gnupg/", false), (".pgpass", false), (".netrc", false),
        ("service-account.json", false), ("keystore", false),
        (".p12", true), (".pfx", true), (".pem", true), (".key", true),
        ("token", false), ("api_key", false), ("apikey", false),
        ("private_key", false), ("master.key", false), ("encryption.key", false),
    ]

    private static let piiKeywords = [
        "social security", "SSN", "date of birth", "passport",
        "driver's license", "credit card", "bank account", "routing number",
    ]

    /// Scan a queue task's output log for sensitive file references and PII.
    /// Logs audit events for any findings (matching Linux GTK behavior).
    private func scanOutputLogForSensitiveAccess(logPath: String, entryId: String) {
        guard let text = try? String(contentsOfFile: logPath, encoding: .utf8) else { return }

        // Extract file paths from output text
        let paths = extractFilePaths(from: text)

        for path in paths {
            let pathLower = path.lowercased()

            // Check for private key files
            if pathLower.hasSuffix(".pem") || pathLower.hasSuffix(".key") ||
               pathLower.hasSuffix(".p12") || pathLower.hasSuffix(".pfx") ||
               pathLower.contains(".ssh/id_") {
                bridge.logAuditEvent(
                    workspaceId: "", eventType: "PrivateKeyAccess", severity: .critical,
                    description: "Private key file referenced in queue output: \(path)",
                    metadata: ["path": path, "source": "queue_output_scan", "entry_id": entryId])
                continue
            }

            // Check for other sensitive files
            for (pattern, _) in Self.sensitiveFilePatterns {
                if pathLower.contains(pattern.lowercased()) {
                    bridge.logAuditEvent(
                        workspaceId: "", eventType: "SecretAccess", severity: .alert,
                        description: "Sensitive file referenced in queue output: \(path)",
                        metadata: ["path": path, "source": "queue_output_scan", "entry_id": entryId])
                    break
                }
            }
        }

        // Detect PII
        var piiFindings: [String] = []
        let textLower = text.lowercased()
        for keyword in Self.piiKeywords {
            if textLower.contains(keyword.lowercased()) {
                piiFindings.append("Keyword match: \(keyword)")
            }
        }
        // Simple email detection
        if text.contains("@") {
            for word in text.split(whereSeparator: { $0.isWhitespace }) {
                let w = word.trimmingCharacters(in: CharacterSet(charactersIn: "<>()\"',;"))
                if w.contains("@") && w.contains(".") && w.count > 5
                    && !w.hasPrefix("@") && !w.hasSuffix("@") {
                    piiFindings.append("Possible email: \(w)")
                    break // One email finding is enough
                }
            }
        }
        // SSN pattern (XXX-XX-XXXX)
        if let _ = text.range(of: #"\b\d{3}-\d{2}-\d{4}\b"#, options: .regularExpression) {
            piiFindings.append("SSN-like pattern detected")
        }

        if !piiFindings.isEmpty {
            bridge.logAuditEvent(
                workspaceId: "", eventType: "PiiDetected", severity: .alert,
                description: "PII detected in queue output: \(piiFindings.joined(separator: ", "))",
                metadata: ["findings": piiFindings.joined(separator: "; "),
                           "source": "queue_output_scan", "entry_id": entryId])
        }
    }

    /// Extract file-path-like strings from text (absolute or ~/relative paths).
    private func extractFilePaths(from text: String) -> [String] {
        var paths: [String] = []
        let separators = CharacterSet.whitespaces.union(CharacterSet(charactersIn: "'\"`"))
        for word in text.components(separatedBy: separators) {
            let trimmed = word.trimmingCharacters(in: CharacterSet(charactersIn: ",;:)("))
            guard trimmed.count >= 3 else { continue }
            if (trimmed.hasPrefix("/") || trimmed.hasPrefix("~/")) &&
               (trimmed.dropFirst().contains("/") || trimmed.contains(".")) {
                paths.append(trimmed)
            }
        }
        return paths
    }

    // MARK: - Config hot-reload

    private func checkConfigFileChanged() {
        let fm = FileManager.default
        let configPath = fm.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/thane/config")

        guard let attrs = try? fm.attributesOfItem(atPath: configPath.path),
              let mtime = attrs[.modificationDate] as? Date else {
            return
        }

        if let lastMtime = lastConfigMtime {
            if mtime > lastMtime {
                lastConfigMtime = mtime
                NSLog("thane: config file changed, reloading")

                // Re-read config file and apply values
                if let contents = try? String(contentsOf: configPath, encoding: .utf8) {
                    for line in contents.components(separatedBy: .newlines) {
                        let trimmed = line.trimmingCharacters(in: .whitespaces)
                        guard !trimmed.isEmpty, !trimmed.hasPrefix("#") else { continue }
                        let parts = trimmed.split(separator: "=", maxSplits: 1)
                        guard parts.count == 2 else { continue }
                        let key = parts[0].trimmingCharacters(in: .whitespaces)
                        let value = parts[1].trimmingCharacters(in: .whitespaces)
                        bridge.configSet(key: key, value: value)
                    }
                }

                bridge.delegate?.configChanged()
            }
        } else {
            lastConfigMtime = mtime
        }
    }

    // MARK: - Menu actions

    @objc func newWorkspace(_ sender: Any?) {
        let homeDir = FileManager.default.homeDirectoryForCurrentUser.path
        _ = try? bridge.createWorkspace(title: "workspace", cwd: homeDir)
    }

    @objc func closeCurrentWorkspace(_ sender: Any?) {
        guard let ws = bridge.activeWorkspace() else { return }

        let alert = NSAlert()
        alert.messageText = "Close workspace \"\(ws.title)\"?"
        alert.informativeText = "This will close all panes and panels in this workspace."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Close")
        alert.addButton(withTitle: "Cancel")

        if alert.runModal() == .alertFirstButtonReturn {
            _ = try? bridge.closeWorkspace(id: ws.id)
        }
    }

    @objc func closeCurrentPanel(_ sender: Any?) {
        mainWindowController.closeCurrentPanel()
    }

    @objc func splitRight(_ sender: Any?) {
        _ = try? bridge.splitTerminal(orientation: .horizontal)
    }

    @objc func splitDown(_ sender: Any?) {
        _ = try? bridge.splitTerminal(orientation: .vertical)
    }

    @objc func toggleSidebar(_ sender: Any?) {
        mainWindowController.toggleSidebar()
    }

    @objc func showSettings(_ sender: Any?) {
        mainWindowController.showRightPanel(.settings)
    }

    @objc func showNotifications(_ sender: Any?) {
        mainWindowController.showRightPanel(.notifications)
    }

    @objc func showAuditLog(_ sender: Any?) {
        mainWindowController.showRightPanel(.audit)
    }

    @objc func showTokenUsage(_ sender: Any?) {
        mainWindowController.showRightPanel(.tokenUsage)
    }

    @objc func showAgentQueue(_ sender: Any?) {
        mainWindowController.showRightPanel(.agentQueue)
    }

    @objc func showSandbox(_ sender: Any?) {
        mainWindowController.showRightPanel(.sandbox)
    }

    @objc func showHelp(_ sender: Any?) {
        mainWindowController.showRightPanel(.help)
    }

    @objc func showGitDiff(_ sender: Any?) {
        mainWindowController.toggleGitDiff()
    }

    @objc func zoomIn(_ sender: Any?) {
        mainWindowController.adjustFontSize(delta: 1)
    }

    @objc func zoomOut(_ sender: Any?) {
        mainWindowController.adjustFontSize(delta: -1)
    }

    @objc func resetZoom(_ sender: Any?) {
        mainWindowController.resetFontSize()
    }

    @objc func toggleFullScreen(_ sender: Any?) {
        mainWindowController.window?.toggleFullScreen(sender)
    }

    @objc func nextWorkspace(_ sender: Any?) {
        mainWindowController.selectNextWorkspace()
    }

    @objc func previousWorkspace(_ sender: Any?) {
        mainWindowController.selectPreviousWorkspace()
    }

    @objc func toggleZoomPane(_ sender: Any?) {
        mainWindowController.toggleZoomPane()
    }

    @objc func showPlans(_ sender: Any?) {
        mainWindowController.showRightPanel(.plans)
    }

    @objc func findInTerminal(_ sender: Any?) {
        mainWindowController.toggleFindInTerminal()
    }

    @objc func renameWorkspace(_ sender: Any?) {
        (mainWindowController.window as? MainWindow)?.showRenameWorkspaceDialog()
    }

    @objc func nextPanelTab(_ sender: Any?) {
        bridge.focusNextPane()
    }

    @objc func previousPanelTab(_ sender: Any?) {
        bridge.focusPrevPane()
    }

    @objc func nextPane(_ sender: Any?) {
        bridge.focusNextPane()
    }

    @objc func previousPane(_ sender: Any?) {
        bridge.focusPrevPane()
    }

    // MARK: - Startup checks (matching Linux setup.rs)

    /// Generate the app icon programmatically: dark grey rounded square with white "t".
    private func generateAppIcon() -> NSImage {
        let size: CGFloat = 512
        let image = NSImage(size: NSSize(width: size, height: size))
        image.lockFocus()

        let margin: CGFloat = size / 32
        let radius: CGFloat = size * 192 / 1024
        let rect = NSRect(x: margin, y: margin, width: size - margin * 2, height: size - margin * 2)

        // Background
        let bg = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
        NSColor(red: 20/255, green: 20/255, blue: 22/255, alpha: 1).setFill()
        bg.fill()

        // Subtle border
        NSColor(red: 42/255, green: 42/255, blue: 46/255, alpha: 1).setStroke()
        bg.lineWidth = 2
        bg.stroke()

        // White "t" in JetBrains Mono Bold (or Menlo as fallback)
        let fontSize: CGFloat = size * 0.58
        let font = NSFont(name: "JetBrainsMonoNL-Bold", size: fontSize)
            ?? NSFont(name: "JetBrains Mono Bold", size: fontSize)
            ?? NSFont.monospacedSystemFont(ofSize: fontSize, weight: .bold)
        let attrs: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: NSColor.white,
        ]
        let str = NSAttributedString(string: "t", attributes: attrs)
        let strSize = str.size()
        let x = (size - strSize.width) / 2
        let y = (size - strSize.height) / 2
        str.draw(at: NSPoint(x: x, y: y))

        image.unlockFocus()
        return image
    }

    /// Whether Claude Code is available on this system.
    private var claudeInstalled: Bool = false

    private func runStartupChecks() {
        installCLITools()
        checkClaudeInstalled()
    }

    // MARK: - Update Check

    /// Fetch the latest version from the marketing site and prompt the user if outdated.
    /// Reads per-platform version from version.json: { "macos": "x.y.z", "linux": "x.y.z" }
    private func checkForUpdates() {
        guard let url = URL(string: "https://getthane.com/version.json") else { return }
        let currentVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.0.0"

        URLSession.shared.dataTask(with: url) { [weak self] data, _, error in
            guard let data = data, error == nil else {
                NSLog("thane: update check failed: \(error?.localizedDescription ?? "no data")")
                return
            }

            guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: String],
                  let remoteVersion = json["macos"],
                  !remoteVersion.isEmpty else {
                NSLog("thane: update check failed: could not parse version.json")
                return
            }

            NSLog("thane: current version \(currentVersion), latest macOS version \(remoteVersion)")

            if AppDelegate.isVersion(remoteVersion, newerThan: currentVersion) {
                DispatchQueue.main.async { [weak self] in
                    self?.showUpdateAlert(currentVersion: currentVersion, latestVersion: remoteVersion)
                }
            }
        }.resume()
    }

    /// Compare two semver version strings. Returns true if `a` is newer than `b`.
    /// Delegates to `VersionUtils.isVersion(_:newerThan:)`.
    nonisolated static func isVersion(_ a: String, newerThan b: String) -> Bool {
        VersionUtils.isVersion(a, newerThan: b)
    }

    /// Show an alert informing the user that a newer version is available.
    private func showUpdateAlert(currentVersion: String, latestVersion: String) {
        let alert = NSAlert()
        alert.messageText = "Update Available"
        alert.informativeText = "A new version of thane is available.\n\nCurrent version: \(currentVersion)\nLatest version: \(latestVersion)"
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Update")
        alert.addButton(withTitle: "Later")

        if alert.runModal() == .alertFirstButtonReturn {
            if let downloadURL = URL(string: "https://getthane.com/#install") {
                NSWorkspace.shared.open(downloadURL)
            }
        }
    }

    /// Check once at startup whether Claude Code is installed.
    private func checkClaudeInstalled() {
        claudeInstalled = checkCommand("claude", args: ["--version"])
        NSLog("thane: claude installed = \(claudeInstalled)")
    }

    /// Show a modal explaining how to install Claude Code. Returns true if user clicked OK.
    @discardableResult
    func showClaudeRequiredModal() -> Bool {
        let alert = NSAlert()
        alert.messageText = "Claude Code Required"
        alert.informativeText = """
        Claude Code is required for agent features (queue, plans, cost tracking).

        Install via Homebrew:

          1. Install Node.js (if needed):
              brew install node

          2. Install Claude Code:
              npm install -g @anthropic-ai/claude-code

        After installing, restart thane.
        """
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        alert.runModal()
        return true
    }

    /// Gate check — call before opening Claude-dependent panels. Returns true if Claude is available.
    func requireClaude() -> Bool {
        if claudeInstalled { return true }
        // Re-check in case user installed it since launch
        claudeInstalled = checkCommand("claude", args: ["--version"])
        if claudeInstalled { return true }
        showClaudeRequiredModal()
        return false
    }

    /// Symlink bundled CLI tools to /usr/local/bin so they're available in terminals.
    private func installCLITools() {
        let fm = FileManager.default
        let bundleBin = Bundle.main.bundlePath + "/Contents/MacOS"
        let installDir = "/usr/local/bin"

        // Ensure /usr/local/bin exists
        if !fm.fileExists(atPath: installDir) {
            NSLog("thane: /usr/local/bin does not exist, skipping CLI install")
            return
        }

        for tool in ["thane-cli"] {
            let src = "\(bundleBin)/\(tool)"
            let dst = "\(installDir)/\(tool)"

            guard fm.fileExists(atPath: src) else {
                NSLog("thane: \(tool) not found in app bundle, skipping")
                continue
            }

            // Check if symlink already points to the right place
            if let existing = try? fm.destinationOfSymbolicLink(atPath: dst), existing == src {
                NSLog("thane: \(tool) symlink already up to date")
                continue
            }

            // Remove stale symlink or file
            try? fm.removeItem(atPath: dst)

            do {
                try fm.createSymbolicLink(atPath: dst, withDestinationPath: src)
                NSLog("thane: symlinked \(dst) → \(src)")
            } catch {
                // May fail without admin privileges — try via osascript
                NSLog("thane: symlink failed (\(error)), trying with admin privileges...")
                let script = "do shell script \"ln -sf '\(src)' '\(dst)'\" with administrator privileges"
                if let appleScript = NSAppleScript(source: script) {
                    var errorDict: NSDictionary?
                    appleScript.executeAndReturnError(&errorDict)
                    if let err = errorDict {
                        NSLog("thane: admin symlink failed: \(err)")
                    } else {
                        NSLog("thane: symlinked \(dst) → \(src) (with admin)")
                    }
                }
            }
        }
    }

    private func checkCommand(_ command: String, args: [String]) -> Bool {
        // Try executing the command at well-known paths directly.
        // Under App Sandbox, PATH-based lookups and FileManager.fileExists
        // may not work for paths like /opt/homebrew/bin.
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let candidates = [
            "\(home)/.local/bin/\(command)",
            "/opt/homebrew/bin/\(command)",
            "/usr/local/bin/\(command)",
            "/usr/bin/\(command)",
            "\(home)/.nvm/current/bin/\(command)",
            "\(home)/.npm/bin/\(command)",
        ]
        for candidate in candidates {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: candidate)
            process.arguments = args
            process.standardOutput = Pipe()
            process.standardError = Pipe()
            do {
                try process.run()
                process.waitUntilExit()
                if process.terminationStatus == 0 {
                    NSLog("thane: checkCommand found \(command) at \(candidate)")
                    return true
                }
            } catch {
                // Binary not at this path, try next
                continue
            }
        }
        NSLog("thane: checkCommand \(command) not found at any known path")
        return false
    }

    // Old installDependencyChain / installHomebrew / installNodeJs / installClaudeCode removed.
    // Claude detection is now lazy — modal shown only when user opens a Claude-dependent panel.

    private func injectClaudeMdIfNeeded() {
        let claudeDir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude")
        let claudeMdPath = claudeDir.appendingPathComponent("CLAUDE.md")

        let marker = "<!-- thane-agent-queue-instructions-v3 -->"

        // Check if already present
        if let content = try? String(contentsOf: claudeMdPath, encoding: .utf8),
           content.contains(marker) {
            return
        }

        // Build instructions block
        let instructions = """

        \(marker)
        ## thane Agent Queue Integration

        When running inside a thane terminal workspace, you have access to the thane agent queue. The `$THANE_SOCKET_PATH` environment variable is automatically set in all thane terminal sessions.

        ### Submitting tasks to the agent queue

        When the user asks you to add a plan to the thane queue, add tasks to the queue, or schedule work for later execution, use:

        ```bash
        echo "task description here" | thane-cli queue submit -
        ```

        Or write to a temp file for multi-line tasks:

        ```bash
        cat <<'TASK' > /tmp/thane-task.md
        Your detailed task description here.
        TASK
        thane-cli queue submit /tmp/thane-task.md
        ```

        ### Queue management

        - `thane-cli queue list` — list all queued tasks
        - `thane-cli queue status <id>` — check a specific task
        - `thane-cli queue cancel <id>` — cancel a queued task

        ### Guidelines

        - Only submit to the queue when the user explicitly asks (e.g., "add this plan to my thane queue", "add to the queue", "run this later")
        - Include sufficient context in the task for an autonomous agent to execute it independently
        - Each queued task runs as an independent Claude Code session
        - When submitting tasks to the queue, always include the absolute working directory path where changes should be applied (e.g., "Working directory: /path/to/project"). Queue tasks run in isolated directories and need this context to locate the correct codebase.
        """

        do {
            try FileManager.default.createDirectory(at: claudeDir, withIntermediateDirectories: true)

            var existing = (try? String(contentsOf: claudeMdPath, encoding: .utf8)) ?? ""

            // Remove old v2 block if present
            let oldMarker = "<!-- thane-agent-queue-instructions-v2 -->"
            if let range = existing.range(of: oldMarker) {
                existing = String(existing[existing.startIndex..<range.lowerBound])
                    .trimmingCharacters(in: .newlines)
            }

            if !existing.isEmpty && !existing.hasSuffix("\n") {
                existing += "\n"
            }
            existing += instructions

            try existing.write(to: claudeMdPath, atomically: true, encoding: .utf8)
            NSLog("thane: injected thane instructions into ~/.claude/CLAUDE.md")
        } catch {
            NSLog("thane: failed to inject CLAUDE.md instructions: \(error)")
        }
    }

    private func showSetupAlert(title: String, message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    // MARK: - Single-instance lock

    private static var lockFilePath: String {
        let tmpDir = NSTemporaryDirectory()
        return (tmpDir as NSString).appendingPathComponent("thane.lock")
    }

    /// Acquire an exclusive lock file. Returns true if this is the only instance.
    private func acquireSingleInstanceLock() -> Bool {
        let path = Self.lockFilePath
        let fd = open(path, O_CREAT | O_RDWR, 0o644)
        guard fd >= 0 else { return true } // can't create lock, allow launch

        // Try non-blocking exclusive lock
        if flock(fd, LOCK_EX | LOCK_NB) != 0 {
            // Another process holds the lock
            close(fd)
            return false
        }

        // Write our PID so we can debug
        let pid = "\(ProcessInfo.processInfo.processIdentifier)\n"
        ftruncate(fd, 0)
        _ = pid.withCString { write(fd, $0, strlen($0)) }

        lockFileDescriptor = fd
        return true
    }

    /// Release the lock file on quit.
    private func releaseSingleInstanceLock() {
        guard lockFileDescriptor >= 0 else { return }
        flock(lockFileDescriptor, LOCK_UN)
        close(lockFileDescriptor)
        lockFileDescriptor = -1
        unlink(Self.lockFilePath)
    }
}
