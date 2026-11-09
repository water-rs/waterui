// swift-tools-version: 6.3
import PackageDescription

import Foundation

let embedded = ProcessInfo.processInfo.environment["WATERUI_EMBEDDED_PACKAGE"]
    ?? { fatalError("WATERUI_EMBEDDED_PACKAGE must point at a generated embedded package") }()

let package = Package(
    name: "apple-embedded-consumer",
    platforms: [.macOS("26.0"), .iOS("26.0")],
    dependencies: [
        .package(name: "EmbeddedWaterUI", path: embedded),
    ],
    targets: [
        .executableTarget(
            name: "EmbeddedConsumer",
            dependencies: [.product(name: "WaterUI", package: "EmbeddedWaterUI")]
        ),
    ]
)
