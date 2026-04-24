import Foundation

/// Result of checking whether thane-cli is installed on PATH.
enum CliInstallStatus: Equatable {
    /// A symlink in a PATH location resolves to this app bundle's thane-cli.
    case installed(at: String)
    /// A symlink exists but points elsewhere (older bundle, different app). Needs reinstall.
    case stale(foundAt: String, pointsAt: String)
    /// No thane-cli found at any known PATH location.
    case notInstalled
}

/// Pure-logic helpers for deciding whether thane-cli needs to be (re)installed
/// to a PATH location. AppDelegate injects real filesystem calls; tests inject
/// fakes to exercise each branch.
enum CliInstallChecker {
    /// Expected symlink destination for this app bundle.
    static func expectedSource(bundlePath: String) -> String {
        "\(bundlePath)/Contents/MacOS/thane-cli"
    }

    /// Standard PATH candidates to check, in preference order.
    static func defaultCandidates(home: String) -> [String] {
        [
            "/usr/local/bin/thane-cli",
            "/opt/homebrew/bin/thane-cli",
            "\(home)/.local/bin/thane-cli",
        ]
    }

    /// Determine install status.
    ///
    /// - `fileExists` — true if the path exists (follows symlinks is irrelevant; we just care it's there).
    /// - `resolveSymlink` — returns the symlink target if the path is a symlink, or `nil` if it's a regular file or missing.
    ///
    /// A regular file (not a symlink) at a candidate path is treated as
    /// `installed` — we don't clobber a user-managed binary.
    static func check(
        bundlePath: String,
        candidates: [String],
        fileExists: (String) -> Bool,
        resolveSymlink: (String) -> String?
    ) -> CliInstallStatus {
        let expected = expectedSource(bundlePath: bundlePath)
        for candidate in candidates {
            guard fileExists(candidate) else { continue }
            if let resolved = resolveSymlink(candidate) {
                if resolved == expected {
                    return .installed(at: candidate)
                }
                return .stale(foundAt: candidate, pointsAt: resolved)
            }
            return .installed(at: candidate)
        }
        return .notInstalled
    }

    /// Whether the app bundle is running from /Applications. We only prompt to
    /// install a PATH symlink when the bundle is at a stable location — dev
    /// builds under target/ or .build/ would create dangling symlinks.
    static func isStableBundleLocation(bundlePath: String) -> Bool {
        bundlePath.hasPrefix("/Applications/")
    }
}
