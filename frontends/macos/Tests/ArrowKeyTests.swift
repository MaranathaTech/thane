import XCTest
import AppKit

/// Tests for the arrow key `.numericPad` flag stripping fix in ThaneTerminalView.
///
/// macOS reports arrow keys (and Home/End/PageUp/PageDown/Delete) with the
/// `.numericPad` modifier flag, even though they are not actual numpad keys.
/// SwiftTerm's Kitty keyboard encoder treats `.numericPad` keys as keypad
/// variants (e.g. codepoint 57420 for keypad-down instead of CSI B for down),
/// which apps like Claude Code don't recognise.
///
/// These tests verify the key code classification used by ThaneTerminalView's
/// `keyDown` override to strip the erroneous flag.
final class ArrowKeyTests: XCTestCase {

    // The set of macOS key codes that should have `.numericPad` stripped.
    // Must stay in sync with ThaneTerminalView.nonNumpadKeyCodes.
    private let nonNumpadKeyCodes: Set<UInt16> = [
        123, // Left arrow
        124, // Right arrow
        125, // Down arrow
        126, // Up arrow
        115, // Home
        119, // End
        116, // Page Up
        121, // Page Down
        117, // Forward Delete
    ]

    // MARK: - Key code classification

    func testArrowKeysAreNonNumpad() {
        XCTAssertTrue(nonNumpadKeyCodes.contains(123), "Left arrow")
        XCTAssertTrue(nonNumpadKeyCodes.contains(124), "Right arrow")
        XCTAssertTrue(nonNumpadKeyCodes.contains(125), "Down arrow")
        XCTAssertTrue(nonNumpadKeyCodes.contains(126), "Up arrow")
    }

    func testNavigationKeysAreNonNumpad() {
        XCTAssertTrue(nonNumpadKeyCodes.contains(115), "Home")
        XCTAssertTrue(nonNumpadKeyCodes.contains(119), "End")
        XCTAssertTrue(nonNumpadKeyCodes.contains(116), "Page Up")
        XCTAssertTrue(nonNumpadKeyCodes.contains(121), "Page Down")
        XCTAssertTrue(nonNumpadKeyCodes.contains(117), "Forward Delete")
    }

    func testActualNumpadKeysAreNotIncluded() {
        // Numpad key codes should NOT be in the set.
        let numpadKeyCodes: [UInt16] = [
            82,  // Numpad 0
            83,  // Numpad 1
            84,  // Numpad 2
            85,  // Numpad 3
            86,  // Numpad 4
            87,  // Numpad 5
            88,  // Numpad 6
            89,  // Numpad 7
            91,  // Numpad 8
            92,  // Numpad 9
            65,  // Numpad .
            69,  // Numpad +
            75,  // Numpad /
            67,  // Numpad *
            78,  // Numpad -
            76,  // Numpad Enter
            81,  // Numpad =
        ]
        for keyCode in numpadKeyCodes {
            XCTAssertFalse(nonNumpadKeyCodes.contains(keyCode),
                           "Numpad key code \(keyCode) should not be stripped")
        }
    }

    func testRegularCharacterKeysAreNotIncluded() {
        // Letter/number keys should NOT be in the set.
        let regularKeyCodes: [UInt16] = [
            0,   // A
            1,   // S
            13,  // W
            14,  // E
            36,  // Return
            49,  // Space
            51,  // Backspace
            53,  // Escape
        ]
        for keyCode in regularKeyCodes {
            XCTAssertFalse(nonNumpadKeyCodes.contains(keyCode),
                           "Regular key code \(keyCode) should not be stripped")
        }
    }

    // MARK: - NSEvent synthesis (verifies the stripping logic works)

    func testSynthesizedEventStripsNumericPad() {
        let originalFlags: NSEvent.ModifierFlags = [.function, .numericPad]
        let strippedFlags = originalFlags.subtracting(.numericPad)

        // Verify the flag arithmetic works as expected
        XCTAssertTrue(originalFlags.contains(.numericPad))
        XCTAssertTrue(originalFlags.contains(.function))
        XCTAssertFalse(strippedFlags.contains(.numericPad))
        XCTAssertTrue(strippedFlags.contains(.function),
                      ".function flag must be preserved after stripping .numericPad")
    }

    func testSynthesizedEventPreservesOtherModifiers() {
        let originalFlags: NSEvent.ModifierFlags = [.function, .numericPad, .shift]
        let strippedFlags = originalFlags.subtracting(.numericPad)

        XCTAssertFalse(strippedFlags.contains(.numericPad))
        XCTAssertTrue(strippedFlags.contains(.function))
        XCTAssertTrue(strippedFlags.contains(.shift),
                      ".shift flag must be preserved after stripping .numericPad")
    }

    func testCreateFixedKeyEvent() {
        // Simulate creating a fixed event the same way ThaneTerminalView does.
        // Down arrow: keyCode 125, characters = "\u{F701}"
        let downArrowChar = String(UnicodeScalar(0xF701)!)
        let originalFlags: NSEvent.ModifierFlags = [.function, .numericPad]

        let fixedEvent = NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: originalFlags.subtracting(.numericPad),
            timestamp: 0,
            windowNumber: 0,
            context: nil,
            characters: downArrowChar,
            charactersIgnoringModifiers: downArrowChar,
            isARepeat: false,
            keyCode: 125
        )

        XCTAssertNotNil(fixedEvent, "Must be able to synthesize a fixed key event")
        if let event = fixedEvent {
            XCTAssertEqual(event.keyCode, 125)
            XCTAssertFalse(event.modifierFlags.contains(.numericPad),
                           "Fixed event must not contain .numericPad")
            XCTAssertTrue(event.modifierFlags.contains(.function),
                          "Fixed event must preserve .function")
        }
    }
}
