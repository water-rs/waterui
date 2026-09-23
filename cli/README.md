# waterui-cli

Cross-platform build orchestration and development tooling for WaterUI applications.

## Overview

`waterui-cli` is the command-line interface that powers the `water` binary, the primary tool for building, running, and managing WaterUI applications across iOS, macOS, and Android. It abstracts platform-specific build systems (Xcode for Apple, Gradle for Android) and provides a unified developer experience with device management, project scaffolding, and instant view previews.

The crate is split into two components:
- **Library** (`src/lib.rs`): Core abstractions for platforms, devices, builds, and project management
- **Terminal** (`src/terminal/`): User-facing CLI with argument parsing and formatted output

This separation ensures all business logic lives in the library, while the terminal layer handles only user interaction.

## Installation

Install the CLI from source within the WaterUI workspace:

```bash
cargo install --path cli
```

Or build for development (not added to PATH):

```bash
cargo build -p waterui-cli
```

## Quick Start

Create a new WaterUI project and run it on iOS Simulator:

```bash
# Create a new project
water create my-app --platform ios,android

# Run on iOS Simulator
cd my-app
water run --platform ios

# Run on Android
water run --platform android
```

Create a playground for quick experimentation (auto-managed backends):

```bash
water create --playground --name my-experiment
cd my-experiment
water run --platform ios
```

### Preview Views

Preview individual view functions without running the full app:

```bash
# Preview a view function and save as PNG
water preview my_view --platform macos --path ./app --output preview.png

# With custom frame size
water preview dashboard --platform macos --frame 800x600 --output dashboard.png
```

Mark functions with `#[preview]` to make them previewable:

```rust
use waterui::prelude::*;

#[preview]
fn my_card() -> impl View {
    vstack((
        text!("Hello Preview!"),
        text!("This renders instantly"),
    ))
    .padding()
    .background(Color::srgb(100, 150, 200))
}
```

The preview system generates symbols with the format `waterui_preview_{crate_name}_{fn_name}` to avoid conflicts between crates.

## Core Concepts

### Platform Abstraction

The `Platform` trait represents a build target (iOS, macOS, Android with different ABIs). Each platform implementation handles:

- **Device scanning**: Enumerate connected devices and emulators
- **Building**: Compile Rust library for the target triple
- **Packaging**: Generate platform-specific artifacts (`.app`, `.apk`)
- **Cleaning**: Remove build artifacts

Example from `src/platform.rs`:

```rust
pub trait Platform: Send {
    type Toolchain: Toolchain;
    type Device: Device;

    fn scan(&self) -> impl Future<Output = eyre::Result<Vec<Self::Device>>> + Send;
    fn build(&self, project: &Project, options: BuildOptions) -> impl Future<Output = eyre::Result<PathBuf>> + Send;
    fn package(&self, project: &Project, options: PackageOptions) -> impl Future<Output = eyre::Result<Artifact>> + Send;
    fn clean(&self, project: &Project) -> impl Future<Output = eyre::Result<()>> + Send;
    fn triple(&self) -> Triple;
    fn toolchain(&self) -> Self::Toolchain;
}
```

Implementations: `ApplePlatform` (iOS, macOS, simulators), `AndroidPlatform` (arm64-v8a, x86_64, etc.)

### Device Management

The `Device` trait represents something that can run an app (simulator, emulator, or physical device). Each device has a two-phase lifecycle:

1. **Launch**: Boot the emulator/simulator (no-op for physical devices)
2. **Run**: Install and execute the artifact, returning a `Running` stream

Example from `src/device.rs`:

```rust
pub trait Device: Send {
    type Platform: Platform;

    fn launch(&self) -> impl Future<Output = eyre::Result<()>> + Send;
    fn run(&self, artifact: Artifact, options: RunOptions) -> impl Future<Output = Result<Running, FailToRun>> + Send;
    fn platform(&self) -> Self::Platform;
}
```

