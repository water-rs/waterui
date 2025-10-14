// swift-tools-version: 6.2.0
// A proxy to shut up Xcode about the package location
import PackageDescription

let package = Package(
    name: "waterui",
    products: [
        .library(name: "WaterUI", targets: ["WaterUI"])
    ],
    targets: [
        .target(name: "WaterUI", path: "backends/swift")
    ]
)
