// swift-tools-version: 6.4
import PackageDescription

let package = Package(
  name: "LayoutTwins",
  platforms: [
    .iOS(.v26),
    .macOS(.v26),
  ],
  targets: [
    .executableTarget(name: "LayoutTwins", path: "Sources")
  ]
)
