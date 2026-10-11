// swift-tools-version: 6.3
import PackageDescription

let package = Package(
    name: "{{ name }}",
    platforms: [.macOS("{{ macos }}"), .iOS("{{ ios }}")],
    products: [.library(name: "WaterUI", targets: ["WaterUI"])],
    targets: [
        .binaryTarget(name: "WaterUINative", path: "WaterUINative.xcframework"),
        .target(
            name: "WaterUI",
            dependencies: ["WaterUINative"],
            resources: [.copy("Resources/waterui_assets"), .copy("Resources/fonts"), .copy("Resources/Notices")],
            swiftSettings: [
                .enableExperimentalFeature("Extern"),
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-Xlinker", "-u", "-Xlinker", "_waterui_apple_runtime_create",
                    "-Xlinker", "-u", "-Xlinker", "_waterui_apple_runtime_drop",
                    "-Xlinker", "-u", "-Xlinker", "_waterui_apple_mount",
                    "-Xlinker", "-u", "-Xlinker", "_waterui_apple_mount_drop",
                ]),
{% for platform in links %}
{% for link in platform.links %}
{% if link.framework %}
                .linkedFramework("{{ link.name }}", .when(platforms: [.{{ platform.platform }}])),
{% else %}
                .linkedLibrary("{{ link.name }}", .when(platforms: [.{{ platform.platform }}])),
{% endif %}
{% endfor %}
{% endfor %}
            ]
        ),
    ]
)