Implementations: `AppleSimulator`, `MacOS`, `AndroidDevice`, `AndroidEmulator`

### Project Management

The `Project` type manages the `Water.toml` manifest and coordinates builds across platforms. Key methods:

- `Project::open()`: Open existing project
- `Project::create()`: Scaffold new project
- `Project::run()`: Build, package, and run on a device

### Rust Build

The `RustBuild` type wraps `cargo build` with platform-specific configuration:

- Target triple selection (e.g., `aarch64-apple-ios-sim`)
- Simulator-specific clang args for bindgen
- Optional sccache integration for faster builds

### Toolchain Management

The `Toolchain` trait checks for required dependencies and provides installation plans:

```rust
pub trait Toolchain: Send + Sync {
    type Installation: Installation;
    fn check(&self) -> impl Future<Output = Result<(), ToolchainError<Self::Installation>>> + Send;
}

pub trait Installation: Send + Sync {
    type Error: Into<eyre::Report> + Send;
    fn install(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

Example: `AppleToolchain` checks for Xcode, simulators, and rust targets. `AndroidToolchain` checks for Android SDK, NDK, and JDK.

## Examples

### Run with Device Logs

```bash
water run --platform ios --logs debug
```

This streams device logs at debug level or above to the terminal.

### Run on Specific Device

```bash
# List available devices
water devices --platform ios

# Run on specific device by ID
water run --platform ios --device "iPhone 15 Pro"
```

### Create Project with Local WaterUI Development

```bash
water create my-app --waterui-path /path/to/waterui --backends apple,android
```

This creates a project that uses the local WaterUI repository.

When the `water` CLI itself was built from a local, non-release WaterUI checkout and you run
`water create` from somewhere inside the WaterUI repository, it automatically detects the repo
root and uses it as the local `waterui_path`. Use `--waterui-path` explicitly when running that
development CLI outside the repository.

### Build Without Running

```bash
water build --platform ios --release
```

### Clean Build Artifacts

```bash
water clean --platform ios
water clean --all  # Clean all platforms
```

### Check Development Environment

```bash
water doctor --platform ios
water doctor --platform android
```

This validates toolchain dependencies (Xcode, Android SDK, Rust targets).

## API Overview

### Library (`src/lib.rs`)

- **`platform`**: Platform trait and implementations (Apple, Android)
- **`device`**: Device trait, device types, run options, and events
- **`project`**: Project management, manifest parsing, create/open
- **`build`**: Rust build orchestration with cargo
- **`debug`**: Crash handling and diagnostics
- **`toolchain`**: Toolchain checking and installation
- **`backend`**: Backend configuration and scaffolding
- **`templates`**: Project scaffolding templates
- **`apple`**: Apple platform, devices, and backend
- **`android`**: Android platform, devices, and backend
- **`brew`**: Homebrew package management utilities
- **`water_dir`**: Global WaterUI directory management
- **`utils`**: Command execution helpers

### Terminal (`src/terminal/`)

- **`main.rs`**: CLI entry point, argument parsing
- **`shell.rs`**: Output formatting, spinners, colors
- **`commands/create.rs`**: Project scaffolding command
- **`commands/run.rs`**: Build and run command
- **`commands/build.rs`**: Build-only command
- **`commands/package.rs`**: Packaging command
- **`commands/clean.rs`**: Cleanup command
- **`commands/doctor.rs`**: Toolchain validation command
- **`commands/devices.rs`**: Device listing command

## Features

The CLI supports:

- **Multi-platform**: iOS, macOS, Android with unified workflow
- **Instant previews**: Render individual views to PNG without running the full app
- **Device management**: Automatic device discovery and simulator launching
- **Interactive creation**: Guided project setup with prompts
- **Playground mode**: Auto-managed backends for quick prototyping
- **Parallel builds**: Device launch overlaps with compilation
- **Log streaming**: Real-time device logs with level filtering
- **JSON output**: Machine-readable output with `--json` flag
- **Graceful cancellation**: Ctrl+C cleanup without errors
