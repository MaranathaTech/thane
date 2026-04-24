import XCTest

/// Tests for CliInstallChecker: the pure-logic decision of whether thane-cli
/// is correctly installed on PATH, stale, or missing.
final class CliInstallCheckerTests: XCTestCase {

    private let bundlePath = "/Applications/thane.app"
    private var expectedSrc: String { "\(bundlePath)/Contents/MacOS/thane-cli" }

    func testExpectedSourceIsInsideBundle() {
        XCTAssertEqual(
            CliInstallChecker.expectedSource(bundlePath: bundlePath),
            "/Applications/thane.app/Contents/MacOS/thane-cli"
        )
    }

    func testDefaultCandidatesIncludeCommonLocations() {
        let candidates = CliInstallChecker.defaultCandidates(home: "/Users/alice")
        XCTAssertTrue(candidates.contains("/usr/local/bin/thane-cli"))
        XCTAssertTrue(candidates.contains("/opt/homebrew/bin/thane-cli"))
        XCTAssertTrue(candidates.contains("/Users/alice/.local/bin/thane-cli"))
    }

    func testNotInstalledWhenNoCandidatesExist() {
        let status = CliInstallChecker.check(
            bundlePath: bundlePath,
            candidates: ["/usr/local/bin/thane-cli", "/opt/homebrew/bin/thane-cli"],
            fileExists: { _ in false },
            resolveSymlink: { _ in nil }
        )
        XCTAssertEqual(status, .notInstalled)
    }

    func testInstalledWhenSymlinkMatchesBundle() {
        let status = CliInstallChecker.check(
            bundlePath: bundlePath,
            candidates: ["/usr/local/bin/thane-cli"],
            fileExists: { $0 == "/usr/local/bin/thane-cli" },
            resolveSymlink: { [expectedSrc] path in
                path == "/usr/local/bin/thane-cli" ? expectedSrc : nil
            }
        )
        XCTAssertEqual(status, .installed(at: "/usr/local/bin/thane-cli"))
    }

    func testStaleWhenSymlinkPointsElsewhere() {
        let otherBundle = "/Applications/thane-old.app/Contents/MacOS/thane-cli"
        let status = CliInstallChecker.check(
            bundlePath: bundlePath,
            candidates: ["/usr/local/bin/thane-cli"],
            fileExists: { $0 == "/usr/local/bin/thane-cli" },
            resolveSymlink: { path in
                path == "/usr/local/bin/thane-cli" ? otherBundle : nil
            }
        )
        XCTAssertEqual(
            status,
            .stale(foundAt: "/usr/local/bin/thane-cli", pointsAt: otherBundle)
        )
    }

    func testRegularFileTreatedAsInstalled() {
        // A non-symlink regular file at the candidate path — could be a
        // user-managed install. Don't clobber it.
        let status = CliInstallChecker.check(
            bundlePath: bundlePath,
            candidates: ["/usr/local/bin/thane-cli"],
            fileExists: { $0 == "/usr/local/bin/thane-cli" },
            resolveSymlink: { _ in nil }
        )
        XCTAssertEqual(status, .installed(at: "/usr/local/bin/thane-cli"))
    }

    func testFirstMatchingCandidateWins() {
        // /opt/homebrew/bin is the existing install; /usr/local/bin is empty.
        let brewPath = "/opt/homebrew/bin/thane-cli"
        let status = CliInstallChecker.check(
            bundlePath: bundlePath,
            candidates: ["/usr/local/bin/thane-cli", brewPath],
            fileExists: { $0 == brewPath },
            resolveSymlink: { [expectedSrc] path in
                path == brewPath ? expectedSrc : nil
            }
        )
        XCTAssertEqual(status, .installed(at: brewPath))
    }

    func testIsStableBundleLocationTrueForApplications() {
        XCTAssertTrue(CliInstallChecker.isStableBundleLocation(bundlePath: "/Applications/thane.app"))
    }

    func testIsStableBundleLocationFalseForDevBuild() {
        XCTAssertFalse(CliInstallChecker.isStableBundleLocation(
            bundlePath: "/Users/me/repo/thane/frontends/macos/.build/arm64-apple-macosx/debug/thane-macos.app"
        ))
        XCTAssertFalse(CliInstallChecker.isStableBundleLocation(bundlePath: "/tmp/thane-macos.app"))
    }
}
