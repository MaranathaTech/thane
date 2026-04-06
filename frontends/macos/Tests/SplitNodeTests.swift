import XCTest

/// Tests for the SplitNode binary tree used for pane layout management.
/// Validates tree operations match Linux GTK's SplitTree behavior.
final class SplitNodeTests: XCTestCase {

    // MARK: - allPanels

    func testLeafAllPanels() {
        let node = SplitNode.leaf(makePanel(id: "p1"))
        XCTAssertEqual(node.allPanels.count, 1)
        XCTAssertEqual(node.allPanels[0].id, "p1")
    }

    func testSplitAllPanelsPreservesOrder() {
        let node = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "left")),
            .leaf(makePanel(id: "right"))
        )
        XCTAssertEqual(node.allPanels.map(\.id), ["left", "right"])
    }

    func testDeeplyNestedAllPanels() {
        // (p1 | (p2 / (p3 | p4)))
        let node = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "p1")),
            .split(.vertical,
                .leaf(makePanel(id: "p2")),
                .split(.horizontal,
                    .leaf(makePanel(id: "p3")),
                    .leaf(makePanel(id: "p4"))
                )
            )
        )
        XCTAssertEqual(node.allPanels.map(\.id), ["p1", "p2", "p3", "p4"])
    }

    // MARK: - replacing

    func testReplacingLeafAtRoot() {
        let root = SplitNode.leaf(makePanel(id: "p1"))
        let replacement = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "p1")),
            .leaf(makePanel(id: "p2"))
        )
        let result = root.replacing(panelId: "p1", with: replacement)
        XCTAssertEqual(result.allPanels.map(\.id), ["p1", "p2"])
    }

    func testReplacingCreatesNestedSplit() {
        // Start: left | right
        let root = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "left")),
            .leaf(makePanel(id: "right"))
        )
        // Replace right with (right / bottom) — nested vertical split
        let nested = SplitNode.split(.vertical,
            .leaf(makePanel(id: "right")),
            .leaf(makePanel(id: "bottom"))
        )
        let result = root.replacing(panelId: "right", with: nested)

        XCTAssertEqual(result.allPanels.map(\.id), ["left", "right", "bottom"])
    }

    func testReplacingNonExistentLeaves() {
        let root = SplitNode.leaf(makePanel(id: "p1"))
        let result = root.replacing(panelId: "nonexistent", with: .leaf(makePanel(id: "p2")))
        XCTAssertEqual(result.allPanels.map(\.id), ["p1"])
    }

    func testReplacingOnlyTargetLeaf() {
        // p1 | p2 | p3 — replace p2 only
        let root = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "p1")),
            .split(.horizontal,
                .leaf(makePanel(id: "p2")),
                .leaf(makePanel(id: "p3"))
            )
        )
        let replacement = SplitNode.split(.vertical,
            .leaf(makePanel(id: "p2")),
            .leaf(makePanel(id: "p4"))
        )
        let result = root.replacing(panelId: "p2", with: replacement)
        XCTAssertEqual(result.allPanels.map(\.id), ["p1", "p2", "p4", "p3"])
    }

    // MARK: - removing

    func testRemovingSingleLeafReturnsNil() {
        let root = SplitNode.leaf(makePanel(id: "p1"))
        XCTAssertNil(root.removing(panelId: "p1"))
    }

    func testRemovingFromSplitReturnsSibling() {
        let root = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "left")),
            .leaf(makePanel(id: "right"))
        )
        let result = root.removing(panelId: "left")
        XCTAssertEqual(result?.allPanels.map(\.id), ["right"])
    }

    func testRemovingFromNestedSplitCollapsesParent() {
        // (p1 | (p2 / p3)) → remove p2 → (p1 | p3)
        let root = SplitNode.split(.horizontal,
            .leaf(makePanel(id: "p1")),
            .split(.vertical,
                .leaf(makePanel(id: "p2")),
                .leaf(makePanel(id: "p3"))
            )
        )
        let result = root.removing(panelId: "p2")
        XCTAssertEqual(result?.allPanels.map(\.id), ["p1", "p3"])
    }

    func testRemovingNonExistentReturnsUnchanged() {
        let root = SplitNode.leaf(makePanel(id: "p1"))
        let result = root.removing(panelId: "nonexistent")
        XCTAssertEqual(result?.allPanels.map(\.id), ["p1"])
    }

    // MARK: - Helpers

    private func makePanel(id: String) -> PanelInfoDTO {
        PanelInfoDTO(id: id, panelType: .terminal, title: "Terminal", location: "/tmp", hasUnread: false)
    }
}
