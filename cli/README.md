# waterui-cli

Cross-platform build orchestration and development tooling for WaterUI applications.

## Overview

`waterui-cli` is the command-line interface that powers the `water` binary, the primary tool for building, running, and managing WaterUI applications across iOS, macOS, and Android. It abstracts platform-specific build systems (Xcode for Apple, Gradle for Android) and provides a unified developer experience with hot reload, device management, and project scaffolding.

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

# Run on iOS Simulator with hot reload
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
- `Project::run()`: Build, package, and run on a device with optional hot reload

Example from `src/project.rs`:

```rust
pub async fn run(&self, device: impl Device, hot_reload: bool) -> Result<Running, FailToRun> {
    let platform = device.platform();

    // Build Rust library
    platform.build(self, BuildOptions::new(false, hot_reload)).await?;

    // Package for platform
    let artifact = platform.package(self, PackageOptions::new(false, true)).await?;

    // Start hot reload server if enabled
    let mut run_options = RunOptions::new();
    let server = if hot_reload {
        let server = HotReloadServer::launch(DEFAULT_PORT).await?;
        run_options.insert_env_var("WATERUI_HOT_RELOAD_HOST".to_string(), server.host());
        run_options.insert_env_var("WATERUI_HOT_RELOAD_PORT".to_string(), server.port().to_string());
        Some(server)
    } else { None };

    // Run on device
    let mut running = device.run(artifact, run_options).await?;
    if let Some(server) = server {
        running.retain(server); // Keep server alive
    }
    Ok(running)
}
```

### Hot Reload System

The hot reload system uses WebSocket to broadcast dylib updates to connected apps:

1. CLI launches `HotReloadServer` on `localhost:2006+`
2. Server monitors file changes with 250ms debouncing
3. On change, rebuild library and broadcast to all connected clients
4. Apps reload the updated library without restarting

Example from `src/debug/hot_reload.rs`:

```rust
pub async fn launch(starting_port: u16) -> Result<Self, FailToLaunch> {
    // Try ports 2006..2056
    for port in starting_port..(starting_port + PORT_RETRY_COUNT) {
        match Self::try_launch_on_port(port).await {
            Ok(server) => return Ok(server),
            Err(FailToLaunch::BindError(_, _)) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(FailToLaunch::NoAvailablePort(starting_port, starting_port + PORT_RETRY_COUNT))
}

pub fn send_library(&self, data: Vec<u8>) {
    let _ = self.broadcast_tx.try_send(BroadcastMessage::Binary(data));
}
```

Environment variables `WATERUI_HOT_RELOAD_HOST` and `WATERUI_HOT_RELOAD_PORT` are passed to running apps.

### Rust Build

The `RustBuild` type wraps `cargo build` with platform-specific configuration:

- Target triple selection (e.g., `aarch64-apple-ios-sim`)
- Hot reload flag (`--cfg waterui_hot_reload_lib`)
- Simulator-specific clang args for bindgen

Example from `src/build.rs`:

```rust
let mut cmd = Command::new("cargo");
let mut cmd = command(&mut cmd)
    .arg("build")
    .arg("--lib")
    .args(["--target", self.triple.to_string().as_str()])
    .current_dir(&self.path);

if self.hot_reload {
    let mut rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    if !rustflags.is_empty() { rustflags.push(' '); }
    rustflags.push_str("--cfg waterui_hot_reload_lib");
    cmd.env("RUSTFLAGS", rustflags);
}
```

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
water create my-app --dev --platform ios,android
```

This creates a project that uses the local WaterUI repository (useful for framework development).

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
- **`debug`**: Hot reload server, build manager, file watcher
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

- **Hot reload**: WebSocket-based live code updates
- **Multi-platform**: iOS, macOS, Android with unified workflow
- **Device management**: Automatic device discovery and simulator launching
- **Interactive creation**: Guided project setup with prompts
- **Playground mode**: Auto-managed backends for quick prototyping
- **Parallel builds**: Device launch overlaps with compilation
- **Log streaming**: Real-time device logs with level filtering
- **JSON output**: Machine-readable output with `--json` flag
- **Graceful cancellation**: Ctrl+C cleanup without errors