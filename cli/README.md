# WaterUI CLI

WaterUI ships with a dedicated command line interface, exposed as the `water`
binary, to help you bootstrap, run, and package cross‑platform applications that
use the framework. The CLI keeps common platform tooling wired together so you
can stay inside your editor instead of juggling Xcode, Gradle, and browser build
chains.

## Features at a Glance

- Scaffold ready-to-run WaterUI projects with optional Android, Apple (SwiftUI),
  and Web backends.
- **Hot-reload** your app on Apple simulators/devices, Android emulators, or the
  browser with a single command.
- **Live panic reporting** - Rust panics in your app are captured and displayed
  with colorful, formatted output including file location and backtrace.
- **Crash collection** - Automatically captures and reports crashes from native
  platforms with log excerpts.
- Package signed artifacts for distribution (APK and iOS app bundles).
- Audit and repair your toolchain with `water doctor`.
- Inspect connected simulators, emulators, and physical devices.
- Clean up build artifacts and caches across supported platforms.
- **JSON output mode** for CI/CD pipelines and LLM/automation integration.

## Installation

The CLI is developed in this repository and built with Rust. Install it with
`cargo` from the workspace root:

```bash
cargo install --path cli
```

During local development you can also run the binary without installing it:

```bash
cargo run -p waterui-cli -- <command> [flags]
```

> **Note:** Release builds embed version information from Git tags. If you are
> working from an unpublished checkout without tags, use the `--dev` flag on
> project-scaffolding commands to depend on the latest framework sources.

### Prerequisites

- A Rust toolchain that supports the 2024 edition (`rustup toolchain install
  stable` keeps you current).
- Git (used by the build script to resolve framework versions).
- Platform tooling for the backends you intend to use:
  - **Apple platforms:** macOS host with Xcode + command line tools.
  - **Android:** Android SDK, NDK, CMake, JDK 17+, and either an emulator or a connected device.
  - **Web:** a modern browser (the CLI spins up a local development server).

Run `water doctor` any time to verify that everything is configured correctly.

## Global Flags

All commands share a couple of helpful flags:

- `-v / -vv`: increase logging verbosity (DEBUG / TRACE).
- `--json`: emit machine-readable JSON output. This mode is designed for:
  - CI/CD pipeline integration
  - LLM agents and automation tools
  - Scripting and programmatic access
  
  JSON output automatically disables interactive prompts, so pass the necessary
  flags up front (details below).

## Commands

| Command | Purpose | Common Flags |
| --- | --- | --- |
| `water create` | Scaffold a new WaterUI project interactively or from flags. | `--name`, `--directory`, `--backend`, `--dev`, `--yes` |
| `water run` | Build and hot-reload the app on a selected backend. | `--platform`, `--project`, `--device`, `--release`, `--no-hot-reload` |
| `water build` | Build native Rust libraries for a platform (used by IDE build scripts). | `android`, `apple`, `--release`, `--targets` |
| `water package` | Produce distributable artifacts without launching them. | `--platform`, `--all`, `--release`, `--project` |
| `water doctor` | Check (and optionally fix) toolchain prerequisites. | `--fix` |
| `water devices` | List available simulators, emulators, and devices. |  |
| `water clean` | Remove Cargo, Gradle, Xcode, and workspace caches. | `-y/--yes` |
| `water add-backend` | Add an additional backend to an existing project. | `--project`, `--dev` |

A brief overview of the most common workflows follows.

### Create a Project

Generate a new project and choose which targets you need:

```bash
water create --name "Water Demo" \
  --bundle-identifier com.example.waterdemo \
  --backend swiftui --backend android --backend web
```

If you omit flags the CLI guides you through the options interactively. Newly
created projects include:

- A Rust library with a starter `lib.rs`.
- `Water.toml`, which declares the package metadata and enabled backends.
- Backend folders (`apple/`, `android/`, `web/`) populated with templates and
  build scripts.

Pass `--dev` to use the latest framework sources directly from Git while new
releases are being cut. When running with `--json`, also supply `--yes` (and
any other configuration flags) so the command can stay non-interactive.

### Run with Hot Reload

```bash
water run
```

When no `--platform` is given, the CLI detects available targets and prompts you
to choose (or uses `--device` to match a specific simulator/emulator name).

**Features during `water run`:**

- **Hot Reload**: Source changes trigger incremental rebuilds via a file watcher.
  Disable this behaviour with `--no-hot-reload`.
- **Live Panic Reporting**: If your Rust code panics, the CLI displays a colorful,
  formatted report with the panic message, source location, thread name, and
  backtrace—all without restarting your IDE.
- **Crash Detection**: Native crashes are automatically captured with log excerpts
  to help diagnose issues.
- **Remote Logging**: Tracing logs from your app are streamed to the terminal.

