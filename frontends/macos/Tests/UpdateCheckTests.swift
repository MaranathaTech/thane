import XCTest

/// Tests for VersionUtils.isVersion(_:newerThan:) semver comparison.
final class UpdateCheckTests: XCTestCase {

    func testNewerMajor() {
        XCTAssertTrue(VersionUtils.isVersion("2.0.0", newerThan: "1.0.0"))
    }

    func testNewerMinor() {
        XCTAssertTrue(VersionUtils.isVersion("0.2.0", newerThan: "0.1.0"))
    }

    func testNewerPatch() {
        XCTAssertTrue(VersionUtils.isVersion("0.1.1", newerThan: "0.1.0"))
    }

    func testSameVersion() {
        XCTAssertFalse(VersionUtils.isVersion("0.1.0", newerThan: "0.1.0"))
    }

    func testOlderVersion() {
        XCTAssertFalse(VersionUtils.isVersion("0.1.0", newerThan: "0.2.0"))
    }

    func testReleaseNewerThanPrerelease() {
        XCTAssertTrue(VersionUtils.isVersion("0.1.0", newerThan: "0.1.0-beta.2"))
    }

    func testPrereleaseNotNewerThanRelease() {
        XCTAssertFalse(VersionUtils.isVersion("0.1.0-beta.2", newerThan: "0.1.0"))
    }

    func testSamePrerelease() {
        XCTAssertFalse(VersionUtils.isVersion("0.1.0-beta.2", newerThan: "0.1.0-beta.2"))
    }

    func testStripsVPrefix() {
        XCTAssertTrue(VersionUtils.isVersion("v0.2.0", newerThan: "v0.1.0"))
        XCTAssertFalse(VersionUtils.isVersion("v0.1.0", newerThan: "v0.2.0"))
    }

    func testHigherVersionWithPrereleaseIsNewer() {
        XCTAssertTrue(VersionUtils.isVersion("0.2.0-beta.1", newerThan: "0.1.0"))
    }
}
