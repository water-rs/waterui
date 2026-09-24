// swift-tools-version: 6.3
// Frame-recording harness for the WaterUI side of the layout twins.
// Depends on a local checkout of water-rs/apple-backend; measure.sh creates
// <repo-root>/../apple-backend as a symlink and this path resolves it.
import PackageDescription

let package = Package(
  name: "LayoutTwinsHarness",
  platforms: [
    .iOS(.v26),
    .macOS(.v26),
  ],
  dependencies: [
    .package(name: "waterui-swift", path: "../../../../apple-backend")
  ],
  targets: [
    .testTarget(
      name: "TwinsTests",
      dependencies: [.product(name: "WaterUI", package: "waterui-swift")]
    )
  ]
)