Pass `--release` for optimized builds once things are ready for profiling. JSON
output requires supplying `--platform` or `--device` ahead of time to avoid
interactive prompts.

### Build Native Artifacts

The `water build` command compiles Rust code for a specific platform. It's
designed to be called from IDE build scripts (Xcode, Gradle) but can also be
used directly:

```bash
water build android --release
water build apple --release
```

**Architecture:**

Both Xcode and Android Studio projects are configured to call `water build`
automatically during their build phases:

- **Android**: Gradle runs `water build android` via a custom task before packaging
- **Apple**: Xcode runs `water build apple` via a "Build Rust Library" script phase

This ensures Rust compilation happens through a single, consistent path whether
you run from the CLI (`water run`) or click "Run" in your IDE.

The command honours environment variables such as `CONFIGURATION`,
`BUILT_PRODUCTS_DIR`, and `ANDROID_BUILD_TARGETS` when invoked from IDE build
phases.

### Package Artifacts

Produce platform bundles without launching them:

```bash
water package --platform android --release
water package --platform ios --release
```

Use `--all` to build every configured backend. The Rust libraries are built
automatically via the platform's native build system (Gradle/Xcode), which
internally calls `water build`. JSON output requires specifying `--platform`
or `--all` so the command can stay non-interactive.

### Inspect and Fix Your Toolchain

```bash
water doctor --fix
```

`doctor` runs platform-specific health checks (Rust, Swift, Android). It verifies:

- **Rust**: cargo, rustup, required targets (e.g., `aarch64-linux-android`), sccache
- **Swift/Apple**: xcodebuild, xcrun
- **Android**: adb, emulator, Java/JDK, SDK, NDK, CMake, clang toolchain

With `--fix` it attempts to repair critical issues automatically (installing
missing Rust targets, JDK via Homebrew, etc.). JSON output removes the
interactive prompts and surfaces structured summaries of each check and fix.

### List Connected Devices

```bash
water devices --json
```

The JSON output is useful for automation—e.g. selecting the first available
device inside a script. Without `--json`, the CLI prints a human-readable table.

### Clean Build Artifacts

```bash
water clean
```

The command enumerates pending cleanup actions and asks for confirmation (skip
the prompt with `--yes`). It deletes Cargo, Gradle, Xcode DerivedData, and other
workspace caches to help you reset stubborn build environments. In JSON mode,
the CLI auto-confirms and returns a structured report of what was removed,
skipped, or failed.

### Add Additional Backends

Extend an existing project with another target:

```bash
water add-backend swiftui
```

The CLI updates `Water.toml`, downloads the necessary templates, and ensures any
generated scripts are executable. As with `create`, you can pass `--dev` if you
need the development versions of the framework dependencies. Combine with
`--json` for a structured summary of what changed.

## Automating with JSON Output

All commands support the `--json` flag for machine-readable output. This is
particularly useful for:

- **CI/CD Pipelines**: Parse build results, test reports, and diagnostics
- **LLM Agents**: Structured output avoids parsing issues and provides clear
  error information
- **Scripting**: Programmatically query device lists, check doctor status, etc.

Because prompts are disabled in JSON mode, provide any required confirmation
flags (`--yes`, `--platform`, `--device`, etc.) up front.

Example JSON output from `water doctor --json`:

```json
{
  "status": "pass",
  "sections": [
    {
      "title": "Rust toolchain",
      "rows": [
        {"status": "pass", "message": "Found `cargo`", "detail": "/usr/bin/cargo"}
      ]
    }
  ],
  "suggestions": []
}
```

## Troubleshooting

- Use `water doctor` first—it highlights missing SDK components, Rust targets,
  and misconfigured environment variables.
- For verbose logs add multiple `-v` flags (up to TRACE level).
- If you cloned the repository and encounter "WATERUI_VERSION is not set" while
  scaffolding, rerun the command with `--dev` to opt into the unreleased
  framework branches.
- If hot reload isn't connecting, check that no firewall is blocking the local
  WebSocket server.

## Contributing

The CLI lives in `cli/` inside the main WaterUI workspace. Standard Rust
development workflows apply:

```bash
cargo fmt
cargo clippy --all-targets
cargo test -p waterui-cli
```

### Architecture

The CLI is structured as a library (`cli/src/lib.rs`) with a terminal frontend
(`cli/src/terminal/`). This separation allows:

- Core logic to be reused by different frontends
- Clean separation between business logic and presentation
- Easy testing of core functionality

Key modules:

- `build.rs` - Unified build system with `BuildOptions` and `BuildCoordinator`
- `installer.rs` - Cross-platform package installation
- `doctor/` - Toolchain checks and auto-fixes
- `platform/` - Platform-specific build and packaging
- `device/` - Device discovery and management

Bug reports and pull requests are welcome—open an issue in the main
[`water-rs/waterui`](https://github.com/water-rs/waterui) repository with CLI
details so we can triage it quickly.
