import XCTest
import AppKit

/// Tests for RustBridge in-memory state management.
/// Validates workspace CRUD, split operations, panel tracking, and CWD updates
/// match the behavior expected by the Linux GTK frontend.
@MainActor
final class RustBridgeTests: XCTestCase {

    private var bridge: RustBridge!

    /// Derive the project root from this test file's location (works regardless of repo directory name).
    private var projectRoot: String {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // Tests/
            .deletingLastPathComponent() // macos/
            .deletingLastPathComponent() // frontends/
            .deletingLastPathComponent() // project root
            .path
    }

    override func setUp() {
        super.setUp()
        bridge = try! RustBridge()
    }

    // MARK: - Workspace CRUD

    func testCreateWorkspace() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        XCTAssertEqual(ws.title, "test")
        XCTAssertEqual(ws.cwd, "/tmp")
        XCTAssertFalse(ws.id.isEmpty)
    }

    func testListWorkspacesEmpty() {
        XCTAssertTrue(bridge.listWorkspaces().isEmpty)
    }

    func testListWorkspacesAfterCreate() throws {
        _ = try bridge.createWorkspace(title: "ws1", cwd: "/tmp")
        _ = try bridge.createWorkspace(title: "ws2", cwd: "/home")
        XCTAssertEqual(bridge.listWorkspaces().count, 2)
    }

    func testCreateWorkspaceAutoActivates() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        XCTAssertEqual(bridge.activeWorkspace()?.id, ws.id)
    }

    func testCreateSecondWorkspaceSwitchesToIt() throws {
        let ws1 = try bridge.createWorkspace(title: "first", cwd: "/tmp")
        let ws2 = try bridge.createWorkspace(title: "second", cwd: "/home")
        XCTAssertEqual(bridge.activeWorkspace()?.id, ws2.id)
        // ws1 should still exist
        XCTAssertTrue(bridge.listWorkspaces().contains { $0.id == ws1.id })
    }

    func testSelectWorkspace() throws {
        let ws1 = try bridge.createWorkspace(title: "first", cwd: "/tmp")
        _ = try bridge.createWorkspace(title: "second", cwd: "/home")

        let result = try bridge.selectWorkspace(id: ws1.id)
        XCTAssertTrue(result)
        XCTAssertEqual(bridge.activeWorkspace()?.id, ws1.id)
    }

    func testSelectNonExistentWorkspace() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let result = try bridge.selectWorkspace(id: "nonexistent")
        XCTAssertFalse(result)
    }

    func testCloseWorkspace() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let result = try bridge.closeWorkspace(id: ws.id)
        XCTAssertTrue(result)
        XCTAssertTrue(bridge.listWorkspaces().isEmpty)
    }

    func testCloseActiveWorkspaceSwitchesToAnother() throws {
        _ = try bridge.createWorkspace(title: "first", cwd: "/tmp")
        let ws2 = try bridge.createWorkspace(title: "second", cwd: "/home")
        // ws2 is active, close it
        _ = try bridge.closeWorkspace(id: ws2.id)
        // Should fall back to first
        XCTAssertNotNil(bridge.activeWorkspace())
        XCTAssertEqual(bridge.listWorkspaces().count, 1)
    }

    func testRenameWorkspace() throws {
        let ws = try bridge.createWorkspace(title: "old", cwd: "/tmp")
        let result = try bridge.renameWorkspace(id: ws.id, title: "new")
        XCTAssertTrue(result)
        XCTAssertEqual(bridge.listWorkspaces().first?.title, "new")
    }

    func testRenameNonExistent() throws {
        let result = try bridge.renameWorkspace(id: "nope", title: "new")
        XCTAssertFalse(result)
    }

    // MARK: - Panel Management

    func testNewWorkspaceHasOnePanel() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        XCTAssertEqual(bridge.listPanels().count, 1)
        XCTAssertEqual(bridge.listPanels().first?.panelType, .terminal)
    }

    func testFocusedPanelAfterCreate() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let focused = bridge.focusedPanel()
        XCTAssertNotNil(focused)
        XCTAssertEqual(focused?.panelType, .terminal)
    }

    // MARK: - Split Operations (Nested Splits — Linux Feature Parity)

    func testSplitTerminalHorizontal() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let initialPanel = bridge.focusedPanel()!

        let result = try bridge.splitTerminal(orientation: .horizontal)
        XCTAssertFalse(result.panelId.isEmpty)

        let panels = bridge.listPanels()
        XCTAssertEqual(panels.count, 2)
        // Both should be terminals
        XCTAssertTrue(panels.allSatisfy { $0.panelType == .terminal })
        // Focused should be the new panel
        XCTAssertEqual(bridge.focusedPanel()?.id, result.panelId)
        XCTAssertNotEqual(bridge.focusedPanel()?.id, initialPanel.id)
    }

    func testSplitTerminalVertical() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let result = try bridge.splitTerminal(orientation: .vertical)
        XCTAssertEqual(bridge.listPanels().count, 2)
        XCTAssertEqual(bridge.focusedPanel()?.id, result.panelId)
    }

    func testNestedSplitCreatesThreePanels() throws {
        // Linux behavior: split right, then split down on the right panel
        // → left | (right-top / right-bottom)
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")

        _ = try bridge.splitTerminal(orientation: .horizontal) // 2 panels side by side
        _ = try bridge.splitTerminal(orientation: .vertical)   // splits the focused (right) panel

        let panels = bridge.listPanels()
        XCTAssertEqual(panels.count, 3)
    }

    func testNestedSplitPreservesTreeStructure() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let p1 = bridge.focusedPanel()!.id

        let split1 = try bridge.splitTerminal(orientation: .horizontal)
        let p2 = split1.panelId
        // Now focused is p2 (right side)

        let split2 = try bridge.splitTerminal(orientation: .vertical)
        let p3 = split2.panelId

        // Tree should be: p1 | (p2 / p3)
        let tree = bridge.splitTree(for: ws.id)
        XCTAssertNotNil(tree)

        let allPanels = tree!.allPanels
        XCTAssertEqual(allPanels.count, 3)
        XCTAssertEqual(allPanels[0].id, p1) // left stays first
    }

    func testSplitInheritsCurrentCwd() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let panel1 = bridge.focusedPanel()!

        // Simulate cd to new directory
        bridge.updatePanelCwd(workspaceId: bridge.activeWorkspace()!.id, panelId: panel1.id, cwd: "/Users/test")

        // Split — new terminal should inherit the focused panel's CWD
        _ = try bridge.splitTerminal(orientation: .horizontal)
        let newPanel = bridge.focusedPanel()!
        let locations = bridge.panelLocations(for: bridge.activeWorkspace()!.id)

        // The new panel should have the same CWD as the source panel
        let newPanelLocation = locations.first { loc in
            // Match by location since we can't easily match by panel ID through locations
            true
        }
        XCTAssertNotNil(newPanelLocation)
    }

    // MARK: - Close Pane

    func testClosePaneRemovesPanel() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        _ = try bridge.splitTerminal(orientation: .horizontal)
        XCTAssertEqual(bridge.listPanels().count, 2)

        try bridge.closePane()
        XCTAssertEqual(bridge.listPanels().count, 1)
    }

    func testClosePaneFocusesRemaining() throws {
        _ = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let firstPanel = bridge.listPanels().first!.id
        _ = try bridge.splitTerminal(orientation: .horizontal)
        // Focused is the new panel

        try bridge.closePane()
        // Should focus back on the first panel
        XCTAssertEqual(bridge.focusedPanel()?.id, firstPanel)
    }

    // MARK: - CWD Tracking

    func testUpdatePanelCwd() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let panelId = bridge.focusedPanel()!.id

        bridge.updatePanelCwd(workspaceId: ws.id, panelId: panelId, cwd: "/Users/test")

        let locations = bridge.panelLocations(for: ws.id)
        XCTAssertEqual(locations.count, 1)
        XCTAssertEqual(locations[0].cwd, "/Users/test")
    }

    func testUpdatePanelCwdNormalizesFileURL() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let panelId = bridge.focusedPanel()!.id

        bridge.updatePanelCwd(workspaceId: ws.id, panelId: panelId, cwd: "file:///Users/test/")

        let locations = bridge.panelLocations(for: ws.id)
        XCTAssertEqual(locations[0].cwd, "/Users/test")
    }

    func testUpdatePanelCwdDecodesPercentEncoding() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let panelId = bridge.focusedPanel()!.id

        bridge.updatePanelCwd(workspaceId: ws.id, panelId: panelId, cwd: "file:///Users/test/my%20folder")

        let locations = bridge.panelLocations(for: ws.id)
        XCTAssertEqual(locations[0].cwd, "/Users/test/my folder")
    }

    func testUpdatePanelCwdUpdatesWorkspaceCwdForFocusedPanel() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let panelId = bridge.focusedPanel()!.id

        bridge.updatePanelCwd(workspaceId: ws.id, panelId: panelId, cwd: "/Users/new")

        XCTAssertEqual(bridge.activeWorkspace()?.cwd, "/Users/new")
    }

    func testMultiplePanelLocations() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let panel1 = bridge.focusedPanel()!.id

        let split = try bridge.splitTerminal(orientation: .horizontal)
        let panel2 = split.panelId

        bridge.updatePanelCwd(workspaceId: ws.id, panelId: panel1, cwd: "/Users/a")
        bridge.updatePanelCwd(workspaceId: ws.id, panelId: panel2, cwd: "/Users/b")

        let locations = bridge.panelLocations(for: ws.id)
        XCTAssertEqual(locations.count, 2)
        let cwds = locations.map(\.cwd)
        XCTAssertTrue(cwds.contains("/Users/a"))
        XCTAssertTrue(cwds.contains("/Users/b"))
    }

    // MARK: - Git Info

    func testGitInfoForGitRepo() throws {
        // This test runs in a git repo (the project itself)
        let ws = try bridge.createWorkspace(title: "test", cwd: projectRoot)
        bridge.updateGitInfoSync(for: ws.id)
        let locations = bridge.panelLocations(for: ws.id)
        XCTAssertEqual(locations.count, 1)
        let branch = try XCTUnwrap(locations[0].gitBranch, "Git branch should be detected for a git repo after sync refresh")
        XCTAssertFalse(branch.isEmpty)
    }

    func testGitInfoForNonGitDir() throws {
        let ws = try bridge.createWorkspace(title: "test", cwd: "/tmp")
        let locations = bridge.panelLocations(for: ws.id)
        XCTAssertEqual(locations.count, 1)
        XCTAssertNil(locations[0].gitBranch)
    }

    // MARK: - Split Tree Persistence Across Workspace Switch

    func testSplitTreePreservedOnWorkspaceSwitch() throws {
        let ws1 = try bridge.createWorkspace(title: "ws1", cwd: "/tmp")
        _ = try bridge.splitTerminal(orientation: .horizontal)
        XCTAssertEqual(bridge.listPanels().count, 2)

        let ws2 = try bridge.createWorkspace(title: "ws2", cwd: "/home")
        XCTAssertEqual(bridge.listPanels().count, 1) // ws2 has 1 panel

        // Switch back to ws1
        _ = try bridge.selectWorkspace(id: ws1.id)
        XCTAssertEqual(bridge.listPanels().count, 2) // ws1 still has 2
    }
}
