// swift-tools-version: 6.2
import PackageDescription

let package = Package(
  name: "LayoutTwins",
  platforms: [
    .iOS(.v27),
    .macOS(.v27),
  ],
  targets: [
    .executableTarget(name: "LayoutTwins", path: "Sources")
  ]
)
