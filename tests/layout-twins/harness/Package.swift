// swift-tools-version: 6.3
// Frame-recording harness for the WaterUI side of the layout twins.
// Depends on the framework's own root manifest: the Apple host target is
// `backends/apple/Sources/WaterUI` in the same repository.
import PackageDescription

let package = Package(
  name: "LayoutTwinsHarness",
  platforms: [
    .iOS(.v26),
    .macOS(.v26),
  ],
  dependencies: [
    .package(name: "waterui-apple-host", path: "../../..")
  ],
  targets: [
    .testTarget(
      name: "TwinsTests",
      dependencies: [.product(name: "WaterUI", package: "waterui-apple-host")]
    )
  ]
)
