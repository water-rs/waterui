// swift-tools-version: 6.3.0
import PackageDescription

let package = Package(
  name: "waterui-apple-host",
  platforms: [.iOS(.v27), .macOS(.v27)],
  products: [.library(name: "WaterUI", targets: ["WaterUI"])],
  targets: [
    .target(
      name: "WaterUI",
      path: "backends/apple/Sources/WaterUI",
      swiftSettings: [.enableExperimentalFeature("Extern")]
    )
  ]
)
