import AppKit
import CoreText

/// Centralized theme constants for the thane macOS app.
/// Dark mode follows macOS system appearance via NSAppearance.
///
/// Color palette aligned with the Linux GTK CSS (`thane-gtk/src/css.rs`):
///   --bg-primary:      #0c0c0e  (terminal / deepest background)
///   --bg-surface:      #141416  (panels, sidebar, header)
///   --bg-raised:       #1a1a1d  (hover states, active items)
///   --border:          #23232a  (subtle dividers)
///   --text-primary:    #e4e4e7
///   --text-secondary:  #a1a1aa
///   --text-muted:      #71717a
///   --accent:          #818cf8  (indigo – brand color)
///   --selection:       alpha(#818cf8, 0.15)
@MainActor
enum ThaneTheme {

    // MARK: - Fonts

    static let fontFamily = "JetBrains Mono NL"
    static let defaultFontSize: CGFloat = 14
    static let uiFontSize: CGFloat = 13
    static let smallFontSize: CGFloat = 11

    /// Register bundled fonts from Resources/Fonts/ at app launch.
    static func registerBundledFonts() {
        guard let resourceURL = Bundle.main.resourceURL else { return }
        let fontsDir = resourceURL.appendingPathComponent("Fonts")
        guard let enumerator = FileManager.default.enumerator(
            at: fontsDir,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ) else { return }

        for case let fileURL as URL in enumerator {
            let ext = fileURL.pathExtension.lowercased()
            guard ext == "ttf" || ext == "otf" else { continue }
            var errorRef: Unmanaged<CFError>?
            CTFontManagerRegisterFontsForURL(fileURL as CFURL, .process, &errorRef)
            if let error = errorRef?.takeRetainedValue() {
                NSLog("thane: failed to register font \(fileURL.lastPathComponent): \(error)")
            }
        }
    }

    static func terminalFont(size: CGFloat? = nil) -> NSFont {
        let sz = size ?? defaultFontSize
        return NSFont(name: fontFamily, size: sz)
            ?? NSFont.monospacedSystemFont(ofSize: sz, weight: .regular)
    }

    static func uiFont(size: CGFloat? = nil) -> NSFont {
        let sz = size ?? uiFontSize
        return NSFont(name: fontFamily, size: sz)
            ?? NSFont.monospacedSystemFont(ofSize: sz, weight: .regular)
    }

    static func labelFont(size: CGFloat? = nil) -> NSFont {
        return NSFont.systemFont(ofSize: size ?? uiFontSize)
    }

    static func boldLabelFont(size: CGFloat? = nil) -> NSFont {
        return NSFont.boldSystemFont(ofSize: size ?? uiFontSize)
    }

    // MARK: - Colors (adaptive to dark/light mode)

