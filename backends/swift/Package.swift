// swift-tools-version: 6.2.0
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "waterui-swift",
    platforms: [.iOS(.v26), .macOS(.v26)],
    products: [
        .library(name: "WaterUI", targets: ["WaterUI"])
    ],
    targets: [
        .target(name: "CWaterUI"),
        .target(name: "WaterUI", dependencies: ["CWaterUI"]),
    ]
)
