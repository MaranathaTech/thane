import Foundation

/// Semver version comparison utilities.
enum VersionUtils {
    /// Compare two semver version strings. Returns true if `a` is newer than `b`.
    /// Handles pre-release suffixes (e.g. "0.1.0-beta.2" < "0.1.0") and optional "v" prefixes.
    static func isVersion(_ a: String, newerThan b: String) -> Bool {
        func parse(_ s: String) -> ([Int], Bool) {
            let stripped = s.hasPrefix("v") ? String(s.dropFirst()) : s
            let (numStr, hasPre): (String, Bool)
            if let dashIdx = stripped.firstIndex(of: "-") {
                numStr = String(stripped[stripped.startIndex..<dashIdx])
                hasPre = true
            } else {
                numStr = stripped
                hasPre = false
            }
            let parts = numStr.split(separator: ".").compactMap { Int($0) }
            return (parts, hasPre)
        }
        let (partsA, preA) = parse(a)
        let (partsB, preB) = parse(b)
        for i in 0..<max(partsA.count, partsB.count) {
            let va = i < partsA.count ? partsA[i] : 0
            let vb = i < partsB.count ? partsB[i] : 0
            if va > vb { return true }
            if va < vb { return false }
        }
        // Same numeric version: release > pre-release.
        return !preA && preB
    }
}
