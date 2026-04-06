// swift-tools-version: 5.10

import PackageDescription

let package = Package(
    name: "thane-macos",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "thane-macos", targets: ["ThaneMacOS"])
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", from: "1.2.0"),
    ],
    targets: [
        // C module exposing the UniFFI-generated FFI header for thane-bridge.
        .target(
            name: "thane_bridgeFFI",
            path: "thane_bridgeFFI",
            publicHeadersPath: "include"
        ),
        .executableTarget(
            name: "ThaneMacOS",
            dependencies: [
                .product(name: "SwiftTerm", package: "SwiftTerm"),
                "thane_bridgeFFI",
            ],
            path: "Sources",
            exclude: [],
            resources: [
                .copy("../Resources/Fonts"),
                .copy("../Resources/AppIcon.icns"),
            ],
            linkerSettings: [
                // Link the Rust static library produced by:
                //   cargo build --release -p thane-bridge
                .unsafeFlags([
                    "-L\(Context.packageDirectory)/../../target/release",
                    "-lthane_bridge",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("WebKit"),
            ]
        ),
        .testTarget(
            name: "ThaneTests",
            dependencies: [],
            path: "Tests",
            linkerSettings: [
                .linkedFramework("AppKit"),
            ]
        ),
    ]
)