    // Matches Linux --bg-primary: #0c0c0e
    static var backgroundColor: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.047, green: 0.047, blue: 0.055, alpha: 1.0) // #0c0c0e
                : NSColor.windowBackgroundColor
        }
    }

    // Matches Linux --bg-surface: #141416
    static var sidebarBackground: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.078, green: 0.078, blue: 0.086, alpha: 1.0) // #141416
                : NSColor(red: 0.95, green: 0.95, blue: 0.96, alpha: 1.0)
        }
    }

    // Matches Linux .tab-bar: #141416
    static var tabBarBackground: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.078, green: 0.078, blue: 0.086, alpha: 1.0) // #141416
                : NSColor(red: 0.93, green: 0.93, blue: 0.94, alpha: 1.0)
        }
    }

    // Matches Linux .tab-item-selected: #0c0c0e
    static var tabSelectedBackground: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.047, green: 0.047, blue: 0.055, alpha: 1.0) // #0c0c0e
                : NSColor.white
        }
    }

    // Matches Linux .status-bar: #141416
    static var statusBarBackground: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.078, green: 0.078, blue: 0.086, alpha: 1.0) // #141416
                : NSColor(red: 0.92, green: 0.92, blue: 0.93, alpha: 1.0)
        }
    }

    // Matches Linux --bg-raised: #1a1a1d (hover/active)
    static var raisedBackground: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.102, green: 0.102, blue: 0.114, alpha: 1.0) // #1a1a1d
                : NSColor(red: 0.96, green: 0.96, blue: 0.97, alpha: 1.0)
        }
    }

    // Selection: alpha(#818cf8, 0.15)
    static var selectionBackground: NSColor {
        NSColor(red: 0.506, green: 0.549, blue: 0.973, alpha: 0.15) // alpha(#818cf8, 0.15)
    }

    // Matches Linux --text-primary: #e4e4e7
    static var primaryText: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.894, green: 0.894, blue: 0.906, alpha: 1.0) // #e4e4e7
                : .labelColor
        }
    }

    // Matches Linux --text-secondary: #a1a1aa
    static var secondaryText: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.631, green: 0.631, blue: 0.667, alpha: 1.0) // #a1a1aa
                : .secondaryLabelColor
        }
    }

    // Matches Linux --text-muted: #71717a
    static var tertiaryText: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.443, green: 0.443, blue: 0.478, alpha: 1.0) // #71717a
                : .tertiaryLabelColor
        }
    }

    // Matches Linux --accent: #818cf8 (indigo – brand color)
    static var accentColor: NSColor {
        NSColor(red: 0.506, green: 0.549, blue: 0.973, alpha: 1.0) // #818cf8
    }

    // Matches Linux --border: #23232a
    static var dividerColor: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.137, green: 0.137, blue: 0.165, alpha: 1.0) // #23232a
                : .separatorColor
        }
    }

    // Matches Linux --success: #4ade80
    static var agentActiveColor: NSColor {
        NSColor(red: 0.290, green: 0.871, blue: 0.502, alpha: 1.0) // #4ade80
    }

    static var agentInactiveColor: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.443, green: 0.443, blue: 0.478, alpha: 1.0) // #71717a
                : .secondaryLabelColor
        }
    }

    // Cost: neutral secondary text so it doesn't compete with warning amber
    static var costColor: NSColor {
        NSColor(name: nil) { appearance in
            appearance.isDark
                ? NSColor(red: 0.631, green: 0.631, blue: 0.667, alpha: 1.0) // #a1a1aa (secondary)
                : .secondaryLabelColor
        }
    }

    // Matches Linux --warning: #fbbf24
    static var warningColor: NSColor {
        NSColor(red: 0.984, green: 0.749, blue: 0.141, alpha: 1.0) // #fbbf24
    }

    // Matches Linux --error: #f87171
    static var errorColor: NSColor {
        NSColor(red: 0.973, green: 0.443, blue: 0.443, alpha: 1.0) // #f87171
    }

    static var badgeColor: NSColor { accentColor }

    // MARK: - Dimensions

    static let sidebarWidth: CGFloat = 240
    static let sidebarCollapsedWidth: CGFloat = 48
    static let statusBarHeight: CGFloat = 28
    static let tabBarHeight: CGFloat = 32
    static let rightPanelWidth: CGFloat = 320
    static let dividerThickness: CGFloat = 1
    static let cornerRadius: CGFloat = 6

    // MARK: - Animation

    static let animationDuration: TimeInterval = 0.2

    // MARK: - Hex color conversion

    /// Parse a hex color string (e.g. "#e4e4e7" or "e4e4e7") into an NSColor.
    static func colorFromHex(_ hex: String) -> NSColor? {
        var h = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if h.hasPrefix("#") { h = String(h.dropFirst()) }
        guard h.count == 6, let int = UInt64(h, radix: 16) else { return nil }
        let r = CGFloat((int >> 16) & 0xFF) / 255.0
        let g = CGFloat((int >> 8) & 0xFF) / 255.0
        let b = CGFloat(int & 0xFF) / 255.0
        return NSColor(red: r, green: g, blue: b, alpha: 1.0)
    }

    /// Convert an NSColor to a hex string like "#e4e4e7".
    static func hexFromColor(_ color: NSColor) -> String {
        guard let rgb = color.usingColorSpace(.sRGB) else { return "#e4e4e7" }
        let r = Int(rgb.redComponent * 255)
        let g = Int(rgb.greenComponent * 255)
        let b = Int(rgb.blueComponent * 255)
        return String(format: "#%02x%02x%02x", r, g, b)
    }
}

// MARK: - NSAppearance helpers

extension NSAppearance {
    var isDark: Bool {
        bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }
}

/// A flipped NSClipView so scroll view content anchors to the top instead of the bottom.
class FlippedClipView: NSClipView {
    override var isFlipped: Bool { true }
}
