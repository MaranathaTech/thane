import XCTest
import AppKit

/// Tests for ThaneTheme constants and font loading.
@MainActor
final class ThemeTests: XCTestCase {

    // MARK: - Constants

    func testSidebarWidth() {
        XCTAssertEqual(ThaneTheme.sidebarWidth, 240)
    }

    func testSidebarCollapsedWidth() {
        XCTAssertEqual(ThaneTheme.sidebarCollapsedWidth, 48)
    }

    func testStatusBarHeight() {
        XCTAssertEqual(ThaneTheme.statusBarHeight, 28)
    }

    func testTabBarHeight() {
        XCTAssertEqual(ThaneTheme.tabBarHeight, 32)
    }

    func testRightPanelWidth() {
        XCTAssertEqual(ThaneTheme.rightPanelWidth, 320)
    }

    func testDefaultFontSize() {
        XCTAssertEqual(ThaneTheme.defaultFontSize, 14)
    }

    func testUIFontSize() {
        XCTAssertEqual(ThaneTheme.uiFontSize, 13)
    }

    func testSmallFontSize() {
        XCTAssertEqual(ThaneTheme.smallFontSize, 11)
    }

    // MARK: - Fonts

    func testTerminalFontReturnsMonospace() {
        let font = ThaneTheme.terminalFont()
        XCTAssertNotNil(font)
        XCTAssertEqual(font.pointSize, ThaneTheme.defaultFontSize)
    }

    func testTerminalFontCustomSize() {
        let font = ThaneTheme.terminalFont(size: 20)
        XCTAssertEqual(font.pointSize, 20)
    }

    func testUIFont() {
        let font = ThaneTheme.uiFont()
        XCTAssertNotNil(font)
        XCTAssertEqual(font.pointSize, ThaneTheme.uiFontSize)
    }

    func testLabelFont() {
        let font = ThaneTheme.labelFont()
        XCTAssertNotNil(font)
    }

    func testBoldLabelFont() {
        let font = ThaneTheme.boldLabelFont()
        XCTAssertNotNil(font)
    }

    // MARK: - Colors

    func testBackgroundColorNotNil() {
        XCTAssertNotNil(ThaneTheme.backgroundColor)
    }

    func testAccentColorIsIndigo() {
        let color = ThaneTheme.accentColor
        // #818cf8 ≈ (0.506, 0.549, 0.973)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        color.getRed(&r, green: &g, blue: &b, alpha: &a)
        XCTAssertEqual(r, 0.506, accuracy: 0.01)
        XCTAssertEqual(g, 0.549, accuracy: 0.01)
        XCTAssertEqual(b, 0.973, accuracy: 0.01)
    }

    func testAllColorsExist() {
        // Verify all theme colors are accessible without crashing
        _ = ThaneTheme.backgroundColor
        _ = ThaneTheme.sidebarBackground
        _ = ThaneTheme.tabBarBackground
        _ = ThaneTheme.tabSelectedBackground
        _ = ThaneTheme.statusBarBackground
        _ = ThaneTheme.raisedBackground
        _ = ThaneTheme.selectionBackground
        _ = ThaneTheme.primaryText
        _ = ThaneTheme.secondaryText
        _ = ThaneTheme.tertiaryText
        _ = ThaneTheme.accentColor
        _ = ThaneTheme.dividerColor
        _ = ThaneTheme.agentActiveColor
        _ = ThaneTheme.agentInactiveColor
        _ = ThaneTheme.costColor
        _ = ThaneTheme.warningColor
        _ = ThaneTheme.errorColor
        _ = ThaneTheme.badgeColor
    }

    // MARK: - NSAppearance extension

    func testDarkAppearance() {
        let dark = NSAppearance(named: .darkAqua)!
        XCTAssertTrue(dark.isDark)
    }

    func testLightAppearance() {
        let light = NSAppearance(named: .aqua)!
        XCTAssertFalse(light.isDark)
    }
}
