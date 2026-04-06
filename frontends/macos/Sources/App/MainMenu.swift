import AppKit

/// Builds the NSMenu bar for the thane macOS app.
/// Keyboard shortcuts map Ctrl→Cmd, Alt→Opt relative to the Linux GTK4 bindings.
enum MainMenu {

    @MainActor
    static func build(target: AppDelegate) -> NSMenu {
        let mainMenu = NSMenu()

        mainMenu.addItem(appMenu(target: target))
        mainMenu.addItem(fileMenu(target: target))
        mainMenu.addItem(editMenu())
        mainMenu.addItem(viewMenu(target: target))
        mainMenu.addItem(windowMenu())
        mainMenu.addItem(helpMenu(target: target))

        return mainMenu
    }

    // MARK: - App menu

    @MainActor
    private static func appMenu(target: AppDelegate) -> NSMenuItem {
        let item = NSMenuItem()
        let menu = NSMenu(title: "thane")

        menu.addItem(withTitle: "About thane", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        menu.addItem(.separator())

        let settingsItem = NSMenuItem(title: "Settings…", action: #selector(AppDelegate.showSettings(_:)), keyEquivalent: ",")
        settingsItem.target = target
        menu.addItem(settingsItem)

        menu.addItem(.separator())
        menu.addItem(withTitle: "Hide thane", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")

        let hideOthersItem = NSMenuItem(title: "Hide Others", action: #selector(NSApplication.hideOtherApplications(_:)), keyEquivalent: "h")
        hideOthersItem.keyEquivalentModifierMask = [.command, .option]
        menu.addItem(hideOthersItem)

        menu.addItem(withTitle: "Show All", action: #selector(NSApplication.unhideAllApplications(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit thane", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")

        item.submenu = menu
        return item
    }

    // MARK: - File menu

    @MainActor
    private static func fileMenu(target: AppDelegate) -> NSMenuItem {
        let item = NSMenuItem()
        let menu = NSMenu(title: "File")

        let newWs = NSMenuItem(title: "New Workspace", action: #selector(AppDelegate.newWorkspace(_:)), keyEquivalent: "t")
        newWs.keyEquivalentModifierMask = [.command, .shift]
        newWs.target = target
        menu.addItem(newWs)

        let renameWs = NSMenuItem(title: "Rename Workspace", action: #selector(AppDelegate.renameWorkspace(_:)), keyEquivalent: "r")
        renameWs.keyEquivalentModifierMask = [.command, .shift]
        renameWs.target = target
        menu.addItem(renameWs)

        menu.addItem(.separator())

        let closeWs = NSMenuItem(title: "Close Workspace", action: #selector(AppDelegate.closeCurrentWorkspace(_:)), keyEquivalent: "w")
        closeWs.target = target
        menu.addItem(closeWs)

        let closePanel = NSMenuItem(title: "Close Panel", action: #selector(AppDelegate.closeCurrentPanel(_:)), keyEquivalent: "w")
        closePanel.keyEquivalentModifierMask = [.command, .shift]
        closePanel.target = target
        menu.addItem(closePanel)

        item.submenu = menu
        return item
    }

    // MARK: - Edit menu

    private static func editMenu() -> NSMenuItem {
        let item = NSMenuItem()
        let menu = NSMenu(title: "Edit")

        menu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")

        item.submenu = menu
        return item
    }

    // MARK: - View menu

    @MainActor
    private static func viewMenu(target: AppDelegate) -> NSMenuItem {
        let item = NSMenuItem()
        let menu = NSMenu(title: "View")

        // Sidebar
        let sidebar = NSMenuItem(title: "Toggle Sidebar", action: #selector(AppDelegate.toggleSidebar(_:)), keyEquivalent: "b")
        sidebar.keyEquivalentModifierMask = [.command, .shift]
        sidebar.target = target
        menu.addItem(sidebar)

        menu.addItem(.separator())

        // Split panes
        let splitR = NSMenuItem(title: "Split Right", action: #selector(AppDelegate.splitRight(_:)), keyEquivalent: "d")
        splitR.keyEquivalentModifierMask = [.command, .shift]
        splitR.target = target
        menu.addItem(splitR)

        let splitD = NSMenuItem(title: "Split Down", action: #selector(AppDelegate.splitDown(_:)), keyEquivalent: "e")
        splitD.keyEquivalentModifierMask = [.command, .shift]
        splitD.target = target
        menu.addItem(splitD)

        menu.addItem(.separator())

        // Zoom
        let zoomIn = NSMenuItem(title: "Zoom In", action: #selector(AppDelegate.zoomIn(_:)), keyEquivalent: "=")
        zoomIn.target = target
        menu.addItem(zoomIn)

        let zoomOut = NSMenuItem(title: "Zoom Out", action: #selector(AppDelegate.zoomOut(_:)), keyEquivalent: "-")
        zoomOut.target = target
        menu.addItem(zoomOut)

        let resetZoom = NSMenuItem(title: "Reset Zoom", action: #selector(AppDelegate.resetZoom(_:)), keyEquivalent: "0")
        resetZoom.target = target
        menu.addItem(resetZoom)

        menu.addItem(.separator())

        // Pane zoom
        let zoomPane = NSMenuItem(title: "Toggle Pane Zoom", action: #selector(AppDelegate.toggleZoomPane(_:)), keyEquivalent: "z")
        zoomPane.keyEquivalentModifierMask = [.command, .shift]
        zoomPane.target = target
        menu.addItem(zoomPane)

        menu.addItem(.separator())

        // Panel tab cycling
        let nextTab = NSMenuItem(title: "Next Panel Tab", action: #selector(AppDelegate.nextPanelTab(_:)), keyEquivalent: "]")
        nextTab.keyEquivalentModifierMask = [.command, .shift]
        nextTab.target = target
        menu.addItem(nextTab)

        let prevTab = NSMenuItem(title: "Previous Panel Tab", action: #selector(AppDelegate.previousPanelTab(_:)), keyEquivalent: "[")
        prevTab.keyEquivalentModifierMask = [.command, .shift]
        prevTab.target = target
        menu.addItem(prevTab)

        // Pane cycling
        let nextPane = NSMenuItem(title: "Next Pane", action: #selector(AppDelegate.nextPane(_:)), keyEquivalent: "}")
        nextPane.keyEquivalentModifierMask = [.command, .shift]
        nextPane.target = target
        menu.addItem(nextPane)

        let prevPane = NSMenuItem(title: "Previous Pane", action: #selector(AppDelegate.previousPane(_:)), keyEquivalent: "{")
        prevPane.keyEquivalentModifierMask = [.command, .shift]
        prevPane.target = target
        menu.addItem(prevPane)

        menu.addItem(.separator())

        // Right-side panels
        let notifications = NSMenuItem(title: "Notifications", action: #selector(AppDelegate.showNotifications(_:)), keyEquivalent: "i")
        notifications.target = target
        menu.addItem(notifications)

        let audit = NSMenuItem(title: "Audit Log", action: #selector(AppDelegate.showAuditLog(_:)), keyEquivalent: "a")
        audit.keyEquivalentModifierMask = [.command, .shift]
        audit.target = target
        menu.addItem(audit)

        let tokens = NSMenuItem(title: "CC Token Usage", action: #selector(AppDelegate.showTokenUsage(_:)), keyEquivalent: "u")
        tokens.keyEquivalentModifierMask = [.command, .shift]
        tokens.target = target
        menu.addItem(tokens)

        let queue = NSMenuItem(title: "Agent Queue", action: #selector(AppDelegate.showAgentQueue(_:)), keyEquivalent: "p")
        queue.keyEquivalentModifierMask = [.command, .shift]
        queue.target = target
        menu.addItem(queue)

        let sandbox = NSMenuItem(title: "Sandbox", action: #selector(AppDelegate.showSandbox(_:)), keyEquivalent: "s")
        sandbox.keyEquivalentModifierMask = [.command, .shift]
        sandbox.target = target
        menu.addItem(sandbox)

        let gitDiff = NSMenuItem(title: "Git Diff", action: #selector(AppDelegate.showGitDiff(_:)), keyEquivalent: "g")
        gitDiff.keyEquivalentModifierMask = [.command, .shift]
        gitDiff.target = target
        menu.addItem(gitDiff)

        let plans = NSMenuItem(title: "Processed", action: #selector(AppDelegate.showPlans(_:)), keyEquivalent: "l")
        plans.keyEquivalentModifierMask = [.command, .shift]
        plans.target = target
        menu.addItem(plans)

        menu.addItem(.separator())

        let findInTerminal = NSMenuItem(title: "Find in Terminal", action: #selector(AppDelegate.findInTerminal(_:)), keyEquivalent: "f")
        findInTerminal.keyEquivalentModifierMask = [.command, .shift]
        findInTerminal.target = target
        menu.addItem(findInTerminal)

        menu.addItem(.separator())

        let fullScreen = NSMenuItem(title: "Enter Full Screen", action: #selector(AppDelegate.toggleFullScreen(_:)), keyEquivalent: "f")
        fullScreen.keyEquivalentModifierMask = [.command, .control]
        fullScreen.target = target
        menu.addItem(fullScreen)

        item.submenu = menu
        return item
    }

    // MARK: - Window menu

    private static func windowMenu() -> NSMenuItem {
        let item = NSMenuItem()
        let menu = NSMenu(title: "Window")

        menu.addItem(withTitle: "Minimize", action: #selector(NSWindow.miniaturize(_:)), keyEquivalent: "m")
        menu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Bring All to Front", action: #selector(NSApplication.arrangeInFront(_:)), keyEquivalent: "")

        item.submenu = menu

        // Register as the app's Window menu so macOS populates it
        NSApp.windowsMenu = menu

        return item
    }

    // MARK: - Help menu

    @MainActor
    private static func helpMenu(target: AppDelegate) -> NSMenuItem {
        let item = NSMenuItem()
        let menu = NSMenu(title: "Help")

        let helpItem = NSMenuItem(title: "thane Help", action: #selector(AppDelegate.showHelp(_:)), keyEquivalent: "")
        helpItem.target = target
        // F1 key
        helpItem.keyEquivalent = "\u{F704}" // NSF1FunctionKey
        helpItem.keyEquivalentModifierMask = []
        menu.addItem(helpItem)

        item.submenu = menu

        // Register as the app's Help menu
        NSApp.helpMenu = menu

        return item
    }
}
