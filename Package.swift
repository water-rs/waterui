// swift-tools-version: 5.10
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "waterui-swift",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "WaterUI", targets: ["WaterUI"]),
    ],
    dependencies: [
        .package(url: "https://github.com/groue/Semaphore.git", from: "0.1.0"),
    ],
    targets: [
        .target(name: "CWaterUI"),
        .target(name: "WaterUI", dependencies: ["CWaterUI", "Semaphore"]),
    ]
)
